use crate::app_utils::paths::{relativize_html_media, relativize_image_nodes, to_relative};
use rusqlite::{params, Connection};
use std::path::Path;

/// Stamped into Toast to Go packages. A pull rejects a mismatch. Bump on any schema change.
/// 4: stat runs replaced numbered versions, so group_stats.version and
/// group.stat_version are gone. An older Toast pulling one of these packages would
/// find its queries referring to columns that no longer exist.
pub const SCHEMA_VERSION: u32 = 4;

/// Adds a column to an existing table if it doesn't already have it.
/// CREATE TABLE IF NOT EXISTS won't alter tables that predate a new column,
/// so databases from released versions are migrated here.
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

/// v1.3.0: media references used to be stored as absolute paths rooted in the
/// user's home directory, which broke when the app dir moved (new machine,
/// renamed username). Rewrites every stored reference to be relative to the
/// app data dir ("cards/images/<uuid>.png"). Idempotent: relative paths pass
/// through to_relative unchanged, so re-running on every startup is free and
/// also converts databases restored from old backups.
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

            // Embedded media only lives in imported HTML. front/back/support are
            // user prose that could contain literal paths, never rewrite those.
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

            // Unparseable content is left untouched rather than clobbered
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
    // MAX over no rows still returns one row, so this never hits QueryReturnedNoRows
    let sql: String = conn.query_row(
        "SELECT COALESCE(MAX(sql), '') FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get(0),
    )?;
    Ok(sql.contains("AUTOINCREMENT"))
}

/// Rebuilds a table so its key is AUTOINCREMENT, which SQLite can only apply when the
/// table is created. Ids are carried over unchanged, then the counter is pushed past
/// the highest id the stats tables still point at, because the whole danger is a gap
/// left behind by an old delete: those ids are free again but their history isn't.
///
/// Follows SQLite's documented rebuild procedure. legacy_alter_table keeps the rename
/// from rewriting other tables' foreign keys, which already name the real table.
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

    conn.execute_batch(&format!(
        r#"
        PRAGMA foreign_keys = OFF;
        PRAGMA legacy_alter_table = ON;
        BEGIN;
        {create_rebuild}
        INSERT INTO "{table}_rebuild" ({columns}) SELECT {columns} FROM "{table}";
        DROP TABLE "{table}";
        ALTER TABLE "{table}_rebuild" RENAME TO "{table}";
        COMMIT;
        PRAGMA legacy_alter_table = OFF;
        PRAGMA foreign_keys = ON;
        "#
    ))?;

    conn.execute("DELETE FROM sqlite_sequence WHERE name = ?1", [table])?;
    conn.execute(
        "INSERT INTO sqlite_sequence (name, seq) VALUES (?1, ?2)",
        rusqlite::params![table, high],
    )?;
    Ok(())
}

