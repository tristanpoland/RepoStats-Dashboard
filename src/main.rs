use anyhow::{Context, Result};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use rusqlite::Connection;
use tracing::info;

mod api;
mod db;
mod importer;

#[derive(Parser, Debug)]
#[command(name = "github-archive", about = "GitHub repo stats archiver & dashboard")]
struct Args {
    /// Directory containing .sql export files to import
    #[arg(short, long, default_value = "./exports")]
    import_dir: PathBuf,

    /// SQLite database file path
    #[arg(short, long, default_value = "./github_archive.db")]
    database: PathBuf,

    /// Port to serve the web UI on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Only import files, don't start web server
    #[arg(long)]
    import_only: bool,
}

pub struct AppState {
    pub db: Mutex<Connection>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    let args = Args::parse();

    let conn = db::open(&args.database).context("Failed to open database")?;
    info!("Database: {}", args.database.display());

    let imported = importer::import_dir(&conn, &args.import_dir)
        .context("Import failed")?;
    if imported == 0 {
        info!("No new SQL files to import.");
    } else {
        info!("Imported {} new file(s).", imported);
    }

    if args.import_only {
        return Ok(());
    }

    let state = Arc::new(AppState { db: Mutex::new(conn) });
    let app = api::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    info!("Dashboard → http://localhost:{}", args.port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
