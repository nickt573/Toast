use crate::app_utils::paths::{relativize_html_media, relativize_image_nodes, to_relative};
use rusqlite::{params, Connection};
use std::path::Path;

/// Written into Toast to Go packages, pull rejects a mismatch, bump on schema change
/// v4, stat runs replaced numbered versions, old Toast would query dropped columns
pub const SCHEMA_VERSION: u32 = 4;

/// CREATE TABLE IF NOT EXISTS won't touch existing tables, old databases migrate here
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let exists = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == column);
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

/// v1.3.0 rewrite absolute media paths to app-dir-relative so they still resolve after the app dir moves
/// Idempotent, relative paths pass through to_relative unchanged
fn migrate_media_paths(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let mut changed_cards = 0usize;
    let mut changed_pages = 0usize;

    {
        type CardRow = (
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
        );
        let rows: Vec<CardRow> = tx
            .prepare(
                "SELECT id, front_image, back_image, front_audio, back_audio,
                        imported_front, imported_back, imported_support, is_uploaded
                 FROM card",
            )?
            .query_map([], |row| {
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
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut update = tx.prepare(
            "UPDATE card SET front_image = ?2, back_image = ?3, front_audio = ?4,
                             back_audio = ?5, imported_front = ?6, imported_back = ?7,
                             imported_support = ?8
             WHERE id = ?1",
        )?;

        for (id, fi, bi, fa, ba, ifront, iback, isupport, is_uploaded) in rows {
            let rel = |v: &Option<String>| v.as_ref().map(|p| to_relative(p, app_dir));
            let (nfi, nbi, nfa, nba) = (rel(&fi), rel(&bi), rel(&fa), rel(&ba));

            // Media only in imported HTML, front/back/support are user text, never rewrite those
            let rel_html =
                |v: &Option<String>| v.as_ref().and_then(|s| relativize_html_media(s, app_dir));
            let (nfront, nback, nsupport) = if is_uploaded {
                (rel_html(&ifront), rel_html(&iback), rel_html(&isupport))
            } else {
                (None, None, None)
            };

            let cols_changed = nfi != fi || nbi != bi || nfa != fa || nba != ba;
            if cols_changed || nfront.is_some() || nback.is_some() || nsupport.is_some() {
                update.execute(params![
                    id,
                    nfi,
                    nbi,
                    nfa,
                    nba,
                    nfront.as_deref().or(ifront.as_deref()),
                    nback.as_deref().or(iback.as_deref()),
                    nsupport.as_deref().or(isupport.as_deref()),
                ])?;
                changed_cards += 1;
            }
        }
    }

    {
        let rows: Vec<(i64, String, Option<String>)> = tx
            .prepare("SELECT id, content, audio_file FROM page")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut update =
            tx.prepare("UPDATE page SET content = ?2, audio_file = ?3 WHERE id = ?1")?;

        for (id, content, audio_file) in rows {
            let new_audio = audio_file.as_ref().map(|p| to_relative(p, app_dir));

            // Leave unparseable content untouched
            let new_content = serde_json::from_str::<serde_json::Value>(&content)
                .ok()
                .and_then(|mut json| {
                    relativize_image_nodes(&mut json, app_dir).then(|| json.to_string())
                });

            if new_content.is_some() || new_audio != audio_file {
                update.execute(params![
                    id,
                    new_content.as_deref().unwrap_or(&content),
                    new_audio.as_deref().or(audio_file.as_deref()),
                ])?;
                changed_pages += 1;
            }
        }
    }

    tx.commit()?;

    if changed_cards > 0 || changed_pages > 0 {
        log::info!(
            "media path migration: rewrote {changed_cards} cards, {changed_pages} pages to app-dir-relative paths"
        );
    }
    Ok(())
}

fn has_autoincrement(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    // MAX over no rows still returns a row, never hits QueryReturnedNoRows
    let sql: String = conn.query_row(
        "SELECT COALESCE(MAX(sql), '') FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get(0),
    )?;
    Ok(sql.contains("AUTOINCREMENT"))
}

/// Rebuild a table with an AUTOINCREMENT key (SQLite only sets it at create), counter bumped past
/// the highest id the stats tables still reference. Pragmas set outside the transaction, rebuild runs in a real one
fn rebuild_with_autoincrement(
    conn: &Connection,
    table: &str,
    create_rebuild: &str,
    columns: &str,
    high_water_sql: &str,
) -> rusqlite::Result<()> {
    if has_autoincrement(conn, table)? {
        return Ok(());
    }
    let high: i64 = conn.query_row(high_water_sql, [], |r| r.get(0))?;

    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA legacy_alter_table = ON;")?;

    let rebuild = || -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(&format!(
            r#"
            {create_rebuild}
            INSERT INTO "{table}_rebuild" ({columns}) SELECT {columns} FROM "{table}";
            DROP TABLE "{table}";
            ALTER TABLE "{table}_rebuild" RENAME TO "{table}";
            "#
        ))?;
        // sqlite_sequence exists only after the rebuilt table is declared AUTOINCREMENT, so only here
        tx.execute("DELETE FROM sqlite_sequence WHERE name = ?1", [table])?;
        tx.execute(
            "INSERT INTO sqlite_sequence (name, seq) VALUES (?1, ?2)",
            rusqlite::params![table, high],
        )?;
        tx.commit()
    };
    let result = rebuild();

    conn.execute_batch("PRAGMA legacy_alter_table = OFF; PRAGMA foreign_keys = ON;")?;
    result
}

/// Split "~5 pages" into (5.0, "pages"), skip leading junk to the first number, trimmed rest is the name
/// None if either half is missing
fn parse_legacy_unit(raw: &str) -> Option<(f64, String)> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|b| b.is_ascii_digit())?;
    let mut end = start;
    let mut seen_dot = false;
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_digit() {
            end += 1;
        } else if c == b'.' && !seen_dot {
            seen_dot = true;
            end += 1;
        } else {
            break;
        }
    }
    // Eat a trailing dot ("5.") so the name starts clean after it
    let value: f64 = raw[start..end].trim_end_matches('.').parse().ok()?;
    let name = raw[end..].trim().to_string();
    // Zero or less isn't an allowed record, leave it unmigrated and blank
    if name.is_empty() || value <= 0.0 {
        return None;
    }
    Some((value, name))
}

