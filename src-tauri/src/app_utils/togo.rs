use crate::app_utils::paths::MEDIA_SUBDIRS;
use crate::crud::scheduling;
use crate::db;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MANIFEST_FORMAT: u32 = 1;
const MAX_RECENT_PULLS: usize = 3;
pub const MIN_PUSH_SECS: i64 = 60;
const TEMP_PREFIX: &str = "toast-togo-";

pub fn endpoint() -> &'static str {
    option_env!("TOAST_TOGO_ENDPOINT").unwrap_or("https://toast-to-go.njt112233.workers.dev")
}

// Instance config
// Lives at app_dir/togo.json and is never bundled. That's what keeps the UUID
// instance-specific: a pull replaces your data but not your identity.

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    Always,
    Ask,
    Never,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentPull {
    pub id: String,
    pub label: Option<String>,
    pub pulled_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToGoConfig {
    pub instance_id: String,
    pub close_behavior: CloseBehavior,
    #[serde(default)]
    pub last_push: Option<String>,
    #[serde(default)]
    pub last_pull: Option<String>,
    #[serde(default)]
    pub recent_pulls: Vec<RecentPull>,
}

fn config_path(app_dir: &Path) -> PathBuf {
    app_dir.join("togo.json")
}

pub fn load_config(app_dir: &Path) -> Result<ToGoConfig, String> {
    if let Ok(raw) = fs::read_to_string(config_path(app_dir)) {
        if let Ok(cfg) = serde_json::from_str::<ToGoConfig>(&raw) {
            return Ok(cfg);
        }
        // instance_id is the slot key, keep the bytes recoverable
        let bad = config_path(app_dir).with_extension("json.bad");
        let _ = fs::rename(config_path(app_dir), &bad);
        log::error!(
            "togo.json unreadable; regenerating identity (old file kept at {})",
            bad.display()
        );
    }

    let cfg = ToGoConfig {
        instance_id: uuid::Uuid::new_v4().to_string(),
        close_behavior: CloseBehavior::Never,
        last_push: None,
        last_pull: None,
        recent_pulls: Vec::new(),
    };
    save_config(app_dir, &cfg)?;
    Ok(cfg)
}

/// Writes via temp + rename so a crash mid-write can't corrupt the identity.
pub fn save_config(app_dir: &Path, cfg: &ToGoConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = config_path(app_dir).with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, config_path(app_dir)).map_err(|e| e.to_string())
}

/// Seconds until this instance may push again, or 0 if it can push now.
pub fn push_cooldown(cfg: &ToGoConfig) -> i64 {
    let Some(last) = cfg.last_push.as_ref() else {
        return 0;
    };
    let Ok(last) = chrono::DateTime::parse_from_rfc3339(last) else {
        return 0;
    };
    let elapsed = chrono::Local::now()
        .signed_duration_since(last)
        .num_seconds();
    (MIN_PUSH_SECS - elapsed).max(0)
}

/// Most-recent-first, deduped by id (a re-pull keeps its label), capped at 3.
pub fn record_pull(cfg: &mut ToGoConfig, id: &str) {
    let now = chrono::Local::now().to_rfc3339();
    let label = cfg
        .recent_pulls
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.label.clone());

    cfg.recent_pulls.retain(|r| r.id != id);
    cfg.recent_pulls.insert(
        0,
        RecentPull {
            id: id.to_string(),
            label,
            pulled_at: now.clone(),
        },
    );
    cfg.recent_pulls.truncate(MAX_RECENT_PULLS);
    cfg.last_pull = Some(now);
}

/// Push/pull scratch space. Prefixed so sweep_stale_temp can find dirs whose
/// TempDir cleanup never ran (force-kill, power loss).
fn togo_tempdir() -> Result<tempfile::TempDir, String> {
    tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir()
        .map_err(|e| e.to_string())
}

/// Startup guard: a killed transfer strands its scratch dir (up to 1 GB).
/// Age-gated so a second running instance mid-transfer keeps its files.
pub fn sweep_stale_temp() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 24 * 60 * 60);
        if stale {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

// Transport

const PART_BYTES: usize = 50 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Deserialize)]
struct StartMpu {
    #[serde(rename = "uploadId")]
    upload_id: String,
}

