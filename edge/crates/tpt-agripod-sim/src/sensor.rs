// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sensor ingest: CSV parsing, range checks, timestamp ordering and rejection bookkeeping.
//!
//! A bad row is rejected and reported, never silently repaired. Missing time between
//! accepted rows is detected later by the simulation (see `SimParams::max_gap_s`).

use crate::time::parse_rfc3339_utc;

/// Columns every input file must have, in any order.
pub const REQUIRED_COLUMNS: [&str; 6] = [
    "ts",
    "temp_c",
    "rh_frac",
    "co2_ppm",
    "reservoir_temp_c",
    "power_w",
];

/// Optional column for outdoor air temperature. Absent means the model default is used.
pub const AMBIENT_COLUMN: &str = "ambient_c";

/// Physically plausible bounds for each numeric column. Values outside are rejected.
/// Order matters: `parse_csv` fills the sample fields in this order.
const RANGES: [(&str, f64, f64); 5] = [
    ("temp_c", -10.0, 50.0),
    ("rh_frac", 0.0, 1.0),
    ("co2_ppm", 0.0, 5000.0),
    ("reservoir_temp_c", 0.0, 40.0),
    ("power_w", 0.0, 50_000.0),
];
/// Bounds for the optional ambient column.
const AMBIENT_RANGE: (f64, f64) = (-30.0, 50.0);

/// One validated set of readings at a single timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Unix time in seconds, UTC.
    pub ts_s: i64,
    /// Indoor air dry-bulb temperature, °C.
    pub temp_c: f64,
    /// Indoor relative humidity, fraction 0 to 1.
    pub rh_frac: f64,
    /// Indoor CO2, ppm. Validated and kept; not used by the Phase 1 models yet.
    pub co2_ppm: f64,
    /// Nutrient reservoir temperature, °C. Validated and kept; used from Phase 3.
    pub reservoir_temp_c: f64,
    /// Non-HVAC electrical load (lighting, pumps, controls), W. All of it becomes heat in the room.
    pub power_w: f64,
    /// Outdoor air temperature, °C, when the file supplies it.
    pub ambient_c: Option<f64>,
}

/// A row that failed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// 1-based line number in the input, counting the header as line 1.
    pub line: usize,
    /// Why the row was rejected.
    pub reason: String,
}

/// Result of parsing a whole input: accepted samples plus every rejected row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ingest {
    /// Accepted samples, strictly increasing in time.
    pub samples: Vec<Sample>,
    /// Rejected rows, in file order.
    pub rejected: Vec<Rejection>,
}

