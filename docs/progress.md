# trovers — Implementation Progress

Status of each module and feature as of the current codebase state.

Legend: ✅ done · 🚧 stub/partial · ⬜ not started

---

## Infrastructure

| Task | Status | Notes |
|------|--------|-------|
| `Cargo.toml` with all deps | ✅ | ratatui 0.30, crossterm 0.29, toml 0.8 |
| `deps.rs` — check yt-dlp + mpv in PATH | ✅ | clear error + install hint on failure |
| `cache.rs` — path helpers + ensure_dirs | ✅ | uses `dirs` crate for XDG paths |
| `config.rs` — load/save config.toml | ✅ | creates with defaults on first run |

---

## Data Layer

| Task | Status | Notes |
|------|--------|-------|
| `Track` + `CacheStatus` in `library.rs` | ✅ | one document per track; full TOML round-trip |
| `Playlist` holds `tracks: Vec<String>` | ✅ | ordered library ids and nothing else (ADR-015) |
| `LoopMode` enum | ✅ | serde rename_all = "lowercase" |
| `Library::load()` | ✅ | indexes by the document's inner `id`; crash recovery: downloading → streaming, cached-but-file-gone → streaming; warn-and-skip an unreadable document |
| `Library::get/get_mut/save/upsert/remove` | ✅ | atomic write per document; `-2` suffix on filename collision |
| `library::migrate()` | ✅ | rewrites playlists that still embed their tracks; shape-detected, so it is a no-op on every later launch; backs `playlists/` up first |
| `Playlist::load()` | ✅ | nothing to repair — that is `Library::load`'s job |
| `Playlist::save()` | ✅ | atomic write via tmp + rename |
| `Playlist::list_all()` | ✅ | returns sorted Vec<PathBuf> |
| `Playlist::create()` | ✅ | creates new empty playlist on disk |

---

## External Wrappers

| Task | Status | Notes |
|------|--------|-------|
| `ytdlp::fetch_metadata()` | ✅ | all fields optional with fallbacks |
| `ytdlp::get_stream_url()` | ✅ | first line of `--get-url` output only |
| `ytdlp::spawn_download()` | ✅ | parses **stdout** progress (`--newline`), watch channel; scratch files removed on failure |
| `ytdlp::download_with_retries()` | ✅ | 3 attempts total, 15s/60s backoff; row goes to `failed` only once every attempt is spent |
| `player::Player::spawn()` | ✅ | retry socket up to 20×50ms, `kill_on_drop` |
| `player::Player::send_command()` | ✅ | JSON over Unix socket, 2s timeout, skips mpv's unsolicited events |
| `player` pause/resume/seek/speed/volume | ✅ | all IPC wrappers implemented; never `?`-propagated into the event loop |
| `player::poll_position_loop()` | ✅ | 1s tick; generation-guarded; reports `PlayerGone` when mpv exits |
| `player::Drop` cleanup | ✅ | kills mpv, removes socket file |
| `player::reap_orphaned_players()` | ✅ | startup net for mpv stranded by a hard kill |

---

## TUI

| Task | Status | Notes |
|------|--------|-------|
| `App` struct with all fields | ✅ | focus, sidebar, eq, search, scroll |
| `Focus` / `InputMode` / `SidebarItem` enums | ✅ | |
| `App::sidebar_items()` | ✅ | builds item list dynamically |
| `App::clamp_scroll()` | ✅ | |
| `App::update_search()` | ✅ | live filter into filtered_indices |
| `App::sidebar_next/prev()` | ✅ | skips non-selectable items |
| Event loop (100ms poll + watch sync) | ✅ | |
| Equalizer animation (LCG, 3-tick update) | ✅ | |
| Header bar (name + clock) | ✅ | |
| Sidebar (collapsible, focus border) | ✅ | |
| Track table (`Table` + `Scrollbar`) | ✅ | 5 columns, row highlight rules |
| Now Playing — line 1 (title/artist/speed) | ✅ | |
| Now Playing — line 2 (custom ━◉─ bar) | ✅ | |
| Now Playing — line 3 (caching bar + eq) | ✅ | |
| Footer (context-sensitive hints) | ✅ | |
| Input overlay (URL / name / search) | ✅ | rounded popup with ACCENT border |
| Tab focus switching | ✅ | |
| Sidebar keyboard navigation | ✅ | ↑↓ Enter |
| Track list navigation (j/k, g/G, Ctrl+D/U) | ✅ | |
| Speed / volume keys (s/S, v/V) | ✅ | |
| Loop / shuffle (l, r) | ✅ | both take effect at end of track; badges in the footer |
| Search mode (/) | ✅ | live filter |
| Delete with confirm (d → y/n) | ✅ | |
| Recache (c) | ✅ | forces a fresh, retrying download regardless of current status; `downloading`/`failed` get their own icon color (yellow/red) in the track table and Now Playing |

