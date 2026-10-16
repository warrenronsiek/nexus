// @feature persistence
// @spec docs/features/persistence.md
// @entrypoint main
use anyhow::{Context, Result};
use clap::Parser;
use nexus::persistence::{backup_sqlite_database, export_sqlite_stream};
use std::io::{self, BufWriter};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "nexus-sqlite-export",
    version,
    about = "Internal portable snapshot helper for nexus-server"
)]
struct Args {
    /// Existing Nexus SQLite database to snapshot without modifying it.
    #[arg(long)]
    database: PathBuf,
    /// Atomically stage an online backup at this path instead of emitting JSON.
    #[arg(long)]
    copy_to: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(destination) = args.copy_to {
        return backup_sqlite_database(&args.database, &destination);
    }
    let stdout = io::stdout();
    export_sqlite_stream(&args.database, BufWriter::new(stdout.lock()))
        .context("write portable SQLite stream")?;
    Ok(())
}
