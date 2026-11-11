// SPDX-License-Identifier: MIT OR Apache-2.0

//! Human-readable and JSON reports. Numbers are fixed-precision so the output is stable
//! across runs and platforms, which the golden-file test relies on.

use std::fmt::Write as _;

use crate::params::SimParams;
use crate::sensor::Rejection;
use crate::simulate::Summary;
use crate::time::format_rfc3339_utc;

/// How many rejected rows to list by name. The rest are counted.
const REJECTION_PREVIEW: usize = 5;

/// Renders the summary as plain text for a terminal.
pub fn human(s: &Summary, p: &SimParams, rejected: &[Rejection]) -> String {
    let mut o = String::new();
    let days = s.simulated_s / 86_400.0;
    let ground = f64::from(p.plants) * p.ground_area_per_plant_m2;

    let _ = writeln!(o, "Agripod Phase 1 simulation");
    let _ = writeln!(
        o,
        "Input:   {} samples, {} rejected rows, {} gap intervals skipped",
        s.samples,
        rejected.len(),
        s.gap_intervals
    );
    let _ = writeln!(
        o,
        "Period:  {} to {} ({days:.2} days integrated)",
        format_rfc3339_utc(s.first_ts_s),
        format_rfc3339_utc(s.last_ts_s)
    );
    let _ = writeln!(o);
    let _ = writeln!(o, "Climate");
    let _ = writeln!(
        o,
        "  VPD:                 mean {:.2} kPa, min {:.2}, max {:.2}",
        s.vpd_mean_kpa, s.vpd_min_kpa, s.vpd_max_kpa
    );
    let _ = writeln!(o);
    let _ = writeln!(o, "Plants ({} plants, {ground:.1} m² ground)", p.plants);
    let _ = writeln!(
        o,
        "  Transpiration:       {:.2} mm/day ({:.1} kg total)",
        s.transpiration_mm_day, s.transpiration_kg
    );
    let _ = writeln!(o, "  Latent load:         {:.2} kWh", s.plant_latent_kwh);
    let _ = writeln!(
        o,
        "  Water reclaimed:     {:.1} L (condensate to makeup tank)",
        s.water_reclaimed_l
    );
    let _ = writeln!(o);
    let _ = writeln!(o, "Loads (kWh)");
    let _ = writeln!(o, "  Envelope:            {:.2}", s.envelope_kwh);
    let _ = writeln!(o, "  Make-up air:         {:.2}", s.ventilation_kwh);
    let _ = writeln!(
        o,
        "  Process (lights, pumps, controls): {:.2}",
        s.process_load_kwh
    );
    let _ = writeln!(o);
    let _ = writeln!(o, "HVAC electricity (kWh)");
    let _ = writeln!(o, "  Sensible cooling:    {:.2}", s.cooling_kwh);
    let _ = writeln!(o, "  Sensible heating:    {:.2}", s.heating_kwh);
    let _ = writeln!(o, "  Dehumidification:    {:.2}", s.dehumidification_kwh);
    let _ = writeln!(o, "  Total HVAC:          {:.2}", s.hvac_kwh);
    let _ = writeln!(o);
    let tariff_text = match &p.tariff {
        Some(t) => format!(
            "peak {:.2}, off-peak {:.2} per kWh",
            t.peak_nzd_kwh, t.offpeak_nzd_kwh
        ),
        None => "not set".to_string(),
    };
    let _ = writeln!(o, "HVAC cost (NZD; tariff {tariff_text})");
    let _ = writeln!(
        o,
        "  Off-peak:            {:.2} kWh, ${:.2}",
        s.hvac_offpeak_kwh, s.hvac_cost_offpeak_nzd
    );
    let _ = writeln!(
        o,
        "  Peak:                {:.2} kWh, ${:.2}",
        s.hvac_peak_kwh, s.hvac_cost_peak_nzd
    );
    let _ = writeln!(o, "  Total:               ${:.2}", s.hvac_cost_nzd);
    let _ = writeln!(o);
    let _ = writeln!(
        o,
        "Checks: energy bookkeeping residual {:.6} kWh",
        s.closure_residual_kwh
    );
    if !rejected.is_empty() {
        let _ = writeln!(o, "Rejected rows:");
        for r in rejected.iter().take(REJECTION_PREVIEW) {
            let _ = writeln!(o, "  line {}: {}", r.line, r.reason);
        }
        if rejected.len() > REJECTION_PREVIEW {
            let _ = writeln!(o, "  ... and {} more", rejected.len() - REJECTION_PREVIEW);
        }
    }
    let _ = writeln!(
        o,
        "Parameters are assumptions, not measurements. See docs/phase1-model.md."
    );
    o
}

