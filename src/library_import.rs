//! Folding a folder of the user's own media into an album.
//!
//! `library_scan` finds and describes files; this module decides what that means
//! for the library and for the album's running order. The two halves are apart
//! because only the second one has an opinion the user can feel: a rescan must
//! never delete a row, never reorder one, and never lose what the user built up
//! on a track.

use crate::library::{CacheStatus, Library, Track, TrackOrigin};
use crate::library_scan::{local_id, probe, scan, ProbedMeta};
use crate::playlist::Playlist;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::error;

/// How many ffprobe processes an import runs at once. Enough to keep a folder of
/// hundreds moving, few enough that the machine stays usable while it does.
const PROBE_CONCURRENCY: usize = 8;

/// What an album is called when the folder's name yields nothing usable.
const FALLBACK_ALBUM_NAME: &str = "Imported folder";

/// Characters `validate_playlist_name` rejects, which a folder name may perfectly
/// well contain — macOS allows `:` in one.
const INVALID_NAME_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];

/// A scanned file and what probing made of it — what an import hands back for
/// merging.
pub struct ImportedFile {
    pub path: PathBuf,
    pub meta: ProbedMeta,
}

/// What a merge did, for the status line.
#[derive(Debug, Default, PartialEq)]
pub struct ImportReport {
    /// Rows appended to the album.
    pub added: usize,
    /// Documents already listed whose contents a rescan brought up to date.
    pub updated: usize,
    /// Rows whose file is no longer in the folder. Marked, never removed.
    pub missing: usize,
}

/// Characters a shell escapes with a backslash, and which therefore arrive
/// escaped when a folder is dragged into a terminal. Anything else after a
/// backslash is taken literally — the backslash is then part of the name.
const SHELL_ESCAPABLE: &[char] = &[
    ' ', '\t', '(', ')', '[', ']', '{', '}', '\'', '"', '`', '$', '&', ';', '|', '<', '>', '*',
    '?', '!', '#', '~', '\\',
];

/// Turn what the user typed or pasted into a path on this filesystem.
///
/// "Copy" on a folder does not put a path on the clipboard — it puts one of
/// several *spellings* of a path, and none of them is what `PathBuf::from` wants:
/// macOS hands over a percent-encoded `file://` URL, a terminal escapes the
/// spaces, and a shell wraps the whole thing in quotes. Every one of those
/// silently fails to be a directory, so they are all normalised here, in the one
/// place typed text becomes a path.
pub fn path_from_input(input: &str, home: Option<&Path>) -> PathBuf {
    let trimmed = strip_quotes(input.trim());
    match file_url_path(trimmed) {
        // A `file://` URL is percent-encoded by definition and absolute by
        // construction: no unescaping, no tilde.
        Some(decoded) => PathBuf::from(decoded),
        None => expand_tilde(&unescape_shell(trimmed), home),
    }
}

/// The path half of a `file://` URL, percent-decoded — or `None` when the input
/// is not one, which is what keeps a literal `%` in an ordinary path literal.
///
/// A host other than `localhost` is left in place rather than guessed at: a
/// remote `file://host/share` is not something trovers can open, and turning it
/// into a plausible-looking local path would be worse than failing.
fn file_url_path(input: &str) -> Option<String> {
    let rest = input.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    Some(percent_decode(rest))
}

/// Decode `%XX` escapes. Bytes first, then UTF-8: one Cyrillic letter is two
/// escapes, so decoding per character would produce mojibake.
///
/// An escape that is not one — a trailing `%`, `%zz` — is left exactly as it is.
/// Hand-rolled for the same reason as the FNV-1a in `library_scan`: it is shorter
/// than the dependency.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Drop the backslashes a shell would have eaten. Only before a character a
/// shell actually escapes, so a backslash that is part of a name survives.
fn unescape_shell(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(next) if SHELL_ESCAPABLE.contains(&next) => out.push(next),
            Some(next) => {
                out.push('\\');
                out.push(next);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Unwrap one matching pair of surrounding quotes — what a shell adds around a
/// path with spaces in it.
fn strip_quotes(input: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = input
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    input
}

/// Resolve a leading `~` against `home`.
///
/// Only a bare `~` or a `~/` prefix: `~alice` is another user's home, which only
/// a shell can resolve, so it is left exactly as typed rather than quietly
/// rewritten into a path under *this* user's home.
pub fn expand_tilde(input: &str, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return PathBuf::from(input);
    };
    if input == "~" {
        return home.to_path_buf();
    }
    match input.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(input),
    }
}

