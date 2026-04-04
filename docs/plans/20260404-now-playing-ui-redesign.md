# Now Playing UI Redesign

## Overview
- Redesign the now playing area (bottom 3 rows) to match modern media player aesthetics with a clean header-centric layout
- Fix existing layout calculation bugs in text truncation and spacing
- Ensure complete UI consistency across all components while maintaining the pirate theme color palette  
- Improve information hierarchy to make track details more prominent and scannable

## Context (from discovery)
- **files/components involved**: 
  - `src/tui/ui.rs`: render_now_playing function and helper functions (build_progress_bar, truncate, format_duration)
  - Layout structure: 4-row vertical split with now playing occupying rows[2] (Length(3))
- **related patterns found**: 
  - Existing color scheme: ACCENT (red-orange), GOLD (yellow), SEA_GREEN (teal), TEXT_DIM (gray), BORDER_IDLE (dark gray)
  - Current 3-row inner layout: title/speed → progress/volume → cache status
  - Layout bugs: meta_max/pad calculation mismatch, unused _empty_color parameter
- **dependencies identified**: 
  - App playback state: position, is_paused, player, download_progress
  - Track data: title, artist, source, user_title, user_artist, cache_status

## Development Approach
- **testing approach**: TDD - write tests first for helper functions and layout calculations
- complete each task fully before moving to the next
- make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
  - tests are not optional - they are a required part of the checklist
  - write unit tests for new functions/methods
  - write unit tests for modified functions/methods
  - add new test cases for new code paths
  - update existing test cases if behavior changes
  - tests cover both success and error scenarios
- **CRITICAL: all tests must pass before starting next task** - no exceptions
- **CRITICAL: update this plan file when scope changes during implementation**
- run tests after each change
- maintain backward compatibility

## Testing Strategy
- **unit tests**: required for every task - test helper functions, layout calculations, text formatting
- **manual testing**: visual verification in terminal at different window sizes
- **e2e tests**: N/A - this is a pure UI layout change without functional behavior changes
- store tests in `src/tui/ui_test.rs` (create new test file)

## Progress Tracking
- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope
- keep plan in sync with actual work done

## Solution Overview
- **Header-centric layout**: Row 1 becomes a proper header with "🎵 Now Playing" label, playback status, and speed
- **Clean track info**: Row 2 focuses purely on track metadata with better visual hierarchy
- **Comprehensive controls**: Row 3 integrates progress, time, volume, and cache status in organized sections
- **Fix layout bugs**: Address text truncation calculation issues and improve helper function consistency
- **UI consistency**: Apply consistent styling patterns and improve overall polish

## Technical Details
- **New 3-row layout**:
  - Row 1: `🎵 Now Playing` (GOLD) | `▶️ Playing` (white) | `1.4x` (ACCENT)
  - Row 2: `TRACK TITLE` (bold white) • `Artist` (TEXT_DIM) • `youtube.com/...` (TEXT_DIM, truncated)
  - Row 3: `0:03 ████████████████──── 830:34` | `♪ 85%` | `◈ Cached`
- **Helper function improvements**:
  - Fix `build_progress_bar` to properly use empty_color parameter
  - Improve `truncate` function to handle edge cases better
  - Add new layout calculation functions to prevent spacing bugs
- **Styling consistency**: Apply consistent border, spacing, and color patterns across all UI components

## What Goes Where
- **Implementation Steps** (`[ ]` checkboxes): tasks achievable within this codebase - code changes, tests, documentation updates
- **Post-Completion** (no checkboxes): items requiring external action - manual testing, changes in consuming projects, deployment configs, third-party verifications

## Implementation Steps

### Task 1: Create UI test infrastructure and test existing helpers

**Files:**
- Create: `src/tui/ui_test.rs`
- Modify: `src/tui/mod.rs` (add test module)

- [x] create `src/tui/ui_test.rs` with test module structure
- [x] write tests for existing `format_duration` function (various duration formats)
- [x] write tests for existing `truncate` function (edge cases: empty string, exact length, overflow)
- [x] write tests for existing `build_progress_bar` function (ratios, widths, fill patterns)
- [x] add test module declaration to `src/tui/mod.rs`
- [x] run tests: `cargo test ui_test` - must pass before task 2

### Task 2: Fix build_progress_bar helper function

**Files:**
- Modify: `src/tui/ui.rs` (build_progress_bar function)
- Modify: `src/tui/ui_test.rs`

- [x] modify `build_progress_bar` to properly use `empty_color` parameter instead of applying `fill_color` to entire span
- [x] create separate spans for filled and empty sections with correct colors
- [x] improve thumb positioning logic to handle edge cases better
- [x] write tests for new color behavior (filled vs empty sections)
- [x] write tests for thumb positioning edge cases
- [x] run tests: `cargo test ui_test` - must pass before task 3

### Task 3: Create layout calculation utilities

**Files:**
- Modify: `src/tui/ui.rs` (add new helper functions)
- Modify: `src/tui/ui_test.rs`

