# Plan: trovers Stabilization

## Status — 2026-08-15

All four phases are implemented, one commit each, with `cargo test` and
`cargo clippy` green after every phase. The plan stays here rather than in
`docs/plans/completed/` until the **manual smoke checklist** at the bottom has
been walked through against a real terminal — automated tests cover the logic,
but "the terminal is clean and `pgrep mpv` is empty" can only be confirmed by
hand.

Where the work departed from the plan as written:

- **Phase 3, EOF detection.** The plan hedged between polling `eof-reached` and
  falling back to `percent-pos`. Neither was needed: mpv runs without `--idle`
  and `--keep-open`, so its own exit is already the end-of-track signal, and it
  was already being detected. Confirmed against real mpv before building on it.
  The guard that makes it safe is a position check, not the signal itself — see
  ADR-013.
- **Phase 3, bug C.** "Move IPC off the render path *if the UI still stutters*."
  Reading the code found a worse and more specific problem than stutter: an mpv
  that accepted a connection and then went quiet parked the awaiting future
  forever — the whole UI frozen with no way out, and a position poller that could
  never report `PlayerGone` again. A 2s timeout fixes that outright and far more
  cheaply than restructuring the loop. The same read turned up a second bug the
  plan had not spotted: mpv pushes unsolicited event lines to every client, and
  the first line after a command was being taken as its reply.
- **Bug F** (socket path in docs) was folded into Phase 3 rather than Phase 4,
  since Phase 3 rewrote the surrounding `AGENTS.md` section anyway.
- **Bug H** (`truncate` measuring chars rather than display width) was left
  alone: cosmetic, and it affects column alignment for CJK/emoji titles only.
- A false failure in Phase 1's own tests was found and fixed along the way: the
  three `real_mpv_*` tests shared `/tmp` and this process's pid, so run the
  documented way they interfered with each other. They now serialize on a mutex.

## Context

Three user-reported symptoms, plus "a bunch of other small bugs":

1. The app crashes periodically.
2. Sometimes mpv keeps playing after the app has died.
3. Adding a track while another one plays makes the new track start at the
   *previous* track's timestamp.

The working tree also holds an **abandoned, non-compiling WIP** (8 errors) from a
previous session that was mid-way through Phase 1 of an earlier stabilization
plan, with pieces of Phases 2–3 pulled in early. Step 0 below deals with it.

Root causes for all three symptoms are traced below. Note that two of the three
are *not* what they look like:

- Symptom 3 is **not** auto-play-on-add. `AGENTS.md:731` states auto-play on add
  is intentionally absent and must not be "fixed". It is a stale-position bleed.
- Symptom 1 is **not** a panic. `ratatui::init()` already installs a panic hook
  (`ratatui-0.30.2/src/init.rs:398`) that restores the terminal, so panics are
  already handled. The crash is an `Err` return propagated out of the event loop.

---

## Verified root causes

### Symptom 1 — periodic crash → clean `Err` exit out of the event loop

A four-link chain, each link confirmed in the code:

1. mpv is spawned with no `--idle` and no `--keep-open` (`src/player.rs:34-38`),
   so it **exits by itself** when a track reaches its end. There is no
   auto-advance to keep it alive.
2. **Nothing clears `app.player` when mpv exits on its own.** The only writers of
   `player = None` are explicit user actions (`src/tui/mod.rs:365`, `:913`,
   `src/tui/input.rs:548`, `:749`). `poll_position_loop` notices the socket is
   gone and just `break`s (`src/player.rs:170`) — it notifies nobody.
3. So `app.player` stays `Some`, pointing at a dead socket. The UI still renders
   "▶ Playing".
4. Pressing `v`, `V`, `[` or `]` calls `send_command` → `UnixStream::connect`
   fails → `?` propagates. **These paths still use `?` even in the WIP:**
   `src/tui/input.rs:249`, `:252`, `:260`, `:267`, `:376`. `handle_key` returns
   `Err` → `run()` returns `Err` → `main` returns `Err` → process exits.

Consequences: at `HEAD` there is no `TerminalGuard`, so the terminal is left in
raw mode + alternate screen — indistinguishable from a crash. And because `main`
returns early, `app.playlist.save()` / `app.config.save()`
(`src/main.rs:216-217`) never run, so **playlist edits from that session are
lost**.

### Symptom 2 — orphaned mpv → no `kill_on_drop`, no signal handling

`grep` confirms **neither `kill_on_drop` nor any signal handler exists anywhere
in `src/`**. Three distinct leak paths:

1. **No `kill_on_drop(true)`** on the mpv `Command` (`src/player.rs:33`).
   `Player::drop` (`:225`) only helps once the `Player` struct is fully built.
   Inside `Player::spawn` there is an up-to-1s socket-wait loop (`:50-61`) during
   which `Child` is a bare local. If the tokio runtime tears down then (quit),
   the future is dropped, `Child` is dropped, and `tokio::process::Child` without
   `kill_on_drop` **detaches the process instead of killing it**. Orphan mpv.
