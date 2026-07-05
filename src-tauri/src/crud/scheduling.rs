use crate::crud::models::*;
use chrono::Datelike;
use chrono::{self};
use rusqlite::{Connection, Result};

pub fn update_scheduler(scheduler: Scheduler, conn: &Connection) -> Result<()> {
    conn.execute(
        r#"
        UPDATE scheduler
        SET max_new = ?1, studied_new = ?2,
            max_review = ?3, studied_review = ?4,
            can_overflow = ?5
        WHERE group_id = ?6
        "#,
        rusqlite::params![
            scheduler.max_new,
            scheduler.studied_new,
            scheduler.max_review,
            scheduler.studied_review,
            scheduler.can_overflow,
            scheduler.group_id,
        ],
    )?;
    let _ = fill_group(scheduler.group_id, conn);
    Ok(())
}

pub fn pause_all(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET is_due = FALSE, is_overdue = NULL, is_paused = TRUE WHERE group_id = ?1",
        [group_id],
    )?;
    Ok(())
}

pub fn unpause_all(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET is_paused = FALSE WHERE group_id = ?1",
        [group_id],
    )?;
    fill_group(group_id, conn)
}

pub fn get_date(conn: &Connection) -> Result<String> {
    Ok(
        conn.query_row("SELECT date FROM app_date WHERE id = 0", [], |row| {
            row.get(0)
        })?,
    )
}

pub fn update_date(conn: &Connection) -> Result<()> {
    let today = chrono::Local::now().date_naive();

    // Always recalculate is_disabled based on today's weekday
    let today_bit = 1i64 << today.weekday().num_days_from_sunday();
    conn.execute(
        "UPDATE todo SET is_disabled = ((frequency & ?1) = 0)",
        [today_bit],
    )?;

    let stored: Option<String> = conn
        .query_row("SELECT date FROM app_date WHERE id = 0", [], |row| {
            row.get(0)
        })
        .ok();

    let n_days = match stored {
        None => {
            // First launch — insert today, no tick needed
            conn.execute(
                "INSERT INTO app_date (id, date) VALUES (0, ?1)",
                rusqlite::params![today.to_string()],
            )?;
            return Ok(());
        }
        Some(s) => {
            let stored_date = chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
            let delta = (today - stored_date).num_days();
            if delta <= 0 {
                return Ok(());
            } // same day, nothing to do
            delta as u32
        }
    };

    // New day — reset todo completion state, then tick SRS
    conn.execute("UPDATE todo SET is_done = FALSE", [])?;

    for _ in 0..n_days {
        tick_all(conn)?;
    }

    conn.execute(
        "UPDATE app_date SET date = ?1 WHERE id = 0",
        rusqlite::params![today.to_string()],
    )?;

    Ok(())
}