---

## Integration (wiring TUI ↔ player ↔ ytdlp)

| Task | Status | Notes |
|------|--------|-------|
| Play track on Enter (spawn player) | ✅ | resumes from `last_position` via `resume_start_pos` |
| Add URL: fetch meta → add to playlist | ✅ | adds + backgrounds download only; never auto-plays or touches `current_track` (fixed add-track playback-hijack bug) |
| Start download after play begins | ✅ | per-track progress via `HashMap<String, f32>` keyed by library id (no cross-track clobbering) |
| Switch player when track changes (n/b) | ✅ | `n`/`b` always step the **displayed** playlist (`app.playlist`), independent of what's actually playing; follow the shuffled order when shuffle is on and no search filter is active |
| Auto-advance at end of track | ✅ | mpv's own exit is the EOF signal (`PlayerGone` + `reached_end_of_track`); honours `loop_mode` and shuffle, and follows the **playing** playlist |
| Position polling → TOML on quit | ✅ | `App::flush_playing_position()` writes `last_position` into the playing track's own document in `run()`'s single quit path, before `ratatui::restore()`; also on a 15s throttle |
| cache_status: streaming→downloading→cached | ✅ | transitions written to the track's document via `patch_track(id, f)` — no playlist involved |
| Reload available_playlists on create | ✅ | |
| Switch playlist from sidebar Enter | ✅ | `switch_to_playlist()` no longer stops playback — player/position/pause state are untouched by playlist switches; also persists `config.active_playlist` |
| Save playlist + config on q | ✅ | quit path flushes playing-track position before saving, closing the previous "player flush missing" gap |

---

## Stabilization (2026-08-15)

Four phases, one commit each — see
`docs/plans/20260815-trovers-stabilization.md` for the full plan and
ADR-012/ADR-013 in `docs/decisions.md` for the designs.

| Phase | Delivered |
|-------|-----------|
| 1 — stop crashing, stop orphaning mpv | mpv IPC failures log instead of `?`-ing out of the event loop; `PlayerGone` clears a dead player; `stop_player()` + `player_generation`; `kill_on_drop` on mpv and yt-dlp; SIGINT/SIGTERM/SIGHUP run the normal shutdown; startup reaper; `TerminalGuard`; state saved on the error path too |
| 2 — position and download integrity | position no longer bleeds from the outgoing track into the incoming one; progress bar reads stdout with `--newline`; download state follows a renamed playlist and is cleared on delete; `downloading` persisted for crash recovery; duplicate `video_id` rejected; throttled `last_position` flush; `loop_mode`/volume saved on change; a cached file shared with another playlist survives a delete |
| 3 — UX and the unimplemented playback features | auto-advance at end of track with all three loop modes; shuffle (`r`) as a stored permutation; footer badges; cursor stays put on add; position reset when the playing track is deleted; `(path, video_id)` identity for the outgoing-position save; bounded mpv IPC |
| 4 — cleanup | leftover `agent_log` debug writer removed from `main.rs` and `input.rs`; CLI `url` doc corrected (adding never plays); `--get-url` first line only; failed downloads clean up their scratch files |

---

## Track library (2026-08-22)

Phase 1 of `Track library, albums, local folders, video` — a behaviour-preserving
refactor. Everything that worked before works identically after, with one
deliberate change: the same track in two playlists now shares one position and
one speed instead of keeping two divergent copies. See ADR-015.

| Piece | Status | Notes |
|-------|--------|-------|
| `src/library.rs` — documents, ids, `Library` | ✅ | `root` injected, so the whole thing tests against a tempdir |
| `Playlist.tracks: Vec<Track>` → `Vec<String>` | ✅ | `current_track` holds an id |
| Startup migration + backup | ✅ | verified on a copy of a real library: 3 playlists → 19 documents, order/`current_track`/`last_position`/`speed`/`cache_status` all preserved |
| `App` rewired onto the library | ✅ | `patch_track(id, f)`; a row whose document is missing renders dimmed rather than vanishing |
| Ownership bookkeeping deleted | ✅ | `download_targets`, `remap_download_targets`, `retarget_download`, `clear_download_state_for_playlist`, `patch_and_save_playlist`, `save_playing_session_playlist` |
| Manual check at a real terminal | ⬜ | resume-on-`Enter`, `◈` rows playing from disk, position surviving quit → relaunch |

