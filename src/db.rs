use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use tracing::info;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    
    // Performance pragmas
    conn.execute_batch("
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA cache_size = -32000;
    ")?;

    // Create the file-tracking table so we never double-import
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS _imported_files (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            filename    TEXT    NOT NULL UNIQUE,
            sha256      TEXT    NOT NULL,
            imported_at TEXT    NOT NULL DEFAULT (datetime('now'))
        );
    ")?;

    info!("Database ready at {}", path.display());
    Ok(conn)
}
