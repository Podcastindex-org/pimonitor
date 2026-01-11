use std::io::IsTerminal as _; // for stdout().is_terminal()
use std::time::Duration;
use std::{fs::File, path::{Path, PathBuf}};
use std::io::{Cursor, Read};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{BarChart, Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;
use serde::Deserialize;
use serde_json::Value;
use xml::reader::EventReader as XmlReader;
use xml::reader::XmlEvent;
use xml::writer::{EmitterConfig as XmlEmitterConfig, EventWriter as XmlWriter, XmlEvent as WriterXmlEvent};
use rodio::{OutputStream, Sink};
use rustfft::{num_complex::Complex32, FftPlanner};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;
use tokio::sync::mpsc;
use tokio::time::interval;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use sha1::{Sha1, Digest};
use pimonitor::{reason_options, reason_code_for_index};
use std::collections::{HashMap, HashSet};

// Global configuration flags populated from pimonitor.yaml
// Default polling interval is 60 seconds
static POLL_INTERVAL: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(60));
// Whether both API key and secret are present (read/write possible)
static PI_READ_WRITE: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
// Path to the config file (default: pimonitor.yaml, can be overridden with --config)
static CONFIG_PATH: Lazy<Mutex<PathBuf>> = Lazy::new(|| Mutex::new(PathBuf::from("pimonitor.yaml")));

#[derive(Debug, Deserialize)]
struct AppConfig {
    #[serde(default)]
    pi_api_key: Option<String>,
    #[serde(default)]
    pi_api_secret: Option<String>,
    #[serde(default)]
    poll_interval: Option<u64>,
}

/// Evaluate a parsed `AppConfig` into concrete settings.
/// Returns a tuple of `(poll_interval_secs, can_read_write)`.
fn evaluate_config(cfg: &AppConfig) -> (u64, bool) {
    // Defaults
    let mut interval_secs: u64 = 60;
    let key_ok = cfg
        .pi_api_key
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let secret_ok = cfg
        .pi_api_secret
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let can_rw = key_ok && secret_ok;

    if let Some(pi) = cfg.poll_interval {
        // Only accept if strictly greater than 30 seconds
        if pi > 30 {
            interval_secs = pi;
        }
    }

    (interval_secs, can_rw)
}

fn read_yaml_config() {
    // Read configuration from pimonitor.yaml before each polling cycle
    // Use the path from CONFIG_PATH global, which can be overridden with --config
    let path = if let Ok(cp) = CONFIG_PATH.lock() {
        cp.clone()
    } else {
        PathBuf::from("pimonitor.yaml")
    };
    
    let mut interval_secs: u64 = 60; // default
    let mut can_rw = false; // default to false unless both present

    if let Ok(file) = File::open(&path) {
        match serde_yaml::from_reader::<_, AppConfig>(file) {
            Ok(cfg) => {
                let (ivl, rw) = evaluate_config(&cfg);
                interval_secs = ivl;
                can_rw = rw;
            }
            Err(_) => {
                // On parse error, fall back to defaults defined above
            }
        }
    } else {
        // No file: keep defaults (60s, read/write = false)
    }

    // Persist into globals
    POLL_INTERVAL.store(interval_secs, Ordering::Relaxed);
    PI_READ_WRITE.store(can_rw, Ordering::Relaxed);
}

/// Ensure a default `pimonitor.yaml` exists on disk with blank values.
/// This creates a minimal YAML file containing empty credentials and no poll_interval,
/// only when the file does not already exist.
fn ensure_config_exists_at(path: &Path) -> Result<()> {
    if !path.exists() {
        // Create with blank values for keys. poll_interval is optional and omitted by default.
        let default_yaml = concat!(
            "pi_api_key: \"\"\n",
            "pi_api_secret: \"\"\n",
        );
        std::fs::write(path, default_yaml)?;
    }
    Ok(())
}

