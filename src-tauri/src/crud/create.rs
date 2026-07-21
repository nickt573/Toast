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
    archive_sources: bool,
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

    // Without a reset the new deck inherits both sources' still-counting study,
    // combined per plan and date, and every source row is archived so the copy is the
    // only thing counting. This is a one-time migration: unarchiving a source later
    // brings that study back on its own terms and never re-syncs the copy.
    //
    // With a reset the new deck starts empty and nothing is copied, so the sources
    // keep counting unless archive_sources asks otherwise. Archiving happens inside
    // this transaction either way: a failed merge must not leave the sources archived
    // with no copy to show for it. archive_sources only means anything with a reset,
    // since without one the copy makes archiving mandatory.
    if !reset {
        tx.execute(
            r#"
            INSERT INTO group_stats (group_id, origin_group_id, plan_id, plan_name, group_name, date,
                                     num_promote, num_demote, num_new, time_spent_minutes, retention_rate)
            SELECT ?1, ?1, plan_id, MAX(plan_name), ?2, date,
                   SUM(num_promote), SUM(num_demote), SUM(num_new), SUM(time_spent_minutes),
                   CASE WHEN SUM(num_promote) + SUM(num_demote) > 0
                        THEN CAST(SUM(num_promote) AS REAL) / (SUM(num_promote) + SUM(num_demote))
                        ELSE 0.0 END
            FROM group_stats
            WHERE group_id IN (?3, ?4) AND is_archived = FALSE
            GROUP BY plan_id, date
            HAVING SUM(num_promote) + SUM(num_demote) + SUM(num_new) > 0
                OR SUM(time_spent_minutes) > 0
            "#,
            rusqlite::params![new_deck_id, new_name, deck_a_id, deck_b_id],
        )?;
    }

    if !reset || archive_sources {
        tx.execute(
            "UPDATE group_stats SET is_archived = TRUE WHERE group_id IN (?1, ?2)",
            rusqlite::params![deck_a_id, deck_b_id],
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

    tx.commit()?;

    // Joining a plan writes nothing. The deck shows up on the stats page with an
    // empty table, and its first row appears when a session is actually opened.
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

// A handful of rules have to hold no matter what order things happen in, so
// this checks all of them after every step of every sequence.
#[cfg(test)]
mod stat_row_invariant_tests {
    use super::*;
    use crate::crud::{
        delete::remove_group_from_plan,
        scheduling::{open_stat_line, reset_deck},
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

    fn rows(conn: &Connection, deck: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM group_stats WHERE group_id = ?1",
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
                // Opening the session is what creates the line, out of a plan there
                // is nowhere to write and the whole step is a no-op
                let line = open_stat_line(deck, conn).unwrap()?;
                conn.execute(
                    "UPDATE group_stats SET num_new = num_new + 1 WHERE id = ?1",
                    [line],
                )
                .unwrap();
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
                    false,
                    conn,
                )
                .unwrap();
                return Some(merged.id);
            }
        }
        Some(deck)
    }

    // One ordinary line per deck, plan, and day. A day can hold more than one line,
    // but only because a reset split it, and those extras are marked as run starts.
    fn check_one_plain_line_per_day(conn: &Connection, trace: &str) {
        let dupes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (
                    SELECT 1 FROM group_stats
                    WHERE starts_era = FALSE
                    GROUP BY origin_group_id, plan_id, date
                    HAVING COUNT(*) > 1
                 )",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dupes, 0, "duplicate plain line for one day after {trace}");
    }

    // Opening a session is the only thing that writes a line. Joining a plan,
    // leaving one, resetting, and the day rolling over all leave the count alone.
    fn check_only_sessions_write(before: i64, now: i64, op: Op, trace: &str) {
        if matches!(op, Op::Study) {
            return;
        }
        assert_eq!(now, before, "line count moved without a session after {trace}");
    }

    // A session inside a plan always consumes a pending reset, so the flag can never
    // outlive the line it was meant to open.
    fn check_sessions_clear_the_reset_flag(conn: &Connection, deck: i64, op: Op, trace: &str) {
        if !matches!(op, Op::Study) || !in_plan(conn, deck) {
            return;
        }
        let flag: bool = conn
            .query_row(
                r#"SELECT was_reset FROM "group" WHERE id = ?1"#,
                [deck],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!flag, "reset flag survived a session after {trace}");
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
            let mut done: Vec<Op> = Vec::new();

            for &op in seq.iter() {
                let carried = matches!(op, Op::Merge | Op::MergeReset);
                let before = rows(&conn, deck);
                let Some(next) = apply(&mut conn, deck, op) else {
                    continue;
                };
                done.push(op);
                let trace = format!("{done:?}");

                // A merge starts a new deck, so the line count baseline restarts too
                if carried {
                    deck = next;
                } else {
                    check_only_sessions_write(before, rows(&conn, deck), op, &trace);
                }

                check_one_plain_line_per_day(&conn, &trace);
                check_sessions_clear_the_reset_flag(&conn, deck, op, &trace);
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
mod stat_row_tests {
    use super::*;
    use crate::crud::{
        delete::{delete_group_stats, remove_group_from_plan},
        read::get_group_stats,
        scheduling::{archive_deck_stats, open_stat_line, reset_deck},
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

    // Opening the session is what builds the line, so every test studies through here
    fn study(conn: &Connection, deck: i64, n: i64) {
        let line = open_stat_line(deck, conn).unwrap().expect("deck is not in a plan");
        conn.execute(
            "UPDATE group_stats SET num_new = num_new + ?2 WHERE id = ?1",
            rusqlite::params![line, n],
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

    fn era_rows(conn: &Connection, deck: i64) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM group_stats WHERE group_id = ?1 AND starts_era = TRUE",
            [deck],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn flag(conn: &Connection, deck: i64) -> bool {
        conn.query_row(r#"SELECT was_reset FROM "group" WHERE id = ?1"#, [deck], |r| r.get(0))
            .unwrap()
    }

    fn new_in(conn: &Connection, origin: i64, plan: i64) -> i64 {
        conn.query_row(
            "SELECT COALESCE(SUM(num_new), 0) FROM group_stats
             WHERE origin_group_id = ?1 AND plan_id = ?2",
            rusqlite::params![origin, plan],
            |r| r.get(0),
        )
        .unwrap()
    }

    // What the plan actually counts, which is every line that isn't archived
    fn plan_total(conn: &Connection, plan: i64) -> i64 {
        get_group_stats(plan, conn)
            .unwrap()
            .iter()
            .filter(|r| !r.is_archived)
            .map(|r| r.num_new)
            .sum()
    }

    // The rows behind one deck card on the stats page, which is what that page hands
    // to the bulk delete and archive commands. Live and dead decks are separate cards
    // even when they share an id, so group_id is part of the pick.
    fn card_rows(conn: &Connection, origin: i64, plan: i64, dead: bool) -> Vec<i64> {
        conn.prepare(
            "SELECT id FROM group_stats
             WHERE plan_id = ?1 AND origin_group_id = ?2 AND (group_id IS NULL) = ?3",
        )
        .unwrap()
        .query_map(rusqlite::params![plan, origin, dead], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    fn archived_split(conn: &Connection, origin: i64) -> (i64, i64) {
        conn.query_row(
            "SELECT COALESCE(SUM(is_archived), 0), COALESCE(SUM(NOT is_archived), 0)
             FROM group_stats WHERE origin_group_id = ?1",
            [origin],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    // Line lifecycle

    #[test]
    fn t01_joining_a_plan_writes_nothing() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        assert_eq!(rows(&conn, 1), 0, "the deck shows up with an empty table");
    }

    #[test]
    fn t02_opening_a_session_builds_the_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        assert_eq!(rows(&conn, 1), 1);
        assert_eq!(new_in(&conn, 1, 1), 5);
    }

    #[test]
    fn t03_a_second_session_the_same_day_reuses_the_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        study(&conn, 1, 3);
        assert_eq!(rows(&conn, 1), 1);
        assert_eq!(new_in(&conn, 1, 1), 8);
    }

    #[test]
    fn t04_a_new_day_opens_its_own_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        roll_day(&conn);
        study(&conn, 1, 3);
        assert_eq!(rows(&conn, 1), 2);
    }

    #[test]
    fn t05_out_of_a_plan_a_session_writes_nothing() {
        let conn = setup();
        assert!(open_stat_line(1, &conn).unwrap().is_none());
        assert_eq!(rows(&conn, 1), 0);
    }

    #[test]
    fn t06_leaving_a_plan_keeps_the_lines() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        assert_eq!(rows(&conn, 1), 1, "study already done is real history");
        assert_eq!(new_in(&conn, 1, 1), 5);
    }

    #[test]
    fn t07_a_deck_never_studied_leaves_nothing_behind() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 1);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        assert_eq!(rows(&conn, 1), 0, "add and remove cycles write nothing at all");
    }

    #[test]
    fn t08_rejoining_the_same_day_resumes_the_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 1);
        study(&conn, 1, 3);
        assert_eq!(rows(&conn, 1), 1);
        assert_eq!(new_in(&conn, 1, 1), 8);
    }

    #[test]
    fn t09_each_plan_keeps_its_own_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 3);
        assert_eq!(new_in(&conn, 1, 1), 5);
        assert_eq!(new_in(&conn, 1, 2), 3);
    }

    // Resets

    #[test]
    fn t10_a_reset_writes_nothing_by_itself() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        assert_eq!(rows(&conn, 1), 1, "the flag is the whole record until a session");
        assert!(flag(&conn, 1));
    }

    #[test]
    fn t11_the_session_after_a_reset_starts_its_own_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);
        assert_eq!(rows(&conn, 1), 2, "same day, but a new run");
        assert_eq!(era_rows(&conn, 1), 1);
        assert!(!flag(&conn, 1), "the flag clears once it has been spent");
        assert_eq!(new_in(&conn, 1, 1), 13, "the old line keeps its 5");
    }

    #[test]
    fn t12_studying_on_after_a_reset_stays_in_the_new_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);
        study(&conn, 1, 2);
        assert_eq!(rows(&conn, 1), 2, "no third line");
    }

    #[test]
    fn t13_repeat_resets_collapse_into_one_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        for _ in 0..10 {
            reset_deck(1, &conn).unwrap();
        }
        study(&conn, 1, 8);
        assert_eq!(rows(&conn, 1), 2, "ten resets, one boundary");
        assert_eq!(era_rows(&conn, 1), 1);
    }

    #[test]
    fn t14_a_reset_survives_leaving_and_rejoining() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, true, &mut conn).unwrap();
        assert!(flag(&conn, 1));
        add(&mut conn, 1, 1);
        study(&conn, 1, 8);
        assert_eq!(era_rows(&conn, 1), 1, "the reset lands on the first session back");
    }

    #[test]
    fn t15_a_reset_cannot_be_laundered_by_waiting_a_day() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        roll_day(&conn);
        study(&conn, 1, 8);
        assert_eq!(era_rows(&conn, 1), 1, "a new day does not absorb the reset");
    }

    #[test]
    fn t16_resetting_outside_a_plan_still_applies_on_return() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        for _ in 0..5 {
            reset_deck(1, &conn).unwrap();
        }
        assert_eq!(rows(&conn, 1), 1, "nowhere to write while out of a plan");
        add(&mut conn, 1, 1);
        study(&conn, 1, 8);
        assert_eq!(era_rows(&conn, 1), 1);
    }

    // Archiving

    #[test]
    fn t17_archiving_the_deck_covers_every_plan() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 3);

        archive_deck_stats(1, &conn).unwrap();
        assert_eq!(archived_split(&conn, 1), (2, 0));
        assert_eq!(plan_total(&conn, 1), 0);
        assert_eq!(plan_total(&conn, 2), 0);
    }

    #[test]
    fn t18_an_archived_line_stops_counting_but_stays() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        archive_deck_stats(1, &conn).unwrap();
        assert_eq!(rows(&conn, 1), 1, "still on the page");
        assert_eq!(plan_total(&conn, 1), 0);
    }

    #[test]
    fn t19_unarchiving_restores_the_total() {
        use crate::crud::update::set_group_stats_archived;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        archive_deck_stats(1, &conn).unwrap();
        set_group_stats_archived(&card_rows(&conn, 1, 1, false), false, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 5);
    }

    #[test]
    fn t20_archiving_one_line_leaves_its_siblings_counting() {
        use crate::crud::update::set_group_stat_archived;
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        roll_day(&conn);
        study(&conn, 1, 3);
        let first: i64 = conn
            .query_row("SELECT MIN(id) FROM group_stats WHERE group_id = 1", [], |r| r.get(0))
            .unwrap();
        set_group_stat_archived(first, true, &conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 3);
    }

    // Merging

    #[test]
    fn t21_merge_migrates_the_study_into_the_new_deck() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), false, false, &mut conn).unwrap();
        assert_eq!(new_in(&conn, merged.id, 1), 8, "5 + 3 on the same day");
    }

    #[test]
    fn t22_merge_archives_the_sources_entirely() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        roll_day(&conn);
        study(&conn, 1, 4);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        merge_decks(1, 2, "joint".into(), false, false, &mut conn).unwrap();
        assert_eq!(archived_split(&conn, 1), (2, 0), "every line, not just the copied ones");
        assert_eq!(archived_split(&conn, 2), (1, 0));
    }

    #[test]
    fn t23_merge_does_not_double_count_the_plan() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        merge_decks(1, 2, "joint".into(), false, false, &mut conn).unwrap();
        assert_eq!(plan_total(&conn, 1), 8, "the copy counts, the sources don't");
    }

    #[test]
    fn t24_merge_skips_source_lines_already_archived() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        archive_deck_stats(1, &conn).unwrap();
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), false, false, &mut conn).unwrap();
        assert_eq!(new_in(&conn, merged.id, 1), 3, "the archived 5 stays behind");
    }

    #[test]
    fn t25_merge_with_reset_copies_and_archives_nothing() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), true, false, &mut conn).unwrap();
        assert_eq!(rows(&conn, merged.id), 0, "the new deck starts empty");
        assert_eq!(archived_split(&conn, 1), (0, 1), "the caller decides, not the merge");
        assert_eq!(plan_total(&conn, 1), 8, "so the sources keep counting");
    }

    #[test]
    fn t26_merge_splits_across_plans() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 4);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), false, false, &mut conn).unwrap();
        assert_eq!(new_in(&conn, merged.id, 1), 8, "plan one keeps its own portion");
        assert_eq!(new_in(&conn, merged.id, 2), 4);
    }

    #[test]
    fn t27_a_merged_deck_can_be_merged_again() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();
        let first = merge_decks(1, 2, "joint".into(), false, false, &mut conn).unwrap();

        let third = create_deck("deck c".into(), &mut conn).unwrap().id;
        add(&mut conn, third, 1);
        study(&conn, third, 2);
        remove_group_from_plan(third, false, &mut conn).unwrap();

        let second = merge_decks(first.id, third, "joint two".into(), false, false, &mut conn).unwrap();
        assert_eq!(new_in(&conn, second.id, 1), 10, "8 carried forward plus 2");
        assert_eq!(plan_total(&conn, 1), 10, "and still counted once");
    }

    // Deleting

    #[test]
    fn t28_deleting_a_decks_stats_clears_that_plan_only() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 1, 2);
        study(&conn, 1, 3);

        delete_group_stats(&card_rows(&conn, 1, 1, false), &conn).unwrap();
        assert_eq!(new_in(&conn, 1, 1), 0);
        assert_eq!(new_in(&conn, 1, 2), 3, "the other plan is untouched");
    }

    #[test]
    fn t29_deleting_clears_every_run_of_the_deck() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        reset_deck(1, &conn).unwrap();
        study(&conn, 1, 8);

        delete_group_stats(&card_rows(&conn, 1, 1, false), &conn).unwrap();
        assert_eq!(rows(&conn, 1), 0, "runs are lines, not a separate scope");
    }

    // On a database old enough to have reused an id before the migration reserved it,
    // a dead deck and a live one can share origin_group_id. The stats page draws them
    // as separate cards, so clearing one must not reach into the other.
    #[test]
    fn t30_clearing_a_dead_decks_stats_leaves_a_live_deck_on_the_same_id() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        // deck 1 deleted, then its id handed to a new deck that also gets studied
        conn.execute("UPDATE group_stats SET group_id = NULL WHERE group_id = 1", [])
            .unwrap();
        study(&conn, 1, 7);

        delete_group_stats(&card_rows(&conn, 1, 1, true), &conn).unwrap();
        assert_eq!(new_in(&conn, 1, 1), 7, "only the dead deck's history goes");

        delete_group_stats(&card_rows(&conn, 1, 1, false), &conn).unwrap();
        assert_eq!(new_in(&conn, 1, 1), 0);
    }

    #[test]
    fn t31_same_named_decks_stay_separate() {
        let mut conn = setup();
        conn.execute(
            "UPDATE \"group\" SET name = 'deck a' WHERE id = 2",
            [],
        )
        .unwrap();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);

        delete_group_stats(&card_rows(&conn, 1, 1, false), &conn).unwrap();
        assert_eq!(new_in(&conn, 2, 1), 3, "origin id keeps them apart, not the name");
    }

    #[test]
    fn t32_studying_after_archiving_today_opens_a_fresh_line() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        archive_deck_stats(1, &conn).unwrap();
        study(&conn, 1, 3);
        assert_eq!(rows(&conn, 1), 2, "an archived line is never written into");
        assert_eq!(plan_total(&conn, 1), 3, "so the new study still counts");
        assert_eq!(archived_split(&conn, 1), (1, 1), "and the old line stays archived");
    }

    #[test]
    fn t33_merge_with_reset_can_archive_the_sources_in_the_same_stroke() {
        let mut conn = setup();
        add(&mut conn, 1, 1);
        study(&conn, 1, 5);
        remove_group_from_plan(1, false, &mut conn).unwrap();
        add(&mut conn, 2, 1);
        study(&conn, 2, 3);
        remove_group_from_plan(2, false, &mut conn).unwrap();

        let merged = merge_decks(1, 2, "joint".into(), true, true, &mut conn).unwrap();
        assert_eq!(rows(&conn, merged.id), 0, "the new deck still starts empty");
        assert_eq!(archived_split(&conn, 1), (1, 0));
        assert_eq!(archived_split(&conn, 2), (1, 0));
        assert_eq!(plan_total(&conn, 1), 0, "nothing counts once both sources are archived");
    }
}
