//! Turning a folder of the user's own media files into library tracks.
//!
//! Three jobs, deliberately kept apart so each is testable on its own: walk a
//! directory (`scan`), work out what a file is (`probe`, and the pure
//! `parse_ffprobe`/`resolve_meta` it is built from), and mint a stable id for a
//! path (`local_id`).
//!
//! ffprobe is a **soft** dependency. `deps.rs` checks for yt-dlp and mpv only,
//! because those are load-bearing; without ffprobe an import still works, with
//! titles taken from filenames and durations left at zero.

// TEMPORARY: nothing outside the tests calls into here until the import and
// rescan keys are wired up. Remove this along with the `F`/`R` handlers, at
// which point every item below has a caller.
#![allow(dead_code)]

use crate::library::{make_id, MediaKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::process::Command;
use tracing::{info, warn};

/// How deep a scan will descend. A bound, not a limit anyone should reach: a
/// media folder nested seventeen levels down is a mistake, and walking forever
/// is worse than missing it.
pub const MAX_DEPTH: usize = 16;

/// Artist of last resort, matching what `ytdlp.rs` falls back to so local and
/// remote rows read the same.
const UNKNOWN_ARTIST: &str = "Unknown artist";
const UNKNOWN_TITLE: &str = "Unknown title";

/// Extensions taken as audio. Not exhaustive — mpv plays more than this — but
/// every one here is unambiguous, and ffprobe corrects the guess when present.
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "opus", "ogg", "oga", "m4a", "m4b", "aac", "wav", "aiff", "aif", "alac", "wma",
    "mka", "ape", "wv",
];

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "m4v", "mpg", "mpeg", "wmv", "flv", "ts", "m2ts", "ogv",
];

/// Codecs that describe a still image rather than moving pictures. Embedded
/// cover art is a "video stream" to ffprobe, and treating it as one would open a
/// video window for an mp3.
const STILL_IMAGE_CODECS: &[&str] = &["mjpeg", "jpeg", "png", "gif", "bmp", "webp", "tiff", "ppm"];

/// Whether spawning ffprobe has already failed once. It is a soft dependency, so
/// a machine without it must not produce one warning per file — the first
/// failure is logged and the rest are skipped.
static FFPROBE_MISSING: AtomicBool = AtomicBool::new(false);

/// A media file found by `scan`, with the kind its extension implies.
pub struct ScannedFile {
    pub path: PathBuf,
    /// A guess from the extension. `probe` overrules it when ffprobe is around.
    pub media: MediaKind,
}

/// Artist and title as read out of a filename.
pub struct ParsedName {
    pub artist: Option<String>,
    pub title: String,
}

/// What ffprobe had to say. Every field optional because tags routinely are —
/// `resolve_meta` decides what to do with the gaps.
pub struct FfprobeMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub duration: Option<u64>,
    /// A moving-picture stream, cover art excluded.
    pub has_video: bool,
}

/// Everything needed to build a `Track` for a local file.
pub struct ProbedMeta {
    pub title: String,
    pub artist: String,
    pub duration: u64,
    pub media: MediaKind,
}

/// Every media file under `root`, sorted by path.
///
/// Sorted because the order becomes the album's running order, and a rescan must
/// not reshuffle what the user is looking at. A directory that cannot be read is
/// skipped rather than fatal: one unreadable subfolder must not lose the import.
pub fn scan(root: &Path) -> Vec<ScannedFile> {
    let mut found = Vec::new();
    // (directory, its depth below `root`)
    let mut pending = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!(err = %e, path = %dir.display(), "cannot read directory, skipping it");
                continue;
            }
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Nothing the user meant to import starts with a dot: `.DS_Store`,
            // `.hidden`, and macOS's `._name.ext` resource forks, which do carry
            // media extensions and are not media.
            if name.starts_with('.') {
                continue;
            }

            let path = entry.path();
            // `file_type` does not follow symlinks, so this asks what the entry
            // *is*, then `is_dir` asks what it points at.
            let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            if path.is_dir() {
                // A directory symlink can point at an ancestor, which makes the
                // tree infinite. Not following any of them is the total answer,
                // and costs only the unusual case of a deliberately linked
                // subfolder. Symlinked *files* are still imported.
                if is_symlink {
                    continue;
                }
                if depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }

            if let Some(media) = media_kind_for_path(&path) {
                found.push(ScannedFile { path, media });
            }
        }
    }

    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