#[derive(Deserialize, Serialize)]
pub struct UploadedPart {
    #[serde(rename = "partNumber")]
    part_number: u32,
    etag: String,
}

const THROTTLED: &str = "Too many requests. Wait a minute and try again.";

/// A 429's body says how long to wait (a minute vs. a day). Pass it through.
async fn throttle_msg(res: reqwest::Response) -> String {
    #[derive(Deserialize)]
    struct Body {
        error: String,
    }
    res.json::<Body>()
        .await
        .map(|b| b.error)
        .unwrap_or_else(|_| THROTTLED.into())
}

fn network_err(e: reqwest::Error) -> String {
    log::error!("togo network error: {e}");
    "Couldn't reach Toast to Go. Check your connection.".into()
}

/// Uploads a package to `id`'s slot in 50 MB parts. Aborts the upload on any
/// failure so an incomplete one can't linger in the bucket.
pub async fn upload(zip_path: &Path, id: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let base = format!("{}/p/{id}", endpoint());

    let started = client
        .post(format!("{base}/mpu"))
        .send()
        .await
        .map_err(network_err)?;
    if started.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(throttle_msg(started).await);
    }
    if !started.status().is_success() {
        return Err(format!("Push failed ({}).", started.status()));
    }
    let upload_id = started
        .json::<StartMpu>()
        .await
        .map_err(network_err)?
        .upload_id;

    let abort = |client: reqwest::Client, base: String, upload_id: String| async move {
        let _ = client.delete(format!("{base}/mpu/{upload_id}")).send().await;
    };

    let mut file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut parts: Vec<UploadedPart> = Vec::new();
    let mut part_number = 1u32;

    loop {
        let mut buf = vec![0u8; PART_BYTES];
        let mut filled = 0;
        while filled < PART_BYTES {
            match file.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => {
                    abort(client.clone(), base.clone(), upload_id).await;
                    return Err(e.to_string());
                }
            }
        }
        if filled == 0 {
            break;
        }
        buf.truncate(filled);

        let sent = client
            .put(format!("{base}/mpu/{upload_id}/{part_number}"))
            .body(buf)
            .send()
            .await;

        let part = match sent {
            Ok(r) if r.status().is_success() => r.json::<UploadedPart>().await.map_err(network_err),
            Ok(r) if r.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE => {
                Err("Your Toast is too large to push (over 1 GB).".to_string())
            }
            Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                Err(throttle_msg(r).await)
            }
            Ok(r) => Err(format!("Upload rejected ({}).", r.status())),
            Err(e) => Err(network_err(e)),
        };

        match part {
            Ok(p) => parts.push(p),
            Err(e) => {
                abort(client.clone(), base.clone(), upload_id).await;
                return Err(e);
            }
        }
        part_number += 1;
    }

    // The slot only changes here, an aborted push never becomes visible.
    let res = client
        .post(format!("{base}/mpu/{upload_id}/complete"))
        .json(&serde_json::json!({ "parts": parts }))
        .send()
        .await
        .map_err(network_err)?;

    if !res.status().is_success() {
        let status = res.status();
        let msg = if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            "Your Toast is too large to push (over 1 GB).".into()
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            throttle_msg(res).await
        } else {
            format!("Push failed ({status}).")
        };
        abort(client, base, upload_id).await;
        return Err(msg);
    }
    Ok(())
}

/// Downloads `id`'s package to a temp file.
pub async fn download(id: &str) -> Result<(tempfile::TempDir, PathBuf), String> {
    let mut res = reqwest::Client::new()
        .get(format!("{}/p/{id}", endpoint()))
        .send()
        .await
        .map_err(network_err)?;

    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("No package found for that ID.".into());
    }
    if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(throttle_msg(res).await);
    }
    if !res.status().is_success() {
        return Err(format!("Pull failed ({}).", res.status()));
    }

    let tmp = togo_tempdir()?;
    let zip_path = tmp.path().join("package.zip");
    let mut file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut written: u64 = 0;
    while let Some(chunk) = res.chunk().await.map_err(network_err)? {
        written += chunk.len() as u64;
        if written > MAX_PACKAGE_BYTES {
            return Err("That package is too large to pull.".into());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
    }
    Ok((tmp, zip_path))
}