- [x] create `calculate_distributed_widths` function to handle proper spacing calculations
- [x] create `build_separated_line` function for clean bullet-separated text layout
- [x] create `format_playback_state` function to standardize status text
- [x] write tests for `calculate_distributed_widths` (various terminal widths)
- [x] write tests for `build_separated_line` (truncation priorities, separators)
- [x] write tests for `format_playback_state` (different player states)
- [x] run tests: `cargo test ui_test` - must pass before task 4

### Task 4: Implement new header row (Row 1)

**Files:**
- Modify: `src/tui/ui.rs` (render_now_playing_header function)
- Modify: `src/tui/ui_test.rs`

- [x] replace `render_now_playing_title` with new `render_now_playing_header` function
- [x] implement three-section layout: "🎵 Now Playing" (left, GOLD) | playback status (center) | speed (right, ACCENT)
- [x] use `calculate_distributed_widths` for proper spacing
- [x] handle edge cases: no track, fetching metadata, paused/playing states
- [x] write tests for header layout calculations
- [x] write tests for different playback states rendering
- [x] run tests: `cargo test ui_test` - must pass before task 5

### Task 5: Implement new track info row (Row 2)

**Files:**
- Create: `src/tui/ui.rs` (render_track_info_row function)
- Modify: `src/tui/ui.rs` (render_now_playing function)
- Modify: `src/tui/ui_test.rs`

- [x] create `render_track_info_row` function for clean track metadata display
- [x] implement bullet-separated layout: TITLE (bold) • Artist (dim) • source (dim, truncated)
- [x] use `build_separated_line` with proper truncation priority
- [x] handle user title/artist overrides consistently
- [x] update `render_now_playing` to call new function for row 1
- [x] write tests for track info formatting and truncation
- [x] write tests for user override handling
- [x] run tests: `cargo test ui_test` - must pass before task 6

### Task 6: Redesign progress/controls row (Row 3)

**Files:**
- Modify: `src/tui/ui.rs` (render_playback_bar function)
- Modify: `src/tui/ui_test.rs`

- [x] redesign `render_playback_bar` to integrate time, progress, volume, and cache status
- [x] implement section layout: `time progress time | volume | cache` 
- [x] use improved `build_progress_bar` with proper colors
- [x] integrate cache status directly instead of separate row
- [x] handle downloading state with inline progress display
- [x] write tests for integrated layout calculations
- [x] write tests for cache status integration
- [x] run tests: `cargo test ui_test` - must pass before task 7

### Task 7: Remove old cache row and update main render function

**Files:**
- Modify: `src/tui/ui.rs` (render_now_playing, remove render_cache_and_eq)
- Modify: `src/tui/ui_test.rs`

- [x] remove `render_cache_and_eq` function (functionality moved to row 3)
- [x] update `render_now_playing` to use new 3-row structure
- [x] ensure proper row allocation and spacing
- [x] clean up unused variables and calculations
- [x] write tests for overall render_now_playing integration
- [x] run tests: `cargo test ui_test` - must pass before task 8

### Task 8: Apply UI consistency improvements

**Files:**
- Modify: `src/tui/ui.rs` (overall styling consistency)
- Modify: `src/tui/ui_test.rs`

- [x] review and standardize border styles across all panels
- [x] ensure consistent spacing patterns in sidebar and track table
- [x] apply consistent color usage patterns throughout UI
- [x] improve visual hierarchy with better contrast and emphasis
- [x] write tests for styling consistency helpers
- [x] run tests: `cargo test ui_test` - must pass before task 9

### Task 9: Verify acceptance criteria and edge cases

**Files:**
- Modify: `src/tui/ui_test.rs`

- [ ] verify all requirements from Overview are implemented
- [ ] test edge cases: very narrow terminal, no tracks, long titles
- [ ] test all playback states: stopped, playing, paused, downloading
- [ ] verify color scheme consistency with pirate theme
- [ ] verify layout calculations work at minimum terminal dimensions
- [ ] run full test suite: `cargo test`
- [ ] manual testing: verify visual layout in terminal at different sizes
- [ ] run tests: `cargo test ui_test` - must pass before task 10

### Task 10: [Final] Update documentation

**Files:**
- Modify: `README.md` (if UI changes warrant documentation)
- Modify: `CLAUDE.md` (capture new UI patterns)

- [ ] update README.md if new UI patterns should be documented
- [ ] update CLAUDE.md with discovered layout calculation patterns
- [ ] document the new helper functions and their usage
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion
*Items requiring manual intervention or external systems - no checkboxes, informational only*

**Manual verification**:
- Test UI at various terminal sizes (minimum 80x24, recommended 120x30+)
- Verify readability with different terminal color schemes and fonts
- Test responsiveness when rapidly switching tracks or seeking
- Validate that all text truncation looks natural and preserves important information

**Performance considerations**:
- Monitor terminal rendering performance with new layout calculations
- Ensure no flickering during playback position updates
- Verify smooth progress bar animations