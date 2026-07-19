use crate::app_utils::duplicate_media::{
    copy_html_media, copy_media_opt, copy_page_content_media, remove_copied_files,
};
use crate::app_utils::{manage_img::*, save_audio::*, save_img::*};
use crate::crud::{
    models::*,
    read::get_card,
    scheduling::{fill_group, get_date, on_item_added},
};
use rusqlite::{Connection, Result};
use std::collections::HashMap;
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
    merge_stats: bool,
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
    } else {
        // The merged deck starts unlinked from any plan, so nothing can be due.
        tx.execute(
            "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1",
            [new_deck_id],
        )?;
    }

    // Sum both decks' same-day stat rows into a joint history under the new name
    if merge_stats {
        tx.execute(
            r#"
            INSERT INTO group_stats (group_id, plan_id, plan_name, group_name, date,
                                     num_promote, num_demote, num_new, time_spent_minutes, retention_rate)
            SELECT ?1, plan_id, MAX(plan_name), ?2, date,
                   SUM(num_promote), SUM(num_demote), SUM(num_new), SUM(time_spent_minutes),
                   CASE WHEN SUM(num_promote) + SUM(num_demote) > 0
                        THEN CAST(SUM(num_promote) AS REAL) / (SUM(num_promote) + SUM(num_demote))
                        ELSE 0.0 END
            FROM group_stats
            WHERE group_id IN (?3, ?4)
            GROUP BY plan_id, date
            "#,
            rusqlite::params![new_deck_id, new_name, deck_a_id, deck_b_id],
        )?;
        tx.execute(
            "DELETE FROM group_stats WHERE group_id IN (?1, ?2)",
            rusqlite::params![deck_a_id, deck_b_id],
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

// One source card's copyable columns for duplicate_deck.
struct DupCardRow {
    front: String,
    back: String,
    is_searchable: bool,
    support: Option<String>,
    imported_front: Option<String>,
    imported_back: Option<String>,
    imported_support: Option<String>,
    front_image: Option<String>,
    back_image: Option<String>,
    front_audio: Option<String>,
    back_audio: Option<String>,
    is_uploaded: bool,
    tier: i32,
    ease: f32,
    sequence: i32,
    is_paused: bool,
    position: Option<i64>,
}

// Copies a deck and all its cards into a new, unassigned deck. Every referenced
// media file is copied under a fresh name so the two decks never share files.
// `reset` wipes SRS state on the copy; otherwise progress carries over but due
// flags are cleared since the copy has no plan.
pub fn duplicate_deck(
    deck_id: i64,
    new_name: String,
    reset: bool,
    conn: &mut Connection,
    app_dir: &Path,
) -> Result<Group> {
    let tx = conn.transaction()?;

    let group_type: String = tx.query_row(
        r#"SELECT group_type FROM "group" WHERE id = ?1"#,
        [deck_id],
        |r| r.get(0),
    )?;
    if group_type != "deck" {
        return Err(rusqlite::Error::InvalidParameterName(
            "That group is not a deck.".to_string(),
        ));
    }

    tx.execute(
        r#"INSERT INTO "group" (name, group_type) VALUES (?1, 'deck')"#,
        rusqlite::params![new_name],
    )?;
    let new_deck_id = tx.last_insert_rowid();

    let cards: Vec<DupCardRow> = {
        let mut stmt = tx.prepare(
            r#"
            SELECT front, back, is_searchable, support,
                   imported_front, imported_back, imported_support,
                   front_image, back_image, front_audio, back_audio, is_uploaded,
                   tier, ease, sequence, is_paused, position
            FROM card WHERE group_id = ?1
            ORDER BY CASE WHEN position IS NULL THEN 1 ELSE 0 END, position ASC, id ASC
            "#,
        )?;
        let rows = stmt
            .query_map([deck_id], |r| {
            Ok(DupCardRow {
                front: r.get(0)?,
                back: r.get(1)?,
                is_searchable: r.get(2)?,
                support: r.get(3)?,
                imported_front: r.get(4)?,
                imported_back: r.get(5)?,
                imported_support: r.get(6)?,
                front_image: r.get(7)?,
                back_image: r.get(8)?,
                front_audio: r.get(9)?,
                back_audio: r.get(10)?,
                is_uploaded: r.get(11)?,
                tier: r.get(12)?,
                ease: r.get(13)?,
                sequence: r.get(14)?,
                is_paused: r.get(15)?,
                position: r.get(16)?,
            })
        })?
            .collect::<Result<Vec<_>>>()?;
        rows
    };

    // If anything fails after files start landing, the copies made so far are
    // deleted and the transaction rolls back, so no half-made deck survives.
    let mut cache: HashMap<String, String> = HashMap::new();
    let inserted = (|| -> Result<()> {
        let mut insert = tx.prepare(
            r#"
            INSERT INTO card (
                group_id, front, back, is_searchable, support,
                imported_front, imported_back, imported_support,
                front_image, back_image, front_audio, back_audio, is_uploaded,
                tier, ease, sequence, is_due, is_overdue, is_paused, position
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
            "#,
        )?;
        for c in &cards {
            let front_image = copy_media_opt(&c.front_image, app_dir, &mut cache)?;
            let back_image = copy_media_opt(&c.back_image, app_dir, &mut cache)?;
            let front_audio = copy_media_opt(&c.front_audio, app_dir, &mut cache)?;
            let back_audio = copy_media_opt(&c.back_audio, app_dir, &mut cache)?;
            let imported_front = copy_html_media(&c.imported_front, app_dir, &mut cache)?;
            let imported_back = copy_html_media(&c.imported_back, app_dir, &mut cache)?;
            let imported_support = copy_html_media(&c.imported_support, app_dir, &mut cache)?;

            // The copy is unlinked from any plan, so nothing can be due.
            let (tier, ease, sequence, is_due, is_overdue, is_paused) = if reset {
                (0i32, 0.0f32, 0i32, false, None::<bool>, false)
            } else {
                (c.tier, c.ease, c.sequence, false, None::<bool>, c.is_paused)
            };

            insert.execute(rusqlite::params![
                new_deck_id,
                c.front,
                c.back,
                c.is_searchable,
                c.support,
                imported_front,
                imported_back,
                imported_support,
                front_image,
                back_image,
                front_audio,
                back_audio,
                c.is_uploaded,
                tier,
                ease,
                sequence,
                is_due,
                is_overdue,
                is_paused,
                c.position,
            ])?;
        }
        Ok(())
    })();
    if let Err(e) = inserted {
        remove_copied_files(&cache, app_dir);
        return Err(e);
    }

    if let Err(e) = tx.commit() {
        remove_copied_files(&cache, app_dir);
        return Err(e);
    }

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
            group_id, front, back, is_searchable, support,
            imported_front, imported_back, imported_support,
            front_image, back_image, front_audio, back_audio, is_uploaded
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        rusqlite::params![
            card.group_id,
            card.front,
            card.back,
            card.is_searchable,
            card.support,
            card.imported_front,
            card.imported_back,
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
    // Re-read after on_item_added since fill_group may have made the card due.
    get_card(id, conn)
}

pub fn create_card_imported(card: NewCard, conn: &Connection) -> Result<Card> {
    conn.execute(
        r#"
        INSERT INTO card (
            group_id, front, back, is_searchable, support,
            imported_front, imported_back, imported_support,
            front_image, back_image, front_audio, back_audio, is_uploaded
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        rusqlite::params![
            card.group_id,
            card.front,
            card.back,
            card.is_searchable,
            card.support,
            card.imported_front,
            card.imported_back,
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
        imported_front: card.imported_front,
        imported_back: card.imported_back,
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

// Copies a notebook and all its pages into a new, unassigned notebook. Page
// images and audio are copied under fresh names so the two notebooks never
// share files. Created dates are preserved so page ordering matches the source.
pub fn duplicate_notebook(
    notebook_id: i64,
    new_name: String,
    conn: &mut Connection,
    app_dir: &Path,
) -> Result<Group> {
    let tx = conn.transaction()?;

    let group_type: String = tx.query_row(
        r#"SELECT group_type FROM "group" WHERE id = ?1"#,
        [notebook_id],
        |r| r.get(0),
    )?;
    if group_type != "notebook" {
        return Err(rusqlite::Error::InvalidParameterName(
            "That group is not a notebook.".to_string(),
        ));
    }

    tx.execute(
        r#"INSERT INTO "group" (name, group_type) VALUES (?1, 'notebook')"#,
        rusqlite::params![new_name],
    )?;
    let new_id = tx.last_insert_rowid();

    let pages: Vec<(String, Option<String>, String, Option<String>, String)> = {
        let mut stmt = tx.prepare(
            r#"
            SELECT title, description, content, audio_file, created_date
            FROM page WHERE group_id = ?1
            ORDER BY created_date ASC, id ASC
            "#,
        )?;
        let rows = stmt
            .query_map([notebook_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<Vec<_>>>()?;
        rows
    };

    // Same unwind rule as duplicate_deck: a failed copy deletes the new files
    // and rolls back, never a notebook that shares media with the source.
    let mut cache: HashMap<String, String> = HashMap::new();
    let inserted = (|| -> Result<()> {
        let mut insert = tx.prepare(
            r#"
            INSERT INTO page (group_id, title, description, content, audio_file, created_date)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )?;
        for (title, description, content, audio_file, created_date) in &pages {
            let new_content = copy_page_content_media(content, app_dir, &mut cache)?;
            let new_audio = copy_media_opt(audio_file, app_dir, &mut cache)?;
            insert.execute(rusqlite::params![
                new_id,
                title,
                description,
                new_content,
                new_audio,
                created_date,
            ])?;
        }
        Ok(())
    })();
    if let Err(e) = inserted {
        remove_copied_files(&cache, app_dir);
        return Err(e);
    }

    if let Err(e) = tx.commit() {
        remove_copied_files(&cache, app_dir);
        return Err(e);
    }

    Ok(Group {
        id: new_id,
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