/// Migrations for databases created by older releases. Each call is idempotent.
fn migrate_schema(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    // v1.1.0: read-only support content mapped from Anki fields on import,
    // kept separate from front/back so it stays out of similar-card matching.
    add_column_if_missing(conn, "card", "imported_support", "TEXT")?;
    // v1.2.0: optional manual order for todos. Numbered todos sort ahead of
    // unnumbered ones and stay contiguous 1..N per plan (see set_todo_position).
    add_column_if_missing(conn, "todo", "position", "INTEGER DEFAULT NULL")?;
    // v1.2.0: todo time is whole minutes now; round decimals logged by older
    // releases (idempotent, the column itself stays FLOAT).
    conn.execute_batch("UPDATE todo_stats SET time_spent_minutes = ROUND(time_spent_minutes);")?;
    // v1.5.0: uploaded cards' Anki HTML moves to imported_front/back so front/back
    // become user fields. Only unmigrated rows are both-NULL (the importer always
    // writes these columns), and this must run before the media-path pass.
    add_column_if_missing(conn, "card", "imported_front", "TEXT")?;
    add_column_if_missing(conn, "card", "imported_back", "TEXT")?;
    conn.execute_batch(
        "UPDATE card SET imported_front = front, imported_back = back, front = '', back = ''
         WHERE is_uploaded = TRUE AND imported_front IS NULL AND imported_back IS NULL;",
    )?;
    // v1.3.0: media paths stored relative to the app data dir.
    migrate_media_paths(conn, app_dir)?;
    // v1.5.0: skip a todo for today only. Cleared on day rollover and when the
    // todo's frequency changes.
    add_column_if_missing(conn, "todo", "is_skipped", "BOOLEAN NOT NULL DEFAULT FALSE")?;
    add_column_if_missing(
        conn,
        "group_stats",
        "is_merged",
        "BOOLEAN NOT NULL DEFAULT FALSE",
    )?;
    // Set when a merge copies a row onto the new deck, and when a reset archives the
    // run it ended. Either way the row stays for history but stops counting.
    add_column_if_missing(
        conn,
        "group_stats",
        "is_archived",
        "BOOLEAN NOT NULL DEFAULT FALSE",
    )?;
    // Deck identity that outlives deletion, so two decks sharing a name never
    // collapse into one card. Rows whose deck is already gone stay NULL.
    add_column_if_missing(conn, "group_stats", "origin_group_id", "INTEGER")?;
    conn.execute_batch(
        "UPDATE group_stats SET origin_group_id = group_id
         WHERE origin_group_id IS NULL AND group_id IS NOT NULL;",
    )?;
    // v1.6.0: a reset flags the deck instead of numbering its stats, and the first row
    // of the run it opens carries the marker so the table can show where it began.
    // Quoted, group is a reserved word and PRAGMA table_info won't parse it bare
    add_column_if_missing(
        conn,
        "\"group\"",
        "was_reset",
        "BOOLEAN NOT NULL DEFAULT FALSE",
    )?;
    add_column_if_missing(
        conn,
        "group_stats",
        "starts_era",
        "BOOLEAN NOT NULL DEFAULT FALSE",
    )?;
    // v1.6.0: stats outlive the deck and plan they belong to, so a rowid handed out
    // twice hands one thing's history to the next thing created. Runs last, since the
    // rebuilt tables have to include every column added above.
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
            was_reset BOOLEAN NOT NULL DEFAULT FALSE,
            FOREIGN KEY(plan_id)
                REFERENCES plan(id)
                ON DELETE SET NULL
        );"#,
        "id, plan_id, name, group_type, was_reset",
        r#"SELECT MAX(v) FROM (
            SELECT COALESCE(MAX(id), 0) AS v FROM "group"
            UNION ALL SELECT COALESCE(MAX(origin_group_id), 0) FROM group_stats
            UNION ALL SELECT COALESCE(MAX(group_id), 0) FROM group_stats
            UNION ALL SELECT COALESCE(MAX(group_id), 0) FROM todo_stat_group
         )"#,
    )?;
    Ok(())
}

