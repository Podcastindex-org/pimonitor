AGENTS GUIDE – pimonitor (Rust TUI)

Line count target: ~150 lines. Keep this file up to date when repo conventions change.

Context
- Language/tooling: Rust 2024 edition; async with tokio; UI via ratatui + crossterm; HTTP via reqwest rustls; serde/serde_yaml/json; audio via rodio/symphonia; hashing via sha1; fft via rustfft; once_cell for globals; anyhow for errors.
- Binary: `pimonitor` (TUI). Config: `pimonitor.yaml` in repo root by default; `--config` CLI flag can override path; `--vim` enables vim-mode bindings.
- Tests live under `tests/`; integration tests rely on public APIs exported from `src/lib.rs`.
- No Cursor/Copilot rules found (no .cursorrules/.cursor/rules or .github/copilot-instructions.md). Follow guidelines below instead.

Build / Run / Test
- Build: `cargo build` (fix all warnings/errors before proceeding).
- Run default config: `cargo run`.
- Run with vim mode: `cargo run -- --vim`.
- Run with custom config: `cargo run -- --config path/to/config.yaml` (errors if missing/unreadable).
- Test suite: `cargo test` (from repo root).
- Single test (pattern): `cargo test name_substring` (e.g., `cargo test search_modal`).
- Single test in file: `cargo test --test test_search numeric_id_search`.
- Avoid running inside a non-TTY terminal for the TUI binary; main() exits early otherwise.

Lint / Format
- No rustfmt config present; use default `cargo fmt` before sending PRs/patches.
- No clippy config committed; optional: `cargo clippy --all-targets -- -D warnings` if available locally.
- Keep output warnings at zero; adjust code instead of allowing warnings to accumulate.

Coding Style (Rust)
- Imports: group std first, then external crates, then local crate (`pimonitor::…`). Use explicit paths; avoid wildcards. Keep ordering stable (alphabetical within groups where practical).
- Types: prefer concrete structs/tuples over ad-hoc maps; keep UI state in `AppState`. Use `Option`/`Result` instead of sentinel values.
- Errors: return `anyhow::Result` from fallible functions; avoid `unwrap`/`expect` in production paths (ok in tests). Surface user-facing issues via status messages or `eprintln!` as already used.
- Concurrency: use `tokio::spawn` for background work; channel types are `tokio::sync::mpsc::UnboundedSender/Receiver`. Mind `Send` bounds before moving into tasks.
- Async timing: polling uses `tokio::time::interval`; keep tick creation aligned with updated poll interval.
- State: update `AppState` carefully to preserve scroll selection, audio handles, and modal state; keep cursor position visible after list refreshes.
- Naming: snake_case for functions/vars, CamelCase for types; prefer descriptive identifiers (`reason_modal`, `search_input`, `pending_problem_feed_id`).
- Pattern matching: prefer `if let`/`match` over indexing panics; clamp indices when accessing feeds list.
- Borrowing: avoid unnecessary clones; prefer slices/refs where possible, but clone Feed when reinserting flagged items as done in main loop.
- Time: use `chrono` for timestamps and `std::time::Instant` for durations; store poll interval in seconds as `u64`.

Configuration Rules (`pimonitor.yaml`)
- If missing at startup and using default path in CWD, create file with blank `pi_api_key` and `pi_api_secret`; omit `poll_interval` by default.
- Read config before each polling interval; use the path in global `CONFIG_PATH` (set by `--config`).
- Keys: `pi_api_key`, `pi_api_secret`, `poll_interval` (optional). `poll_interval` must be an integer > 30 seconds; otherwise default to 60 seconds. Store chosen interval in global `POLL_INTERVAL`.
- Set global `PI_READ_WRITE` to true only when both key and secret are present and non-empty; otherwise false.

