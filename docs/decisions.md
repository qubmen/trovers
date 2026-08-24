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

**Security note:** `Player::spawn` chmods the socket to `0600` right after mpv
creates it, since mpv itself creates it with whatever the process umask
allows. On a shared machine an unrestricted socket would let any other local
user send play/pause/seek/quit commands to someone else's mpv — low severity
(it is a media player, not a credential store), but free to close off.

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
  makes both albums (ADR-016) and eventual export possible.

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

---

## ADR-016: An album is an ordinary playlist file naming its parent

> **Amended by ADR-019.** The storage decision stands whole: an album is still a
> playlist file with `kind` and `parent`, still flat on disk, still two levels
> deep. What changed is *where an album is drawn* — not the sidebar, but the
> parent's own track list, as a collapsible group. Where the reasoning below
> talks about the sidebar rendering an album, read it as history: the sidebar
> now lists only top-level playlists and orphaned albums.

**Decision:** An album is a `playlists/<name>.toml` like any other, with two extra
fields: `kind = "album"` and `parent = "<parent's file stem>"`. There is no album
type, no nested structure inside a playlist document, and no third directory.
Nesting stops at two levels: playlist → album.

**Reasoning:**
- **Every playlist operation works on an album for nothing.** Rename, delete,
  shuffle, loop mode, move-a-track-here, cursor restore, the `current_track`
  field — all of it already operates on a playlist file, so an album inherits the
  lot. A nested `Vec<Album>` inside a playlist document would have meant a second
  implementation of each.
- **A flat directory keeps the failure modes flat.** With `parent` a plain string
  the worst case is a dangling reference, and the answer to it is obvious: an
  album whose parent is gone renders at the top level. A tree in a single file
  would make one unparseable document lose a whole branch.
- **Two levels is a product decision, not a limitation of the model.** Arbitrary
  depth costs a recursive sidebar, a recursive delete/rename, and cycle
  detection, to serve a case nobody asked for. Importing a folder while an album
  is displayed attaches the new album to that album's *parent*.

**Why `parent` holds a file stem rather than `Playlist.name`.** The stem is what
`Playlist::list_entries` and the sidebar have in hand without opening every file,
and it is the thing the filesystem already keeps unique. Renaming a parent
rewrites its children's `parent`.

**Trade-off accepted:** deleting a parent orphans its albums to the top level
instead of deleting them. Surprising for a moment, but the alternative is a
single confirmation prompt destroying playlists the user never named — and
trovers' rule is that deleting one thing deletes one thing.

---

## ADR-017: ffprobe is a soft dependency; yt-dlp and mpv stay hard

**Decision:** `deps.rs` fails startup only on a missing yt-dlp or mpv. ffprobe is
used when it happens to be on PATH and quietly not when it isn't: `MediaKind`
falls back to the file extension, title and artist to the filename, and
`duration` to `0`. The first spawn failure is logged once — an `AtomicBool`, not a
warning per file, because an import is hundreds of them.

**Reasoning:**
- **yt-dlp and mpv are the program; ffprobe is a nicety.** Without either of the
  first two trovers cannot fetch or play anything, so failing loudly at startup
  is the honest response. Without ffprobe an import still produces playable rows
  with readable names — refusing to run would trade a full feature for a better
  one.
