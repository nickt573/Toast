use rusqlite::Connection;

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

/// Migrations for databases created by older releases. Each call is idempotent.
fn migrate_schema(conn: &Connection) -> rusqlite::Result<()> {
    // v1.1.0: read-only support content mapped from Anki fields on import,
    // kept separate from front/back so it stays out of similar-card matching.
    add_column_if_missing(conn, "card", "imported_support", "TEXT")?;
    // v1.2.0: optional manual order for todos; numbered todos sort ahead of
    // unnumbered ones and stay contiguous 1..N per plan (see set_todo_position).
    add_column_if_missing(conn, "todo", "position", "INTEGER DEFAULT NULL")?;
    // v1.2.0: todo time is whole minutes now; round decimals logged by older
    // releases (idempotent, the column itself stays FLOAT).
    conn.execute_batch("UPDATE todo_stats SET time_spent_minutes = ROUND(time_spent_minutes);")?;
    Ok(())
}

/// Creates all tables (idempotent) and enables foreign keys.
pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
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

    migrate_schema(conn)
}