---

## Albums and local folders (2026-08-22)

Phase 2 of the same plan. New user-facing behaviour: point trovers at a folder
and it becomes an album under the current playlist. See ADR-016 (album as a child
playlist), ADR-017 (ffprobe as a soft dependency) and ADR-018 (never delete a
user's file).

| Piece | Status | Notes |
|-------|--------|-------|
| `Track.origin` / `media` / `resume`, `CacheStatus::Missing` | ✅ | all `serde(default)`ed, so every Phase 1 document loads as what it is; `Missing` heals back to `Cached` when the file reappears |
| `src/library_scan.rs` — walk, probe, filename parse | ✅ | stack-based `read_dir`, depth-capped at 16, does not follow directory symlinks; ffprobe optional, warned about once |
| Albums as child playlists (`kind`/`parent`/`source_folder`) | ✅ | `Playlist::list_entries()`, sidebar indent, orphans fall back to top level, rename rewrites children |
| `src/library_import.rs` + `F` import / `R` rescan | ✅ | rescan appends and marks, never deletes or reorders; counts reported in the status line |
| Never-delete-user-files guard | ✅ | three sites: `handle_confirm_delete`, `recache_track`, the playback path |
| Panel title totals + `J`/`K` reorder | ✅ | `Live Sets · 42 tracks · 6h 12m  [ 12–20 / 42 ]`; reorder refuses under a search filter |
| Manual check at a real terminal | ⬜ | import a mixed folder, resume across a relaunch, rename-and-rescan → `⊘`, `d` on a local row leaves the file, an import with ffprobe off PATH |

---

## Albums in the track list (2026-08-22)

Albums shipped as indented sidebar rows, and the sidebar has 22 columns — real names
arrived as `Кино - Гр…`, indistinguishable from each other. So they moved into the
panel that has room for a name and holds the tracks they belong with. See ADR-019,
which amends the sidebar half of ADR-016; storage is unchanged.

| Piece | Status | Notes |
|-------|--------|-------|
| `Playlist.collapsed` + albums leave the sidebar | ✅ | defaults to folded, so a two-hundred-file import arrives as one row; `playlist::sidebar_entries` keeps orphans, which is the only way left to reach one |
| The row model — `RowSource` / `VisibleRow` / `LoadedAlbum` | ✅ | `rebuild_rows` is the only writer of `App::rows`; `track_index_at` and `filtered_indices` are gone, the search filter is an input rather than a parallel answer |
| Headers, indented album tracks, panel title | ✅ | `▸`/`▾` glyphs, deliberately not the playing marker's `▶`; count and duration in the artist and duration columns; the scroll counter counts rows |
| An album plays as its own list | ✅ | `play_from_list(source, idx, start_pos)` is the single door; `n`/`b`, loop, shuffle and auto-advance stay inside the album, each with its own shuffled order |
| Header keys and owner-aware edits | ✅ | `Enter` folds (a header is not playable), `r`/`d`/`R` reach that album, `J`/`K` refuse; `d`, `m` and `J`/`K` on a track row edit and save the list the row came out of |
| Imports, rescans and switches keep the rows in step | ✅ | an import lands as an open album under the cursor; a rename repoints the loaded copies; deleting from the sidebar takes the row with it |
| Manual check at a real terminal | ⬜ | fold two albums and confirm the state survives a restart; play through the end of an album and confirm auto-advance stays inside it; rename and forget an album from its header and confirm the folder is untouched |

Phase 3 (video playback in a real window) is not started.

---

## Next Steps

All originally-listed Integration wiring steps are now complete. The
"Decouple Player from Displayed Playlist" effort (see
`docs/plans/completed/20260801-decouple-player-from-playlist.md`) closed the
remaining gaps: `switch_to_playlist` no longer stops playback, the playing
track is tracked independently via `PlayingSession` (see ADR-011 in
`docs/decisions.md`), resume-from-`last_position` works, position is flushed
to TOML on quit, `active_playlist` persists across restarts, and download
progress is tracked per track. No further Integration TODOs are
outstanding; future work should be tracked as new plans rather than appended
here.
