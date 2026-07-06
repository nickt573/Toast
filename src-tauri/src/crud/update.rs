use crate::app_utils::{delete_img::*, manage_img::*, save_audio::*, save_img::*};
use crate::crud::{models::*, scheduling::*};
use chrono::Datelike;
use rusqlite::{Connection, Result};
use std::path::Path;

/// Snapshot a resource's full info (name/url/type/notes) into a todo_stats log row.
/// The snapshot persists after the resource is deleted; live values override it via
/// COALESCE in the read query while the resource still exists.
fn insert_stat_resource(stat_id: i64, resource_id: i64, conn: &Connection) -> Result<()> {
    let snap: (String, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT name, url, \"type\", notes FROM resource WHERE id = ?1",
            [resource_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or_default();
    conn.execute(
        "INSERT INTO todo_stat_resource (stat_id, resource_id, resource_name, resource_url, resource_type, resource_notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![stat_id, resource_id, snap.0, snap.1, snap.2, snap.3],
    )?;
    Ok(())
}

pub fn update_plan(id: i64, name: String, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE plan SET name = ?1 WHERE id = ?2",
        rusqlite::params![name, id],
    )?;
    conn.execute(
        "UPDATE group_stats SET plan_name = ?1 WHERE plan_id = ?2",
        rusqlite::params![name, id],
    )?;
    conn.execute(
        "UPDATE todo_stats SET plan_name = ?1 WHERE plan_id = ?2",
        rusqlite::params![name, id],
    )?;
    Ok(())
}

pub fn update_todo(todo: Todo, conn: &Connection) -> Result<()> {
    // Recalculate is_disabled based on today's weekday
    let date_str = get_date(conn)?;
    let today = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?
        .weekday()
        .num_days_from_sunday();
    let today_bit = 1i64 << today;
    let is_disabled = (todo.frequency & today_bit) == 0;

    conn.execute(
        r#"
        UPDATE todo
        SET text = ?1, frequency = ?2, category = ?3, is_done = ?4, is_disabled = ?5
        WHERE id = ?6
        "#,
        rusqlite::params![
            todo.text,
            todo.frequency,
            todo.category,
            todo.is_done,
            is_disabled,
            todo.id
        ],
    )?;
    Ok(())
}

pub fn update_deck(deck: Group, conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        UPDATE "group"
        SET
            name = ?1
        WHERE id = ?2
        "#,
        rusqlite::params![deck.name, deck.id],
    )?;

    conn.execute(
        "UPDATE group_stats SET group_name = ?1 WHERE group_id = ?2",
        rusqlite::params![deck.name, deck.id],
    )?;

    conn.execute(
        "UPDATE todo_stat_group SET group_name = ?1 WHERE group_id = ?2",
        rusqlite::params![deck.name, deck.id],
    )?;

    Ok(())
}

pub fn update_card(card: Card, conn: &Connection, app_dir: &Path) -> Result<()> {
    let (old_paused, old_is_due, old_front_image, old_back_image, old_front_audio, old_back_audio):
        (bool, bool, Option<String>, Option<String>, Option<String>, Option<String>) = conn.query_row(
        "SELECT is_paused, is_due, front_image, back_image, front_audio, back_audio FROM card WHERE id = ?1",
        [card.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).unwrap_or((false, false, None, None, None, None));

    let new_front_image = if card.front_image != old_front_image {
        if old_front_image.is_some() {
            delete_media_file(app_dir, old_front_image);
        }
        save_card_image(card.front_image.clone(), app_dir)?
    } else {
        card.front_image.clone()
    };

    let new_back_image = if card.back_image != old_back_image {
        if old_back_image.is_some() {
            delete_media_file(app_dir, old_back_image);
        }
        save_card_image(card.back_image.clone(), app_dir)?
    } else {
        card.back_image.clone()
    };

    let new_front_audio = if card.front_audio != old_front_audio {
        if let Some(ref p) = old_front_audio {
            let _ = std::fs::remove_file(p);
        }
        save_card_audio_file(card.front_audio.clone(), app_dir)?
    } else {
        card.front_audio.clone()
    };

    let new_back_audio = if card.back_audio != old_back_audio {
        if let Some(ref p) = old_back_audio {
            let _ = std::fs::remove_file(p);
        }
        save_card_audio_file(card.back_audio.clone(), app_dir)?
    } else {
        card.back_audio.clone()
    };

    // imported_support is intentionally never updated: it is read-only content
    // set once at Anki import time (the user-editable slot is `support`).
    conn.execute(
        r#"
        UPDATE card SET
            group_id = ?1, front = ?2, back = ?3,
            is_searchable = ?4, support = ?5,
            front_image = ?6, back_image = ?7,
            front_audio = ?8, back_audio = ?9,
            is_paused = ?10
        WHERE id = ?11
        "#,
        rusqlite::params![
            card.group_id,
            card.front,
            card.back,
            card.is_searchable,
            card.support,
            new_front_image,
            new_back_image,
            new_front_audio,
            new_back_audio,
            card.is_paused,
            card.id
        ],
    )?;

    if card.is_paused != old_paused {
        on_pause_changed(card.id, card.group_id, card.is_paused, old_is_due, conn)?;
    }

    Ok(())
}

pub fn set_all_searchable(group_id: i64, searchable: bool, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET is_searchable = ?1 WHERE group_id = ?2",
        rusqlite::params![searchable, group_id],
    )?;
    Ok(())
}

