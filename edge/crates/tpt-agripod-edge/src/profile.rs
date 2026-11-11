// SPDX-License-Identifier: MIT OR Apache-2.0

//! Crop Profile validation and runtime apply on the edge (spec section 4.1; docs/mqtt-topics.md).
//!
//! The edge is the authority: it validates every profile itself, whatever the orchestrator
//! checked. A profile that fails is rejected with a list of problems, and the previous profile
//! stays active. The rules mirror `profiles/crop-profile.schema.json`, with the same
//! `additionalProperties: false` strictness, and the pump limit comes from the shipped pump
//! contract, not from a copied constant.

use std::collections::HashSet;

use tpt_agripod_solver::{Decision, Rational, check_command, parse_contract};

use crate::json::{self, Value};

/// Largest profile accepted, bytes. Refused before parsing.
pub const MAX_PROFILE_BYTES: usize = 64 * 1024;

/// Pump contract shipped with the edge. The pump duty limit is read from it.
pub const PUMP_CONTRACT: &str = include_str!("../../../contracts/pump.contract");

const TOP_KEYS: [&str; 10] = [
    "schema_version",
    "kind",
    "profile_id",
    "profile_version",
    "name",
    "description",
    "cartridge_types",
    "targets",
    "drip_schedule",
    "controls",
];
const TARGET_KEYS: [&str; 7] = [
    "vpd_kpa",
    "dli_mol_m2_day",
    "photoperiod_h",
    "root_zone_temp_c",
    "npk_ratio",
    "ec_s_m",
    "ph",
];
const CONTROL_KEYS: [&str; 5] = ["value", "unit", "min", "max", "contract_ref"];
/// Control names the edge can map to a controller and a contract.
const KNOWN_CONTROLS: [&str; 2] = ["pump_duty", "led_dimming"];

/// One thing wrong with a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// JSON pointer-like path, e.g. `/controls/pump_duty/max`.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

/// A profile that passed validation, reduced to what the edge applies.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveProfile {
    /// Stable slug.
    pub profile_id: String,
    /// MAJOR.MINOR.PATCH.
    pub profile_version: String,
    /// Pump duty setpoint, fraction. Applied to irrigation.
    pub pump_duty_frac: f64,
    /// LED dimming setpoint, fraction. Stored; no LED loop uses it yet.
    pub led_dimming_frac: Option<f64>,
}

/// Outcome of an apply attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyOutcome {
    /// The profile is now active.
    Applied(ActiveProfile),
    /// The profile was refused. The previous profile, if any, is still active.
    Rejected(Vec<Problem>),
}

/// Holds the active profile and replaces it only with one that validates.
#[derive(Debug, Default)]
pub struct ProfileStore {
    active: Option<ActiveProfile>,
}

impl ProfileStore {
    /// An empty store: no profile is active until one applies.
    pub fn new() -> Self {
        Self::default()
    }

    /// The active profile, if any.
    pub fn active(&self) -> Option<&ActiveProfile> {
        self.active.as_ref()
    }

    /// Validates `text` and, only if it passes, makes it the active profile.
    pub fn apply(&mut self, text: &str) -> ApplyOutcome {
        match validate(text) {
            Ok(p) => {
                self.active = Some(p.clone());
                ApplyOutcome::Applied(p)
            }
            Err(problems) => ApplyOutcome::Rejected(problems),
        }
    }
}

