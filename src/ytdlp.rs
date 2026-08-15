use crate::config::AudioQuality;
use anyhow::{bail, Context, Result};
use regex::Regex;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{sleep, Duration};
use tracing::warn;

/// How many trailing stderr lines to keep as the reason for a failed download.
/// yt-dlp is chatty; the tail is where the actual `ERROR:` line lands.
const STDERR_TAIL_LINES: usize = 10;

/// Delay before each retry of a failed download. A quick retry first, in case
/// the failure (commonly an HTTP 403 from YouTube's anti-bot throttling) was a
/// one-off blip; a longer wait for the second, in case it's a short-lived block.
/// Three attempts total, ~75s worst case before giving up.
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(15), Duration::from_secs(60)];

static PROGRESS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[download\]\s+([\d.]+)%").unwrap());

/// yt-dlp announces the pre-conversion download as `[download] Destination:` and
/// the extracted audio as `[ExtractAudio] Destination:`. Both are matched so the
/// later line wins and `dest` ends up holding the file that actually survives.
static DEST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(?:download|ExtractAudio)\] Destination: (.+)").unwrap());

#[derive(Debug)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub channel: String,
    pub duration: u64,
    pub video_id: String,
    pub source: String,
}

/// Run `yt-dlp -j --no-playlist <url>` and parse the JSON into TrackMeta.
/// All fields except video_id and source are optional in yt-dlp output.
pub async fn fetch_metadata(url: &str) -> Result<TrackMeta> {
    let output = Command::new("yt-dlp")
        .args(["-j", "--no-playlist", url])
        // Cancelling this future (app quits mid-fetch) must not leave yt-dlp
        // running detached — tokio detaches rather than kills by default.
        .kill_on_drop(true)
        .output()
        .await
        .context("failed to spawn yt-dlp")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("yt-dlp failed: {stderr}");
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse yt-dlp JSON")?;

    let video_id = json["id"]
        .as_str()
        .context("yt-dlp JSON missing required 'id' field")?
        .to_string();

    let webpage_url = json["webpage_url"].as_str().unwrap_or(url);
    let source = extract_domain(webpage_url);

    let title = json["title"]
        .as_str()
        .unwrap_or("Unknown title")
        .to_string();

    let artist = json["artist"]
        .as_str()
        .or_else(|| json["uploader"].as_str())
        .unwrap_or("Unknown artist")
        .to_string();

    let channel = json["channel"]
        .as_str()
        .or_else(|| json["uploader"].as_str())
        .unwrap_or(&artist)
        .to_string();

    let duration = json["duration"].as_f64().unwrap_or(0.0) as u64;

    Ok(TrackMeta {
        title,
        artist,
        channel,
        duration,
        video_id,
        source,
    })
}

/// Run `yt-dlp -f <quality> --get-url --no-playlist <url>` and return the direct stream URL.
pub async fn get_stream_url(url: &str, quality: &AudioQuality) -> Result<String> {
    let output = Command::new("yt-dlp")
        .args([
            "-f",
            quality.to_format_str(),
            "--get-url",
            "--no-playlist",
            url,
        ])
        .kill_on_drop(true)
        .output()
        .await
        .context("failed to spawn yt-dlp")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("yt-dlp --get-url failed: {stderr}");
    }

    let stdout = String::from_utf8(output.stdout).context("yt-dlp returned non-UTF8 URL")?;

    // First line only. A format selector that resolves to separate audio and
    // video streams makes yt-dlp print one URL per line; handing the whole blob
    // to mpv gave it a "URL" with an embedded newline that it could not open.
    let stream_url = first_url_line(&stdout).context("yt-dlp returned an empty stream URL")?;

    Ok(stream_url)
}

/// Retry `attempt` up to `delays.len() + 1` times, sleeping `delays[i]` between
/// the `i`th failure and the next try. Returns the last error if every attempt
/// fails. Generic over the attempt itself so the backoff logic is testable
/// without shelling out to yt-dlp.
pub(crate) async fn retry_with_backoff<F, Fut, T>(delays: &[Duration], mut attempt: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut i = 0;
    loop {
        match attempt().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if i >= delays.len() {
                    return Err(e);
                }
                warn!(attempt = i + 1, err = %e, "attempt failed, retrying");
                sleep(delays[i]).await;
                i += 1;
            }
        }
    }
}

/// `spawn_download`, retried on failure per `RETRY_DELAYS`. This is what the
/// caller should use for a download that should recover from a transient
/// failure (an HTTP 403 that goes away on its own) on its own, rather than
/// leaving the track stuck at `streaming` after the first hiccup.
pub async fn download_with_retries(
    url: &str,
    audio_dir: &Path,
    video_id: &str,
    quality: &AudioQuality,
    progress_tx: watch::Sender<(String, f32)>,
) -> Result<PathBuf> {
    retry_with_backoff(&RETRY_DELAYS, || {
        spawn_download(url, audio_dir, video_id, quality, progress_tx.clone())
    })
    .await
}

/// Spawn a yt-dlp download, reporting progress via watch channel.
/// Uses `video_id.%(ext)s` as the output template so yt-dlp picks the extension.
/// Returns the actual downloaded file path when complete.
///
/// A failed download leaves nothing behind: yt-dlp keeps its half-written data in
/// `<video_id>.<ext>.part` and its resume state in `.ytdl` companions, and those
/// used to accumulate in the audio cache with no row referring to them — one
/// abandoned copy per retry of the same track.
pub async fn spawn_download(
    url: &str,
    audio_dir: &Path,
    video_id: &str,
    quality: &AudioQuality,
    progress_tx: watch::Sender<(String, f32)>,
) -> Result<PathBuf> {
    let result = run_download(url, audio_dir, video_id, quality, progress_tx).await;
    if result.is_err() {
        clean_partial_downloads(audio_dir, video_id);
    }
    result
}