/// Parses CSV text. The header line is mandatory and must contain every required column.
///
/// Returns `Err` only for a structural problem (missing header or columns). Row-level
/// problems go to [`Ingest::rejected`].
pub fn parse_csv(text: &str) -> Result<Ingest, String> {
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| (i + 1, l));

    let (_, header_line) = lines.next().ok_or("input is empty: no header line")?;
    let header: Vec<&str> = header_line.split(',').map(str::trim).collect();
    let col = |name: &str| header.iter().position(|h| *h == name);

    let missing: Vec<&str> = REQUIRED_COLUMNS
        .iter()
        .copied()
        .filter(|c| col(c).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "header is missing required columns: {}",
            missing.join(", ")
        ));
    }
    let ts_col = col("ts").ok_or("internal: ts column")?;
    let ambient_col = col(AMBIENT_COLUMN);
    // Required by the check above, so these lookups cannot fail.
    let range_cols: Vec<(usize, &str, f64, f64)> = RANGES
        .iter()
        .map(|&(name, lo, hi)| (col(name).unwrap_or(0), name, lo, hi))
        .collect();

    let mut out = Ingest::default();
    let mut last_ts: Option<i64> = None;

    for (line_no, line) in lines {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let mut reject = |reason: String| {
            out.rejected.push(Rejection {
                line: line_no,
                reason,
            })
        };
        if fields.len() != header.len() {
            reject(format!(
                "expected {} fields, found {}",
                header.len(),
                fields.len()
            ));
            continue;
        }

        let ts_s = match parse_rfc3339_utc(fields[ts_col]) {
            Ok(ts) => ts,
            Err(e) => {
                reject(e);
                continue;
            }
        };
        if last_ts.is_some_and(|prev| ts_s <= prev) {
            reject("timestamp is not after the previous accepted row".into());
            continue;
        }

        let mut values = [0.0_f64; 5];
        let mut problem = None;
        for (slot, &(i, name, lo, hi)) in range_cols.iter().enumerate() {
            match parse_bounded(fields[i], name, lo, hi) {
                Ok(v) => values[slot] = v,
                Err(e) => {
                    problem = Some(e);
                    break;
                }
            }
        }
        let ambient_c = match (problem.is_none(), ambient_col) {
            (true, Some(i)) => {
                match parse_bounded(fields[i], AMBIENT_COLUMN, AMBIENT_RANGE.0, AMBIENT_RANGE.1) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        problem = Some(e);
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(e) = problem {
            reject(e);
            continue;
        }

        out.samples.push(Sample {
            ts_s,
            temp_c: values[0],
            rh_frac: values[1],
            co2_ppm: values[2],
            reservoir_temp_c: values[3],
            power_w: values[4],
            ambient_c,
        });
        last_ts = Some(ts_s);
    }

    Ok(out)
}

fn parse_bounded(raw: &str, name: &str, lo: f64, hi: f64) -> Result<f64, String> {
    if raw.is_empty() {
        return Err(format!("missing value in column {name}"));
    }
    let v: f64 = raw
        .parse()
        .map_err(|_| format!("column {name}: '{raw}' is not a number"))?;
    if !v.is_finite() {
        return Err(format!("column {name}: value is not finite"));
    }
    if !(lo..=hi).contains(&v) {
        return Err(format!("column {name}: {v} is outside [{lo}, {hi}]"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "ts,temp_c,rh_frac,co2_ppm,reservoir_temp_c,power_w";

    #[test]
    fn accepts_a_clean_file() {
        let text = format!(
            "{HEADER}\n2026-01-01T00:00:00Z,20.0,0.6,800,18.0,150\n2026-01-01T00:05:00Z,20.1,0.61,801,18.1,151\n"
        );
        let ingest = parse_csv(&text).unwrap();
        assert_eq!(ingest.samples.len(), 2);
        assert!(ingest.rejected.is_empty());
        assert_eq!(ingest.samples[1].ts_s - ingest.samples[0].ts_s, 300);
        assert_eq!(ingest.samples[0].ambient_c, None);
    }

    #[test]
    fn reads_optional_ambient_column_in_any_order() {
        let text = "power_w,ts,temp_c,rh_frac,co2_ppm,reservoir_temp_c,ambient_c\n\
                    150,2026-01-01T00:00:00Z,20,0.6,800,18,12.5\n";
        let ingest = parse_csv(text).unwrap();
        assert_eq!(ingest.samples[0].ambient_c, Some(12.5));
        assert_eq!(ingest.samples[0].power_w, 150.0);
    }

    #[test]
    fn rejects_bad_rows_and_keeps_going() {
        let text = format!(
            "{HEADER}\n\
             2026-01-01T00:00:00Z,20.0,0.6,800,18.0,150\n\
             2026-01-01T00:05:00Z,NaN,0.6,800,18.0,150\n\
             2026-01-01T00:10:00Z,20.0,1.5,800,18.0,150\n\
             2026-01-01T00:15:00Z,20.0,0.6,800,18.0,\n\
             2026-01-01T00:00:00Z,20.0,0.6,800,18.0,150\n\
             2026-01-01T00:20:00Z,20.0,0.6,800,18.0\n\
             2026-01-01T00:25:00Z,20.0,0.6,800,18.0,150\n"
        );
        let ingest = parse_csv(&text).unwrap();
        assert_eq!(ingest.samples.len(), 2);
        // Header is line 1. Lines 3 to 7 are bad; line 8 (00:25) is a valid row.
        let lines: Vec<usize> = ingest.rejected.iter().map(|r| r.line).collect();
        assert_eq!(lines, vec![3, 4, 5, 6, 7]);
        assert!(ingest.rejected[0].reason.contains("not finite"));
        assert!(ingest.rejected[1].reason.contains("outside"));
        assert!(ingest.rejected[2].reason.contains("missing value"));
        assert!(ingest.rejected[3].reason.contains("not after"));
        assert!(ingest.rejected[4].reason.contains("expected 6 fields"));
    }

    #[test]
    fn rejects_missing_columns_and_empty_input() {
        assert!(parse_csv("").is_err());
        let err = parse_csv("ts,temp_c\n").unwrap_err();
        assert!(err.contains("rh_frac"));
    }

    #[test]
    fn accepts_a_byte_order_mark() {
        let text = format!("\u{feff}{HEADER}\n2026-01-01T00:00:00Z,20,0.6,800,18,150\n");
        assert_eq!(parse_csv(&text).unwrap().samples.len(), 1);
    }
}
