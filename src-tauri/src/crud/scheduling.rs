use crate::crud::models::*;
use chrono::Datelike;
use chrono::{self};
use rusqlite::{Connection, OptionalExtension, Result};

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
    let today_bit = 1i64 << today.weekday().num_days_from_sunday();

    // Recalculates is_disabled from today's weekday. Skips stay disabled, so a
    // same-day relaunch can't revive a skipped todo.
    let recalc_disabled = |conn: &Connection| {
        conn.execute(
            "UPDATE todo SET is_disabled = ((frequency & ?1) = 0) OR is_skipped",
            [today_bit],
        )
    };

    let stored: Option<String> = conn
        .query_row("SELECT date FROM app_date WHERE id = 0", [], |row| {
            row.get(0)
        })
        .ok();

    let n_days = match stored {
        None => {
            // First launch: insert today, no tick needed
            recalc_disabled(conn)?;
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
                recalc_disabled(conn)?;
                return Ok(());
            } // same day, nothing else to do
            delta as u32
        }
    };

    // New day: reset todo completion and skips, then tick SRS
    conn.execute("UPDATE todo SET is_done = FALSE, is_skipped = FALSE", [])?;
    recalc_disabled(conn)?;

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

/// Tops up a group's due queue to its daily quota. Studied today + currently due
/// counts against the maxes, but overflow carry-overs (is_overdue = TRUE) are free,
/// hence the is_overdue = FALSE filters (tri-state, see db.rs).
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