/// Validates a profile document. Returns the applied view, or every problem found.
pub fn validate(text: &str) -> Result<ActiveProfile, Vec<Problem>> {
    if text.len() > MAX_PROFILE_BYTES {
        return Err(vec![problem("/", "profile is larger than 64 KiB")]);
    }
    let root = json::parse(text).map_err(|e| {
        vec![problem(
            "/",
            format!("not valid JSON at byte {}: {}", e.offset, e.message),
        )]
    })?;
    let mut v = Checker::default();
    v.check_root(&root);
    if !v.problems.is_empty() {
        return Err(v.problems);
    }
    Ok(ActiveProfile {
        profile_id: root
            .get("profile_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        profile_version: root
            .get("profile_version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        pump_duty_frac: v.pump_duty.unwrap_or(0.0),
        led_dimming_frac: v.led_dimming,
    })
}

fn problem(path: &str, message: impl Into<String>) -> Problem {
    Problem {
        path: path.to_string(),
        message: message.into(),
    }
}

#[derive(Default)]
struct Checker {
    problems: Vec<Problem>,
    pump_duty: Option<f64>,
    led_dimming: Option<f64>,
}

impl Checker {
    fn fail(&mut self, path: &str, message: impl Into<String>) {
        self.problems.push(problem(path, message));
    }

    fn check_root(&mut self, root: &Value) {
        let Some(members) = root.as_object() else {
            self.fail("/", "profile must be a JSON object");
            return;
        };
        self.unknown_keys("", members, &TOP_KEYS);
        for key in TOP_KEYS {
            if root.get(key).is_none() && !matches!(key, "description" | "drip_schedule") {
                self.fail(&format!("/{key}"), "required field is missing");
            }
        }

        match root.get("schema_version").and_then(Value::as_str) {
            Some("0") => {}
            _ => self.fail("/schema_version", "must be the string \"0\""),
        }
        match root.get("kind").and_then(Value::as_str) {
            Some("plant") => {}
            Some("culture") => self.fail(
                "/kind",
                "\"culture\" is reserved and not supported in schema v0",
            ),
            _ => self.fail("/kind", "must be \"plant\""),
        }
        match root.get("profile_id").and_then(Value::as_str) {
            Some(id) if valid_id(id) => {}
            _ => self.fail("/profile_id", "must be a 2 to 63 character lower-case slug"),
        }
        match root.get("profile_version").and_then(Value::as_str) {
            Some(v) if valid_version(v) => {}
            _ => self.fail("/profile_version", "must be MAJOR.MINOR.PATCH"),
        }
        match root.get("name").and_then(Value::as_str) {
            Some(n) if (1..=120).contains(&n.chars().count()) => {}
            _ => self.fail("/name", "must be 1 to 120 characters"),
        }
        if let Some(d) = root.get("description") {
            match d.as_str() {
                Some(s) if s.chars().count() <= 1000 => {}
                _ => self.fail(
                    "/description",
                    "must be a string of at most 1000 characters",
                ),
            }
        }
        self.check_cartridges(root.get("cartridge_types"));

        if let Some(t) = root.get("targets") {
            self.check_targets(t);
        }
        if let Some(d) = root.get("drip_schedule") {
            self.check_drip(d);
        }
        if let Some(c) = root.get("controls") {
            self.check_controls(c);
        }
    }

    fn unknown_keys(&mut self, base: &str, members: &[(String, Value)], allowed: &[&str]) {
        for (key, _) in members {
            if !allowed.contains(&key.as_str()) {
                self.fail(&format!("{base}/{key}"), "unknown field");
            }
        }
    }

    fn check_cartridges(&mut self, v: Option<&Value>) {
        let Some(items) = v.and_then(Value::as_array) else {
            self.fail("/cartridge_types", "must be an array");
            return;
        };
        if items.is_empty() {
            self.fail("/cartridge_types", "needs at least one entry");
        }
        let mut seen = HashSet::new();
        for (i, item) in items.iter().enumerate() {
            match item.as_str() {
                Some(c @ ("A" | "B" | "C")) => {
                    if !seen.insert(c) {
                        self.fail(&format!("/cartridge_types/{i}"), "repeated");
                    }
                }
                _ => self.fail(&format!("/cartridge_types/{i}"), "must be A, B or C"),
            }
        }
    }

    fn check_targets(&mut self, t: &Value) {
        let Some(members) = t.as_object() else {
            self.fail("/targets", "must be an object");
            return;
        };
        self.unknown_keys("/targets", members, &TARGET_KEYS);
        for key in [
            "vpd_kpa",
            "dli_mol_m2_day",
            "photoperiod_h",
            "root_zone_temp_c",
        ] {
            if t.get(key).is_none() {
                self.fail(&format!("/targets/{key}"), "required field is missing");
            }
        }
        self.check_band("/targets/vpd_kpa", t.get("vpd_kpa"), 0.0, 5.0);
        self.check_band(
            "/targets/root_zone_temp_c",
            t.get("root_zone_temp_c"),
            0.0,
            60.0,
        );
        self.check_band("/targets/ec_s_m", t.get("ec_s_m"), 0.0, 10.0);
        self.check_band("/targets/ph", t.get("ph"), 0.0, 14.0);
        self.check_scalar(
            "/targets/dli_mol_m2_day",
            t.get("dli_mol_m2_day"),
            0.0,
            80.0,
        );
        self.check_scalar("/targets/photoperiod_h", t.get("photoperiod_h"), 0.0, 24.0);
        if let Some(npk) = t.get("npk_ratio") {
            for k in ["n", "p", "k"] {
                self.check_scalar(
                    &format!("/targets/npk_ratio/{k}"),
                    npk.get(k),
                    0.0,
                    f64::MAX,
                );
            }
        }
    }

    fn check_band(&mut self, path: &str, v: Option<&Value>, lo: f64, hi: f64) {
        let Some(v) = v else { return };
        let (min, max) = (
            v.get("min").and_then(Value::as_f64),
            v.get("max").and_then(Value::as_f64),
        );
        match (min, max) {
            (Some(min), Some(max)) if lo <= min && min <= max && max <= hi => {}
            _ => self.fail(
                path,
                format!("needs numeric min and max with {lo} <= min <= max <= {hi}"),
            ),
        }
    }

    fn check_scalar(&mut self, path: &str, v: Option<&Value>, lo: f64, hi: f64) {
        match v.and_then(Value::as_f64) {
            Some(x) if (lo..=hi).contains(&x) => {}
            _ => self.fail(path, format!("must be a number from {lo} to {hi}")),
        }
    }

    fn check_drip(&mut self, d: &Value) {
        let Some(members) = d.as_object() else {
            self.fail("/drip_schedule", "must be an object");
            return;
        };
        self.unknown_keys("/drip_schedule", members, &["interval_min", "volume_l"]);
        let interval = d.get("interval_min").and_then(Value::as_f64);
        let volume = d.get("volume_l").and_then(Value::as_f64);
        if !matches!(interval, Some(x) if x > 0.0) {
            self.fail("/drip_schedule/interval_min", "must be a positive number");
        }
        if !matches!(volume, Some(x) if x >= 0.0) {
            self.fail("/drip_schedule/volume_l", "must be zero or positive");
        }
    }

    fn check_controls(&mut self, c: &Value) {
        let Some(members) = c.as_object() else {
            self.fail("/controls", "must be an object");
            return;
        };
        if members.is_empty() {
            self.fail("/controls", "needs at least one entry");
        }
        for (name, ctrl) in members {
            let base = format!("/controls/{name}");
            if !KNOWN_CONTROLS.contains(&name.as_str()) {
                self.fail(&base, "unknown to the edge");
                continue;
            }
            let Some(cm) = ctrl.as_object() else {
                self.fail(&base, "must be an object");
                continue;
            };
            self.unknown_keys(&base, cm, &CONTROL_KEYS);
            let value = ctrl.get("value").and_then(Value::as_f64);
            let min = ctrl.get("min").and_then(Value::as_f64);
            let max = ctrl.get("max").and_then(Value::as_f64);
            if ctrl
                .get("unit")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                self.fail(&format!("{base}/unit"), "needs a unit");
            }
            let Some(value) = value else {
                self.fail(&format!("{base}/value"), "needs a number");
                continue;
            };
            if min.is_some_and(|m| value < m) {
                self.fail(&format!("{base}/value"), "is below min");
            }
            if max.is_some_and(|m| value > m) {
                self.fail(&format!("{base}/value"), "is above max");
            }
            if min.is_some_and(|m| max.is_some_and(|x| m > x)) {
                self.fail(&base, "min is above max");
            }
            match name.as_str() {
                "pump_duty" => {
                    if !(0.0..=1.0).contains(&value) {
                        self.fail(&format!("{base}/value"), "must be a fraction from 0 to 1");
                    }
                    if max.is_some_and(|m| m > pump_duty_limit()) {
                        self.fail(
                            &format!("{base}/max"),
                            "exceeds the pump contract limit (edge/contracts/pump.contract)",
                        );
                    }
                    self.pump_duty = Some(value);
                }
                "led_dimming" => {
                    if !(0.0..=1.0).contains(&value) {
                        self.fail(&format!("{base}/value"), "must be a fraction from 0 to 1");
                    }
                    self.led_dimming = Some(value);
                }
                _ => {}
            }
        }
    }
}

fn valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    (2..=63).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn valid_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
}

