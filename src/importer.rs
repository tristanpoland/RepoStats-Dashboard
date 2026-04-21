use anyhow::{Context, Result};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tracing::{info, warn};
use walkdir::WalkDir;

/// Walk `dir`, find *.sql files, skip already-imported ones (by sha256),
/// execute each file's SQL against `conn`, then record it as imported.
pub fn import_dir(conn: &Connection, dir: &Path) -> Result<usize> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Cannot create import dir {}", dir.display()))?;
        info!("Created import directory: {}", dir.display());
        return Ok(0);
    }

    let mut count = 0;

    let mut entries: Vec<_> = WalkDir::new(dir)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|x| x == "sql")
                    .unwrap_or(false)
        })
        .collect();

    // Sort by filename so snapshots import in chronological order
    entries.sort_by_key(|e| e.file_name().to_owned());

    for entry in entries {
        let path = entry.path();
        let filename = path.file_name().unwrap().to_string_lossy().to_string();

        let raw = fs::read(path)
            .with_context(|| format!("Cannot read {}", path.display()))?;

        let hash = hex::encode(Sha256::digest(&raw));

        // Check if already imported by hash (handles renamed files correctly too)
        let already: bool = conn
            .query_row(
                "SELECT 1 FROM _imported_files WHERE sha256 = ?1",
                rusqlite::params![hash],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if already {
            info!("  skip (already imported): {}", filename);
            continue;
        }

        let sql = String::from_utf8_lossy(&raw);

        info!("  importing: {}", filename);

        // Execute the entire SQL file. We strip PRAGMA lines that rusqlite
        // doesn't like in execute_batch (e.g. journal_mode response rows).
        let filtered = sql
            .lines()
            .filter(|l| {
                let lt = l.trim().to_lowercase();
                !lt.starts_with("pragma journal_mode")
                    && !lt.starts_with("pragma foreign_keys")
            })
            .collect::<Vec<_>>()
            .join("\n");

        conn.execute_batch(&filtered)
            .with_context(|| format!("SQL error in {}", filename))?;

        conn.execute(
            "INSERT OR IGNORE INTO _imported_files (filename, sha256) VALUES (?1, ?2)",
            rusqlite::params![filename, hash],
        )?;

        count += 1;
        info!("  ✓ imported: {}", filename);
    }

    Ok(count)
}