pub fn update_notebook(notebook: Group, conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        UPDATE "group"
        SET name = ?1
        WHERE id = ?2
          AND group_type = 'notebook'
        "#,
        rusqlite::params![notebook.name, notebook.id],
    )?;

    conn.execute(
        "UPDATE todo_stat_group SET group_name = ?1 WHERE group_id = ?2",
        rusqlite::params![notebook.name, notebook.id],
    )?;

    Ok(())
}

pub fn update_page(page: Page, conn: &Connection, app_dir: &Path) -> Result<()> {
    let (old_content, old_audio): (String, Option<String>) = conn.query_row(
        "SELECT content, audio_file FROM page WHERE id = ?1",
        [page.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let new_content = rewrite_images_in_content(&page.content, app_dir)?;

    if old_audio != page.audio_file {
        if let Some(old_path) = &old_audio {
            let _ = std::fs::remove_file(old_path);
        }
    }

    for path in removed_image_paths(&old_content, &new_content) {
        delete_media_file(app_dir, Some(path));
    }

    conn.execute(
        r#"
        UPDATE page
        SET title = ?1, description = ?2, content = ?3, audio_file = ?4
        WHERE id = ?5
        "#,
        rusqlite::params![
            page.title,
            page.description,
            new_content,
            page.audio_file,
            page.id
        ],
    )?;

    Ok(())
}

pub fn update_resource(resource: Resource, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE resource SET name = ?1, type = ?2, url = ?3, notes = ?4 WHERE id = ?5",
        rusqlite::params![
            resource.name,
            resource.resource_type,
            resource.url,
            resource.notes,
            resource.id
        ],
    )?;
    // Keep the snapshot in sync with the current resource info
    conn.execute(
        "UPDATE todo_stat_resource SET resource_name = ?1, resource_url = ?2, resource_type = ?3, resource_notes = ?4 WHERE resource_id = ?5",
        rusqlite::params![resource.name, resource.url, resource.resource_type, resource.notes, resource.id],
    )?;
    Ok(())
}

pub fn set_todo_resources(
    todo_id: i64,
    resource_ids: Vec<i64>,
    conn: &mut Connection,
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM todo_resource WHERE todo_id = ?1", [todo_id])?;
    for resource_id in &resource_ids {
        tx.execute(
            "INSERT INTO todo_resource (todo_id, resource_id) VALUES (?1, ?2)",
            rusqlite::params![todo_id, resource_id],
        )?;
    }
    tx.commit()
}

pub fn set_todo_groups(todo_id: i64, group_ids: Vec<i64>, conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM todo_group WHERE todo_id = ?1", [todo_id])?;
    for group_id in &group_ids {
        tx.execute(
            "INSERT INTO todo_group (todo_id, group_id) VALUES (?1, ?2)",
            rusqlite::params![todo_id, group_id],
        )?;
    }
    tx.commit()
}

pub fn complete_todo(
    todo_id: i64,
    time_spent_minutes: f64,
    num_unit: Option<String>,
    details: Option<String>,
    resource_ids: Vec<i64>,
    group_ids: Vec<i64>,
    category: i64,
    conn: &Connection,
) -> Result<()> {
    let today = get_date(&conn)?;

    let (plan_id, text): (i64, String) = conn.query_row(
        "SELECT plan_id, text FROM todo WHERE id = ?1",
        [todo_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    if category == 0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "category required".into(),
        ));
    }
    if time_spent_minutes < 0.0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "time_spent must be >= 0".into(),
        ));
    }

    let category_str = category_mask_to_string(category);
    let plan_name: String = conn
        .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
            r.get(0)
        })
        .unwrap_or_default();

    conn.execute(
        r#"
        INSERT INTO todo_stats (todo_id, plan_id, plan_name, date, text, category, details, time_spent_minutes, num_unit)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        rusqlite::params![todo_id, plan_id, plan_name, &today, text, category_str, details, time_spent_minutes, num_unit],
    )?;

    let stat_id = conn.last_insert_rowid();

    for group_id in &group_ids {
        let (group_name, group_type): (String, String) = conn
            .query_row(
                r#"SELECT name, group_type FROM "group" WHERE id = ?1"#,
                [group_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO todo_stat_group (stat_id, group_id, group_name, group_type) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![stat_id, group_id, group_name, group_type],
        )?;
    }

    for resource_id in &resource_ids {
        insert_stat_resource(stat_id, *resource_id, conn)?;
    }

    conn.execute("UPDATE todo SET is_done = TRUE WHERE id = ?1", [todo_id])?;

    Ok(())
}