/// The pump duty ceiling, read from the pump contract. The contract is checked by the solver:
/// the largest duty it accepts is the largest a profile may request.
///
/// Returns the limit as a float. If the contract cannot be read or has no usable limit, the
/// ceiling is zero, so every pump profile is refused rather than trusting a guess.
pub fn pump_duty_limit() -> f64 {
    static LIMIT: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(compute_pump_duty_limit)
}

fn compute_pump_duty_limit() -> f64 {
    let Ok(contract) = parse_contract(PUMP_CONTRACT) else {
        return 0.0;
    };
    // Binary-search the largest duty the ensures clause accepts, using exact rationals. The
    // search space is [0, 1] at six decimal places: enough to recover a limit written with
    // up to six decimals.
    let accepts = |micro: i64| {
        let mut cmd = tpt_agripod_solver::Bindings::new();
        let Ok(v) = Rational::new(i128::from(micro), 1_000_000) else {
            return false;
        };
        cmd.set("pump_duty_cycle_frac", v);
        let state = {
            let mut s = tpt_agripod_solver::Bindings::new();
            s.set("reservoir_level_l", Rational::from_int(100));
            s.set("grid_power_headroom_w", Rational::from_int(5000));
            s
        };
        matches!(check_command(&contract, &state, &cmd), Decision::Allow)
    };
    if !accepts(0) {
        return 0.0;
    }
    let (mut lo, mut hi) = (0i64, 1_000_000i64);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if accepts(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../../profiles/examples/example-plant-v0.json");

    fn mutate(old: &str, new: &str) -> Result<ActiveProfile, Vec<Problem>> {
        assert!(EXAMPLE.contains(old), "setup: {old:?} not in example");
        validate(&EXAMPLE.replacen(old, new, 1))
    }

    fn fails_at(old: &str, new: &str, path: &str) {
        let problems = mutate(old, new).expect_err("accepted a bad profile");
        assert!(
            problems.iter().any(|p| p.path.starts_with(path)),
            "no problem at {path}: {problems:?}"
        );
    }

    #[test]
    fn shipped_example_is_accepted_and_applied() {
        let p = validate(EXAMPLE).expect("example must validate");
        assert_eq!(p.profile_id, "example-plant");
        assert_eq!(p.profile_version, "0.1.0");
        assert!((p.pump_duty_frac - 0.10).abs() < 1e-12);
        assert_eq!(p.led_dimming_frac, Some(0.9));
    }

    #[test]
    fn pump_limit_is_read_from_the_contract() {
        // The contract says pump_duty_cycle_frac <= 0.15.
        assert!(
            (pump_duty_limit() - 0.15).abs() < 1e-9,
            "limit {}",
            pump_duty_limit()
        );
    }

    #[test]
    fn a_pump_max_above_the_contract_is_refused() {
        fails_at(
            r#""max": 0.15,"#,
            r#""max": 0.20,"#,
            "/controls/pump_duty/max",
        );
    }

    #[test]
    fn a_pump_value_above_its_max_is_refused() {
        fails_at(
            r#""value": 0.10,"#,
            r#""value": 0.30,"#,
            "/controls/pump_duty/value",
        );
    }

    #[test]
    fn schema_strictness() {
        fails_at(r#""kind": "plant""#, r#""kind": "culture""#, "/kind");
        fails_at(r#""kind": "plant""#, r#""kind": "animal""#, "/kind");
        fails_at(
            r#""profile_id": "example-plant""#,
            r#""profile_id": "Example""#,
            "/profile_id",
        );
        fails_at(
            r#""profile_version": "0.1.0""#,
            r#""profile_version": "0.1""#,
            "/profile_version",
        );
        fails_at(
            r#""cartridge_types": ["C"]"#,
            r#""cartridge_types": ["D"]"#,
            "/cartridge_types/0",
        );
        fails_at(
            r#""schema_version": "0""#,
            r#""schema_version": "1""#,
            "/schema_version",
        );
        fails_at(
            r#""kind": "plant","#,
            r#""kind": "plant", "extra": 1,"#,
            "/extra",
        );
        fails_at(
            r#""interval_min": 15,"#,
            r#""interval_min": 15, "x": 2,"#,
            "/drip_schedule/x",
        );
    }

    #[test]
    fn unknown_control_names_are_refused() {
        fails_at(
            r#""pump_duty": {"#,
            r#""pump_speed": {"#,
            "/controls/pump_speed",
        );
    }

    #[test]
    fn target_ranges_are_checked() {
        fails_at(
            r#""vpd_kpa": { "min": 0.8, "max": 1.0 }"#,
            r#""vpd_kpa": { "min": 1.2, "max": 1.0 }"#,
            "/targets/vpd_kpa",
        );
        fails_at(
            r#""dli_mol_m2_day": 14"#,
            r#""dli_mol_m2_day": 120"#,
            "/targets/dli_mol_m2_day",
        );
        fails_at(
            r#""photoperiod_h": 16"#,
            r#""photoperiod_h": 30"#,
            "/targets/photoperiod_h",
        );
    }

    #[test]
    fn problems_are_collected_not_just_the_first() {
        let text = EXAMPLE
            .replacen(r#""schema_version": "0""#, r#""schema_version": "9""#, 1)
            .replacen(r#""kind": "plant""#, r#""kind": "x""#, 1);
        let problems = validate(&text).expect_err("two faults must both be refused");
        assert!(problems.len() >= 2, "{problems:?}");
    }

    #[test]
    fn oversize_and_malformed_documents_are_refused() {
        assert!(validate(&" ".repeat(MAX_PROFILE_BYTES + 1)).is_err());
        assert!(validate("{").is_err());
        assert!(validate("[]").is_err());
    }

    #[test]
    fn a_rejected_profile_leaves_the_previous_one_active() {
        let mut store = ProfileStore::new();
        assert!(matches!(store.apply(EXAMPLE), ApplyOutcome::Applied(_)));
        let bad = EXAMPLE.replacen(r#""max": 0.15,"#, r#""max": 0.9,"#, 1);
        assert!(matches!(store.apply(&bad), ApplyOutcome::Rejected(_)));
        assert_eq!(
            store.active().map(|p| p.profile_id.as_str()),
            Some("example-plant")
        );
    }

    #[test]
    fn a_new_valid_profile_replaces_the_old_one() {
        let mut store = ProfileStore::new();
        store.apply(EXAMPLE);
        let newer = EXAMPLE
            .replacen(r#""value": 0.10,"#, r#""value": 0.05,"#, 1)
            .replacen(
                r#""profile_version": "0.1.0""#,
                r#""profile_version": "0.2.0""#,
                1,
            );
        assert!(matches!(store.apply(&newer), ApplyOutcome::Applied(_)));
        let active = store.active().unwrap();
        assert_eq!(active.profile_version, "0.2.0");
        assert!((active.pump_duty_frac - 0.05).abs() < 1e-12);
    }

    #[test]
    fn id_and_version_rules_match_the_go_validator() {
        assert!(valid_id("lettuce-v1"));
        assert!(!valid_id("Lettuce"));
        assert!(!valid_id("a"));
        assert!(valid_version("1.0.0"));
        assert!(!valid_version("1.0"));
        assert!(!valid_version("1.0.x"));
    }
}
