mod cache;
mod config;
mod deps;
mod player;
mod playlist;
mod tui;
mod ytdlp;

use anyhow::{Context, Result};
use clap::Parser;
use std::io::Write;
use tracing::info;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

// #region agent log
fn agent_log(
    run_id: &str,
    hypothesis_id: &str,
    location: &str,
    message: &str,
    data: serde_json::Value,
) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let payload = serde_json::json!({
        "sessionId": "d28f88",
        "runId": run_id,
        "hypothesisId": hypothesis_id,
        "location": location,
        "message": message,
        "data": data,
        "timestamp": ts
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/Users/den/Projects/trovers/.cursor/debug-d28f88.log")
    {
        let _ = writeln!(f, "{}", payload);
    }
}
// #endregion agent log

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
    let cli_playlist_for_log = cli.playlist.clone();

    deps::check()?;
    cache::ensure_dirs()?;

    let config = config::Config::load()?;

    // Determine which playlist to open
    let cli_playlist = cli.playlist.clone();
    let config_playlist = config.active_playlist.clone();
    let playlist_name = cli_playlist.clone().or_else(|| config_playlist.clone());

    // #region agent log
    agent_log(
        "pre",
        "A",
        "src/main.rs:playlist_select",
        "startup playlist selection inputs",
        serde_json::json!({
            "cli_playlist": cli_playlist_for_log,
            "cli_url_present": cli.url.is_some(),
            "config_active_playlist": config.active_playlist,
            "resolved_playlist_name": playlist_name,
            "playlists_dir": crate::cache::playlists_dir().display().to_string(),
        }),
    );
    // #endregion agent log

    let (playlist, playlist_path) = match playlist_name {
        Some(name) => {
            let path = cache::playlists_dir().join(format!("{name}.toml"));
            // #region agent log
            agent_log(
                "pre",
                "D",
                "src/main.rs:open_named_playlist",
                "opening named playlist (exists?)",
                serde_json::json!({
                    "name": name,
                    "path": path.display().to_string(),
                    "exists": path.exists(),
                }),
            );
            // #endregion agent log
            if path.exists() {
                let pl = playlist::Playlist::load(&path)
                    .with_context(|| format!("failed to load playlist '{name}'"))?;
                (pl, path)
            } else {
                // If the name came from CLI, creating a new playlist is expected.
                // If it came from config (active_playlist) but the file is missing, we should NOT
                // create a new empty playlist file when other playlists exist; instead, fallback.
                let from_cli = cli_playlist.as_deref() == Some(&name);
                if from_cli {
                    playlist::Playlist::create(&name)
                        .with_context(|| format!("failed to create playlist '{name}'"))?
                } else {
                    let existing = playlist::Playlist::list_all()?;
                    if let Some(first) = existing.into_iter().next() {
                        let pl = playlist::Playlist::load(&first)?;
                        (pl, first)
                    } else {
                        playlist::Playlist::create("Default")?
                    }
                }
            }
        }
        None => {
            let existing = playlist::Playlist::list_all()?;
            // #region agent log
            agent_log(
                "pre",
                "D",
                "src/main.rs:open_fallback_playlist",
                "no active playlist set; fallback to first existing or Default",
                serde_json::json!({
                    "existing_count": existing.len(),
                    "existing_paths": existing.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                }),
            );
            // #endregion agent log
            if let Some(path) = existing.into_iter().next() {
                let pl = playlist::Playlist::load(&path)?;
                (pl, path)
            } else {
                playlist::Playlist::create("Default")?
            }
        }
    };

    info!(playlist = %playlist.name, "opened playlist");

    // Keep config.active_playlist in sync with whatever actually got opened
    // (covers first launch, a since-renamed/deleted playlist, or CLI override),
    // saving immediately so a crash before exit still leaves the config accurate.
    let mut config = config;
    if config.active_playlist.as_deref() != Some(playlist.name.as_str()) {
        config.active_playlist = Some(playlist.name.clone());
        let _ = config.save();
    }

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
    // #region agent log
    agent_log(
        "pre",
        "B",
        "src/main.rs:exit_save",
        "saving state on exit",
        serde_json::json!({
            "playlist_name_in_app": app.playlist.name,
            "playlist_path_in_app": app.playlist_path.display().to_string(),
            "playlist_path_in_main_var": playlist_path.display().to_string(),
            "same_path": app.playlist_path == playlist_path,
            "config_active_playlist_in_app": app.config.active_playlist,
        }),
    );
    // #endregion agent log
    // IMPORTANT: use app.playlist_path here because it can change at runtime
    // (e.g. when renaming the active playlist). Saving to the original startup
    // path can recreate a deleted playlist file with copied contents.
    app.playlist.save(&app.playlist_path)?;
    app.config.save()?;
    info!("exiting cleanly");

    Ok(())
}
