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
    // A benched deck stays frozen, only an in-plan deck reschedules
    if in_plan(scheduler.group_id, conn) {
        let _ = fill_group(scheduler.group_id, conn);
    }
    Ok(())
}

pub fn pause_all(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET is_due = FALSE, is_overdue = NULL, is_paused = TRUE, is_cram = FALSE WHERE group_id = ?1",
        [group_id],
    )?;
    Ok(())
}

pub fn unpause_all(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET is_paused = FALSE WHERE group_id = ?1",
        [group_id],
    )?;
    // A benched deck stays frozen, only an in-plan deck reschedules
    if in_plan(group_id, conn) {
        fill_group(group_id, conn)?;
    }
    Ok(())
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

    // Recalc is_disabled from today's weekday, skips stay disabled so a relaunch can't
    // revive a skipped todo
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
            // First launch, insert today, no tick needed
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

    // New day, reset todo completion and skips, then tick SRS
    conn.execute("UPDATE todo SET is_done = FALSE, is_skipped = FALSE", [])?;
    recalc_disabled(conn)?;

    let today_str = today.to_string();
    for _ in 0..n_days {
        tick_all(conn, &today_str)?;
    }

    conn.execute(
        "UPDATE app_date SET date = ?1 WHERE id = 0",
        rusqlite::params![today.to_string()],
    )?;

    Ok(())
}