/// The album name a folder suggests: its own name, with anything a playlist
/// filename cannot hold replaced.
pub fn album_name_for_folder(root: &Path) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let cleaned: String = name
        .chars()
        .map(|c| {
            if INVALID_NAME_CHARS.contains(&c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        FALLBACK_ALBUM_NAME.to_string()
    } else {
        cleaned
    }
}

/// `base`, or `base (2)`, `base (3)`, ... — the first spelling no playlist in
/// `taken` already claims.
///
/// Importing the same folder twice is a mistake worth surviving, and two
/// playlists cannot share a filename.
pub fn unique_album_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|name| name == base) {
        return base.to_string();
    }
    for n in 2.. {
        let candidate = format!("{base} ({n})");
        if !taken.iter().any(|name| name == &candidate) {
            return candidate;
        }
    }
    unreachable!("the loop returns as soon as a name is free")
}

/// Scan `root` and probe everything found, at most `PROBE_CONCURRENCY` at a time,
/// calling `progress(done, total)` as each file finishes.
///
/// The result is in scan order — which becomes the album's running order —
/// however out of order the probes complete.
pub async fn scan_and_probe<F>(root: &Path, progress: F) -> Vec<ImportedFile>
where
    F: Fn(usize, usize),
{
    let files = scan(root);
    let total = files.len();

    let permits = Arc::new(Semaphore::new(PROBE_CONCURRENCY));
    let mut probes = JoinSet::new();
    for (idx, file) in files.into_iter().enumerate() {
        let permits = Arc::clone(&permits);
        probes.spawn(async move {
            // Held until the probe returns. `acquire_owned` cannot fail here —
            // nothing closes this semaphore.
            let _permit = permits.acquire_owned().await;
            let meta = probe(&file).await;
            (
                idx,
                ImportedFile {
                    path: file.path,
                    meta,
                },
            )
        });
    }

    // Slots rather than a growing list: a probe that finishes early must not
    // overtake the file before it.
    let mut slots: Vec<Option<ImportedFile>> = (0..total).map(|_| None).collect();
    let mut done = 0;
    while let Some(finished) = probes.join_next().await {
        match finished {
            Ok((idx, imported)) => slots[idx] = Some(imported),
            // A panicking probe loses that one file, not the import.
            Err(e) => error!(err = %e, "a probe task failed"),
        }
        done += 1;
        progress(done, total);
    }
    slots.into_iter().flatten().collect()
}