/// Turn retired free-text units into unit variants, once, skipping already-migrated entries
/// Names match exactly, no case or plural folding
fn migrate_legacy_units(conn: &Connection) -> rusqlite::Result<()> {
    // Existing variants by exact name, so a partial earlier run is reused not doubled
    let mut by_name: std::collections::HashMap<String, i64> = conn
        .prepare("SELECT id, name FROM unit_variant")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .map(|(id, name)| (name, id))
        .collect();

    let rows: Vec<(i64, String)> = conn
        .prepare(
            "SELECT id, num_unit FROM todo_stats
             WHERE num_unit IS NOT NULL AND variant_id IS NULL",
        )?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    for (id, raw) in rows {
        let Some((value, name)) = parse_legacy_unit(&raw) else {
            continue;
        };
        let variant_id = match by_name.get(&name) {
            Some(&existing) => existing,
            None => {
                let vid = new_unit_group(conn, &name)?;
                by_name.insert(name, vid);
                vid
            }
        };
        conn.execute(
            "UPDATE todo_stats SET num_value = ?1, variant_id = ?2 WHERE id = ?3",
            params![value, variant_id, id],
        )?;
    }
    Ok(())
}

/// Start a new unit as one variant anchoring its own group, return its id
/// The group is the shared group_id, named by the anchor's id so it never shifts on rename or removal
fn new_unit_group(conn: &Connection, name: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO unit_variant (group_id, name, position) VALUES (0, ?1, 0)",
        params![name],
    )?;
    let vid = conn.last_insert_rowid();
    conn.execute(
        "UPDATE unit_variant SET group_id = ?1 WHERE id = ?1",
        params![vid],
    )?;
    Ok(vid)
}