#[derive(Serialize, Debug)]
pub struct SlotInfo {
    pub size: u64,
    pub uploaded: Option<String>,
}

/// The package at `id`, or None if the slot is empty.
pub async fn slot_info(id: &str) -> Result<Option<SlotInfo>, String> {
    let res = reqwest::Client::new()
        .head(format!("{}/p/{id}", endpoint()))
        .send()
        .await
        .map_err(network_err)?;

    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(THROTTLED.into());
    }
    if !res.status().is_success() {
        return Err(format!("Couldn't check that ID ({}).", res.status()));
    }

    let header = |name: &str| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    Ok(Some(SlotInfo {
        size: header("x-package-size")
            .or_else(|| header("content-length"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        uploaded: header("x-package-uploaded"),
    }))
}

// Package

#[derive(Serialize, Deserialize, Debug)]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    pub schema_version: u32,
    pub instance_id: String,
    pub app_date: String,
    pub created_at: String,
}

/// Zips a database snapshot plus the media trees. The TempDir in the result
/// owns the path and must outlive the caller's use.
pub fn bundle(
    app_dir: &Path,
    conn: &Connection,
    instance_id: &str,
) -> Result<(tempfile::TempDir, PathBuf), String> {
    let tmp = togo_tempdir()?;

    // VACUUM INTO, not a byte copy: the live connection may be mid-write.
    let snapshot = tmp.path().join("database.db");
    conn.execute("VACUUM INTO ?1", [snapshot.to_string_lossy().as_ref()])
        .map_err(|e| format!("Could not snapshot the database: {e}"))?;

    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: db::SCHEMA_VERSION,
        instance_id: instance_id.to_string(),
        app_date: scheduling::get_date(conn).map_err(|e| e.to_string())?,
        created_at: chrono::Local::now().to_rfc3339(),
    };

    let zip_path = tmp.path().join("package.zip");
    let file = fs::File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::SimpleFileOptions = Default::default();

    zip.start_file::<_, ()>("manifest.json", opts)
        .map_err(|e| e.to_string())?;
    zip.write_all(
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;

    zip.start_file::<_, ()>("database.db", opts)
        .map_err(|e| e.to_string())?;
    zip.write_all(&fs::read(&snapshot).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    for subdir in MEDIA_SUBDIRS {
        let dir = app_dir.join(subdir);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue; // a fresh install may not have every media dir
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name();
            let rel = format!("{subdir}/{}", name.to_string_lossy());
            zip.start_file::<_, ()>(&rel, opts)
                .map_err(|e| e.to_string())?;
            zip.write_all(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok((tmp, zip_path))
}

fn read_manifest(zip_path: &Path, stage: &Path) -> Result<Manifest, String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "Not a valid package.".to_string())?;
    let mut budget = MAX_UNPACKED_BYTES;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;

        // Reject zip-slip: absolute paths and traversal.
        let Some(name) = entry.enclosed_name() else {
            return Err("Package contains an unsafe path.".into());
        };
        let out = stage.join(&name);
        if !out.starts_with(stage) {
            return Err("Package contains an unsafe path.".into());
        }

        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // Cap what a zip can expand to, and don't trust its declared sizes.
        let mut buf = Vec::new();
        (&mut entry)
            .take(budget + 1)
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        if buf.len() as u64 > budget {
            return Err("Package is unreasonably large.".into());
        }
        budget -= buf.len() as u64;
        fs::write(&out, buf).map_err(|e| e.to_string())?;
    }

    let raw = fs::read_to_string(stage.join("manifest.json"))
        .map_err(|_| "Package has no manifest.".to_string())?;
    let manifest: Manifest =
        serde_json::from_str(&raw).map_err(|_| "Package manifest is unreadable.".to_string())?;

    if !stage.join("database.db").exists() {
        return Err("Package has no database.".into());
    }
    Ok(manifest)
}

