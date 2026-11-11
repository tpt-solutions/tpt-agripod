// SPDX-License-Identifier: MIT OR Apache-2.0

//! `agripod` command-line tool.
//!
//! ```text
//! agripod                                  print version and profile schema
//! agripod generate-mock [--days N] [--seed N] [--out FILE]
//! agripod simulate --input FILE [--format human|json] [model options]
//! ```

use std::process::ExitCode;

use tpt_agripod_sim::params::Tariff;
use tpt_agripod_sim::{SimParams, mock, parse_csv, report, simulate};

const USAGE: &str = "\
usage:
  agripod                                   print version and profile schema
  agripod generate-mock [--days N] [--seed N] [--out FILE]
  agripod simulate --input FILE [--format human|json] [options]

simulate options (defaults are assumptions, see docs/phase1-model.md):
  --plants N           potato plants (default 100)
  --lai X              leaf area index (default 3.0)
  --cop-cooling X      sensible cooling COP (default 3.0)
  --cop-heating X      sensible heating COP (default 3.0)
  --mer X              dehumidifier kg removed per kWh (default 1.2)
  --tariff-peak X      NZD per kWh, peak band (required)
  --tariff-offpeak X   NZD per kWh, off-peak band (required)
  --offpeak-start H    off-peak window start, local hour 0-24 (required)
  --offpeak-end H      off-peak window end, local hour 0-24; below start wraps midnight (required)
  --utc-offset H       local time offset from UTC in hours (default 12)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        println!(
            "agripod {} (profile schema {})",
            env!("CARGO_PKG_VERSION"),
            tpt_agripod_core::PROFILE_SCHEMA_VERSION
        );
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("agripod: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    match args[0].as_str() {
        "generate-mock" => generate_mock(&args[1..]),
        "simulate" => simulate_cmd(&args[1..]),
        "help" | "--help" | "-h" => Ok(format!("{USAGE}\n")),
        other => Err(format!("unknown command '{other}'\n\n{USAGE}")),
    }
}

fn generate_mock(args: &[String]) -> Result<String, String> {
    let mut spec = mock::MockSpec::default();
    let mut out_path: Option<String> = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--days" => spec.days = parse_value(flag, it.next())?,
            "--seed" => spec.seed = parse_value(flag, it.next())?,
            "--out" => out_path = Some(value(flag, it.next())?),
            other => return Err(format!("unknown option '{other}' for generate-mock")),
        }
    }
    if spec.days == 0 {
        return Err("--days must be at least 1".into());
    }
    let csv = mock::generate_csv(&spec);
    match out_path {
        Some(path) => {
            std::fs::write(&path, &csv).map_err(|e| format!("cannot write {path}: {e}"))?;
            Ok(format!(
                "wrote {} rows to {path}\n",
                csv.lines().count() - 1
            ))
        }
        None => Ok(csv),
    }
}

fn simulate_cmd(args: &[String]) -> Result<String, String> {
    let mut p = SimParams::default();
    let mut input: Option<String> = None;
    let (mut peak, mut offpeak, mut start, mut end): (
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
    ) = (None, None, None, None);
    let mut format = "human".to_string();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--input" => input = Some(value(flag, it.next())?),
            "--format" => format = value(flag, it.next())?,
            "--plants" => p.plants = parse_value(flag, it.next())?,
            "--lai" => p.lai = parse_value(flag, it.next())?,
            "--cop-cooling" => p.cop_cooling = parse_value(flag, it.next())?,
            "--cop-heating" => p.cop_heating = parse_value(flag, it.next())?,
            "--mer" => p.dehumidifier_mer_kg_kwh = parse_value(flag, it.next())?,
            "--tariff-peak" => peak = Some(parse_value(flag, it.next())?),
            "--tariff-offpeak" => offpeak = Some(parse_value(flag, it.next())?),
            "--offpeak-start" => start = Some(parse_value(flag, it.next())?),
            "--offpeak-end" => end = Some(parse_value(flag, it.next())?),
            "--utc-offset" => p.utc_offset_h = parse_value(flag, it.next())?,
            other => return Err(format!("unknown option '{other}' for simulate")),
        }
    }
    p.tariff = Some(Tariff {
        peak_nzd_kwh: peak
            .ok_or("simulate needs --tariff-peak (no default: the tariff is a site fact)")?,
        offpeak_nzd_kwh: offpeak.ok_or("simulate needs --tariff-offpeak")?,
        offpeak_start_h: start.ok_or("simulate needs --offpeak-start")?,
        offpeak_end_h: end.ok_or("simulate needs --offpeak-end")?,
    });
    let path = input.ok_or("simulate needs --input FILE")?;
    if format != "human" && format != "json" {
        return Err(format!(
            "--format must be 'human' or 'json', not '{format}'"
        ));
    }

    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let ingest = parse_csv(&text).map_err(|e| format!("{path}: {e}"))?;
    let summary = simulate(&ingest.samples, &p)?;

    Ok(match format.as_str() {
        "json" => format!("{}\n", report::json(&summary, &p, &ingest.rejected)),
        _ => report::human(&summary, &p, &ingest.rejected),
    })
}

fn value(flag: &str, next: Option<&String>) -> Result<String, String> {
    next.cloned().ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_value<T: std::str::FromStr>(flag: &str, next: Option<&String>) -> Result<T, String> {
    let raw = value(flag, next)?;
    raw.parse()
        .map_err(|_| format!("{flag}: '{raw}' is not a valid value"))
}
