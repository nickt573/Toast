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

    // Close the gap so numbered todos stay contiguous
    if let Some((plan_id, Some(pos))) = row {
        tx.execute(
            "UPDATE todo SET position = position - 1 WHERE plan_id = ?1 AND position > ?2",
            rusqlite::params![plan_id, pos],
        )?;
    }

    tx.commit()
}

/// Collects the image and audio file paths embedded in an uploaded card's imported HTML
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

    // Drop any page tags on this notebook before its pages go, so no stat bar keeps a number
    // for a page that no longer exists
    conn.execute(
        "UPDATE todo_stat_group SET page_id = NULL WHERE page_id IN (SELECT id FROM page WHERE group_id = ?1)",
        [id],
    )?;

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

    // Migrated databases never got the page FK, so drop the tag by hand. This also guards
    // against SQLite handing the freed rowid to a new page and a stale tag pointing at it
    conn.execute(
        "UPDATE todo_stat_group SET page_id = NULL WHERE page_id = ?1",
        [id],
    )?;

    conn.execute("DELETE FROM page WHERE id = ?1", [id])?;

    Ok(())
}

pub fn remove_group_from_plan(group_id: i64, reset: bool, conn: &mut Connection) -> Result<()> {
    // Reset wipes progress first, while still in-plan, so its marker and refill are unchanged
    if reset {
        reset_deck(group_id, conn)?;
    }

    // Unbind only: scheduler and card flags stay frozen, plan_id now marks it as left
    conn.execute(
        r#"UPDATE "group" SET plan_id = NULL WHERE id = ?1"#,
        [group_id],
    )?;

    Ok(())
}

use std::collections::HashSet;
pub fn cleanup_orphaned_media(conn: &Connection, app_dir: &Path) -> Result<usize> {
    // All sets hold app-dir-relative keys, and stored paths are normalized through
    // to_relative so legacy absolute rows still protect their files
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

    // Keys are the subdir and filename so comparisons don't depend on absolute paths or OS
    // separators, and liveness checks the union since a file can land under another subdir
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

        // Files and references both exist yet none match, which is a systematic key
        // mismatch rather than real orphans, and deleting here would wipe every file
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

/// A reset is kept until the last stat line of its deck is gone, and SQLite has no foreign
/// key for that, so every path that deletes stat lines calls this afterwards
fn sweep_orphan_resets(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM deck_reset
         WHERE origin_group_id NOT IN
             (SELECT origin_group_id FROM group_stats WHERE origin_group_id IS NOT NULL)",
        [],
    )?;
    Ok(())
}

pub fn delete_group_stat(id: i64, conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM group_stats WHERE id = ?1", [id])?;
    sweep_orphan_resets(conn)
}

/// Clears the lines behind one deck card on the stats page, which sends their ids since it
/// is what decides the grouping, so nothing is re-derived from a description here
pub fn delete_group_stats(ids: &[i64], conn: &Connection) -> Result<()> {
    for id in ids {
        conn.execute("DELETE FROM group_stats WHERE id = ?1", [id])?;
    }
    sweep_orphan_resets(conn)
}

pub fn delete_todo_stat(id: i64, conn: &Connection) -> Result<()> {
    // Deleting today's stat unchecks the todo it came from
    let row: Option<(Option<i64>, String)> = conn
        .query_row(
            "SELECT todo_id, date FROM todo_stats WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    conn.execute("DELETE FROM todo_stats WHERE id = ?1", [id])?;
    if let Some((Some(todo_id), date)) = row {
        if date == get_date(conn)? {
            conn.execute("UPDATE todo SET is_done = FALSE WHERE id = ?1", [todo_id])?;
        }
    }
    Ok(())
}

// Deletes a whole unit, clearing it off every entry that counted in any of its names, and
// since an amount can't sit without a unit those entries keep their time but lose the count
pub fn delete_unit(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE todo_stats SET variant_id = NULL, num_value = NULL
         WHERE variant_id IN (SELECT id FROM unit_variant WHERE group_id = ?1)",
        [group_id],
    )?;
    conn.execute("DELETE FROM unit_variant WHERE group_id = ?1", [group_id])?;
    Ok(())
}

// Deletes one name, clearing unit and amount off the entries that chose it, and the last
// name can't leave alone, while removing the main promotes the next lowest-positioned one
pub fn delete_variant(id: i64, conn: &Connection) -> Result<()> {
    let siblings: i64 = conn.query_row(
        "SELECT COUNT(*) FROM unit_variant WHERE group_id = (SELECT group_id FROM unit_variant WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    if siblings <= 1 {
        return Err(rusqlite::Error::InvalidParameterName(
            "a unit needs at least one name".into(),
        ));
    }
    conn.execute(
        "UPDATE todo_stats SET variant_id = NULL, num_value = NULL WHERE variant_id = ?1",
        [id],
    )?;
    conn.execute("DELETE FROM unit_variant WHERE id = ?1", [id])?;
    Ok(())
}

pub fn delete_deleted_plan_stats(plan_id: i64, conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM group_stats WHERE plan_id = ?1", [plan_id])?;
    conn.execute("DELETE FROM todo_stats WHERE plan_id = ?1", [plan_id])?;
    sweep_orphan_resets(conn)
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

    // Units are global, so deleting one clears it off entries in every plan, and the amount
    // goes with it since a count can't sit unit-less
    #[test]
    fn deleting_a_unit_clears_it_from_entries_in_every_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, tmp.path()).unwrap();

        conn.execute("INSERT INTO plan (name) VALUES ('plan A')", []).unwrap();
        let plan_a = conn.last_insert_rowid();
        conn.execute("INSERT INTO plan (name) VALUES ('plan B')", []).unwrap();
        let plan_b = conn.last_insert_rowid();

        let group = crate::crud::update::create_unit(vec!["page".into(), "pages".into()], &conn).unwrap();
        let main_variant: i64 = conn
            .query_row("SELECT id FROM unit_variant WHERE group_id = ?1 ORDER BY position LIMIT 1", [group], |r| r.get(0))
            .unwrap();
        let alt: i64 = conn
            .query_row("SELECT id FROM unit_variant WHERE group_id = ?1 AND id != ?2", [group, main_variant], |r| r.get(0))
            .unwrap();
        // Logged under both plans, one against each name
        for (plan, variant) in [(plan_a, main_variant), (plan_b, alt)] {
            conn.execute(
                "INSERT INTO todo_stats (plan_id, date, text, category, time_spent_minutes, num_value, variant_id)
                 VALUES (?1, '2026-01-01', 't', 'Reading', 30, 5, ?2)",
                [plan, variant],
            )
            .unwrap();
        }

        // Deleting one name clears only the entries that chose it, keeping their time
        delete_variant(alt, &conn).unwrap();
        let cleared: i64 = conn
            .query_row("SELECT COUNT(*) FROM todo_stats WHERE variant_id IS NULL AND num_value IS NULL AND time_spent_minutes = 30", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cleared, 1, "the alt's entry is cleared but keeps its time");

        // Deleting the whole unit clears the rest and removes the group entirely
        delete_unit(group, &conn).unwrap();
        let variants: i64 = conn.query_row("SELECT COUNT(*) FROM unit_variant WHERE group_id = ?1", [group], |r| r.get(0)).unwrap();
        assert_eq!(variants, 0, "the unit is gone");
        let still_linked: i64 = conn.query_row("SELECT COUNT(*) FROM todo_stats WHERE variant_id IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(still_linked, 0, "no entry still points at a deleted unit");
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
        // An audio src can point into the images folder when the importer didn't classify
        // the extension as audio, so the images walk must not reap it
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