/// Validates a package, then swaps it in and reopens `conn` against it.
/// Destructive on success. `app_date` is deliberately left as it came, see staleness check below.
pub fn restore(app_dir: &Path, zip_path: &Path, conn: &mut Connection) -> Result<(), String> {
    // Staged inside app_dir: the swap renames below can't cross filesystems,
    // and the system temp dir often lives on one (tmpfs).
    let staged = tempfile::Builder::new()
        .prefix(".togo-staging")
        .tempdir_in(app_dir)
        .map_err(|e| e.to_string())?;
    let manifest = read_manifest(zip_path, staged.path())?;

    if manifest.format != MANIFEST_FORMAT {
        return Err("This package was made by a newer version of Toast.".into());
    }
    let ours = env!("CARGO_PKG_VERSION");
    if manifest.app_version != ours {
        return Err(format!(
            "That package was made by Toast {}; you're on {}. Both machines must run the same version.",
            manifest.app_version, ours
        ));
    }
    if manifest.schema_version != db::SCHEMA_VERSION {
        return Err("That package's data format doesn't match this version of Toast.".into());
    }

    // Reject a package dated ahead of local: pulling it would import stats and
    // SRS state dated past today, and update_date only ticks forward.
    let local_date = scheduling::get_date(conn).map_err(|e| e.to_string())?;
    if local_date < manifest.app_date {
        return Err(format!(
            "That package is from the future ({}; you're on {}). Pull refused.",
            manifest.app_date, local_date
        ));
    }

    // Fail before destroying anything, not after. Migrations run here, on the
    // staged db, so a failure can't strand a half-migrated live one.
    let staged_db = staged.path().join("database.db");
    {
        let probe = Connection::open(&staged_db)
            .map_err(|_| "Package database can't be opened.".to_string())?;
        let ok: String = probe
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if ok != "ok" {
            return Err("Package database is corrupt.".into());
        }
        db::init_schema(&probe, app_dir).map_err(|e| e.to_string())?;
    }

    // Close the live connection before touching the file: Windows won't rename a file with an open handle.
    let old = std::mem::replace(
        conn,
        Connection::open_in_memory().map_err(|e| e.to_string())?,
    );
    drop(old);

    let rollback = app_dir.join(".togo-rollback");
    let _ = fs::remove_dir_all(&rollback);
    fs::create_dir_all(&rollback).map_err(|e| e.to_string())?;

    // db moves aside first and back last, so "live db missing" brackets the
    // whole swap, that's what recover_interrupted_swap keys on.
    let swap = || -> std::io::Result<()> {
        for name in ["database.db", "cards", "pages"] {
            let live = app_dir.join(name);
            if live.exists() {
                fs::rename(&live, rollback.join(name))?;
            }
        }
        for name in ["cards", "pages", "database.db"] {
            let incoming = staged.path().join(name);
            if incoming.exists() {
                fs::rename(&incoming, app_dir.join(name))?;
            }
        }
        Ok(())
    };

    if let Err(e) = swap() {
        log::error!("togo restore failed mid-swap: {e}");
        for name in ["database.db", "cards", "pages"] {
            let saved = rollback.join(name);
            if saved.exists() {
                let _ = fs::remove_dir_all(app_dir.join(name));
                let _ = fs::rename(&saved, app_dir.join(name));
            }
        }
        *conn = Connection::open(app_dir.join("database.db")).map_err(|e| e.to_string())?;
        return Err("Pull failed; your data was left untouched.".into());
    }

    let _ = fs::remove_dir_all(&rollback);

    *conn = Connection::open(app_dir.join("database.db"))
        .map_err(|e| format!("Pull succeeded, but the database couldn't be reopened ({e}). Restart Toast."))?;
    Ok(())
}

