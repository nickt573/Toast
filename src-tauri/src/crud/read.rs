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
        SELECT id, plan_id, text, frequency, category, is_done, is_disabled
        FROM todo
        WHERE plan_id = ?1
        ORDER BY text COLLATE NOCASE ASC
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

pub fn get_cards(deck_id: i64, conn: &Connection) -> Result<Vec<Card>> {
    conn.prepare(
        r#"
        SELECT
            id, group_id, front, back, is_searchable, support,
            front_image, back_image, front_audio, back_audio, is_uploaded,
            tier, ease, sequence, is_due, is_overdue, is_paused, position
        FROM card
        WHERE group_id = ?1
        ORDER BY CASE WHEN position IS NULL THEN 1 ELSE 0 END, position ASC, id ASC
        "#,
    )?
    .query_map([deck_id], |row| {
        Ok(Card {
            id: row.get(0)?,
            group_id: row.get(1)?,
            front: row.get(2)?,
            back: row.get(3)?,
            is_searchable: row.get(4)?,
            support: row.get(5)?,
            front_image: row.get(6)?,
            back_image: row.get(7)?,
            front_audio: row.get(8)?,
            back_audio: row.get(9)?,
            is_uploaded: row.get(10)?,
            tier: row.get(11)?,
            ease: row.get(12)?,
            sequence: row.get(13)?,
            is_due: row.get(14)?,
            is_overdue: row.get(15)?,
            is_paused: row.get(16)?,
            position: row.get(17)?,
        })
    })?
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

/// Returns all groups not currently assigned to any plan — available to add to a plan.
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

pub fn get_group_stats(plan_id: i64, conn: &Connection) -> Result<Vec<GroupStat>> {
    conn.prepare(
        r#"
        SELECT id, group_id, plan_id, plan_name, group_name, date,
               num_promote, num_demote, num_new, time_spent_minutes, retention_rate
        FROM group_stats
        WHERE plan_id = ?1
        ORDER BY date DESC, id DESC
        "#,
    )?
    .query_map([plan_id], |row| {
        Ok(GroupStat {
            id: row.get(0)?,
            group_id: row.get(1)?,
            plan_id: row.get(2)?,
            plan_name: row.get(3)?,
            group_name: row.get(4)?,
            date: row.get(5)?,
            num_promote: row.get(6)?,
            num_demote: row.get(7)?,
            num_new: row.get(8)?,
            time_spent_minutes: row.get(9)?,
            retention_rate: row.get(10)?,
        })
    })?
    .collect()
}

pub fn get_deleted_plan_ids(conn: &Connection) -> Result<Vec<(i64, String)>> {
    conn.prepare(
        r#"
        SELECT DISTINCT plan_id, plan_name FROM group_stats
        WHERE plan_id NOT IN (SELECT id FROM plan)
        UNION
        SELECT DISTINCT plan_id, plan_name FROM todo_stats
        WHERE plan_id NOT IN (SELECT id FROM plan)
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
            ORDER BY date DESC, id DESC
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
        SELECT tsg.stat_id, tsg.group_id, COALESCE(g.name, tsg.group_name), COALESCE(g.group_type, tsg.group_type)
        FROM todo_stat_group tsg
        LEFT JOIN "group" g ON g.id = tsg.group_id
        WHERE tsg.stat_id IN (SELECT id FROM todo_stats WHERE plan_id = ?1)
        "#,
    )?
    .query_map([plan_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            TodoStatGroup {
                group_id: row.get::<_, Option<i64>>(1)?,
                name: row.get::<_, String>(2)?,
                group_type: row.get::<_, Option<String>>(3)?,
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
        SELECT tsr.stat_id,
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
                name: row.get::<_, String>(1)?,
                url: row.get::<_, Option<String>>(2)?,
                resource_type: row.get::<_, Option<String>>(3)?,
                notes: row.get::<_, Option<String>>(4)?,
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
