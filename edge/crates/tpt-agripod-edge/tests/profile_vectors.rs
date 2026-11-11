// SPDX-License-Identifier: MIT OR Apache-2.0

//! The shared profile vectors (profiles/testvectors). The edge must accept every valid case and
//! reject every invalid case, with a rejection that names the problem. The orchestrator's Go test
//! reads the same manifest, so the two validators cannot drift apart without a failing test.

use std::path::PathBuf;

use tpt_agripod_edge::json::{self, Value};
use tpt_agripod_edge::profile;

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../profiles/testvectors")
}

#[test]
fn every_shared_vector_is_judged_the_same_way_by_the_edge() {
    let dir = vectors_dir();
    let manifest_text = std::fs::read_to_string(dir.join("manifest.json")).expect("manifest");
    let manifest = json::parse(&manifest_text).expect("manifest is JSON");
    let cases = manifest
        .get("cases")
        .and_then(Value::as_array)
        .expect("cases array");
    assert!(!cases.is_empty(), "manifest has no cases");

    for case in cases {
        let file = case.get("file").and_then(Value::as_str).expect("file");
        let want_valid = matches!(case.get("valid"), Some(Value::Bool(true)));
        let mentions = case.get("mentions").and_then(Value::as_str);
        let text =
            std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));

        match profile::validate(&text) {
            Ok(_) => assert!(want_valid, "{file}: invalid case was accepted"),
            Err(problems) => {
                assert!(!want_valid, "{file}: valid case was rejected: {problems:?}");
                if let Some(word) = mentions {
                    let joined: String = problems
                        .iter()
                        .map(|p| format!("{} {}", p.path, p.message))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    assert!(
                        joined.contains(word),
                        "{file}: no problem mentions {word:?}: {joined}"
                    );
                }
            }
        }
    }
}
