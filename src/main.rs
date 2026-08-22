mod cache;
mod config;
#[cfg(test)]
mod config_test;
mod deps;
mod library;
mod library_import;
#[cfg(test)]
mod library_import_test;
mod library_scan;
#[cfg(test)]
mod library_scan_test;
#[cfg(test)]
mod library_test;
mod player;
#[cfg(test)]
mod player_test;
mod playlist;
#[cfg(test)]
mod playlist_test;
mod tui;
mod ytdlp;

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "trovers", about = "Personal audio cache and player")]
struct Cli {
    /// URL of a track to add to the opened playlist. Adding never starts
    /// playback — press Enter on the row to play it.
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

    // Before any playlist is read: playlists written by an older trovers embed
    // their tracks, and everything downstream expects id lists. Detection is by
    // shape, so this is a no-op on every launch after the first.
    let migration = library::migrate(&cache::playlists_dir(), &cache::tracks_dir())
        .context("failed to migrate playlists to the track library")?;

    // Clean up after any previous instance that was killed hard enough to skip
    // its own teardown, so a stranded mpv is not still playing over this one.
    player::reap_orphaned_players().await;

    let config = config::Config::load()?;

    // Determine which playlist to open
    let cli_playlist = cli.playlist.clone();
    let config_playlist = config.active_playlist.clone();
    let playlist_name = cli_playlist.clone().or_else(|| config_playlist.clone());

    let (playlist, playlist_path) = match playlist_name {
        Some(name) => {
            let path = cache::playlists_dir().join(format!("{name}.toml"));
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

    let available_playlists =
        playlist::Playlist::list_entries(&cache::playlists_dir()).unwrap_or_default();

    // The one place the library's location is decided; `Library` itself only ever
    // works against the root it is handed, which is what makes it testable.
    let library = library::Library::load(&cache::tracks_dir())?;
    info!(tracks = library.len(), "loaded track library");

    let mut app = tui::App::new(
        playlist,
        config,
        available_playlists,
        playlist_path,
        library,
    );

    // Tell the user a migration happened, and where the backup went — the only
    // moment they get to notice before the status line moves on.
    if let Some(report) = migration {
        app.set_status(format!(
            "Migrated {} playlist(s), {} track(s) · backup: {}",
            report.playlists,
            report.tracks,
            report.backup.display()
        ));
    }

    // If a URL was provided on CLI, queue it for fetch immediately
    if let Some(url) = cli.url {
        app.fetch_url(url);
    }

    let run_result = tui::run(&mut app).await;

    // Save state on exit.
    //
    // IMPORTANT: use app.playlist_path here because it can change at runtime
    // (e.g. when renaming the active playlist). Saving to the original startup
    // path can recreate a deleted playlist file with copied contents.
    //
    // This runs whether or not the event loop returned an error. It used to sit
    // behind a `?` on `run`, so any error bubbling out of the loop threw away
    // every playlist edit made during the session — the invisible half of the
    // "it just crashed" symptom.
    if let Err(e) = app.playlist.save(&app.playlist_path) {
        error!(err = %e, "failed to save playlist on exit");
    }
    if let Err(e) = app.config.save() {
        error!(err = %e, "failed to save config on exit");
    }

    run_result?;
    info!("exiting cleanly");

    Ok(())
}