/// What a path's extension says it is, or `None` when it says nothing — which is
/// how `scan` filters out artwork, cue sheets and stray text files.
pub fn media_kind_for_path(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Audio)
    } else if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// The library id for a local file: `local:<fnv1a-of-the-path>`.
///
/// Derived from the path so a rescan lands on the same id and reuses the same
/// document — which is what makes `last_position`, `speed` and a renamed title
/// survive being rescanned. Normalisation is lexical only, because the file may
/// well be on an unplugged drive and `canonicalize` needs it to exist.
pub fn local_id(path: &Path) -> String {
    let normalized: PathBuf = path.components().collect();
    let hash = fnv1a(normalized.to_string_lossy().as_bytes());
    make_id("local", &format!("{hash:016x}"))
}

/// FNV-1a, 64-bit. Hand-rolled for the same reason as the xorshift in
/// `playlist.rs`: naming a file is not security-sensitive and this is shorter
/// than a dependency.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Read artist and title out of a filename stem.
///
/// `Artist - Title` is the one convention worth honouring; everything else is
/// left as a title, because guessing wrong is worse than not guessing.
pub fn parse_filename(stem: &str) -> ParsedName {
    let stem = stem.trim();

    if let Some((left, right)) = stem.split_once(" - ") {
        let (left, right) = (left.trim(), right.trim());
        if !right.is_empty() {
            // `01 - Intro` is a track number, not an artist called "01".
            if is_track_number(left) {
                return ParsedName {
                    artist: None,
                    title: right.to_string(),
                };
            }
            if !left.is_empty() {
                return ParsedName {
                    artist: Some(left.to_string()),
                    title: right.to_string(),
                };
            }
        }
    }

    let title = strip_track_number_prefix(stem);
    ParsedName {
        artist: None,
        title: if title.is_empty() {
            UNKNOWN_TITLE.to_string()
        } else {
            title
        },
    }
}

/// Short and all digits — long runs are years and catalogue numbers, which are
/// part of a name rather than a position in one.
fn is_track_number(s: &str) -> bool {
    !s.is_empty() && s.len() <= 3 && s.chars().all(|c| c.is_ascii_digit())
}

/// Drop a `03. `, `7) ` or `12 - ` prefix.
///
/// The punctuation is required: without it, `2001 A Space Odyssey` would lose
/// its year.
fn strip_track_number_prefix(s: &str) -> String {
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return s.to_string();
    }
    // Digits are ASCII, so the byte offset is the character count.
    let rest = s[digits..].trim_start();
    let after_punctuation = rest
        .strip_prefix('.')
        .or_else(|| rest.strip_prefix(')'))
        .or_else(|| rest.strip_prefix('-'))
        .or_else(|| rest.strip_prefix('_'));
    match after_punctuation {
        Some(rest) if !rest.trim().is_empty() => rest.trim().to_string(),
        _ => s.to_string(),
    }
}

/// Everything known about a local file: ffprobe's answer where there is one, the
/// filename's where there is not.
pub async fn probe(path: &Path) -> ProbedMeta {
    let from_extension = media_kind_for_path(path).unwrap_or_default();
    let probed = run_ffprobe(path).await;
    resolve_meta(path, from_extension, probed)
}

