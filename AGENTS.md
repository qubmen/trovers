# trovers — Claude Code Instructions

## Language Policy

**All code comments, documentation, commit messages, variable names, error messages,
and any other text inside the codebase must be written in English only** — regardless
of the language used in the chat conversation or by the user interacting with Claude Code.
This applies to every file in the project without exception.

---

## What This Is

A Rust CLI utility for local caching of audio tracks from YouTube and other
supported platforms (SoundCloud, Bandcamp, Mixcloud, Vimeo, etc. — anything
yt-dlp can handle). The user provides a URL — the tool immediately starts playing
the audio and simultaneously downloads the audio file to disk, adding it to a
local playlist. The primary interface is a terminal UI built with ratatui.

The name **trovers** reflects the core idea: a personal treasure trove of media,
plundered from any source and stored locally for playback anytime.

---

## External Dependencies (binaries, not Rust crates)

The tool deliberately delegates the heavy lifting to two external binaries. **Do not
attempt to replace them with Rust crates** — this is an intentional architectural decision.

### yt-dlp
- **Role:** all interaction with streaming platforms — fetching metadata, extracting
  direct audio stream URLs, downloading files to disk. Works with YouTube, SoundCloud,
  Bandcamp, Mixcloud, Vimeo, and 1800+ other sites out of the box. No code changes
  needed when adding support for a new platform — just pass the URL.
- **Why not rusty_ytdl:** yt-dlp is actively maintained and updated whenever platforms
  change their APIs. Our Rust code never needs to change as a result. Rust crates for
  YouTube tend to break and go unmaintained, and cover only YouTube.
- **Installation:** `pip install yt-dlp` or via system package manager.
- **Key commands we use:**
  ```bash
  # Fetch metadata as JSON
  yt-dlp -j --no-playlist "<URL>"

  # Get direct URL of the best audio stream (no download)
  yt-dlp -f bestaudio --get-url --no-playlist "<URL>"

  # Download best audio, progress written to stderr
  yt-dlp -f bestaudio[ext=m4a]/bestaudio \
         --no-playlist \
         -o "<output_path>" \
         "<URL>"
  ```

### mpv
- **Role:** audio playback. Always launched with `--no-video` flag.
- **Why not rodio/symphonia:** mpv handles any audio format out of the box and
  supports IPC control from Rust.
- **Installation:** system package manager (`brew install mpv`, `apt install mpv`, etc.).
- **Key flags:**
  ```bash
  # Play without opening a video window
  mpv --no-video --really-quiet "<file_or_url>"

  # With IPC socket for control from Rust
  mpv --no-video --really-quiet \
      --input-ipc-server=/tmp/trovers-<pid>.sock \
      "<file_or_url>"
  ```
- **IPC commands** (JSON over Unix socket):
  ```json
  {"command": ["set_property", "pause", true]}
  {"command": ["set_property", "pause", false]}
  {"command": ["set_property", "volume", 80]}
  {"command": ["set_property", "speed", 1.5]}
  {"command": ["seek", 30, "relative"]}
  {"command": ["seek", 0, "absolute"]}
  {"command": ["get_property", "time-pos"]}
  {"command": ["get_property", "duration"]}
  ```

---

## Architecture

```
trovers [<URL>]
       │
       ├─ 1. check that yt-dlp and mpv exist in PATH → exit with error if missing
       │
       ├─ 2. launch ratatui TUI
       │       ├─ if URL provided → add track to current playlist and start playing
       │       └─ if no URL → open TUI with last active playlist
       │
       └─ TUI event loop:
               ├─ user picks track → resolve playback source:
               │       ├─ cache_status: cached  → play local file directly
               │       └─ cache_status: streaming → yt-dlp --get-url → mpv stream URL
               │                                    + spawn yt-dlp download in background
               │
               ├─ mpv spawned with IPC socket /tmp/trovers-<pid>.sock
               │       └─ resume from last_position if > 0
               │
               ├─ yt-dlp download progress parsed from stderr → caching bar in TUI
               │
               └─ on download complete:
                       ├─ set cache_status: cached, file: <path> in playlist TOML
                       └─ mpv seamlessly continues (already playing stream URL)
```

---

## Project Structure