fn ensure_config_exists() -> Result<()> {
    ensure_config_exists_at(Path::new("pimonitor.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn mk_cfg(key: Option<&str>, secret: Option<&str>, poll: Option<u64>) -> AppConfig {
        AppConfig {
            pi_api_key: key.map(|s| s.to_string()),
            pi_api_secret: secret.map(|s| s.to_string()),
            poll_interval: poll,
        }
    }

    #[test]
    fn eval_config_both_keys_and_valid_interval() {
        let cfg = mk_cfg(Some("abc"), Some("def"), Some(120));
        let (secs, rw) = evaluate_config(&cfg);
        assert_eq!(secs, 120);
        assert!(rw);
    }

    #[test]
    fn eval_config_missing_key_and_too_low_interval() {
        let cfg = mk_cfg(Some("abc"), None, Some(30)); // 30 is not strictly greater than 30
        let (secs, rw) = evaluate_config(&cfg);
        assert_eq!(secs, 60); // default
        assert!(!rw);
    }

    #[test]
    fn eval_config_no_interval_defaults_to_60() {
        let cfg = mk_cfg(None, None, None);
        let (secs, rw) = evaluate_config(&cfg);
        assert_eq!(secs, 60);
        assert!(!rw);
    }

    #[test]
    fn eval_config_min_valid_interval_31() {
        let cfg = mk_cfg(None, None, Some(31));
        let (secs, _rw) = evaluate_config(&cfg);
        assert_eq!(secs, 31);
    }

    #[test]
    fn ensure_config_creates_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("pimonitor.yaml");
        assert!(!p.exists());
        ensure_config_exists_at(&p).expect("create config");
        assert!(p.exists());
        let contents = fs::read_to_string(&p).expect("read config");
        // Should contain blank key and secret lines, poll_interval omitted
        assert!(contents.contains("pi_api_key: \"\""));
        assert!(contents.contains("pi_api_secret: \"\""));
        assert!(!contents.contains("poll_interval"));
    }

    #[test]
    fn ensure_config_does_not_overwrite_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("pimonitor.yaml");
        // Pre-create with custom content
        let mut f = File::create(&p).expect("create file");
        write!(f, "pi_api_key: \"abc\"\npi_api_secret: \"def\"\npoll_interval: 45\n").unwrap();
        drop(f);
        let before = fs::read_to_string(&p).unwrap();
        ensure_config_exists_at(&p).expect("no overwrite");
        let after = fs::read_to_string(&p).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn config_path_file_exists_and_readable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("pimonitor.yaml");
        // Create a valid config file
        let mut f = File::create(&p).expect("create file");
        write!(f, "pi_api_key: \"test\"\npi_api_secret: \"test\"\n").unwrap();
        drop(f);
        
        // Verify file exists
        assert!(p.exists());
        
        // Verify file is readable
        let contents = fs::read(&p);
        assert!(contents.is_ok());
        assert!(!contents.unwrap().is_empty());
    }

    #[test]
    fn config_path_nonexistent_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("nonexistent.yaml");
        
        // Verify file does not exist
        assert!(!p.exists());
    }

    #[test]
    fn load_pi_creds_from_custom_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("custom_config.yaml");
        let mut f = File::create(&p).expect("create file");
        write!(f, "pi_api_key: \"my_key\"\npi_api_secret: \"my_secret\"\n").unwrap();
        drop(f);
        
        let creds = load_pi_creds_from(&p);
        assert!(creds.is_some());
        let (key, secret) = creds.unwrap();
        assert_eq!(key, "my_key");
        assert_eq!(secret, "my_secret");
    }

    #[test]
    fn load_pi_creds_missing_secret() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("incomplete_config.yaml");
        let mut f = File::create(&p).expect("create file");
        write!(f, "pi_api_key: \"my_key\"\npi_api_secret: \"\"\n").unwrap();
        drop(f);
        
        let creds = load_pi_creds_from(&p);
        // Should return None because secret is empty
        assert!(creds.is_none());
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct ApiResponse {
    // PodcastIndex sometimes returns `status` as a boolean (true/false) or as a string.
    // Use a flexible type so deserialization doesn't fail on type variation.
    #[serde(default)]
    status: Option<Value>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    feeds: Vec<Feed>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct Feed {
    id: Option<u64>,
    status: Option<String>,
    url: Option<String>,
    title: Option<String>,
    description: Option<String>,
    link: Option<String>,
    #[serde(rename = "timeAdded")]
    time_added: Option<i64>,
    #[serde(rename = "contentHash")]
    content_hash: Option<String>,
    language: Option<String>,
    image: Option<String>,
    #[serde(rename = "itunesId")]
    itunes_id: Option<u64>,
    source: Option<u32>,
    // The Podcast Index API includes a `dead` field indicating dead feeds.
    // It may be returned as a boolean or an integer (1/0). Use Value to be flexible.
    #[serde(default)]
    dead: Option<Value>,
}

#[derive(Debug, Clone)]
enum DataSource {
    Api,
    Empty,
}

// Do not derive Debug/Clone because we hold non-Clone, non-Debug audio handles
struct AppState {
    feeds: Vec<Feed>,
    scroll: usize,
    selected: usize,
    last_updated: Option<DateTime<Local>>, 
    source: DataSource,
    status_msg: String,
    show_popup: bool,
    // XML viewer state
    xml_show: bool,
    xml_text: String,
    // Modal scroll state
    xml_scroll: u16,
    popup_scroll: u16,
    // Audio playback state
    audio_stream: Option<OutputStream>,
    audio_sink: Option<Sink>,
    temp_audio_path: Option<PathBuf>,
    playing_feed_id: Option<u64>,
    playing_feed_title: Option<String>,
    playing_duration: Option<Duration>,
    volume: f32,
    playback_start: Option<std::time::Instant>,
    // EQ UI state
    eq_levels: [f32; 12],
    eq_visible: bool,
    // Problematic reason selection modal
    reason_modal: bool,
    reason_index: usize,
    pending_problem_feed_id: Option<u64>,
    // Vim mode flag
    vim_mode: bool,
    // Help modal
    help_modal: bool,
    // App start time and ephemeral new markers
    start_time: std::time::Instant,
    new_feed_marks: HashMap<u64, std::time::Instant>,
}

impl AppState {
    fn new(vim_mode: bool) -> Self {
        Self {
            feeds: Vec::new(),
            scroll: 0,
            selected: 0,
            last_updated: None,
            source: DataSource::Empty,
            status_msg: String::from("Press q to quit. Fetching…"),
            show_popup: false,
            xml_show: false,
            xml_text: String::new(),
            xml_scroll: 0,
            popup_scroll: 0,
            audio_stream: None,
            audio_sink: None,
            temp_audio_path: None,
            playing_feed_id: None,
            playing_feed_title: None,
            playing_duration: None,
            volume: 1.0,
            playback_start: None,
            eq_levels: [0.0; 12],
            eq_visible: false,
            reason_modal: false,
            reason_index: 0,
            pending_problem_feed_id: None,
            vim_mode,
            help_modal: false,
            start_time: std::time::Instant::now(),
            new_feed_marks: HashMap::new(),
        }
    }

    fn is_playing(&self) -> bool {
        self.audio_sink
            .as_ref()
            .map(|s| !s.is_paused())
            .unwrap_or(false)
            && self.audio_sink.as_ref().map(|s| !s.empty()).unwrap_or(false)
    }

    fn stop_playback(&mut self) {
        if let Some(sink) = self.audio_sink.take() {
            sink.stop();
        }
        // Drop stream last
        let _ = self.audio_stream.take();
        // Attempt to remove temp file if any
        if let Some(p) = self.temp_audio_path.take() {
            let _ = std::fs::remove_file(p);
        }
        self.playing_feed_id = None;
        self.playing_feed_title = None;
        self.playing_duration = None;
        self.playback_start = None;
        self.eq_visible = false;
        self.status_msg = "Playback stopped".into();
    }
}

// Helper to create a centered rectangle with a percentage of the given area
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);
    horizontal[1]
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let vim_mode = args.iter().any(|arg| arg == "--vim");
    
    // Parse --config flag for custom config file path
    let config_path = if let Some(pos) = args.iter().position(|arg| arg == "--config") {
        if pos + 1 < args.len() {
            let path = PathBuf::from(&args[pos + 1]);
            // Validate that the file exists and is readable
            if !path.exists() {
                eprintln!("Error: config file '{}' does not exist", path.display());
                return Ok(());
            }
            // Try to read the file to ensure it's readable
            match std::fs::read(&path) {
                Ok(_) => {
                    // Update the global CONFIG_PATH
                    if let Ok(mut cp) = CONFIG_PATH.lock() {
                        *cp = path.clone();
                    }
                    path
                }
                Err(e) => {
                    eprintln!("Error: cannot read config file '{}': {}", path.display(), e);
                    return Ok(());
                }
            }
        } else {
            eprintln!("Error: --config flag requires a path argument");
            return Ok(());
        }
    } else {
        // Use default path
        PathBuf::from("pimonitor.yaml")
    };

    // Ensure we're running in a real terminal (TTY). Many IDE "Run" consoles are not TTYs
    // and Ratatui won't be able to draw there, which looks like a blank window.
    if !std::io::stdout().is_terminal() {
        eprintln!(
            "This application must be run in a real terminal (TTY).\n\
             Please run from Windows Terminal, PowerShell, or cmd.exe.\n\
             Example:\n\
               cargo run\n"
        );
        return Ok(());
    }

    // Create default config file on startup if missing (only for default path)
    if config_path.file_name().map(|f| f == "pimonitor.yaml").unwrap_or(false) &&
       config_path.parent().map(|p| p.as_os_str().is_empty() || p == Path::new(".")).unwrap_or(true) {
        let _ = ensure_config_exists();
    }

    // Setup terminal UI
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    // Clear once so the first draw is visible even if previous output existed
    let _ = terminal.clear();

    let (tx, mut rx) = mpsc::unbounded_channel::<AppUpdate>();
    // UI message channel for async helpers like playback
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiMsg>();
    let mut app = AppState::new(vim_mode);

    // Spawn background task to periodically fetch
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        // Initial configuration load and ticker creation
        read_yaml_config();
        let mut current_secs = POLL_INTERVAL.load(Ordering::Relaxed);
        if current_secs <= 30 { current_secs = 60; }
        let mut ticker = interval(Duration::from_secs(current_secs));
        // First immediate fetch without waiting
        if let Err(e) = poll_for_new_feeds(tx_clone.clone()).await {
            let mut st = AppUpdate::new();
            st.status_msg = format!("Initial fetch failed: {}", e);
            let _ = tx_clone.send(st);
        }
        loop {
            // Read config right before each polling interval starts
            read_yaml_config();
            let new_secs = POLL_INTERVAL.load(Ordering::Relaxed);
            if new_secs != current_secs {
                current_secs = if new_secs > 30 { new_secs } else { 60 };
                ticker = interval(Duration::from_secs(current_secs));
            }
            ticker.tick().await;
            if let Err(e) = poll_for_new_feeds(tx_clone.clone()).await {
                let mut st = AppUpdate::new();
                st.status_msg = format!("Fetch failed: {}", e);
                let _ = tx_clone.send(st);
            }
        }
    });

    // UI loop
    let mut ui_tick = interval(Duration::from_millis(200));
    loop {
        // Determine current viewport rows for the list area so we can clamp scrolling.
        // This mirrors the same layout used in the draw pass below.
        let term_size = terminal.size()?;
        let term_rect = Rect::new(0, 0, term_size.width, term_size.height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),        // list
                Constraint::Length(4),     // status (2 inner lines; +2 borders)
            ])
            .split(term_rect);
        // Account for list block borders (top+bottom) when computing visible rows.
        // Each feed is rendered as TWO lines (image on first line; id, title, url on second),
        // so compute the number of visible ITEMS (feeds) as half of visible rows.
        let viewport_rows_lines: usize = chunks[0].height.saturating_sub(2) as usize;
        let viewport_items: usize = (viewport_rows_lines / 2).max(1);

        // Apply incoming state updates (data fetch)
        while let Ok(new_state) = rx.try_recv() {
            // Merge: replace data and status, keep scroll if possible
            let scroll = app.scroll;
            let selected = app.selected;
            // Preserve audio state across data updates
            let audio_stream = app.audio_stream.take();
            let audio_sink = app.audio_sink.take();
            let temp_audio_path = app.temp_audio_path.take();
            let playing_feed_id = app.playing_feed_id;
            let playing_feed_title = app.playing_feed_title.take();
            let playing_duration = app.playing_duration;
            let playback_start = app.playback_start;
            let volume = app.volume;
            // Capture previously seen feed IDs
            let mut prev_ids: HashSet<u64> = HashSet::new();
            for f in app.feeds.iter() {
                if let Some(id) = f.id { prev_ids.insert(id); }
            }
            // Apply incoming update
            app.feeds = new_state.feeds;
            app.last_updated = new_state.last_updated;
            app.source = new_state.source;
            app.status_msg = new_state.status_msg;
            // Mark newly added feeds with a temporary star if beyond first minute
            let now = std::time::Instant::now();
            if now.duration_since(app.start_time) >= std::time::Duration::from_secs(60) {
                for f in app.feeds.iter() {
                    if let Some(id) = f.id {
                        if !prev_ids.contains(&id) {
                            app.new_feed_marks.insert(id, now + std::time::Duration::from_secs(60));
                        }
                    }
                }
            }
            // Clamp preserved scroll so the last item remains visible when list shrinks.
            let max_scroll = app
                .feeds
                .len()
                .saturating_sub(viewport_items);
            app.scroll = scroll.min(max_scroll);
            // Clamp selected within bounds
            if app.feeds.is_empty() {
                app.selected = 0;
            } else {
                app.selected = selected.min(app.feeds.len() - 1);
            }
            // Restore audio state and playback tracking
            app.audio_stream = audio_stream;
            app.audio_sink = audio_sink;
            app.temp_audio_path = temp_audio_path;
            app.playing_feed_id = playing_feed_id;
            app.playing_feed_title = playing_feed_title;
            app.playing_duration = playing_duration;
            app.playback_start = playback_start;
            app.volume = volume;
        }

        // Handle UI messages like playback
        while let Ok(msg) = ui_rx.try_recv() {
            match msg {
                UiMsg::Status(s) => {
                    app.status_msg = s;
                }
                UiMsg::XmlReady(xml) => {
                    app.xml_text = xml;
                    app.xml_show = true;
                    app.xml_scroll = 0;
                    app.status_msg = "XML loaded".into();
                }
                UiMsg::XmlError(e) => {
                    app.status_msg = format!("XML error: {}", e);
                    app.xml_show = false;
                    app.xml_scroll = 0;
                }
                UiMsg::PlayReady(path) => {
                    match start_playback_from_file(&path) {
                        Ok((stream, sink)) => {
                            app.temp_audio_path = Some(path.clone());
                            app.audio_stream = Some(stream);
                            app.audio_sink = Some(sink);
                            app.playback_start = Some(std::time::Instant::now());
                            // Calculate duration only if vim mode (optimization/strict adherence)
                            if app.vim_mode {
                                app.playing_duration = get_duration_from_file(&path);
                            }
                            // Start EQ analyzer in background
                            if let Some(p) = app.temp_audio_path.clone() {
                                let ui_tx3 = ui_tx.clone();
                                tokio::spawn(async move {
                                    let _ = spawn_eq_analyzer(p, ui_tx3).await;
                                });
                            }
                            app.eq_visible = true;
                        }
                        Err(e) => {
                            app.status_msg = format!("Failed to start playback: {}", e);
                            // cleanup file
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
                UiMsg::PlayError(e) => {
                    app.status_msg = format!("Playback error: {}", e);
                }
                UiMsg::EqUpdate(bands) => {
                    // Simple smoothing to avoid flicker
                    for i in 0..12 {
                        let old = app.eq_levels[i];
                        app.eq_levels[i] = old * 0.6 + bands[i] * 0.4;
                    }
                    app.eq_visible = true;
                }
                UiMsg::EqEnd => {
                    // Only hide EQ if playback has stopped
                    if app.audio_sink.is_none() {
                        app.eq_visible = false;
                        app.eq_levels = [0.0; 12];
                    }
                }
            }
        }

        // Also clamp scroll on window resizes (no new state), to keep last item visible.
        let max_scroll_now = app
            .feeds
            .len()
            .saturating_sub(viewport_items);
        if app.scroll > max_scroll_now {
            app.scroll = max_scroll_now;
        }

        terminal.draw(|f| {
            let size = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                        Constraint::Min(1), // list
                        Constraint::Length(4), // status (2 inner lines; +2 borders)
                    ])
                    .split(size);
            // Prune expired new markers
            let now = std::time::Instant::now();
            app.new_feed_marks.retain(|_, exp| *exp > now);

            
            // Render scrolling by using a viewport via Paragraph/List with offset isn't native; we can slice
            let visible = if app.scroll < app.feeds.len() {
                &app.feeds[app.scroll..]
            } else {
                &[]
            };
            let vis_items: Vec<ListItem> = visible
                .iter()
                .map(|feed| {
                    let title = feed.title.clone().unwrap_or_else(|| "<untitled>".into());
                    let lang_str = feed.language.clone().unwrap_or_else(|| "n/a".into());
                    let link = feed.link.clone().unwrap_or_else(|| "<no link>".into());
                    let url = feed.url.clone().unwrap_or_default();
                    let _image = feed.image.clone().unwrap_or_default();
                    let id_str = feed
                        .id
                        .map(|i| i.to_string())
                        .unwrap_or_else(|| "?".into());
                    let now = std::time::Instant::now();
                    let is_new_mark = feed.id
                        .and_then(|i| app.new_feed_marks.get(&i).copied())
                        .map(|exp| exp > now)
                        .unwrap_or(false);

                    // First line: [id]. [title] ([language]) - [url]
                    let mut parts = Vec::new();
                    if is_new_mark { parts.push(Span::styled("*", Style::default().fg(Color::Green))); parts.push(Span::raw(" ")); }
                    parts.extend_from_slice(&[
                        Span::styled(format!("{}.", id_str), Style::default().fg(Color::Yellow)),
                        Span::raw(" "),
                        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" ({})", lang_str), Style::default().fg(Color::LightMagenta)),
                        Span::raw(" - "),
                        Span::styled(url, Style::default().fg(Color::Cyan)),
                    ]);
                    let line1 = Line::from(parts);

                    // Second line: link
                    let line2 = Line::from(vec![
                        Span::styled(format!("    {}.", link), Style::default().fg(Color::LightMagenta)),
                    ]);

                    ListItem::new(vec![line1, line2])
                })
                .collect();

            let mut list_state = ratatui::widgets::ListState::default();
            // Selected relative to current scroll viewport
            if !app.feeds.is_empty() && app.selected >= app.scroll {
                let rel = app.selected - app.scroll;
                list_state.select(Some(rel));
            } else {
                list_state.select(None);
            }

            let list = List::new(vis_items)
                .block(Block::default().borders(Borders::ALL).title("Recent New Feeds"))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("▶ ");
            f.render_stateful_widget(list, chunks[0], &mut list_state);

            let updated = app
                .last_updated
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "n/a".into());
            let src = match app.source {
                DataSource::Api => "API",
                DataSource::Empty => "-",
            };
            // Status widget: commands on the top line, status message on the bottom line
            let status_text = vec![
                // Top line: key commands and metadata
                Line::from(vec![
                    Span::raw(if app.vim_mode {
                        "q: quit  ?: help  "
                    } else {
                        "q: quit  r: refresh  p: play latest  x: view XML  d: report problematic  Esc: stop/close  ↑/↓: select  Enter: open  PgUp/PgDn/Home/End: nav  "
                    }),
                    Span::styled(
                        format!("Updated: {}  Source: {}", updated, src),
                        Style::default().fg(Color::Yellow),
                    ),
                ]),
                // Bottom line: status message and playing info
                Line::from({
                    let mut spans = vec![Span::raw(&app.status_msg)];
                    // Add currently playing podcast info if available
                    if let (Some(id), Some(title)) = (&app.playing_feed_id, &app.playing_feed_title) {
                        spans.push(Span::raw("  | "));
                        let elapsed = app.playback_start
                            .map(|start| start.elapsed().as_secs())
                            .unwrap_or(0);
                        let minutes = elapsed / 60;
                        let seconds = elapsed % 60;
                        
                        let duration_str = if app.vim_mode {
                            if let Some(d) = app.playing_duration {
                                let total_secs = d.as_secs();
                                let t_min = total_secs / 60;
                                let t_sec = total_secs % 60;
                                format!(" / {}:{:02}", t_min, t_sec)
                            } else {
                                "".to_string()
                            }
                        } else {
                            "".to_string()
                        };

                        spans.push(Span::styled(
                            format!("Playing: [{}] {} [{}:{:02}{}]", id, title, minutes, seconds, duration_str),
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                        ));
                    }
                    spans
                }),
            ];
            let status = Paragraph::new(status_text)
                .block(
                    Block::default().borders(Borders::ALL)
                        .title("Status")
                )
                .wrap(Wrap { trim: true });
            f.render_widget(status, chunks[1]);

            // If popup requested, draw it centered over everything
            if app.show_popup {
                let area = centered_rect(70, 60, size);
                let feed_opt = app.feeds.get(app.selected);
                if let Some(feed) = feed_opt {
                    let title = feed.title.clone().unwrap_or_else(|| "<untitled>".into());
                    let desc = feed.description.clone().unwrap_or_else(|| "<no description>".into());
                    let popup_text = vec![
                        Line::from(Span::styled(title.clone(), Style::default().add_modifier(Modifier::BOLD))),
                        Line::from(""),
                        Line::from(desc),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Esc: close  ↑/↓ PgUp/PgDn Home/End: scroll",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ];
                    let popup = Paragraph::new(popup_text)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_style(Style::default().fg(Color::Yellow))
                                .title(Span::styled("Feed Details", Style::default().fg(Color::Yellow)))
                        )
                        .wrap(Wrap { trim: true })
                        .scroll((app.popup_scroll, 0));
                    // Clear the area first so the popup is readable
                    f.render_widget(Clear, area);
                    f.render_widget(popup, area);
                }
            }

            // XML modal viewer
            if app.xml_show {
                let area = centered_rect(80, 70, size);
                let title = "Feed XML";
                let mut lines: Vec<Line> = Vec::new();
                // Show up to some reasonable number of lines; Paragraph will clip nonetheless
                for l in app.xml_text.lines() {
                    lines.push(Line::from(l.to_string()));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Esc: close  ↑/↓ PgUp/PgDn Home/End: scroll",
                    Style::default().fg(Color::DarkGray),
                )));
                let popup = Paragraph::new(lines)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Yellow))
                            .title(Span::styled(title, Style::default().fg(Color::Yellow)))
                    )
                    .wrap(Wrap { trim: false })
                    .scroll((app.xml_scroll, 0));
                f.render_widget(Clear, area);
                f.render_widget(popup, area);
            }

            // Problematic reason selection modal
            if app.reason_modal {
                let area = centered_rect(60, 60, size);
                let reasons = reason_options();
                let mut items: Vec<ListItem> = Vec::new();
                for (label, code) in reasons.iter() {
                    let line = Line::from(vec![
                        Span::styled(format!("{}: {}", code, label), Style::default()),
                    ]);
                    items.push(ListItem::new(line));
                }
                let mut list_state = ratatui::widgets::ListState::default();
                list_state.select(Some(app.reason_index));
                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Yellow))
                            .title(Span::styled("Select reason (Enter=confirm, Esc=cancel)", Style::default().fg(Color::Yellow))),
                    )
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                    .highlight_symbol("▶ ");
                f.render_widget(Clear, area);
                f.render_stateful_widget(list, area, &mut list_state);
            }

            // Help modal (vim mode)
            if app.help_modal {
                let area = centered_rect(70, 70, size);
                let help_text = vec![
                    Line::from(Span::styled("Vim Mode Key Bindings", Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow))),
                    Line::from(""),
                    Line::from(Span::styled("Navigation:", Style::default().add_modifier(Modifier::BOLD))),
                    Line::from("  j / k       - Move down / up"),
                    Line::from("  h / l       - Jump to beginning / end"),
                    Line::from("  Ctrl-n / p  - Next / previous"),
                    Line::from("  PgUp/PgDn   - Page up / down"),
                    Line::from("  Home/End    - Jump to start / end"),
                    Line::from(""),
                    Line::from(Span::styled("Actions:", Style::default().add_modifier(Modifier::BOLD))),
                    Line::from("  Space       - Play/pause toggle"),
                    Line::from("  -           - Volume down"),
                    Line::from("  =           - Volume up"),
                    Line::from("  Enter       - View feed details"),
                    Line::from("  x           - View feed XML"),
                    Line::from("  r           - Refresh feed list"),
                    Line::from("  d           - Report feed as problematic"),
                    Line::from(""),
                    Line::from(Span::styled("Other:", Style::default().add_modifier(Modifier::BOLD))),
                    Line::from("  q           - Quit"),
                    Line::from("  Esc         - Close modal / stop playback"),
                    Line::from("  ?           - Show this help"),
                    Line::from(""),
                    Line::from(Span::styled("Indicators:", Style::default().add_modifier(Modifier::BOLD))),
                    Line::from("  *           - New feed marker (shows for 1 minute)"),
                    Line::from("                Appears for feeds fetched after the first minute"),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press Esc or ? to close",
                        Style::default().fg(Color::DarkGray),
                    )),
                ];
                let popup = Paragraph::new(help_text)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Yellow))
                            .title(Span::styled("Help", Style::default().fg(Color::Yellow)))
                    )
                    .wrap(Wrap { trim: true });
                f.render_widget(Clear, area);
                f.render_widget(popup, area);
            }

            // Draw EQ widget in bottom-right corner only while audio is playing
            if app.is_playing() {
                // For 12 bars with bar_width=2 and gap=1 → ~12*3=36 + borders => ~40-44
                let eq_width: u16 = 44;
                let eq_height: u16 = 10;
                // Place it above the status bar in the list area, anchored bottom-right
                let list_area = chunks[0];
                let eq_x = list_area.x + list_area.width.saturating_sub(eq_width + 1);
                let eq_y = list_area.y + list_area.height.saturating_sub(eq_height + 1);
                let eq_area = Rect { x: eq_x, y: eq_y, width: eq_width, height: eq_height };

                // Build 12-band labels and values (normalized 0..100)
                let band_names: [&str; 12] = [
                    "B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "B9", "B10", "B11", "B12",
                ];
                let mut data: Vec<(&str, u64)> = Vec::with_capacity(12);
                for i in 0..12 {
                    let v = (app.eq_levels[i].clamp(0.0, 1.0) * 100.0) as u64;
                    data.push((band_names[i], v));
                }
                let barchart = BarChart::default()
                    .block(Block::default().borders(Borders::ALL).title("EQ"))
                    .bar_width(2)
                    .bar_gap(1)
                    .data(data.as_slice())
                    .max(100)
                    .value_style(Style::default().fg(Color::Green));
                // Clear area so it overlays cleanly
                f.render_widget(Clear, eq_area);
                f.render_widget(barchart, eq_area);
            }
        })?;

        // Input handling with non-blocking poll
        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    // If reason selection modal is open, handle its navigation first
                    if app.reason_modal {
                        match k.code {
                            KeyCode::Esc => {
                                app.reason_modal = false;
                                app.pending_problem_feed_id = None;
                                app.status_msg = "Canceled problematic report".into();
                            }
                            KeyCode::Up => {
                                if app.reason_index > 0 { app.reason_index -= 1; }
                            }
                            KeyCode::Down => {
                                // Clamp to last index of reasons list
                                let last = reason_options().len().saturating_sub(1);
                                if app.reason_index < last { app.reason_index += 1; }
                            }
                            // Direct numeric selection (0-6) per AGENTS.md
                            KeyCode::Char('0') | KeyCode::Char('1') | KeyCode::Char('2') |
                            KeyCode::Char('3') | KeyCode::Char('4') | KeyCode::Char('5') |
                            KeyCode::Char('6') => {
                                if let Some(feed_id) = app.pending_problem_feed_id.take() {
                                    // Map char to index and submit immediately
                                    let idx = match k.code {
                                        KeyCode::Char('0') => 0,
                                        KeyCode::Char('1') => 1,
                                        KeyCode::Char('2') => 2,
                                        KeyCode::Char('3') => 3,
                                        KeyCode::Char('4') => 4,
                                        KeyCode::Char('5') => 5,
                                        KeyCode::Char('6') => 6,
                                        _ => 0,
                                    };
                                    app.reason_index = idx;
                                    app.reason_modal = false;
                                    let reason_code: u8 = reason_code_for_index(idx);
                                    app.status_msg = format!("Reporting feed {} as problematic (reason {})…", feed_id, reason_code);
                                    let ui_tx2 = ui_tx.clone();
                                    let tx2 = tx.clone();
                                    tokio::spawn(async move {
                                        match pi_report_problematic(feed_id, reason_code).await {
                                            Ok(desc) => {
                                                let _ = ui_tx2.send(UiMsg::Status(format!("Problematic reported: {}", desc)));
                                                let _ = poll_for_new_feeds(tx2).await;
                                            }
                                            Err(e) => {
                                                eprintln!("Problematic report failed: {}", e);
                                                let _ = ui_tx2.send(UiMsg::Status(format!("Report failed: {}", e)));
                                            }
                                        }
                                    });
                                } else {
                                    app.reason_modal = false;
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(feed_id) = app.pending_problem_feed_id.take() {
                                    app.reason_modal = false;
                                    let sel = app.reason_index;
                                    let reason_code: u8 = reason_code_for_index(sel);
                                    app.status_msg = format!("Reporting feed {} as problematic (reason {})…", feed_id, reason_code);
                                    let ui_tx2 = ui_tx.clone();
                                    let tx2 = tx.clone();
                                    tokio::spawn(async move {
                                        match pi_report_problematic(feed_id, reason_code).await {
                                            Ok(desc) => {
                                                let _ = ui_tx2.send(UiMsg::Status(format!("Problematic reported: {}", desc)));
                                                let _ = poll_for_new_feeds(tx2).await;
                                            }
                                            Err(e) => {
                                                eprintln!("Problematic report failed: {}", e);
                                                let _ = ui_tx2.send(UiMsg::Status(format!("Report failed: {}", e)));
                                            }
                                        }
                                    });
                                } else {
                                    app.reason_modal = false;
                                }
                            }
                            _ => {}
                        }
                        // Skip other handlers while modal is active
                        continue;
                    }
                    match k.code {
                        KeyCode::Char('q') => break,
                        // Vim mode ? for help
                        KeyCode::Char('?') if app.vim_mode => {
                            app.help_modal = !app.help_modal;
                        }
                        // Volume down with -
                        KeyCode::Char('-') => {
                            app.volume = (app.volume - 0.1).max(0.0);
                            if let Some(sink) = &app.audio_sink {
                                sink.set_volume(app.volume);
                            }
                            app.status_msg = format!("Volume: {}%", (app.volume * 100.0) as u8);
                        }
                        // Volume up with =
                        KeyCode::Char('=') => {
                            app.volume = (app.volume + 0.1).min(2.0);
                            if let Some(sink) = &app.audio_sink {
                                sink.set_volume(app.volume);
                            }
                            app.status_msg = format!("Volume: {}%", (app.volume * 100.0) as u8);
                        }
                        // Vim mode space for play/pause toggle
                        KeyCode::Char(' ') if app.vim_mode => {
                            if let Some(feed) = app.feeds.get(app.selected) {
                                let selected_feed_id = feed.id;
                                let selected_feed_title = feed.title.clone();
                                let selected_feed_url = feed.url.clone();
                                // Check if we're playing the currently selected feed
                                let is_same_feed = app.playing_feed_id == selected_feed_id;
                                
                                if is_same_feed && app.audio_sink.is_some() {
                                    // Same feed - toggle pause/resume
                                    if let Some(sink) = &app.audio_sink {
                                        if sink.is_paused() {
                                            sink.play();
                                            app.status_msg = "Playback resumed".into();
                                        } else {
                                            sink.pause();
                                            app.status_msg = "Playback paused".into();
                                        }
                                    }
                                } else {
                                    // Different feed or no audio loaded - start playing the selected feed
                                    if let Some(feed_url) = selected_feed_url {
                                        // Stop current playback if any
                                        if app.audio_sink.is_some() {
                                            app.stop_playback();
                                        }
                                        app.playing_feed_id = selected_feed_id;
                                        app.playing_feed_title = selected_feed_title;
                                        app.status_msg = "Fetching feed…".into();
                                        let ui_tx2 = ui_tx.clone();
                                        tokio::spawn(async move {
                                            let _ = fetch_play_latest(feed_url, ui_tx2).await;
                                        });
                                    } else {
                                        app.status_msg = "Selected feed has no URL".into();
                                    }
                                }
                            }
                        }
                        // Vim mode j for down
                        KeyCode::Char('j') if app.vim_mode => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let max_scroll = {
                                        let lines = app.xml_text.lines().count();
                                        lines.saturating_sub(visible_rows)
                                    };
                                    let next = (app.xml_scroll as usize + 1).min(max_scroll);
                                    app.xml_scroll = next as u16;
                                } else {
                                    let feed_opt = app.feeds.get(app.selected);
                                    let max_scroll = if let Some(feed) = feed_opt {
                                        let desc = feed.description.clone().unwrap_or_default();
                                        let lines = desc.lines().count() + 5;
                                        lines.saturating_sub(visible_rows)
                                    } else { 0 };
                                    let next = (app.popup_scroll as usize + 1).min(max_scroll);
                                    app.popup_scroll = next as u16;
                                }
                                continue;
                            }
                            if app.selected + 1 < app.feeds.len() {
                                app.selected += 1;
                                let bottom = app.scroll + viewport_items;
                                if app.selected >= bottom {
                                    app.scroll = app.selected.saturating_sub(viewport_items - 1);
                                }
                            }
                        }
                        // Vim mode k for up
                        KeyCode::Char('k') if app.vim_mode => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let max_scroll = {
                                        let lines = app.xml_text.lines().count();
                                        lines.saturating_sub(visible_rows)
                                    };
                                    if app.xml_scroll > 0 {
                                        app.xml_scroll -= 1;
                                    }
                                    if app.xml_scroll > max_scroll as u16 {
                                        app.xml_scroll = max_scroll as u16;
                                    }
                                } else {
                                    let feed_opt = app.feeds.get(app.selected);
                                    let max_scroll = if let Some(feed) = feed_opt {
                                        let desc = feed.description.clone().unwrap_or_default();
                                        let lines = desc.lines().count() + 5;
                                        lines.saturating_sub(visible_rows)
                                    } else { 0 };
                                    if app.popup_scroll > 0 {
                                        app.popup_scroll -= 1;
                                    }
                                    if app.popup_scroll > max_scroll as u16 {
                                        app.popup_scroll = max_scroll as u16;
                                    }
                                }
                                continue;
                            }
                            if app.selected > 0 {
                                app.selected -= 1;
                                if app.selected < app.scroll {
                                    app.scroll = app.selected;
                                }
                            }
                        }
                        // Vim mode h for left (acts like Home)
                        KeyCode::Char('h') if app.vim_mode => {
                            if app.xml_show || app.show_popup {
                                if app.xml_show { app.xml_scroll = 0; } else { app.popup_scroll = 0; }
                                continue;
                            }
                            app.selected = 0;
                            app.scroll = 0;
                        }
                        // Vim mode l for right (acts like End)
                        KeyCode::Char('l') if app.vim_mode => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let max_scroll = {
                                        let lines = app.xml_text.lines().count();
                                        lines.saturating_sub(visible_rows)
                                    };
                                    app.xml_scroll = max_scroll.max(0) as u16;
                                } else {
                                    let feed_opt = app.feeds.get(app.selected);
                                    let max_scroll = if let Some(feed) = feed_opt {
                                        let desc = feed.description.clone().unwrap_or_default();
                                        let lines = desc.lines().count() + 5;
                                        lines.saturating_sub(visible_rows)
                                    } else { 0 };
                                    app.popup_scroll = max_scroll.max(0) as u16;
                                }
                                continue;
                            }
                            if !app.feeds.is_empty() {
                                app.selected = app.feeds.len() - 1;
                                let max_scroll = app
                                    .feeds
                                    .len()
                                    .saturating_sub(viewport_items);
                                app.scroll = max_scroll;
                            }
                        }
                        KeyCode::Esc => {
                            if app.help_modal {
                                app.help_modal = false;
                            } else if app.xml_show {
                                app.xml_show = false;
                                app.xml_scroll = 0;
                            } else if app.show_popup {
                                app.show_popup = false;
                                app.popup_scroll = 0;
                            } else if app.reason_modal {
                                app.reason_modal = false;
                                app.pending_problem_feed_id = None;
                            } else if app.is_playing() || app.audio_sink.is_some() {
                                app.stop_playback();
                            }
                        }
                        KeyCode::Char('r') => {
                            // Trigger a manual refresh in the background
                            app.status_msg = String::from("Refreshing…");
                            let tx2 = tx.clone();
                            tokio::spawn(async move {
                                let _ = poll_for_new_feeds(tx2).await;
                            });
                        }
                        // Ctrl-n for next (vim mode)
                        KeyCode::Char('n') if app.vim_mode && k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let max_scroll = {
                                        let lines = app.xml_text.lines().count();
                                        lines.saturating_sub(visible_rows)
                                    };
                                    let next = (app.xml_scroll as usize + 1).min(max_scroll);
                                    app.xml_scroll = next as u16;
                                } else {
                                    let feed_opt = app.feeds.get(app.selected);
                                    let max_scroll = if let Some(feed) = feed_opt {
                                        let desc = feed.description.clone().unwrap_or_default();
                                        let lines = desc.lines().count() + 5;
                                        lines.saturating_sub(visible_rows)
                                    } else { 0 };
                                    let next = (app.popup_scroll as usize + 1).min(max_scroll);
                                    app.popup_scroll = next as u16;
                                }
                                continue;
                            }
                            if app.selected + 1 < app.feeds.len() {
                                app.selected += 1;
                                let bottom = app.scroll + viewport_items;
                                if app.selected >= bottom {
                                    app.scroll = app.selected.saturating_sub(viewport_items - 1);
                                }
                            }
                        }
                        // Ctrl-p for previous (vim mode)
                        KeyCode::Char('p') if app.vim_mode && k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let max_scroll = {
                                        let lines = app.xml_text.lines().count();
                                        lines.saturating_sub(visible_rows)
                                    };
                                    if app.xml_scroll > 0 {
                                        app.xml_scroll -= 1;
                                    }
                                    if app.xml_scroll > max_scroll as u16 {
                                        app.xml_scroll = max_scroll as u16;
                                    }
                                } else {
                                    let feed_opt = app.feeds.get(app.selected);
                                    let max_scroll = if let Some(feed) = feed_opt {
                                        let desc = feed.description.clone().unwrap_or_default();
                                        let lines = desc.lines().count() + 5;
                                        lines.saturating_sub(visible_rows)
                                    } else { 0 };
                                    if app.popup_scroll > 0 {
                                        app.popup_scroll -= 1;
                                    }
                                    if app.popup_scroll > max_scroll as u16 {
                                        app.popup_scroll = max_scroll as u16;
                                    }
                                }
                                continue;
                            }
                            if app.selected > 0 {
                                app.selected -= 1;
                                if app.selected < app.scroll {
                                    app.scroll = app.selected;
                                }
                            }
                        }
                        KeyCode::Char('p') => {
                            if let Some(feed) = app.feeds.get(app.selected) {
                                if let Some(feed_url) = feed.url.clone() {
                                    app.status_msg = "Fetching feed…".into();
                                    let ui_tx2 = ui_tx.clone();
                                    tokio::spawn(async move {
                                        if let Err(_e) = fetch_play_latest(feed_url, ui_tx2).await {
                                            // best-effort status on error
                                        }
                                    });
                                } else {
                                    app.status_msg = "Selected feed has no URL".into();
                                }
                            }
                        }
                        KeyCode::Char('x') => {
                            if let Some(feed) = app.feeds.get(app.selected) {
                                if let Some(feed_url) = feed.url.clone() {
                                    app.status_msg = "Downloading feed XML…".into();
                                    let ui_tx2 = ui_tx.clone();
                                    tokio::spawn(async move {
                                        if let Err(_e) = fetch_feed_xml(feed_url, ui_tx2).await {
                                            // Send error to UI channel if possible
                                        }
                                    });
                                } else {
                                    app.status_msg = "Selected feed has no URL".into();
                                }
                            }
                        }
                        KeyCode::Char('d') => {
                            // Mark selected feed as problematic via Podcast Index API
                            if !PI_READ_WRITE.load(Ordering::Relaxed) {
                                app.status_msg = "Read/write disabled: missing API credentials".into();
                            } else if let Some(feed) = app.feeds.get(app.selected) {
                                if let Some(feed_id) = feed.id {
                                    // Open reason selection modal
                                    app.pending_problem_feed_id = Some(feed_id);
                                    app.reason_index = 0; // default to "No Reason" (0)
                                    app.reason_modal = true;
                                } else {
                                    app.status_msg = "Selected feed has no id".into();
                                }
                            }
                        }
                        KeyCode::Enter => {
                            if !app.feeds.is_empty() {
                                app.show_popup = true;
                                app.popup_scroll = 0;
                            }
                        }
                        KeyCode::Up => {
                            // If a modal is open, scroll it instead of moving list selection
                            if app.xml_show || app.show_popup {
                                // Determine which modal and its area/visible rows
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let max_scroll = {
                                        let total_lines = app.xml_text.lines().count() + 2; // blank + hint
                                        total_lines.saturating_sub(visible_rows) as u16
                                    };
                                    if app.xml_scroll > 0 {
                                        app.xml_scroll -= 1;
                                    }
                                    if app.xml_scroll > max_scroll {
                                        app.xml_scroll = max_scroll;
                                    }
                                } else {
                                    let feed_opt = app.feeds.get(app.selected);
                                    let total_lines: usize = if let Some(feed) = feed_opt {
                                        let title = feed.title.clone().unwrap_or_else(|| "<untitled>".into());
                                        let desc = feed.description.clone().unwrap_or_else(|| "<no description>".into());
                                        // title + blank + desc + blank + hint
                                        let mut n: usize = 4; // two blanks + title + hint
                                        n += 1; // desc line (approx; wrapping not counted)
                                        let _ = title; // silence unused
                                        let _ = desc; // silence unused
                                        n
                                    } else { 0 };
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as u16;
                                    if app.popup_scroll > 0 {
                                        app.popup_scroll -= 1;
                                    }
                                    if app.popup_scroll > max_scroll {
                                        app.popup_scroll = max_scroll;
                                    }
                                }
                                continue;
                            }
                            if app.selected > 0 {
                                app.selected -= 1;
                                // Ensure selected remains visible
                                if app.selected < app.scroll {
                                    app.scroll = app.selected;
                                }
                            }
                        }
                        KeyCode::Down => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let total_lines = app.xml_text.lines().count() + 2;
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as i32;
                                    let next = (app.xml_scroll as i32 + 1).min(max_scroll.max(0));
                                    app.xml_scroll = next as u16;
                                } else {
                                    // approximate line count
                                    let total_lines: usize = 5; // title + blank + desc(approx1) + blank + hint
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as i32;
                                    let next = (app.popup_scroll as i32 + 1).min(max_scroll.max(0));
                                    app.popup_scroll = next as u16;
                                }
                                continue;
                            }
                            if app.selected + 1 < app.feeds.len() {
                                app.selected += 1;
                                let bottom = app.scroll + viewport_items;
                                if app.selected >= bottom {
                                    app.scroll = app.selected + 1 - viewport_items;
                                }
                            }
                        }
                        KeyCode::PageUp => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                let step = visible_rows.max(1) as i32;
                                if app.xml_show {
                                    let next = (app.xml_scroll as i32 - step).max(0);
                                    app.xml_scroll = next as u16;
                                } else {
                                    let next = (app.popup_scroll as i32 - step).max(0);
                                    app.popup_scroll = next as u16;
                                }
                                continue;
                            }
                            let step = viewport_items.max(1);
                            if app.selected >= step {
                                app.selected -= step;
                            } else {
                                app.selected = 0;
                            }
                            if app.selected < app.scroll {
                                app.scroll = app.selected;
                            }
                        }
                        KeyCode::PageDown => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                let step = visible_rows.max(1) as i32;
                                if app.xml_show {
                                    let total_lines = app.xml_text.lines().count() + 2;
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as i32;
                                    let next = (app.xml_scroll as i32 + step).min(max_scroll.max(0));
                                    app.xml_scroll = next as u16;
                                } else {
                                    let total_lines: usize = 5; // rough
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as i32;
                                    let next = (app.popup_scroll as i32 + step).min(max_scroll.max(0));
                                    app.popup_scroll = next as u16;
                                }
                                continue;
                            }
                            let step = viewport_items.max(1);
                            if !app.feeds.is_empty() {
                                let max_idx = app.feeds.len() - 1;
                                app.selected = (app.selected + step).min(max_idx);
                                let bottom = app.scroll + viewport_items;
                                if app.selected >= bottom {
                                    app.scroll = app.selected + 1 - viewport_items;
                                }
                            }
                        }
                        KeyCode::Home => {
                            if app.xml_show || app.show_popup {
                                if app.xml_show { app.xml_scroll = 0; } else { app.popup_scroll = 0; }
                                continue;
                            }
                            app.selected = 0;
                            app.scroll = 0;
                        }
                        KeyCode::End => {
                            if app.xml_show || app.show_popup {
                                let modal_area = if app.xml_show {
                                    centered_rect(80, 70, term_rect)
                                } else {
                                    centered_rect(70, 60, term_rect)
                                };
                                let visible_rows: usize = modal_area.height.saturating_sub(2) as usize;
                                if app.xml_show {
                                    let total_lines = app.xml_text.lines().count() + 2;
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as i32;
                                    app.xml_scroll = max_scroll.max(0) as u16;
                                } else {
                                    let total_lines: usize = 5;
                                    let max_scroll = total_lines.saturating_sub(visible_rows) as i32;
                                    app.popup_scroll = max_scroll.max(0) as u16;
                                }
                                continue;
                            }
                            if !app.feeds.is_empty() {
                                app.selected = app.feeds.len() - 1;
                                let max_scroll = app
                                    .feeds
                                    .len()
                                    .saturating_sub(viewport_items);
                                app.scroll = max_scroll;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        ui_tick.tick().await;
    }

    // Restore terminal
    disable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(LeaveAlternateScreen)?;
    
    // Exit the process to terminate the background polling task
    std::process::exit(0);
}