/// A group only has a scheduler row while it belongs to a plan, and fill_group
/// errors without one. Anything that may run on an unplanned deck must check.
fn has_scheduler(group_id: i64, conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM scheduler WHERE group_id = ?1",
        [group_id],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

pub fn on_item_added(group_id: i64, conn: &Connection) -> Result<()> {
    if !has_scheduler(group_id, conn) {
        return Ok(());
    }
    fill_group(group_id, conn)
}

pub fn on_item_removed(group_id: i64, was_due: bool, conn: &Connection) -> Result<()> {
    if !has_scheduler(group_id, conn) || !was_due {
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

    if !has_scheduler(group_id, conn) {
        return Ok(());
    }

    if !now_paused || was_due {
        fill_group(group_id, conn)?;
    }

    Ok(())
}

pub fn grade_item(item_id: i64, grade: u8, conn: &mut Connection) -> Result<()> {
    // Grades 0-3 are graduated-card ratings (Nope/Rough/Fine/Easy); 4-5 are new-card ratings (One More Time/Got It).
    let (tier_delta, ease_delta): (i32, f64) = match grade {
        0 => (-2, -0.12),
        1 => (-1, -0.08),
        2 => (1, -0.08),
        3 => (1, 0.10),
        4 => (-1, -0.05),
        5 => (1, 0.00),
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
    // Cap at tier 10, roughly a 1.4 year interval. Beyond that a card is effectively retired.
    let new_tier = (old_tier + tier_delta).max(floor).min(10);
    // Fine (grade 2) never pushes ease below 0 or deepens an already-negative ease.
    let ease_floor = if grade == 2 { old_ease.min(0.0) } else { -0.35 };
    let new_ease = (old_ease + ease_delta).max(ease_floor).min(0.35);

    let new_sequence: i32 = if new_tier == 0 {
        old_sequence
    } else {
        let raw = 2f64.powi(new_tier - 1) * (1.0 + new_ease);
        let base = raw.round() as i32;
        // Scatter same-day gradings by +-15% so cards stop advancing in tandem but stay within the tier.
        let span = (raw * 0.15).round() as i32;
        let jitter = if span > 0 {
            let r: i64 = tx.query_row("SELECT random()", [], |row| row.get(0))?;
            (r.rem_euclid(2 * span as i64 + 1)) as i32 - span
        } else {
            0
        };
        (base + jitter).max(1)
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
    // Rows predating the new-card renumbering store One More Time/Got It as grades 1/2, not 4/5; any per-grade read of new-card history must handle both.
    tx.execute(
        "INSERT INTO card_grade_log (card_id, grade, graded_at, old_tier, new_tier) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![item_id, grade, today, old_tier, new_tier],
    )?;

    let is_new = old_tier == 0 && new_tier > 0;
    let is_promote = old_tier > 0 && new_tier > old_tier;
    // A same-tier grade on a graduated card counts as a demotion: tier clamps at 1,
    // so grading "again" on tier 1 keeps the tier but is still a failed review.
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

/// Opens today's line for a deck in its plan and returns it. Nothing happens for a
/// deck outside a plan.
///
/// A pending reset forces a brand new line even when the day already has one, so a
/// run always begins on its own row. That line is marked as the start of the run and
/// the flag clears, which means repeat resets before the next session collapse into
/// one boundary. Otherwise the day's newest line is reused, since a reset earlier the
/// same day can leave more than one. An archived newest line is never reused, or
/// study logged after archiving the deck would land somewhere it doesn't count.
pub fn open_stat_line(group_id: i64, conn: &Connection) -> Result<Option<i64>> {
    let (plan_id, was_reset): (Option<i64>, bool) = conn.query_row(
        r#"SELECT plan_id, was_reset FROM "group" WHERE id = ?1"#,
        [group_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let Some(plan_id) = plan_id else {
        return Ok(None);
    };
    let today = get_date(conn)?;

    let existing: Option<(i64, bool)> = conn
        .query_row(
            "SELECT id, is_archived FROM group_stats
             WHERE group_id = ?1 AND plan_id = ?2 AND date = ?3
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![group_id, plan_id, &today],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    if let Some((id, archived)) = existing {
        if !was_reset && !archived {
            return Ok(Some(id));
        }
    }

    conn.execute(
        r#"
        INSERT INTO group_stats (group_id, origin_group_id, plan_id, plan_name, group_name, date, starts_era)
        SELECT g.id, g.id, p.id, p.name, g.name, ?3, ?4
        FROM "group" g, plan p
        WHERE g.id = ?1 AND p.id = ?2
        "#,
        rusqlite::params![group_id, plan_id, &today, was_reset],
    )?;

    if was_reset {
        conn.execute(
            r#"UPDATE "group" SET was_reset = FALSE WHERE id = ?1"#,
            [group_id],
        )?;
    }

    Ok(Some(conn.last_insert_rowid()))
}

fn write_group_stat(
    group_id: i64,
    is_promote: bool,
    is_demote: bool,
    is_new_review: bool,
    conn: &Connection,
) -> Result<()> {
    let Some(line) = open_stat_line(group_id, conn)? else {
        return Ok(());
    };

    conn.execute(
        "UPDATE group_stats
         SET num_promote = num_promote + ?2,
             num_demote = num_demote + ?3,
             num_new = num_new + ?4,
             retention_rate = CASE WHEN num_promote + ?2 + num_demote + ?3 > 0
                 THEN CAST(num_promote + ?2 AS REAL) / (num_promote + ?2 + num_demote + ?3)
                 ELSE 0.0 END
         WHERE id = ?1",
        rusqlite::params![line, is_promote as i32, is_demote as i32, is_new_review as i32],
    )?;
    Ok(())
}

pub fn add_group_time(group_id: i64, minutes: f64, conn: &Connection) -> Result<()> {
    let Some(line) = open_stat_line(group_id, conn)? else {
        return Ok(());
    };
    conn.execute(
        "UPDATE group_stats SET time_spent_minutes = time_spent_minutes + ?2 WHERE id = ?1",
        rusqlite::params![line, minutes],
    )?;
    Ok(())
}

// Wipes card progress and starts a blank group_stats row for today, splitting
/// Wipes card progress and marks the deck so the next session opened starts its own
/// stat row. Nothing is written here: a deck outside a plan has nowhere to write, and
/// flagging instead of writing means it makes no difference where the reset happened.
/// Resetting repeatedly just leaves the flag set. Also called from
/// remove_group_from_plan. Archiving what came before is a separate, optional step.
pub fn reset_deck(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET tier = 0, ease = 0.0, sequence = 0, is_due = FALSE, is_overdue = NULL, is_paused = FALSE WHERE group_id = ?1",
        [group_id],
    )?;
    conn.execute(
        "DELETE FROM card_grade_log WHERE card_id IN (SELECT id FROM card WHERE group_id = ?1)",
        [group_id],
    )?;
    conn.execute(
        r#"UPDATE "group" SET was_reset = TRUE WHERE id = ?1"#,
        [group_id],
    )?;
    conn.execute(
        "UPDATE scheduler SET studied_new = 0, studied_review = 0 WHERE group_id = ?1",
        [group_id],
    )?;
    let _ = fill_group(group_id, conn);

    Ok(())
}

/// Archives every stat row a deck has, across all plans. Offered alongside a reset so
/// the run that just ended can be dropped from totals while staying on the page.
pub fn archive_deck_stats(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE group_stats SET is_archived = TRUE WHERE group_id = ?1",
        [group_id],
    )?;
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

/// Below PRIORITY_CEIL the 1/day tick can't reach (~137 years). An empty range
/// anchors at PRIORITY_ANCHOR, marks grow LIFO below it, priorities FIFO above.
const PRIORITY_CEIL: i64 = -50_000;
const PRIORITY_ANCHOR: i64 = -1_000_000;

/// MIN/MAX sequence among a group's priority-range cards, None if empty.
/// Per-group, since order is only ever compared within a group.
fn priority_bound(group_id: i64, agg: &str, conn: &Connection) -> Result<Option<i64>> {
    conn.query_row(
        &format!("SELECT {agg}(sequence) FROM card WHERE group_id = ?1 AND sequence < ?2"),
        rusqlite::params![group_id, PRIORITY_CEIL],
        |row| row.get(0),
    )
}

pub fn prioritize_card(card_id: i64, conn: &Connection) -> Result<()> {
    let (group_id, sequence): (i64, i64) = conn.query_row(
        "SELECT group_id, sequence FROM card WHERE id = ?1",
        [card_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    // Already queued: re-stamping at MAX + 1 would demote it to the back.
    if sequence >= PRIORITY_CEIL {
        let next = match priority_bound(group_id, "MAX", conn)? {
            Some(max) => max + 1,
            None => PRIORITY_ANCHOR,
        };
        conn.execute(
            "UPDATE card SET sequence = ?1 WHERE id = ?2",
            rusqlite::params![next, card_id],
        )?;
    }

    // A queue jump, not a forced due: only fills if the quota has a free slot.
    if !has_scheduler(group_id, conn) {
        return Ok(());
    }
    fill_group(group_id, conn)
}

/// (streak, studied_today) for a plan. A day counts with any todo_stats row or any
/// graded group_stats row. Not studied today yet: streak carries from yesterday.
pub fn get_plan_streak(plan_id: i64, conn: &Connection) -> Result<(i64, bool)> {
    use std::collections::HashSet;

    let today = get_date(conn)?;

    let active: HashSet<String> = {
        let mut set = HashSet::new();
        conn.prepare(
            r#"
            SELECT DISTINCT date FROM group_stats
            WHERE plan_id = ?1 AND is_archived = FALSE
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
    let (group_id, is_due): (i64, bool) = conn.query_row(
        "SELECT group_id, is_due FROM card WHERE id = ?1",
        [card_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    // LIFO: newest mark lands ahead of everything queued. Re-stamped even when
    // already due, so the mark survives an overflow-off tick's unscheduling.
    let next = match priority_bound(group_id, "MIN", conn)? {
        Some(min) => min - 1,
        None => PRIORITY_ANCHOR,
    };

    if is_due {
        // Flags left alone: rewriting them would turn a quota-free overflow
        // carry-over (is_overdue = TRUE) into one that consumes quota.
        conn.execute(
            "UPDATE card SET sequence = ?1 WHERE id = ?2",
            rusqlite::params![next, card_id],
        )?;
    } else {
        conn.execute(
            "UPDATE card SET sequence = ?1, is_due = TRUE, is_overdue = FALSE, is_paused = FALSE WHERE id = ?2",
            rusqlite::params![next, card_id],
        )?;
    }

    Ok(())
}
