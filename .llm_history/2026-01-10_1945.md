# LLM Changes: 2026-01-10 - Add --config Flag for Custom Configuration Path

## Summary
Added a `--config` command-line flag to allow pimonitor to read configuration from a custom file path instead of the default `pimonitor.yaml`. The flag includes validation for file existence and readability.

## Changes Made

### 1. Global CONFIG_PATH Static
- Added `static CONFIG_PATH: Lazy<Mutex<PathBuf>>` to store the configuration file path
- Defaults to `pimonitor.yaml` in the current directory
- Can be overridden via the `--config` command-line flag
- Uses `Mutex` for thread-safe access, mirroring the pattern used by `POLL_INTERVAL` and `PI_READ_WRITE`

### 2. Command-Line Argument Parsing in main()
- Added parsing of `--config` flag before checking for TTY
- Validates that the specified config file exists
- Validates that the config file is readable by attempting to read it
- Updates the global `CONFIG_PATH` on successful validation
- Returns early with error messages if validation fails:
  - File does not exist
  - File cannot be read (permission issue or other I/O error)
  - Missing path argument after `--config` flag

### 3. Updated Configuration Reading Functions
- **read_yaml_config()**: Modified to read the config path from the global `CONFIG_PATH`
- **load_pi_creds()**: Modified to read the config path from the global `CONFIG_PATH`
- Both functions fall back to the default path if mutex locking fails

### 4. Unit Tests Added
Created 5 new unit tests in `src/main.rs`:
- `config_path_file_exists_and_readable()`: Verifies file existence and readability
- `config_path_nonexistent_file()`: Tests behavior with non-existent files
- `load_pi_creds_from_custom_path()`: Validates loading credentials from a custom config path
- `load_pi_creds_missing_secret()`: Ensures validation fails when credentials are incomplete

## Usage Examples

### Default behavior (unchanged)
```bash
cargo run
```

### Using custom configuration file
```bash
cargo run -- --config /etc/pimonitor/config.yaml
```

### Using custom config in vim mode
```bash
cargo run -- --vim --config ./custom.yaml
```

## Validation & Testing
- ✅ All 10 unit tests pass (4 new + 6 existing)
- ✅ All existing integration tests pass
- ✅ Build succeeds with no warnings or errors
- ✅ File existence and readability are validated before use
- ✅ Global CONFIG_PATH is safely updated via Mutex

## Backward Compatibility
- ✅ Fully backward compatible
- ✅ Default behavior unchanged when `--config` flag is not provided
- ✅ Existing configuration files continue to work without modification
