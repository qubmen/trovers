mod cache;
mod config;
mod deps;
mod player;
mod playlist;
mod tui;
mod ytdlp;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "trovers", about = "Personal audio cache and player")]
struct Cli {
    /// URL of the track to add and play immediately
    url: Option<String>,

    /// Open a specific playlist by name
    #[arg(long, short)]
    playlist: Option<String>,
}

fn init_logging() -> WorkerGuard {
    let log_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("trovers");
    let file_appender = tracing_appender::rolling::never(&log_path, "trovers.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(EnvFilter::new("trovers=debug"))
        .with_ansi(false)
        .init();
    guard
}

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = init_logging();

    let cli = Cli::parse();

    deps::check()?;
    cache::ensure_dirs()?;

    let config = config::Config::load()?;

    // Determine which playlist to open
    let playlist_name = cli
        .playlist
        .or_else(|| config.active_playlist.clone());

    let (playlist, playlist_path) = match playlist_name {
        Some(name) => {
            let path = cache::playlists_dir().join(format!("{name}.toml"));
            if path.exists() {
                let pl = playlist::Playlist::load(&path)
                    .with_context(|| format!("failed to load playlist '{name}'"))?;
                (pl, path)
            } else {
                playlist::Playlist::create(&name)
                    .with_context(|| format!("failed to create playlist '{name}'"))?
            }
        }
        None => {
            let existing = playlist::Playlist::list_all()?;
            if let Some(path) = existing.into_iter().next() {
                let pl = playlist::Playlist::load(&path)?;
                (pl, path)
            } else {
                playlist::Playlist::create("Default")?
            }
        }
    };

    info!(playlist = %playlist.name, "opened playlist");

    let available_playlists = playlist::Playlist::list_all()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|name| (name.to_string(), p.clone()))
        })
        .collect();

    let mut app = tui::App::new(playlist, config, available_playlists, playlist_path.clone());

    // If a URL was provided on CLI, queue it for fetch immediately
    if let Some(url) = cli.url {
        app.fetch_url(url);
    }

    tui::run(&mut app).await?;

    // Save state on clean exit
    app.playlist.save(&playlist_path)?;
    app.config.save()?;
    info!("exiting cleanly");

    Ok(())
}