/// Migrations for older databases, each call idempotent
fn migrate_schema(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    // v1.1.0 read-only Anki support, kept out of front/back and similar-card matching
    add_column_if_missing(conn, "card", "imported_support", "TEXT")?;
    // v1.2.0 manual todo order, numbered sort first, contiguous 1..N per plan (set_todo_position)
    add_column_if_missing(conn, "todo", "position", "INTEGER DEFAULT NULL")?;
    // v1.2.0 todo time is whole minutes now, round decimals from older releases
    conn.execute_batch("UPDATE todo_stats SET time_spent_minutes = ROUND(time_spent_minutes);")?;
    // v1.5.0 move uploaded Anki HTML to imported_front/back so front/back are user fields
    // Unmigrated rows are both-NULL, must run before the media-path pass
    add_column_if_missing(conn, "card", "imported_front", "TEXT")?;
    add_column_if_missing(conn, "card", "imported_back", "TEXT")?;
    conn.execute_batch(
        "UPDATE card SET imported_front = front, imported_back = back, front = '', back = ''
         WHERE is_uploaded = TRUE AND imported_front IS NULL AND imported_back IS NULL;",
    )?;
    // v1.3.0 media paths relative to the app data dir
    migrate_media_paths(conn, app_dir)?;
    // v1.5.0 skip a todo for today only, cleared on rollover and frequency change
    add_column_if_missing(conn, "todo", "is_skipped", "BOOLEAN NOT NULL DEFAULT FALSE")?;
    add_column_if_missing(
        conn,
        "group_stats",
        "is_merged",
        "BOOLEAN NOT NULL DEFAULT FALSE",
    )?;
    // Set on merge copy or reset archive, row stays for history but stops counting
    add_column_if_missing(
        conn,
        "group_stats",
        "is_archived",
        "BOOLEAN NOT NULL DEFAULT FALSE",
    )?;
    // Deck identity kept after deletion, so same-named decks never merge into one card
    add_column_if_missing(conn, "group_stats", "origin_group_id", "INTEGER")?;
    conn.execute_batch(
        "UPDATE group_stats SET origin_group_id = group_id
         WHERE origin_group_id IS NULL AND group_id IS NOT NULL;",
    )?;
    add_column_if_missing(conn, "card", "is_cram", "BOOLEAN NOT NULL DEFAULT FALSE")?;
    // A logged todo's units split into a number and a unit variant
    add_column_if_missing(conn, "todo_stats", "num_value", "FLOAT")?;
    add_column_if_missing(conn, "todo_stats", "variant_id", "INTEGER")?;
    migrate_legacy_units(conn)?;
    // v1.6.0 stat rows are kept after their deck and plan are deleted, so a reused rowid would mix their history into a new one
    // Runs last so the rebuilt tables include every column added above
    rebuild_with_autoincrement(
        conn,
        "plan",
        r#"CREATE TABLE "plan_rebuild" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL
        );"#,
        "id, name",
        "SELECT MAX(v) FROM (
            SELECT COALESCE(MAX(id), 0) AS v FROM plan
            UNION ALL SELECT COALESCE(MAX(plan_id), 0) FROM group_stats
            UNION ALL SELECT COALESCE(MAX(plan_id), 0) FROM todo_stats
         )",
    )?;
    rebuild_with_autoincrement(
        conn,
        "group",
        r#"CREATE TABLE "group_rebuild" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            plan_id INTEGER,
            name TEXT NOT NULL,
            group_type TEXT NOT NULL
                CHECK(group_type IN ('deck', 'notebook')),
            FOREIGN KEY(plan_id)
                REFERENCES plan(id)
                ON DELETE SET NULL
        );"#,
        "id, plan_id, name, group_type",
        r#"SELECT MAX(v) FROM (
            SELECT COALESCE(MAX(id), 0) AS v FROM "group"
            UNION ALL SELECT COALESCE(MAX(origin_group_id), 0) FROM group_stats
            UNION ALL SELECT COALESCE(MAX(group_id), 0) FROM group_stats
            UNION ALL SELECT COALESCE(MAX(group_id), 0) FROM todo_stat_group
         )"#,
    )?;
    // deck_reset uses a stat line's id to place it either side of a reset, so a reused id
    // would put post-reset study before the boundary. Also clears a watermark from a deleted line
    rebuild_with_autoincrement(
        conn,
        "group_stats",
        r#"CREATE TABLE "group_stats_rebuild" (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id INTEGER,
            origin_group_id INTEGER,
            plan_id INTEGER NOT NULL,
            plan_name TEXT NOT NULL DEFAULT '',
            group_name TEXT NOT NULL,
            date DATE NOT NULL,
            num_promote INTEGER NOT NULL DEFAULT 0,
            num_demote INTEGER NOT NULL DEFAULT 0,
            num_new INTEGER NOT NULL DEFAULT 0,
            time_spent_minutes FLOAT NOT NULL DEFAULT 0,
            retention_rate REAL NOT NULL DEFAULT 0,
            is_merged BOOLEAN NOT NULL DEFAULT FALSE,
            is_archived BOOLEAN NOT NULL DEFAULT FALSE,
            FOREIGN KEY(group_id)
                REFERENCES "group"(id)
                ON DELETE SET NULL
        );"#,
        "id, group_id, origin_group_id, plan_id, plan_name, group_name, date,
         num_promote, num_demote, num_new, time_spent_minutes, retention_rate,
         is_merged, is_archived",
        "SELECT MAX(v) FROM (
            SELECT COALESCE(MAX(id), 0) AS v FROM group_stats
            UNION ALL SELECT COALESCE(MAX(after_stat_id), 0) FROM deck_reset
         )",
    )?;
    // Added after the plan rebuild above, which only carries id and name across
    add_column_if_missing(conn, "plan", "longest_streak", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(conn, "plan", "is_disabled", "BOOLEAN NOT NULL DEFAULT FALSE")?;
    Ok(())
}

