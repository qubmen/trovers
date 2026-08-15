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
| `Playlist` + `Track` structs with serde | ✅ | full TOML round-trip |
| `CacheStatus` / `LoopMode` enums | ✅ | serde rename_all = "lowercase" |
| `Playlist::load()` | ✅ | crash recovery: downloading → streaming |
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
| Start download after play begins | ✅ | per-`video_id` progress via `HashMap<String, f32>` (no cross-track clobbering) |
| Switch player when track changes (n/b) | ✅ | `n`/`b` always step the **displayed** playlist (`app.playlist`), independent of what's actually playing; follow the shuffled order when shuffle is on and no search filter is active |
| Auto-advance at end of track | ✅ | mpv's own exit is the EOF signal (`PlayerGone` + `reached_end_of_track`); honours `loop_mode` and shuffle, and follows the **playing** playlist |
| Position polling → TOML on quit | ✅ | `App::flush_playing_position()` writes `last_position` for the `PlayingSession`'s track to disk in `run()`'s single quit path, before `ratatui::restore()` |
| cache_status: streaming→downloading→cached | ✅ | transitions routed through `patch_and_save_playlist` path-aware helper |
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

## Next Steps

All originally-listed Integration wiring steps are now complete. The
"Decouple Player from Displayed Playlist" effort (see
`docs/plans/completed/20260801-decouple-player-from-playlist.md`) closed the
remaining gaps: `switch_to_playlist` no longer stops playback, the playing
track is tracked independently via `PlayingSession` (see ADR-011 in
`docs/decisions.md`), resume-from-`last_position` works, position is flushed
to TOML on quit, `active_playlist` persists across restarts, and download
progress is tracked per `video_id`. No further Integration TODOs are
outstanding; future work should be tracked as new plans rather than appended
here.
