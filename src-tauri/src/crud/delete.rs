use crate::app_utils::{delete_audio::*, delete_img::*, manage_audio::*, manage_img::*};
use crate::crud::scheduling::*;
use rusqlite::{Connection, Result};
use std::path::Path;

pub fn delete_plan(id: i64, conn: &mut Connection) -> Result<()> {
    // Get all groups assigned to this plan before deleting
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

pub fn delete_todo(id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        DELETE FROM todo
        WHERE id = ?1
        "#,
        [id],
    )?;

    Ok(())
}

/// Collects (image, audio) file paths embedded in an uploaded card's front/back HTML.
fn html_media_paths(front: &str, back: &str) -> (Vec<String>, Vec<String>) {
    let mut images = extract_image_paths_from_html(front);
    images.extend(extract_image_paths_from_html(back));
    let mut audio = extract_audio_paths_from_html(front);
    audio.extend(extract_audio_paths_from_html(back));
    (images, audio)
}

pub fn delete_card(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    type CardMediaRow = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
        String,
        String,
        bool,
        i64,
    );
    let row: CardMediaRow = match conn.query_row(
        r#"
        SELECT front_image, back_image, front_audio, back_audio,
               is_uploaded, front, back, is_due, group_id
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
            ))
        },
    ) {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
        Err(e) => return Err(e),
    };
    let (front_image, back_image, front_audio, back_audio, is_uploaded, front, back, is_due, group_id) =
        row;

    // For uploaded cards, also collect images and audio embedded in HTML
    let (html_images, html_audio) = if is_uploaded {
        html_media_paths(&front, &back)
    } else {
        (vec![], vec![])
    };

    conn.execute("DELETE FROM card WHERE id = ?1", [id])?;

    delete_media_file(app_dir, front_image);
    delete_media_file(app_dir, back_image);
    delete_card_audio_file(front_audio);
    delete_card_audio_file(back_audio);

    for path in html_images.iter().chain(html_audio.iter()) {
        let _ = std::fs::remove_file(path);
    }

    let _ = on_item_removed(group_id, is_due, conn);

    Ok(())
}

pub fn delete_deck(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    // Collect per-side media for custom cards
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

    // Collect HTML-embedded images and audio from uploaded cards
    let (mut html_images, mut html_audio) = {
        let mut stmt = conn
            .prepare("SELECT front, back FROM card WHERE group_id = ?1 AND is_uploaded = TRUE")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut images = Vec::new();
        let mut audio = Vec::new();
        for (front, back) in rows {
            let (i, a) = html_media_paths(&front, &back);
            images.extend(i);
            audio.extend(a);
        }
        (images, audio)
    };

    // After collecting, deduplicate
    html_images.sort();
    html_images.dedup();

    html_audio.sort();
    html_audio.dedup();

    for path in html_images.iter().chain(html_audio.iter()) {
        let _ = std::fs::remove_file(path);
    }

    conn.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'deck'"#,
        [id],
    )?;

    for (fi, bi, fa, ba) in media {
        delete_media_file(app_dir, fi);
        delete_media_file(app_dir, bi);
        delete_card_audio_file(fa);
        delete_card_audio_file(ba);
    }

    Ok(())
}

pub fn delete_notebook(id: i64, conn: &Connection, app_dir: &Path) -> Result<()> {
    // Fetch all page content and audio files before cascade delete
    let pages: Vec<(String, Option<String>)> = conn
        .prepare("SELECT content, audio_file FROM page WHERE group_id = ?1")?
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    for (content, audio_file) in pages {
        for path in extract_image_paths(&content) {
            delete_media_file(app_dir, Some(path));
        }
        if let Some(audio) = audio_file {
            let _ = std::fs::remove_file(&audio);
        }
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

    if let Some(audio) = audio_file {
        let _ = std::fs::remove_file(&audio);
    }

    conn.execute("DELETE FROM page WHERE id = ?1", [id])?;

    Ok(())
}

pub fn remove_group_from_plan(group_id: i64, reset: bool, conn: &mut Connection) -> Result<()> {
    {
        let tx = conn.transaction()?;

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
    let mut referenced_images: HashSet<String> = HashSet::new();
    let mut referenced_audio: HashSet<String> = HashSet::new();
    let mut referenced_page_audio: HashSet<String> = HashSet::new();

    // ── Custom card front/back images ─────────────────────────────────────────
    for col in &["front_image", "back_image"] {
        let mut stmt = conn.prepare(&format!("SELECT {col} FROM card WHERE {col} IS NOT NULL"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok());
        for p in rows {
            referenced_images.insert(p);
        }
    }

    // ── Custom card front/back audio ──────────────────────────────────────────
    for col in &["front_audio", "back_audio"] {
        let mut stmt = conn.prepare(&format!("SELECT {col} FROM card WHERE {col} IS NOT NULL"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok());
        for p in rows {
            referenced_audio.insert(p);
        }
    }

    // ── Uploaded card HTML (images and audio embedded in front/back) ─────────
    {
        let mut stmt = conn.prepare("SELECT front, back FROM card WHERE is_uploaded = TRUE")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        for (front, back) in rows {
            let (images, audio) = html_media_paths(&front, &back);
            referenced_images.extend(images);
            referenced_audio.extend(audio);
        }
    }

    // ── Page content (embedded images via TipTap JSON) ────────────────────────
    {
        let mut stmt = conn.prepare("SELECT content FROM page")?;
        let rows: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        for content in rows {
            for p in extract_image_paths(&content) {
                referenced_images.insert(p);
            }
        }
    }

    // ── Page audio_file ───────────────────────────────────────────────────────
    {
        let mut stmt = conn.prepare("SELECT audio_file FROM page WHERE audio_file IS NOT NULL")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok());
        for p in rows {
            referenced_page_audio.insert(p);
        }
    }

    let mut deleted = 0;

    // ── Delete orphaned card images ───────────────────────────────────────────
    let img_dir = app_dir.join("cards").join("images");
    if let Ok(entries) = std::fs::read_dir(&img_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            if !referenced_images.contains(&path_str) {
                if std::fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
            }
        }
    }

    // ── Delete orphaned card audio ────────────────────────────────────────────
    let aud_dir = app_dir.join("cards").join("audio");
    if let Ok(entries) = std::fs::read_dir(&aud_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            if !referenced_audio.contains(&path_str) {
                if std::fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
            }
        }
    }

    // ── Delete orphaned page audio ────────────────────────────────────────────
    let page_aud_dir = app_dir.join("pages").join("audio");
    if let Ok(entries) = std::fs::read_dir(&page_aud_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            if !referenced_page_audio.contains(&path_str) {
                if std::fs::remove_file(&path).is_ok() {
                    deleted += 1;
                }
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

pub fn delete_group_stats_for_deck(
    group_name: &str,
    plan_id: i64,
    conn: &Connection,
) -> Result<()> {
    conn.execute(
        "DELETE FROM group_stats WHERE group_name = ?1 AND plan_id = ?2",
        rusqlite::params![group_name, plan_id],
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
