use crate::crud::models::*;
use rusqlite::{Connection, Result};

pub fn get_plans(conn: &Connection) -> Result<Vec<Plan>> {
    conn.prepare("SELECT id, name FROM plan ORDER BY name COLLATE NOCASE ASC")?
        .query_map([], |row| {
            Ok(Plan {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect()
}

/// One summary row per plan: (plan_id, todo_count, resource_count, linked_deck_count).
pub fn get_plan_summaries(conn: &Connection) -> Result<Vec<(i64, i64, i64, i64)>> {
    conn.prepare(
        r#"
        SELECT
            p.id,
            (SELECT COUNT(*) FROM todo t WHERE t.plan_id = p.id),
            (SELECT COUNT(*) FROM resource r WHERE r.plan_id = p.id),
            (SELECT COUNT(*) FROM "group" g WHERE g.plan_id = p.id AND g.group_type = 'deck')
        FROM plan p
        "#,
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
    .collect()
}

pub fn get_todos(plan_id: i64, conn: &Connection) -> Result<Vec<Todo>> {
    conn.prepare(
        r#"
        SELECT id, plan_id, text, frequency, category, is_done, is_disabled, is_skipped, position
        FROM todo
        WHERE plan_id = ?1
        ORDER BY CASE WHEN position IS NULL THEN 1 ELSE 0 END, position ASC, text COLLATE NOCASE ASC
        "#,
    )?
    .query_map([plan_id], |row| {
        Ok(Todo {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            text: row.get(2)?,
            frequency: row.get(3)?,
            category: row.get(4)?,
            is_done: row.get(5)?,
            is_disabled: row.get(6)?,
            is_skipped: row.get(7)?,
            position: row.get(8)?,
        })
    })?
    .collect()
}

pub fn get_groups(conn: &Connection) -> Result<Vec<Group>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, plan_id, name, group_type
        FROM "group"
        ORDER BY group_type ASC, name COLLATE NOCASE ASC
        "#,
    )?;

    let groups = stmt.query_map([], |row| {
        let group_type_str: String = row.get(3)?;

        let group_type = match group_type_str.as_str() {
            "deck" => GroupType::Deck,
            "notebook" => GroupType::Notebook,
            other => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unknown group_type: {}", other),
                    )),
                ));
            }
        };

        Ok(Group {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            name: row.get(2)?,
            group_type,
        })
    })?;

    groups.collect()
}

pub fn get_decks(conn: &Connection) -> Result<Vec<Group>> {
    conn.prepare(
        r#"
        SELECT
            id,
            plan_id,
            name
        FROM "group"
        WHERE
            group_type = 'deck'
        ORDER BY name COLLATE NOCASE ASC
        "#,
    )?
    .query_map([], |row| {
        Ok(Group {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            name: row.get(2)?,
            group_type: GroupType::Deck,
        })
    })?
    .collect()
}

// Column order is load-bearing: card_from_row indexes into it.
const CARD_COLUMNS: &str = r#"
    id, group_id, front, back, is_searchable, support,
    imported_front, imported_back, imported_support,
    front_image, back_image, front_audio, back_audio, is_uploaded,
    tier, ease, sequence, is_due, is_overdue, is_paused, position, is_cram
"#;

fn card_from_row(row: &rusqlite::Row) -> Result<Card> {
    Ok(Card {
        id: row.get(0)?,
        group_id: row.get(1)?,
        front: row.get(2)?,
        back: row.get(3)?,
        is_searchable: row.get(4)?,
        support: row.get(5)?,
        imported_front: row.get(6)?,
        imported_back: row.get(7)?,
        imported_support: row.get(8)?,
        front_image: row.get(9)?,
        back_image: row.get(10)?,
        front_audio: row.get(11)?,
        back_audio: row.get(12)?,
        is_uploaded: row.get(13)?,
        tier: row.get(14)?,
        ease: row.get(15)?,
        sequence: row.get(16)?,
        is_due: row.get(17)?,
        is_overdue: row.get(18)?,
        is_paused: row.get(19)?,
        position: row.get(20)?,
        is_cram: row.get(21)?,
    })
}

