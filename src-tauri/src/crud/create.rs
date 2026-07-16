use crate::app_utils::{manage_img::*, save_audio::*, save_img::*};
use crate::crud::{
    models::*,
    scheduling::{fill_group, get_date, on_item_added},
};
use rusqlite::{Connection, Result};
use std::path::Path;

pub fn create_plan(name: &str, conn: &mut Connection) -> Result<Plan> {
    conn.execute(
        r#"
        INSERT INTO plan (name)
        VALUES (?1)
        "#,
        rusqlite::params![name],
    )?;

    let id = conn.last_insert_rowid();

    Ok(Plan {
        id,
        name: name.to_string(),
    })
}

pub fn create_todo(todo: NewTodo, conn: &mut Connection) -> Result<Todo> {
    use chrono::Datelike;
    let today = get_date(conn)?;
    let weekday = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?
        .weekday()
        .num_days_from_sunday();
    let is_disabled = (todo.frequency & (1 << weekday)) == 0;

    conn.execute(
        r#"
        INSERT INTO todo (plan_id, text, frequency, category, is_done, is_disabled)
        VALUES (?1, ?2, ?3, ?4, FALSE, ?5)
        "#,
        rusqlite::params![
            todo.plan_id,
            todo.text,
            todo.frequency,
            todo.category,
            is_disabled
        ],
    )?;

    let id = conn.last_insert_rowid();

    Ok(Todo {
        id,
        plan_id: todo.plan_id,
        text: todo.text,
        frequency: todo.frequency,
        category: todo.category,
        is_done: false,
        is_disabled,
        is_skipped: false,
        position: None,
    })
}

pub fn create_deck(name: String, conn: &Connection) -> Result<Group> {
    conn.execute(
        r#"
        INSERT INTO "group" (
            name,
            group_type
        )
        VALUES (?1, 'deck')
        "#,
        rusqlite::params![name],
    )?;

    let id = conn.last_insert_rowid();

    Ok(Group {
        id,

        plan_id: None,

        name,

        group_type: GroupType::Deck,
    })
}

pub fn merge_decks(
    deck_a_id: i64,
    deck_b_id: i64,
    new_name: String,
    reset: bool,
    conn: &mut Connection,
) -> Result<Group> {
    let tx = conn.transaction()?;

    // Fetch card IDs from each source deck BEFORE moving, sorted by id.
    let fetch_ids = |gid: i64| -> Result<Vec<i64>> {
        tx.prepare("SELECT id FROM card WHERE group_id = ?1 ORDER BY id")?
            .query_map([gid], |r| r.get(0))?
            .collect()
    };
    let a_ids = fetch_ids(deck_a_id)?;
    let b_ids = fetch_ids(deck_b_id)?;

    tx.execute(
        r#"INSERT INTO "group" (name, group_type) VALUES (?1, 'deck')"#,
        rusqlite::params![new_name],
    )?;
    let new_deck_id = tx.last_insert_rowid();

    tx.execute(
        "UPDATE card SET group_id = ?1 WHERE group_id = ?2 OR group_id = ?3",
        rusqlite::params![new_deck_id, deck_a_id, deck_b_id],
    )?;

    if reset {
        tx.execute(
            r#"
            UPDATE card
            SET tier = 0, ease = 0.0, sequence = 0,
                is_due = FALSE, is_overdue = NULL, is_paused = FALSE
            WHERE group_id = ?1
            "#,
            [new_deck_id],
        )?;
    }

    // Delete the two source decks (cards already moved, so no cascade loss)
    tx.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'deck'"#,
        [deck_a_id],
    )?;
    tx.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'deck'"#,
        [deck_b_id],
    )?;

    // Assign zipper positions so fill_track interleaves cards from both decks.
    // A[0]→pos 0, B[0]→pos 1, A[1]→pos 2, B[1]→pos 3, …
    let max_len = a_ids.len().max(b_ids.len());
    let mut interleaved: Vec<i64> = Vec::with_capacity(a_ids.len() + b_ids.len());
    for i in 0..max_len {
        if i < a_ids.len() {
            interleaved.push(a_ids[i]);
        }
        if i < b_ids.len() {
            interleaved.push(b_ids[i]);
        }
    }
    if interleaved.len() > 1 {
        let mut stmt = tx.prepare("UPDATE card SET position = ?1 WHERE id = ?2")?;
        for (p, &card_id) in interleaved.iter().enumerate() {
            stmt.execute(rusqlite::params![p as i64, card_id])?;
        }
    }

    tx.commit()?;

    Ok(Group {
        id: new_deck_id,
        plan_id: None,
        name: new_name,
        group_type: GroupType::Deck,
    })
}

