// SPDX-License-Identifier: MIT OR Apache-2.0

//! Deterministic mock sensor data.
//!
//! The same seed always produces the same bytes. The generator uses only integer arithmetic
//! and basic IEEE float operations (+, −, ×, ÷, clamp), with no transcendental functions, so
//! the output does not depend on the platform's math library.

use std::fmt::Write as _;

use crate::time::{format_rfc3339_utc, local_hour};

/// What to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockSpec {
    /// First timestamp, Unix seconds UTC. Default is 2026-01-01T00:00:00Z.
    pub start_ts_s: i64,
    /// Length in whole days.
    pub days: u32,
    /// Interval between rows, seconds.
    pub step_s: i64,
    /// PRNG seed.
    pub seed: u64,
}

impl Default for MockSpec {
    fn default() -> Self {
        Self {
            start_ts_s: 1_767_225_600,
            days: 7,
            step_s: 300,
            seed: 42,
        }
    }
}

/// SplitMix64. Small, fast, and fully specified, so the sequence is stable.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [-1, 1).
    fn signed_unit(&mut self) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        2.0 * u - 1.0
    }
}

/// Triangle wave in [0, 1], peaking at 15:00 local and bottoming at 03:00. No trig needed.
fn diurnal_triangle(local_h: f64) -> f64 {
    let mut d = (local_h - 15.0).abs();
    if d > 12.0 {
        d = 24.0 - d;
    }
    (1.0 - d / 12.0).max(0.0)
}

/// Generates a CSV in the format `sensor::parse_csv` reads, including the optional `ambient_c` column.
///
/// The mock is a plausible scenario, not a calibrated one: lights 06:00 to 22:00 local at
/// 1800 W, a 150 W process load, indoor air held near 20 °C and 62 % RH, and outdoor air
/// swinging about 9 to 18 °C over the day.
pub fn generate_csv(spec: &MockSpec) -> String {
    let mut rng = SplitMix64(spec.seed);
    let mut out = String::from("ts,temp_c,rh_frac,co2_ppm,reservoir_temp_c,power_w,ambient_c\n");
    let steps = i64::from(spec.days) * 86_400 / spec.step_s;

    for i in 0..steps {
        let ts = spec.start_ts_s + i * spec.step_s;
        let local_h = local_hour(ts, 12.0);
        let lit = (6.0..22.0).contains(&local_h);

        let ambient = 9.0 + 9.0 * diurnal_triangle(local_h) + 0.3 * rng.signed_unit();
        let temp = 20.0 + 0.2 * rng.signed_unit();
        let rh = (0.62 + 0.03 * rng.signed_unit()).clamp(0.3, 0.9);
        let co2 = 800.0 + 40.0 * rng.signed_unit();
        let reservoir = 18.0 + 0.3 * rng.signed_unit();
        let lights_w = if lit { 1800.0 } else { 0.0 };
        let power = (lights_w + 150.0 + 20.0 * rng.signed_unit()).max(0.0);

        let _ = writeln!(
            out,
            "{},{temp:.2},{rh:.3},{co2:.1},{reservoir:.2},{power:.1},{ambient:.2}",
            format_rfc3339_utc(ts)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::parse_csv;

    #[test]
    fn same_seed_gives_identical_bytes() {
        let spec = MockSpec::default();
        assert_eq!(generate_csv(&spec), generate_csv(&spec));
    }

    #[test]
    fn different_seed_changes_the_noise() {
        let a = generate_csv(&MockSpec::default());
        let b = generate_csv(&MockSpec {
            seed: 43,
            ..MockSpec::default()
        });
        assert_ne!(a, b);
    }

    #[test]
    fn output_is_accepted_by_the_ingest_validator() {
        let text = generate_csv(&MockSpec {
            days: 2,
            ..MockSpec::default()
        });
        let ingest = parse_csv(&text).unwrap();
        assert_eq!(ingest.samples.len(), 2 * 288);
        assert!(ingest.rejected.is_empty(), "{:?}", ingest.rejected);
        assert!(ingest.samples.iter().all(|s| s.ambient_c.is_some()));
    }

    #[test]
    fn lights_and_outdoor_temperature_follow_the_day() {
        let text = generate_csv(&MockSpec::default());
        let ingest = parse_csv(&text).unwrap();
        // Looked up by UTC hour of the first day. Local is UTC+12.
        let at_utc = |h: i64| {
            ingest
                .samples
                .iter()
                .find(|s| s.ts_s.rem_euclid(86_400) == h * 3600)
                .copied()
                .unwrap()
        };
        let warm = at_utc(3).ambient_c.unwrap(); // 15:00 local, warmest
        let cold = at_utc(15).ambient_c.unwrap(); // 03:00 local, coldest
        assert!(warm > cold + 5.0, "warm {warm}, cold {cold}");
        let lit = at_utc(0); // 12:00 local, lights on
        let dark = at_utc(14); // 02:00 local, lights off
        assert!(lit.power_w > dark.power_w + 1000.0);
    }
}
