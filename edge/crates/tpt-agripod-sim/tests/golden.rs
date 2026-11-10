// SPDX-License-Identifier: MIT OR Apache-2.0

//! Golden-file test: a fixed seed and default parameters must reproduce the committed report
//! byte for byte. Any change to the model, the parameters or the number formatting shows up
//! here. To accept an intentional change, run with `AGRIPOD_BLESS=1` and review the diff.

use std::path::PathBuf;

use tpt_agripod_sim::{SimParams, mock, parse_csv, report, simulate};

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

fn check_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("AGRIPOD_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden file {}: {e}", path.display()));
    assert_eq!(
        actual.replace("\r\n", "\n"),
        expected.replace("\r\n", "\n"),
        "golden mismatch for {name}"
    );
}

#[test]
fn one_day_seed_42_human_report() {
    let csv = mock::generate_csv(&mock::MockSpec {
        days: 1,
        ..mock::MockSpec::default()
    });
    let ingest = parse_csv(&csv).unwrap();
    let p = SimParams::default();
    let summary = simulate(&ingest.samples, &p).unwrap();
    check_golden(
        "day1-seed42.human.txt",
        &report::human(&summary, &p, &ingest.rejected),
    );
}