```
trovers/
├── Cargo.toml
├── AGENTS.md                        ← this file
├── docs/                            ← decision logs and implementation notes
├── src/
│   ├── main.rs                      ← entry point, clap CLI, launches TUI
│   ├── config.rs                    ← Config struct, load/save ~/.config/trovers/config.toml
│   ├── deps.rs                      ← verify yt-dlp and mpv in PATH
│   ├── ytdlp.rs                     ← wrapper: metadata, stream URL, download
│   ├── player.rs                    ← mpv process + IPC socket communication
│   ├── playlist.rs                  ← load/save playlist TOML files
│   ├── cache.rs                     ← file paths, cache directory management
│   └── tui/
│       ├── mod.rs                   ← App struct, Focus/InputMode/SidebarItem enums,
│       │                               ratatui event loop
│       ├── ui.rs                    ← full layout rendering (sidebar, track table,
│       │                               now-playing, footer, overlays)
│       └── input.rs                 ← keyboard event dispatch by focus + mode
└── README.md

~/.config/trovers/
└── config.toml                      ← global settings (default speed, volume, etc.)

~/.local/share/trovers/
├── playlists/
│   ├── Progressive.toml             ← playlist file (also stores per-track state)
│   └── Chill.toml
└── audio/
    ├── vK2io4J708A.m4a
    └── -iVXs77l7tE.m4a
```

---

## CLI Interface (clap)

```bash
trovers                         # open TUI with last active playlist
trovers <URL>                   # add URL to current playlist and start playing
trovers --playlist <name>       # open TUI with a specific playlist
```

All further interaction (playback control, playlist management, adding tracks)
happens inside the TUI. There are no additional subcommands.

---

## Playlist TOML Schema

Each playlist is a single `.toml` file that stores both the track list and
per-track playback state. This is the single source of truth — no separate
database or state files.

```toml
name = "Progressive"
created = "2026-04-01T12:06:59.713523Z"
loop_mode = "none"       # none | track | playlist
shuffle = false
current_index = 0

[[tracks]]
index = 0
url = "https://www.youtube.com/watch?v=vK2io4J708A"
source = "youtube.com"
title = "Miss Monique @ The Dome at UNVRS (Ibiza, Spain)"
artist = "Miss Monique"
channel = "Miss Monique"
duration = 3529
video_id = "vK2io4J708A"
cache_status = "cached"
file = "~/.local/share/trovers/audio/vK2io4J708A.m4a"
last_position = 176
speed = 1.5
added_at = "2026-04-01T12:06:59Z"

[[tracks]]
index = 1
url = "https://soundcloud.com/artbat/live-ultra-2026"
source = "soundcloud.com"
title = "ARTBAT - Live at Ultra Music Festival, Miami 2026"
artist = "ARTBAT"
channel = "ARTBAT"
duration = 3595
video_id = "artbat-live-ultra-2026"
cache_status = "streaming"
last_position = 0
speed = 1.0
added_at = "2026-04-01T13:00:00Z"
```

### source field
`source` stores the bare domain extracted from the track URL (e.g. `youtube.com`,
`soundcloud.com`, `bandcamp.com`, `mixcloud.com`). It is set once when the track is
added and never changes. The TUI uses it to render a small source icon next to the
track title. Extract it with a simple URL parse — do not hardcode a list of known
domains, just take whatever host the URL contains.

### cache_status values
- `cached` — audio file exists on disk, play locally
- `streaming` — no local file, will stream via mpv + download in background
- `downloading` — currently being downloaded (transient state, set while yt-dlp runs)

**Startup recovery:** on `Playlist::load()`, any track with `cache_status = "downloading"`
must be reset to `"streaming"`. This handles the case where the app crashed mid-download.

### Per-track speed
Speed is stored per-track and persisted between sessions. When a track is played,
the saved `speed` value is applied immediately via IPC. If no speed was ever set for
a track, use `default_speed` from `config.toml`.

---

## config.toml Schema

```toml
default_speed = 1.0
default_volume = 80      # 0–100
active_playlist = "Progressive"
```

Stored at `~/.config/trovers/config.toml`. Created with defaults on first run.

---

## TUI Layout

Built with **ratatui**. Full-screen, keyboard-driven, no mouse support.

### Color palette

