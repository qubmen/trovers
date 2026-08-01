# trovers — Architecture Decisions

A running log of non-obvious choices made during development.
Each entry explains *what* was decided and *why*, so future changes
can be made with full context.

---

## ADR-001: TOML over YAML for playlists and config

**Decision:** Use `.toml` files (via the `toml` crate) for both playlist files
and `config.toml`. The original spec used YAML.

**Reasoning:**
- TOML is the native Rust ecosystem format (used by Cargo itself).
- YAML has well-known parsing ambiguities (implicit typing, the "Norway problem",
  indentation sensitivity) that can cause silent data corruption.
- The `toml` crate is well-maintained and has no surprising behaviour.
- TOML array-of-tables syntax (`[[tracks]]`) maps cleanly to `Vec<Track>`.

**Trade-off:** TOML does not support inline comments mid-value and is slightly more
verbose for nested structures — not a concern here.

---

## ADR-002: External binaries (yt-dlp + mpv) instead of Rust crates

**Decision:** Delegate all platform interaction and audio playback to `yt-dlp`
and `mpv` spawned as child processes. Do not use `rusty_ytdl`, `rodio`,
`symphonia`, or similar Rust crates for these roles.

**Reasoning:**
- `yt-dlp` supports 1800+ sites and is updated whenever platforms change their
  APIs. Rust crates covering only YouTube tend to break and go unmaintained.
- `mpv` handles every audio/video format without format-specific code and
  exposes a stable JSON IPC protocol for programmatic control.
- This architecture means *zero code changes* when adding support for a new
  platform — just pass the URL.

---

## ADR-003: IPC socket path includes PID

**Decision:** mpv IPC socket is `/tmp/trovers-<pid>.sock`, not a fixed path.

**Reasoning:**
- A fixed path (`/tmp/trovers.sock`) would break if two instances of trovers
  run simultaneously (e.g. two terminal windows).
- PID-based path is unique per process and is cleaned up in `Player::drop()`.

---

## ADR-004: Crash recovery for `downloading` status

**Decision:** `Playlist::load()` resets any track with
`cache_status = "downloading"` to `"streaming"` before returning.

**Reasoning:**
- `downloading` is a transient state set at the start of a yt-dlp job.
- If the app crashes mid-download, the TOML file is left with `downloading`,
  which is no longer accurate (no active download exists).
- On next launch, treating it as `streaming` is safe: the track will be
  re-streamed and a fresh download can be initiated.

---

## ADR-005: Atomic TOML writes

**Decision:** `Playlist::save()` writes to `<name>.toml.tmp` then renames
to `<name>.toml`.

**Reasoning:**
- A direct write to the target file leaves it in a partially-written state
  if the process is killed mid-write, corrupting the playlist.
- `rename()` is atomic on POSIX systems — the reader always sees either the
  old complete file or the new complete file.

---

## ADR-006: Two-panel TUI layout with focusable sidebar

**Decision:** The TUI is split into a left sidebar (22 cols) and a right track
table, rather than a single full-width track list with popup overlays for
playlist switching.

**Reasoning:**
- A persistent sidebar makes the available playlists always visible without
  requiring a modal popup.
- Collapsible sections (Playlists `▼/▶`) keep the sidebar compact as the
  playlist count grows.
- `Tab` switches focus; border colour (Rust Orange vs grey) makes the active
  panel unambiguous.
- Music/Video category items are present in the sidebar in `TEXT_DIM` as
  reserved placeholders for future functionality.

---

## ADR-007: `Table` widget for track list, not `List`

**Decision:** Tracks are rendered using ratatui's `Table` widget with explicit
column constraints, not `List` with manually formatted strings.

**Reasoning:**
- `Table` provides proper column alignment regardless of content length.
- Artist, duration, and status columns are right- or fixed-width aligned
  without string padding tricks.
- Easier to add/remove columns without reformatting every row string.

---

## ADR-008: Custom progress bar instead of ratatui `LineGauge`/`Gauge`

**Decision:** The playback progress bar (line 2 of Now Playing) is rendered
as a `Paragraph` built from unicode chars (`━`, `◉`, `─`) rather than a
ratatui `Gauge` or `LineGauge` widget.

**Reasoning:**
- `LineGauge` does not support a custom thumb character (`◉`).
- `Gauge` renders as a block-filled bar which looks heavy for a single line.
- Building the bar manually as `Span`s gives full control over the thumb
  position, fill characters, and colour without fighting widget internals.
- The same helper (`build_progress_bar`) is reused for the caching bar on
  line 3 with `▓`/`░` chars and Gold colour.

---

## ADR-009: Equalizer animation without audio data

**Decision:** The `▁▂▃▄▅▆▇` equalizer bars in Now Playing line 3 are purely
cosmetic — heights are pseudo-random, not derived from actual audio spectrum data.

**Reasoning:**
- Getting real spectrum data from mpv would require a complex audio filter
  pipeline and is out of scope.
- A pseudo-random animation (LCG seeded by `eq_tick + bar_index`) gives the
  visual impression of an active player with zero audio processing.