/// Combine a probe with what the filename says.
///
/// Tags win where they exist and are not blank; the filename covers the gaps.
/// Kept separate from `probe` so both the with-ffprobe and without-ffprobe paths
/// are testable without ffprobe.
pub fn resolve_meta(
    path: &Path,
    from_extension: MediaKind,
    probed: Option<FfprobeMeta>,
) -> ProbedMeta {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let parsed = parse_filename(&stem);

    let (title, artist, duration, media) = match probed {
        Some(p) => (
            p.title,
            p.artist,
            p.duration,
            // ffprobe overrules the extension in both directions: an `.mkv`
            // holding only audio needs no window, and an `.m4a` that turns out
            // to carry video does.
            if p.has_video {
                MediaKind::Video
            } else {
                MediaKind::Audio
            },
        ),
        None => (None, None, None, from_extension),
    };

    ProbedMeta {
        title: title.unwrap_or(parsed.title),
        artist: artist
            .or(parsed.artist)
            .unwrap_or_else(|| UNKNOWN_ARTIST.to_string()),
        // Zero is a fine answer: auto-advance already tolerates an unknown
        // duration, and the row simply shows `--:--`.
        duration: duration.unwrap_or(0),
        media,
    }
}

/// Run ffprobe over one file. `None` covers every failure — not installed, an
/// unreadable file, output that is not JSON — because all of them mean the same
/// thing to the caller: fall back to the filename.
async fn run_ffprobe(path: &Path) -> Option<FfprobeMeta> {
    if FFPROBE_MISSING.load(Ordering::Relaxed) {
        return None;
    }

    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        // An import cancelled mid-flight must not leave ffprobe processes behind.
        .kill_on_drop(true)
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => parse_ffprobe(&String::from_utf8_lossy(&out.stdout)),
        Ok(_) => {
            // The file itself is the problem, not ffprobe — keep probing the rest.
            warn!(path = %path.display(), "ffprobe could not read this file");
            None
        }
        Err(e) => {
            // Almost certainly not installed. Said once, not once per file: an
            // import is hundreds of these.
            if !FFPROBE_MISSING.swap(true, Ordering::Relaxed) {
                info!(err = %e, "ffprobe is not available, reading metadata from filenames");
            }
            None
        }
    }
}

/// Parse `ffprobe -print_format json -show_format -show_streams` output.
pub fn parse_ffprobe(json: &str) -> Option<FfprobeMeta> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    let format = root.get("format");
    let tags = format.and_then(|f| f.get("tags"));

    let has_video = root
        .get("streams")
        .and_then(|s| s.as_array())
        .is_some_and(|streams| streams.iter().any(is_moving_picture_stream));

    Some(FfprobeMeta {
        title: tag(tags, "title"),
        // `album_artist` is what a compilation's per-track artist is missing.
        artist: tag(tags, "artist").or_else(|| tag(tags, "album_artist")),
        duration: format.and_then(|f| f.get("duration")).and_then(as_seconds),
        has_video,
    })
}

/// A video stream that is actually moving pictures.
///
/// Embedded artwork is a video stream of a still-image codec, usually flagged
/// `attached_pic`. Older ffprobe builds omit the flag, so the codec is checked
/// too — either signal is enough to call it artwork.
fn is_moving_picture_stream(stream: &serde_json::Value) -> bool {
    if stream.get("codec_type").and_then(|t| t.as_str()) != Some("video") {
        return false;
    }
    let attached_pic = stream
        .get("disposition")
        .and_then(|d| d.get("attached_pic"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if attached_pic == 1 {
        return false;
    }
    let codec = stream
        .get("codec_name")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_lowercase();
    !STILL_IMAGE_CODECS.contains(&codec.as_str())
}

/// A tag by name, case-insensitively — Matroska shouts `TITLE`, most others do
/// not. A blank value is no value: `title = ""` is common and useless.
fn tag(tags: Option<&serde_json::Value>, want: &str) -> Option<String> {
    let object = tags?.as_object()?;
    object
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(want))
        .and_then(|(_, value)| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// ffprobe reports `duration` as a string of fractional seconds; some builds
/// use a number. Truncated to whole seconds, which is all a track row shows.
fn as_seconds(value: &serde_json::Value) -> Option<u64> {
    let seconds = match value {
        serde_json::Value::String(s) => s.parse::<f64>().ok()?,
        other => other.as_f64()?,
    };
    if seconds.is_finite() && seconds >= 0.0 {
        Some(seconds as u64)
    } else {
        None
    }
}
