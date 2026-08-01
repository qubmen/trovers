# Plan: Decouple Player from Displayed Playlist

## Context

Right now `App.playlist` serves two incompatible roles at once: it's both
"the playlist currently shown in the track list" and "the source of truth for
what's playing" (via `Playlist.current_track`). Because of this, several
things are broken by design, not by accident:

0. **[Most severe, user-reported] Adding a track mid-playback hijacks
   playback and jumps it to the wrong position.** Root cause fully traced:
   `TaskMsg::MetaReady`'s active-playlist branch
   (`src/tui/mod.rs:386-392`) unconditionally does
   `self.playlist.current_track = Some(video_id.clone())` for every newly
   added track — even though nothing asked to play it, and the user is
   simply adding a link to the current playlist while track A is playing.
   This silently makes the *brand new, undownloaded* track "the current
   track" in the data model. Later, when its background download finishes,
   `TaskMsg::DownloadDone` (`src/tui/mod.rs:461-462`) checks
   `self.playlist.current_track.as_deref() == Some(&video_id)` to decide
   "should I hot-switch mpv from stream to local file" (a mechanism meant
   for a track that was *already* streaming and just finished caching).
   Because of the bogus assignment above, this check now wrongly evaluates
   `true` for the newly added track, so it calls
   `self.request_playback(idx, Some(self.position))` — but `self.position`
   at that moment is track A's live playback position (it was never track
   B's), since track A was the one actually driving `pos_tx` the whole
   time. Result: mpv is killed and restarted playing the new, unrelated
   track B, seeked to whatever second track A happened to be at. Two bugs
   compounding into one very confusing symptom: (a) adding a track must
   never change `current_track`/what's playing at all, and (b) the
   stream→file hot-switch's "is this the currently playing track" check
   must be tied to actual playback identity, not a field any code path can
   casually overwrite. Fixed as **Task 1**, standalone and before anything
   else, because it's cheap, isolated, and the scariest symptom to leave
   unfixed while other refactoring is in progress.
1. **`switch_to_playlist()`** (`src/tui/mod.rs:652-683`) forcibly stops
   playback (`self.player = None`, resets `position`, sends `0.0` on
   `pos_tx`) every time the user switches playlists in the sidebar. This
   makes it impossible to browse other playlists while something is
   playing — confirmed by the user as unwanted behavior.
2. Once player state is detached from the displayed playlist, `Now Playing`
   and the `▶` row highlight (`src/tui/ui.rs:221,388,469,523`) — which all
   currently resolve the playing track via `app.playlist.current_track` —
   need a different source of truth that survives playlist switches.

The user wants: browsing/editing/adding tracks to any playlist must never
affect what's currently playing, and playback must keep showing in Now
Playing regardless of which playlist is on screen. Additionally, adding a
track (CLI URL or the `a`/Plunder flow) must **not** auto-start playback —
this was previously assumed to be a bug in review, but the user confirmed
it's the desired behavior, so `AGENTS.md` needs correcting instead of the
code.

While auditing this area, three more real bugs were found that touch the
exact same code paths this refactor rewrites, so per user's decision they're
folded into this plan rather than done separately:

- **Resume-on-start is dead**: `request_playback()` accepts `start_pos`, but
  every call site for user-initiated playback (`Enter`/`Space`/`n`/`b` in
  `src/tui/input.rs:234,251,331,341`) passes `None`. `Track.last_position` is
  never read to resume playback — tracks always start at 0:00.
- **Position is never flushed on quit**: `last_position` is only updated
  inside `request_playback` when switching *away* from a track
  (`src/tui/mod.rs:245`), never on `q` (`main.rs:186-207` just calls
  `playlist.save()` with whatever is already in memory, which is stale).
- **`config.active_playlist` is write-only... backwards**: it's read at
  startup (`main.rs:86-87`) to decide which playlist to open, but nothing in
  the codebase ever assigns to it. Switching playlists never persists the
  choice, so `active_playlist` in `config.toml` is permanently `None`.

Also found during this investigation (real, but only loosely related —
included because they sit in the exact functions being touched):

