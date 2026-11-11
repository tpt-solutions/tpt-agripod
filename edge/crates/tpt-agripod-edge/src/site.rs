// SPDX-License-Identifier: MIT OR Apache-2.0

//! Site configuration: the values that depend on the installation, not on the software.
//!
//! The edge has no defaults for these. A site file must state every value, and it must say
//! `"status": "production"`. A file marked `"example"` is refused, so the example in
//! `edge/config/site.example.json` cannot run by accident. The values come from the flood study,
//! the supply rating and the irrigation calibration (docs/phase2-edge-loop.md).

use crate::control::IrrigationPolicy;
use crate::json::{self, Value};
use crate::link::is_slug;
use crate::profile::pump_duty_limit;

/// Contract template for the solenoid valve. `{valve_open_max_s}` is rendered from the site file.
pub const VALVE_CONTRACT_TEMPLATE: &str = include_str!("../../../contracts/solenoid.contract");
/// Contract template for the LED driver. `{led_headroom_min_w}` is rendered from the site file.
pub const LED_CONTRACT_TEMPLATE: &str = include_str!("../../../contracts/led.contract");

/// One installation's configuration, validated.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteConfig {
    /// Site id.
    pub site_id: String,
    /// Node id.
    pub node_id: String,
    /// Broker, host:port.
    pub broker: String,
    /// Longest the valve may be open in one cycle, seconds. From the flood study.
    pub valve_open_max_s: f64,
    /// Smallest grid headroom the LED driver may run with, watts. From the supply rating.
    pub led_headroom_min_w: f64,
    /// Irrigation thresholds and timing, from calibration.
    pub irrigation: IrrigationPolicy,
    /// Reservoir drop per cycle, with the pump and valve off, that counts as a fail-open pump, litres.
    pub drain_limit_l: f64,
}

static NULL: Value = Value::Null;

const TOP: [&str; 8] = [
    "_comment",
    "status",
    "site_id",
    "node_id",
    "broker",
    "limits",
    "irrigation",
    "drain_limit_l",
];
const LIMITS: [&str; 2] = ["valve_open_max_s", "led_headroom_min_w"];
const IRRIGATION: [&str; 6] = [
    "moisture_low_frac",
    "moisture_high_frac",
    "pulse_s",
    "cooldown_s",
    "cycle_s",
    "pump_duty_frac",
];