| Constant        | Hex       | Role                                                     |
|-----------------|-----------|----------------------------------------------------------|
| `ACCENT`        | `#CE412B` | Rust Orange — focused borders, selection bg              |
| `ACCENT_DIM`    | `#642015` | Dimmed ACCENT — selection background in track table rows |
| `SEA_GREEN`     | `#20B288` | Playback progress bar, currently-playing row             |
| `GOLD`          | `#D4AF37` | 🎵 Now Playing header label, caching progress bar        |
| `TEXT_DIM`      | `#828282` | Secondary text, disabled items                           |
| `BORDER_IDLE`   | `#464646` | Unfocused panel borders                                  |
| `ITEM_DISABLED` | `#5A5A5A` | Non-interactive sidebar items (Music / Video labels)     |

### Screen layout (vertical)

```
Constraint::Length(1)   header bar
Constraint::Min(0)      main area  (sidebar 22 cols | track table fill)
Constraint::Length(4)   now playing block
Constraint::Length(1)   footer hint line
```

### Rendered layout

```
 ☠ trovers v0.1                                              14:35:02
╭─ Navigation ──────╮╭─ Sea Shanties  [ 12-20 / 142 ] ─────────────╮
│ ▼ ≡ Playlists     ││  #   Title                 Artist     Time ▲ │
│   Sea Shanties ◄  ││▶ 13  Drunken Sailor         Irish R.  03:12 █ │
│   Tavern Vibes    ││  14  Leave Her Johnny        Assassin  04:05 ┃ │
│   ▼ 3 more…       ││  15  Bones in the Ocean      Longest   03:55 ▼ │
│                   ││                                               │
│ ♪ Music           ││                                               │
│ ▶ Video           ││                                               │
│                   ││                                               │
│ ↓ Plunder         ││                                               │
│ ⚙ Settings        ││                                               │
╰───────────────────╯╰───────────────────────────────────────────────╯
─────────────────────────────────────────────────────────────────────
 🎵 Now Playing              ▶ Playing                         1.5×
 Drunken Sailor • Irish Rovers • youtube.com
 01:15 ━━━━━━━◉──────────────────────────────── 03:12   ♪ 80%  │ ◈ Cached
 [tab] sidebar · [↑↓] nav · [enter] play · [spc] pause · [q] quit
```

### Sidebar

- Active border colour: `ACCENT` when focused, `BORDER_IDLE` otherwise
- `▼/▶ ≡ Playlists` — collapsible section; `Enter` toggles; shows up to 5 playlists,
  overflow shown as `▼ N more…`
- Active playlist marked with `◄` suffix in `ACCENT` colour
- `♪ Music` / `▶ Video` — shown in `TEXT_DIM`, not selectable (reserved for future)
- `↓ Plunder` — opens URL input prompt (same as `a` in track list)
- `⚙ Settings` — reserved, no-op for now

### Track table

`Table` widget (not `List`) with columns:

| Col     | Width   | Content                                     |
|---------|---------|---------------------------------------------|
| icons   | 4       | play `▶` + status `◈`/`◌`/`⟳`             |
| `#`     | 5       | track number, right-aligned, `TEXT_DIM`     |
| Title   | fill    | truncated with `…`                          |
| Artist  | 16      | truncated, `TEXT_DIM`                       |
| Duration| 7       | `MM:SS` or `HH:MM:SS`, right-aligned        |

Row highlight rules (highest precedence first):
1. Playing **and** selected → bg `ACCENT`, white bold
2. Playing only → fg `SEA_GREEN`, bold
3. Selected cursor only → bg `Rgb(60,60,60)`, white
4. Normal → default

`Scrollbar` on right edge: `▲ █ ┃ ▼` symbols. Title shows
`[ first–last / total ]`.

### Now Playing block

Three lines separated from main area by a top border:

**Line 1 (header)** — `🎵 Now Playing` (GOLD bold, left) + playback status
`▶ Playing` / `⏸ Paused` / `⏳ Loading…` (white, center) + speed `1.4×`
(ACCENT bold, right). Uses `calculate_distributed_widths` for even three-section
layout.

**Line 2 (track info)** — bullet-separated metadata:
```
 TRACK TITLE • Artist • source.com
```
Title: bold white (priority truncation). Artist + source: `TEXT_DIM`. Uses
`build_separated_line` with title as primary segment (kept longest on truncation).

**Line 3 (playback bar)** — integrated progress, time, volume, and cache status:
```
 01:15 ━━━━━━━◉──────────────────────────────── 03:12   ♪ 80%  │ ◈ Cached
```
Filled: `━` in `SEA_GREEN`. Thumb: `◉`. Unfilled: `─`. Time labels + volume +
cache status in one line. When downloading, replaces right section with a download
progress bar: `⟳ caching ▓▓▓▓▓░░░░ 45%`.

