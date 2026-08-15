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

**Required Cannons:** ffmpeg, mpv, and [yt-dlp](https://github.com/yt-dlp/yt-dlp) (for the heavy lifting) — trovers shells out to these rather than bundling them, so install them separately:

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
q	Abandon ship

## 🗂️ Playlist Management

Manage multiple playlists without leaving the keyboard:

- **Switch playlists**: Tab to sidebar → highlight playlist → Enter
- **Create playlist**: `N` in track list, or sidebar → `↓ Plunder` → opens a name prompt
- **Rename playlist**: Tab to sidebar → highlight playlist → `r`
- **Delete playlist**: Tab to sidebar → highlight playlist → `d` → confirm with `y`
- **Move track**: highlight track → `m` → pick destination playlist from context menu
- **Add to specific playlist**: `a` → type URL → Tab to cycle target playlist → Enter

## ⏱️ Playback That Survives the Storm

- **Playback keeps sailing across playlist switches**: browsing or editing a
  different playlist never stops, pauses, or resets whatever's currently playing.
- **Resume near where you left off**: pressing play on a track picks up from
  its last known position instead of always starting at 0:00.
- **Your last playlist is remembered**: `trovers` reopens whichever playlist
  was active when you last quit.

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
