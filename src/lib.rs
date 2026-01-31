use serde_yaml::Value as YamlValue;
use sha1::{Digest, Sha1};
use std::fs::File;
use std::path::Path;

// =========================
// Podcast Index auth helpers
// =========================
// Public helper to build PodcastIndex auth headers components.
// Returns (x_auth_key, x_auth_date, authorization)
pub fn build_pi_auth_headers(key: &str, secret: &str, now_unix: i64) -> (String, String, String) {
    let date_str = now_unix.to_string();
    // Compute Authorization as sha1(key + secret + X-Auth-Date)
    let payload = format!("{}{}{}", key, secret, date_str);
    let mut hasher = Sha1::new();
    hasher.update(payload.as_bytes());
    let digest = hasher.finalize();
    let auth = format!("{:x}", digest);
    (key.to_string(), date_str, auth)
}

// (test-only debug helpers were removed)

// Public helper to load creds from a YAML file path without relying on internal types.
// Returns Some((key, secret)) if present and non-empty, else None.
pub fn load_pi_creds_from(path: &Path) -> Option<(String, String)> {
    let file = File::open(path).ok()?;
    let yaml: YamlValue = serde_yaml::from_reader(file).ok()?;
    let key = yaml
        .get("pi_api_key")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let secret = yaml
        .get("pi_api_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match (key, secret) {
        (Some(k), Some(s)) => Some((k, s)),
        _ => None,
    }
}

// =========================
// Problematic reason helpers
// =========================
// Returns the list of problematic reasons (label, code) per AGENTS.md.
// Index positions correspond to UI selection order.
pub fn reason_options() -> Vec<(&'static str, u8)> {
    vec![
        ("No Reason", 0),
        ("Spam", 1),
        ("AI Slop", 2),
        ("Illegal Content", 3),
        ("Duplicate", 4),
        ("Malicious Payload", 5),
        ("Feed Hijack", 6),
    ]
}

// Given a UI index, return the corresponding reason code, clamped to the valid range.
pub fn reason_code_for_index(idx: usize) -> u8 {
    let opts = reason_options();
    let i = idx.min(opts.len().saturating_sub(1));
    opts[i].1
}

// =========================
// Feed search helpers
// =========================
// Find the index of the first feed matching the query.
// - If `query` is a positive integer, it is treated as a feed id and the first
//   entry whose id equals that value is returned.
// - Otherwise, the query is split into whitespace-separated words; all words
//   must be found (case-insensitive) as substrings within the title for a match.
// - Empty or whitespace-only query returns None (caller may choose to "do nothing").
// The function operates over a simplified slice of (id, title) so it can be used
// from both the binary and tests without depending on the internal Feed struct.
pub fn find_feed_index_by_query<T: AsRef<str>>(feeds: &[(u64, T)], query: &str) -> Option<usize> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    // Numeric id search (must be > 0)
    if let Ok(id_val) = q.parse::<u64>() {
        if id_val > 0 {
            return feeds.iter().position(|(id, _)| *id == id_val);
        }
    }
    // Word-conjunction search on title
    let words: Vec<String> = q.split_whitespace().map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return None;
    }
    feeds.iter().position(|(_, title)| {
        let t = title.as_ref().to_lowercase();
        words.iter().all(|w| t.contains(w))
    })
}