### Track status icons

- `▶` — currently playing
- `◈` — cached (file on disk)
- `◌` — streaming only (no local file)
- `⟳` — downloading right now

### Do not use `indicatif`

The `indicatif` crate writes directly to stdout/stderr and conflicts with
ratatui's full terminal control. All progress bars are rendered as ratatui
widgets or custom `Paragraph` lines built from unicode block characters.

---

## Keymap

### Global (all modes)

| Key   | Action                              |
|-------|-------------------------------------|
| `Tab` | Toggle focus: Sidebar ↔ Track list  |
| `q`   | Quit (saves state to TOML)          |

### Track list focus (Normal mode)

| Key              | Action                                          |
|------------------|-------------------------------------------------|
| `↑` / `k`        | Move selection up                               |
| `↓` / `j`        | Move selection down                             |
| `g`              | Jump to first track                             |
| `G`              | Jump to last track                              |
| `Ctrl+D`         | Half-page down                                  |
| `Ctrl+U`         | Half-page up                                    |
| `Enter`          | Play selected track (resume from last_position) |
| `Space`          | Play / Pause                                    |
| `←` / `→`        | Seek −10s / +10s                               |
| `Shift+←/→`      | Seek −60s / +60s                               |
| `s` / `S`        | Speed +0.1 / −0.1 (saved to TOML immediately)  |
| `v` / `V`        | Volume +5 / −5                                  |
| `l`              | Cycle loop mode: none → track → playlist → none |
| `r`              | Toggle shuffle                                  |
| `n`              | Next track                                      |
| `b`              | Previous track                                  |
| `a`              | Add track: open URL input prompt                |
| `/`              | Search/filter tracks (live, case-insensitive)   |
| `d`              | Delete selected track (confirm prompt)          |
| `N`              | Create new playlist (name prompt)               |

### Sidebar focus

| Key     | Action                                                      |
|---------|-------------------------------------------------------------|
| `↑`/`↓` | Move between selectable items (skips disabled/separators)  |
| `Enter` | Playlists header: expand/collapse · Playlist: switch to it |
|         | Plunder: open URL input · Settings: (reserved)             |
| `r`     | Rename focused playlist (opens name input overlay)         |
| `d`     | Delete focused playlist (confirm prompt)                   |

### Track list focus — additional playlist keys

| Key | Action                                               |
|-----|------------------------------------------------------|
| `m` | Move selected track: open context menu with playlist targets |
| `N` | Create new playlist (name input prompt)              |

In URL input mode (`a` key):

| Key   | Action                                              |
|-------|-----------------------------------------------------|
| `Tab` | Cycle target playlist (instead of switching focus)  |

### Input / overlay modes

| Mode              | Keys                                   |
|-------------------|----------------------------------------|
| URL / name input  | type freely · `Enter` confirm · `Esc` cancel |
| URL input (Tab)   | cycle target playlist while typing URL |
| Search            | type to filter live · `Enter`/`Esc` exit    |
| Confirm delete    | `y` confirm · `n` cancel                    |
| Track context menu| `↑`/`↓` navigate · `Enter` confirm · `Esc` cancel |
| Playlist rename   | type new name · `Enter` confirm · `Esc` cancel |
| Playlist delete   | `y`/`Enter` confirm · `n`/`Esc` cancel |

---

