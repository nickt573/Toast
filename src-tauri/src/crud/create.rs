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

    // The new deck gets a copy of both decks' latest versions, combined. Nothing is
    // taken from the sources, they keep every version and only pick up a flag so
    // the stats page can say "merged" rather than "deleted". Resetting on merge
    // copies nothing, leaving the new deck a clean first version.
    if !reset {
        let latest_a: i64 = tx.query_row(
            r#"SELECT stat_version FROM "group" WHERE id = ?1"#,
            [deck_a_id],
            |r| r.get(0),
        )?;
        let latest_b: i64 = tx.query_row(
            r#"SELECT stat_version FROM "group" WHERE id = ?1"#,
            [deck_b_id],
            |r| r.get(0),
        )?;

        tx.execute(
            r#"
            INSERT INTO group_stats (group_id, origin_group_id, plan_id, plan_name, group_name, date, version,
                                     num_promote, num_demote, num_new, time_spent_minutes, retention_rate)
            SELECT ?1, ?1, plan_id, MAX(plan_name), ?2, date, 1,
                   SUM(num_promote), SUM(num_demote), SUM(num_new), SUM(time_spent_minutes),
                   CASE WHEN SUM(num_promote) + SUM(num_demote) > 0
                        THEN CAST(SUM(num_promote) AS REAL) / (SUM(num_promote) + SUM(num_demote))
                        ELSE 0.0 END
            FROM group_stats
            WHERE (group_id = ?3 AND version = ?4)
               OR (group_id = ?5 AND version = ?6)
            GROUP BY plan_id, date
            HAVING SUM(num_promote) + SUM(num_demote) + SUM(num_new) > 0
                OR SUM(time_spent_minutes) > 0
            "#,
            rusqlite::params![
                new_deck_id,
                new_name,
                deck_a_id,
                latest_a,
                deck_b_id,
                latest_b
            ],
        )?;

        // Only what was actually copied is archived. Every other version is still
        // the sole record of that study and keeps counting toward the plan.
        tx.execute(
            "UPDATE group_stats SET is_archived = TRUE
             WHERE (group_id = ?1 AND version = ?2) OR (group_id = ?3 AND version = ?4)",
            rusqlite::params![deck_a_id, latest_a, deck_b_id, latest_b],
        )?;
    }

    tx.execute(
        "UPDATE group_stats SET is_merged = TRUE WHERE group_id IN (?1, ?2)",
        rusqlite::params![deck_a_id, deck_b_id],
    )?;

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
    // Today's line for this plan in the current version gets picked back up, so
    // repeated add and remove can't stack empty rows. A reset already moved the
    // deck onto a new version, so this naturally starts that version's first line.
    let version = crate::crud::scheduling::current_version(group_id, conn)?;
    let has_line_today: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM group_stats
            WHERE group_id = ?1 AND date = ?2 AND plan_id = ?3 AND version = ?4
        )",
        rusqlite::params![group_id, &today, plan_id, version],
        |r| r.get(0),
    )?;

    if !has_line_today {
        conn.execute(
            r#"
            INSERT INTO group_stats (group_id, origin_group_id, plan_id, plan_name, group_name, date, version, num_promote, num_demote, time_spent_minutes, retention_rate)
            VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, 0, 0, 0.0, 0.0)
            "#,
            rusqlite::params![group_id, plan_id, plan_name, group_name, &today, version],
        )?;
    }

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


// this checks all of them after every step of every sequence.
#[cfg(test)]
mod version_invariant_tests {
    use super::*;
    use crate::crud::{
        delete::remove_group_from_plan,
        scheduling::{current_version, reset_deck},
    };

    #[derive(Clone, Copy, Debug)]
    enum Op {
        AddP1,
        AddP2,
        RemovePreserve,
        RemoveReset,
        Reset,
        Study,
        RollDay,
        Merge,
        MergeReset,
    }

