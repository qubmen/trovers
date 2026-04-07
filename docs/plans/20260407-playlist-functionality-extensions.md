# Playlist Functionality Extensions

## Overview
- Implement complete playlist management workflow with contextual actions
- Add track context menu for moving tracks between playlists (m key)
- Extend sidebar playlist management (create, rename, delete, switch)
- Add playlist selection during URL input
- Problem it solves: Currently users can only work with one playlist and cannot organize tracks by theme/type
- Key benefits: Seamless playlist organization, track categorization, improved workflow efficiency
- Integration: Builds on existing TOML storage, popup patterns, and input handling system

## Context (from discovery)
- **Files/components involved:** 
  - `src/playlist.rs` - TOML storage with atomic saves, `Playlist::create()` exists
  - `src/tui/ui.rs` - existing popup pattern at line 700-728, sidebar rendering
  - `src/tui/input.rs` - input dispatch, TODO for playlist switching at line 64
  - `src/tui/mod.rs` - `App` state, `InputMode` enum, `SidebarItem` enum
- **Related patterns found:**
  - Popup overlay: `Clear` + centered `Rect` + `Paragraph` in `ACCENT` rounded block
  - Input modes: `UrlInput`, `NewPlaylist`, `ConfirmDelete` gate key handling
  - Atomic saves: `.tmp` file + `fs::rename` pattern used in playlist and config
- **Dependencies identified:** 
  - Existing `available_playlists: Vec<(String, PathBuf)>` needs playlist switching logic
  - `TaskMsg` async bridge for potential background playlist operations
  - Current sidebar navigation with `sidebar_selected` index management

## Development Approach
- **Testing approach**: Regular - implement code first, then write comprehensive tests
- Complete each task fully before moving to the next
- Make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
  - Tests are not optional - they are a required part of the checklist
  - Write unit tests for new functions/methods
  - Write unit tests for modified functions/methods  
  - Add new test cases for new code paths
  - Update existing test cases if behavior changes
  - Tests cover both success and error scenarios
- **CRITICAL: all tests must pass before starting next task** - no exceptions
- **CRITICAL: update this plan file when scope changes during implementation**
- Run tests after each change
- Maintain backward compatibility

## Testing Strategy
- **Unit tests**: required for every task (see Development Approach above)
- **E2E tests**: project has no existing e2e test framework
- Focus on unit testing new popup rendering, input handling, and playlist operations
- Follow existing pattern in `ui_test.rs` - test pure functions with data assertions
- Test state transitions in `App` struct for new input modes and focus changes

## Progress Tracking
- Mark completed items with `[x]` immediately when done
- Add newly discovered tasks with ➕ prefix
- Document issues/blockers with ⚠️ prefix
- Update plan if implementation deviates from original scope
- Keep plan in sync with actual work done

## Solution Overview
- **Contextual actions approach**: Context menu for tracks (m key), sidebar management for playlists
- **Three new input modes**: `TrackContextMenu`, `PlaylistRename`, `PlaylistContextMenu`
- **Enhanced URL flow**: Show current playlist with option to change via Tab key
- **Sidebar interactions**: n/r/d keys for create/rename/delete when focused on playlists
- **Architecture**: Extends existing popup pattern, reuses TOML storage, maintains flat App state
- **Key design decisions**:
  - Context menu as popup overlay (consistent with existing pattern)
  - Playlist operations through sidebar focus (spatial/intuitive)
  - Tab key for playlist switching during URL input (discoverable)

## Technical Details
- **New InputMode variants**: `TrackContextMenu`, `PlaylistRename`, `PlaylistDelete`
- **New App state fields**:
  - `context_menu_selected: usize` - index in context menu options
  - `target_playlist_for_url: Option<String>` - playlist name for URL input
- **Context menu data structure**: `Vec<String>` of available playlist names, filtered to exclude current
- **Playlist operations**: Extend `Playlist::load()` usage, add playlist delete/rename methods
- **Processing flow**:
  1. Track context menu → select target → move track → update both playlists → save both
  2. Sidebar playlist actions → validate → perform operation → update `available_playlists`
  3. URL input with playlist selection → set target → add track to target playlist

## What Goes Where
- **Implementation Steps** (`[ ]` checkboxes): tasks achievable within this codebase - code changes, tests, documentation updates
- **Post-Completion** (no checkboxes): items requiring external action - manual testing, changes in consuming projects, deployment configs, third-party verifications

## Implementation Steps