pub fn log_free_todo(
    plan_id: i64,
    text: String,
    category: i64,
    details: Option<String>,
    time_spent_minutes: f64,
    num_unit: Option<String>,
    group_ids: Vec<i64>,
    resource_ids: Vec<i64>,
    conn: &Connection,
) -> Result<()> {
    if category == 0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "category required".into(),
        ));
    }
    if time_spent_minutes < 0.0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "time_spent must be >= 0".into(),
        ));
    }

    let today = get_date(&conn)?;
    let category_str = category_mask_to_string(category);
    let plan_name: String = conn
        .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
            r.get(0)
        })
        .unwrap_or_default();

    conn.execute(
        r#"
        INSERT INTO todo_stats (todo_id, plan_id, plan_name, date, text, category, details, time_spent_minutes, num_unit)
        VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        rusqlite::params![plan_id, plan_name, &today, text, category_str, details, time_spent_minutes, num_unit],
    )?;

    let stat_id = conn.last_insert_rowid();

    for group_id in &group_ids {
        let (group_name, group_type): (String, String) = conn
            .query_row(
                r#"SELECT name, group_type FROM "group" WHERE id = ?1"#,
                [group_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO todo_stat_group (stat_id, group_id, group_name, group_type) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![stat_id, group_id, group_name, group_type],
        )?;
    }

    for resource_id in &resource_ids {
        insert_stat_resource(stat_id, *resource_id, conn)?;
    }

    Ok(())
}

pub fn uncomplete_todo(todo_id: i64, conn: &Connection) -> Result<()> {
    let today = get_date(&conn)?;
    conn.execute(
        "DELETE FROM todo_stats WHERE todo_id = ?1 AND date = ?2",
        rusqlite::params![todo_id, &today],
    )?;
    conn.execute("UPDATE todo SET is_done = FALSE WHERE id = ?1", [todo_id])?;
    Ok(())
}

pub fn update_todo_stat(
    id: i64,
    text: String,
    category: i64,
    details: Option<String>,
    time_spent_minutes: f64,
    num_unit: Option<String>,
    remove_group_names: Vec<String>,
    remove_resource_names: Vec<String>,
    conn: &Connection,
) -> Result<()> {
    if category == 0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "category required".into(),
        ));
    }
    if time_spent_minutes < 0.0 {
        return Err(rusqlite::Error::InvalidParameterName(
            "time_spent must be >= 0".into(),
        ));
    }
    let category_str = category_mask_to_string(category);
    conn.execute(
        "UPDATE todo_stats SET text=?1, category=?2, details=?3, time_spent_minutes=?4, num_unit=?5 WHERE id=?6",
        rusqlite::params![text, category_str, details, time_spent_minutes, num_unit, id],
    )?;
    for name in &remove_group_names {
        conn.execute(
            "DELETE FROM todo_stat_group WHERE stat_id=?1 AND group_name=?2",
            rusqlite::params![id, name],
        )?;
    }
    for name in &remove_resource_names {
        conn.execute(
            "DELETE FROM todo_stat_resource WHERE stat_id=?1 AND resource_name=?2",
            rusqlite::params![id, name],
        )?;
    }
    Ok(())
}

fn category_mask_to_string(mask: i64) -> String {
    let categories = [
        (1, "Reading"),
        (2, "Writing"),
        (4, "Speaking"),
        (8, "Listening"),
        (16, "Vocabulary"),
        (32, "Grammar"),
        (64, "Culture"),
    ];
    let parts: Vec<&str> = categories
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    parts.join(", ")
}