- **The fallbacks were already there.** `duration == 0` is what a streaming
  track's row shows before metadata arrives, `reached_end_of_track` already
  tolerates it (mpv's own exit is the end-of-track signal, ADR-013), and the row
  renders `--:--`. Filename parsing is ~20 lines and `Artist - Title` is a
  convention worth honouring.
- **ffprobe ships with ffmpeg, which many machines have and no machine promises.**
  Making it hard would turn "import my music folder" into "install ffmpeg first"
  for a duration column.

**Where it does overrule the guess:** an `.mkv` carrying only audio needs no video
window, and an `.m4a` that turns out to hold video does. Embedded cover art is a
"video stream" to ffprobe, so `attached_pic` and a still-image codec list are both
checked — otherwise every tagged mp3 would open a window.

---

## ADR-018: trovers never deletes a file the user brought

**Decision:** Anything under `origin = "local"` is read-only to trovers. Deleting
a row removes the row and, when nothing else references it, the *document* — never
the media file. Deleting an entire album leaves the folder untouched. Recaching a
local track is a no-op with a status message. A `Missing` row refuses to play
instead of spawning mpv.

**Reasoning:**
- **The two origins mean opposite things by ownership.** A cached file under
  `audio/` is trovers' own copy of something re-downloadable, so removing the last
  row that references it is housekeeping. A file in the user's Music folder is the
  only copy in the world, and trovers is a player pointed at it. One `remove_file`
  call on the wrong branch is unrecoverable data loss, which puts it in a
  different class from every other bug in this codebase.
- **The guard belongs at the deletion sites, not in the confirmation text.** Three
  places touch it — `handle_confirm_delete`, `recache_track`, `request_playback` —
  and each checks `origin` itself rather than trusting a caller to have asked.
- **`Missing` exists so a gone file is a visible row, not a silent one.** A local
  track whose path is empty keeps its row and its recorded path, renders a dim
  `⊘`, and heals back to `Cached` on the next load once the drive is plugged back
  in. Dropping the row would lose the position and the id the moment a drive was
  unmounted.

**Consequence for rescan:** a rescan appends and marks, never deletes or reorders.
The same instinct — the user's folder is the source of truth about files, and
trovers' list is the source of truth about their order.

---

## ADR-019: An album is drawn inside its parent's track list, not in the sidebar

**Amends ADR-016**, which is unchanged about storage.

**Decision:** The track list is a two-level tree rather than a flat window over
`playlist.tracks`: the displayed playlist's own tracks first, then each of its
albums as a collapsible group — a header row, and its tracks when open. The
sidebar lists only playlists that are not albums with a live parent. `App` holds
`albums: Vec<LoadedAlbum>` and a computed `rows: Vec<VisibleRow>`; every cursor
position resolves through a row, which names both the list its track comes from
and the index within it. An album plays as its own list. A new
`collapsed: bool` on `Playlist` remembers the fold, defaulting to folded.

**Reasoning:**
- **The sidebar has 22 columns and a nested album row had 14 of them.** Real
  names arrived as `Кино - Гр…` and `Суржиков …` — indistinguishable from each
  other and from anything else imported from the same series. The panel with room
  for a name is the track table, and it is also where an album's contents belong:
  an album is part of the playlist you are looking at, not a sibling of it.
- **A row that names its owner beats a flattened list with a side table.** The
  alternative was one display vector of ids plus a map from row to owning file.
  Smaller diff, but ownership becomes implicit and every mutation — `d`, `J`/`K`,
  a rescan — has to re-derive it. That is the bookkeeping ADR-015 deleted; adding
  it back to save a struct would be a bad trade.
- **`rows` is rebuilt, never edited.** `rebuild_rows` is its only writer and runs
  after anything that changes the screen: a switch, a search keystroke, a fold, an
  import, a rescan, a rename, a delete, a reorder. One derivation cannot disagree
  with itself, and the old `filtered_indices` — a parallel copy of the answer —
  goes away.
- **An album playing as its own list is nearly free.** `PlayingSession` has
  carried its own `path` and `playlist` since ADR-011, so `n`/`b`, `loop_mode`,
  `shuffle` and auto-advance stay inside the album with no new machinery, each
  album keeping its own shuffled order in its own file. It is also the honest
  reading of what the user asked for: an album is a thing you play.
- **A header is not playable.** `Enter` on one opens or closes it. Making a header
  play its first track would put "start something" and "look inside" on the same
  key, and the wrong one on a 200-file folder is loud.
- **Folded by default, and the fold is the album's own business.** A file written
  before this field existed loads folded, which is what a two-hundred-file import
  should arrive as. Storing it in the album rather than in a global UI-state file
  means it travels with the thing it describes and needs no second writer. A
  freshly imported album is stored open, so the import is visibly there.

**Where the keys went.** `r` and `d` used to reach an album through the sidebar. On
the header row they mean rename and forget-this-album — `InputMode::AlbumRename`
and `AlbumDelete`, separate modes from the sidebar's because they address the album
under the cursor rather than the sidebar's selected row. `R` on a header rescans
that album's folder. `J`/`K` refuse on a header: albums are sorted by name.
Deleting an album still never touches the folder (ADR-018).

**Consequence for ownership.** Anything that edits or deletes a row now edits the
row's *owning* list and saves that file — `handle_confirm_delete`,
`move_track_to_playlist`, the `J`/`K` swap, which also refuses to cross a list
boundary. And because the displayed playlist's albums are held in memory with
edits not yet on disk, the two functions that read other playlist files —
`platform_id_referenced_elsewhere` and `import_target_for` — answer from
`self.albums` first and skip those paths on disk. Reading the file instead would
miss the removal that just happened and leak the document, or write a stale copy
back over it.

**Trade-off accepted:** the scroll counter in the panel title counts rows, so with
albums present its denominator exceeds the track count by the number of headers.
The alternative — counting only tracks — would make the counter disagree with the
cursor. With no albums the two are equal and the title is what it always was.

**Out of scope:** reordering albums by hand, moving an album to another parent,
nesting deeper than two levels, and a search that matches an album's folder path.

---

## ADR-020: A video window only for a video track, and no focus policy of our own

**Decision:** `Player::spawn` takes a `video: bool`. When set, `--no-video` is
dropped and `--force-window=yes` added; otherwise nothing changes. The flag comes
from the track's own `media`, which is `Video` only for a local file. mpv's whole
command line is built by a pure `player::mpv_args(socket, start_pos, video, extra)`
so it can be tested without mpv. `--no-terminal` and `--really-quiet` stay
unconditional. We ship **no** window-management flags; `config.video_mpv_args`
(default `[]`) is appended last, for video only. A video row is marked `▣` in the
track list before it is played.

**Reasoning:**
- **The trigger is the track, not a mode.** A playlist can hold both kinds, and
  auto-advance can walk from one to the other, so "am I in video mode" has no
  answer. `media` does, and all three spawn sites already hold the track — the
  change is one argument threaded through `spawn_player_for`.
- **`--force-window=yes`, not the absence of `--no-video`.** Dropping `--no-video`
  is not enough: mpv can decide a file has nothing worth a window and play it with
  none at all — a video row that plays sound into an empty terminal, which reads
  exactly like a bug.
- **The tty stays the TUI's.** `--no-terminal` and `--really-quiet` are not audio
  concessions to relax now that there is a window: mpv shares a terminal with
  ratatui, and one line of its output or one stolen keystroke corrupts the display.
  A window changes where mpv draws video, not who owns the tty.
- **mpv exits on an option it does not know, so a default flag is a loaded gun.**
  `--focus-on=never` is the flag we would want — a video window stealing focus
  pulls the keyboard away from the TUI — and it needs mpv 0.38. Shipping it would
  turn every older install's playback into an immediate spawn failure. So it is
  documented in the README and the user opts in.
- **The user's flags come last, and only for video.** Last because mpv takes the
  later of two conflicting options, so someone who sets `--force-window=no` gets
  what they asked for rather than fighting us. Video-only because these are
  window-management flags: on an audio track they would be at best pointless, and
  at worst — a typo is fatal to mpv — enough to stop music playing at all. The blast
  radius of a bad entry is bounded to the files that need a window.
- **`▣` before the title, not in the status column.** The status column says what
  the *file* is (cached, missing, downloading); this says what playing the row will
  *do to your screen*, which is worth knowing before the keypress rather than after
  a window has covered the terminal. It sits after an album row's indent, because an
  album's video row is still one of the album's rows.

**Trade-off accepted:** the spawn decision itself has no automated test. `mpv_args`
is covered exhaustively, but whether `spawn_player_for` passes the right `video` for
a given row is only observable by watching a real mpv — asserting it would mean
recording spawn arguments in production state purely for tests. So that one line is
verified by hand (`docs/progress.md` records it) and kept trivial enough to read.

**Out of scope:** any window behaviour we would have to implement ourselves —
placement, size, always-on-top, focus policy, a second window for a second track.
mpv already has options for all of it and `video_mpv_args` is the door.