- Animation only runs when `player.is_some()`, so bars are static (hidden)
  when nothing is playing.

---

## ADR-010: No `indicatif` crate

**Decision:** `indicatif` was removed from dependencies and must not be used.

**Reasoning:**
- `indicatif` writes directly to stdout/stderr, bypassing ratatui's terminal
  control. This causes rendering corruption in full-screen TUI mode.
- All progress indication is done through ratatui widgets or custom `Paragraph`
  lines built from unicode block characters.

---

## ADR-011: `PlayingSession` — playback identity decoupled from the displayed playlist

**Decision:** The currently-playing track is tracked via
`App.playing: Option<PlayingSession>`, where `PlayingSession` holds a full
second `Playlist` (not a lightweight metadata snapshot), its file path, and
the playing track's index within it — entirely independent of
`App.playlist`/`App.playlist_path` (the playlist currently shown in the
track list). `App::playing_track()`/`playing_track_mut()` are the only
sanctioned way to read/mutate "what's playing", with a sync rule: if
`playing.path == app.playlist_path` (the user happens to be browsing the same
playlist that's playing), the accessors borrow live data from `app.playlist`
by matching `video_id`, so edits made through the track list (delete, rename,
speed change, etc.) are reflected immediately with no manual sync step. Only
when the user has switched to browsing a *different* playlist does
`PlayingSession` fall back to its own private `Playlist` copy.

**Reasoning:**
- Before this change, `App.playlist.current_track` served two incompatible
  roles at once: "the source of truth for what's playing" and "the playlist
  currently shown on screen." Switching playlists in the sidebar forcibly
  stopped playback (`switch_to_playlist` set `self.player = None`, zeroed
  `position`, reset `pos_tx`) because there was no way to represent "still
  playing playlist A" while "now displaying playlist B" — they were the same
  field.
- The playing track needs full `Track` data regardless (title, artist,
  `last_position`, `speed`, `cache_status` transitions during hot-switch from
  stream to local file) — a metadata-only snapshot would need its own parallel
  update/staleness logic. Reusing the existing `Playlist`/`Track` types for
  the second in-memory copy avoids inventing a shadow struct.
- The borrow-when-paths-match rule specifically prevents two silently
  diverging copies of the same on-disk playlist: if a user deletes or edits
  the very track that's playing (because they're browsing the playlist that
  contains it), that edit must be visible immediately in Now Playing without
  a second write path — otherwise `app.playlist` and `playing.playlist` would
  each hold their own copy of the same track and drift apart on every edit.
- Rendering (`render_now_playing_header`, `render_track_info_row`,
  `render_playback_bar`, and the `▶` row highlight in `render_track_table`)
  now reads from `app.playing_track()` / the extracted `row_is_playing`
  predicate instead of `app.playlist.current_track`, so Now Playing and the
  highlight survive playlist switches and always show the actually-playing
  track regardless of what's on screen.

**`n`/`b` intentionally still operate on the displayed playlist:** despite
playback identity living in `PlayingSession`, the `n`/`b` handlers derive
their cursor from `app.selected`/`app.track_index_at` (the **displayed**
playlist), never from `app.playing`. This is deliberate, not an oversight:
the confirmed product behavior is "browsing playlist X and pressing `n`/`b`
walks X's tracks," even while a track from an unrelated playlist Y continues
playing in the background. Resolving `n`/`b` against `app.playing` instead
would silently change what "next track" means depending on what happens to
be playing, which contradicts the whole point of decoupling the two in the
first place. Any future change that makes `n`/`b` "smarter" by preferring the
playing session's neighbors needs a new product decision, not a silent
reversion.

**Root cause of the add-track playback-hijack bug (do not reintroduce):**
`TaskMsg::MetaReady`'s active-playlist branch used to unconditionally run
`self.playlist.current_track = Some(video_id.clone())` for *every* newly
added track, even though adding a track never asked to play it. Because
`current_track` was the sole "what's playing" signal at the time, this
silently made the brand-new, undownloaded track "the current track." When
its background download finished, `TaskMsg::DownloadDone`'s
`current_track.as_deref() == Some(&video_id)` check then wrongly matched the
new track and triggered a stream→local-file hot-switch — restarting mpv on
the unrelated new track, seeked to whatever position the *actually* playing
track was at. The fix (Task 1) removed that assignment entirely: adding a
track only pushes it into `tracks` and populates `download_targets`/
`downloading`; it must never write to any "what's playing" field. Task 3
finished closing this off by rebasing the hot-switch identity check on
`self.playing.as_ref().map(|p| &p.track().video_id) == Some(&video_id)` —
`PlayingSession`, not any field on the displayed `Playlist`, is now the only
valid source of truth for "is this the track that's actually playing."
**Guard rail:** any future "convenience" change to `MetaReady` (or any other
add/import path) that writes to `current_track`, `app.playing`, or otherwise
implies a freshly-added track is "now playing" must be rejected — adding a
track must never change playback state, by design, confirmed by the product
owner.