pub fn get_card(card_id: i64, conn: &Connection) -> Result<Card> {
    conn.query_row(
        &format!("SELECT {CARD_COLUMNS} FROM card WHERE id = ?1"),
        [card_id],
        card_from_row,
    )
}

/// Picks the next card for a study session. Preference order: a due card other than
/// the one just shown; the just-shown due card if it is the only one left (a repeat
/// beats dropping into cram early, so cram never appears while any due card remains);
/// then the same two steps for the cram pool.
pub fn next_session_card(
    conn: &Connection,
    group_id: i64,
    exclude_id: Option<i64>,
) -> Result<Option<Card>> {
    let attempts: [(&str, bool); 4] = [
        ("is_due = TRUE", true),
        ("is_due = TRUE", false),
        ("is_cram = TRUE", true),
        ("is_cram = TRUE", false),
    ];
    for (pool_clause, use_exclude) in attempts {
        let exclude = use_exclude && exclude_id.is_some();
        let exclude_clause = if exclude { "AND id != ?2" } else { "" };
        let sql = format!(
            "SELECT {CARD_COLUMNS} FROM card
             WHERE group_id = ?1 AND {pool_clause} AND is_paused = FALSE {exclude_clause}
             ORDER BY RANDOM() LIMIT 1"
        );
        let result = if exclude {
            conn.query_row(&sql, rusqlite::params![group_id, exclude_id.unwrap()], card_from_row)
        } else {
            conn.query_row(&sql, rusqlite::params![group_id], card_from_row)
        };
        match result {
            Ok(card) => return Ok(Some(card)),
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

pub fn get_cards(deck_id: i64, conn: &Connection) -> Result<Vec<Card>> {
    conn.prepare(&format!(
        r#"
        SELECT {CARD_COLUMNS}
        FROM card
        WHERE group_id = ?1
        ORDER BY CASE WHEN position IS NULL THEN 1 ELSE 0 END, position ASC, id ASC
        "#
    ))?
    .query_map([deck_id], card_from_row)?
    .collect()
}

pub fn get_card_last_seen_dates(deck_id: i64, conn: &Connection) -> Result<Vec<(i64, String)>> {
    conn.prepare(
        r#"
        SELECT card.id, MAX(cgl.graded_at) AS last_seen
        FROM card
        JOIN card_grade_log cgl ON card.id = cgl.card_id
        WHERE card.group_id = ?1
        GROUP BY card.id
        "#,
    )?
    .query_map([deck_id], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// (deck_id, card_count) for every deck, including decks with no cards.
pub fn get_deck_card_counts(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    conn.prepare(
        r#"
        SELECT g.id, COUNT(c.id)
        FROM "group" g
        LEFT JOIN card c ON c.group_id = g.id
        WHERE g.group_type = 'deck'
        GROUP BY g.id
        "#,
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

pub fn get_notebooks(conn: &Connection) -> Result<Vec<Group>> {
    conn.prepare(
        r#"
        SELECT
            id,
            plan_id,
            name
        FROM "group"
        WHERE
            group_type = 'notebook'
        ORDER BY name COLLATE NOCASE ASC
        "#,
    )?
    .query_map([], |row| {
        Ok(Group {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            name: row.get(2)?,
            group_type: GroupType::Notebook,
        })
    })?
    .collect()
}

pub fn get_pages(notebook_id: i64, conn: &Connection) -> Result<Vec<Page>> {
    conn.prepare(
        r#"
        SELECT id, group_id, title, description, content, audio_file, created_date
        FROM page
        WHERE group_id = ?1
        ORDER BY created_date ASC, id ASC
        "#,
    )?
    .query_map([notebook_id], |row| {
        Ok(Page {
            id: row.get(0)?,
            group_id: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            content: row.get(4)?,
            audio_file: row.get(5)?,
            created_date: row.get(6)?,
        })
    })?
    .collect()
}

/// (deck_id, new_total, review_total) for every deck.
pub fn get_deck_srs_summaries(conn: &Connection) -> Result<Vec<(i64, i64, i64)>> {
    conn.prepare(
        r#"
        SELECT g.id,
            COUNT(c.id) FILTER (WHERE c.tier = 0),
            COUNT(c.id) FILTER (WHERE c.tier > 0)
        FROM "group" g
        LEFT JOIN card c ON c.group_id = g.id
        WHERE g.group_type = 'deck'
        GROUP BY g.id
        "#,
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
    .collect()
}

/// (notebook_id, page_count) for every notebook, including empty notebooks.
pub fn get_notebook_page_counts(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    conn.prepare(
        r#"
        SELECT g.id, COUNT(p.id)
        FROM "group" g
        LEFT JOIN page p ON p.group_id = g.id
        WHERE g.group_type = 'notebook'
        GROUP BY g.id
        "#,
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// Returns all groups in a plan that have a scheduler, along with the scheduler.
pub fn get_plan_srs_groups(plan_id: i64, conn: &Connection) -> Result<Vec<(Group, Scheduler)>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            g.id, g.plan_id, g.name, g.group_type,
            s.group_id, s.studied_new, s.max_new,
            s.studied_review, s.max_review, s.can_overflow
        FROM "group" g
        INNER JOIN scheduler s ON s.group_id = g.id
        WHERE g.plan_id = ?1
        ORDER BY g.name COLLATE NOCASE ASC
        "#,
    )?;

    let results = stmt.query_map([plan_id], |row| {
        let group_type_str: String = row.get(3)?;
        let group_type = match group_type_str.as_str() {
            "deck" => GroupType::Deck,
            "notebook" => GroupType::Notebook,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };

        let group = Group {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            name: row.get(2)?,
            group_type,
        };

        let scheduler = Scheduler {
            group_id: row.get(4)?,
            studied_new: row.get(5)?,
            max_new: row.get(6)?,
            studied_review: row.get(7)?,
            max_review: row.get(8)?,
            can_overflow: row.get(9)?,
        };

        Ok((group, scheduler))
    })?;

    results.collect()
}

/// Returns all groups not currently assigned to any plan, available to add to a plan.
pub fn get_unassigned_groups(conn: &Connection) -> Result<Vec<Group>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, plan_id, name, group_type
        FROM "group"
        WHERE plan_id IS NULL
            AND group_type = 'deck'
        ORDER BY name COLLATE NOCASE ASC
        "#,
    )?;

    let results = stmt.query_map([], |row| {
        Ok(Group {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            name: row.get(2)?,
            group_type: GroupType::Deck,
        })
    })?;

    results.collect()
}

pub fn get_resources(plan_id: i64, conn: &Connection) -> Result<Vec<Resource>> {
    let mut stmt = conn.prepare(
        "SELECT id, plan_id, name, type, url, notes FROM resource WHERE plan_id = ?1 ORDER BY name ASC"
    )?;
    let rows = stmt
        .query_map([plan_id], |row| {
            Ok(Resource {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                name: row.get(2)?,
                resource_type: row.get(3)?,
                url: row.get(4)?,
                notes: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn get_todo_resources(todo_id: i64, conn: &Connection) -> Result<Vec<Resource>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT r.id, r.plan_id, r.name, r.type, r.url, r.notes
        FROM resource r
        INNER JOIN todo_resource tr ON tr.resource_id = r.id
        WHERE tr.todo_id = ?1
        ORDER BY r.name ASC
        "#,
    )?;
    let rows = stmt
        .query_map([todo_id], |row| {
            Ok(Resource {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                name: row.get(2)?,
                resource_type: row.get(3)?,
                url: row.get(4)?,
                notes: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn get_todo_groups(todo_id: i64, conn: &Connection) -> Result<Vec<Group>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT g.id, g.plan_id, g.name, g.group_type
        FROM "group" g
        INNER JOIN todo_group tg ON tg.group_id = g.id
        WHERE tg.todo_id = ?1
        ORDER BY g.group_type ASC, g.name COLLATE NOCASE ASC
        "#,
    )?;
    let rows = stmt
        .query_map([todo_id], |row| {
            let group_type_str: String = row.get(3)?;
            let group_type = match group_type_str.as_str() {
                "deck" => GroupType::Deck,
                _ => GroupType::Notebook,
            };
            Ok(Group {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                name: row.get(2)?,
                group_type,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// group_name is a snapshot taken when the session was logged, so a later rename or
/// merge leaves it stale. Prefer the live deck's name and keep the snapshot as the
/// fallback for decks that no longer exist, the same way todo stat groups do.
pub fn get_group_stats(plan_id: i64, conn: &Connection) -> Result<Vec<GroupStat>> {
    conn.prepare(
        r#"
        SELECT gs.id, gs.group_id, gs.origin_group_id, gs.plan_id, gs.plan_name,
               COALESCE(g.name, gs.group_name), gs.date,
               gs.num_promote, gs.num_demote, gs.num_new, gs.time_spent_minutes, gs.retention_rate,
               gs.starts_era, gs.is_merged, gs.is_archived
        FROM group_stats gs
        LEFT JOIN "group" g ON g.id = gs.group_id
        WHERE gs.plan_id = ?1
        ORDER BY gs.date DESC, gs.id DESC
        "#,
    )?
    .query_map([plan_id], |row| {
        Ok(GroupStat {
            id: row.get(0)?,
            group_id: row.get(1)?,
            origin_group_id: row.get(2)?,
            plan_id: row.get(3)?,
            plan_name: row.get(4)?,
            group_name: row.get(5)?,
            date: row.get(6)?,
            num_promote: row.get(7)?,
            num_demote: row.get(8)?,
            num_new: row.get(9)?,
            time_spent_minutes: row.get(10)?,
            retention_rate: row.get(11)?,
            starts_era: row.get(12)?,
            is_merged: row.get(13)?,
            is_archived: row.get(14)?,
        })
    })?
    .collect()
}

/// A deleted plan's name only survives in its stat rows, and renaming a plan never
/// rewrote them, so a plan renamed mid-life left rows under several names. DISTINCT
/// then handed back one pair per name and the tab drew a pill for each. Group by the
/// plan and keep the name from its most recent recorded day.
pub fn get_deleted_plan_ids(conn: &Connection) -> Result<Vec<(i64, String)>> {
    conn.prepare(
        r#"
        SELECT plan_id, plan_name FROM (
            SELECT plan_id, plan_name, date FROM group_stats
            UNION ALL
            SELECT plan_id, plan_name, date FROM todo_stats
        )
        WHERE plan_id NOT IN (SELECT id FROM plan)
        GROUP BY plan_id
        HAVING date = MAX(date)
        "#,
    )?
    .query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?
    .collect()
}

pub fn get_todo_stats(plan_id: i64, conn: &Connection) -> Result<Vec<TodoStat>> {
    let rows: Vec<(i64, Option<i64>, i64, String, String, String, String, Option<String>, f64, Option<String>)> = conn
        .prepare(
            r#"
            SELECT id, todo_id, plan_id, plan_name, date, text, category, details, time_spent_minutes, num_unit
            FROM todo_stats
            WHERE plan_id = ?1
            ORDER BY date DESC, id ASC
            "#,
        )?
        .query_map([plan_id], |row| {
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
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Batch-load all groups and resources for this plan's stats in two queries
    use std::collections::HashMap;

    let mut stat_groups: HashMap<i64, Vec<TodoStatGroup>> = HashMap::new();
    conn.prepare(
        r#"
        SELECT tsg.stat_id, tsg.rowid, tsg.group_id, COALESCE(g.name, tsg.group_name), COALESCE(g.group_type, tsg.group_type)
        FROM todo_stat_group tsg
        LEFT JOIN "group" g ON g.id = tsg.group_id
        WHERE tsg.stat_id IN (SELECT id FROM todo_stats WHERE plan_id = ?1)
        "#,
    )?
    .query_map([plan_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            TodoStatGroup {
                row_id: row.get::<_, i64>(1)?,
                group_id: row.get::<_, Option<i64>>(2)?,
                name: row.get::<_, String>(3)?,
                group_type: row.get::<_, Option<String>>(4)?,
            },
        ))
    })?
    .filter_map(|r| r.ok())
    .for_each(|(sid, group)| stat_groups.entry(sid).or_default().push(group));

    // Resources persist their full info (url/type/notes) like the name does:
    // live values win via COALESCE while the resource exists; the snapshot
    // remains after it's deleted (resource_id goes NULL).
    let mut stat_resources: HashMap<i64, Vec<StatResource>> = HashMap::new();
    conn.prepare(
        r#"
        SELECT tsr.stat_id, tsr.rowid,
               COALESCE(r.name, tsr.resource_name),
               COALESCE(r.url, tsr.resource_url),
               COALESCE(r."type", tsr.resource_type),
               COALESCE(r.notes, tsr.resource_notes)
        FROM todo_stat_resource tsr
        LEFT JOIN resource r ON r.id = tsr.resource_id
        WHERE tsr.stat_id IN (SELECT id FROM todo_stats WHERE plan_id = ?1)
        "#,
    )?
    .query_map([plan_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            StatResource {
                row_id: row.get::<_, i64>(1)?,
                name: row.get::<_, String>(2)?,
                url: row.get::<_, Option<String>>(3)?,
                resource_type: row.get::<_, Option<String>>(4)?,
                notes: row.get::<_, Option<String>>(5)?,
            },
        ))
    })?
    .filter_map(|r| r.ok())
    .for_each(|(sid, res)| stat_resources.entry(sid).or_default().push(res));

    let result = rows
        .into_iter()
        .map(
            |(
                id,
                todo_id,
                plan_id,
                plan_name,
                date,
                text,
                category,
                details,
                time_spent_minutes,
                num_unit,
            )| {
                TodoStat {
                    id,
                    todo_id,
                    plan_id,
                    plan_name,
                    date,
                    text,
                    category,
                    details,
                    time_spent_minutes,
                    num_unit,
                    groups: stat_groups.remove(&id).unwrap_or_default(),
                    resources: stat_resources.remove(&id).unwrap_or_default(),
                }
            },
        )
        .collect();

    Ok(result)
}

pub fn get_card_grade_log(card_id: i64, conn: &Connection) -> Result<Vec<CardGradeLog>> {
    conn.prepare(
        "SELECT id, card_id, grade, graded_at, old_tier, new_tier FROM card_grade_log WHERE card_id = ?1 ORDER BY id DESC LIMIT 200",
    )?
    .query_map([card_id], |row| {
        Ok(CardGradeLog {
            id: row.get(0)?,
            card_id: row.get(1)?,
            grade: row.get(2)?,
            graded_at: row.get(3)?,
            old_tier: row.get(4)?,
            new_tier: row.get(5)?,
        })
    })?
    .collect()
}

#[cfg(test)]
mod cram_tests {
    use super::next_session_card;
    use crate::crud::create::add_group_to_plan;
    use crate::crud::models::NewScheduler;
    use crate::crud::read::get_card;
    use crate::crud::scheduling::{count_due_items, grade_item, mark_for_review, update_date};
    use crate::db::init_schema;
    use rusqlite::Connection;
    use std::path::PathBuf;

    fn counts(conn: &Connection) -> (i64, i64, i64) {
        count_due_items(&1, conn).unwrap()
    }

    fn is_cram(conn: &Connection, id: i64) -> bool {
        get_card(id, conn).unwrap().is_cram
    }

    // Reproduces the reported session: a due new card plus a card that gets crammed,
    // then the mark-for-review-while-crammed case, then a day tick.
    #[test]
    fn cram_serving_counts_and_clearing() {
        let mut conn = Connection::open_in_memory().unwrap();
        init_schema(&conn, &PathBuf::from("/tmp/toast-cram-test")).unwrap();
        conn.execute_batch(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'deck', 'deck');
             INSERT INTO plan (id, name) VALUES (1, 'plan');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_date (id, date) VALUES (0, ?1)",
            [chrono::Local::now().date_naive().to_string()],
        )
        .unwrap();
        add_group_to_plan(
            1,
            1,
            NewScheduler { group_id: 1, max_new: 10, max_review: 10, can_overflow: false },
            &mut conn,
        )
        .unwrap();
        // id 10 is a fresh new card, id 20 a graduated review card. fill_group (run by
        // add_group_to_plan) schedules whatever exists, so insert then top up.
        conn.execute_batch(
            "INSERT INTO card (id, group_id, front, back, tier, ease, sequence)
             VALUES (10, 1, 'nf', 'nb', 0, 0.0, 0), (20, 1, 'rf', 'rb', 3, 0.0, 0);",
        )
        .unwrap();
        crate::crud::scheduling::fill_group(1, &conn).unwrap();
        assert_eq!(counts(&conn), (1, 1, 0), "one new + one review due, no cram");

        // Rate the review card poorly: it should leave the due pool and enter cram.
        grade_item(20, 1, &mut conn).unwrap();
        assert!(is_cram(&conn, 20), "demote flags the card as cram");
        assert!(!get_card(20, &conn).unwrap().is_due, "a crammed card is not due");
        assert_eq!(counts(&conn), (1, 0, 1), "review clears, cram appears");

        // The bug: spamming One More Time on the new card kept it due; cram must not
        // surface while a due card remains, even when that card is the one excluded.
        grade_item(10, 4, &mut conn).unwrap();
        assert!(get_card(10, &conn).unwrap().is_due, "One More Time keeps the new card due");
        for _ in 0..8 {
            let served = next_session_card(&conn, 1, Some(10)).unwrap().unwrap();
            assert_eq!(served.id, 10, "the lone due card repeats instead of dropping into cram");
            assert!(served.is_due, "served as a real due card, not cram");
        }

        // Clear the new card. Now cram is the legitimate next serving, and it is is_due
        // = false so the frontend renders the cram buttons.
        grade_item(10, 5, &mut conn).unwrap();
        assert_eq!(counts(&conn), (0, 0, 1), "only the cram card is left");
        let served = next_session_card(&conn, 1, Some(10)).unwrap().unwrap();
        assert_eq!(served.id, 20, "cram card served once no due cards remain");
        assert!(!served.is_due, "cram serving carries is_due = false");

        // One More Time on the last cram card loops it rather than ending the session.
        let looped = next_session_card(&conn, 1, Some(20)).unwrap().unwrap();
        assert_eq!(looped.id, 20, "the sole cram card repeats");

        // Mark-for-review while crammed: the card counts in BOTH pools (+1 each).
        mark_for_review(20, &conn).unwrap();
        assert_eq!(counts(&conn), (0, 1, 1), "counts as review AND cram at once");
        let review = next_session_card(&conn, 1, None).unwrap().unwrap();
        assert_eq!(review.id, 20);
        assert!(review.is_due, "served from the due pool first, as review");

        // Rating the review version poorly clears the review but keeps the cram flag.
        grade_item(20, 1, &mut conn).unwrap();
        assert_eq!(counts(&conn), (0, 0, 1), "review cleared, still crammed");
        assert!(!next_session_card(&conn, 1, None).unwrap().unwrap().is_due, "back to cram-only");

        // A new day clears every cram.
        conn.execute("UPDATE app_date SET date = date(date, '-1 day')", []).unwrap();
        update_date(&conn).unwrap();
        assert!(!is_cram(&conn, 20), "the day tick clears the cram flag");
        assert_eq!(counts(&conn).2, 0, "no cram cards after the tick");
    }
}
