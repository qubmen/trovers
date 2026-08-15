# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/qubmen/trovers/releases/tag/v0.1.1) - 2026-08-15

### Added

- automate version bumps and tagging with release-plz
- add release CI via cargo-dist, ship a shell/Homebrew install story
- retry failed downloads automatically, add a manual recache key
- advance at end of track, add shuffle, bound mpv IPC
- flush playing position on quit, track download progress per video_id
- resume playback from last_position on track start
- introduce PlayingSession, decouple playback from playlist switching
- fix add-track playback-hijack bug
- add core application modules
- task 7 - update documentation and complete playlist functionality plan
- task 6 - verify acceptance criteria and edge cases
- task 5 - add playlist selection during URL input
- task 4 - add playlist management in sidebar (rename/delete)
- task 3 - implement playlist switching functionality
- task 2 - implement track moving between playlists
- task 1 - add track context menu infrastructure
- task 9 - verify acceptance criteria and edge cases
- apply UI consistency improvements across all panels
- remove render_cache_and_eq and add now-playing integration tests
- redesign playback bar row to integrate progress, volume, and cache status
- implement render_track_info_row for now-playing track metadata display
- implement render_now_playing_header with three-section layout
- add layout calculation utilities (Task 3)
- fix build_progress_bar to use separate colored spans for fill/empty sections
- create UI test infrastructure and test existing helpers

### Fixed

- replace manual min/max clamping with clamp() in popup sizing
- refresh the playing session's playlist clone before switching away
- stop the position bleed and keep download state with its row
- stop crashing on dead mpv and stop orphaning players
- address code review findings
- persist active_playlist to config on switch and startup
- n/b navigate displayed playlist; guard delete/move on playing-session identity
- address code review findings
- address code review findings
- address code review findings
- address code review findings

### Other

- manually bump version to 0.1.1, note git_only bug in release-plz.toml
- release v0.1.0
- remove debug leftovers, tidy yt-dlp edges, document the work
- update progress.md, add ADR for PlayingSession decoupling, move plan to completed
- correct AGENTS.md for no-autoplay and playback-unaffected-by-switch behavior
- render Now Playing and track highlight from app.playing
- add path-aware playlist patch helper, use for DownloadDone and speed changes
- status messages, help overlay, playlist selection debug logging
- add project decisions and progress documentation
- remove duplicate now-playing plan from plans root (already in completed/)
- add playlist functionality extensions implementation plan
- update documentation and move completed plan
- add now playing UI redesign implementation plan
- Initial commit

## [0.1.0](https://github.com/qubmen/trovers/releases/tag/v0.1.0) - 2026-08-15

### Added

- automate version bumps and tagging with release-plz
- add release CI via cargo-dist, ship a shell/Homebrew install story
- retry failed downloads automatically, add a manual recache key
- advance at end of track, add shuffle, bound mpv IPC
- flush playing position on quit, track download progress per video_id
- resume playback from last_position on track start
- introduce PlayingSession, decouple playback from playlist switching
- fix add-track playback-hijack bug
- add core application modules
- task 7 - update documentation and complete playlist functionality plan
- task 6 - verify acceptance criteria and edge cases
- task 5 - add playlist selection during URL input
- task 4 - add playlist management in sidebar (rename/delete)
- task 3 - implement playlist switching functionality
- task 2 - implement track moving between playlists
- task 1 - add track context menu infrastructure
- task 9 - verify acceptance criteria and edge cases
- apply UI consistency improvements across all panels
- remove render_cache_and_eq and add now-playing integration tests
- redesign playback bar row to integrate progress, volume, and cache status
- implement render_track_info_row for now-playing track metadata display
- implement render_now_playing_header with three-section layout
- add layout calculation utilities (Task 3)
- fix build_progress_bar to use separate colored spans for fill/empty sections
- create UI test infrastructure and test existing helpers

### Fixed

- refresh the playing session's playlist clone before switching away
- stop the position bleed and keep download state with its row
- stop crashing on dead mpv and stop orphaning players
- address code review findings
- persist active_playlist to config on switch and startup
- n/b navigate displayed playlist; guard delete/move on playing-session identity
- address code review findings
- address code review findings
- address code review findings
- address code review findings

### Other

- remove debug leftovers, tidy yt-dlp edges, document the work
- update progress.md, add ADR for PlayingSession decoupling, move plan to completed
- correct AGENTS.md for no-autoplay and playback-unaffected-by-switch behavior
- render Now Playing and track highlight from app.playing
- add path-aware playlist patch helper, use for DownloadDone and speed changes
- status messages, help overlay, playlist selection debug logging
- add project decisions and progress documentation
- remove duplicate now-playing plan from plans root (already in completed/)
- add playlist functionality extensions implementation plan
- update documentation and move completed plan
- add now playing UI redesign implementation plan
- Initial commit