- **`download_targets` is only populated for non-active-playlist adds**
  (`src/tui/mod.rs:383` vs the `else` branch at `386-391` which never
  inserts). Once the user can freely add tracks while browsing elsewhere,
  the "active playlist" at completion time may differ from the one at
  add-time, and the download-completion handler silently updates the wrong
  file (or the in-memory playlist that's no longer the source of truth).
  Fixed by always recording the target path at add-time, no special case.
- **Download progress is a single global `f32`/`HashSet<String>`
  (`downloading`, `download_progress` in `App`)**, not keyed reliably to
  which track's bar should show it — with multiple concurrent downloads
  (now a normal outcome of "add tracks anywhere, anytime") one track's
  progress bar can show another track's percentage, and completion of one
  download resets the percentage to 0 for a still-running one. Fixed by
  making the progress channel carry `(video_id, pct)` and `download_progress`
  a `HashMap<String, f32>`.

## Design decisions (confirmed with user)

1. **Playing-track storage**: not a lightweight metadata-only snapshot.
   Instead, `App` holds a full second `Playlist` in memory —
   `playing: Option<PlayingSession>` — where `PlayingSession` wraps the
   loaded `Playlist` the playing track belongs to, its path, and the track's
   index within it. Rationale (user's call): the playing track needs full
   `Track` data anyway (for resume, speed, cache_status transitions), and if
   the playing playlist happens to be the same file the user is currently
   browsing/editing, we want that edited *first-class*, not two silently
   diverging copies.
   - **Sync rule**: if `playing.path == app.playlist_path` (user is
     browsing the same playlist that's currently playing), any edit made
     through the track list (delete, add via non-Plunder-redirect, rename
     playlist, move track out) must go through a single shared mutation
     path so both views of "the same playlist" never diverge. Concretely:
     `App` exposes `fn active_track_mut(&mut self, video_id) -> Option<&mut Track>`-style
     helpers that check identity by path and, when they match, only ever
     mutate `app.playlist` — then `PlayingSession` is defined to *borrow its
     display data from `app.playlist` whenever paths match*, and only fall
     back to its own private `Playlist` copy when the user has switched
     away to a different playlist. See Task 2 for the exact shape.
2. **`n`/`b` navigation**: always operates on the **displayed** playlist
   (`app.playlist`), regardless of where the currently-playing track lives.
   This matches "browsing playlist X should let me just walk through X's
   tracks with n/b" even while something else plays in the background.
3. **Scope**: everything above is one plan — the add-track hijack bug,
   decoupling player from playlist, resume-on-start, flush-on-quit,
   `active_playlist` persistence, and the two download-tracking bugs —
   because all of it edits `request_playback`, `handle_task_msg`, `App`
   fields, and `save`/`quit` flow in the same handful of functions.
4. **AGENTS.md correction**: adding a track never auto-plays. This is a
   docs-only change (Task 9), no code involved.

## Files involved

- `src/tui/mod.rs` — `App` struct, `request_playback`, `handle_task_msg`,
  `switch_to_playlist`, `save_playlist`, `sync_channels`, event loop `run()`
- `src/tui/ui.rs` — `render_now_playing_header`, `render_track_info_row`,
  `render_playback_bar`, `render_track_table` (is_playing highlight),
  `effective_speed` caller
- `src/tui/input.rs` — all `request_playback` call sites, `q`/quit handling,
  `d`/`m` (delete/move) current-track checks, volume/speed handlers
- `src/playlist.rs` — `Playlist`, `Track` (need `Clone` already present;
  may need a small helper for "patch track by video_id and save")
- `src/main.rs` — startup playlist resolution (`active_playlist` read),
  exit-save flow
- `src/tui/ui_test.rs` — existing tests reference `app.playlist.current_track`
  directly and assert `switch_to_playlist` stops playback / resets
  position — these are being intentionally changed, so those tests must be
  updated (not just left failing)
- `AGENTS.md` — playlist TOML schema section, architecture diagram, CLI
  description, `App` state field list, keymap "resume from last_position"
  notes — all need small wording corrections to match new/confirmed
  behavior

## Existing patterns being reused

- Atomic save-by-path pattern already used for `download_targets` in
  `TaskMsg::DownloadDone` (`src/tui/mod.rs:427-447`): load playlist by path
  → find track by `video_id` → mutate → save. This becomes the general
  "patch a track that might not be in the currently displayed playlist"
  mechanism (Task 3), generalized instead of duplicated a third time.
- `watch::channel` pattern already used for `pos_tx`/`download_tx` — reused
  as-is, just changing the download channel's payload type (Task 7).

## Implementation Steps

### Task 1: Fix the add-track playback-hijack bug (standalone, do first)

**Files:**
- Modify: `src/tui/mod.rs`

- [x] In `TaskMsg::MetaReady`'s active-playlist branch
      (`src/tui/mod.rs:384-392`), delete the line
      `self.playlist.current_track = Some(video_id.clone());`. Adding a
      track must only push it into `tracks` and set `self.downloading` —
      it must never touch "what's playing".
- [x] Re-examine `self.selected = self.playlist.tracks.len() - 1;` on the
      same branch: leave as-is for now (moving the cursor to a freshly
      added track is a separate, milder UX question the user hasn't
      flagged) — do not fold into this fix, just confirm via test that it
      doesn't interact with playback state.
- [x] In `TaskMsg::DownloadDone` (`src/tui/mod.rs:461-462`), this task does
      NOT yet need the full `PlayingSession` from Task 2 — as an immediate
      belt-and-suspenders fix, tighten `is_current` to also require
      `self.player.is_some()` **and** that the track being completed is the
      one that was actually streaming when the download started, by
      checking `self.downloading` no longer contains other in-flight
      assumptions — practically, once the line above is removed,
      `current_track` can no longer be hijacked by a fresh add, so this
      check becomes correct again for its original purpose (a track that
      really was playing and streaming). No further change needed here in
      this task; Task 3 will replace this check's data source entirely when
      `PlayingSession` lands.
- [x] Write a regression test: playlist has track A (`current_track =
      "A"`, simulate `app.position = 137.0`), simulate adding track B via
      `handle_task_msg(TaskMsg::MetaReady { ... })` for a new `video_id =
      "B"`, then assert `app.playlist.current_track.as_deref() ==
      Some("A")` (unchanged). Then simulate
      `handle_task_msg(TaskMsg::DownloadDone { video_id: "B", .. })` and
      assert no hot-switch is triggered for B (e.g. assert `app.position`
      is untouched / `current_track` still `"A"` / — since `request_playback`
      spawns a real tokio task, assert on the *decision*, i.e. that the
      `is_current` condition evaluates false, by checking state right after
      the call rather than mocking the spawn).
- [x] Write a second regression test mirroring the exact user report: track
      A playing, non-zero `app.position`, add track B, fire `DownloadDone`
      for B — assert `app.playlist.current_track` is still `"A"`, proving
      the specific "jumps to new track at old track's timestamp" scenario
      is closed.
- [x] Run `cargo test` — must pass before Task 2.

### Task 2: Introduce `PlayingSession` and decouple it from `switch_to_playlist`

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/ui_test.rs`

- [x] Add `pub struct PlayingSession { pub path: PathBuf, pub playlist: Playlist, pub track_idx: usize }`
      near `App` in `src/tui/mod.rs`. `playlist` is a full loaded `Playlist`
      (already `Clone`, no derive changes needed on `Playlist`/`Track`).
- [x] Add `pub playing: Option<PlayingSession>` field to `App`, initialized
      to `None` in `App::new`.
- [x] Add helper `impl PlayingSession { fn track(&self) -> &Track; fn track_mut(&mut self) -> &mut Track }`
      indexing `self.playlist.tracks[self.track_idx]`.
- [x] Add `impl App { fn playing_track(&self) -> Option<&Track> }` — returns
      `self.playing.as_ref().map(|p| p.track())`, but if
      `self.playing.as_ref().map(|p| &p.path) == Some(&self.playlist_path)`,
      returns the track from `self.playlist` instead (found by matching
      `video_id`), so edits made to the currently-displayed-and-playing
      playlist are always reflected immediately without needing a manual
      sync step. Add the mutable counterpart `fn playing_track_mut`.
- [x] Rewrite `request_playback(&mut self, idx: usize, start_pos: Option<f64>)`
      to build a `PlayingSession` from `self.playlist.clone()` (path =
      `self.playlist_path.clone()`, `track_idx = idx`) instead of mutating
      `self.playlist.current_track`. Remove all direct
      `self.playlist.current_track = ...` assignments from this function.
      `PlayerReady`/`PlayerError` messages in `handle_task_msg` keep
      matching on `video_id`, but check against `self.playing` instead of
      `self.playlist.current_track`.
- [x] Update `current_track_index(&self)` → keep for displayed-playlist
      cursor logic (used by `n`/`b`, see Task 4), but it must now read
      `self.playlist.current_track` only for restoring cursor position on
      load — decouple it from "what's playing" naming-wise: rename to
      `fn track_marked_current_index(&self) -> Option<usize>` to avoid
      confusion with the new `playing` concept, OR simplify by having
      `Playlist.current_track` mean strictly "last track selected/played in
      *this* playlist file, used only to restore cursor on load" — keep it
      being written whenever the user explicitly plays a track that lives
      in the currently displayed playlist (i.e. when `path == playlist_path`).
      (Kept the name `current_track_index`, documented its narrower meaning,
      and `request_playback` still writes `self.playlist.current_track`
      since `idx` always indexes the displayed playlist there.)
- [x] `switch_to_playlist()`: **delete** the three lines that stop playback
      (`self.player = None`, `self.position = 0.0`,
      `let _ = self.pos_tx.send(0.0)`, and `self.is_paused = false`). Loading
      the new playlist, resetting selection/scroll/search, and focusing the
      track list all stay as-is. `self.player`/`self.playing`/`self.position`
      must remain untouched by this function after the change.
- [x] Update `switch_to_playlist_stops_playback`,
      `switch_to_playlist_clears_paused_state`,
      `switch_to_playlist_resets_position_to_zero` in `src/tui/ui_test.rs`
      (lines ~2003, 2496, 2508) to assert the **opposite**: switching
      playlists must NOT touch `is_paused`/`position`/`player`. Rename tests
      accordingly (e.g. `switch_to_playlist_does_not_stop_playback`).
- [x] Write new tests: playing session survives `switch_to_playlist` (set
      `app.playing = Some(...)` with a fake session, switch playlists, assert
      `app.playing` is still `Some` and unchanged); `playing_track()` returns
      the live-edited track when `playing.path == playlist_path` and the
      user mutates `app.playlist.tracks` directly (simulating what delete/
      rename would do) — assert the getter reflects the edit without any
      extra sync call.
- [x] Run `cargo test` — must pass before Task 3.

### Task 3: Route all track/playlist mutations through path-aware patch helper

**Files:**
- Modify: `src/tui/mod.rs`

- [x] Add `fn patch_and_save_playlist(&mut self, path: &Path, video_id: &str, f: impl FnOnce(&mut Track))`
      to `App`: if `path == self.playlist_path`, mutate the track in
      `self.playlist.tracks` directly then call `self.save_playlist()`;
      otherwise `Playlist::load(path)` → find by `video_id` → mutate → save
      to `path`, logging + returning early on any load/save error (same
      error handling style already used in `DownloadDone`'s target-playlist
      branch, `src/tui/mod.rs:429-444`).
- [x] Replace the duplicated load/find/mutate/save block in
      `TaskMsg::DownloadDone` (`src/tui/mod.rs:420-469`, both the
      `download_targets` branch and the active-playlist branch) with a
      single call to `patch_and_save_playlist`, setting `cache_status` and
      `file`. After the patch, keep the existing "if this track is the one
      currently playing and it's a stream, hot-switch to local file" logic,
      but base the "is this the currently playing track" check on
      `self.playing.as_ref().map(|p| &p.track().video_id) == Some(&video_id)`
      instead of `self.playlist.current_track` (this fully replaces the
      belt-and-suspenders check from Task 1 with the real fix). Also fixed
      `TaskMsg::MetaReady` to always populate `download_targets` at add-time
      (not just for non-active-playlist adds), so `DownloadDone` never has to
      guess the owning playlist's path from whatever happens to be displayed
      when the download completes. Added `hot_switch_to_local_file` and
      `spawn_player_for` helpers (extracted from `request_playback`) so the
      hot-switch path shares the "spawn mpv" logic without re-triggering the
      leaving-track position-save/`current_track` bookkeeping that
      `request_playback` does.
- [x] Update speed-change handlers in `src/tui/input.rs` (`]`/`[` keys,
      lines ~271-296) to mutate through `playing_track_mut()` when adjusting
      the speed of the *playing* track, not `app.current_track_index()` +
      `app.playlist.tracks[idx]` (today's code assumes the playing track is
      always in the displayed playlist — no longer true). Extracted into a
      shared `adjust_playing_track_speed` helper that persists through
      whichever copy is the source of truth (displayed playlist if paths
      match, else the playing session's own playlist file).
- [x] Write tests for `patch_and_save_playlist`: patch when path matches
      displayed playlist (in-memory + saved to disk), patch when path is a
      different playlist (round-trips through disk, displayed playlist
      untouched), patch with a video_id that doesn't exist (no-op, no
      error/log-only).
- [x] Write/update tests for `DownloadDone` using the new helper — reuse
      existing `download_done_updates_non_active_playlist_on_disk` and
      `download_done_for_active_playlist_updates_in_memory_state`
      (`ui_test.rs:2794`, `2844`) as the behavioral contract; they should
      keep passing unmodified if the helper is correct (add one more test:
      download completes for the *playing* track when the user has switched
      to browsing a different playlist — assert the hot-switch to local file
      still occurs even though `app.playlist` is no longer that track's
      playlist). Confirmed both pre-existing tests still pass unmodified;
      added `download_done_hot_switches_playing_track_even_when_browsing_elsewhere`
      as the new one.
- [x] Run `cargo test` — must pass before Task 4. (240 passed, 0 failed.)

### Task 4: Rewrite Now Playing / track-highlight rendering to use `app.playing`

**Files:**
- Modify: `src/tui/ui.rs`

- [x] `render_now_playing_header`, `render_track_info_row`,
      `render_playback_bar` (`src/tui/ui.rs:381-409, 462-483, 518-557`):
      replace the `app.playlist.current_track` + `.tracks.iter().position(...)`
      lookup with `app.playing_track()` (from Task 2). `effective_speed`
      call in the header (`ui.rs:396`) takes a `&Playlist` for
      `default_speed` fallback — pass `app.playing.as_ref().map(|p| &p.playlist)`
      so the fallback speed comes from the *playing* playlist's
      `default_speed`, not the displayed one.
- [x] `render_track_table` (`ui.rs:217-268`): change the `is_playing` check
      (line 221) to: `app.playing.as_ref().is_some_and(|p| p.path == app.playlist_path && p.track().video_id == track.video_id)`
      — only highlight `▶` when the playing track actually belongs to the
      currently displayed playlist file. (Extracted as `pub(crate) fn
      row_is_playing(app: &App, video_id: &str) -> bool` for testability.)
- [x] Write tests: Now Playing renders the playing track's title even when
      `app.playlist` (displayed) is a different playlist with different
      tracks (construct `app.playing` pointing at playlist A, set
      `app.playlist`/`app.playlist_path` to playlist B, assert
      `render_track_info_row`'s underlying data — test via
      `build_track_info_line`/`playing_track()` directly since rendering
      itself isn't unit-tested per existing convention in this file).
      Write a test that `render_track_table`'s `is_playing` condition (test
      the extracted predicate as a small pure function if convenient, or
      via `app.playing`/`app.playlist_path` state assertions) is `false`
      when paths differ even if `video_id` coincidentally matches. (Added
      `playing_track_shows_data_from_unrelated_displayed_playlist`,
      `row_is_playing_false_when_paths_differ_even_with_matching_video_id`,
      `row_is_playing_true_when_paths_and_video_id_match`,
      `row_is_playing_false_when_nothing_playing`.)
- [x] Run `cargo test` — must pass before Task 5. (244 passed, 0 failed.)

### Task 5: `n`/`b` operate on displayed playlist; delete/move guard on playing-session identity

**Files:**
- Modify: `src/tui/input.rs`

- [x] Rewrite `n`/`b` handlers (`src/tui/input.rs:324-343`): cursor
      (`cur`) must come from `app.selected`/`app.track_index_at`, not
      `app.current_track_index()` — i.e. `n`/`b` always step relative to the
      cursor position in the **displayed** playlist, wrapping at its bounds,
      and call `request_playback` on the resulting displayed-playlist index.
      This matches the confirmed behavior: browsing playlist X and pressing
      `n`/`b` walks X's tracks regardless of what's playing.
- [x] `Space` handler (`input.rs:239-254`): "toggle pause if playing"
      must check `app.player.is_some()` (unchanged) but "otherwise start"
      should still fall back to `app.track_index_at(app.selected)` — remove
      the `app.current_track_index()` fallback since it no longer means
      "the track in *this* playlist that's playing" reliably. Verify by
      reading the updated logic against: user has nothing playing, browsing
      playlist X, presses Space on a track → plays that track. Unchanged
      behavior expected here, just simplifying away a now-misleading path.
- [x] `handle_confirm_delete` (`input.rs:535-566`) and
      `move_track_to_playlist` (`src/tui/mod.rs:701-767`): the "is this the
      currently playing track" check must become
      "`app.playing` matches this `video_id` **and** `playing.path == app.playlist_path`"
      — i.e. deleting/moving a track only stops playback if the track being
      removed is literally the one playing right now, not just any track
      with a matching id that happens to exist in a differently-playing
      session. Update both call sites to use a shared helper, e.g.
      `App::is_playing_track(&self, path: &Path, video_id: &str) -> bool`.
      (Also clears `app.playing = None` in both call sites when the guard
      trips, since `PlayingSession` — not just `player`/`current_track` — is
      now the source of truth for "what's playing".)
- [x] Write tests: `n`/`b` step through displayed playlist tracks while
      `app.playing` points at an unrelated playlist (assert selection moves
      within displayed playlist bounds, `request_playback` is invoked with
      displayed-playlist indices). Write a test that deleting a track in
      playlist B does not clear `app.player`/`app.playing` when the actually
      playing track lives in playlist A (even if by coincidence video_ids
      matched — use distinct ids to keep the test meaningful, then add one
      more test with intentionally colliding ids across two playlists to
      prove the path check is what protects it). Added
      `n_steps_cursor_within_displayed_playlist_ignoring_unrelated_playing_session`,
      `n_wraps_to_first_track_at_end_of_displayed_playlist`,
      `b_steps_cursor_backward_within_displayed_playlist`,
      `b_wraps_to_last_track_at_start_of_displayed_playlist`,
      `n_is_noop_on_empty_displayed_playlist`,
      `space_falls_back_to_cursor_track_when_nothing_playing`,
      `is_playing_track_true_for_exact_path_and_video_id_match`,
      `is_playing_track_false_when_video_id_matches_but_path_differs`,
      `delete_does_not_stop_playback_for_colliding_video_id_in_different_playlist`,
      `delete_stops_playback_when_deleting_the_actually_playing_track`,
      `move_track_does_not_stop_playback_for_colliding_video_id_in_different_playlist`.
      Updated the pre-existing `move_track_stops_playback_when_moving_current_track`
      to also set up `app.playing` (now the real identity source of truth)
      and assert it's cleared.
- [x] Run `cargo test` — must pass before Task 6. (255 passed, 0 failed.)

### Task 6: Resume playback from `last_position`

**Files:**
- Modify: `src/tui/input.rs`

- [x] At all user-initiated `request_playback` call sites in
      `src/tui/input.rs` (`Enter` line ~234, `Space` fallback line ~251,
      `n` line ~331, `b` line ~341), read the target track's
      `last_position` before calling `request_playback` and pass
      `Some(track.last_position as f64)` when `> 0`, else `None`. This
      mirrors what `TaskMsg::DownloadDone`'s stream→file hot-switch already
      does at `src/tui/mod.rs:466` (`Some(pos)`), just sourcing the value
      from the track's persisted field instead of live `app.position`.
      (Added `pub(crate) fn resume_start_pos(track: &Track) -> Option<f64>`
      and used it at all four call sites via
      `app.playlist.tracks.get(idx).and_then(resume_start_pos)`.)
- [x] Write tests: a track with `last_position = 90` played via the `Enter`
      path results in `request_playback` being called with `Some(90.0)`
      (test by asserting on the arguments — if `request_playback` isn't
      easily mockable, test the resume-position *selection* logic as an
      extracted pure function, e.g. `fn resume_start_pos(track: &Track) -> Option<f64>`,
      and unit test that directly: `0` → `None`, `90` → `Some(90.0)`).
  ⚠️ Note: `request_playback` spawns a real `tokio::spawn` calling
  `ytdlp::get_stream_url`/`Player::spawn` — existing tests never exercise
  this path end-to-end (confirmed: no `request_playback` tests exist in
  `ui_test.rs` today). Keep testing at the pure-logic level as the rest of
  this codebase does; don't introduce process-spawning tests.
      (Added `resume_start_pos_returns_none_for_zero_last_position`,
      `resume_start_pos_returns_some_for_nonzero_last_position` for the pure
      function directly, plus behavioral tests
      `enter_resumes_from_last_position_via_request_playback_arg`,
      `enter_starts_fresh_when_last_position_is_zero`,
      `n_resumes_next_track_from_its_last_position`,
      `b_resumes_previous_track_from_its_last_position`,
      `space_resumes_from_last_position_when_nothing_playing` that assert on
      the observable side-effect of `start_pos` being `Some`/`None` — namely
      whether `app.position` gets reset to 0.0 by `request_playback`.)
- [x] Run `cargo test` — must pass before Task 7. (262 passed, 0 failed.)

### Task 7: Flush position on quit + per-track download progress

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/main.rs`
- Modify: `src/tui/ui.rs`
- Modify: `src/ytdlp.rs`

- [x] Add `App::flush_playing_position(&mut self)`: if `self.playing` is
      `Some`, set `playing_track_mut().last_position = self.position as u64`,
      then save via `patch_and_save_playlist`-equivalent logic (or, simpler,
      since `PlayingSession` owns a full `Playlist`, just call
      `self.playing.as_ref().unwrap().playlist.save(&path)` after mutating
      — but route through the same in-memory-vs-disk identity rule as Task 2
      so an in-progress edit to the displayed playlist isn't clobbered).
- [x] Call `app.flush_playing_position()` right before
      `app.playlist.save(&app.playlist_path)` in `src/main.rs` (~line 207),
      inside `run()`'s quit path in `src/tui/mod.rs`, or both — pick
      whichever guarantees it runs exactly once on every quit path (`q` from
      sidebar/tracklist/settings all set `should_quit`/return `Action::Quit`,
      confirmed single exit point in `run()` at `mod.rs:799-821`). Prefer
      placing it in `run()` right before `ratatui::restore()`.
      (Placed in `run()` in `src/tui/mod.rs`, immediately before
      `ratatui::restore()` — the single confirmed exit point covering every
      quit path.)
- [x] On `App::new`, when `playlist.current_track` resolves to a track in
      the just-loaded playlist, also populate `app.playing` with a
      `PlayingSession` for that track *without* spawning a player (so Now
      Playing can show "last played, not currently running" — actually,
      simplify: only pre-populate `app.playing` if a genuine resume-on-launch
      feature is wanted; since none was requested, skip this and leave
      `app.playing = None` on startup — Now Playing shows "No track
      selected" until the user picks something, which matches current
      startup behavior). ⚠️ Decision: do NOT pre-populate `playing` on
      startup — out of scope, avoids scope creep. (Confirmed already the
      case: `App::new` leaves `playing: None`; no code change needed, as
      documented by the plan's own scope decision.)
- [x] Change `download_tx`/`download_rx` in `App` from `watch::Sender<f32>`/
      `Receiver<f32>` to `watch::Sender<(String, f32)>`/`Receiver<(String, f32)>`
      (video_id, percent). Update `ytdlp::spawn_download`'s `progress_tx`
      parameter type and its one send call (`src/ytdlp.rs:99,137`) to include
      `video_id.to_string()`.
- [x] Change `App.download_progress: f32` → `download_progress: HashMap<String, f32>`.
      Update `sync_channels` (`mod.rs:327-329`) to
      `.insert(video_id, pct)` on change. On `DownloadDone`/`DownloadError`,
      remove the entry for that `video_id` instead of sending a global
      `0.0` reset (delete the `let _ = self.download_tx.send(0.0)` lines at
      `mod.rs:423`; the `downloading.remove(&video_id)` already scopes
      correctly, just also `self.download_progress.remove(&video_id)`).
- [x] Update `render_playback_bar`/`build_playback_bar_line` call site in
      `src/tui/ui.rs` (~line 533,538) to read
      `app.download_progress.get(&track.video_id).copied().unwrap_or(0.0)`
      instead of the bare `app.download_progress`.
- [x] Write tests: two tracks downloading concurrently, sending progress for
      one does not affect the other's stored percentage; completing one
      download removes only its entry, leaving the other's percentage
      intact (`ui_test.rs`, follow existing `HashMap`-based test style already
      used for `downloading: HashSet`). (Added
      `download_progress_is_tracked_per_video_id`,
      `download_done_removes_only_its_own_progress_entry`,
      `download_error_removes_only_its_own_progress_entry`.)
- [x] Write test for `flush_playing_position`: playing track's
      `last_position` updates in the saved TOML file after calling it with
      a non-zero `app.position`. (Added
      `flush_playing_position_persists_to_disk_for_displayed_playlist`,
      `flush_playing_position_persists_to_disk_for_unrelated_playlist`,
      `flush_playing_position_is_noop_when_nothing_playing`.)
- [x] Run `cargo test` — must pass before Task 8. (268 passed, 0 failed.)

### Task 8: Persist `active_playlist` on switch

**Files:**
- Modify: `src/tui/mod.rs`

- [x] In `switch_to_playlist()`, after successfully replacing
      `self.playlist`/`self.playlist_path`, set
      `self.config.active_playlist = Some(name.to_string())` and call
      `self.config.save()` (mirror the existing fire-and-forget
      `let _ = self.config.save()` pattern already used for volume/speed/
      quality changes in `src/tui/input.rs:301,308,431,439`).
- [x] Also set `active_playlist` once at startup in `src/main.rs` right
      after the playlist is resolved (~line 165, after the `match` block),
      in case it was previously unset or pointed at a since-renamed/deleted
      playlist — keeps config in sync with whatever actually got opened,
      then `config.save()` (this can piggyback on the existing
      `app.config.save()` call at exit, or save immediately — prefer saving
      immediately so a crash before exit still leaves the config accurate).
      (Implemented in `src/main.rs` right after the `info!(playlist = ...)`
      log line: only writes+saves when `config.active_playlist` doesn't
      already match the resolved playlist's name, avoiding a redundant save
      on every launch.)
- [x] Write tests: `switch_to_playlist` updates `app.config.active_playlist`
      to the new playlist's name; startup resolution (if testable without
      spawning the full binary — likely needs a small extraction of the
      "resolve playlist_name" logic out of `main.rs` into a testable
      function, e.g. `fn resolve_playlist_selection(cli: Option<&str>, config: Option<&str>, existing: &[PathBuf]) -> ...` —
      only do this extraction if it doesn't balloon scope; otherwise settle
      for testing `switch_to_playlist`'s config update and leave startup
      resolution covered by existing/manual testing). (Per the plan's own
      caveat, skipped the `resolve_playlist_selection` extraction to avoid
      ballooning scope — startup resolution remains covered by
      existing/manual testing. Added
      `switch_to_playlist_updates_config_active_playlist`,
      `switch_to_playlist_updates_config_active_playlist_across_multiple_switches`,
      `switch_to_playlist_does_not_update_config_active_playlist_on_error`
      in `src/tui/ui_test.rs` covering `switch_to_playlist`'s config update.)
- [x] Run `cargo test` — must pass before Task 9. (271 passed, 0 failed.)

### Task 9: Correct AGENTS.md

**Files:**
- Modify: `AGENTS.md`

- [x] Line 16: change "the tool immediately starts playing the audio and
      simultaneously downloads..." to reflect that adding a track only adds
      + downloads in the background; playback is always user-initiated.
- [x] Line 91 (architecture diagram) and line 153 (CLI usage comment):
      same correction — "add track to current playlist and start playing"
      → "add track to current playlist and start caching (no auto-play)".
- [x] Add a short explicit note near the `ytdlp.rs`/`fetch_url` description
      (or under "Out of Scope") stating adding a track never auto-plays and
      never changes what's currently playing — by design, confirmed by
      product owner — to prevent this from being "fixed" again in a future
      pass.
- [x] Update `App::switch_to_playlist(name, path)` bullet (line ~501): remove
      "pauses playback" — it must now explicitly say playback is
      **unaffected** by playlist switches.
- [x] Update the "Now Playing" / playback-state description and any
      `current_track` references in the `App` struct field list (~lines
      508-541) to mention the new `playing: Option<PlayingSession>` field
      and what it represents, replacing the outdated assumption that the
      playing track always lives in `app.playlist`.
- [x] No tests needed (docs-only task) — just proofread the diff against
      the actual final code from Tasks 1-8 before committing.

### Task 10: Verify acceptance criteria

- [x] manual test (skipped - not automatable, covered by automated
      regression tests added in Task 1: `handle_task_msg`-based tests
      asserting `app.playlist.current_track` stays `"A"` after adding track
      B and after `DownloadDone` fires for B; and Task 3's
      `download_done_hot_switches_playing_track_even_when_browsing_elsewhere`,
      which proves A keeps playing unaffected by B's background download
      completing.) Manually confirm via `cargo run` (see Post-Completion)
      that: playing track A mid-song, adding a new track B to the same
      playlist does not change what's playing, does not move the playback
      position, and once B finishes downloading in the background, A is
      still the one playing, unaffected.
- [x] manual test (skipped - not automatable, covered by automated
      regression tests added in Tasks 2, 4, and 5: `switch_to_playlist_does_not_stop_playback`,
      `switch_to_playlist_does_not_reset_position`,
      `switch_to_playlist_does_not_clear_paused_state` (Task 2) prove
      switching playlists never stops audio; `playing_track_shows_data_from_unrelated_displayed_playlist`
      and the `row_is_playing_*` tests (Task 4) prove Now Playing/highlight
      correctly track the playing session independent of the displayed
      playlist; `n_steps_cursor_within_displayed_playlist_ignoring_unrelated_playing_session`
      and `delete_does_not_stop_playback_for_colliding_video_id_in_different_playlist`
      (Task 5) prove browsing/editing/`n`/`b` on a different playlist never
      touches playback.) Manually confirm: playing track A, switching
      sidebar to playlist B does not stop audio; Now Playing keeps showing
      track A's info/progress while browsing B; deleting/adding tracks in B
      never touches playback; `n`/`b` while browsing B walk B's tracks;
      switching back to A shows the `▶` highlight again on the right row.
- [x] manual test (skipped - not automatable, covered by automated
      regression tests added in Task 6: `resume_start_pos_returns_some_for_nonzero_last_position`
      and the behavioral tests `enter_resumes_from_last_position_via_request_playback_arg`,
      `n_resumes_next_track_from_its_last_position`,
      `b_resumes_previous_track_from_its_last_position` prove the
      resume-position selection logic and its wiring into
      `request_playback` call sites.) Verify resume: play a track partway,
      switch tracks, come back via `Enter` — it resumes near the last
      position (allow a few seconds of drift from polling interval).
- [x] manual test (skipped - not automatable, covered by automated
      regression tests added in Task 7:
      `flush_playing_position_persists_to_disk_for_displayed_playlist` and
      `flush_playing_position_persists_to_disk_for_unrelated_playlist`
      prove `last_position` is written to the on-disk TOML before quit, and
      Task 6's resume tests prove that value is read back on the next
      `Enter`.) Verify quit: play a track partway, press `q`, restart the
      app, `Enter` on the same track — resumes near where it left off.
- [x] manual test (skipped - not automatable, covered by automated
      regression tests added in Task 8:
      `switch_to_playlist_updates_config_active_playlist` and
      `switch_to_playlist_updates_config_active_playlist_across_multiple_switches`
      prove `config.active_playlist` is persisted on every switch; startup
      resolution itself was explicitly left to manual/existing coverage per
      Task 8's own scope decision.) Verify `active_playlist`: switch
      playlists, quit, relaunch with no CLI args — the last-active playlist
      opens, not the alphabetically first one.
- [x] manual test (skipped - not automatable, covered by automated
      regression tests added in Task 7:
      `download_progress_is_tracked_per_video_id`,
      `download_done_removes_only_its_own_progress_entry`, and
      `download_error_removes_only_its_own_progress_entry` prove concurrent
      downloads' percentages are tracked independently per `video_id` and
      completion of one never resets or clobbers another's entry.) Verify
      concurrent downloads: add two tracks in different playlists in quick
      succession, confirm their progress bars (when each is the
      playing/displayed track) don't cross-contaminate percentages.
- [x] Run full test suite: `cargo test`. Result: 271 passed; 0 failed; 0
      ignored.
- [x] Run `cargo build` to confirm no warnings introduced (project has no
      CI config found; treat `cargo build`/`cargo test` clean as the bar).
      Result: build succeeds with exactly 3 warnings, all pre-existing
      dead-code warnings noted in earlier task summaries (`audio_path` in
      `src/cache.rs:21`, `get_position` in `src/player.rs:120`,
      `is_downloading` in `src/tui/mod.rs:730`) — no new warnings
      introduced.

### Task 11: [Final] Update documentation

- [x] Update `docs/progress.md` rows affected by this change (Integration
      section: playlist switching, position polling → TOML on quit) to
      reflect the new decoupled design.
- [x] Add a new `docs/decisions.md` ADR entry documenting: why the playing
      track is stored as an independent `PlayingSession` rather than
      resolved through the displayed playlist, why `n`/`b` intentionally
      operate on the displayed playlist rather than the playing one, and
      the root cause of the add-track playback-hijack bug (so it isn't
      reintroduced by a future "convenience" change to `MetaReady`).
- [x] Move this plan to `docs/plans/completed/`.

## Post-Completion
*Manual verification only — no external systems involved.*

- Manually exercise the full browse-while-playing workflow end-to-end in a
  real terminal (`cargo run`) with actual `yt-dlp`/`mpv` installed, since
  `request_playback`'s process-spawning path has no automated test coverage
  in this codebase (confirmed pattern — tests stop at pure-logic level).
- Spot-check behavior with a genuinely large playlist (50+ tracks) and 5+
  playlists to make sure scrolling/highlight logic in Task 4 doesn't
  regress performance or visual correctness.