## Rust Crates

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
ratatui = "0.30"
crossterm = "0.29"
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
dirs = "5"
regex = "1"
```

---

## Implementation Details

### deps.rs — dependency check on startup
Check via `Command::new("yt-dlp").arg("--version")` and same for `mpv`.
If either is missing, print a clear error with installation instructions and exit.

### ytdlp.rs — metadata parsing
`yt-dlp -j` returns a JSON object whose shape varies by platform. **All fields except
`webpage_url` and `id` must be treated as optional.** Use `Option<T>` for everything
else and fall back to sensible defaults:

| Field we need   | yt-dlp JSON key           | Fallback if missing          |
|-----------------|---------------------------|------------------------------|
| `title`         | `title`                   | `"Unknown title"`            |
| `artist`        | `artist` or `uploader`    | `"Unknown artist"`           |
| `channel`       | `channel` or `uploader`   | same as artist               |
| `duration`      | `duration`                | `0`                          |
| `video_id`      | `id`                      | *(required, never null)*     |
| `source`        | parsed from `webpage_url` | *(required, never null)*     |

Do not crash or return an error when optional fields are absent — different platforms
(SoundCloud, Bandcamp, Mixcloud, etc.) populate different subsets of these fields.

### ytdlp.rs — parsing download progress
yt-dlp writes progress to stderr:
```
[download]  45.3% of    4.23MiB at    1.23MiB/s ETA 00:02
```
Parse with regex `r"\[download\]\s+([\d.]+)%"` → send percentage to a
`tokio::sync::watch` channel → TUI reads from the channel on each render tick
to update the caching progress bar on Now Playing line 3.

### player.rs — IPC over Unix socket
- Socket path: `/tmp/trovers-<pid>.sock` (pid = current process id, avoids conflicts
  when multiple instances run simultaneously)
- On play: send `seek <last_position> absolute` immediately after mpv starts
- On speed change: send `set_property speed <value>` + save to TOML
- Poll `get_property time-pos` every second → update `last_position` in memory
- On quit or track change: flush `last_position` to TOML
- **Socket connection:** retry up to 20 times with 50ms delay — the socket is not
  available immediately after mpv spawns

### playlist.rs — TOML persistence
- Read playlist on startup with `toml` crate + `serde`
- Write back to TOML on: speed change, track end, quit, download complete
- Writes are atomic: write to `<name>.toml.tmp` then rename
- `cache_status` transitions: `streaming` → `downloading` (when yt-dlp starts)
  → `cached` (when yt-dlp finishes, file path written to `file` field)
- On load: reset any `downloading` → `streaming` (crash recovery)
- `Playlist::add_track(track)` — append a track and atomically save
- `Playlist::remove_track_by_video_id(id)` — remove and return track, atomically save
- `Playlist::rename(new_name, old_path)` — renames TOML file + updates internal name, returns new path
- `Playlist::delete(path)` — deletes the TOML file from disk
- `App::move_track_to_playlist(target_name)` — loads target, removes from source, appends to target, saves both
- `App::switch_to_playlist(name, path)` — loads playlist from path, resets track selection, pauses playback
- `App::available_playlist_names()` — returns names of all playlists except the currently active one

### config.rs — global config
- Read/write `~/.config/trovers/config.toml`
- Create with defaults (`default_speed = 1.0`, `default_volume = 80`) if file missing

### tui/mod.rs — state and event loop

**Key types:**
```rust
pub enum Focus     { Sidebar, TrackList, Settings }
pub enum InputMode {
    Normal, UrlInput, NewPlaylist, ConfirmDelete, SearchInput,
    TrackContextMenu,   // move-track popup: pick destination playlist
    PlaylistRename,     // sidebar: rename selected playlist
    PlaylistDelete,     // sidebar: confirm delete selected playlist
}
pub enum SidebarItem {
    PlaylistsHeader,
    Playlist { name, path },
    PlaylistsOverflow { count },
    Separator,
    Music,    // future, not selectable
    Video,    // future, not selectable
    Plunder,
    Settings,
}
```

**App struct** holds: playlist + config + optional player, watch channels,
`focus`, `input_mode`, `input_buf`, `selected` (track cursor), `track_offset`
(scroll), `track_list_height` (set each frame), `filtered_indices` (search),
`sidebar_selected`, `playlists_expanded`, `available_playlists`,
`position`, `download_progress`, `is_paused`,
`context_menu_selected` (selected index in track move context menu),
`target_playlist_for_url` (playlist name selected during URL input via Tab).

**Event loop:**
```
loop:
  sync_channels()         ← read watch receivers
  terminal.draw(render)   ← render full frame
  event::poll(100ms)      ← non-blocking
  handle_key(event)       ← dispatch by focus + mode
  clamp_scroll()          ← keep selected in visible window