fn tick_all(conn: &Connection) -> Result<()> {
    // Only decks are SRS-scheduled; legacy notebook schedulers are ignored.
    let groups: Vec<(i64, bool)> = {
        let mut stmt = conn.prepare(
            r#"
            SELECT s.group_id, s.can_overflow
            FROM scheduler s
            INNER JOIN "group" g ON g.id = s.group_id
            WHERE g.group_type = 'deck'
            "#,
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    for (group_id, can_overflow) in &groups {
        // Step 1: Decrement all non-paused sequences
        conn.execute(
            "UPDATE card SET sequence = sequence - 1 WHERE group_id = ?1 AND is_paused = FALSE",
            [group_id],
        )?;

        // Step 2: Roll over yesterday's due cards
        if *can_overflow {
            // Overflow ON: every still-due card becomes overflow.
            conn.execute(
                "UPDATE card SET is_overdue = TRUE WHERE group_id = ?1 AND is_due = TRUE",
                [group_id],
            )?;
        } else {
            // Overflow OFF: unschedule everything, so 15/10 collapses to 0/10
            // before refilling to 10/10 (no carry-over).
            conn.execute(
                "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1 AND is_due = TRUE",
                [group_id],
            )?;
        }

        // Step 3: Reset study counters
        conn.execute(
            "UPDATE scheduler SET studied_new = 0, studied_review = 0 WHERE group_id = ?1",
            [group_id],
        )?;

        // Step 4: Fill up to max
        fill_group(*group_id, conn)?;
    }

    Ok(())
}

pub fn count_due_items(group_id: &i64, conn: &Connection) -> Result<(i64, i64)> {
    conn.query_row(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE tier = 0) AS new_due,
            COUNT(*) FILTER (WHERE tier > 0) AS review_due
        FROM card
        WHERE group_id = ?1
          AND is_due = TRUE
          AND is_paused = FALSE
        "#,
        [group_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// Tops up a group's due queue to its daily quota.
///
/// Quota invariant: what counts against max_new/max_review is
/// `studied today (non-overflow) + currently due non-overflow cards`.
/// Overflow cards (is_overdue = TRUE, carried over from yesterday) are
/// "free" — they never consume quota, which is why both COUNT queries
/// filter on is_overdue = FALSE. The tri-state is_overdue encoding is:
/// TRUE = overflow carry-over, FALSE = scheduled normally today,
/// NULL = not due (see db.rs).
pub fn fill_group(group_id: i64, conn: &Connection) -> Result<()> {
    let (max_new, studied_new, max_review, studied_review): (i64, i64, i64, i64) = conn.query_row(
        "SELECT max_new, studied_new, max_review, studied_review FROM scheduler WHERE group_id = ?1",
        [group_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;

    let due_non_overflow_new: i64 = conn.query_row(
        "SELECT COUNT(*) FROM card
         WHERE group_id = ?1 AND is_paused = FALSE
           AND is_due = TRUE AND is_overdue = FALSE AND tier = 0",
        [group_id],
        |r| r.get(0),
    )?;

    let due_non_overflow_review: i64 = conn.query_row(
        "SELECT COUNT(*) FROM card
         WHERE group_id = ?1 AND is_paused = FALSE
           AND is_due = TRUE AND is_overdue = FALSE AND tier > 0",
        [group_id],
        |r| r.get(0),
    )?;

    let scheduled_new = studied_new + due_non_overflow_new;
    let scheduled_review = studied_review + due_non_overflow_review;

    fill_track(conn, group_id, "tier = 0", max_new, scheduled_new)?;
    fill_track(conn, group_id, "tier > 0", max_review, scheduled_review)?;
    Ok(())
}

fn fill_track(
    conn: &Connection,
    group_id: i64,
    tier_filter: &str,
    max: i64,
    scheduled: i64,
) -> Result<()> {
    let slots = max - scheduled; // signed: >0 fill, <0 unschedule

    if slots > 0 {
        conn.execute(
            &format!(
                r#"UPDATE card SET is_due = TRUE, is_overdue = FALSE
                   WHERE id IN (
                       SELECT id FROM card
                       WHERE group_id = ?1
                         AND is_paused = FALSE
                         AND is_due = FALSE
                         AND is_overdue IS NULL
                         AND {tier_filter}
                         AND sequence <= 0
                       ORDER BY sequence ASC, (position IS NULL) ASC, position ASC, id ASC
                       LIMIT ?2
                   )"#
            ),
            rusqlite::params![group_id, slots],
        )?;
    }

    Ok(())
}

pub fn on_item_added(group_id: i64, conn: &Connection) -> Result<()> {
    let has_scheduler: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM scheduler WHERE group_id = ?1",
            [group_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_scheduler {
        return Ok(());
    }
    fill_group(group_id, conn)
}

pub fn on_item_removed(group_id: i64, was_due: bool, conn: &Connection) -> Result<()> {
    let has_scheduler: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM scheduler WHERE group_id = ?1",
            [group_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_scheduler || !was_due {
        return Ok(());
    }

    fill_group(group_id, conn)
}

pub fn on_pause_changed(
    card_id: i64,
    group_id: i64,
    now_paused: bool,
    was_due: bool,
    conn: &Connection,
) -> Result<()> {
    if now_paused {
        conn.execute(
            "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE id = ?1",
            [card_id],
        )?;
    }

    let has_scheduler: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM scheduler WHERE group_id = ?1",
            [group_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_scheduler {
        return Ok(());
    }

    if !now_paused || was_due {
        fill_group(group_id, conn)?;
    }

    Ok(())
}

pub fn grade_item(item_id: i64, grade: u8, conn: &mut Connection) -> Result<()> {
    let (tier_delta, ease_delta): (i32, f64) = match grade {
        0 => (-2, -0.12),
        1 => (-1, -0.05),
        2 => (1, 0.04),
        3 => (1, 0.10),
        _ => {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Invalid grade: {}",
                grade
            )))
        }
    };

    let tx = conn.transaction()?;

    let (group_id, old_tier, old_sequence, old_ease, old_overdue): (
        i64,
        i32,
        i32,
        f64,
        Option<bool>,
    ) = tx.query_row(
        "SELECT group_id, tier, sequence, ease, is_overdue FROM card WHERE id = ?1",
        [item_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;

    // Floor depends on graduation status: graduated cards clamp at tier 1, ungraduated at tier 0
    let floor = if old_tier > 0 { 1 } else { 0 };
    let new_tier = (old_tier + tier_delta).max(floor).min(30);
    let new_ease = (old_ease + ease_delta).max(-0.35).min(0.35);

    let new_sequence: i32 = if new_tier == 0 {
        old_sequence
    } else {
        let raw = 2f64.powi(new_tier - 1) * (1.0 + new_ease);
        raw.round() as i32
    };

    let is_due = new_sequence <= 0;
    let is_overdue = if is_due { old_overdue } else { None };

    tx.execute(
        r#"
        UPDATE card
        SET tier = ?1, ease = ?2, sequence = ?3,
            is_due = ?4, is_overdue = ?5
        WHERE id = ?6
        "#,
        rusqlite::params![
            new_tier,
            new_ease,
            new_sequence,
            is_due,
            is_overdue,
            item_id
        ],
    )?;

    let today = get_date(&tx)?;
    tx.execute(
        "INSERT INTO card_grade_log (card_id, grade, graded_at, old_tier, new_tier) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![item_id, grade, today, old_tier, new_tier],
    )?;

    let is_new = old_tier == 0 && new_tier > 0;
    let is_promote = old_tier > 0 && new_tier > old_tier;
    // A same-tier grade on a graduated card deliberately counts as a demotion:
    // tier is clamped at a floor of 1, so grading "again" on tier 1 keeps the
    // tier while still being a failed review.
    let is_demote = old_tier > 0 && new_tier <= old_tier;

    // Only non-overflow cards (is_overdue == FALSE) consume the daily quota;
    // overflow carry-overs (TRUE) and off-schedule grades (NULL) are free.
    if old_overdue == Some(false) {
        if is_new {
            tx.execute(
                "UPDATE scheduler SET studied_new = studied_new + 1 WHERE group_id = ?1",
                rusqlite::params![group_id],
            )?;
        } else if is_promote || is_demote {
            tx.execute(
                "UPDATE scheduler SET studied_review = studied_review + 1 WHERE group_id = ?1",
                rusqlite::params![group_id],
            )?;
        }
    }

    write_group_stat(group_id, is_promote, is_demote, is_new, &tx)?;
    tx.commit()
}

fn write_group_stat(
    group_id: i64,
    is_promote: bool,
    is_demote: bool,
    is_new_review: bool,
    conn: &Connection,
) -> Result<()> {
    let today = get_date(&conn)?;

    let (group_name, plan_id): (String, Option<i64>) = conn.query_row(
        r#"SELECT name, plan_id FROM "group" WHERE id = ?1"#,
        [group_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let plan_id = match plan_id {
        Some(id) => id,
        None => return Ok(()),
    };

    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM group_stats WHERE group_id = ?1 AND date = ?2 ORDER BY id DESC LIMIT 1",
            rusqlite::params![group_id, &today],
            |row| row.get(0),
        )
        .ok();

    let stat_id = match existing_id {
        Some(id) => {
            conn.execute(
                r#"
                UPDATE group_stats
                SET num_promote = num_promote + ?1,
                    num_demote = num_demote + ?2,
                    num_new = num_new + ?3
                WHERE id = ?4
                "#,
                rusqlite::params![
                    is_promote as i32,
                    is_demote as i32,
                    is_new_review as i32,
                    id
                ],
            )?;
            id
        }
        // Should theoretically never execute, adding to a plan builds a blank stat
        None => {
            let plan_name: String = conn
                .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
                    r.get(0)
                })
                .unwrap_or_default();
            conn.execute(
                r#"
                INSERT INTO group_stats (group_id, plan_id, plan_name, group_name, date, num_promote, num_demote, num_new)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                rusqlite::params![group_id, plan_id, plan_name, group_name, &today, is_promote as i32, is_demote as i32, is_new_review as i32],
            )?;
            conn.last_insert_rowid()
        }
    };

    let (p, d): (i64, i64) = conn.query_row(
        "SELECT num_promote, num_demote FROM group_stats WHERE id = ?1",
        [stat_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let total = p + d;
    let retention = if total > 0 {
        p as f64 / total as f64
    } else {
        0.0
    };

    conn.execute(
        "UPDATE group_stats SET retention_rate = ?1 WHERE id = ?2",
        rusqlite::params![retention, stat_id],
    )?;

    Ok(())
}

pub fn add_group_time(group_id: i64, minutes: f64, conn: &Connection) -> Result<()> {
    let today = get_date(&conn)?;

    let (group_name, plan_id): (String, Option<i64>) = conn.query_row(
        r#"SELECT name, plan_id FROM "group" WHERE id = ?1"#,
        [group_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let plan_id = match plan_id {
        Some(id) => id,
        None => return Ok(()),
    };

    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM group_stats WHERE group_id = ?1 AND date = ?2 ORDER BY id DESC LIMIT 1",
            rusqlite::params![group_id, &today],
            |row| row.get(0),
        )
        .ok();

    match existing_id {
        Some(id) => {
            conn.execute(
                "UPDATE group_stats SET time_spent_minutes = time_spent_minutes + ?1 WHERE id = ?2",
                rusqlite::params![minutes, id],
            )?;
        }
        // Should theoretically never happen, as adding a deck to a plan creates a new row automatically
        None => {
            let plan_name: String = conn
                .query_row("SELECT name FROM plan WHERE id = ?1", [plan_id], |r| {
                    r.get(0)
                })
                .unwrap_or_default();
            conn.execute(
                r#"
                INSERT INTO group_stats (group_id, plan_id, plan_name, group_name, date, time_spent_minutes)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                rusqlite::params![group_id, plan_id, plan_name, group_name, today, minutes],
            )?;
        }
    }

    Ok(())
}

// Called both from the deck editor and from remove_group_from_plan with reset=true.
// Wipes card progress and inserts a blank group_stats row for today so subsequent
// grading writes to a fresh slot, splitting pre/post reset data.
// If not in plan, a new row will be generated anyway on add
pub fn reset_deck(group_id: i64, conn: &Connection) -> Result<()> {
    let today = get_date(conn)?;

    let (group_name, plan_id): (String, Option<i64>) = conn.query_row(
        r#"SELECT name, plan_id FROM "group" WHERE id = ?1"#,
        [group_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    conn.execute(
        "UPDATE card SET tier = 0, ease = 0.0, sequence = 0, is_due = FALSE, is_overdue = NULL, is_paused = FALSE WHERE group_id = ?1",
        [group_id],
    )?;
    conn.execute(
        "DELETE FROM card_grade_log WHERE card_id IN (SELECT id FROM card WHERE group_id = ?1)",
        [group_id],
    )?;

    // Delete today's stat if it only has zeros (no real data yet today, no point keeping it)
    conn.execute(
        r#"
        DELETE FROM group_stats
        WHERE group_id = ?1 AND date = ?2
          AND num_new = 0 AND num_promote = 0 AND num_demote = 0 AND time_spent_minutes = 0
        "#,
        rusqlite::params![group_id, &today],
    )?;

    // If in plan, zero the scheduler counters, insert a fresh blank stats row, and fill group
    if let Some(plan_id) = plan_id {
        conn.execute(
            "UPDATE scheduler SET studied_new = 0, studied_review = 0 WHERE group_id = ?1",
            [group_id],
        )?;

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
    }

    Ok(())
}

pub fn clamp_group(group_id: i64, conn: &Connection) -> Result<()> {
    // Relative clamp: clear all due non-paused cards, then refill to (max - studied).
    // Accounts for cards already studied today, so the total work remains capped at max.
    conn.execute(
        "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1 AND is_paused = FALSE AND is_due = TRUE",
        [group_id],
    )?;
    fill_group(group_id, conn)
}

pub fn max_clamp_group(group_id: i64, conn: &Connection) -> Result<()> {
    // Max clamp: clear all due non-paused cards, then refill up to the raw max
    // ignoring how many cards have already been studied today.
    conn.execute(
        "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1 AND is_paused = FALSE AND is_due = TRUE",
        [group_id],
    )?;

    let (max_new, max_review): (i64, i64) = conn.query_row(
        "SELECT max_new, max_review FROM scheduler WHERE group_id = ?1",
        [group_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // After clearing, due_non_overflow = 0, so passing scheduled = 0 means
    // fill_track will schedule up to max cards regardless of today's study count.
    fill_track(conn, group_id, "tier = 0", max_new, 0)?;
    fill_track(conn, group_id, "tier > 0", max_review, 0)?;
    Ok(())
}

pub fn prioritize_card(card_id: i64, conn: &Connection) -> Result<()> {
    conn.execute("UPDATE card SET sequence = -9999 WHERE id = ?1", [card_id])?;
    Ok(())
}

/// Returns (streak, studied_today) for a plan.
/// A day counts if it has any todo_stats row OR any group_stats row with cards graded.
/// studied_today reflects whether qualifying activity exists on the current stored date.
/// If not studied today, the streak carries forward from yesterday (standard behaviour).
pub fn get_plan_streak(plan_id: i64, conn: &Connection) -> Result<(i64, bool)> {
    use std::collections::HashSet;

    let today = get_date(conn)?;

    let active: HashSet<String> = {
        let mut set = HashSet::new();
        conn.prepare(
            r#"
            SELECT DISTINCT date FROM group_stats
            WHERE plan_id = ?1
              AND (num_new > 0 OR num_promote > 0 OR num_demote > 0)
            UNION
            SELECT DISTINCT date FROM todo_stats WHERE plan_id = ?1
            "#,
        )?
        .query_map([plan_id], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .for_each(|d| {
            set.insert(d);
        });
        set
    };

    let studied_today = active.contains(&today);

    let today_date = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;

    let mut d = if studied_today {
        today_date
    } else {
        match today_date.pred_opt() {
            Some(prev) => prev,
            None => return Ok((0, false)),
        }
    };

    let mut streak = 0i64;
    loop {
        if active.contains(&d.to_string()) {
            streak += 1;
            match d.pred_opt() {
                Some(prev) => d = prev,
                None => break,
            }
        } else {
            break;
        }
    }

    Ok((streak, studied_today))
}

pub fn mark_for_review(card_id: i64, conn: &Connection) -> Result<()> {
    let is_due: bool =
        conn.query_row("SELECT is_due FROM card WHERE id = ?1", [card_id], |row| {
            row.get(0)
        })?;

    if is_due {
        return Ok(());
    }

    conn.execute(
        "UPDATE card SET sequence = -9999, is_due = TRUE, is_overdue = FALSE, is_paused = FALSE WHERE id = ?1",
        [card_id],
    )?;

    Ok(())
}
