use crate::app_utils::{delete_img::*, manage_img::*, save_audio::*, save_img::*};
use crate::crud::{models::*, scheduling::*};
use chrono::Datelike;
use rusqlite::{Connection, Result};
use std::path::Path;

/// Snapshot a resource's full info into a todo_stats log row, kept after the resource is
/// deleted while live values override it through COALESCE in the read query
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

// Archiving keeps a stat line visible but drops it from every total, chart and streak,
// and a merge archives its copies automatically, so this is the manual toggle
pub fn set_group_stat_archived(id: i64, archived: bool, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE group_stats SET is_archived = ?2 WHERE id = ?1",
        rusqlite::params![id, archived],
    )?;
    Ok(())
}

/// Archives or restores the lines behind one deck card, addressed by id the same way
/// delete_group_stats is
pub fn set_group_stats_archived(ids: &[i64], archived: bool, conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("UPDATE group_stats SET is_archived = ?2 WHERE id = ?1")?;
    for id in ids {
        stmt.execute(rusqlite::params![id, archived])?;
    }
    Ok(())
}

pub fn set_plan_disabled(id: i64, disabled: bool, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE plan SET is_disabled = ?1 WHERE id = ?2",
        rusqlite::params![disabled, id],
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

    // Changing the frequency drops the skip, so disabling and re-enabling a day starts it
    // fresh instead of coming back still skipped
    let (old_frequency, old_skipped): (i64, bool) = conn.query_row(
        "SELECT frequency, is_skipped FROM todo WHERE id = ?1",
        [todo.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let is_skipped = old_skipped && old_frequency == todo.frequency;
    let is_disabled = (todo.frequency & today_bit) == 0 || is_skipped;

    conn.execute(
        r#"
        UPDATE todo
        SET text = ?1, frequency = ?2, category = ?3, is_done = ?4, is_disabled = ?5, is_skipped = ?6
        WHERE id = ?7
        "#,
        rusqlite::params![
            todo.text,
            todo.frequency,
            todo.category,
            todo.is_done,
            is_disabled,
            is_skipped,
            todo.id
        ],
    )?;
    Ok(())
}

pub fn set_todo_skipped(todo_id: i64, skipped: bool, conn: &Connection) -> Result<()> {
    let date_str = get_date(conn)?;
    let today = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?
        .weekday()
        .num_days_from_sunday();
    let today_bit = 1i64 << today;

    conn.execute(
        "UPDATE todo SET is_skipped = ?1, is_disabled = ((frequency & ?2) = 0) OR ?1 WHERE id = ?3",
        rusqlite::params![skipped, today_bit, todo_id],
    )?;
    Ok(())
}

/// Sets or clears a todo's manual order, keeping numbered todos contiguous within the plan
/// by pulling the todo out, compacting the gap, then reinserting and shifting later ones up
pub fn set_todo_position(todo_id: i64, position: Option<i64>, conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    let (plan_id, old_pos): (i64, Option<i64>) = tx.query_row(
        "SELECT plan_id, position FROM todo WHERE id = ?1",
        [todo_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    if let Some(old) = old_pos {
        tx.execute(
            "UPDATE todo SET position = position - 1 WHERE plan_id = ?1 AND position > ?2",
            rusqlite::params![plan_id, old],
        )?;
    }

    let new_pos = match position {
        None => None,
        Some(p) => {
            let numbered: i64 = tx.query_row(
                "SELECT COUNT(*) FROM todo WHERE plan_id = ?1 AND position IS NOT NULL AND id != ?2",
                rusqlite::params![plan_id, todo_id],
                |row| row.get(0),
            )?;
            let p = p.clamp(1, numbered + 1);
            tx.execute(
                "UPDATE todo SET position = position + 1 WHERE plan_id = ?1 AND position >= ?2 AND id != ?3",
                rusqlite::params![plan_id, p, todo_id],
            )?;
            Some(p)
        }
    };

    tx.execute(
        "UPDATE todo SET position = ?1 WHERE id = ?2",
        rusqlite::params![new_pos, todo_id],
    )?;

    tx.commit()
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
        delete_media_file(app_dir, old_front_audio);
        save_card_audio_file(card.front_audio.clone(), app_dir)?
    } else {
        card.front_audio.clone()
    };

    let new_back_audio = if card.back_audio != old_back_audio {
        delete_media_file(app_dir, old_back_audio);
        save_card_audio_file(card.back_audio.clone(), app_dir)?
    } else {
        card.back_audio.clone()
    };

    // The imported columns are read-only content set at import time and never updated
    // here, the user-editable slots are front, back and support
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

/// Flips one card's pause without touching the rest of it, which update_card would, and
/// on_pause_changed refills the queue so something eligible takes a freed slot
pub fn set_card_paused(card_id: i64, paused: bool, conn: &Connection) -> Result<()> {
    let (group_id, was_paused, was_due): (i64, bool, bool) = conn.query_row(
        "SELECT group_id, is_paused, is_due FROM card WHERE id = ?1",
        [card_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if was_paused == paused {
        return Ok(());
    }
    conn.execute(
        "UPDATE card SET is_paused = ?1 WHERE id = ?2",
        rusqlite::params![paused, card_id],
    )?;
    on_pause_changed(card_id, group_id, paused, was_due, conn)
}

/// Pauses a card mid-session and says whether the freed slot pulled a replacement in, by
/// counting the rest of the deck either side of the pause with the swapped card left out
pub fn swap_card(card_id: i64, conn: &Connection) -> Result<bool> {
    let group_id: i64 = conn.query_row(
        "SELECT group_id FROM card WHERE id = ?1",
        [card_id],
        |r| r.get(0),
    )?;
    let others = |conn: &Connection| -> Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM card
             WHERE group_id = ?1 AND id != ?2 AND is_due = TRUE AND is_paused = FALSE",
            rusqlite::params![group_id, card_id],
            |r| r.get(0),
        )
    };
    let before = others(conn)?;
    set_card_paused(card_id, true, conn)?;
    Ok(others(conn)? > before)
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
        delete_media_file(app_dir, old_audio.clone());
    }

    for path in removed_image_paths(&old_content, &new_content, app_dir) {
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
    num_value: Option<f64>,
    variant_id: Option<i64>,
    details: Option<String>,
    resource_ids: Vec<i64>,
    group_ids: Vec<i64>,
    category: i64,
    text_override: Option<String>,
    conn: &Connection,
) -> Result<()> {
    let today = get_date(&conn)?;

    let (plan_id, text): (i64, String) = conn.query_row(
        "SELECT plan_id, text FROM todo WHERE id = ?1",
        [todo_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    // Only the logged entry uses the override, the todo keeps its name
    let text = match text_override.map(|t| t.trim().to_string()) {
        Some(t) if !t.is_empty() => t,
        _ => text,
    };

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
    require_unit_pairing(num_value, variant_id)?;
    // Todo time is stored as whole minutes, the column stays FLOAT
    let time_spent_minutes = time_spent_minutes.round();

    let category_str = category_mask_to_string(category);
    let plan_name: String = conn
        .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
            r.get(0)
        })
        .unwrap_or_default();

    conn.execute(
        r#"
        INSERT INTO todo_stats (todo_id, plan_id, plan_name, date, text, category, details, time_spent_minutes, num_value, variant_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        rusqlite::params![todo_id, plan_id, plan_name, &today, text, category_str, details, time_spent_minutes, num_value, variant_id],
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
    num_value: Option<f64>,
    variant_id: Option<i64>,
    group_ids: Vec<i64>,
    resource_ids: Vec<i64>,
    date: Option<String>,
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
    require_unit_pairing(num_value, variant_id)?;
    // Todo time is stored as whole minutes, the column stays FLOAT
    let time_spent_minutes = time_spent_minutes.round();

    let app_today = get_date(&conn)?;
    let today = match date {
        Some(d) => {
            chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
                .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
            if d > app_today {
                return Err(rusqlite::Error::InvalidParameterName(
                    "date cannot be in the future".into(),
                ));
            }
            d
        }
        None => app_today,
    };
    let category_str = category_mask_to_string(category);
    let plan_name: String = conn
        .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
            r.get(0)
        })
        .unwrap_or_default();

    conn.execute(
        r#"
        INSERT INTO todo_stats (todo_id, plan_id, plan_name, date, text, category, details, time_spent_minutes, num_value, variant_id)
        VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        rusqlite::params![plan_id, plan_name, &today, text, category_str, details, time_spent_minutes, num_value, variant_id],
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
    date: String,
    text: String,
    category: i64,
    details: Option<String>,
    time_spent_minutes: f64,
    num_value: Option<f64>,
    variant_id: Option<i64>,
    remove_group_row_ids: Vec<i64>,
    remove_resource_row_ids: Vec<i64>,
    add_group_ids: Vec<i64>,
    add_resource_ids: Vec<i64>,
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
    if date.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName("date required".into()));
    }
    // Nothing may be logged past the day the app is on, since the streak walks back from
    // today and the charts end there, so a future entry is history nobody can see
    if date > get_date(conn)? {
        return Err(rusqlite::Error::InvalidParameterName(
            "date can't be in the future".into(),
        ));
    }
    require_unit_pairing(num_value, variant_id)?;
    // One transaction for the whole edit, since re-dating moves the row to a new id and a
    // failure partway would leave the entry at that new id with the old text and time
    let tx = conn.unchecked_transaction()?;
    let conn = &tx;

    let old_date: String = conn.query_row("SELECT date FROM todo_stats WHERE id=?1", [id], |r| {
        r.get(0)
    })?;
    // Entries within a day are ordered by id, so handing a re-dated one the next id past
    // every other row drops it at the bottom of the day it moved to
    let id = if old_date == date {
        id
    } else {
        // Both join tables point at this id, and their foreign keys are checked statement
        // by statement, so the children can only follow the parent in here
        conn.execute_batch("PRAGMA defer_foreign_keys = ON")?;
        let new_id: i64 =
            conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM todo_stats", [], |r| {
                r.get(0)
            })?;
        conn.execute(
            "UPDATE todo_stats SET id=?1, date=?2 WHERE id=?3",
            rusqlite::params![new_id, date, id],
        )?;
        conn.execute(
            "UPDATE todo_stat_group SET stat_id=?1 WHERE stat_id=?2",
            rusqlite::params![new_id, id],
        )?;
        conn.execute(
            "UPDATE todo_stat_resource SET stat_id=?1 WHERE stat_id=?2",
            rusqlite::params![new_id, id],
        )?;
        new_id
    };
    // Todo time is stored as whole minutes, the column stays FLOAT
    let time_spent_minutes = time_spent_minutes.round();
    let category_str = category_mask_to_string(category);
    conn.execute(
        "UPDATE todo_stats SET text=?1, category=?2, details=?3, time_spent_minutes=?4, num_value=?5, variant_id=?6 WHERE id=?7",
        rusqlite::params![text, category_str, details, time_spent_minutes, num_value, variant_id, id],
    )?;
    // By rowid, never by name, since the snapshot keeps whatever the group or resource was
    // called when it was logged, and the stat_id guard keeps a stray id out of another entry
    for row_id in &remove_group_row_ids {
        conn.execute(
            "DELETE FROM todo_stat_group WHERE stat_id=?1 AND rowid=?2",
            rusqlite::params![id, row_id],
        )?;
    }
    for row_id in &remove_resource_row_ids {
        conn.execute(
            "DELETE FROM todo_stat_resource WHERE stat_id=?1 AND rowid=?2",
            rusqlite::params![id, row_id],
        )?;
    }
    // Only live groups and resources can be added, since the snapshot is pulled from the
    // source row and the SELECT matches nothing for deleted ids
    for group_id in &add_group_ids {
        conn.execute(
            r#"
            INSERT INTO todo_stat_group (stat_id, group_id, group_name, group_type)
            SELECT ?1, g.id, g.name, g.group_type FROM "group" g
            WHERE g.id = ?2
              AND NOT EXISTS (SELECT 1 FROM todo_stat_group WHERE stat_id = ?1 AND group_id = ?2)
            "#,
            rusqlite::params![id, group_id],
        )?;
    }
    for resource_id in &add_resource_ids {
        conn.execute(
            r#"
            INSERT INTO todo_stat_resource (stat_id, resource_id, resource_name, resource_url, resource_type, resource_notes)
            SELECT ?1, r.id, r.name, r.url, r."type", r.notes FROM resource r
            WHERE r.id = ?2
              AND NOT EXISTS (SELECT 1 FROM todo_stat_resource WHERE stat_id = ?1 AND resource_id = ?2)
            "#,
            rusqlite::params![id, resource_id],
        )?;
    }
    tx.commit()
}

// A logged amount and its unit travel together, both filled or both empty, since one
// without the other is a number with no meaning or a unit counting nothing
fn require_unit_pairing(num_value: Option<f64>, variant_id: Option<i64>) -> Result<()> {
    if num_value.is_some() != variant_id.is_some() {
        return Err(rusqlite::Error::InvalidParameterName(
            "a unit and its amount must both be set or both be empty".into(),
        ));
    }
    // Counting zero of something isn't a record worth keeping, so a zero amount is refused
    if matches!(num_value, Some(v) if v <= 0.0) {
        return Err(rusqlite::Error::InvalidParameterName(
            "a unit amount must be more than 0".into(),
        ));
    }
    Ok(())
}

// Starts a new unit from its spellings, the first as the main, whose id names the group
// every variant carries, so the grouping holds after that spelling is renamed or removed
pub fn create_unit(names: Vec<String>, conn: &Connection) -> Result<i64> {
    let cleaned: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let (main, alts) = cleaned.split_first().ok_or_else(|| {
        rusqlite::Error::InvalidParameterName("unit name required".into())
    })?;
    conn.execute(
        "INSERT INTO unit_variant (group_id, name, position) VALUES (0, ?1, 0)",
        rusqlite::params![main],
    )?;
    let group_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE unit_variant SET group_id = ?1 WHERE id = ?1",
        rusqlite::params![group_id],
    )?;
    for (i, alt) in alts.iter().enumerate() {
        conn.execute(
            "INSERT INTO unit_variant (group_id, name, position) VALUES (?1, ?2, ?3)",
            rusqlite::params![group_id, alt, (i as i64) + 1],
        )?;
    }
    Ok(group_id)
}