pub fn create_card(card: NewCard, conn: &mut Connection, app_dir: &Path) -> Result<Card> {
    let front_image = save_card_image(card.front_image, app_dir)?;
    let back_image = save_card_image(card.back_image, app_dir)?;
    let front_audio = save_card_audio_file(card.front_audio, app_dir)?;
    let back_audio = save_card_audio_file(card.back_audio, app_dir)?;

    conn.execute(
        r#"
        INSERT INTO card (
            group_id, front, back, is_searchable, support, imported_support,
            front_image, back_image, front_audio, back_audio, is_uploaded
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        rusqlite::params![
            card.group_id,
            card.front,
            card.back,
            card.is_searchable,
            card.support,
            card.imported_support,
            front_image,
            back_image,
            front_audio,
            back_audio,
            card.is_uploaded,
        ],
    )?;

    let id = conn.last_insert_rowid();
    let _ = on_item_added(card.group_id, conn);
    Ok(Card {
        id,
        group_id: card.group_id,
        front: card.front,
        back: card.back,
        tier: 0,
        ease: 0.0,
        sequence: 0,
        is_searchable: card.is_searchable,
        support: card.support,
        imported_support: card.imported_support,
        front_image,
        back_image,
        front_audio,
        back_audio,
        is_uploaded: card.is_uploaded,
        is_due: false,
        is_overdue: None,
        is_paused: false,
        position: None,
    })
}

pub fn create_card_imported(card: NewCard, conn: &Connection) -> Result<Card> {
    conn.execute(
        r#"
        INSERT INTO card (
            group_id, front, back, is_searchable, support, imported_support,
            front_image, back_image, front_audio, back_audio, is_uploaded
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        rusqlite::params![
            card.group_id,
            card.front,
            card.back,
            card.is_searchable,
            card.support,
            card.imported_support,
            card.front_image,
            card.back_image,
            card.front_audio,
            card.back_audio,
            card.is_uploaded,
        ],
    )?;

    let id = conn.last_insert_rowid();

    Ok(Card {
        id,
        group_id: card.group_id,
        front: card.front,
        back: card.back,
        tier: 0,
        ease: 0.0,
        sequence: 0,
        is_searchable: card.is_searchable,
        support: card.support,
        imported_support: card.imported_support,
        front_image: card.front_image,
        back_image: card.back_image,
        front_audio: card.front_audio,
        back_audio: card.back_audio,
        is_uploaded: card.is_uploaded,
        is_due: false,
        is_overdue: None,
        is_paused: false,
        position: None,
    })
}

pub fn create_notebook(name: String, conn: &Connection) -> Result<Group> {
    conn.execute(
        r#"
        INSERT INTO "group" (name, group_type)
        VALUES (?1, 'notebook')
        "#,
        rusqlite::params![name],
    )?;

    let id = conn.last_insert_rowid();

    Ok(Group {
        id,
        plan_id: None,
        name,
        group_type: GroupType::Notebook,
    })
}

pub fn merge_notebooks(
    notebook_a_id: i64,
    notebook_b_id: i64,
    new_name: String,
    conn: &mut Connection,
) -> Result<Group> {
    if notebook_a_id == notebook_b_id {
        return Err(rusqlite::Error::InvalidParameterName(
            "Cannot merge a notebook with itself".to_string(),
        ));
    }

    let tx = conn.transaction()?;

    tx.execute(
        r#"INSERT INTO "group" (name, group_type) VALUES (?1, 'notebook')"#,
        rusqlite::params![new_name],
    )?;
    let new_notebook_id = tx.last_insert_rowid();

    tx.execute(
        "UPDATE page SET group_id = ?1 WHERE group_id = ?2 OR group_id = ?3",
        rusqlite::params![new_notebook_id, notebook_a_id, notebook_b_id],
    )?;

    tx.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'notebook'"#,
        [notebook_a_id],
    )?;
    tx.execute(
        r#"DELETE FROM "group" WHERE id = ?1 AND group_type = 'notebook'"#,
        [notebook_b_id],
    )?;

    tx.commit()?;

    Ok(Group {
        id: new_notebook_id,
        plan_id: None,
        name: new_name,
        group_type: GroupType::Notebook,
    })
}

pub fn create_page(page: NewPage, conn: &Connection, app_dir: &Path) -> Result<Page> {
    let content = rewrite_images_in_content(&page.content, app_dir)?;
    let created_date = get_date(conn)?;

    conn.execute(
        r#"
        INSERT INTO page (group_id, title, description, content, audio_file, created_date)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        rusqlite::params![
            page.group_id,
            page.title,
            page.description,
            content,
            page.audio_file,
            created_date,
        ],
    )?;

    let id = conn.last_insert_rowid();

    Ok(Page {
        id,
        group_id: page.group_id,
        title: page.title,
        description: page.description,
        content,
        audio_file: page.audio_file,
        created_date,
    })
}

pub fn add_group_to_plan(
    group_id: i64,
    plan_id: i64,
    scheduler: NewScheduler,
    conn: &mut Connection,
) -> Result<Scheduler> {
    let tx = conn.transaction()?;

    tx.execute(
        r#"UPDATE "group" SET plan_id = ?1 WHERE id = ?2"#,
        rusqlite::params![plan_id, group_id],
    )?;

    tx.execute(
        r#"
        INSERT INTO scheduler (
            group_id, studied_new, max_new,
            studied_review, max_review, can_overflow
        )
        VALUES (?1, 0, ?2, 0, ?3, ?4)
        "#,
        rusqlite::params![
            group_id,
            scheduler.max_new,
            scheduler.max_review,
            scheduler.can_overflow
        ],
    )?;

    // Fetch group name while still in scope before commit
    let group_name: String = tx.query_row(
        r#"SELECT name FROM "group" WHERE id = ?1"#,
        [group_id],
        |row| row.get(0),
    )?;

    tx.commit()?;

    let today = get_date(conn)?;

    let plan_name: String = conn
        .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    conn.execute(
        r#"
        INSERT INTO group_stats (group_id, plan_id, plan_name, group_name, date, num_promote, num_demote, time_spent_minutes, retention_rate)
        VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0.0, 0.0)
        "#,
        rusqlite::params![group_id, plan_id, plan_name, group_name, &today],
    )?;

    let _ = fill_group(group_id, conn);

    Ok(Scheduler {
        group_id,
        studied_new: 0,
        max_new: scheduler.max_new,
        studied_review: 0,
        max_review: scheduler.max_review,
        can_overflow: scheduler.can_overflow,
    })
}

pub fn create_resource(resource: NewResource, conn: &Connection) -> Result<Resource> {
    conn.execute(
        "INSERT INTO resource (plan_id, name, type, url, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            resource.plan_id,
            resource.name,
            resource.resource_type,
            resource.url,
            resource.notes,
        ],
    )?;
    Ok(Resource {
        id: conn.last_insert_rowid(),
        plan_id: resource.plan_id,
        name: resource.name,
        resource_type: resource.resource_type,
        url: resource.url,
        notes: resource.notes,
    })
}
