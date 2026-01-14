# Podcast Index Monitor

This command line application monitors the Podcast Index API for new feeds and
allows rudimentary analysis of them.  This is a work in progress.

## Key bindings
- `q` to quit
- `r` to refresh the feed list
- `p` to play the latest enclosure for the selected feed
- `x` to view the RSS xml for the selected feed
- `d` to mark a feed as problematic
- `Esc` to exit a dialog or stop 
- (on vim mode) `?` to show help and vim key bindings

## Features / Options
* `--config` command-line flag to allow pimonitor to read configuration from a custom file path instead of the default `pimonitor.yaml`.
* `--vim` command-line flag to enable vim mode. Press `?` to show what it does.

## Planned features
- Add a feed
- Rescan a feed
- Adjust apple directory linkage
- Mark a feed as dead
- Mark a feed as spam
- Plugin architecture for automation

## LLM/AI guidelines
This is an LLM/AI friendly repository.  When coding on this repository using LLM/AI agents
please follow the guidelines in the AGENTS.md file.

## Dependencies
### Rust
You need `rust` in order to lauch/compile this tool.
- In [ArchLinux](https://wiki.archlinux.org/title/Rust) you can just install `rust` or `rustup` (recommended if you intend to do development) with : `pacman -S rust`

### Podcast Index API Key
Optional functionality: In order to interact with the Index to report a problem (`d` key) you need to configure `pimonitor.yaml` with your [PI key](https://api.podcastindex.org/):
```yaml
pi_api_key: ""
pi_api_secret: ""
```

## Installation
You can just clone this repo and run the tool with:
```bash
git clone https://github.com/Podcastindex-org/pimonitor.git && cd "$(basename "$_" .git)"
cargo run
```

## Usage examples
* Using vim mode
```bash
cargo run -- --vim
```
* Using custom configuration file
```bash
cargo run -- --config /etc/pimonitor/config.yaml
```
* Using custom config in vim mode
```bash
cargo run -- --vim --config ./custom.yaml
```

## Screenshot
<img width="1681" height="1325" alt="imagen" src="https://github.com/user-attachments/assets/3c69ba44-c208-473e-b0be-67804628f351" />
