use std::fs;
use std::path::PathBuf;

// Use the library helpers
use pimonitor::{build_pi_auth_headers, load_pi_creds_from};

#[test]
fn test_build_pi_auth_headers_sha1() {
    // Deterministic example
    let key = "abc";
    let secret = "def";
    let ts: i64 = 1_700_000_000; // fixed timestamp
    let (x_key, x_date, auth) = build_pi_auth_headers(key, secret, ts);
    assert_eq!(x_key, "abc");
    assert_eq!(x_date, ts.to_string());
    // Expected hex digest of sha1("abc"+"def"+ts) with ts in decimal (no whitespace):
    // echo -n "abcdef1700000000" | sha1sum -> 4b441b27cc7e571834673d1e05b29806a6ad2c4a
    assert_eq!(auth, "4b441b27cc7e571834673d1e05b29806a6ad2c4a");
}

#[test]
fn test_load_pi_creds_from_yaml() {
    let mut p = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    p.push("pimonitor_creds_test.yaml");
    let yaml = "pi_api_key: \"KEY123\"\npi_api_secret: \"SEC456\"\n";
    fs::write(&p, yaml).unwrap();
    let creds = load_pi_creds_from(&p);
    assert!(creds.is_some());
    let (k, s) = creds.unwrap();
    assert_eq!(k, "KEY123");
    assert_eq!(s, "SEC456");
}