// The main polling function which fetches new feeds from the Podcast Index API.
async fn poll_for_new_feeds(tx: mpsc::UnboundedSender<AppUpdate>) -> Result<()> {
    let (feeds, source) = match pi_get_recent_newfeeds().await {
        Ok(feeds) => (feeds, DataSource::Api),
        Err(e) => return Err(e),
    };

    let mut st = AppUpdate::new();
    st.feeds = feeds;
    st.last_updated = Some(Local::now());
    st.source = source;
    st.status_msg = String::from("OK");
    tx.send(st).map_err(|_| anyhow::anyhow!("failed to send state"))?;
    Ok(())
}

// The call to Podcast Index API /recent/newfeeds endpoint.
// Per AGENTS.md requirement: include `since` query param = (now - 24h) as unix timestamp (seconds)
// and `max` set to 500.
async fn pi_get_recent_newfeeds() -> Result<Vec<Feed>> {
    let client = reqwest::Client::new();

    // Compute the unix timestamp for current time minus 86,400 seconds (24 hours).
    let since: i64 = (chrono::Utc::now() - chrono::Duration::seconds(86_400)).timestamp();

    let resp = client
        .get("https://api.podcastindex.org/api/1.0/recent/newfeeds")
        .query(&[("since", since), ("max", 500)])
        .header("User-Agent", "pimonitor/0.1")
        .send()
        .await?
        .error_for_status()?;

    let api: ApiResponse = resp.json().await?;
    // Filter out feeds marked as dead per AGENTS.md requirement.
    let feeds: Vec<Feed> = api
        .feeds
        .into_iter()
        .filter(|f| {
            match &f.dead {
                None => true,
                Some(v) => match v {
                    Value::Bool(b) => !*b,
                    Value::Number(n) => n.as_i64().map(|i| i == 0).unwrap_or(true),
                    Value::String(s) => {
                        let s = s.to_lowercase();
                        !(s == "1" || s == "true")
                    }
                    _ => true,
                },
            }
        })
        .collect();

    // Keep a stable ordering (as provided) and return
    Ok(feeds)
}