### Task 1: Add track context menu infrastructure

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/ui.rs`
- Modify: `src/tui/input.rs`

- [x] Add `TrackContextMenu` variant to `InputMode` enum
- [x] Add `context_menu_selected: usize` field to `App` struct  
- [x] Add helper method `App::available_playlist_names()` returning filtered playlist names
- [x] Implement `render_track_context_menu()` function using existing popup pattern
- [x] Add 'm' key handler in `handle_tracklist()` to enter context menu mode
- [x] Add context menu navigation (up/down/enter/escape) in `handle_track_context_menu()`
- [x] Write tests for context menu rendering with different playlist counts
- [x] Write tests for context menu navigation and selection logic
- [x] Run tests - must pass before task 2

### Task 2: Implement track moving between playlists

**Files:**
- Modify: `src/playlist.rs`
- Modify: `src/tui/mod.rs`

- [x] Add `Playlist::add_track()` method to append track to playlist
- [x] Add `Playlist::remove_track_by_video_id()` method returning removed track
- [x] Implement `App::move_track_to_playlist()` method handling the full operation
- [x] Handle edge cases: moving current playing track, target playlist errors
- [x] Update `available_playlists` state when creating new playlists during move
- [x] Add atomic save pattern for both source and target playlists
- [x] Write tests for `Playlist::add_track()` and `remove_track_by_video_id()`
- [x] Write tests for `App::move_track_to_playlist()` success and error cases
- [x] Run tests - must pass before task 3

### Task 3: Implement playlist switching functionality

**Files:**
- Modify: `src/tui/input.rs`
- Modify: `src/tui/mod.rs`

- [x] Replace TODO in `handle_sidebar()` with actual playlist loading logic
- [x] Add `App::switch_to_playlist()` method loading playlist and updating state
- [x] Handle current player state when switching (pause/stop if needed)
- [x] Update `playlist_path` field to point to new playlist file
- [x] Reset track selection and filtering state on playlist switch
- [x] Preserve sidebar selection position when appropriate
- [x] Write tests for `App::switch_to_playlist()` with various app states
- [x] Write tests for playlist switching edge cases (file not found, corrupted playlist)
- [x] Run tests - must pass before task 4

### Task 4: Add playlist management in sidebar

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/input.rs`
- Modify: `src/playlist.rs`

- [x] Add `PlaylistRename` and `PlaylistDelete` variants to `InputMode`
- [x] Add 'r' key handler for rename when focused on sidebar playlist
- [x] Add 'd' key handler for delete with confirmation when focused on sidebar playlist
- [x] Add `Playlist::rename()` method with atomic file operations
- [x] Add `Playlist::delete()` method with file cleanup
- [x] Implement playlist rename popup using existing input overlay pattern
- [x] Add validation for playlist names (no duplicates, valid filenames)
- [x] Write tests for `Playlist::rename()` and `delete()` methods
- [x] Write tests for sidebar playlist management key handling
- [x] Run tests - must pass before task 5

### Task 5: Add playlist selection during URL input

**Files:**
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/ui.rs` 
- Modify: `src/tui/input.rs`

- [x] Add `target_playlist_for_url: Option<String>` field to `App` struct
- [x] Modify `render_input_overlay()` to show current target playlist
- [x] Add Tab key handler in `handle_url_input()` to cycle through playlists
- [x] Update `fetch_url()` to use target playlist instead of current playlist
- [x] Add visual indication of target playlist in URL input overlay
- [x] Reset target playlist to current on URL input mode entry
- [x] Add helper method for playlist cycling logic
- [x] Write tests for playlist selection during URL input
- [x] Write tests for URL target playlist visual rendering
- [x] Run tests - must pass before task 6

### Task 6: Verify acceptance criteria and edge cases

- [ ] Verify track context menu works with all playlist combinations
- [ ] Verify playlist switching preserves playback state correctly  
- [ ] Verify playlist management (create/rename/delete) handles file system errors
- [ ] Verify URL input playlist selection works with 1 and many playlists
- [ ] Test edge cases: empty playlists, corrupted files, permission issues
- [ ] Run full test suite: `cargo test`
- [ ] Verify backward compatibility with existing playlist files

### Task 7: [Final] Update documentation

- [ ] Update README.md with new playlist management features if needed
- [ ] Update CLAUDE.md with discovered patterns for future reference
- [ ] Move this plan to `docs/plans/completed/`

## Post-Completion
*Items requiring manual intervention or external systems - no checkboxes, informational only*

**Manual verification**:
- Test UI/UX flow with realistic playlist collections (10+ playlists, 50+ tracks)
- Verify keyboard shortcuts are discoverable and intuitive
- Test performance with large playlists during context menu operations
- Verify all edge cases work smoothly in actual usage scenarios

**No external system updates needed** - this is purely internal playlist management functionality