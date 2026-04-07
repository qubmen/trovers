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
| Play track on Enter (spawn player) | 🚧 | cursor + index update done; player spawn TODO |
| Add URL: fetch meta → add to playlist | 🚧 | input capture done; ytdlp call TODO |
| Start download after play begins | ⬜ | |
| Switch player when track changes (n/b) | 🚧 | index update done; player restart TODO |
| Position polling → TOML on quit | ⬜ | |
| cache_status: streaming→downloading→cached | ⬜ | |
| Reload available_playlists on create | ⬜ | |
| Switch playlist from sidebar Enter | 🚧 | skeleton in place; load TODO |
| Save playlist + config on q | ⬜ | main.rs save() calls present but player flush missing |

---

## Next Steps (suggested order)

1. Wire `Enter` in track list → spawn `Player`, start `poll_position_loop` tokio task
2. Wire `a` / Plunder → `ytdlp::fetch_metadata` → push `Track` → save playlist
3. Wire download: after `Player::spawn`, call `ytdlp::spawn_download` in tokio task,
   update `cache_status` transitions, save TOML on completion
4. Wire `n`/`b` → stop old player, start new one
5. On `q`: get final position via IPC, save playlist + config
6. Sidebar `Enter` on playlist name: `Playlist::load` + replace `app.playlist`
7. Reload `available_playlists` after `Playlist::create`
