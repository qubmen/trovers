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
- TOML maps cleanly to the small flat structs used here — a track document is a
  single table, a playlist a table with one array of strings.

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

**Decision:** mpv IPC socket is `/tmp/trovers-<pid>-<seq>.sock`, not a fixed
path. `<seq>` is a per-process counter bumped on every `Player::spawn`.

**Reasoning:**
- A fixed path (`/tmp/trovers.sock`) would break if two instances of trovers
  run simultaneously (e.g. two terminal windows).
- PID-based path is unique per process and is cleaned up in `Player::drop()`.
- The `<seq>` half keeps *successive* players within one process apart. mpv does
  not unlink its socket on exit, so an outgoing player's file is still on disk
  when the next one starts; sharing one path per process would let a poll aimed
  at the new player connect to the old socket and read the old track's position.
- The pid stays first and parseable so `reap_orphaned_players` (ADR-012) can
  tell whose socket a leftover file is.

---

## ADR-004: Crash recovery for `downloading` status

**Decision:** `Library::load()` resets any track with
`cache_status = "downloading"` to `"streaming"` before returning. (Originally
`Playlist::load()`; it moved with ADR-015, which also gave it the
`cached`-but-`file`-gone case — one place, once per launch, instead of once per
playlist file.)

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

> **Amended by ADR-015.** The decoupling stands, and so does everything below
> about *why* playback identity cannot live on the displayed playlist. What went
> away is the dual-resolution machinery: `PlayingSession` now holds a
> `track_id: String` and the accessors are a single library lookup, because a
> track's state has exactly one home on disk. Read the paragraphs about
> "borrow-when-paths-match" as history — they describe the problem ADR-015
> deleted rather than solved.

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
track only pushes its id into `tracks` and marks it `downloading`; it must never write to any "what's playing" field. Task 3
finished closing this off by rebasing the hot-switch identity check on
`self.playing.as_ref().map(|p| &p.track().video_id) == Some(&video_id)` —
`PlayingSession`, not any field on the displayed `Playlist`, is now the only
valid source of truth for "is this the track that's actually playing."
**Guard rail:** any future "convenience" change to `MetaReady` (or any other
add/import path) that writes to `current_track`, `app.playing`, or otherwise
implies a freshly-added track is "now playing" must be rejected — adding a
track must never change playback state, by design, confirmed by the product
owner.

---

## ADR-012: Orphaned mpv is prevented in four independent layers

**Decision:** Nothing is allowed to leave mpv playing with no UI attached. Four
mechanisms cover it, deliberately overlapping:

1. `kill_on_drop(true)` on the mpv `Command` (and on both yt-dlp ones).
2. `Player::drop` — `start_kill()` plus unlinking the socket file.
3. A `tokio::signal` task for SIGINT/SIGTERM/SIGHUP that sets `should_quit`, so
   the event loop runs its normal shutdown (flush position → save → kill mpv →
   restore terminal) rather than dying without unwinding.
4. `player::reap_orphaned_players()` at startup: connect to every
   `/tmp/trovers-<pid>-<seq>.sock` whose pid is no longer alive, send
   `{"command":["quit"]}`, unlink the file.

**Reasoning:**
- Each layer covers a hole the others cannot reach, which is why all four exist:
  - `Player::drop` only helps once the `Player` struct is fully built. Inside
    `Player::spawn` there is a socket-wait loop of up to a second during which
    `Child` is a bare local; if that future is cancelled (user quits or switches
    track), only `kill_on_drop` can still kill the process. `tokio::process`
    *detaches* on drop by default — this is the single most surprising fact in
    the whole file, and the original cause of the reported symptom.
  - Both of the above need unwinding to happen at all, which a signal does not
    provide. Hence layer 3. Ctrl+C is handled separately in `handle_key`, since
    raw mode suppresses the terminal's own SIGINT translation.
  - Nothing in-process survives `SIGKILL` or a lost power supply. Layer 4 is the
    self-healing net that makes the symptom stop recurring rather than merely
    becoming rarer.
- The reaper only touches sockets whose encoded pid is no longer a live process,
  so a second trovers running concurrently is never disturbed. If a dead
  instance's pid has since been recycled, its socket is skipped this time round;
  the next launch that sees the pid free cleans it up. Erring towards leaving one
  stale file behind is much cheaper than killing a live sibling's player.
- A refused connection is not an error for the reaper: mpv does **not** unlink
  its IPC socket when it exits, so a leftover file usually means the process is
  already gone and only the file needs removing.

---

## ADR-013: mpv's own exit is the end-of-track signal

**Decision:** Auto-advance is driven by mpv exiting, surfaced as
`TaskMsg::PlayerGone`, not by polling `eof-reached` or `percent-pos`. A
`PlayerGone` counts as "the track finished" only when
`App::reached_end_of_track()` agrees: the last polled position is within
`EOF_SLACK_SECS` (10s) of the track's duration, or the duration is unknown (`0`).
Anything else is reported as "Playback stopped unexpectedly" and does not
advance.

**Reasoning:**
- mpv is spawned without `--idle` and without `--keep-open`, so it exits by
  itself the moment a track ends. That exit is already detected reliably —
  `poll_position_loop` sees `ECONNREFUSED` — so there is nothing to poll for.
  An `eof-reached` property poll would add a second, weaker signal for an event
  the existing one already reports, and would have to race mpv's teardown to
  read it. Verified against real mpv in the `real_mpv_*` tests.
- The position check is the load-bearing half. mpv also exits when a stream
  breaks, when it meets a codec it cannot handle, and when something kills it
  from outside. Advancing on those would walk the entire playlist in seconds,
  respawning mpv and yt-dlp for every track on the way — a far worse failure
  than not advancing.