fn tick_all(conn: &Connection, today: &str) -> Result<()> {
    // Only decks in a plan advance, a benched deck stays frozen until it is re-added
    let groups: Vec<i64> = {
        let mut stmt = conn.prepare(
            r#"
            SELECT s.group_id
            FROM scheduler s
            INNER JOIN "group" g ON g.id = s.group_id
            INNER JOIN plan p ON p.id = g.plan_id
            WHERE g.group_type = 'deck' AND p.is_disabled = FALSE
            "#,
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?
            .filter_map(|r| r.ok())
            .collect();
        rows
    };

    for group_id in &groups {
        tick_one(*group_id, today, conn)?;
    }

    Ok(())
}

/// Advance one deck a single day, identical for the daily tick and a re-add that spans a day
pub fn tick_one(group_id: i64, today: &str, conn: &Connection) -> Result<()> {
    let can_overflow: bool = conn.query_row(
        "SELECT can_overflow FROM scheduler WHERE group_id = ?1",
        [group_id],
        |r| r.get(0),
    )?;

    // Step 1 decrement all non-paused sequences
    conn.execute(
        "UPDATE card SET sequence = sequence - 1 WHERE group_id = ?1 AND is_paused = FALSE",
        [group_id],
    )?;

    // Step 2 roll over yesterday's due cards
    if can_overflow {
        // Overflow on, every still-due card becomes overflow
        conn.execute(
            "UPDATE card SET is_overdue = TRUE WHERE group_id = ?1 AND is_due = TRUE",
            [group_id],
        )?;
    } else {
        // Overflow off, unschedule everything so the queue collapses and refills with no carry-over
        conn.execute(
            "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1 AND is_due = TRUE",
            [group_id],
        )?;
    }

    // Step 3 reset study counters and stamp the day this deck last advanced
    conn.execute(
        "UPDATE scheduler SET studied_new = 0, studied_review = 0, last_synced_date = ?2 WHERE group_id = ?1",
        rusqlite::params![group_id, today],
    )?;

    // A new day clears the cram pool
    conn.execute(
        "UPDATE card SET is_cram = FALSE WHERE group_id = ?1",
        [group_id],
    )?;

    // Step 4 fill up to max
    fill_group(group_id, conn)?;

    Ok(())
}

pub fn count_due_items(group_id: &i64, conn: &Connection) -> Result<(i64, i64, i64)> {
    conn.query_row(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE is_due = TRUE AND tier = 0) AS new_due,
            COUNT(*) FILTER (WHERE is_due = TRUE AND tier > 0) AS review_due,
            COUNT(*) FILTER (WHERE is_cram = TRUE) AS cram_due
        FROM card
        WHERE group_id = ?1
          AND is_paused = FALSE
          AND (is_due = TRUE OR is_cram = TRUE)
        "#,
        [group_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
}

/// Top up a group's due queue to its daily quota, where studied today plus currently due
/// counts against the maxes and overflow carry-overs are free
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
    // Negative slots mean the cap is already exceeded, which only happens when a re-add
    // hands the deck a lower one, and that path clears the queue first so this only fills
    let slots = max - scheduled;

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

/// Whether the deck is in a plan, which now decides scheduling instead of scheduler existence
fn in_plan(group_id: i64, conn: &Connection) -> bool {
    conn.query_row(
        r#"SELECT plan_id IS NOT NULL FROM "group" WHERE id = ?1"#,
        [group_id],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

pub fn on_item_added(group_id: i64, conn: &Connection) -> Result<()> {
    if !in_plan(group_id, conn) {
        return Ok(());
    }
    fill_group(group_id, conn)
}

pub fn on_item_removed(group_id: i64, was_due: bool, conn: &Connection) -> Result<()> {
    if !in_plan(group_id, conn) || !was_due {
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
            "UPDATE card SET is_due = FALSE, is_overdue = NULL, is_cram = FALSE WHERE id = ?1",
            [card_id],
        )?;
    }

    if !in_plan(group_id, conn) {
        return Ok(());
    }

    if !now_paused || was_due {
        fill_group(group_id, conn)?;
    }

    Ok(())
}

pub fn grade_item(item_id: i64, grade: u8, conn: &mut Connection) -> Result<()> {
    // Grades 0 to 3 are graduated-card ratings, 4 and 5 are the new-card ratings
    let (tier_delta, ease_delta): (i32, f64) = match grade {
        0 => (-2, -0.12),
        1 => (-1, -0.08),
        2 => (1, -0.08),
        3 => (1, 0.10),
        4 => (-1, -0.03),
        5 => (1, 0.03),
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

    // Floor depends on graduation status, graduated cards clamp at tier 1, ungraduated at tier 0
    let floor = if old_tier > 0 { 1 } else { 0 };
    // Cap at tier 10, roughly a year and a half, beyond which a card is effectively retired
    let new_tier = (old_tier + tier_delta).max(floor).min(10);
    // A Fine rating never pushes ease below 0 or deepens an already-negative ease
    let ease_floor = if grade == 2 { old_ease.min(0.0) } else { -0.35 };
    let new_ease = (old_ease + ease_delta).max(ease_floor).min(0.35);

    let new_sequence: i32 = if new_tier == 0 {
        old_sequence
    } else {
        let raw = 2f64.powi(new_tier - 1) * (1.0 + new_ease);
        let base = raw.round() as i32;
        // Scatter same-day gradings a little so cards stop advancing in tandem but stay
        // within the tier
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

    let is_new = old_tier == 0 && new_tier > 0;
    let is_promote = old_tier > 0 && new_tier > old_tier;
    // A same-tier grade on a graduated card is a demotion, and tier clamps at 1 so grading
    // again on tier 1 is still a failed review
    let is_demote = old_tier > 0 && new_tier <= old_tier;

    // A demoted review card enters the cram pool, other grades leave the flag alone
    tx.execute(
        r#"
        UPDATE card
        SET tier = ?1, ease = ?2, sequence = ?3,
            is_due = ?4, is_overdue = ?5,
            is_cram = CASE WHEN ?6 THEN 1 ELSE is_cram END
        WHERE id = ?7
        "#,
        rusqlite::params![
            new_tier,
            new_ease,
            new_sequence,
            is_due,
            is_overdue,
            is_demote,
            item_id
        ],
    )?;

    let today = get_date(&tx)?;
    // Rows predating the new-card renumbering store One More Time and Got It as grades 1 and
    // 2, so any per-grade read of new-card history must handle both
    tx.execute(
        "INSERT INTO card_grade_log (card_id, grade, graded_at, old_tier, new_tier) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![item_id, grade, today, old_tier, new_tier],
    )?;

    // Only non-overflow cards consume the daily quota, carry-overs and off-schedule grades
    // are free
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

/// Open today's line for a deck in its plan, None outside a plan, and a line closed by a
/// reset or archive is never reused so the next session opens its own row
pub fn open_stat_line(group_id: i64, conn: &Connection) -> Result<Option<i64>> {
    let plan_id: Option<i64> = conn.query_row(
        r#"SELECT plan_id FROM "group" WHERE id = ?1"#,
        [group_id],
        |r| r.get(0),
    )?;
    let Some(plan_id) = plan_id else {
        return Ok(None);
    };
    let today = get_date(conn)?;

    let existing: Option<(i64, bool, bool)> = conn
        .query_row(
            "SELECT id, is_archived,
                    EXISTS(SELECT 1 FROM deck_reset
                           WHERE origin_group_id = ?1 AND after_stat_id >= group_stats.id)
             FROM group_stats
             WHERE group_id = ?1 AND plan_id = ?2 AND date = ?3
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![group_id, plan_id, &today],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    if let Some((id, archived, closed_by_reset)) = existing {
        if !archived && !closed_by_reset {
            return Ok(Some(id));
        }
    }

    conn.execute(
        r#"
        INSERT INTO group_stats (group_id, origin_group_id, plan_id, plan_name, group_name, date)
        SELECT g.id, g.id, p.id, p.name, g.name, ?3
        FROM "group" g, plan p
        WHERE g.id = ?1 AND p.id = ?2
        "#,
        rusqlite::params![group_id, plan_id, &today],
    )?;

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

/// Wipe card progress and record the reset against the deck at the highest stat line id so
/// every plan can tell which lines fall either side, and an empty deck records nothing
pub fn reset_deck(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE card SET tier = 0, ease = 0.0, sequence = 0, is_due = FALSE, is_overdue = NULL, is_paused = FALSE, is_cram = FALSE WHERE group_id = ?1",
        [group_id],
    )?;
    conn.execute(
        "DELETE FROM card_grade_log WHERE card_id IN (SELECT id FROM card WHERE group_id = ?1)",
        [group_id],
    )?;
    let today = get_date(conn)?;
    conn.execute(
        "INSERT INTO deck_reset (origin_group_id, date, after_stat_id)
         SELECT ?1, ?2, COALESCE((SELECT MAX(id) FROM group_stats), 0)
         WHERE EXISTS (SELECT 1 FROM group_stats WHERE origin_group_id = ?1)",
        rusqlite::params![group_id, &today],
    )?;
    conn.execute(
        "UPDATE scheduler SET studied_new = 0, studied_review = 0 WHERE group_id = ?1",
        [group_id],
    )?;
    // A benched deck stays frozen, only an in-plan deck reschedules after the wipe
    if in_plan(group_id, conn) {
        let _ = fill_group(group_id, conn);
    }

    Ok(())
}

/// Archive every stat row a deck has across all plans, offered with a reset so the ended run
/// drops from totals but stays on the page
pub fn archive_deck_stats(group_id: i64, conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE group_stats SET is_archived = TRUE WHERE group_id = ?1",
        [group_id],
    )?;
    Ok(())
}

pub fn clamp_group(group_id: i64, conn: &Connection) -> Result<()> {
    // A benched deck stays frozen
    if !in_plan(group_id, conn) {
        return Ok(());
    }
    // Relative clamp, clear all due non-paused cards then refill to what is left of the max,
    // so total work stays capped
    conn.execute(
        "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1 AND is_paused = FALSE AND is_due = TRUE",
        [group_id],
    )?;
    fill_group(group_id, conn)
}

pub fn max_clamp_group(group_id: i64, conn: &Connection) -> Result<()> {
    // A benched deck stays frozen
    if !in_plan(group_id, conn) {
        return Ok(());
    }
    // Max clamp, clear all due non-paused cards then refill to the raw max, ignoring today's study count
    conn.execute(
        "UPDATE card SET is_due = FALSE, is_overdue = NULL WHERE group_id = ?1 AND is_paused = FALSE AND is_due = TRUE",
        [group_id],
    )?;

    let (max_new, max_review): (i64, i64) = conn.query_row(
        "SELECT max_new, max_review FROM scheduler WHERE group_id = ?1",
        [group_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // After clearing there is nothing due, so it fills up to max regardless of today's study
    fill_track(conn, group_id, "tier = 0", max_new, 0)?;
    fill_track(conn, group_id, "tier > 0", max_review, 0)?;
    Ok(())
}

/// Below PRIORITY_CEIL the daily tick can't reach, and an empty range anchors at
/// PRIORITY_ANCHOR with marks growing downward and priorities upward
const PRIORITY_CEIL: i64 = -50_000;
const PRIORITY_ANCHOR: i64 = -1_000_000;

/// The lowest and highest sequence among a group's priority-range cards, None if empty, and
/// per-group since order is only compared within a group
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

    // Already queued, re-stamping past the highest would demote it to the back
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

    // A queue jump not a forced due, only fills if the quota has a free slot
    if !in_plan(group_id, conn) {
        return Ok(());
    }
    fill_group(group_id, conn)
}

/// The streak, whether today was studied, and the longest run for a plan, where a day counts
/// with any todo_stats row or graded group_stats row
pub fn get_plan_streak(plan_id: i64, conn: &Connection) -> Result<(i64, bool, i64)> {
    use std::collections::HashSet;

    let today = get_date(conn)?;

    let stored_longest: i64 = conn
        .query_row("SELECT longest_streak FROM plan WHERE id = ?1", [plan_id], |r| r.get(0))
        .optional()?
        .unwrap_or(0);

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
            None => return Ok((0, false, stored_longest)),
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

    // Recompute the longest run from the active days so unstudying pulls an inflated record down
    let mut longest = 0i64;
    for day in &active {
        let Ok(nd) = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d") else { continue };
        let prev_studied = nd.pred_opt().is_some_and(|p| active.contains(&p.to_string()));
        if prev_studied {
            continue;
        }
        let mut run = 0i64;
        let mut cur = Some(nd);
        while let Some(c) = cur {
            if !active.contains(&c.to_string()) {
                break;
            }
            run += 1;
            cur = c.succ_opt();
        }
        longest = longest.max(run);
    }

    if longest != stored_longest {
        conn.execute(
            "UPDATE plan SET longest_streak = ?1 WHERE id = ?2",
            rusqlite::params![longest, plan_id],
        )?;
    }

    Ok((streak, studied_today, longest))
}

pub fn mark_for_review(card_id: i64, conn: &Connection) -> Result<()> {
    let (group_id, is_due): (i64, bool) = conn.query_row(
        "SELECT group_id, is_due FROM card WHERE id = ?1",
        [card_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    // The newest mark lands ahead of everything queued, re-stamped even when already due so
    // it isn't lost to an overflow-off tick's unscheduling
    let next = match priority_bound(group_id, "MIN", conn)? {
        Some(min) => min - 1,
        None => PRIORITY_ANCHOR,
    };

    if is_due {
        // Flags left alone, rewriting them would turn a quota-free overflow carry-over into
        // one that consumes quota
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

#[cfg(test)]
mod streak_tests {
    use super::*;

    const TODAY: &str = "2026-07-30";

    fn setup() -> Connection {
        let tmp = tempfile::tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, tmp.path()).unwrap();
        conn.execute("INSERT INTO plan (id, name) VALUES (1, 'p')", []).unwrap();
        conn.execute(r#"INSERT INTO "group" (id, plan_id, name, group_type) VALUES (1, 1, 'd', 'deck')"#, []).unwrap();
        conn.execute("INSERT INTO app_date (id, date) VALUES (0, ?1)", [TODAY]).unwrap();
        conn
    }

    fn studied(conn: &Connection, date: &str) {
        conn.execute(
            "INSERT INTO group_stats (group_id, plan_id, group_name, date, num_new) VALUES (1, 1, 'd', ?1, 1)",
            [date],
        )
        .unwrap();
    }

    fn unstudy(conn: &Connection, date: &str) {
        conn.execute("DELETE FROM group_stats WHERE date = ?1", [date]).unwrap();
    }

    fn stored_longest(conn: &Connection) -> i64 {
        conn.query_row("SELECT longest_streak FROM plan WHERE id = 1", [], |r| r.get(0)).unwrap()
    }

    // A real earlier run of 3 outlives unstudying today, since its days are still on record
    #[test]
    fn longest_keeps_a_real_earlier_run() {
        let conn = setup();
        studied(&conn, "2026-01-01");
        studied(&conn, "2026-01-02");
        studied(&conn, "2026-01-03");
        studied(&conn, "2026-07-29");
        studied(&conn, TODAY);

        let (streak, today, longest) = get_plan_streak(1, &conn).unwrap();
        assert_eq!((streak, today, longest), (2, true, 3));

        unstudy(&conn, TODAY);
        let (streak, today, longest) = get_plan_streak(1, &conn).unwrap();
        assert_eq!((streak, today, longest), (1, false, 3), "the January run still stands");
    }

    // A 3 that only existed because of today falls back to 2 once today is undone
    #[test]
    fn longest_drops_when_today_made_it() {
        let conn = setup();
        studied(&conn, "2026-07-28");
        studied(&conn, "2026-07-29");
        studied(&conn, TODAY);

        let (streak, _, longest) = get_plan_streak(1, &conn).unwrap();
        assert_eq!((streak, longest), (3, 3));
        assert_eq!(stored_longest(&conn), 3);

        unstudy(&conn, TODAY);
        let (streak, _, longest) = get_plan_streak(1, &conn).unwrap();
        assert_eq!((streak, longest), (2, 2), "the momentary 3 is squashed");
        assert_eq!(stored_longest(&conn), 2, "the cache is reconciled down");
    }

    // The record is the longest run across gaps, not the most recent one
    #[test]
    fn longest_is_the_max_run_across_gaps() {
        let conn = setup();
        studied(&conn, "2026-03-01");
        studied(&conn, "2026-03-02");
        studied(&conn, "2026-03-03");
        studied(&conn, "2026-03-04");
        studied(&conn, "2026-06-10");
        studied(&conn, "2026-06-11");

        let (streak, today, longest) = get_plan_streak(1, &conn).unwrap();
        assert_eq!((streak, today, longest), (0, false, 4));
    }

    // Archived days are set aside, so they neither extend a run nor bridge a gap
    #[test]
    fn archived_days_do_not_count() {
        let conn = setup();
        studied(&conn, "2026-05-01");
        conn.execute("UPDATE group_stats SET is_archived = TRUE WHERE date = '2026-05-01'", []).unwrap();
        studied(&conn, "2026-05-02");
        studied(&conn, "2026-05-03");

        let (_, _, longest) = get_plan_streak(1, &conn).unwrap();
        assert_eq!(longest, 2, "the archived day is left out of the run");
    }
}

#[cfg(test)]
mod decouple_tests {
    use super::*;
    use crate::crud::create::{add_group_to_plan, create_deck};
    use crate::crud::delete::remove_group_from_plan;
    use crate::crud::models::NewScheduler;

    const D1: &str = "2026-07-30";
    const D2: &str = "2026-07-31";

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn, &std::path::PathBuf::from("/tmp/toast-decouple-test")).unwrap();
        conn.execute("INSERT INTO plan (id, name) VALUES (1, 'p')", []).unwrap();
        conn.execute("INSERT INTO app_date (id, date) VALUES (0, ?1)", [D1]).unwrap();
        conn
    }

    fn sched(group: i64) -> NewScheduler {
        NewScheduler { group_id: group, max_new: 5, max_review: 5, can_overflow: false }
    }

    fn add_card(conn: &Connection, id: i64, group: i64, tier: i32, sequence: i32, is_due: bool) {
        conn.execute(
            "INSERT INTO card (id, group_id, front, back, tier, ease, sequence, is_due, is_overdue)
             VALUES (?1, ?2, 'f', 'b', ?3, 0.0, ?4, ?5, ?6)",
            rusqlite::params![id, group, tier, sequence, is_due, if is_due { Some(false) } else { None }],
        )
        .unwrap();
    }

    fn studied_new(conn: &Connection, group: i64) -> i64 {
        conn.query_row("SELECT studied_new FROM scheduler WHERE group_id = ?1", [group], |r| r.get(0)).unwrap()
    }
    fn has_sched(conn: &Connection, group: i64) -> bool {
        conn.query_row("SELECT COUNT(*) FROM scheduler WHERE group_id = ?1", [group], |r| r.get::<_, i64>(0)).unwrap() > 0
    }
    fn seq(conn: &Connection, card: i64) -> i32 {
        conn.query_row("SELECT sequence FROM card WHERE id = ?1", [card], |r| r.get(0)).unwrap()
    }

    #[test]
    fn create_deck_makes_a_scheduler_synced_today() {
        let conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        assert!(has_sched(&conn, d.id));
        let ls: Option<String> = conn
            .query_row("SELECT last_synced_date FROM scheduler WHERE group_id = ?1", [d.id], |r| r.get(0))
            .unwrap();
        assert_eq!(ls.as_deref(), Some(D1));
    }

    #[test]
    fn same_day_readd_preserves_the_days_count() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_group_to_plan(d.id, 1, sched(d.id), &mut conn).unwrap();
        conn.execute("UPDATE scheduler SET studied_new = 3 WHERE group_id = ?1", [d.id]).unwrap();

        remove_group_from_plan(d.id, false, &mut conn).unwrap();
        assert!(has_sched(&conn, d.id), "scheduler survives removal");

        add_group_to_plan(d.id, 1, sched(d.id), &mut conn).unwrap();
        assert_eq!(studied_new(&conn, d.id), 3, "same-day re-add keeps the count");
    }

    #[test]
    fn cross_day_readd_resets_the_count_and_advances_one_day() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_card(&conn, 1, d.id, 1, 2, false);
        add_group_to_plan(d.id, 1, sched(d.id), &mut conn).unwrap();
        conn.execute("UPDATE scheduler SET studied_new = 3 WHERE group_id = ?1", [d.id]).unwrap();
        remove_group_from_plan(d.id, false, &mut conn).unwrap();

        conn.execute("UPDATE app_date SET date = ?1 WHERE id = 0", [D2]).unwrap();
        add_group_to_plan(d.id, 1, sched(d.id), &mut conn).unwrap();

        assert_eq!(studied_new(&conn, d.id), 0, "a new day resets the count");
        assert_eq!(seq(&conn, 1), 1, "exactly one day advanced, not the whole gap");
        let ls: Option<String> = conn
            .query_row("SELECT last_synced_date FROM scheduler WHERE group_id = ?1", [d.id], |r| r.get(0))
            .unwrap();
        assert_eq!(ls.as_deref(), Some(D2));
    }

    #[test]
    fn the_tick_freezes_a_benched_deck_but_advances_an_in_plan_one() {
        let mut conn = setup();
        let benched = create_deck("benched".into(), &conn).unwrap();
        add_card(&conn, 1, benched.id, 1, 5, false);
        let live = create_deck("live".into(), &conn).unwrap();
        add_card(&conn, 2, live.id, 1, 5, false);
        add_group_to_plan(live.id, 1, sched(live.id), &mut conn).unwrap();

        tick_all(&conn, D2).unwrap();

        assert_eq!(seq(&conn, 1), 5, "benched deck is frozen");
        assert_eq!(seq(&conn, 2), 4, "in-plan deck advances");
    }

    #[test]
    fn preserve_removal_freezes_the_queue() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_card(&conn, 1, d.id, 1, 0, true);
        add_group_to_plan(d.id, 1, sched(d.id), &mut conn).unwrap();
        conn.execute("UPDATE card SET is_due = TRUE, is_overdue = FALSE WHERE id = 1", []).unwrap();

        remove_group_from_plan(d.id, false, &mut conn).unwrap();

        let due: bool = conn.query_row("SELECT is_due FROM card WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert!(due, "the frozen due card is kept");
        assert!(has_sched(&conn, d.id), "the scheduler is kept");
    }

    #[test]
    fn deleting_a_deck_drops_its_scheduler() {
        let conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        assert!(has_sched(&conn, d.id));
        conn.execute(r#"DELETE FROM "group" WHERE id = ?1"#, [d.id]).unwrap();
        assert!(!has_sched(&conn, d.id), "cascade removes the scheduler");
    }

    #[test]
    fn readd_trims_the_queue_to_a_smaller_max() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        for id in 10..16 { add_card(&conn, id, d.id, 1, 0, false); }
        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 5, can_overflow: false }, &mut conn).unwrap();
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 5, "first add fills to max");

        remove_group_from_plan(d.id, false, &mut conn).unwrap();
        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 2, can_overflow: false }, &mut conn).unwrap();
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 2, "same-day re-add trims to the smaller max");
    }

    #[test]
    fn readd_grows_the_queue_to_a_larger_max() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        for id in 10..16 { add_card(&conn, id, d.id, 1, 0, false); }
        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 2, can_overflow: false }, &mut conn).unwrap();
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 2);

        remove_group_from_plan(d.id, false, &mut conn).unwrap();
        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 5, can_overflow: false }, &mut conn).unwrap();
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 5, "same-day re-add grows to the larger max");
    }

    #[test]
    fn readd_keeps_overflow_on_top_of_the_clamped_due() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 2, can_overflow: true }, &mut conn).unwrap();
        // Three carried-over overflow cards plus four fresh non-overflow due cards
        conn.execute(
            r#"INSERT INTO card (id, group_id, front, back, tier, ease, sequence, is_due, is_overdue) VALUES
               (10, ?1, 'f', 'b', 1, 0.0, 0, TRUE, TRUE), (11, ?1, 'f', 'b', 1, 0.0, 0, TRUE, TRUE),
               (12, ?1, 'f', 'b', 1, 0.0, 0, TRUE, TRUE),
               (20, ?1, 'f', 'b', 1, 0.0, 0, TRUE, FALSE), (21, ?1, 'f', 'b', 1, 0.0, 0, TRUE, FALSE),
               (22, ?1, 'f', 'b', 1, 0.0, 0, TRUE, FALSE), (23, ?1, 'f', 'b', 1, 0.0, 0, TRUE, FALSE)"#,
            [d.id],
        ).unwrap();
        remove_group_from_plan(d.id, false, &mut conn).unwrap();

        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 2, can_overflow: true }, &mut conn).unwrap();

        let overflow: i64 = conn.query_row(
            "SELECT COUNT(*) FROM card WHERE group_id = ?1 AND is_overdue = TRUE", [d.id], |r| r.get(0)).unwrap();
        let non_overflow_due: i64 = conn.query_row(
            "SELECT COUNT(*) FROM card WHERE group_id = ?1 AND is_due = TRUE AND is_overdue = FALSE", [d.id], |r| r.get(0)).unwrap();
        assert_eq!(overflow, 3, "the overflow pile is left on top");
        assert_eq!(non_overflow_due, 2, "the non-overflow due is clamped to the max");
    }

    #[test]
    fn readd_with_overflow_off_drops_the_frozen_pile() {
        let mut conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 2, can_overflow: true }, &mut conn).unwrap();
        conn.execute(
            r#"INSERT INTO card (id, group_id, front, back, tier, ease, sequence, is_due, is_overdue) VALUES
               (10, ?1, 'f', 'b', 1, 0.0, 0, TRUE, TRUE), (11, ?1, 'f', 'b', 1, 0.0, 0, TRUE, TRUE),
               (20, ?1, 'f', 'b', 1, 0.0, 0, TRUE, FALSE), (21, ?1, 'f', 'b', 1, 0.0, 0, TRUE, FALSE)"#,
            [d.id],
        ).unwrap();
        remove_group_from_plan(d.id, false, &mut conn).unwrap();

        add_group_to_plan(d.id, 1, NewScheduler { group_id: d.id, max_new: 0, max_review: 2, can_overflow: false }, &mut conn).unwrap();

        let overflow: i64 = conn.query_row(
            "SELECT COUNT(*) FROM card WHERE group_id = ?1 AND is_overdue = TRUE", [d.id], |r| r.get(0)).unwrap();
        assert_eq!(overflow, 0, "unchecking the box discards the pile, same as a tick would");
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 2, "the queue refills to the max");
    }

    #[test]
    fn benched_clamps_and_scheduler_edits_never_schedule() {
        let conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_card(&conn, 1, d.id, 1, 0, false);

        clamp_group(d.id, &conn).unwrap();
        max_clamp_group(d.id, &conn).unwrap();
        update_scheduler(
            Scheduler { group_id: d.id, studied_new: 0, max_new: 5, studied_review: 0, max_review: 5, can_overflow: false },
            &conn,
        ).unwrap();

        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 0, "a benched deck stays frozen");
    }

    #[test]
    fn benched_unpause_and_reset_never_schedule() {
        let conn = setup();
        let d = create_deck("d".into(), &conn).unwrap();
        add_card(&conn, 1, d.id, 1, 0, false);

        unpause_all(d.id, &conn).unwrap();
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 0, "benched unpause does not schedule");

        reset_deck(d.id, &conn).unwrap();
        assert_eq!(count_due_items(&d.id, &conn).unwrap().1, 0, "benched reset does not schedule");
    }
}