// Load API credentials from pimonitor.yaml on demand.
// Returns Some((key, secret)) if both are present and non-empty, otherwise None.
fn load_pi_creds_from(path: &Path) -> Option<(String, String)> {
    if let Ok(file) = File::open(path) {
        if let Ok(cfg) = serde_yaml::from_reader::<_, AppConfig>(file) {
            let key_ok = cfg
                .pi_api_key
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let sec_ok = cfg
                .pi_api_secret
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if key_ok && sec_ok {
                return Some((
                    cfg.pi_api_key.unwrap(),
                    cfg.pi_api_secret.unwrap(),
                ));
            }
        }
    }
    None
}

fn load_pi_creds() -> Option<(String, String)> {
    let path = if let Ok(cp) = CONFIG_PATH.lock() {
        cp.clone()
    } else {
        PathBuf::from("pimonitor.yaml")
    };
    load_pi_creds_from(&path)
}

// Build PodcastIndex authentication headers.
// Returns (x_auth_key, x_auth_date, authorization)
fn build_pi_auth_headers(key: &str, secret: &str, now_unix: i64) -> (String, String, String) {
    let date_str = now_unix.to_string();
    // Per Podcast Index docs and tests, the Authorization header is
    // sha1( key + secret + X-Auth-Date )
    let payload = format!("{}{}{}", key, secret, date_str);
    let mut hasher = Sha1::new();
    hasher.update(payload.as_bytes());
    let digest = hasher.finalize();
    let auth = format!("{:x}", digest);
    (key.to_string(), date_str, auth)
}

