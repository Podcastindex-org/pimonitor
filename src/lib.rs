use sha1::{Digest, Sha1};
use serde_yaml::Value as YamlValue;
use std::fs::File;
use std::path::Path;

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