```

### tui/ui.rs — rendering

- `render()` — top-level: splits frame into 4 rows, calls sub-renderers
- `render_header()` — single line: app name (ACCENT bold) + real-time clock
- `render_sidebar()` — `List` inside rounded `Block`; border ACCENT when focused
- `render_track_table()` — `Table` + `Scrollbar`; sets `app.track_list_height`
- `render_now_playing()` — 3 rows: header, track info, playback bar
  - `render_now_playing_header()` — row 1: "🎵 Now Playing" | status | speed
  - `render_track_info_row()` — row 2: TITLE • Artist • source (bullet-separated)
  - `render_playback_bar()` — row 3: progress bar + time + volume + cache status
- `render_footer()` — context-sensitive hint line
- `render_input_overlay()` — `Clear` + centred rounded popup for text input; shows current target playlist when in `UrlInput` mode (Tab cycles through playlists)
- `render_track_context_menu()` — centred popup listing available playlists for track move; highlights selected entry with `ACCENT` bg; same overlay pattern as `render_input_overlay()`
- `make_panel_block()` — reusable rounded-border block with consistent focus styling
- `build_progress_bar(width, ratio, fill, empty, thumb, fill_color, empty_color)` —
  builds `Vec<Span>` with separate colored spans for filled and empty sections.
  Pass `thumb = '\0'` to disable thumb. Returns correctly colored spans.
- `build_now_playing_header_line(width, center, speed)` — builds header row spans
  using `calculate_distributed_widths` for three-section layout
- `build_track_info_line(width, title, artist, source)` — builds track metadata row
  using `build_separated_line` with title as primary (highest truncation priority)
- `build_playback_bar_line(width, pos, ratio, dur, vol, cache_state)` — builds the
  integrated progress+controls row; switches to download bar when `CacheState::Downloading`
- `calculate_distributed_widths(total_width, section_count, fixed_widths)` —
  distributes width across N sections given fixed-width constraints; remaining width
  goes to the first flexible (unfixed) section
- `build_separated_line(segments, max_width)` — joins text segments with ` • ` separator,
  applying truncation with priority: first segment (primary) is preserved longest,
  subsequent segments share remaining budget evenly
- `format_playback_state(has_player, is_paused, has_track)` — returns `(icon, text)`
  tuple for consistent status display: `("▶","Playing")`, `("⏸","Paused")`,
  `("⏳","Loading…")`, `("","No track")`
- `format_duration(secs)` — formats seconds as `MM:SS` or `HH:MM:SS`
- `truncate(s, max)` — truncates string to `max` chars, appending `…` if needed

### Layout calculation patterns

When building multi-section rows in the now-playing area:

1. Use `calculate_distributed_widths` for fixed-count sections with known constraints.
   Example — three-section header: left (fixed) | center (flex) | right (fixed):
   ```rust
   let fixed = [(0, left_len), (2, right_len)];
   let widths = calculate_distributed_widths(total_width, 3, &fixed);
   // widths[1] gets remaining space for center
   ```

2. Use `build_separated_line` for bullet-separated metadata with truncation priority.
   First segment has priority (title over artist over source):
   ```rust
   let parts = build_separated_line(&[(title, true), (artist, false), (source, false)], budget);
   ```

3. Progress bar with separate fill/empty colors: always pass both `fill_color` and
   `empty_color` explicitly. Use `SEA_GREEN`/`BORDER_IDLE` for playback,
   `GOLD`/`TEXT_DIM` for download caching bars.

4. The `CacheState` enum (`Cached`, `Streaming`, `Downloading(f64)`) controls which
   variant of the playback bar is rendered — normal vs. download-progress layout.

### tui/input.rs — keyboard dispatch

- `Tab` always switches focus (except during text input modes)
- Sidebar keys: `↑↓` skip non-selectable items; `Enter` acts on current item
- Track list keys: full keymap including vim navigation (`j/k`, `g/G`, `Ctrl+D/U`)
- `/` enters `SearchInput` mode; typing calls `App::update_search()` which
  updates `filtered_indices` live
- `Ctrl+D/U`: half-page jump using `app.track_list_height`

### Concurrency model
- Main thread: ratatui event loop (non-blocking via `event::poll` with 100ms timeout)
- tokio task: mpv IPC polling every 1s (sends `time-pos` via `watch::Sender<f64>`)
- tokio task: yt-dlp download process (sends progress % via `watch::Sender<f32>`)
- TUI reads both `watch::Receiver`s on each render tick via `has_changed()` +
  `borrow_and_update()`

### State save on exit
On `q` key: write current `time-pos` from mpv via IPC → save to TOML → quit.
This ensures `last_position` is always up to date for the next session.

---

## Out of Scope (do not implement unless explicitly asked)

- Format conversion (use whatever yt-dlp outputs)
- ID3/mp4 metadata tagging
- Importing playlists from external services (tracks added manually one by one only)
- Mouse support in TUI
- Video playback (architecture supports it via mpv, but UI is audio-only for now)
- Settings screen (⚙ Settings sidebar item is reserved but not implemented)
