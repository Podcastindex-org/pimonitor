# Vim Keybindings Implementation

**Date:** 2026-01-09
**Last Updated:** 2026-01-09

## Summary
Added vim-style keyboard navigation to the pimonitor TUI application, enabled via a `--vim` command line flag. Includes help modal, simplified status bar, and intelligent play/pause functionality.

## Changes Made

### 1. Command Line Argument Parsing
- Added argument parsing to detect `--vim` flag
- Parse arguments in `main()` function before initializing the application

### 2. AppState Structure
- Added `vim_mode: bool` field to `AppState` struct
- Modified `AppState::new()` to accept and store the vim_mode parameter
- Added `help_modal: bool` field to track help window visibility
- Added `playing_feed_id: Option<u64>` field to track which feed is currently playing
- Added `playing_feed_title: Option<String>` field to display currently playing podcast name
- Added `volume: f32` field to track playback volume (default 1.0 = 100%)

### 3. Vim Key Bindings
Implemented the following vim-style keybindings when `--vim` flag is enabled:

#### Navigation
- **j**: Move down (equivalent to Down arrow)
- **k**: Move up (equivalent to Up arrow)
- **h**: Move to beginning (equivalent to Home)
- **l**: Move to end (equivalent to End)
- **Ctrl-n**: Next item/scroll down (equivalent to Down arrow)
- **Ctrl-p**: Previous item/scroll up (equivalent to Up arrow)

#### Actions
- **Space**: Intelligent Play/Pause toggle
  - If the same feed is playing: pauses or resumes playback
  - If a different feed is selected: stops current playback and starts playing the newly selected feed
  - If no audio is loaded: starts playing the latest episode of the selected feed
- **-**: Volume down (decreases by 10%, minimum 0%)
- **=**: Volume up (increases by 10%, maximum 200%)
- **?**: Toggle help modal (shows/hides vim keybindings reference)

### 4. Modal Scroll Support
All vim keybindings work correctly in modal contexts:
- XML viewer modal
- Feed details popup modal
- Main feed list

### 5. Conditional Activation
- Vim bindings only activate when `vim_mode` is true
- Standard keybindings (arrow keys, 'p', etc.) continue to work regardless of vim mode
- This allows users to choose their preferred navigation style

### 6. Help Modal System
- Press `?` to display a comprehensive help window with all vim keybindings
- Help modal can be closed with `?` or `Esc`
- Shows navigation commands, actions, and other key mappings
- Styled with color-coded sections for easy reference

### 7. Simplified Status Bar (Vim Mode)
- When `--vim` flag is enabled, status bar shows minimal commands: `q: quit  ?: help`
- All detailed keybindings moved to the help modal
- Keeps interface clean and distraction-free
- Standard mode still shows full command list in status bar

### 8. Smart Playback Tracking
- Application tracks which feed is currently playing via `playing_feed_id` and `playing_feed_title`
- Enables intelligent play/pause behavior based on feed selection
- Automatically stops previous feed when switching to a new one
- `stop_playback()` properly clears the tracked feed ID and title
- Status bar displays currently playing podcast with format: "Playing: [ID] Title [MM:SS]"
- Elapsed playback time shown in minutes:seconds format
- Playing info appears in the status bar after the status message, separated by " | "
- Styled with green bold text for easy visibility
- Playback timer updates continuously while audio is playing

### 9. Panic-Safe Audio Playback
- Wrapped rodio/symphonia decoder initialization in panic handler
- Catches decoder panics and converts them to friendly error messages
- Prevents application crashes when encountering unsupported audio formats
- Shows "Audio decoder panic - this audio format may not be supported" error
- Application continues running even if playback fails

### 10. Volume Control
- Added volume control using `-` (decrease) and `=` (increase) keys
- Volume adjusts in 10% increments
- Range: 0% (mute) to 200% (amplified)
- Volume changes apply immediately to playing audio
- Status message displays current volume percentage
- Volume state persists across feed changes
- Help modal updated with volume control keybindings

## Usage

Run the application with vim keybindings:
```bash
cargo run -- --vim
# or with nix:
nix run .#vim
```

Run the application with standard keybindings:
```bash
cargo run
# or with nix:
nix run
```

Build the application:
```bash
cargo build
# or with nix:
nix build
```

## Testing Recommendations
- Test all vim keybindings in the main feed list
- Test vim keybindings within the XML viewer modal
- Test vim keybindings within the feed details popup
- Verify that standard keybindings still work in both modes
- Test Ctrl-p and Ctrl-n combinations
- Test intelligent space bar play/pause functionality:
  - Press space to start playback on feed A
  - Press space again to pause feed A
  - Press space once more to resume feed A
  - Navigate to feed B and press space (should stop A and play B)
  - Verify status messages update correctly
- Test help modal:
  - Press `?` to open help
  - Press `?` or `Esc` to close help
  - Verify all keybindings are documented
- Test status bar simplification in vim mode vs standard mode