/// Fold the contents of `root` into `album` and the library.
///
/// Three rules, in the order they matter:
///
/// 1. **Nothing is deleted.** A file that has gone leaves its row in place,
///    marked `Missing`, so a renamed file or an unplugged drive costs nothing but
///    a dim row — and the row heals itself when the file comes back.
/// 2. **Nothing is reordered.** Rows already listed keep their positions and new
///    files land at the end. Reordering would invalidate the shuffle order and
///    the user's sense of place.
/// 3. **A known file keeps its document.** The id is derived from the path, so a
///    rescan finds the same document and `last_position`, `speed` and a renamed
///    title survive. Only what the file itself decides — title, artist, duration,
///    media kind — is brought up to date.
pub fn merge_scan(
    library: &mut Library,
    album: &mut Playlist,
    root: &Path,
    files: Vec<ImportedFile>,
) -> ImportReport {
    let mut report = ImportReport::default();
    let mut scanned_ids = Vec::with_capacity(files.len());

    for file in files {
        let id = local_id(&file.path);
        scanned_ids.push(id.clone());

        match library.get_mut(&id) {
            Some(track) => {
                if refresh(track, &file) {
                    save_document(library, &id);
                    report.updated += 1;
                }
            }
            None => {
                if let Err(e) = library.upsert(local_track(id.clone(), file)) {
                    error!(err = %e, id = %id, "failed to write the imported track's document");
                }
            }
        }

        if !album.tracks.contains(&id) {
            album.tracks.push(id);
            report.added += 1;
        }
    }

    // Anything the scan did not turn up. Scoped to local rows inside this folder:
    // a track added from a URL, or a file the user dropped in from elsewhere, is
    // not something this folder's scan can have an opinion about.
    for id in &album.tracks {
        if scanned_ids.contains(id) {
            continue;
        }
        let vanished = library.get(id).is_some_and(|track| {
            track.origin == TrackOrigin::Local
                && track.cache_status != CacheStatus::Missing
                && track_is_under(track, root)
        });
        if !vanished {
            continue;
        }
        if let Some(track) = library.get_mut(id) {
            // The recorded path stays: it is the only way back to `Cached`.
            track.cache_status = CacheStatus::Missing;
        }
        save_document(library, id);
        report.missing += 1;
    }

    // Being linked to the folder is what makes the album rescannable at all.
    album.source_folder = Some(root.to_path_buf());
    report
}

/// Bring a known file's document back in line with the file, returning whether
/// anything actually changed — a rescan that finds nothing new must write nothing
/// and report nothing.
fn refresh(track: &mut Track, file: &ImportedFile) -> bool {
    let url = file.path.to_string_lossy().to_string();
    let changed = track.url != url
        || track.title != file.meta.title
        || track.artist != file.meta.artist
        || track.duration != file.meta.duration
        || track.media != file.meta.media
        || track.cache_status != CacheStatus::Cached
        || track.file.as_deref() != Some(file.path.as_path());
    if !changed {
        return false;
    }

    track.url = url;
    // What the file says about itself. The user's own renaming lives in
    // `user_title`/`user_artist`, which is why overwriting these is safe.
    track.title = file.meta.title.clone();
    track.artist = file.meta.artist.clone();
    track.channel = file.meta.artist.clone();
    track.duration = file.meta.duration;
    track.media = file.meta.media;
    track.cache_status = CacheStatus::Cached;
    track.file = Some(file.path.clone());
    true
}

/// A document for a file the library has not seen before.
fn local_track(id: String, file: ImportedFile) -> Track {
    Track {
        url: file.path.to_string_lossy().to_string(),
        source: "local".to_string(),
        title: file.meta.title,
        // A local file has no channel; saying the artist twice reads better in the
        // track table than a blank column.
        artist: file.meta.artist.clone(),
        channel: file.meta.artist,
        duration: file.meta.duration,
        id,
        // The scan just found it, so it is there. `Library::load` checks again on
        // every launch.
        cache_status: CacheStatus::Cached,
        file: Some(file.path),
        last_position: 0,
        speed: None,
        user_title: None,
        user_artist: None,
        added_at: Utc::now(),
        origin: TrackOrigin::Local,
        media: file.meta.media,
        resume: true,
    }
}

/// Whether a track's file sits inside `root`.
///
/// Lexical, like `local_id`: the file may be on an unplugged drive, and
/// `canonicalize` needs it to exist.
fn track_is_under(track: &Track, root: &Path) -> bool {
    let path = track
        .file
        .clone()
        .unwrap_or_else(|| PathBuf::from(&track.url));
    let normalize = |p: &Path| -> PathBuf { p.components().collect() };
    normalize(&path).starts_with(normalize(root))
}

/// Persist one document, logging rather than failing: the row is right in memory
/// either way, and the next rescan writes it again.
fn save_document(library: &Library, id: &str) {
    if let Err(e) = library.save(id) {
        error!(err = %e, id = %id, "failed to save an imported track's document");
    }
}