// Report a feed as problematic to Podcast Index.
// Uses POST https://api.podcastindex.org/api/1.0/report/problematic?id=<feed_id>&reason=<0..5>
async fn pi_report_problematic(feed_id: u64, reason: u8) -> Result<String> {
    let (key, secret) = load_pi_creds()
        .ok_or_else(|| anyhow::anyhow!("API credentials missing in pimonitor.yaml"))?;

    let now = chrono::Utc::now().timestamp();
    let (x_key, x_date, auth) = build_pi_auth_headers(&key, &secret, now);
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.podcastindex.org/api/1.0/report/problematic")
        .query(&[("id", feed_id.to_string()), ("reason", reason.to_string())])
        .header("User-Agent", "pimonitor/0.1")
        .header("X-Auth-Key", x_key)
        .header("X-Auth-Date", x_date)
        .header("Authorization", auth)
        .send()
        .await?
        .error_for_status()?;

    // This endpoint returns { status, description, ... }
    let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
    let desc = v
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("Reported");
    Ok(desc.to_string())
}

// Messages sent to UI loop for async tasks like playback
enum UiMsg {
    Status(String),
    PlayReady(PathBuf),
    PlayError(String),
    EqUpdate([f32; 12]),
    EqEnd,
    XmlReady(String),
    XmlError(String),
}

