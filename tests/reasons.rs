use pimonitor::{reason_options, reason_code_for_index, build_pi_auth_headers};
use sha1::{Sha1, Digest};

#[test]
fn reason_options_match_agents_md_order_and_codes() {
    let opts = reason_options();
    let labels: Vec<&str> = opts.iter().map(|(l, _)| *l).collect();
    let codes: Vec<u8> = opts.iter().map(|(_, c)| *c).collect();

    assert_eq!(labels, vec![
        "No Reason",
        "Spam",
        "AI Slop",
        "Illegal Content",
        "Duplicate",
        "Malicious Payload",
        "Feed Hijack",
    ]);

    assert_eq!(codes, vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn reason_code_for_index_clamps_and_maps() {
    // In-range indices map correctly
    for (i, code) in [0u8,1,2,3,4,5,6].iter().enumerate() {
        assert_eq!(reason_code_for_index(i), *code);
    }
    // Out of range clamps to last
    assert_eq!(reason_code_for_index(999), 6);
}

#[test]
fn build_auth_headers_are_stable_for_known_input() {
    let (key, date, auth) = build_pi_auth_headers("abc", "def", 1_700_000_000);
    assert_eq!(key, "abc");
    assert_eq!(date, "1700000000");
    // Compute expected sha1("abc" + "def" + "1700000000") here to avoid drift
    let mut hasher = Sha1::new();
    hasher.update(b"abcdef1700000000");
    let expected = format!("{:x}", hasher.finalize());
    assert_eq!(auth, expected);
}