pub fn add_variant(group_id: i64, name: &str, conn: &Connection) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "unit name required".into(),
        ));
    }
    let next: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM unit_variant WHERE group_id = ?1",
        [group_id],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO unit_variant (group_id, name, position) VALUES (?1, ?2, ?3)",
        rusqlite::params![group_id, name, next],
    )?;
    Ok(conn.last_insert_rowid())
}

// Folds one unit into another, its names becoming alternates kept after the target's own,
// and logged entries need no touching since each name now belongs to the target's group
pub fn merge_units(from_group: i64, into_group: i64, conn: &Connection) -> Result<()> {
    if from_group == into_group {
        return Err(rusqlite::Error::InvalidParameterName(
            "cannot merge a unit into itself".into(),
        ));
    }
    let offset: i64 = conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM unit_variant WHERE group_id = ?1",
        [into_group],
        |r| r.get(0),
    )?;
    conn.execute(
        "UPDATE unit_variant SET group_id = ?1, position = position + ?2 WHERE group_id = ?3",
        rusqlite::params![into_group, offset, from_group],
    )?;
    Ok(())
}

// Makes a name the main by dropping its position below the rest of the group, since the
// group reads its main as the lowest-positioned name and positions may go negative
pub fn set_main_variant(id: i64, conn: &Connection) -> Result<()> {
    let min: i64 = conn.query_row(
        "SELECT COALESCE(MIN(position), 0) FROM unit_variant
         WHERE group_id = (SELECT group_id FROM unit_variant WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    conn.execute(
        "UPDATE unit_variant SET position = ?1 WHERE id = ?2",
        rusqlite::params![min - 1, id],
    )?;
    Ok(())
}

// Renames one name, and entries showing it read through a live join so the change reaches
// all of them, the way renaming a deck reaches its logged sessions
pub fn rename_variant(id: i64, name: &str, conn: &Connection) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "unit name required".into(),
        ));
    }
    conn.execute(
        "UPDATE unit_variant SET name = ?1 WHERE id = ?2",
        rusqlite::params![name, id],
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crud::models::Card;

    // What Swap promises during a session, the card steps out and an eligible one takes
    // the slot rather than the day quietly getting one card shorter
    #[test]
    fn pausing_a_due_card_pulls_in_a_replacement() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, tmp.path()).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO plan (id, name) VALUES (1, 'p');
            INSERT INTO "group" (id, plan_id, name, group_type) VALUES (1, 1, 'd', 'deck');
            INSERT INTO scheduler (group_id, max_new, max_review, can_overflow)
            VALUES (1, 1, 0, FALSE);
            -- two eligible new cards, only one slot, so one is due and one waits
            INSERT INTO card (id, group_id, front, back, tier, sequence, is_due, is_overdue)
            VALUES (1, 1, 'a', 'a', 0, 0, TRUE, FALSE),
                   (2, 1, 'b', 'b', 0, 0, FALSE, NULL);
            "#,
        )
        .unwrap();

        set_card_paused(1, true, &conn).unwrap();

        let (paused, due): (bool, bool) = conn
            .query_row("SELECT is_paused, is_due FROM card WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert!(paused && !due, "the swapped card leaves the queue");

        let replacement_due: bool = conn
            .query_row("SELECT is_due FROM card WHERE id = 2", [], |r| r.get(0))
            .unwrap();
        assert!(replacement_due, "the freed slot goes to the next eligible card");
    }

    // Nothing eligible to promote just means the session is one card shorter
    #[test]
    fn pausing_with_no_candidates_just_removes_the_card() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, tmp.path()).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO plan (id, name) VALUES (1, 'p');
            INSERT INTO "group" (id, plan_id, name, group_type) VALUES (1, 1, 'd', 'deck');
            INSERT INTO scheduler (group_id, max_new, max_review, can_overflow)
            VALUES (1, 1, 0, FALSE);
            INSERT INTO card (id, group_id, front, back, tier, sequence, is_due, is_overdue)
            VALUES (1, 1, 'a', 'a', 0, 0, TRUE, FALSE),
            -- not ready: its countdown hasn't run out
                   (2, 1, 'b', 'b', 0, 5, FALSE, NULL);
            "#,
        )
        .unwrap();

        set_card_paused(1, true, &conn).unwrap();

        let due_now: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM card WHERE group_id = 1 AND is_due = TRUE AND is_paused = FALSE",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(due_now, 0, "no candidate, so nothing takes its place");
    }

    #[test]
    fn update_card_saves_user_fields_and_leaves_imported_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, tmp.path()).unwrap();
        conn.execute(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'g', 'deck')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO card (id, group_id, front, back, imported_front, imported_back, imported_support, is_uploaded)
             VALUES (1, 1, '', '', '<b>anki front</b>', '<b>anki back</b>', '<b>anki support</b>', TRUE)",
            [],
        )
        .unwrap();

        let card = Card {
            id: 1,
            group_id: 1,
            front: "my front".into(),
            back: "my back".into(),
            tier: 0,
            ease: 0.0,
            sequence: 0,
            support: Some("my support".into()),
            imported_front: None,
            imported_back: None,
            imported_support: None,
            front_image: None,
            back_image: None,
            front_audio: None,
            back_audio: None,
            is_searchable: true,
            is_uploaded: true,
            is_due: false,
            is_overdue: None,
            is_paused: false,
            is_cram: false,
            position: None,
        };
        update_card(card, &conn, tmp.path()).unwrap();

        let row: (String, String, String, String, String, String) = conn
            .query_row(
                "SELECT front, back, support, imported_front, imported_back, imported_support FROM card WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!(row.0, "my front");
        assert_eq!(row.1, "my back");
        assert_eq!(row.2, "my support");
        // imported content survives even though the incoming card carried None
        assert_eq!(row.3, "<b>anki front</b>");
        assert_eq!(row.4, "<b>anki back</b>");
        assert_eq!(row.5, "<b>anki support</b>");
    }
}