    const OPS: [Op; 9] = [
        Op::AddP1,
        Op::AddP2,
        Op::RemovePreserve,
        Op::RemoveReset,
        Op::Reset,
        Op::Study,
        Op::RollDay,
        Op::Merge,
        Op::MergeReset,
    ];

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, &std::path::PathBuf::from("/tmp/toast-test")).unwrap();
        conn.execute_batch(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'd', 'deck');
             INSERT INTO plan (id, name) VALUES (1, 'p one'), (2, 'p two');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_date (id, date) VALUES (0, ?1)",
            [chrono::Local::now().date_naive().to_string()],
        )
        .unwrap();
        conn
    }

    fn in_plan(conn: &Connection, deck: i64) -> bool {
        conn.query_row(
            r#"SELECT plan_id IS NOT NULL FROM "group" WHERE id = ?1"#,
            [deck],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn apply(conn: &mut Connection, deck: i64, op: Op) -> Option<i64> {
        match op {
            Op::AddP1 | Op::AddP2 => {
                if in_plan(conn, deck) {
                    return None;
                }
                let plan = if matches!(op, Op::AddP1) { 1 } else { 2 };
                add_group_to_plan(
                    deck,
                    plan,
                    NewScheduler {
                        group_id: deck,
                        max_new: 10,
                        max_review: 10,
                        can_overflow: false,
                    },
                    conn,
                )
                .unwrap();
            }
            Op::RemovePreserve | Op::RemoveReset => {
                if !in_plan(conn, deck) {
                    return None;
                }
                remove_group_from_plan(deck, matches!(op, Op::RemoveReset), conn).unwrap();
            }
            Op::Reset => reset_deck(deck, conn).unwrap(),
            Op::Study => {
                let hit = conn
                    .execute(
                        "UPDATE group_stats SET num_new = num_new + 1
                         WHERE id = (SELECT MAX(id) FROM group_stats WHERE group_id = ?1)",
                        [deck],
                    )
                    .unwrap();
                if hit == 0 {
                    return None;
                }
            }
            Op::RollDay => {
                conn.execute(
                    "UPDATE app_date SET date = date(date, '+1 day') WHERE id = 0",
                    [],
                )
                .unwrap();
            }
            Op::Merge | Op::MergeReset => {
                // A merge unlinks the result, so only run it from outside a plan
                if in_plan(conn, deck) {
                    return None;
                }
                let partner = create_deck("partner".into(), conn).unwrap().id;
                let merged = merge_decks(
                    deck,
                    partner,
                    "merged".into(),
                    matches!(op, Op::MergeReset),
                    conn,
                )
                .unwrap();
                return Some(merged.id);
            }
        }
        Some(deck)
    }

    // One line per deck per version per plan per day, always
    fn check_one_line_per_slot(conn: &Connection, trace: &str) {
        let dupes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (
                    SELECT 1 FROM group_stats
                    GROUP BY origin_group_id, version, plan_id, date
                    HAVING COUNT(*) > 1
                 )",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 0, "duplicate line for one day after {trace}");
    }

    // Leaving a plan takes that plan's empty lines with it
    fn check_no_bare_lines_when_out_of_plan(conn: &Connection, deck: i64, trace: &str) {
        if in_plan(conn, deck) {
            return;
        }
        let bare: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM group_stats
                 WHERE group_id = ?1 AND num_new = 0 AND num_promote = 0 AND num_demote = 0",
                [deck],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bare, 0, "empty line left behind after {trace}");
    }

    // A version number only ever goes up, so history can't be rewritten
    fn check_version_never_goes_backwards(conn: &Connection, deck: i64, prev: i64, trace: &str) {
        let now = current_version(deck, conn).unwrap();
        assert!(now >= prev, "version went backwards after {trace}");
    }

    // Nothing is ever written into a version the deck hasn't reached
    fn check_no_lines_beyond_current_version(conn: &Connection, deck: i64, trace: &str) {
        let ahead: i64 = conn
            .query_row(
                r#"SELECT COUNT(*) FROM group_stats gs
                   INNER JOIN "group" g ON g.id = gs.group_id
                   WHERE gs.group_id = ?1 AND gs.version > g.stat_version"#,
                [deck],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ahead, 0, "line written past the current version after {trace}");
    }

    fn sweep(depth: usize) {
        let mut seq = vec![OPS[0]; depth];
        let total = OPS.len().pow(depth as u32);

        for n in 0..total {
            let mut k = n;
            for slot in seq.iter_mut() {
                *slot = OPS[k % OPS.len()];
                k /= OPS.len();
            }

            let mut conn = fresh();
            let mut deck = 1i64;
            let mut prev_version = 1i64;
            let mut done: Vec<Op> = Vec::new();

            for &op in seq.iter() {
                let carried = matches!(op, Op::Merge | Op::MergeReset);
                let Some(next) = apply(&mut conn, deck, op) else {
                    continue;
                };
                done.push(op);
                let trace = format!("{done:?}");

                // A merge starts a new deck, so the version baseline restarts with it
                if carried {
                    deck = next;
                    prev_version = current_version(deck, &conn).unwrap();
                } else {
                    check_version_never_goes_backwards(&conn, deck, prev_version, &trace);
                    prev_version = current_version(deck, &conn).unwrap();
                }

                check_one_line_per_slot(&conn, &trace);
                check_no_bare_lines_when_out_of_plan(&conn, deck, &trace);
                check_no_lines_beyond_current_version(&conn, deck, &trace);
            }
        }
    }

    #[test]
    fn every_three_step_sequence_holds_the_invariants() {
        sweep(3);
    }

    #[test]
    fn every_four_step_sequence_holds_the_invariants() {
        sweep(4);
    }
}

