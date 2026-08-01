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
| `ytdlp::get_stream_url()` | ✅ | |
| `ytdlp::spawn_download()` | ✅ | parses stderr progress, watch channel |
| `player::Player::spawn()` | ✅ | retry socket up to 20×50ms |
| `player::Player::send_command()` | ✅ | JSON over Unix socket |
| `player` pause/resume/seek/speed/volume | ✅ | all IPC wrappers implemented |
| `player::poll_position_loop()` | ✅ | 1s tick, stops when receiver dropped |
| `player::Drop` cleanup | ✅ | removes socket file |

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
| Loop / shuffle (l, r) | ✅ | |
| Search mode (/) | ✅ | live filter |
| Delete with confirm (d → y/n) | ✅ | |

---

## Integration (wiring TUI ↔ player ↔ ytdlp)

| Task | Status | Notes |
|------|--------|-------|
| Play track on Enter (spawn player) | ✅ | resumes from `last_position` via `resume_start_pos` |
| Add URL: fetch meta → add to playlist | ✅ | adds + backgrounds download only; never auto-plays or touches `current_track` (fixed add-track playback-hijack bug) |
| Start download after play begins | ✅ | per-`video_id` progress via `HashMap<String, f32>` (no cross-track clobbering) |
| Switch player when track changes (n/b) | ✅ | `n`/`b` always step the **displayed** playlist (`app.playlist`), independent of what's actually playing |
| Position polling → TOML on quit | ✅ | `App::flush_playing_position()` writes `last_position` for the `PlayingSession`'s track to disk in `run()`'s single quit path, before `ratatui::restore()` |
| cache_status: streaming→downloading→cached | ✅ | transitions routed through `patch_and_save_playlist` path-aware helper |
| Reload available_playlists on create | ✅ | |
| Switch playlist from sidebar Enter | ✅ | `switch_to_playlist()` no longer stops playback — player/position/pause state are untouched by playlist switches; also persists `config.active_playlist` |
| Save playlist + config on q | ✅ | quit path flushes playing-track position before saving, closing the previous "player flush missing" gap |

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