async fn fetch_feed_xml(feed_url: String, tx: mpsc::UnboundedSender<UiMsg>) -> Result<()> {
    let client = reqwest::Client::new();
    tx.send(UiMsg::Status("Downloading RSS…".into())).ok();
    match client.get(&feed_url).send().await {
        Ok(resp) => match resp.error_for_status() {
            Ok(ok) => match ok.text().await {
                Ok(text) => {
                    // Pretty-print the downloaded XML for readability in the UI.
                    // If pretty-printing fails for any reason, fall back to the raw text.
                    let pretty = pretty_print_xml(&text).unwrap_or(text);
                    tx.send(UiMsg::XmlReady(pretty)).ok();
                }
                Err(e) => {
                    tx.send(UiMsg::XmlError(format!("failed to read body: {}", e))).ok();
                }
            },
            Err(e) => {
                tx.send(UiMsg::XmlError(format!("HTTP error: {}", e))).ok();
            }
        },
        Err(e) => {
            tx.send(UiMsg::XmlError(format!("request failed: {}", e))).ok();
        }
    }
    Ok(())
}

// Pretty-print XML using xml-rs event reader and writer with indentation.
// This aims to be tolerant: in case of malformed XML, the caller should fall back to the original text.
fn pretty_print_xml(input: &str) -> Result<String> {
    let parser = XmlReader::from_str(input);

    // Configure writer to indent with two spaces and avoid auto-inserting declarations.
    let mut out: Vec<u8> = Vec::new();
    let config = XmlEmitterConfig::new()
        .perform_indent(true)
        .indent_string("  ")
        .write_document_declaration(false);
    {
        let mut writer = XmlWriter::new_with_config(&mut out, config);

        for ev in parser {
            match ev {
                Ok(XmlEvent::StartElement { name, attributes, .. }) => {
                    // Clone to owned strings so lifetimes outlive the builder until write.
                    let elem_name = name.local_name.clone();
                    let attrs_owned: Vec<(String, String)> = attributes
                        .into_iter()
                        .map(|a| (a.name.local_name, a.value))
                        .collect();

                    // Build a start element with attributes
                    let mut elem = WriterXmlEvent::start_element(elem_name.as_str());
                    for (k, v) in &attrs_owned {
                        elem = elem.attr(k.as_str(), v.as_str());
                    }
                    writer.write(elem)?;
                }
                Ok(XmlEvent::EndElement { .. }) => {
                    writer.write(WriterXmlEvent::end_element())?;
                }
                Ok(XmlEvent::Characters(text)) => {
                    writer.write(WriterXmlEvent::characters(&text))?;
                }
                Ok(XmlEvent::CData(text)) => {
                    writer.write(WriterXmlEvent::cdata(&text))?;
                }
                Ok(XmlEvent::Comment(text)) => {
                    writer.write(WriterXmlEvent::comment(&text))?;
                }
                Ok(XmlEvent::ProcessingInstruction { name, data }) => {
                    writer.write(WriterXmlEvent::processing_instruction(&name, data.as_deref()))?;
                }
                // Ignore other events such as StartDocument/EndDocument/Whitespace to avoid duplicating declarations
                Ok(_) => {}
                Err(e) => {
                    return Err(anyhow::anyhow!("XML parse error: {}", e));
                }
            }
        }
    }

    let s = String::from_utf8(out)?;
    Ok(s)
}