// One test per documented behaviour. If a row of the behaviour table isn't
// provable here, it isn't a behaviour, it's a hope.
#[cfg(test)]
mod version_tests {
    use super::*;
    use crate::crud::{
        delete::{delete_group_stats_for_deck, remove_group_from_plan},
        read::get_group_stats,
        scheduling::{current_version, reset_deck},
    };

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, &std::path::PathBuf::from("/tmp/toast-test")).unwrap();
        conn.execute_batch(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'deck a', 'deck'), (2, 'deck b', 'deck');
             INSERT INTO plan (id, name) VALUES (1, 'p one'), (2, 'p two'), (3, 'p three');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_date (id, date) VALUES (0, ?1)",
            [chrono::Local::now().date_naive().to_string()],
        )
        .unwrap();
        conn
    }

    fn add(conn: &mut Connection, deck: i64, plan: i64) {
        add_group_to_plan(
            deck,
            plan,
            NewScheduler { group_id: deck, max_new: 10, max_review: 10, can_overflow: false },
            conn,
        )
        .unwrap();
    }

    fn study(conn: &Connection, deck: i64, n: i64) {
        conn.execute(
            "UPDATE group_stats SET num_new = num_new + ?2
             WHERE id = (SELECT MAX(id) FROM group_stats WHERE group_id = ?1)",
            rusqlite::params![deck, n],
        )
        .unwrap();
    }

    fn roll_day(conn: &Connection) {
        conn.execute("UPDATE app_date SET date = date(date, '+1 day')", []).unwrap();
    }

    fn rows(conn: &Connection, deck: i64) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM group_stats WHERE group_id = ?1", [deck], |r| r.get(0))
            .unwrap()
    }

    fn new_at(conn: &Connection, origin: i64, version: i64, plan: i64) -> i64 {
        conn.query_row(
            "SELECT COALESCE(SUM(num_new), 0) FROM group_stats
             WHERE origin_group_id = ?1 AND version = ?2 AND plan_id = ?3",
            rusqlite::params![origin, version, plan],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn versions(conn: &Connection, origin: i64) -> Vec<i64> {
        let mut v: Vec<i64> = conn
            .prepare("SELECT DISTINCT version FROM group_stats WHERE origin_group_id = ?1 ORDER BY version")
            .unwrap()
            .query_map([origin], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        v.dedup();
        v
    }

    // What the plan actually counts, which is every line that isn't an archived copy
    fn plan_total(conn: &Connection, plan: i64) -> i64 {
        get_group_stats(plan, conn)
            .unwrap()
            .iter()
            .filter(|r| !r.is_archived)
            .map(|r| r.num_new)
            .sum()
    }

    fn flags(conn: &Connection, origin: i64, version: i64) -> (bool, bool) {
        conn.query_row(
            "SELECT MIN(is_merged), MIN(is_archived) FROM group_stats
             WHERE origin_group_id = ?1 AND version = ?2",
            rusqlite::params![origin, version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    // Versions

    #[test]
    fn t01_new_deck_starts_at_v1() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        assert_eq!(current_version(1, &conn).unwrap(), 1);
        assert_eq!(rows(&conn, 1), 1);
    }

    #[test]
    fn t02_reset_with_data_opens_the_next_version() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        assert_eq!(current_version(1, &conn).unwrap(), 2);
        assert_eq!(new_at(&conn, 1, 1, 1), 5, "v1 keeps its data");
        assert_eq!(new_at(&conn, 1, 2, 1), 0, "v2 starts empty");
    }

    #[test]
    fn t03_reset_on_an_unused_version_replaces_it() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        for _ in 0..4 {
            reset_deck(1, &conn).unwrap();
        }
        assert_eq!(current_version(1, &conn).unwrap(), 2, "no empty versions stack up");
    }

    #[test]
    fn t04_repeated_resets_out_of_plan_only_bump_once() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        for _ in 0..5 {
            reset_deck(1, &conn).unwrap();
        }
        assert_eq!(current_version(1, &conn).unwrap(), 2);
    }

    #[test]
    fn t05_resets_on_an_unstudied_deck_never_bump() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        for _ in 0..5 {
            reset_deck(1, &conn).unwrap();
        }
        assert_eq!(current_version(1, &conn).unwrap(), 1);
        assert_eq!(rows(&conn, 1), 0);
    }

    #[test]
    fn t06_a_reset_survives_leaving_and_rejoining_a_plan() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 1);
        assert_eq!(current_version(1, &conn).unwrap(), 2);
        assert_eq!(versions(&conn, 1), vec![1, 2]);
    }

    #[test]
    fn t07_a_reset_cannot_be_laundered_by_waiting_a_day() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        reset_deck(1, &conn).unwrap();
        roll_day(&conn);
        add(&mut conn, 1, 1);
        assert_eq!(current_version(1, &conn).unwrap(), 2);
    }

    // Plan membership

    #[test]
    fn t08_add_remove_cycles_leave_no_empty_lines() {
        let mut conn = setup();
        for _ in 0..5 {
            add(&mut conn, 1, 1);
            remove_group_from_plan(1, false, &mut conn).unwrap();
        }
        assert_eq!(rows(&conn, 1), 0);
        assert_eq!(current_version(1, &conn).unwrap(), 1);
    }

    #[test]
    fn t09_rejoining_the_same_day_resumes_the_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 1);
        assert_eq!(new_at(&conn, 1, 1, 1), 5);
        assert_eq!(rows(&conn, 1), 1);
    }

    #[test]
    fn t10_a_new_day_opens_its_own_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        roll_day(&conn);
        add(&mut conn, 1, 1);
        assert_eq!(rows(&conn, 1), 2);
        assert_eq!(current_version(1, &conn).unwrap(), 1, "a new day is not a reset");
    }

    #[test]
    fn t11_one_version_spans_several_plans() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 20);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 7);
        assert_eq!(current_version(1, &conn).unwrap(), 1, "moving plans is not a reset");
        assert_eq!(new_at(&conn, 1, 1, 1), 20);
        assert_eq!(new_at(&conn, 1, 1, 2), 7);
    }

    #[test]
    fn t12_each_plan_keeps_its_own_history() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 20);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 7);
        assert_eq!(plan_total(&conn, 1), 20);
        assert_eq!(plan_total(&conn, 2), 7);
    }

    // Merging without a reset

    #[test]
    fn t13_merge_copies_the_latest_versions_into_the_new_deck() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();
        assert_eq!(new_at(&conn, merged.id, 1, 1), 11, "8 + 3");
    }

    #[test]
    fn t14_sources_keep_every_version_after_a_merge() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();
        assert_eq!(versions(&conn, 1), vec![1, 2], "nothing is taken from the source");
        assert_eq!(new_at(&conn, 1, 1, 1), 5);
        assert_eq!(new_at(&conn, 1, 2, 1), 8);
    }

    #[test]
    fn t15_only_the_copied_versions_are_archived() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();
        assert_eq!(flags(&conn, 1, 1), (true, false), "v1 merged, not copied, still counts");
        assert_eq!(flags(&conn, 1, 2), (true, true), "v2 was copied, so it's archived");
        assert_eq!(flags(&conn, 2, 1), (true, true));
        assert_eq!(flags(&conn, merged.id, 1), (false, false), "the living copy counts");
    }

    #[test]
    fn t16_merge_does_not_double_count_the_plan() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let before = plan_total(&conn, 1);
        assert_eq!(before, 16, "5 + 8 + 3");
        merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 16, "the merge moved nothing into the total");
    }

    #[test]
    fn t17_merge_splits_across_plans() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 20);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 10);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 5);
        remove_group_from_plan(2, false, &mut conn).unwrap();
        add(&mut conn, 2, 3);
        study(&conn, 2, 10);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();
        assert_eq!(new_at(&conn, merged.id, 1, 1), 25);
        assert_eq!(new_at(&conn, merged.id, 1, 2), 10);
        assert_eq!(new_at(&conn, merged.id, 1, 3), 10);
    }

    #[test]
    fn t18_a_merged_deck_can_be_merged_again() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();
        let first = merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();

        let third = create_deck("deck c".into(), &conn).unwrap().id;
        add(&mut conn, third, 1);
        study(&conn, third, 4);
        remove_group_from_plan(third, false, &mut conn).unwrap();

        let second = merge_decks(first.id, third, "joint two".into(), false, &mut conn).unwrap();
        assert_eq!(new_at(&conn, second.id, 1, 1), 12);
        assert_eq!(plan_total(&conn, 1), 12, "still counted once after two merges");
    }

    // Merging with a reset

    #[test]
    fn t19_merge_with_reset_copies_nothing() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), true, &mut conn).unwrap();
        assert_eq!(rows(&conn, merged.id), 0, "the new deck starts clean");
        assert_eq!(current_version(merged.id, &conn).unwrap(), 1);
    }

    #[test]
    fn t20_merge_with_reset_archives_nothing_so_totals_hold() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        merge_decks(1, 2, "joint".into(), true, &mut conn).unwrap();
        assert_eq!(flags(&conn, 1, 1), (true, false), "merged but never copied");
        assert_eq!(flags(&conn, 2, 1), (true, false));
        assert_eq!(plan_total(&conn, 1), 8, "nothing was archived, so nothing was lost");
    }

    // Deletion

    #[test]
    fn t21_deleting_one_version_leaves_the_others() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);

        delete_group_stats_for_deck(Some(1), "deck a", Some(1), 1, &conn).unwrap();
        assert_eq!(versions(&conn, 1), vec![2]);
        assert_eq!(plan_total(&conn, 1), 8);
    }

    #[test]
    fn t22_deleting_the_deck_clears_every_version() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);

        delete_group_stats_for_deck(Some(1), "deck a", None, 1, &conn).unwrap();
        assert!(versions(&conn, 1).is_empty());
        assert_eq!(plan_total(&conn, 1), 0);
    }

    #[test]
    fn t23_deleting_a_living_copy_does_not_revive_the_archive() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();
        let merged = merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();

        assert_eq!(plan_total(&conn, 1), 8);
        delete_group_stats_for_deck(Some(merged.id), "joint", None, 1, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 0, "the archive stays archived, as chosen");
        assert_eq!(new_at(&conn, 1, 1, 1), 5, "but it is still there to look at");
    }

    #[test]
    fn t24_deleting_one_plans_stats_leaves_the_other_plan() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 20);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 7);

        delete_group_stats_for_deck(Some(1), "deck a", None, 1, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 0);
        assert_eq!(plan_total(&conn, 2), 7);
    }

    // Identity

    #[test]
    fn t25_same_named_decks_stay_separate() {
        let mut conn = setup();
        conn.execute("UPDATE \"group\" SET name = 'same' WHERE id IN (1, 2)", []).unwrap();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 9);

        assert_eq!(new_at(&conn, 1, 1, 1), 5);
        assert_eq!(new_at(&conn, 2, 1, 1), 9);
        delete_group_stats_for_deck(Some(1), "same", None, 1, &conn).unwrap();
        assert_eq!(new_at(&conn, 2, 1, 1), 9, "deleting one leaves its namesake alone");
    }

    #[test]
    fn t26_streaks_ignore_archived_copies() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();
        let merged = merge_decks(1, 2, "joint".into(), false, &mut conn).unwrap();

        let (with_copy, _) = crate::crud::scheduling::get_plan_streak(1, &conn).unwrap();
        delete_group_stats_for_deck(Some(merged.id), "joint", None, 1, &conn).unwrap();
        let (without, _) = crate::crud::scheduling::get_plan_streak(1, &conn).unwrap();
        assert_eq!(with_copy, 1);
        assert_eq!(without, 0, "the archived copy never propped the streak up");
    }

    // Manual archiving

    fn archived_at(conn: &Connection, origin: i64, version: i64) -> (i64, i64) {
        conn.query_row(
            "SELECT SUM(is_archived), SUM(NOT is_archived) FROM group_stats
             WHERE origin_group_id = ?1 AND version = ?2",
            rusqlite::params![origin, version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn t27_archiving_a_version_drops_it_from_the_total() {
        use crate::crud::update::set_group_stats_archived_for_deck;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);

        assert_eq!(plan_total(&conn, 1), 13);
        set_group_stats_archived_for_deck(Some(1), "deck a", Some(1), 1, true, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 8, "v1 no longer counts");
        assert_eq!(new_at(&conn, 1, 1, 1), 5, "but it is still there");
    }

    #[test]
    fn t28_unarchiving_restores_the_total() {
        use crate::crud::update::set_group_stats_archived_for_deck;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);

        set_group_stats_archived_for_deck(Some(1), "deck a", None, 1, true, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 0);
        set_group_stats_archived_for_deck(Some(1), "deck a", None, 1, false, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 5, "unarchive brings it back");
    }

    #[test]
    fn t29_archiving_the_deck_covers_every_version() {
        use crate::crud::update::set_group_stats_archived_for_deck;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);

        set_group_stats_archived_for_deck(Some(1), "deck a", None, 1, true, &conn).unwrap();
        assert_eq!(archived_at(&conn, 1, 1), (1, 0));
        assert_eq!(archived_at(&conn, 1, 2), (1, 0));
        assert_eq!(plan_total(&conn, 1), 0);
    }

    #[test]
    fn t30_archiving_one_row_leaves_its_siblings_counting() {
        use crate::crud::scheduling::open_stat_line;
        use crate::crud::update::set_group_stat_archived;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        // A second day's line, opened the way studying opens it, still in the plan
        roll_day(&conn);
        open_stat_line(1, &conn).unwrap();
        study(&conn, 1, 3);

        let first: i64 = conn
            .query_row("SELECT MIN(id) FROM group_stats WHERE group_id = 1", [], |r| r.get(0))
            .unwrap();
        set_group_stat_archived(first, true, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 3, "only the archived day drops out");
    }

    #[test]
    fn t31_archiving_is_scoped_to_the_plan() {
        use crate::crud::update::set_group_stats_archived_for_deck;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 20);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 7);

        set_group_stats_archived_for_deck(Some(1), "deck a", None, 1, true, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 0);
        assert_eq!(plan_total(&conn, 2), 7, "the other plan is untouched");
    }
}