2. **No SIGINT/SIGTERM/SIGHUP handling.** Ctrl+C, or closing the terminal
   window, terminates the process without unwinding → no `Drop` → orphan mpv.
   This is almost certainly the everyday cause.
3. Same applies to the yt-dlp children in `src/ytdlp.rs`.

### Symptom 3 — new track resumes at the previous track's position

At `HEAD`:

1. `request_playback` **never kills the outgoing mpv** — there is no
   `stop_player()` at `HEAD` at all. The old process and its poller stay alive.
2. `poll_position_loop` at `HEAD` has no generation guard, so it keeps pushing
   the **old** track's `time-pos` into `pos_tx` → `app.position`.
3. `request_playback` only resets `self.position` when `start_pos.is_none()`.
   Playing a track that has a `last_position` passes `Some(..)`, so
   `app.position` keeps holding the outgoing track's value.
4. When the new track's download finishes, `hot_switch_to_local_file`
   (`src/tui/mod.rs:514`) respawns mpv with `--start={self.position}` — the new
   track literally jumps to the old track's timestamp.

The WIP addresses links 1–3 (`stop_player()` + `player_generation` + an
unconditional position reset). That work is correct in direction and worth
re-landing.

---

## Additional bugs found (not in the earlier plan)

| # | Bug | Location |
|---|-----|----------|
| A | Download progress bar never moves. Progress is parsed from **stderr**, but yt-dlp writes progress to **stdout**, which is `Stdio::null()`. Also missing `--newline`, so progress uses `\r` and `lines()` never yields. `AGENTS.md:475` asserts the wrong stream — **the spec itself is wrong**; fix code *and* doc. | `src/ytdlp.rs:122-140` |
| B | Deleting a track removes `audio_dir/<video_id>.opus`, which is **shared** by any other playlist holding the same `video_id` — silently destroys that playlist's cached copy. | `src/tui/input.rs:563-567` |
| C | `handle_key` awaits mpv IPC **inside the render loop**, so a hung socket freezes the whole UI. | `src/tui/input.rs` |
| D | Dead code in the WIP: `clear_download_state`, `remap_download_targets`, `rebuild_shuffle_order` are defined and never called; `TaskMsg::TrackEnded` is never constructed. | `src/tui/mod.rs` |
| E | `get_stream_url` returns all of stdout trimmed; take the first line only. | `src/ytdlp.rs:79-82` |
| F | Doc drift: `AGENTS.md:485` says socket is `/tmp/trovers-<pid>.sock`; code uses `-<pid>-<seq>.sock`. | `AGENTS.md` |
| G | Leftover `agent_log` debug fn writing to a hardcoded `.cursor/` path. | `src/main.rs:16-45` |
| H | `truncate` is char-safe (no panic) but measures `char` count, so CJK/emoji titles misalign columns. Cosmetic. | `src/tui/ui.rs:1318` |

`truncate` and the overlay popup rects were both checked for panics and are
**safe** — `Clear` and `Buffer::set_style` intersect with the buffer area. Do not
spend time there.

---

## Step 0 — restore a green build

The build is broken; nothing can be verified until it isn't.

1. `git branch wip/eof-generations` (or `git stash`) to preserve the WIP —
   it is the only record of the symptom-3 fix. **Do this before resetting.**
2. `git checkout -- src/` to return to `HEAD`, which compiles.
3. Confirm baseline: `cargo test` (≈100 existing tests in `src/tui/ui_test.rs`)
   and `cargo clippy` both green.

Then re-land the work phase by phase, lifting the good parts out of
`wip/eof-generations` with a test for each. Do **not** re-apply the WIP wholesale.

---

## Phase 1 — P0: stop crashing, stop orphaning mpv

Files: `src/player.rs`, `src/tui/mod.rs`, `src/tui/input.rs`, `src/main.rs`

- **Never `?` on mpv IPC.** Convert all five sites (`input.rs:249,252,260,267,376`)
  to log-and-continue, matching the pattern the WIP already used for
  `pause`/`resume`/`seek`.
- **Detect mpv exiting on its own.** When `poll_position_loop` breaks because the
  socket is gone, send a `TaskMsg::PlayerGone { generation }`; the handler clears
  `app.player`, sets `is_paused = false`, and leaves `app.playing` alone. This
  removes the crash *trigger*, not just the crash.
- **`stop_player()` + `player_generation`** — lift from the WIP
  (`mod.rs:364-367`), call before every `spawn_player_for`, and add the
  generation guards in `spawn_player_for` and `poll_position_loop`.
- **`kill_on_drop(true)`** on the mpv `Command` and on both yt-dlp `Command`s.
- **Signal handling.** A `tokio::signal` task for SIGINT/SIGTERM/SIGHUP that sets
  `app.should_quit`, so the normal shutdown path (flush position → save → kill
  mpv → restore terminal) runs instead of being skipped.
- **Startup reaper.** Scan `/tmp/trovers-*.sock`; for each, try to connect and
  send `{"command":["quit"]}`, then unlink. Self-heals leftovers from any past
  hard kill — the safety net that makes symptom 2 stop recurring.