/// Startup guard: a crash mid-swap leaves the live db in `.togo-rollback/`.
pub fn recover_interrupted_swap(app_dir: &Path) {
    if let Ok(entries) = fs::read_dir(app_dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(".togo-staging") {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    let rollback = app_dir.join(".togo-rollback");
    if !rollback.exists() {
        return;
    }
    if rollback.join("database.db").exists() && !app_dir.join("database.db").exists() {
        log::error!("interrupted togo pull detected; rolling back");
        for name in ["database.db", "cards", "pages"] {
            let saved = rollback.join(name);
            if saved.exists() {
                let _ = fs::remove_dir_all(app_dir.join(name));
                let _ = fs::rename(&saved, app_dir.join(name));
            }
        }
    }
    let _ = fs::remove_dir_all(&rollback);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app dir with a schema'd db, one card, and one media file.
    fn make_app(date: &str) -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let app_dir = dir.path();
        for sub in MEDIA_SUBDIRS {
            fs::create_dir_all(app_dir.join(sub)).unwrap();
        }
        fs::write(app_dir.join("cards/images/a.png"), b"PNGDATA").unwrap();

        let conn = Connection::open(app_dir.join("database.db")).unwrap();
        db::init_schema(&conn, app_dir).unwrap();
        conn.execute("INSERT INTO app_date (id, date) VALUES (0, ?1)", [date])
            .unwrap();
        conn.execute(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'D', 'deck')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card (id, group_id, front, back, front_image)
             VALUES (1, 1, 'f', 'b', 'cards/images/a.png')",
            [],
        )
        .unwrap();
        (dir, conn)
    }

    /// Rewrites a field in the package's manifest, repacking the zip.
    fn tamper(zip_path: &Path, edit: impl Fn(&mut Manifest)) -> PathBuf {
        let staged = tempfile::tempdir().unwrap();
        let mut manifest = read_manifest(zip_path, staged.path()).unwrap();
        edit(&mut manifest);
        fs::write(
            staged.path().join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let out = zip_path.with_extension("tampered.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&out).unwrap());
        let opts: zip::write::SimpleFileOptions = Default::default();
        for name in ["manifest.json", "database.db"] {
            zip.start_file::<_, ()>(name, opts).unwrap();
            zip.write_all(&fs::read(staged.path().join(name)).unwrap())
                .unwrap();
        }
        zip.finish().unwrap();
        out
    }

    #[test]
    fn round_trips_cards_and_media() {
        let (src, conn) = make_app("2026-07-14");
        let (_tmp, zip) = bundle(src.path(), &conn, "instance-a").unwrap();

        let (dst, mut dconn) = make_app("2026-07-14");
        dconn.execute("DELETE FROM card", []).unwrap();
        fs::remove_file(dst.path().join("cards/images/a.png")).unwrap();

        restore(dst.path(), &zip, &mut dconn).unwrap();

        let front: String = dconn
            .query_row("SELECT front FROM card WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(front, "f");
        assert_eq!(
            fs::read(dst.path().join("cards/images/a.png")).unwrap(),
            b"PNGDATA"
        );
    }

    #[test]
    fn restore_leaves_app_date_alone() {
        // The pulled date must survive: update_date ticks the SRS forward by
        // (today - stored), so overwriting it here would skip the rollover.
        let (src, conn) = make_app("2026-07-10");
        let (_tmp, zip) = bundle(src.path(), &conn, "a").unwrap();

        let (dst, mut dconn) = make_app("2026-07-14");
        restore(dst.path(), &zip, &mut dconn).unwrap();

        let date: String = scheduling::get_date(&dconn).unwrap();
        assert_eq!(date, "2026-07-10");
    }

    #[test]
    fn accepts_older_and_equal_dates() {
        for pkg_date in ["2026-07-10", "2026-07-14"] {
            let (src, conn) = make_app(pkg_date);
            let (_tmp, zip) = bundle(src.path(), &conn, "a").unwrap();
            let (dst, mut dconn) = make_app("2026-07-14");
            assert!(restore(dst.path(), &zip, &mut dconn).is_ok(), "{pkg_date}");
        }
    }

    #[test]
    fn rejects_future_dated_package() {
        let (src, conn) = make_app("2026-07-20");
        let (_tmp, zip) = bundle(src.path(), &conn, "a").unwrap();

        let (dst, mut dconn) = make_app("2026-07-14");
        let err = restore(dst.path(), &zip, &mut dconn).unwrap_err();
        assert!(err.contains("from the future"), "{err}");

        // Local data must be untouched by a refused pull.
        let n: i64 = dconn
            .query_row("SELECT COUNT(*) FROM card", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn rejects_version_mismatch() {
        let (src, conn) = make_app("2026-07-14");
        let (_tmp, zip) = bundle(src.path(), &conn, "a").unwrap();
        let bad = tamper(&zip, |m| m.app_version = "9.9.9".into());

        let (dst, mut dconn) = make_app("2026-07-14");
        let err = restore(dst.path(), &bad, &mut dconn).unwrap_err();
        assert!(err.contains("9.9.9"), "{err}");
    }

    #[test]
    fn rejects_schema_mismatch() {
        let (src, conn) = make_app("2026-07-14");
        let (_tmp, zip) = bundle(src.path(), &conn, "a").unwrap();
        let bad = tamper(&zip, |m| m.schema_version = db::SCHEMA_VERSION + 1);

        let (dst, mut dconn) = make_app("2026-07-14");
        assert!(restore(dst.path(), &bad, &mut dconn).is_err());
    }

    #[test]
    fn rejects_zip_slip() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("evil.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        let opts: zip::write::SimpleFileOptions = Default::default();
        zip.start_file::<_, ()>("../../escaped.txt", opts).unwrap();
        zip.write_all(b"pwned").unwrap();
        zip.finish().unwrap();

        let stage = tempfile::tempdir().unwrap();
        assert!(read_manifest(&zip_path, stage.path()).is_err());
        assert!(!dir.path().parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn recovers_interrupted_swap() {
        let (dir, conn) = make_app("2026-07-14");
        drop(conn);

        // Simulate a crash after the live tree moved aside.
        let rollback = dir.path().join(".togo-rollback");
        fs::create_dir_all(&rollback).unwrap();
        for name in ["database.db", "cards", "pages"] {
            fs::rename(dir.path().join(name), rollback.join(name)).unwrap();
        }
        fs::create_dir_all(dir.path().join(".togo-staging-abc")).unwrap();

        recover_interrupted_swap(dir.path());

        assert!(dir.path().join("database.db").exists());
        assert!(dir.path().join("cards/images/a.png").exists());
        assert!(!rollback.exists());
        assert!(!dir.path().join(".togo-staging-abc").exists());
    }

    /// Hits the live Worker. Run explicitly:
    ///   cargo test live_round_trip -- --ignored --nocapture
    #[ignore]
    #[tokio::test]
    async fn live_round_trip() {
        let id = uuid::Uuid::new_v4().to_string();

        let (src, conn) = make_app("2026-07-14");
        // Something big enough to force a multi-part upload path is overkill
        // here; the worker was already proven at 130 MB. Prove the wiring.
        fs::write(src.path().join("cards/audio/big.wav"), vec![7u8; 3 * 1024 * 1024]).unwrap();
        let (_tmp, zip) = bundle(src.path(), &conn, &id).unwrap();

        assert!(slot_info(&id).await.unwrap().is_none(), "slot should start empty");
        upload(&zip, &id).await.expect("upload");
        let info = slot_info(&id).await.unwrap().expect("slot should now exist");
        println!("uploaded {} bytes to {id}", info.size);

        let (_dtmp, pulled) = download(&id).await.expect("download");

        let (dst, mut dconn) = make_app("2026-07-14");
        dconn.execute("DELETE FROM card", []).unwrap();
        restore(dst.path(), &pulled, &mut dconn).expect("restore");

        let front: String = dconn
            .query_row("SELECT front FROM card WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(front, "f");
        assert_eq!(
            fs::read(dst.path().join("cards/images/a.png")).unwrap(),
            b"PNGDATA"
        );
        assert_eq!(
            fs::read(dst.path().join("cards/audio/big.wav")).unwrap().len(),
            3 * 1024 * 1024
        );
        println!("round trip ok, card and media restored from R2");
    }

    #[test]
    fn record_pull_dedupes_keeps_label_and_caps_at_three() {
        let mut cfg = ToGoConfig {
            instance_id: "me".into(),
            close_behavior: CloseBehavior::Ask,
            last_push: None,
            last_pull: None,
            recent_pulls: Vec::new(),
        };

        for id in ["a", "b", "c", "d"] {
            record_pull(&mut cfg, id);
        }
        let ids: Vec<_> = cfg.recent_pulls.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "b"]); // most recent first, "a" evicted

        cfg.recent_pulls[2].label = Some("Laptop".into());
        record_pull(&mut cfg, "b"); // re-pull moves it up, keeps its label
        assert_eq!(cfg.recent_pulls[0].id, "b");
        assert_eq!(cfg.recent_pulls[0].label.as_deref(), Some("Laptop"));
        assert_eq!(cfg.recent_pulls.len(), 3);
    }
}