- The 10s slack is not tuning slop: the position poller samples once a second, so
  the last reading always lags where mpv actually reached, and a stream whose
  reported duration is slightly optimistic lags further still. `duration == 0`
  (metadata gave no duration) is treated as finished, because there is nothing to
  compare against and refusing to advance forever is the worse default.
- `handle_track_ended` rewinds the finished track's `last_position` to 0 before
  advancing. Otherwise the resume logic would reopen the track at its own EOF —
  and with auto-advance live, playing it again would skip straight past it.
- Auto-advance follows the **playing** playlist (`PlayingSession`), not the
  displayed one. This is the one place that differs from `n`/`b`, which
  deliberately step what is on screen (see ADR-011): a track ending is not a
  cursor movement, and the user browsing elsewhere must not redirect it.

---

## ADR-014: Shuffle is a stored permutation, not a random pick per step

**Decision:** `playlist::shuffled_indices(len, seed)` builds a Fisher-Yates
permutation of `0..len`, cached on `App` alongside the playlist path and length
it was built for, and rebuilt when either changes. `next`/`previous` and
auto-advance walk that order. Shuffle is ignored while a search filter is active.

**Reasoning:**
- A permutation is what makes a shuffled walk visit every track exactly once
  before repeating any, and it is the only way "previous" has a meaningful
  answer. Drawing a random index per step gives both properties up.
- Taking the seed as a parameter (callers pass `shuffle_seed()`, which reads the
  clock) is what makes the order testable: tests pin an explicit permutation
  instead of asserting statistics.
- Fisher-Yates with an inline `xorshift64*` avoids a `rand` dependency. Nothing
  about choosing play order is security-sensitive.
- Deferring to an active search filter: with a filter on, the visible rows are
  already a deliberately chosen subset in a deliberate order, and the indices
  `n`/`b` step are positions in that subset, not in the playlist. Shuffling them
  would make the cursor jump around inside a list the user is reading. Clearing
  the search restores the shuffled walk.

---

## ADR-015: Each track is its own document; playlists are ordered id lists

**Decision:** A track lives in one file — `tracks/<slug>-<platform-id>[-N].toml`
— indexed by the `id` recorded *inside* it. A playlist is that playlist's own
settings plus `tracks: Vec<String>`, an ordered list of those ids. `Library`
owns the documents; `Playlist` owns nothing but running order. Migration from
the embedded format runs at startup, detects the old shape with an untagged
serde enum, and backs `playlists/` up before it writes anything.

**Reasoning:**
- **It deleted more code than it added.** Three separate mechanisms existed only
  because one track's state could sit in several playlist files at once:
  `download_targets` + `remap_download_targets` + `retarget_download` +
  `clear_download_state_for_playlist` (all answering "which playlist file owns
  this download's row"), `PlayingSession`'s dual resolution plus
  `save_playing_session_playlist` and `patch_and_save_playlist`, and
  `video_id_referenced_elsewhere` loading and parsing every playlist in full.
  With one home per track, a download patches a document by id and renaming,
  moving or deleting a playlist has nothing to repoint.
- **It fixed a real quirk rather than merely tidying.** The same video in two
  playlists used to carry two independent positions and speeds, and whichever
  copy was written last won. One document means one position.
- **Writes got proportional.** The 15-second position flush rewrote the entire
  playlist TOML — every track's metadata — to record one integer. It now writes
  one small file.
- **A track becomes movable and shareable.** Moving between playlists is an
  id-list edit, and a single file describes a track completely, which is what
  makes both albums (ADR pending) and eventual export possible.

**`id` is authoritative; the filename is only a hint.** macOS filesystems are
case-insensitive and YouTube ids are not, so `aB` and `Ab` can collide as
filenames while being different tracks. The colliding document gets a `-2`
suffix and its `id` stays exact. Nothing reads the filename to identify a track,
so a user is free to rename a document.

**Why the id is `<slug>:<platform-id>` and not the URL.** `youtube.com/watch?v=X`
and `music.youtube.com/watch?v=X` are the same track; so are URLs differing only
in tracking parameters. The slug is the registrable (second-to-last) dot label
of `source`, lowercased, so both map to `youtube` and one video is one document.

**Why the audio cache still uses platform-id filenames.** Cached audio stays at
`audio/<platform-id>.opus`, and `ytdlp.rs` keeps calling its parameter
`video_id` — there it genuinely is a platform video id, handed to yt-dlp. Two
reasons: every file users have already downloaded stays valid, and the id
scheme is a trovers concept that has no business leaking into the code that
talks to yt-dlp. The consequence to keep straight is that `Track::platform_id()`
(derived, everything after the first `:`) is what the cache and the player take,
while progress reporting is keyed by the *library* id — which is why
`ytdlp::download` takes a `progress_key` parameter separate from `video_id`.

**Migration, and why it is safe to run on every launch.** Detection is by shape,
not by a version field: `tracks` parses as `Vec<String>` (already migrated, left
untouched) or `Vec<LegacyTrack>` (rewrite). The backup directory
`playlists.backup-<utc>/` is written before the first mutation, so a failure
halfway leaves the originals recoverable. Two playlists sharing a video produce
one document — first writer wins, and the skipped duplicates are logged.
An unparseable playlist is logged and skipped rather than aborting the whole
migration: one bad file must not hold the other playlists hostage.

**Trade-off accepted:** a playlist file is no longer self-contained, so copying
one out of `playlists/` gives a list of ids and nothing else. Sharing wants a
bundle command; see "Deferred" in the plan. The upside — one position per track,
proportional writes, a whole class of ownership bookkeeping gone — is worth it.
