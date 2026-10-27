use rusqlite::Connection;

// Historical projections of the tables exercised by the CLI. These deliberately
// do not use production profile requirements. Sources and omissions are recorded
// in docs/schema-support.md; direct tests use real upstream initialization.
pub fn create_schema(db: &Connection, number: i32) {
    db.execute_batch(
        "CREATE TABLE providers (
            id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
            settings_config TEXT NOT NULL DEFAULT '{}', website_url TEXT, category TEXT,
            created_at INTEGER, sort_index INTEGER, notes TEXT, icon TEXT, icon_color TEXT,
            meta TEXT NOT NULL DEFAULT '{}', is_current BOOLEAN NOT NULL DEFAULT 0,
            PRIMARY KEY (id, app_type));
         CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, server_config TEXT NOT NULL,
            enabled_claude BOOLEAN DEFAULT 0, enabled_codex BOOLEAN DEFAULT 0,
            enabled_gemini BOOLEAN DEFAULT 0);
         CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
         INSERT INTO settings VALUES ('unrelated-sentinel', 'preserve-me');",
    )
    .unwrap();

    if number == 1 {
        db.execute_batch("CREATE TABLE skills (key TEXT PRIMARY KEY, installed BOOLEAN DEFAULT 0, installed_at INTEGER DEFAULT 0);").unwrap();
    } else if number == 2 {
        db.execute_batch("CREATE TABLE skills (directory TEXT, app_type TEXT, installed BOOLEAN DEFAULT 0, installed_at INTEGER DEFAULT 0, PRIMARY KEY(directory, app_type));").unwrap();
    } else {
        db.execute_batch("CREATE TABLE skills (id TEXT PRIMARY KEY, name TEXT NOT NULL, directory TEXT NOT NULL, enabled_claude BOOLEAN DEFAULT 0, enabled_codex BOOLEAN DEFAULT 0, enabled_gemini BOOLEAN DEFAULT 0, installed_at INTEGER DEFAULT 0);").unwrap();
    }
    if number >= 2 {
        db.execute_batch(
            "ALTER TABLE providers ADD COLUMN in_failover_queue BOOLEAN NOT NULL DEFAULT 0;",
        )
        .unwrap();
    }
    for (since, column) in [
        (4, "enabled_opencode"),
        (10, "enabled_hermes"),
        (15, "enabled_grokbuild"),
        (19, "enabled_mcode"),
        (21, "enabled_agy"),
        (22, "enabled_kimi"),
    ] {
        if number >= since {
            for table in ["mcp_servers", "skills"] {
                db.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} BOOLEAN NOT NULL DEFAULT 0;"
                ))
                .unwrap();
            }
        }
    }
    if number >= 7 {
        db.execute_batch(
            "ALTER TABLE skills ADD COLUMN content_hash TEXT;
            ALTER TABLE skills ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;",
        )
        .unwrap();
    }
    if number >= 8 {
        db.execute_batch(
            "CREATE TABLE session_log_sync (
            file_path TEXT PRIMARY KEY, last_modified INTEGER NOT NULL,
            last_line_offset INTEGER NOT NULL DEFAULT 0, last_synced_at INTEGER NOT NULL);",
        )
        .unwrap();
    }
    if number >= 17 {
        db.execute_batch(
            "CREATE TABLE session_usage_dedup (
            data_source TEXT NOT NULL, request_id TEXT NOT NULL, semantic_id TEXT NOT NULL,
            has_entry_id INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(data_source, request_id));",
        )
        .unwrap();
    }
    if number >= 18 {
        db.execute_batch(
            "ALTER TABLE session_log_sync ADD COLUMN last_byte_offset INTEGER;
            ALTER TABLE session_log_sync ADD COLUMN last_tail_fingerprint INTEGER;",
        )
        .unwrap();
    }
    if number >= 20 {
        db.execute_batch(
            "ALTER TABLE mcp_servers ADD COLUMN enabled_pi BOOLEAN NOT NULL DEFAULT 0;",
        )
        .unwrap();
    }
    if number >= 23 {
        db.execute_batch(
            "ALTER TABLE mcp_servers ADD COLUMN enabled_omp BOOLEAN NOT NULL DEFAULT 0;",
        )
        .unwrap();
    }
    db.pragma_update(None, "user_version", number).unwrap();
}
