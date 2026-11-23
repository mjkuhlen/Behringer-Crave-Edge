mod api;
mod crave;
mod clock;
mod library;
mod midi;
mod modulation;
mod patch;
mod player;
mod sim;
mod song;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "craved", about = "Behringer Crave companion daemon (Linux/Docker)")]
struct Args {
    /// HTTP port for API + web UI
    #[arg(long, default_value_t = 8090)]
    port: u16,
    /// Data directory (pattern library, settings) — mount a volume here
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,
    /// Static web UI directory
    #[arg(long, default_value = "./web")]
    web_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    std::fs::create_dir_all(&args.data_dir)?;

    let (events, _) = broadcast::channel::<midi::Event>(1024);
    let enumerator = Arc::new(midi::MidiManager::new(events.clone()));
    let library = Arc::new(library::Library::new(args.data_dir.join("patterns"))?);
    let songs = Arc::new(song::SongStore::new(args.data_dir.join("songs"))?);
    let patches = Arc::new(patch::PatchStore::new(args.data_dir.join("patches"))?);

    let state = api::AppState {
        contexts: Arc::new(Mutex::new(std::collections::HashMap::new())),
        selected: Arc::new(Mutex::new(None)),
        enumerator,
        library,
        songs,
        patches,
        events,
        data_dir: args.data_dir.clone(),
    };

    let app = api::router(state, &args.web_dir);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", args.port)).await?;
    tracing::info!(
        "craved v{} listening on http://0.0.0.0:{} (data: {:?}, web: {:?})",
        env!("CARGO_PKG_VERSION"),
        args.port,
        args.data_dir,
        args.web_dir
    );
    axum::serve(listener, app).await?;
    Ok(())
}
