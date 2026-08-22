```text
  ████████╗██████╗  ██████╗ ██╗   ██╗███████╗██████╗ ███████╗
  ╚══██╔══╝██╔══██╗██╔═══██╗██║   ██║██╔════╝██╔══██╗██╔════╝
     ██║   ██████╔╝██║   ██║██║   ██║█████╗  ██████╔╝███████╗
     ██║   ██╔══██╗██║   ██║╚██╗ ██╔╝██╔══╝  ██╔══██╗╚════██║
     ██║   ██║  ██║╚██████╔╝ ╚████╔╝ ███████╗██║  ██║███████║
     ╚═╝   ╚═╝  ╚═╝ ╚═════╝   ╚═══╝  ╚══════╝╚═╝  ╚═╝╚══════╝

             --- T H E  M E D I A  P L U N D E R E R ---
```                       
🏴‍☠️ Heave Ho!

Blazingly fast, keyboard-driven TUI (Terminal User Interface) designed for the modern digital privateer. Built with Rust, it’s a rugged galleon capable of plundering the web, organizing your spoils, and playing your shanties without ever touching a mouse.

Stop drowning in folders. Start ruling your media seas.

## ⚔️ Features from the Captain's Log

⚓ Plunder (Download): Hook into the web’s deepest trenches to snatch video and audio files directly to your hold.

🗺️ Charting (Catalog): Automatically index your mess of files into a beautiful, searchable treasure map.

🎵 Shanties & Spectacles (Play): A built-in, low-latency media player for both your ears and your eyes.

🦀 Forged in Iron: Powered by Rust for memory safety—no leaks on this ship, even in the roughest storms.

⌨️ Ghost Navigation: Full Vim-like keybindings because a true captain never leaves the wheel (or the home row).

## 🛠 Arming the Crew (Installation)

**Installing trovers itself:**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/qubmen/trovers/releases/latest/download/trovers-installer.sh | sh
```

Or via Homebrew:

```sh
brew install qubmen/trovers/trovers
```

Or grab a prebuilt binary directly from the [Releases page](https://github.com/qubmen/trovers/releases).

**Required Cannons:** mpv and [yt-dlp](https://github.com/yt-dlp/yt-dlp) (for the heavy lifting) — trovers shells out to these rather than bundling them, so install them separately. ffmpeg is optional but recommended: its `ffprobe` is what reads real titles and durations out of your own files when you import a folder. Without it an import still works — names come from filenames and durations show `--:--`.

```sh
brew install ffmpeg mpv yt-dlp   # macOS / Linux (via Homebrew)
# or keep yt-dlp fresher via pip, since it needs frequent updates to keep up with YouTube:
pip install yt-dlp
```

## 🧭 Navigating the Waters
Key	Action
j/k	Sail up/down through your booty
/	Scout the horizon (Search/filter)
a	Plunder a new link (Add URL)
Enter	Fire! (Play media)
m	Move track to another playlist
F	Press your own folder into service (import as an album)
R	Rescan that folder for new arrivals
J/K	Haul the selected track down/up the running order
q	Abandon ship

## 🗂️ Playlist Management

Manage multiple playlists without leaving the keyboard:

- **Switch playlists**: Tab to sidebar → highlight playlist → Enter
- **Create playlist**: `N` in track list, or sidebar → `↓ Plunder` → opens a name prompt
- **Rename playlist**: Tab to sidebar → highlight playlist → `r`
- **Delete playlist**: Tab to sidebar → highlight playlist → `d` → confirm with `y`
- **Move track**: highlight track → `m` → pick destination playlist from context menu
- **Add to specific playlist**: `a` → type URL → Tab to cycle target playlist → Enter
- **Reorder tracks**: highlight track → `J`/`K` (clear the search filter first)

## 📁 Your Own Folders (Albums)

Point trovers at a folder and it becomes an **album** — a sub-list nested under
the playlist you're looking at, holding everything in there that mpv can play.

- **Import**: `F` in the track list (or sidebar → `+ Folder`) → type or paste a
  path → Enter. Subfolders are included; the album is named after the folder.
  Paste whatever your Mac gives you — a `file://` URL full of `%D0%9A` escapes, a
  dragged path with `\ ` in it, quotes around the lot, or a plain `~/Music/…`.
- **Rescan**: `R` on a linked album picks up files added since. New files land at
  the end, files that have vanished go dim (`⊘`) instead of disappearing, and
  **nothing is ever deleted or reshuffled**.
- **Your files stay yours**: trovers only ever *reads* them. Deleting a row — or
  the whole album — forgets the track and never touches the file on disk. That is
  the one rule with no exceptions.
- Positions are remembered per file, so a three-hour set picks up where you left
  it even after a rename-and-rescan.

## ⏱️ Playback That Survives the Storm

- **Playback keeps sailing across playlist switches**: browsing or editing a
  different playlist never stops, pauses, or resets whatever's currently playing.
- **Resume near where you left off**: pressing play on a track picks up from
  its last known position instead of always starting at 0:00.
- **Your last playlist is remembered**: `trovers` reopens whichever playlist
  was active when you last quit.
- **One track, one memory**: each track is stored as its own small file, so the
  same set listed in two playlists shares one position and one speed instead of
  drifting apart. Playlists are just ordered lists pointing at them.

## 🖥️ Now Playing

The bottom of the screen shows track info and playback controls in a clean three-row layout:

```
 🎵 Now Playing              ▶ Playing                         1.5×
 Drunken Sailor • Irish Rovers • youtube.com
 01:15 ━━━━━━━◉────────────────────── 03:12   ♪ 80%  │ ◈ Cached
```

When caching a track in the background:
```
 01:15 ━━━━━━━◉── 03:12   ⟳ caching ▓▓▓▓▓░░░░░ 45%
```

## 🏗 Built With

    Ratatui — For the sturdy wooden (TUI) frame.

    Tokio — To manage the many-armed kraken of async tasks.

    Rodio/MPV-RS — For the acoustic thunder.

    "The code is the law, but the UI is the legend."

*trovers* is open-source. If you find a bug, walk the plank (or open an Issue). 
Contributions are welcome - join the crew! 🦜
