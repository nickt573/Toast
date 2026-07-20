use crate::app_utils::paths::to_relative;
use crate::app_utils::{delete_img::*, manage_audio::*, manage_img::*};
use crate::crud::scheduling::*;
use rusqlite::{Connection, OptionalExtension, Result};
use std::path::Path;

pub fn delete_plan(id: i64, conn: &mut Connection) -> Result<()> {
    let group_ids: Vec<i64> = conn
        .prepare("SELECT group_id FROM scheduler INNER JOIN \"group\" g ON g.id = scheduler.group_id WHERE g.plan_id = ?1")?
        .query_map([id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    for group_id in group_ids {
        remove_group_from_plan(group_id, false, conn)?;
    }

    conn.execute("DELETE FROM plan WHERE id = ?1", [id])?;

    Ok(())
}

pub fn delete_todo(id: i64, conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    let row: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT plan_id, position FROM todo WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    tx.execute("DELETE FROM todo WHERE id = ?1", [id])?;

    // Close the gap so numbered todos stay contiguous 1..N
    if let Some((plan_id, Some(pos))) = row {
        tx.execute(
            "UPDATE todo SET position = position - 1 WHERE plan_id = ?1 AND position > ?2",
            rusqlite::params![plan_id, pos],
        )?;
    }

    tx.commit()
}

/// Collects (image, audio) file paths embedded in an uploaded card's imported HTML.
fn html_media_paths(
    imported_front: Option<&str>,
    imported_back: Option<&str>,
    imported_support: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let mut images = Vec::new();
    let mut audio = Vec::new();
    for html in [imported_front, imported_back, imported_support]
        .into_iter()
        .flatten()
    {
        images.extend(extract_image_paths_from_html(html));
        audio.extend(extract_audio_paths_from_html(html));
    }
    (images, audio)
}

pub fn delete_card(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    type CardMediaRow = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
        i64,
    );
    let row: CardMediaRow = match conn.query_row(
        r#"
        SELECT front_image, back_image, front_audio, back_audio,
               is_uploaded, imported_front, imported_back, imported_support, is_due, group_id
        FROM card WHERE id = ?1
        "#,
        [id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        },
    ) {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
        Err(e) => return Err(e),
    };
    let (front_image, back_image, front_audio, back_audio, is_uploaded, imported_front, imported_back, imported_support, is_due, group_id) =
        row;

    let (html_images, html_audio) = if is_uploaded {
        html_media_paths(
            imported_front.as_deref(),
            imported_back.as_deref(),
            imported_support.as_deref(),
        )
    } else {
        (vec![], vec![])
    };

    conn.execute("DELETE FROM card WHERE id = ?1", [id])?;

    delete_media_file(app_dir, front_image);
    delete_media_file(app_dir, back_image);
    delete_media_file(app_dir, front_audio);
    delete_media_file(app_dir, back_audio);

    for path in html_images.iter().chain(html_audio.iter()) {
        delete_media_file(app_dir, Some(path.clone()));
    }

    let _ = on_item_removed(group_id, is_due, conn);

    Ok(())
}

pub fn delete_deck(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    let media: Vec<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = {
        let mut stmt = conn.prepare(
            "SELECT front_image, back_image, front_audio, back_audio FROM card WHERE group_id = ?1",
        )?;
        let rows = stmt
            .query_map([id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    let (mut html_images, mut html_audio) = {
        let mut stmt = conn.prepare(
            "SELECT imported_front, imported_back, imported_support FROM card WHERE group_id = ?1 AND is_uploaded = TRUE",
        )?;
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut images = Vec::new();
        let mut audio = Vec::new();
        for (ifront, iback, isupport) in rows {
            let (i, a) =
                html_media_paths(ifront.as_deref(), iback.as_deref(), isupport.as_deref());
            images.extend(i);
            audio.extend(a);
        }
        (images, audio)
    };

    html_images.sort();
    html_images.dedup();

    html_audio.sort();
    html_audio.dedup();

    for path in html_images.iter().chain(html_audio.iter()) {
        delete_media_file(app_dir, Some(path.clone()));
    }

    conn.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'deck'"#,
        [id],
    )?;

    for (fi, bi, fa, ba) in media {
        delete_media_file(app_dir, fi);
        delete_media_file(app_dir, bi);
        delete_media_file(app_dir, fa);
        delete_media_file(app_dir, ba);
    }

    Ok(())
}

pub fn delete_notebook(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    let pages: Vec<(String, Option<String>)> = conn
        .prepare("SELECT content, audio_file FROM page WHERE group_id = ?1")?
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    for (content, audio_file) in pages {
        for path in extract_image_paths(&content) {
            delete_media_file(app_dir, Some(path));
        }
        delete_media_file(app_dir, audio_file);
    }

    conn.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'notebook'"#,
        [id],
    )?;

    Ok(())
}

pub fn delete_page(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    let (content, audio_file): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT content, audio_file FROM page WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((None, None));

    if let Some(c) = content {
        for path in extract_image_paths(&c) {
            delete_media_file(app_dir, Some(path));
        }
    }

    delete_media_file(app_dir, audio_file);

    conn.execute("DELETE FROM page WHERE id = ?1", [id])?;

    Ok(())
}

pub fn remove_group_from_plan(group_id: i64, reset: bool, conn: &mut Connection) -> Result<()> {
    {
        let tx = conn.transaction()?;

        // Drop this plan's empty lines in the current version so add and remove
        // cycles leave nothing behind. The reset marker lives on the deck's version
        // counter, so nothing of meaning can be lost here.
        tx.execute(
            r#"
            DELETE FROM group_stats
            WHERE id IN (
                SELECT gs.id FROM group_stats gs
                INNER JOIN "group" g
                    ON g.id = gs.group_id
                   AND g.plan_id = gs.plan_id
                   AND g.stat_version = gs.version
                WHERE gs.group_id = ?1
                  AND gs.num_new = 0 AND gs.num_promote = 0 AND gs.num_demote = 0
            )
            "#,
            [group_id],
        )?;

        tx.execute("DELETE FROM scheduler WHERE group_id = ?1", [group_id])?;
        tx.execute(
            r#"UPDATE "group" SET plan_id = NULL WHERE id = ?1"#,
            [group_id],
        )?;

        if reset {
            tx.execute(
                r#"UPDATE card SET tier = 0, ease = 0.0, sequence = 0, is_due = FALSE, is_overdue = NULL, is_paused = FALSE WHERE group_id = ?1"#,
                [group_id],
            )?;
        } else {
            tx.execute(
                "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1",
                [group_id],
            )?;
        }

        tx.commit()?;
    } // tx dropped here, releasing the borrow on conn

    if reset {
        reset_deck(group_id, conn)?;
    }

    Ok(())
}

use std::collections::HashSet;
pub fn cleanup_orphaned_media(conn: &Connection, app_dir: &Path) -> Result<usize> {
    // All sets hold app-dir-relative keys ("cards/images/<file>"); stored
    // paths are normalized through to_relative so legacy absolute rows still
    // protect their files.
    let mut referenced_images: HashSet<String> = HashSet::new();
    let mut referenced_audio: HashSet<String> = HashSet::new();
    let mut referenced_page_audio: HashSet<String> = HashSet::new();

    // Custom card images
    for col in &["front_image", "back_image"] {
        let mut stmt = conn.prepare(&format!("SELECT {col} FROM card WHERE {col} IS NOT NULL"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok());
        for p in rows {
            referenced_images.insert(to_relative(&p, app_dir));
        }
    }

    // Custom card audio
    for col in &["front_audio", "back_audio"] {
        let mut stmt = conn.prepare(&format!("SELECT {col} FROM card WHERE {col} IS NOT NULL"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok());
        for p in rows {
            referenced_audio.insert(to_relative(&p, app_dir));
        }
    }

    // Uploaded card HTML
    {
        let mut stmt = conn.prepare(
            "SELECT imported_front, imported_back, imported_support FROM card WHERE is_uploaded = TRUE",
        )?;
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();
        for (ifront, iback, isupport) in rows {
            let (images, audio) =
                html_media_paths(ifront.as_deref(), iback.as_deref(), isupport.as_deref());
            referenced_images.extend(images.iter().map(|p| to_relative(p, app_dir)));
            referenced_audio.extend(audio.iter().map(|p| to_relative(p, app_dir)));
        }
    }

    // Page content images
    {
        let mut stmt = conn.prepare("SELECT content FROM page")?;
        let rows: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        for content in rows {
            for p in extract_image_paths(&content) {
                referenced_images.insert(to_relative(&p, app_dir));
            }
        }
    }

    // Page audio
    {
        let mut stmt = conn.prepare("SELECT audio_file FROM page WHERE audio_file IS NOT NULL")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok());
        for p in rows {
            referenced_page_audio.insert(to_relative(&p, app_dir));
        }
    }

    let mut deleted = 0;

    // Keys are "{subdir}/{filename}" so comparisons don't depend on absolute paths or OS separators.
    // Liveness checks against the union of all sets because a file can be referenced under a different subdir than where it landed.
    let all_referenced: HashSet<&String> = referenced_images
        .iter()
        .chain(referenced_audio.iter())
        .chain(referenced_page_audio.iter())
        .collect();
    let dirs = [
        ("cards/images", &referenced_images),
        ("cards/audio", &referenced_audio),
        ("pages/audio", &referenced_page_audio),
    ];
    for (subdir, referenced) in dirs {
        let dir = app_dir.join(subdir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let files: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();

        let orphans: Vec<&std::path::PathBuf> = files
            .iter()
            .filter(|p| {
                let key = format!(
                    "{subdir}/{}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                );
                !all_referenced.contains(&key)
            })
            .collect();

        // Safety valve: files exist AND references exist, yet not a single
        // file matches a reference. That's a systematic key mismatch (a bug),
        // not real orphans. Deleting here would wipe every media file.
        if !referenced.is_empty() && !files.is_empty() && orphans.len() == files.len() {
            log::error!(
                "cleanup_orphaned_media: refusing to delete all {} files in {subdir}: \
                 no stored reference matches any file, which indicates a path-format bug",
                files.len()
            );
            continue;
        }

        for path in orphans {
            if std::fs::remove_file(path).is_ok() {
                deleted += 1;
            }
        }
    }

    Ok(deleted)
}

pub fn delete_resource(id: i64, conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM resource WHERE id = ?1", [id])?;
    Ok(())
}

pub fn delete_group_stat(id: i64, conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM group_stats WHERE id = ?1", [id])?;
    Ok(())
}

// Clears every version of one deck within a plan, matching the deck level card on
// the stats page. origin_group_id outlives the deck, so two decks sharing a name
// stay independent even after both are gone. Rows predating it fall back to name.
// Passing a version clears just that one, otherwise every version of the deck goes.
pub fn delete_group_stats_for_deck(
    origin_group_id: Option<i64>,
    group_name: &str,
    version: Option<i64>,
    plan_id: i64,
    conn: &Connection,
) -> Result<()> {
    conn.execute(
        "DELETE FROM group_stats
         WHERE plan_id = ?3
           AND (?4 IS NULL OR version = ?4)
           AND CASE WHEN ?1 IS NULL
                    THEN origin_group_id IS NULL AND group_name = ?2
                    ELSE origin_group_id = ?1 END",
        rusqlite::params![origin_group_id, group_name, plan_id, version],
    )?;
    Ok(())
}

pub fn delete_todo_stat(id: i64, conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM todo_stats WHERE id = ?1", [id])?;
    Ok(())
}

pub fn delete_deleted_plan_stats(plan_id: i64, conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM group_stats WHERE plan_id = ?1", [plan_id])?;
    conn.execute("DELETE FROM todo_stats WHERE plan_id = ?1", [plan_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(app_dir: &Path) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, app_dir).unwrap();
        conn.execute(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'g', 'deck')",
            [],
        )
        .unwrap();
        conn
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    #[test]
    fn cleanup_keeps_referenced_media_and_deletes_orphans() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path();
        let conn = setup(app_dir);

        let audio_dir = app_dir.join("cards/audio");
        touch(&audio_dir, "kept-rel.mp3");
        touch(&audio_dir, "kept-abs.mp3");
        touch(&audio_dir, "orphan.mp3");

        // one relative reference, one legacy absolute reference
        let abs = app_dir.join("cards/audio/kept-abs.mp3");
        conn.execute(
            "INSERT INTO card (group_id, front, back, front_audio, back_audio)
             VALUES (1, 'f', 'b', 'cards/audio/kept-rel.mp3', ?1)",
            [abs.to_string_lossy()],
        )
        .unwrap();

        let deleted = cleanup_orphaned_media(&conn, app_dir).unwrap();

        assert_eq!(deleted, 1);
        assert!(audio_dir.join("kept-rel.mp3").exists());
        assert!(audio_dir.join("kept-abs.mp3").exists());
        assert!(!audio_dir.join("orphan.mp3").exists());
    }

    #[test]
    fn cleanup_keeps_media_referenced_as_another_kind() {
        // An <audio> src can point into cards/images when the importer didn't
        // classify the extension as audio; the images walk must not reap it
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path();
        let conn = setup(app_dir);

        touch(&app_dir.join("cards/images"), "clip.xyz");
        conn.execute(
            "INSERT INTO card (group_id, front, back, imported_front, is_uploaded)
             VALUES (1, '', '', '<audio controls src=\"cards/images/clip.xyz\"></audio>', TRUE)",
            [],
        )
        .unwrap();

        let deleted = cleanup_orphaned_media(&conn, app_dir).unwrap();

        assert_eq!(deleted, 0);
        assert!(app_dir.join("cards/images/clip.xyz").exists());
    }

    #[test]
    fn cleanup_refuses_to_wipe_directory_on_systematic_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path();
        let conn = setup(app_dir);

        let audio_dir = app_dir.join("cards/audio");
        touch(&audio_dir, "a.mp3");
        touch(&audio_dir, "b.mp3");

        // references exist but match no file at all, must not delete anything
        conn.execute(
            "INSERT INTO card (group_id, front, back, front_audio)
             VALUES (1, 'f', 'b', 'cards/audio/elsewhere.mp3')",
            [],
        )
        .unwrap();

        let deleted = cleanup_orphaned_media(&conn, app_dir).unwrap();

        assert_eq!(deleted, 0);
        assert!(audio_dir.join("a.mp3").exists());
        assert!(audio_dir.join("b.mp3").exists());
    }

    #[test]
    fn cleanup_deletes_everything_when_nothing_is_referenced() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path();
        let conn = setup(app_dir);

        touch(&app_dir.join("cards/audio"), "a.mp3");

        let deleted = cleanup_orphaned_media(&conn, app_dir).unwrap();
        assert_eq!(deleted, 1);
    }
}
