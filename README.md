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
Required Cannons: ffmpeg and mpv (for the heavy lifting).

## 🧭 Navigating the Waters
Key	Action
j/k	Sail up/down through your booty
s	Scout the horizon (Search)
p	Plunder a new link (Download)
Enter	Fire! (Play media)
q	Abandon ship

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