// Lightweight update passed from background fetch to UI thread
#[derive(Debug, Clone)]
struct AppUpdate {
    feeds: Vec<Feed>,
    last_updated: Option<DateTime<Local>>, 
    source: DataSource,
    status_msg: String,
}

impl AppUpdate {
    fn new() -> Self {
        Self {
            feeds: Vec::new(),
            last_updated: None,
            source: DataSource::Empty,
            status_msg: String::new(),
        }
    }
}

async fn fetch_play_latest(feed_url: String, tx: mpsc::UnboundedSender<UiMsg>) -> Result<()> {
    let client = reqwest::Client::new();
    tx.send(UiMsg::Status("Downloading RSS…".into())).ok();
    let xml = client.get(&feed_url).send().await?.error_for_status()?.text().await?;
    match extract_latest_enclosure_url(&xml) {
        Some(enclosure_url) => {
            tx.send(UiMsg::Status("Downloading audio…".into())).ok();
            match download_to_temp(&enclosure_url).await {
                Ok(path) => {
                    tx.send(UiMsg::PlayReady(path)).ok();
                }
                Err(e) => {
                    tx.send(UiMsg::PlayError(format!("download failed: {}", e))).ok();
                }
            }
        }
        None => {
            tx.send(UiMsg::PlayError("No enclosure url found".into())).ok();
        }
    }
    Ok(())
}

fn extract_latest_enclosure_url(xml: &str) -> Option<String> {
    let parser = XmlReader::from_str(xml);
    let mut in_item = false;
    let mut saw_any_item = false;
    // Track whether we saw an <enclosure> at all in the first <item>
    let mut saw_enclosure_tag = false;

    for event in parser {
        match event {
            Ok(XmlEvent::StartElement { name, attributes, .. }) => {
                if name.local_name == "item" {
                    in_item = true;
                    saw_any_item = true;
                } else if in_item && name.local_name == "enclosure" {
                    saw_enclosure_tag = true;
                    // Look for url attribute
                    for attr in attributes {
                        if attr.name.local_name == "url" {
                            return Some(attr.value);
                        }
                    }
                    // If we reached here, enclosure had no 'url' attribute
                    eprintln!(
                        "extract_latest_enclosure_url: first <item> contains <enclosure> without a 'url' attribute"
                    );
                }
            }
            Ok(XmlEvent::EndElement { name }) => {
                if name.local_name == "item" {
                    // Finished first item without returning. Provide details.
                    if !saw_enclosure_tag {
                        eprintln!(
                            "extract_latest_enclosure_url: first <item> has no <enclosure> tag"
                        );
                    } else {
                        eprintln!(
                            "extract_latest_enclosure_url: first <item>'s <enclosure> lacked a usable 'url' attribute"
                        );
                    }
                    return None;
                }
            }
            Ok(XmlEvent::EndDocument) => {
                if !saw_any_item {
                    eprintln!(
                        "extract_latest_enclosure_url: no <item> element found in RSS/Atom feed"
                    );
                } else if in_item {
                    eprintln!(
                        "extract_latest_enclosure_url: reached EOF while parsing first <item> without finding an <enclosure url=…>"
                    );
                } else {
                    eprintln!(
                        "extract_latest_enclosure_url: reached EOF before locating <enclosure url=…> in first <item>"
                    );
                }
                break;
            }
            Err(e) => {
                eprintln!(
                    "extract_latest_enclosure_url: XML parse error while reading feed: {}",
                    e
                );
                break;
            }
            _ => {}
        }
    }
    None
}