/// Creates all tables (idempotent) and enables foreign keys.
pub fn init_schema(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    conn.execute_batch(r#"
            PRAGMA foreign_keys = ON;

            -- AUTOINCREMENT, not a plain rowid: group_stats keeps plan_id after the
            -- plan is gone so its history stays browsable, and a reissued id would
            -- hand all of it to whatever plan is created next.
            CREATE TABLE IF NOT EXISTS plan (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL
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

                position INTEGER DEFAULT NULL, -- manual order; contiguous 1..N per plan, NULL sorts last

                FOREIGN KEY(plan_id)
                    REFERENCES plan(id)
                    ON DELETE CASCADE
            );

            -- AUTOINCREMENT for the same reason as plan: group_stats.origin_group_id
            -- outlives the deck, so an id must never be handed to a second deck.
            CREATE TABLE IF NOT EXISTS "group" (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id INTEGER,

                name TEXT NOT NULL,

                group_type TEXT NOT NULL
                    CHECK(group_type IN ('deck', 'notebook')),

                was_reset BOOLEAN NOT NULL DEFAULT FALSE, -- a reset sets this; the next session opened starts its own row instead of adding to the day's

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
                -- imported_x: read-only Anki HTML from import; x is the user's own text
                imported_front TEXT,
                imported_back TEXT,
                imported_support TEXT,
                front_image TEXT,
                back_image TEXT,
                front_audio TEXT,
                back_audio TEXT,

                tier INTEGER NOT NULL DEFAULT 0, -- the number of its tier,
                ease FLOAT NOT NULL DEFAULT 0, -- (-.12 -.05 +.02 +.06)
                sequence INTEGER NOT NULL DEFAULT 0, -- set to tier's value, decrements 1 per day, and due when <= 0

                is_searchable BOOLEAN NOT NULL DEFAULT FALSE,
                is_uploaded BOOLEAN NOT NULL DEFAULT FALSE, --custom Anki

                -- SRS info
                is_overdue BOOLEAN DEFAULT NULL, -- true if overdue, false if newly scheduled, null if is_due == false
                is_due BOOLEAN NOT NULL DEFAULT FALSE, -- flagged to TRUE by scheduler
                is_paused BOOLEAN NOT NULL DEFAULT FALSE, -- ignored by scheduler, does not progress sequence

                position INTEGER DEFAULT NULL, -- zipper order set on deck merge; tiebreaker in fill_track

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
            CREATE TABLE IF NOT EXISTS group_stats(
                id INTEGER PRIMARY KEY,
                group_id INTEGER, -- goes NULL when the deck is deleted, which is how the stats page spots a dead deck
                origin_group_id INTEGER, -- survives deletion, so same named decks never merge into one card
                plan_id INTEGER NOT NULL, -- no FK: value persists after plan deletion so stats remain browsable
                plan_name TEXT NOT NULL DEFAULT '', -- preserved for display after plan deletion; synced on rename

                group_name TEXT NOT NULL,
                date DATE NOT NULL,

                num_promote INTEGER NOT NULL DEFAULT 0, -- review card increasing in tier
                num_demote INTEGER NOT NULL DEFAULT 0, -- review card decreasing in tier (or tier 0 -> tier 0)
                num_new INTEGER NOT NULL DEFAULT 0, -- new card studied
                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                retention_rate REAL NOT NULL DEFAULT 0,

                starts_era BOOLEAN NOT NULL DEFAULT FALSE, -- first row after a reset, so the table can mark where a run began
                is_merged BOOLEAN NOT NULL DEFAULT FALSE, -- this deck was merged into another one
                is_archived BOOLEAN NOT NULL DEFAULT FALSE, -- copied into a merge, or archived by the reset that ended its run; either way it doesn't count

                FOREIGN KEY(group_id)
                    REFERENCES "group"(id)
                    ON DELETE SET NULL
            );

            -- Stat table for a todo
            CREATE TABLE IF NOT EXISTS todo_stats (
                id INTEGER PRIMARY KEY,
                todo_id INTEGER , -- for the purpose of collecting data and sorting (null if free)
                plan_id INTEGER NOT NULL, -- no FK: value persists after plan deletion
                plan_name TEXT NOT NULL DEFAULT '', -- preserved for display after plan deletion; synced on rename

                date DATE NOT NULL,

                text TEXT NOT NULL, -- pulled from the todo's name, locked in
                category TEXT NOT NULL, -- pulled from the todo's category,

                details TEXT,

                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                num_unit TEXT -- pages, minutes, books, chapters, etc
            );

            -- todo_stat + group join table
            CREATE TABLE IF NOT EXISTS todo_stat_group (
                stat_id INTEGER NOT NULL,
                group_id INTEGER,
                group_name TEXT NOT NULL, -- flexible until id null
                group_type TEXT,          -- snapshot; preserved after group deletion

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

    // group_stats holds onto origin_group_id and plan_id long after the deck or plan
    // is deleted, so a reissued rowid silently hands one thing's history to the next
    // thing created. A plain INTEGER PRIMARY KEY does exactly that.
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

    // The migration has to cover ids that were already freed before it ran. Those are
    // the dangerous ones: the row is gone, so MAX(id) forgets it, but group_stats
    // still points at it and would hand that history to the next deck created.
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
            -- study logged against both, then both deleted, exactly as before an
            -- upgrade. group_id goes null with the deck, origin_group_id is what
            -- keeps the history addressable, and so is what has to stay reserved.
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
        // group and group_stats as released, before a reset had anywhere to record
        // itself. New columns land by migration, since CREATE TABLE IF NOT EXISTS
        // leaves an existing table alone.
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
        assert!(!rows[0].starts_era, "an old line never began a run");
        assert!(!rows[0].is_archived);
        assert_eq!(
            rows[0].origin_group_id,
            Some(1),
            "backfilled so the deck keeps its identity"
        );

        let flag: bool = conn
            .query_row(r#"SELECT was_reset FROM "group" WHERE id = 1"#, [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!flag, "an upgraded deck has no reset pending");
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

        // Idempotent: a second run must leave every row byte-identical
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

        // a user front typed after migration must survive the next startup
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
        // The card table as released before imported_front/imported_back existed.
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

    /// The column move must carry media references with it: cleanup only scans
    /// imported_*, so anything still sitting in front/back would look orphaned.
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