impl SiteConfig {
    /// Parses and validates a site file. Every problem is reported, not just the first.
    pub fn parse(text: &str) -> Result<Self, Vec<String>> {
        let root = json::parse(text).map_err(|e| {
            vec![format!(
                "not valid JSON at byte {}: {}",
                e.offset, e.message
            )]
        })?;
        let mut errs = Vec::new();
        let Some(members) = root.as_object() else {
            return Err(vec!["site file must be a JSON object".into()]);
        };
        for (k, _) in members {
            if !TOP.contains(&k.as_str()) {
                errs.push(format!("unknown field {k:?}"));
            }
        }
        match root.get("status").and_then(Value::as_str) {
            Some("production") => {}
            Some("example") => errs.push(
                "this is the example site file; copy it, set the real values, and set status to \"production\""
                    .into(),
            ),
            _ => errs.push("status must be \"production\"".into()),
        }
        let site_id = slug(&root, "site_id", &mut errs);
        let node_id = slug(&root, "node_id", &mut errs);
        let broker = match root.get("broker").and_then(Value::as_str) {
            Some(b) if !b.is_empty() && b.contains(':') => b.to_string(),
            _ => {
                errs.push("broker must be host:port".into());
                String::new()
            }
        };
        let drain = number(&root, "drain_limit_l", &mut errs);
        if let Some(d) = drain {
            if d <= 0.0 {
                errs.push("drain_limit_l must be positive".into());
            }
        }

        let limits = root.get("limits");
        check_members(limits, "limits", &LIMITS, &mut errs);
        let valve_open_max_s = number(limits.unwrap_or(&NULL), "valve_open_max_s", &mut errs);
        let led_headroom_min_w = number(limits.unwrap_or(&NULL), "led_headroom_min_w", &mut errs);
        if let Some(v) = valve_open_max_s {
            if !(1.0..=3600.0).contains(&v) {
                errs.push("limits.valve_open_max_s must be 1 to 3600 seconds".into());
            }
        }
        if let Some(w) = led_headroom_min_w {
            if !(0.0..=50_000.0).contains(&w) {
                errs.push("limits.led_headroom_min_w must be 0 to 50000 watts".into());
            }
        }

        let irr = root.get("irrigation");
        check_members(irr, "irrigation", &IRRIGATION, &mut errs);
        let irr = irr.unwrap_or(&NULL);
        let low = number(irr, "moisture_low_frac", &mut errs);
        let high = number(irr, "moisture_high_frac", &mut errs);
        let pulse = number(irr, "pulse_s", &mut errs);
        let cooldown = number(irr, "cooldown_s", &mut errs);
        let cycle = number(irr, "cycle_s", &mut errs);
        let pump = number(irr, "pump_duty_frac", &mut errs);
        if let (Some(l), Some(h)) = (low, high) {
            if !(0.0 <= l && l < h && h <= 1.0) {
                errs.push(
                    "irrigation: need 0 <= moisture_low_frac < moisture_high_frac <= 1".into(),
                );
            }
        }
        if let Some(p) = pulse {
            if p <= 0.0 {
                errs.push("irrigation.pulse_s must be positive".into());
            }
        }
        if let Some(c) = cooldown {
            if c < 0.0 {
                errs.push("irrigation.cooldown_s must be zero or positive".into());
            }
        }
        if let Some(c) = cycle {
            if c <= 0.0 {
                errs.push("irrigation.cycle_s must be positive".into());
            }
            if let Some(v) = valve_open_max_s {
                if c > v {
                    errs.push("irrigation.cycle_s is longer than limits.valve_open_max_s".into());
                }
            }
        }
        if let Some(p) = pump {
            if !(p > 0.0 && p <= pump_duty_limit()) {
                errs.push(format!(
                    "irrigation.pump_duty_frac must be above 0 and at most {}",
                    pump_duty_limit()
                ));
            }
        }

        if !errs.is_empty() {
            return Err(errs);
        }
        Ok(Self {
            site_id: site_id.unwrap_or_default(),
            node_id: node_id.unwrap_or_default(),
            broker,
            valve_open_max_s: valve_open_max_s.unwrap_or_default(),
            led_headroom_min_w: led_headroom_min_w.unwrap_or_default(),
            irrigation: IrrigationPolicy {
                low_frac: low.unwrap_or_default(),
                high_frac: high.unwrap_or_default(),
                pulse_s: pulse.unwrap_or_default() as u64,
                cooldown_s: cooldown.unwrap_or_default() as u64,
                cycle_s: cycle.unwrap_or_default(),
                pump_duty_frac: pump.unwrap_or_default(),
            },
            drain_limit_l: drain.unwrap_or_default(),
        })
    }

    /// Reads and validates a site file from disk.
    pub fn load(path: &std::path::Path) -> Result<Self, Vec<String>> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| vec![format!("cannot read {}: {e}", path.display())])?;
        Self::parse(&text)
    }

    /// The solenoid contract with this site's open-time limit.
    pub fn valve_contract(&self) -> String {
        render(
            VALVE_CONTRACT_TEMPLATE,
            &[("valve_open_max_s", &self.valve_open_max_s.to_string())],
        )
    }

    /// The LED contract with this site's headroom threshold.
    pub fn led_contract(&self) -> String {
        render(
            LED_CONTRACT_TEMPLATE,
            &[("led_headroom_min_w", &self.led_headroom_min_w.to_string())],
        )
    }
}