async fn download_to_temp(url: &str) -> Result<PathBuf> {
    let client = reqwest::Client::new();
    let mut resp = client.get(url).send().await?.error_for_status()?;
    let mut tmp = tempfile::NamedTempFile::new()?;
    while let Some(chunk) = resp.chunk().await? {
        use std::io::Write;
        tmp.write_all(&chunk)?;
    }
    let (_file, path) = tmp.keep()?; // persist so path remains valid
    Ok(path)
}

fn start_playback_from_file(path: &PathBuf) -> Result<(OutputStream, Sink)> {
    use std::panic;
    
    // Keep OutputStream alive together with Sink
    let (stream, handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&handle)?;

    // Work around rodio/symphonia init panic on some platforms due to unexpected seek errors
    // by providing a fully in-memory, seekable source (Cursor<Vec<u8>>).
    let mut file = File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let cursor = Cursor::new(buf);

    // Catch panics from rodio/symphonia decoder initialization
    // Set a temporary panic hook that does nothing to suppress panic messages
    let old_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    
    let decoder_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        rodio::Decoder::new(cursor)
    }));
    
    // Restore the original panic hook
    panic::set_hook(old_hook);
    
    let decoder = match decoder_result {
        Ok(Ok(dec)) => dec,
        Ok(Err(e)) => return Err(anyhow::anyhow!("Decoder error: {}", e)),
        Err(_) => return Err(anyhow::anyhow!("Audio decoder panic - this audio format may not be supported")),
    };
    
    sink.append(decoder);
    sink.play();
    Ok((stream, sink))
}

fn get_duration_from_file(path: &Path) -> Option<Duration> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();
    let probed = get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let format = probed.format;
    
    // Select the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.sample_rate.is_some())?;
        
    let params = &track.codec_params;
    
    if let (Some(n_frames), Some(tb)) = (params.n_frames, params.time_base) {
        let time = tb.calc_time(n_frames);
        return Some(Duration::from_secs(time.seconds) + Duration::from_secs_f64(time.frac));
    }
    
    None
}

// Spawn an analyzer that computes 5-band EQ magnitudes from the audio file and sends periodic updates.
async fn spawn_eq_analyzer(path: PathBuf, tx: mpsc::UnboundedSender<UiMsg>) -> Result<()> {
    // Run in blocking thread since decoding is CPU-bound
    tokio::task::spawn_blocking(move || {
        if let Err(e) = analyze_file_eq(path, tx.clone()) {
            let _ = tx.send(UiMsg::Status(format!("EQ analyzer error: {}", e)));
            let _ = tx.send(UiMsg::EqEnd);
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("join error: {}", e))?;
    Ok(())
}

fn analyze_file_eq(path: PathBuf, tx: mpsc::UnboundedSender<UiMsg>) -> Result<()> {
    // Open media source
    let file = std::fs::File::open(&path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();
    let probed = get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
    .map_err(|e| anyhow::anyhow!("probe error: {}", e))?;
    let mut format = probed.format;

    // Select the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.sample_rate.is_some())
        .ok_or_else(|| anyhow::anyhow!("no audio track"))?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| anyhow::anyhow!("decoder error: {}", e))?;

    let sample_rate = track.codec_params.sample_rate.unwrap_or(44100) as f32;

    // FFT setup
    let nfft: usize = 2048;
    let hop: usize = 1024; // 50% overlap
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(nfft);
    let window: Vec<f32> = (0..nfft)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * (i as f32) / (nfft as f32)).cos())
        .collect();

    let mut frame: Vec<f32> = Vec::with_capacity(nfft * 2);
    let mut last_send = std::time::Instant::now();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::ResetRequired) => {
                // Handle stream reset
                decoder.reset();
                continue;
            }
            Err(SymphoniaError::IoError(_)) => {
                // IO errors might be temporary, try to continue
                continue;
            }
            Err(SymphoniaError::DecodeError(_)) => {
                // Decode errors might be recoverable
                continue;
            }
            Err(_) => {
                // Other errors are likely EOF or fatal, exit the loop
                break;
            }
        };

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(_) => {
                // For other decode errors, try to continue
                continue;
            }
        };

        // Convert to f32 mono samples
        match decoded {
            AudioBufferRef::F32(buf) => {
                let chs = buf.spec().channels.count();
                for f in 0..buf.frames() {
                    let mut s = 0.0f32;
                    for ch in 0..chs {
                        s += buf.chan(ch)[f];
                    }
                    s /= chs as f32;
                    frame.push(s);
                }
            }
            AudioBufferRef::U8(buf) => {
                let chs = buf.spec().channels.count();
                for f in 0..buf.frames() {
                    let mut s = 0.0f32;
                    for ch in 0..chs {
                        s += (buf.chan(ch)[f] as f32 - 128.0) / 128.0;
                    }
                    s /= chs as f32;
                    frame.push(s);
                }
            }
            AudioBufferRef::S16(buf) => {
                let chs = buf.spec().channels.count();
                for f in 0..buf.frames() {
                    let mut s = 0.0f32;
                    for ch in 0..chs {
                        s += buf.chan(ch)[f] as f32 / 32768.0;
                    }
                    s /= chs as f32;
                    frame.push(s);
                }
            }
            AudioBufferRef::S24(buf) => {
                let chs = buf.spec().channels.count();
                for f in 0..buf.frames() {
                    let mut s = 0.0f32;
                    for ch in 0..chs {
                        let v = buf.chan(ch)[f].inner() as f32 / 8_388_608.0; // 2^23
                        s += v;
                    }
                    s /= chs as f32;
                    frame.push(s);
                }
            }
            AudioBufferRef::S32(buf) => {
                let chs = buf.spec().channels.count();
                for f in 0..buf.frames() {
                    let mut s = 0.0f32;
                    for ch in 0..chs {
                        s += buf.chan(ch)[f] as f32 / 2_147_483_648.0; // 2^31
                    }
                    s /= chs as f32;
                    frame.push(s);
                }
            }
            AudioBufferRef::F64(buf) => {
                let chs = buf.spec().channels.count();
                for f in 0..buf.frames() {
                    let mut s = 0.0f32;
                    for ch in 0..chs {
                        s += buf.chan(ch)[f] as f32;
                    }
                    s /= chs as f32;
                    frame.push(s);
                }
            }
            _ => {}
        }

        while frame.len() >= nfft {
            // Build FFT input with windowing over the first nfft samples
            let mut input: Vec<Complex32> = (0..nfft)
                .map(|i| Complex32::new(frame[i] * window[i], 0.0))
                .collect();

            // Execute FFT
            fft.process(&mut input);

            // Magnitude spectrum for positive frequencies
            let half = nfft / 2;
            let mags: Vec<f32> = input.into_iter().take(half).map(|c| c.norm()).collect();

            // Frequency band edges (Hz) for 12 bands, log-spaced from ~20 Hz to Nyquist (capped at 20 kHz)
            let nyquist = sample_rate / 2.0;
            let high_cap = nyquist.min(20_000.0).max(200.0); // ensure sensible upper bound >= 200 Hz
            let low_cap = 20.0_f32.min(high_cap / 32.0); // if Nyquist is tiny, push low down proportionally
            let mut edges: [f32; 13] = [0.0; 13];
            // Create 13 edges for 12 bands, geometrically spaced
            let ratio = if high_cap > low_cap {
                (high_cap / low_cap).powf(1.0 / 12.0)
            } else {
                1.0
            };
            edges[0] = 0.0; // include DC in first band
            let mut f = low_cap;
            for i in 1..13 {
                edges[i] = f.min(high_cap);
                f *= ratio;
            }

            // Sum bins into 12 bands
            let bin_hz = sample_rate / nfft as f32;
            let mut bands = [0.0f32; 12];
            for (bi, mag) in mags.iter().enumerate() {
                let freq = bi as f32 * bin_hz;
                for b in 0..12 {
                    if freq >= edges[b] && freq < edges[b + 1] {
                        bands[b] += *mag;
                        break;
                    }
                }
            }

            // Log scale and normalize each band approximately to 0..1
            for b in 0..12 {
                let v = (bands[b] + 1e-6).log10();
                // Rough normalization: map approximately [-6..2] -> [0..1]
                let norm = ((v + 6.0) / 8.0).clamp(0.0, 1.0);
                bands[b] = norm;
            }

            // Throttle updates to ~8 Hz
            if last_send.elapsed() >= std::time::Duration::from_millis(120) {
                let _ = tx.send(UiMsg::EqUpdate(bands));
                last_send = std::time::Instant::now();
            }

            // Advance by hop size
            let drain = hop.min(frame.len());
            frame.drain(0..drain);
        }
    }

    let _ = tx.send(UiMsg::EqEnd);
    Ok(())
}