- **`TerminalGuard`** — lift from the WIP (`mod.rs:1304-1310`), covering the `?`
  early-return path (panics are already covered by ratatui's hook).
- **Save state on the error path too**: `main` should flush playlist/config even
  when `run()` returns `Err`.
- Don't spawn a download when the track failed to save to its playlist
  (`mod.rs:799-802` currently sets a status but the download has already begun).

Tests: dead-socket keypress does not return `Err`; `PlayerGone` clears `player`;
stale `PlayerReady` (wrong generation) is discarded; `request_playback` bumps the
generation before spawning.

## Phase 2 — P1: position + download/playlist integrity

Files: `src/tui/mod.rs`, `src/tui/input.rs`, `src/ytdlp.rs`

- **Fix symptom 3 outright:** unconditional `self.position = start_pos.unwrap_or(0.0)`
  in `request_playback` (lift from WIP), plus the generation guard from Phase 1
  so the outgoing poller can't write into `app.position`.
- **Fix the progress bar (bug A):** pipe **stdout**, add `--newline`, keep stderr
  piped for errors. Correct `AGENTS.md:475`. Verify empirically with a real URL.
- `remap_download_targets` on playlist rename; `clear_download_state` on track
  delete/move — wire up the WIP's already-written helpers (bug D).
- Persist `cache_status = Downloading` to TOML at download start so the crash
  recovery in `Playlist::load:65` actually has something to recover.
- Reject duplicate `video_id` at `MetaReady` for the target playlist.
- Periodic throttled `last_position` flush (lift `maybe_flush_position` from WIP)
  so a hard kill doesn't lose the whole track's progress.
- Auto-save `loop_mode` and volume on change (currently only saved at quit).
- **Bug B:** before deleting a cached file, check no other playlist references
  that `video_id`.

Tests: rename with an in-flight download patches the correct file; deleting a
downloading track clears state; duplicate URL adds no second row; delete does not
remove a file another playlist still uses.

## Phase 3 — P2: UX + the unimplemented playback features

Files: `src/tui/mod.rs`, `src/tui/input.rs`, `src/player.rs`, `src/tui/ui.rs`

- Don't move the cursor when a track is added to the displayed playlist.
- Reset `position` to 0 when the playing track is deleted.
- Save the outgoing track's position even when the incoming track shares its
  `video_id` — compare on `(path, video_id)` identity (WIP already does this).
- **Auto-advance on EOF** — currently `l` cycles `loop_mode` but nothing reads it,
  so the switch does nothing. Lift the WIP's `eof-reached` polling and
  `handle_track_ended`, but verify EOF reliability for audio-only streams; fall
  back to `percent-pos >= 99.5` + `idle-active` if `eof-reached` proves flaky.
- **Shuffle (`r`)** — specified at `AGENTS.md:388` and entirely unimplemented.
  The WIP already has `Playlist.shuffle` and `rebuild_shuffle_order`; add the key
  handler, a UI indicator, and rebuild the order on toggle. Decide and document
  how shuffle interacts with an active search filter.
- Bug C: move mpv IPC off the render path if the UI still stutters after Phase 1.

## Phase 4 — P3: cleanup

- Remove `agent_log` from `src/main.rs` (bug G) — the WIP already removed the
  `input.rs` copy but left the call sites, which is what broke the build.
- Fix the CLI doc comment at `src/main.rs:51` ("play immediately" is wrong —
  adding never plays, per `AGENTS.md:731`).
- Bug E (first line of `--get-url`), bug F (socket path in docs).
- Clean up partial download files on yt-dlp error.
- Copy this plan to `docs/plans/20260815-trovers-stabilization.md` and add an ADR
  to `docs/decisions.md` for the signal-handling / reaper design.

---

## Verification

Per phase: `cargo test` + `cargo clippy` green, then the manual checks below.
`AGENTS.md` requires all in-repo text be **English**.

Manual smoke checklist (the symptoms, reproduced deliberately):

1. **Symptom 1:** play a short track, let it end, then press `v` / `]` / `←`.
   App must stay alive and show "stopped", not exit.
2. **Symptom 2:** while playing, `Ctrl+C`; then close the terminal window mid-play;
   then `kill -9`. After each: `pgrep mpv` must be empty (for `kill -9`, empty
   after the next app launch, via the reaper).
3. **Symptom 3:** play A to ~2:00, add B via `a`, wait for B to cache, then press
   Enter on B → B starts at 0:00 (or its own `last_position`), never at 2:00.
   Also confirm adding B never interrupts A (`AGENTS.md:731`).
4. Progress bar actually advances during a download (bug A).
5. Same track in two playlists; delete it from one → the other keeps its cached
   file (bug B).
6. Rename a playlist mid-download → the track ends up `cached` in the renamed file.
7. `l` through all three loop modes and confirm each takes effect at EOF; `r`
   toggles shuffle and next/prev follow the shuffled order.
8. `q` → terminal clean, playlist saved, `pgrep mpv` empty.