/// Renders the summary and parameters as a JSON object with a fixed key order.
pub fn json(s: &Summary, p: &SimParams, rejected: &[Rejection]) -> String {
    let mut fields: Vec<(&str, String)> = vec![
        ("samples", s.samples.to_string()),
        ("rejected_rows", rejected.len().to_string()),
        ("intervals", s.intervals.to_string()),
        ("gap_intervals", s.gap_intervals.to_string()),
        ("first_ts", quote(&format_rfc3339_utc(s.first_ts_s))),
        ("last_ts", quote(&format_rfc3339_utc(s.last_ts_s))),
        ("simulated_s", num(s.simulated_s)),
        ("vpd_mean_kpa", num(s.vpd_mean_kpa)),
        ("vpd_min_kpa", num(s.vpd_min_kpa)),
        ("vpd_max_kpa", num(s.vpd_max_kpa)),
        ("transpiration_kg", num(s.transpiration_kg)),
        ("transpiration_mm_day", num(s.transpiration_mm_day)),
        ("plant_latent_kwh", num(s.plant_latent_kwh)),
        ("water_reclaimed_l", num(s.water_reclaimed_l)),
        ("humidified_kg", num(s.humidified_kg)),
        ("envelope_kwh", num(s.envelope_kwh)),
        ("ventilation_kwh", num(s.ventilation_kwh)),
        ("process_load_kwh", num(s.process_load_kwh)),
        ("cooling_kwh", num(s.cooling_kwh)),
        ("heating_kwh", num(s.heating_kwh)),
        ("dehumidification_kwh", num(s.dehumidification_kwh)),
        ("hvac_kwh", num(s.hvac_kwh)),
        ("hvac_offpeak_kwh", num(s.hvac_offpeak_kwh)),
        ("hvac_peak_kwh", num(s.hvac_peak_kwh)),
        ("hvac_cost_nzd", num(s.hvac_cost_nzd)),
        ("hvac_cost_offpeak_nzd", num(s.hvac_cost_offpeak_nzd)),
        ("hvac_cost_peak_nzd", num(s.hvac_cost_peak_nzd)),
        ("closure_residual_kwh", num(s.closure_residual_kwh)),
    ];
    let params = [
        ("plants", p.plants.to_string()),
        ("lai", num(p.lai)),
        ("g_leaf_max_m_s", num(p.g_leaf_max_m_s)),
        ("cop_cooling", num(p.cop_cooling)),
        ("cop_heating", num(p.cop_heating)),
        ("dehumidifier_mer_kg_kwh", num(p.dehumidifier_mer_kg_kwh)),
        (
            "tariff_peak_nzd_kwh",
            p.tariff
                .as_ref()
                .map_or("null".to_string(), |t| num(t.peak_nzd_kwh)),
        ),
        (
            "tariff_offpeak_nzd_kwh",
            p.tariff
                .as_ref()
                .map_or("null".to_string(), |t| num(t.offpeak_nzd_kwh)),
        ),
        ("utc_offset_h", num(p.utc_offset_h)),
    ];
    let params_json = object(&params);
    fields.push(("parameters", params_json));
    let rej: Vec<String> = rejected
        .iter()
        .map(|r| format!("{{\"line\":{},\"reason\":{}}}", r.line, quote(&r.reason)))
        .collect();
    fields.push(("rejected", format!("[{}]", rej.join(","))));
    object(&fields)
}

fn num(v: f64) -> String {
    format!("{v:.6}")
}

fn object<S: AsRef<str>>(fields: &[(S, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}:{}", quote(k.as_ref()), v))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// Minimal JSON string escaping for the characters this report can contain.
fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_json_specials() {
        assert_eq!(quote("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }

    #[test]
    fn json_is_one_object_with_fixed_keys() {
        let s = Summary::default();
        let out = json(&s, &SimParams::default(), &[]);
        assert!(out.starts_with('{') && out.ends_with('}'));
        assert!(out.contains("\"transpiration_mm_day\":0.000000"));
        assert!(out.contains("\"rejected\":[]"));
    }
}