/// Creates all tables (idempotent) and enables foreign keys
pub fn init_schema(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    conn.execute_batch(r#"
            PRAGMA foreign_keys = ON;

            -- AUTOINCREMENT not plain rowid, group_stats still has plan_id after the plan is deleted,
            -- so a reused id would attach that history to the next plan
            CREATE TABLE IF NOT EXISTS plan (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                longest_streak INTEGER NOT NULL DEFAULT 0, -- high-water mark, bumped as the live streak grows
                is_disabled BOOLEAN NOT NULL DEFAULT FALSE -- hidden from the homepage, greyed on the plan list
            );

            CREATE TABLE IF NOT EXISTS todo (
                id INTEGER PRIMARY KEY,
                plan_id INTEGER NOT NULL,

                text TEXT NOT NULL,
                frequency INTEGER DEFAULT 127, -- 0b1111111 (every day)
                category INTEGER DEFAULT 64, -- 0b1000000 (other)

                is_done BOOLEAN NOT NULL DEFAULT FALSE,
                is_disabled BOOLEAN NOT NULL DEFAULT FALSE, -- disabled by frequency or skip
                is_skipped BOOLEAN NOT NULL DEFAULT FALSE, -- skipped for today, resets on rollover

                position INTEGER DEFAULT NULL, -- manual order, contiguous 1..N per plan, NULL sorts last

                FOREIGN KEY(plan_id)
                    REFERENCES plan(id)
                    ON DELETE CASCADE
            );

            -- AUTOINCREMENT like plan, group_stats.origin_group_id is still set after the deck is deleted,
            -- so an id must never go to a second deck
            CREATE TABLE IF NOT EXISTS "group" (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id INTEGER,

                name TEXT NOT NULL,

                group_type TEXT NOT NULL
                    CHECK(group_type IN ('deck', 'notebook')),

                FOREIGN KEY(plan_id)
                    REFERENCES plan(id)
                    ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS scheduler (
                group_id INTEGER PRIMARY KEY,

                studied_new INTEGER NOT NULL DEFAULT 0, -- *only counts non overflow cards
                max_new INTEGER NOT NULL,

                studied_review INTEGER NOT NULL DEFAULT 0, -- *only counts non overflow cards
                max_review INTEGER NOT NULL,

                can_overflow BOOLEAN NOT NULL DEFAULT FALSE, -- ex) 10/20 --> 20/20 (F) or 30/20 (T)

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE CASCADE
            );


            CREATE TABLE IF NOT EXISTS card (
                id INTEGER PRIMARY KEY,
                group_id INTEGER NOT NULL,

                front TEXT NOT NULL,
                back TEXT NOT NULL,

                support TEXT,
                -- imported_x is read-only Anki HTML from import, x is the user's own text
                imported_front TEXT,
                imported_back TEXT,
                imported_support TEXT,
                front_image TEXT,
                back_image TEXT,
                front_audio TEXT,
                back_audio TEXT,

                tier INTEGER NOT NULL DEFAULT 0,
                ease FLOAT NOT NULL DEFAULT 0, -- (-.12 -.05 +.02 +.06)
                sequence INTEGER NOT NULL DEFAULT 0, -- set to tier's value, decrements 1 per day, and due when <= 0

                is_searchable BOOLEAN NOT NULL DEFAULT FALSE,
                is_uploaded BOOLEAN NOT NULL DEFAULT FALSE, --custom Anki

                is_overdue BOOLEAN DEFAULT NULL, -- true if overdue, false if newly scheduled, null if is_due == false
                is_due BOOLEAN NOT NULL DEFAULT FALSE, -- flagged to TRUE by scheduler
                is_paused BOOLEAN NOT NULL DEFAULT FALSE, -- ignored by scheduler, does not progress sequence
                is_cram BOOLEAN NOT NULL DEFAULT FALSE, -- set when a review card is demoted, cleared by day tick or Got It

                position INTEGER DEFAULT NULL, -- zipper order set on deck merge, tiebreaker in fill_track

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS page (
                id INTEGER PRIMARY KEY,
                group_id INTEGER NOT NULL,

                title TEXT NOT NULL,
                description TEXT,

                content TEXT NOT NULL DEFAULT '{}',
                audio_file TEXT,

                created_date DATE NOT NULL,

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS resource (
                id INTEGER PRIMARY KEY,
                plan_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                type TEXT,
                url TEXT,
                notes TEXT,

                FOREIGN KEY(plan_id)
                    REFERENCES plan(id)
                    ON DELETE CASCADE
            );

            -- todo + resource join table
            CREATE TABLE IF NOT EXISTS todo_resource (
                todo_id INTEGER NOT NULL,
                resource_id INTEGER NOT NULL,
                PRIMARY KEY(todo_id, resource_id),

                FOREIGN KEY(todo_id)
                    REFERENCES todo(id)
                    ON DELETE CASCADE,

                FOREIGN KEY(resource_id)
                    REFERENCES resource(id)
                    ON DELETE CASCADE
            );

            -- todo + group join table
            CREATE TABLE IF NOT EXISTS todo_group (
                todo_id INTEGER NOT NULL,
                group_id INTEGER NOT NULL,
                PRIMARY KEY(todo_id, group_id),

                FOREIGN KEY(todo_id)
                    REFERENCES todo(id)
                    ON DELETE CASCADE,

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE CASCADE
            );

            -- Stat table for a DECK ONLY (SRS), deprecated from Notebooks
            -- AUTOINCREMENT, deck_reset marks a reset by the highest line id, a reused id would misplace a post-reset line
            CREATE TABLE IF NOT EXISTS group_stats(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER, -- NULL when the deck is deleted, how the stats page detects a dead deck
                origin_group_id INTEGER, -- still set after deletion, so same-named decks never merge into one card
                plan_id INTEGER NOT NULL, -- no FK, value persists after plan deletion so stats stay browsable
                plan_name TEXT NOT NULL DEFAULT '', -- kept for display after plan deletion, synced on rename

                group_name TEXT NOT NULL,
                date DATE NOT NULL,

                num_promote INTEGER NOT NULL DEFAULT 0, -- review card increasing in tier
                num_demote INTEGER NOT NULL DEFAULT 0, -- review card decreasing in tier (or tier 0 -> tier 0)
                num_new INTEGER NOT NULL DEFAULT 0, -- new card studied
                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                retention_rate REAL NOT NULL DEFAULT 0,

                is_merged BOOLEAN NOT NULL DEFAULT FALSE, -- this deck was merged into another one
                is_archived BOOLEAN NOT NULL DEFAULT FALSE, -- copied into a merge or archived by a reset, doesn't count

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE SET NULL
            );

            -- One reset row per deck, keyed by origin_group_id (still set after the deck is deleted)
            -- Cleaned up by sweep_orphan_resets once the deck's stat lines are gone
            CREATE TABLE IF NOT EXISTS deck_reset (
                id INTEGER PRIMARY KEY,
                origin_group_id INTEGER NOT NULL,
                date DATE NOT NULL,

                -- Highest group_stats id at the reset, splits a shared date into before and after
                -- Ordering mark only, never read, so the row at that id can be deleted freely
                after_stat_id INTEGER NOT NULL
            );

            -- A custom unit a logged todo counts in (lessons, posts, pages), stored as its spellings
            -- Variants sharing a group_id are one unit, position orders them (lowest is "main"), global across plans
            CREATE TABLE IF NOT EXISTS unit_variant (
                id INTEGER PRIMARY KEY,
                group_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS todo_stats (
                id INTEGER PRIMARY KEY,
                todo_id INTEGER , -- for data and sorting, null if free
                plan_id INTEGER NOT NULL, -- no FK, value persists after plan deletion
                plan_name TEXT NOT NULL DEFAULT '', -- kept for display after plan deletion, synced on rename

                date DATE NOT NULL,

                text TEXT NOT NULL, -- pulled from the todo's name, locked in
                category TEXT NOT NULL, -- pulled from the todo's category

                details TEXT,

                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                -- How much got done and its unit variant, both filled or both empty
                -- No FK on variant_id so the delete guard stays in one place, live name comes via a join
                num_value FLOAT,
                variant_id INTEGER,
                num_unit TEXT -- retired free-text units, kept only so older databases can migrate
            );

            -- todo_stat + group join table
            CREATE TABLE IF NOT EXISTS todo_stat_group (
                stat_id INTEGER NOT NULL,
                group_id INTEGER,
                group_name TEXT NOT NULL, -- flexible until id null
                group_type TEXT,          -- snapshot, kept after group deletion

                FOREIGN KEY(stat_id)
                    REFERENCES todo_stats(id)
                    ON DELETE CASCADE,

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE SET NULL
            );

            -- todo_stat + resource join table
            CREATE TABLE IF NOT EXISTS todo_stat_resource (
                stat_id INTEGER NOT NULL,
                resource_id INTEGER,
                resource_name TEXT NOT NULL, -- snapshot, live-overridden via COALESCE until id null
                resource_url TEXT,           -- snapshot of url / type / notes (same persistence as name)
                resource_type TEXT,
                resource_notes TEXT,

                FOREIGN KEY(stat_id)
                    REFERENCES todo_stats(id)
                    ON DELETE CASCADE,

                FOREIGN KEY(resource_id)
                    REFERENCES resource(id)
                    ON DELETE SET NULL
            );

            -- Per-card grade event log
            CREATE TABLE IF NOT EXISTS card_grade_log (
                id         INTEGER PRIMARY KEY,
                card_id    INTEGER NOT NULL,
                grade      INTEGER NOT NULL,
                graded_at  TEXT NOT NULL,
                old_tier   INTEGER NOT NULL,
                new_tier   INTEGER NOT NULL,
                FOREIGN KEY(card_id) REFERENCES card(id) ON DELETE CASCADE
            );

            -- Singleton table
            CREATE TABLE IF NOT EXISTS app_date (
                id INTEGER UNIQUE DEFAULT 0, -- for querying this specific column
                date DATE NOT NULL
            );
            "#
    )?;

    migrate_schema(conn, app_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app_dir() -> PathBuf {
        PathBuf::from("/home/alice/.local/share/com.toast.app")
    }

    #[test]
    fn legacy_units_split_at_the_first_number() {
        assert_eq!(parse_legacy_unit("~5 pages"), Some((5.0, "pages".into())));
        assert_eq!(parse_legacy_unit("5.5 chapters"), Some((5.5, "chapters".into())));
        // Word before the number is skipped too
        assert_eq!(parse_legacy_unit("read 5 pages"), Some((5.0, "pages".into())));
        // Everything past the number is the name, spaces and all
        assert_eq!(parse_legacy_unit("3 lessons done"), Some((3.0, "lessons done".into())));
        // Trailing dot stays out of the number
        assert_eq!(parse_legacy_unit("2. articles"), Some((2.0, "articles".into())));
    }

    #[test]
    fn legacy_units_with_no_pair_migrate_to_nothing() {
        // No number at all
        assert_eq!(parse_legacy_unit("pages"), None);
        // Number but no name, neither field fills
        assert_eq!(parse_legacy_unit("5"), None);
        assert_eq!(parse_legacy_unit("  12  "), None);
        assert_eq!(parse_legacy_unit(""), None);
    }

    #[test]
    fn identical_legacy_unit_names_share_one_variant_but_others_stay_apart() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn, &app_dir()).unwrap();
        conn.execute("INSERT INTO plan (name) VALUES ('p')", []).unwrap();
        let plan_id = conn.last_insert_rowid();

        // Same spelling twice, a case variant (its own unit, no folding), an
        // unparseable string, and a bare number
        for raw in ["5 pages", "read 3 pages", "2 Pages", "grandma's address", "10"] {
            conn.execute(
                "INSERT INTO todo_stats (plan_id, date, text, category, num_unit)
                 VALUES (?1, '2026-01-01', 't', 'Reading', ?2)",
                params![plan_id, raw],
            )
            .unwrap();
        }
        migrate_legacy_units(&conn).unwrap();

        // "pages" is one variant shared by both entries that spelled it that way
        let pages: i64 = conn
            .query_row("SELECT COUNT(*) FROM unit_variant WHERE name = 'pages'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pages, 1, "identical spellings must not duplicate");
        // "pages" and "Pages" differ, so two separate units
        let variants: i64 = conn.query_row("SELECT COUNT(*) FROM unit_variant", [], |r| r.get(0)).unwrap();
        assert_eq!(variants, 2, "different spellings each get their own unit");
        let filled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM todo_stats WHERE num_value IS NOT NULL AND variant_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(filled, 3, "the three parseable entries carry units");

        // Each migrated variant anchors its own group (group_id equals its own id)
        let anchored: i64 = conn
            .query_row("SELECT COUNT(*) FROM unit_variant WHERE group_id = id", [], |r| r.get(0))
            .unwrap();
        assert_eq!(anchored, 2, "a fresh variant names its own group");

        // Re-running changes nothing, converted rows skipped, failures stay blank
        migrate_legacy_units(&conn).unwrap();
        let after: i64 = conn.query_row("SELECT COUNT(*) FROM unit_variant", [], |r| r.get(0)).unwrap();
        assert_eq!(after, 2, "a second pass must not add units");
    }

    // group_stats still has origin_group_id and plan_id after the deck or plan is deleted, so a
    // reused rowid would attach one thing's history to the next. A plain INTEGER PRIMARY KEY does that
    #[test]
    fn deleted_decks_and_plans_never_hand_their_id_to_the_next_one() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn, &app_dir()).unwrap();

        conn.execute(
            r#"INSERT INTO "group" (name, group_type) VALUES ('deck one', 'deck')"#,
            [],
        )
        .unwrap();
        let first_deck = conn.last_insert_rowid();
        conn.execute(r#"DELETE FROM "group" WHERE id = ?1"#, [first_deck])
            .unwrap();
        conn.execute(
            r#"INSERT INTO "group" (name, group_type) VALUES ('deck two', 'deck')"#,
            [],
        )
        .unwrap();
        assert_ne!(
            conn.last_insert_rowid(),
            first_deck,
            "a new deck must not inherit a deleted deck's stats"
        );

        conn.execute("INSERT INTO plan (name) VALUES ('plan one')", [])
            .unwrap();
        let first_plan = conn.last_insert_rowid();
        conn.execute("DELETE FROM plan WHERE id = ?1", [first_plan])
            .unwrap();
        conn.execute("INSERT INTO plan (name) VALUES ('plan two')", [])
            .unwrap();
        assert_ne!(
            conn.last_insert_rowid(),
            first_plan,
            "a new plan must not inherit a deleted plan's stats"
        );
    }

    // The migration must cover ids freed before it ran, the dangerous case. The row is gone so
    // MAX(id) misses it, but group_stats still references it and would reuse it for the next deck
    #[test]
    fn upgrading_an_old_database_keeps_freed_ids_out_of_circulation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE plan (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE "group" (
                id INTEGER PRIMARY KEY,
                plan_id INTEGER,
                name TEXT NOT NULL,
                group_type TEXT NOT NULL CHECK(group_type IN ('deck', 'notebook'))
            );
            CREATE TABLE group_stats(
                id INTEGER PRIMARY KEY,
                group_id INTEGER,
                origin_group_id INTEGER,
                plan_id INTEGER NOT NULL,
                plan_name TEXT NOT NULL DEFAULT '',
                group_name TEXT NOT NULL,
                date DATE NOT NULL,
                num_promote INTEGER NOT NULL DEFAULT 0,
                num_demote INTEGER NOT NULL DEFAULT 0,
                num_new INTEGER NOT NULL DEFAULT 0,
                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                retention_rate REAL NOT NULL DEFAULT 0
            );
            INSERT INTO plan (id, name) VALUES (7, 'old plan');
            INSERT INTO "group" (id, plan_id, name, group_type) VALUES (9, 7, 'old deck', 'deck');
            -- study logged against both then both deleted, as before an upgrade. group_id goes null
            -- with the deck, origin_group_id is what makes the history addressable so it stays reserved
            INSERT INTO group_stats (group_id, origin_group_id, plan_id, group_name, date, num_new)
            VALUES (NULL, 9, 7, 'old deck', '2026-07-01', 12);
            DELETE FROM "group" WHERE id = 9;
            DELETE FROM plan WHERE id = 7;
            "#,
        )
        .unwrap();

        init_schema(&conn, &app_dir()).unwrap();

        conn.execute("INSERT INTO plan (name) VALUES ('brand new plan')", [])
            .unwrap();
        let new_plan = conn.last_insert_rowid();
        assert!(
            new_plan > 7,
            "a new plan reused id {new_plan}, inheriting the deleted plan's stats"
        );

        conn.execute(
            r#"INSERT INTO "group" (name, group_type) VALUES ('brand new deck', 'deck')"#,
            [],
        )
        .unwrap();
        let new_deck = conn.last_insert_rowid();
        assert!(
            new_deck > 9,
            "a new deck reused id {new_deck}, inheriting the deleted deck's stats"
        );

        // Rebuilding must not have cost anything that was already there
        let kept: i64 = conn
            .query_row("SELECT num_new FROM group_stats WHERE plan_id = 7", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 12, "existing history survives the rebuild");
    }

    #[test]
    fn upgrades_a_real_pre_stat_run_database() {
        // group and group_stats as released, before resets had anywhere to record. New columns come
        // from migration, since CREATE TABLE IF NOT EXISTS leaves an existing table alone
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE "group" (
                id INTEGER PRIMARY KEY,
                plan_id INTEGER,
                name TEXT NOT NULL,
                group_type TEXT NOT NULL CHECK(group_type IN ('deck', 'notebook'))
            );
            CREATE TABLE group_stats(
                id INTEGER PRIMARY KEY,
                group_id INTEGER,
                plan_id INTEGER NOT NULL,
                plan_name TEXT NOT NULL DEFAULT '',
                group_name TEXT NOT NULL,
                date DATE NOT NULL,
                num_promote INTEGER NOT NULL DEFAULT 0,
                num_demote INTEGER NOT NULL DEFAULT 0,
                num_new INTEGER NOT NULL DEFAULT 0,
                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                retention_rate REAL NOT NULL DEFAULT 0
            );
            INSERT INTO "group" (id, plan_id, name, group_type) VALUES (1, 1, 'deck a', 'deck');
            INSERT INTO group_stats (id, group_id, plan_id, group_name, date, num_new)
            VALUES (1, 1, 1, 'deck a', '2026-07-01', 4);
            "#,
        )
        .unwrap();

        init_schema(&conn, &app_dir()).unwrap();

        // The stats page reads through here, so this is the query that was failing
        let rows = crate::crud::read::get_group_stats(1, &conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].num_new, 4, "the study survives the upgrade");
        assert!(!rows[0].is_archived);
        assert_eq!(
            rows[0].origin_group_id,
            Some(1),
            "backfilled so the deck keeps its identity"
        );

        let resets: i64 = conn
            .query_row("SELECT COUNT(*) FROM deck_reset", [], |r| r.get(0))
            .unwrap();
        assert_eq!(resets, 0, "an upgraded deck has never been reset");

        // Rebuilt table must keep issuing ids above those in use, since a reset marks its place
        // with the highest id at the time
        conn.execute(
            "INSERT INTO group_stats (group_id, origin_group_id, plan_id, group_name, date)
             VALUES (1, 1, 1, 'deck a', '2026-07-02')",
            [],
        )
        .unwrap();
        assert!(
            conn.last_insert_rowid() > 1,
            "a new line reused the id of an existing one"
        );
    }

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn, &app_dir()).unwrap();
        conn.execute(
            "INSERT INTO \"group\" (id, name, group_type) VALUES (1, 'g', 'deck'), (2, 'n', 'notebook')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn migrates_absolute_media_paths_to_relative() {
        let conn = setup();
        let stale = "/home/renamed/.local/share/com.toast.app";
        let cur = "/home/alice/.local/share/com.toast.app";

        conn.execute(
            &format!(
                "INSERT INTO card (id, group_id, front, back, imported_front, imported_back, imported_support, front_image, back_image, front_audio, back_audio, is_uploaded)
                 VALUES
                 (1, 1, 'plain front mentioning /home/alice/x.png', 'back', NULL, NULL, NULL,
                  '{cur}/cards/images/a.png', '{stale}/cards/images/b.png',
                  '{cur}/cards/audio/c.mp3', NULL, FALSE),
                 (2, 1, '', '', '<img src=\"{cur}/cards/images/d.png\">', '<audio controls src=\"{stale}/cards/audio/e.mp3\"></audio>',
                  '<img src=\"{cur}/cards/images/f.png\">', NULL, NULL, NULL, NULL, TRUE)"
            ),
            [],
        )
        .unwrap();
        conn.execute(
            &format!(
                "INSERT INTO page (id, group_id, title, content, audio_file, created_date)
                 VALUES (1, 2, 'p',
                 '{{\"type\":\"doc\",\"content\":[{{\"type\":\"image\",\"attrs\":{{\"src\":\"{cur}/pages/images/g.png\",\"rawPath\":\"/home/alice/Pictures/orig.png\"}}}}]}}',
                 '{stale}/pages/audio/h.mp4', '2026-01-01')"
            ),
            [],
        )
        .unwrap();

        migrate_media_paths(&conn, &app_dir()).unwrap();

        let (fi, bi, fa, front): (String, String, String, String) = conn
            .query_row(
                "SELECT front_image, back_image, front_audio, front FROM card WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(fi, "cards/images/a.png");
        assert_eq!(bi, "cards/images/b.png");
        assert_eq!(fa, "cards/audio/c.mp3");
        // non-uploaded card text is never rewritten
        assert_eq!(front, "plain front mentioning /home/alice/x.png");

        let (ufront, uback, usupport): (String, String, String) = conn
            .query_row(
                "SELECT imported_front, imported_back, imported_support FROM card WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(ufront, "<img src=\"cards/images/d.png\">");
        assert_eq!(uback, "<audio controls src=\"cards/audio/e.mp3\"></audio>");
        assert_eq!(usupport, "<img src=\"cards/images/f.png\">");

        let (content, audio): (String, String) = conn
            .query_row(
                "SELECT content, audio_file FROM page WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(content.contains("\"src\":\"pages/images/g.png\""));
        // rawPath synced to the stored copy, not the originally picked file
        assert!(content.contains("\"rawPath\":\"pages/images/g.png\""));
        assert_eq!(audio, "pages/audio/h.mp4");

        // Idempotent, a second run must leave every row byte-identical
        migrate_media_paths(&conn, &app_dir()).unwrap();
        let content2: String = conn
            .query_row("SELECT content FROM page WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, content2);
    }

    #[test]
    fn moves_uploaded_html_into_imported_columns() {
        let conn = setup();
        let stale = "/home/renamed/.local/share/com.toast.app";
        conn.execute(
            &format!(
                "INSERT INTO card (id, group_id, front, back, is_uploaded)
                 VALUES
                 (1, 1, '<img src=\"{stale}/cards/images/a.png\">', 'back html', TRUE),
                 (2, 1, 'custom front', 'custom back', FALSE)"
            ),
            [],
        )
        .unwrap();

        migrate_schema(&conn, &app_dir()).unwrap();

        let (front, back, ifront, iback): (String, String, String, String) = conn
            .query_row(
                "SELECT front, back, imported_front, imported_back FROM card WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(front, "");
        assert_eq!(back, "");
        // moved and relativized in the same startup
        assert_eq!(ifront, "<img src=\"cards/images/a.png\">");
        assert_eq!(iback, "back html");

        let (cfront, cifront): (String, Option<String>) = conn
            .query_row(
                "SELECT front, imported_front FROM card WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(cfront, "custom front");
        assert!(cifront.is_none());

        // a user front typed after migration must still be there after the next startup
        conn.execute("UPDATE card SET front = 'my note' WHERE id = 1", [])
            .unwrap();
        migrate_schema(&conn, &app_dir()).unwrap();
        let front2: String = conn
            .query_row("SELECT front FROM card WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(front2, "my note");
    }

    #[test]
    fn upgrades_a_real_pre_imported_columns_database() {
        // The card table as released before imported_front/imported_back existed
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE card (
                id INTEGER PRIMARY KEY,
                group_id INTEGER NOT NULL,
                front TEXT NOT NULL,
                back TEXT NOT NULL,
                support TEXT,
                imported_support TEXT,
                front_image TEXT,
                back_image TEXT,
                front_audio TEXT,
                back_audio TEXT,
                tier INTEGER NOT NULL DEFAULT 0,
                ease FLOAT NOT NULL DEFAULT 0,
                sequence INTEGER NOT NULL DEFAULT 0,
                is_searchable BOOLEAN NOT NULL DEFAULT FALSE,
                is_uploaded BOOLEAN NOT NULL DEFAULT FALSE,
                is_overdue BOOLEAN DEFAULT NULL,
                is_due BOOLEAN NOT NULL DEFAULT FALSE,
                is_paused BOOLEAN NOT NULL DEFAULT FALSE,
                position INTEGER DEFAULT NULL
            );
            INSERT INTO card (id, group_id, front, back, imported_support, support, tier, is_uploaded)
            VALUES
              (1, 1, '<img src="/home/renamed/.local/share/com.toast.app/cards/images/a.png">',
                     '<b>anki back</b>', '<i>anki support</i>', NULL, 3, TRUE),
              (2, 1, 'custom front', 'custom back', NULL, 'my support', 5, FALSE);
            "#,
        )
        .unwrap();

        init_schema(&conn, &app_dir()).unwrap();

        let (front, back, ifront, iback, isupport, tier): (
            String,
            String,
            String,
            String,
            String,
            i64,
        ) = conn
            .query_row(
                "SELECT front, back, imported_front, imported_back, imported_support, tier
                 FROM card WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(front, "");
        assert_eq!(back, "");
        assert_eq!(ifront, "<img src=\"cards/images/a.png\">");
        assert_eq!(iback, "<b>anki back</b>");
        assert_eq!(isupport, "<i>anki support</i>");
        assert_eq!(tier, 3, "SRS state must survive the column move");

        let (cfront, cback, cifront, csupport): (String, String, Option<String>, String) = conn
            .query_row(
                "SELECT front, back, imported_front, support FROM card WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(cfront, "custom front");
        assert_eq!(cback, "custom back");
        assert!(cifront.is_none());
        assert_eq!(csupport, "my support");

        // A second launch must be a no-op and not affect the db again
        conn.execute("UPDATE card SET front = 'my note' WHERE id = 1", [])
            .unwrap();
        init_schema(&conn, &app_dir()).unwrap();
        let front2: String = conn
            .query_row("SELECT front FROM card WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(front2, "my note");
    }

    /// The column move must carry media references, cleanup only scans imported_*, so
    /// anything left in front/back would look orphaned
    #[test]
    fn migrated_html_media_survives_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("cards/images")).unwrap();
        std::fs::create_dir_all(dir.join("cards/audio")).unwrap();
        std::fs::write(dir.join("cards/images/pic.png"), "x").unwrap();
        std::fs::write(dir.join("cards/audio/say.mp3"), "x").unwrap();
        std::fs::write(dir.join("cards/images/orphan.png"), "x").unwrap();

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE card (
                id INTEGER PRIMARY KEY, group_id INTEGER NOT NULL,
                front TEXT NOT NULL, back TEXT NOT NULL,
                support TEXT, imported_support TEXT,
                front_image TEXT, back_image TEXT, front_audio TEXT, back_audio TEXT,
                tier INTEGER NOT NULL DEFAULT 0, ease FLOAT NOT NULL DEFAULT 0,
                sequence INTEGER NOT NULL DEFAULT 0,
                is_searchable BOOLEAN NOT NULL DEFAULT FALSE,
                is_uploaded BOOLEAN NOT NULL DEFAULT FALSE,
                is_overdue BOOLEAN DEFAULT NULL, is_due BOOLEAN NOT NULL DEFAULT FALSE,
                is_paused BOOLEAN NOT NULL DEFAULT FALSE, position INTEGER DEFAULT NULL
            );
            INSERT INTO card (id, group_id, front, back, is_uploaded) VALUES
              (1, 1, '<img src="cards/images/pic.png">',
                     '<audio controls src="cards/audio/say.mp3"></audio>', TRUE);
            "#,
        )
        .unwrap();

        init_schema(&conn, dir).unwrap();
        let deleted = crate::crud::delete::cleanup_orphaned_media(&conn, dir).unwrap();

        assert_eq!(deleted, 1, "only the genuine orphan should go");
        assert!(dir.join("cards/images/pic.png").exists(), "front media wiped");
        assert!(dir.join("cards/audio/say.mp3").exists(), "back media wiped");
        assert!(!dir.join("cards/images/orphan.png").exists());
    }

    #[test]
    fn migration_skips_unparseable_page_content() {
        let conn = setup();
        conn.execute(
            "INSERT INTO page (id, group_id, title, content, created_date)
             VALUES (1, 2, 'p', 'not valid json {', '2026-01-01')",
            [],
        )
        .unwrap();
        migrate_media_paths(&conn, &app_dir()).unwrap();
        let content: String = conn
            .query_row("SELECT content FROM page WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, "not valid json {");
    }
}