async fn run_download(
    url: &str,
    audio_dir: &Path,
    video_id: &str,
    quality: &AudioQuality,
    progress_tx: watch::Sender<(String, f32)>,
) -> Result<PathBuf> {
    let template = audio_dir
        .join(format!("{video_id}.%(ext)s"))
        .to_str()
        .context("audio dir path is not valid UTF-8")?
        .to_string();

    let mut child = Command::new("yt-dlp")
        .args([
            "-f",
            quality.to_format_str(),
            "-x",
            "--audio-format",
            "opus",
            "--no-playlist",
            // Progress is rewritten in place with `\r` by default, so `lines()`
            // yields nothing at all until the download is over. One line per
            // update is what lets the progress bar actually move.
            "--newline",
            "-o",
            &template,
            url,
        ])
        // Progress goes to *stdout*. This used to be `Stdio::null()` with the
        // parser reading stderr, where yt-dlp writes no progress whatsoever —
        // which is why the caching bar never advanced.
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // A download that outlives the app would keep writing into the audio
        // cache with nothing left to record the result in the playlist.
        .kill_on_drop(true)
        .spawn()
        .context("failed to spawn yt-dlp download")?;

    // Drain stderr on its own task rather than after the fact: it is where
    // errors land, and an unread pipe stops yt-dlp dead once the kernel buffer
    // fills.
    let stderr = child.stderr.take().expect("stderr was piped");
    let stderr_tail = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail: Vec<String> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if tail.len() == STDERR_TAIL_LINES {
                tail.remove(0);
            }
            tail.push(line);
        }
        tail
    });

    let stdout = child.stdout.take().expect("stdout was piped");
    let mut lines = BufReader::new(stdout).lines();
    let mut dest: Option<PathBuf> = None;

    while let Some(line) = lines.next_line().await? {
        if let Some(path) = parse_destination_line(&line) {
            dest = Some(path);
        }
        if let Some(pct) = parse_progress_line(&line) {
            let _ = progress_tx.send((video_id.to_string(), pct));
        }
    }

    let status = child
        .wait()
        .await
        .context("yt-dlp download process failed")?;
    let reason = stderr_tail.await.unwrap_or_default().join(" | ");
    if !status.success() {
        bail!("yt-dlp download exited with status {status}: {reason}");
    }

    let _ = progress_tx.send((video_id.to_string(), 100.0));

    // Use the destination logged by yt-dlp, or scan the directory as fallback
    let file = dest
        .filter(|p| p.exists())
        .or_else(|| find_downloaded_file(audio_dir, video_id))
        .with_context(|| format!("could not find downloaded file for {video_id}"))?;

    Ok(file)
}

/// Delete the half-finished files a failed download left in the audio cache.
///
/// Only yt-dlp's own scratch files are touched. A finished `<video_id>.opus` is
/// deliberately left alone: a download is started even when the track is already
/// cached from another playlist, so deleting every `<video_id>.*` on failure would
/// destroy a good file that other playlists still play.
pub(crate) fn clean_partial_downloads(audio_dir: &Path, video_id: &str) {
    let Ok(entries) = std::fs::read_dir(audio_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_partial_artifact(name, video_id) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// True for the scratch files yt-dlp writes while downloading `video_id`:
/// `<id>.<ext>.part`, its `.part-Frag<n>` fragments, `<id>.<ext>.ytdl` resume
/// state, and `.temp` intermediates.
pub(crate) fn is_partial_artifact(name: &str, video_id: &str) -> bool {
    let Some(rest) = name.strip_prefix(video_id) else {
        return false;
    };
    if !rest.starts_with('.') {
        return false; // a different id that merely starts with ours
    }
    rest.ends_with(".part")
        || rest.ends_with(".ytdl")
        || rest.ends_with(".temp")
        || rest.contains(".part-")
}

/// The first non-blank line of `--get-url` output, trimmed.
pub(crate) fn first_url_line(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Parse a download-progress percentage out of a single yt-dlp output line.
pub(crate) fn parse_progress_line(line: &str) -> Option<f32> {
    PROGRESS_RE.captures(line)?[1].parse::<f32>().ok()
}

/// Parse a `Destination:` path out of a single yt-dlp output line.
pub(crate) fn parse_destination_line(line: &str) -> Option<PathBuf> {
    Some(PathBuf::from(DEST_RE.captures(line)?[1].trim()))
}

/// Fallback for when yt-dlp logged no usable `Destination:` line: find
/// `<video_id>.<ext>` in the audio dir.
///
/// `.opus` wins when several extensions share the stem. `-x --audio-format opus`
/// always converts to it, and a leftover pre-conversion file would otherwise be
/// picked at the mercy of `read_dir` ordering — recording a path that the
/// converter is about to delete.
pub(crate) fn find_downloaded_file(audio_dir: &Path, video_id: &str) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = std::fs::read_dir(audio_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) == Some(video_id))
        .collect();

    candidates
        .iter()
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("opus"))
        .or_else(|| candidates.first())
        .cloned()
}

/// Extract the bare domain from a URL (e.g. "youtube.com" from "https://www.youtube.com/...").
fn extract_domain(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}
