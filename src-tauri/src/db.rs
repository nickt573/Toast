use crate::app_utils::paths::{relativize_html_media, relativize_image_nodes, to_relative};
use rusqlite::{params, Connection};
use std::path::Path;

/// Stamped into Toast to Go packages; a pull rejects a mismatch. Bump on any
/// schema change.
pub const SCHEMA_VERSION: u32 = 1;

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
            String,
            String,
            Option<String>,
            bool,
        );
        let rows: Vec<CardRow> = tx
            .prepare(
                "SELECT id, front_image, back_image, front_audio, back_audio,
                        front, back, imported_support, is_uploaded
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
                             back_audio = ?5, front = ?6, back = ?7, imported_support = ?8
             WHERE id = ?1",
        )?;

        for (id, fi, bi, fa, ba, front, back, support, is_uploaded) in rows {
            let rel = |v: &Option<String>| v.as_ref().map(|p| to_relative(p, app_dir));
            let (nfi, nbi, nfa, nba) = (rel(&fi), rel(&bi), rel(&fa), rel(&ba));

            // Only uploaded cards embed media in their HTML; custom card text
            // is user prose and could contain literal paths — never rewrite it.
            let (nfront, nback, nsupport) = if is_uploaded {
                (
                    relativize_html_media(&front, app_dir),
                    relativize_html_media(&back, app_dir),
                    support
                        .as_ref()
                        .and_then(|s| relativize_html_media(s, app_dir)),
                )
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
                    nfront.as_deref().unwrap_or(&front),
                    nback.as_deref().unwrap_or(&back),
                    nsupport.as_deref().or(support.as_deref()),
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

/// Migrations for databases created by older releases. Each call is idempotent.
fn migrate_schema(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    // v1.1.0: read-only support content mapped from Anki fields on import,
    // kept separate from front/back so it stays out of similar-card matching.
    add_column_if_missing(conn, "card", "imported_support", "TEXT")?;
    // v1.2.0: optional manual order for todos; numbered todos sort ahead of
    // unnumbered ones and stay contiguous 1..N per plan (see set_todo_position).
    add_column_if_missing(conn, "todo", "position", "INTEGER DEFAULT NULL")?;
    // v1.2.0: todo time is whole minutes now; round decimals logged by older
    // releases (idempotent, the column itself stays FLOAT).
    conn.execute_batch("UPDATE todo_stats SET time_spent_minutes = ROUND(time_spent_minutes);")?;
    // v1.3.0: media paths stored relative to the app data dir.
    migrate_media_paths(conn, app_dir)?;
    Ok(())
}

/// Creates all tables (idempotent) and enables foreign keys.
pub fn init_schema(conn: &Connection, app_dir: &Path) -> rusqlite::Result<()> {
    conn.execute_batch(r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS plan (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS todo (
                id INTEGER PRIMARY KEY,
                plan_id INTEGER NOT NULL,

                text TEXT NOT NULL,
                frequency INTEGER DEFAULT 127, -- 0b1111111 (every day)
                category INTEGER DEFAULT 64, -- 0b1000000 (other)

                is_done BOOLEAN NOT NULL DEFAULT FALSE,
                is_disabled BOOLEAN NOT NULL DEFAULT FALSE, -- disabled by frequency

                position INTEGER DEFAULT NULL, -- manual order; contiguous 1..N per plan, NULL sorts last

                FOREIGN KEY(plan_id)
                    REFERENCES plan(id)
                    ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS "group" (
                id INTEGER PRIMARY KEY,
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
                imported_support TEXT, -- read-only support from mapped Anki fields (Anki HTML)
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
                group_id INTEGER, -- for the purpose of collecting data and sorting on stats page, not persistence
                plan_id INTEGER NOT NULL, -- no FK: value persists after plan deletion so stats remain browsable
                plan_name TEXT NOT NULL DEFAULT '', -- preserved for display after plan deletion; synced on rename

                group_name TEXT NOT NULL,
                date DATE NOT NULL,

                num_promote INTEGER NOT NULL DEFAULT 0, -- review card increasing in tier
                num_demote INTEGER NOT NULL DEFAULT 0, -- review card decreasing in tier (or tier 0 -> tier 0)
                num_new INTEGER NOT NULL DEFAULT 0, -- new card studied
                time_spent_minutes FLOAT NOT NULL DEFAULT 0,
                retention_rate REAL NOT NULL DEFAULT 0,

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
                "INSERT INTO card (id, group_id, front, back, imported_support, front_image, back_image, front_audio, back_audio, is_uploaded)
                 VALUES
                 (1, 1, 'plain front mentioning /home/alice/x.png', 'back', NULL,
                  '{cur}/cards/images/a.png', '{stale}/cards/images/b.png',
                  '{cur}/cards/audio/c.mp3', NULL, FALSE),
                 (2, 1, '<img src=\"{cur}/cards/images/d.png\">', '<audio controls src=\"{stale}/cards/audio/e.mp3\"></audio>',
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
                "SELECT front, back, imported_support FROM card WHERE id = 2",
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