/// Replaces each `{name}` in `template` with its value. Unknown placeholders are left as they
/// are, and the contract parser then refuses the result, so a missing value cannot slip through.
pub fn render(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn slug(root: &Value, key: &str, errs: &mut Vec<String>) -> Option<String> {
    match root.get(key).and_then(Value::as_str) {
        Some(s) if is_slug(s) => Some(s.to_string()),
        _ => {
            errs.push(format!("{key} must be a lower-case slug"));
            None
        }
    }
}

fn number(obj: &Value, key: &str, errs: &mut Vec<String>) -> Option<f64> {
    match obj.get(key).and_then(Value::as_f64) {
        Some(v) if v.is_finite() => Some(v),
        _ => {
            errs.push(format!("{key} must be a number"));
            None
        }
    }
}

fn check_members(obj: Option<&Value>, path: &str, allowed: &[&str], errs: &mut Vec<String>) {
    match obj {
        Some(v) => match v.as_object() {
            Some(members) => {
                for (k, _) in members {
                    if !allowed.contains(&k.as_str()) {
                        errs.push(format!("unknown field {path}.{k}"));
                    }
                }
            }
            None => errs.push(format!("{path} must be an object")),
        },
        None => errs.push(format!("{path} is required")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config/site.example.json");

    fn production() -> String {
        EXAMPLE.replacen(r#""status": "example""#, r#""status": "production""#, 1)
    }

    #[test]
    fn the_example_file_is_refused_so_it_cannot_run_by_accident() {
        let errs = SiteConfig::parse(EXAMPLE).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("example site file")),
            "{errs:?}"
        );
    }

    #[test]
    fn a_completed_file_parses_into_the_values_it_states() {
        let site = SiteConfig::parse(&production()).unwrap();
        assert_eq!(site.site_id, "wgtn-bay-01");
        assert_eq!(site.node_id, "nuc-01");
        assert_eq!(site.irrigation.cycle_s, 10.0);
        assert_eq!(site.irrigation.pulse_s, 60);
        assert!(site.valve_contract().contains("valve_open_s <= 900"));
        assert!(site.led_contract().contains("grid_power_headroom_w > 500"));
    }

    #[test]
    fn every_problem_is_reported_together() {
        let bad = production()
            .replacen(r#""valve_open_max_s": 900"#, r#""valve_open_max_s": 0"#, 1)
            .replacen(r#""pump_duty_frac": 0.12"#, r#""pump_duty_frac": 0.5"#, 1)
            .replacen(
                r#""moisture_low_frac": 0.35"#,
                r#""moisture_low_frac": 0.5"#,
                1,
            )
            .replacen(
                r#""broker": "127.0.0.1:1883""#,
                r#""broker": "localhost""#,
                1,
            );
        let errs = SiteConfig::parse(&bad).unwrap_err();
        assert!(errs.len() >= 4, "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("valve_open_max_s")));
        assert!(errs.iter().any(|e| e.contains("pump_duty_frac")));
        assert!(errs.iter().any(|e| e.contains("moisture_low_frac")));
        assert!(errs.iter().any(|e| e.contains("broker")));
    }

    #[test]
    fn a_cycle_longer_than_the_valve_limit_is_refused() {
        let bad = production().replacen(r#""cycle_s": 10"#, r#""cycle_s": 1000"#, 1);
        let errs = SiteConfig::parse(&bad).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("longer than limits.valve_open_max_s")),
            "{errs:?}"
        );
    }

    #[test]
    fn unknown_fields_are_refused() {
        let bad = production().replacen(
            r#""drain_limit_l": 2.0"#,
            r#""drain_limit_l": 2.0, "tariff": 1"#,
            1,
        );
        let errs = SiteConfig::parse(&bad).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("tariff")), "{errs:?}");
    }

    #[test]
    fn a_missing_placeholder_is_left_visible_to_the_contract_parser() {
        let left = render("ensures: valve_open_s <= {valve_open_max_s}", &[]);
        assert!(tpt_agripod_solver::parse_contract(&left).is_err());
    }
}