Polling for New/Recent Feeds
- Request params: `since` = current Unix time minus 86,400 seconds (24h); `max` = 500.
- Response handling: each feed has `dead` field (bool or 0/1). Hide feeds where `dead` is true/1. Preserve cursor/selection after refresh; keep selection visible by clamping scroll.
- Mark feeds added after first minute with transient “new” marker (`*`) for ~60s.
- Preserve flagged feeds locally even if API response drops them; reinject with stored reason.

Search Behavior (`s` key)
- Opens small dialog; Enter triggers search; empty term does nothing.
- If term is all digits and >0, treat as feed ID and jump to first matching feed; otherwise split into words, all must appear (case-insensitive substring) in title.
- On match: jump selection and adjust scroll to keep item visible; on miss: print error to console and set status message.

Problematic Reporting (`d` key)
- Show reason selection dialog; allow number keys 0-6, arrows + Enter; Esc cancels.
- Map reasons exactly: 0 `No Reason`, 1 `Spam`, 2 `AI Slop`, 3 `Illegal Content`, 4 `Duplicate`, 5 `Malicious Payload`, 6 `Feed Hijack`.
- POST to `https://api.podcastindex.org/api/1.0/report/problematic` with url params `id=<feed_id>` and `reason=<code>`; requires API key/secret from config.
- On success: refresh feeds (poll again). On failure: print error to console and show status.

Vim Mode (`--vim` flag)
- Navigation: `j/k` down/up, `h/l` home/end, `g/G` top/bottom, `0/$` start/end, `Ctrl-u/d` half-page, `Ctrl-n/p` next/prev.
- Actions: `Space` play/pause, `-/=` volume, `Enter` details, `x` XML, `s` search, `r` refresh, `d` report problematic.
- `?` toggles help modal; `q` quits immediately from anywhere (including modals).
- Flagged feeds show `[FLAG reason]` label but no strike-through styling.

Audio / Playback
- Use rodio OutputStream/Sink; always stop playback and drop temp files when done. Keep EQ analyzer gated on playback.
- Detect playback end via `sink.empty()` and call `stop_playback()`.
- For vim mode, duration display is enabled; avoid heavy duration work otherwise.

XML Viewer / Popups
- Use `centered_rect` helper for modal layout. Clear area before drawing popups for readability.
- Keep scroll bounds within widget height; use `Wrap` with `trim` where appropriate.

Error Handling & Logging
- User-facing errors: prefer `status_msg` updates; critical failures log via `eprintln!`.
- Network failures should not crash TUI; send status update through channel.
- Parsing YAML: on error, keep defaults (interval=60, read_write=false); do not panic.

Testing Expectations
- When adding functions, add matching tests in `tests/` (integration) or inline `#[cfg(test)]` unit tests if they need private access.
- Use public helpers in `src/lib.rs` for integration tests (e.g., `reason_options`, `find_feed_index_by_query`).
- Keep tests deterministic; avoid network I/O.

LLM/AI Contributions
- Every LLM/AI change must add a new datestamped note in `.llm_history/` describing the changes; include that file in commits.
- Follow repo instructions above for functionality; do not bypass TUI/UX constraints.

Git Hygiene (for agents)
- Do not force-push; avoid amending unless explicitly requested. Keep worktree clean before handing off. Respect existing user changes; do not revert unrelated edits.

Dependencies / Environment
- Requires Rust toolchain with Cargo; uses rustls TLS (no OpenSSL dep). Audio stack may need system audio libs (rodio/symphonia). Nix flake is present for dev-shell (`direnv` available).

Checks to Run Before Hand-off
- `cargo fmt`
- `cargo build`
- `cargo test`
- Optional: `cargo clippy --all-targets -- -D warnings` if available

Tips for UI Work
- Maintain selection visibility when list size changes (clamp scroll/selection to viewport). Keep status bar informative (commands + status message + playing info).
- Avoid blocking the UI loop; use non-blocking poll for input and channels for background work.

Adding New Features
- Keep public helpers in `src/lib.rs` when tests need access. Expose minimal API surface for tests.
- Match existing keybinding patterns; update README and status hints if keys change.

Release Notes
- None required here; just keep this guide synchronized with behavior and tests.
