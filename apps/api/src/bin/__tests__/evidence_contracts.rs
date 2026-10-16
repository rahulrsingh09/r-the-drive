use super::contract_support::freeze;
use flate2::{write::GzEncoder, Compression};
use serde_json::json;
use std::io::Write;

fn expected() -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(br#"{"sample":1}"#).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn frozen_match_does_not_require_optional_evidence() {
    freeze("evidence-match", json!({"sample": 1}), &expected());
}

#[test]
#[should_panic(expected = "frozen evidence-mismatch contract differs")]
fn frozen_mismatch_still_fails_without_optional_evidence() {
    freeze("evidence-mismatch", json!({"sample": 2}), &expected());
}
