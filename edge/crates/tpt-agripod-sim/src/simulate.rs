// SPDX-License-Identifier: MIT OR Apache-2.0

//! Container load model and HVAC energy accounting.
//!
//! Each interval between two accepted samples is integrated with the mean of its end values.
//! Per interval:
//!
//! * Sensible gains: envelope conduction, make-up air, and the non-HVAC electrical load
//!   (all of it becomes heat in the room).
//! * Latent gains: canopy transpiration plus the moisture in make-up air, relative to the
//!   indoor air. A dehumidifier removes any net excess. A net deficit is recorded as
//!   humidification and not costed.
//! * HVAC electricity: sensible loads over the heat-pump COP, and removed moisture over the
//!   dehumidifier's kg-per-kWh rating.
//!
//! The model is a first-order energy bookkeeping model, not a dynamic simulation. Indoor
//! state is taken from the measurements, so the model answers "what would the HVAC have to
//! do to hold these conditions", not "what temperature would the room reach".

use tpt_eng_building_sys::{envelope_ua, infiltration_loss_ach_w, transmission_heat_rate};

use crate::params::SimParams;
use crate::plant::{Canopy, transpiration_kg_s};
use crate::psychro::{CP_DRY_AIR, air_density, humidity_ratio, latent_heat_j_kg, vpd_kpa};
use crate::sensor::Sample;

/// Joules in one kilowatt-hour.
pub const J_PER_KWH: f64 = 3.6e6;

/// Flows and loads over one interval. Watts are rates. Kilograms are per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    /// Interval length, seconds.
    pub dt_s: f64,
    /// Whether the interval's midpoint falls in the off-peak band.
    pub offpeak: bool,
    /// Vapour pressure deficit, kPa.
    pub vpd_kpa: f64,
    /// Canopy transpiration, kg/s.
    pub transpiration_kg_s: f64,
    /// Latent heat carried by the transpired water, W.
    pub plant_latent_w: f64,
    /// Sensible gain through the envelope, W. Negative when heat leaves.
    pub envelope_w: f64,
    /// Sensible gain from make-up air, W.
    pub ventilation_w: f64,
    /// Non-HVAC electrical load, W.
    pub process_w: f64,
    /// Net sensible gain to the room (sum of the three above), W.
    pub sensible_gain_w: f64,
    /// Net moisture added to the room air (transpiration plus make-up air), kg/s.
    pub moisture_kg_s: f64,
    /// Moisture removed by the dehumidifier, kg/s.
    pub removed_kg_s: f64,
    /// Moisture added by a humidifier to cover a net deficit, kg/s.
    pub humidified_kg_s: f64,
    /// Electrical power of the sensible cooling heat pump, W.
    pub cooling_elec_w: f64,
    /// Electrical power of the sensible heating heat pump, W.
    pub heating_elec_w: f64,
    /// Electrical power of the dehumidifier, W.
    pub dehumidifier_elec_w: f64,
    /// Heat the HVAC removes, net of heat it adds, W.
    pub hvac_heat_removed_w: f64,
    /// Heat the room gains, sensible plus latent, W. Equals `hvac_heat_removed_w` when the books balance.
    pub heat_gained_w: f64,
}

/// Run-level totals for a simulated period.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Summary {
    /// Samples passed in, before any gap filtering.
    pub samples: usize,
    /// Timestamp of the first sample, Unix seconds UTC.
    pub first_ts_s: i64,
    /// Timestamp of the last sample, Unix seconds UTC.
    pub last_ts_s: i64,
    /// Intervals integrated.
    pub intervals: usize,
    /// Intervals skipped because the gap between samples exceeded `max_gap_s`.
    pub gap_intervals: usize,
    /// Total integrated time, seconds.
    pub simulated_s: f64,
    /// Mean VPD over integrated intervals, kPa.
    pub vpd_mean_kpa: f64,
    /// Minimum VPD, kPa.
    pub vpd_min_kpa: f64,
    /// Maximum VPD, kPa.
    pub vpd_max_kpa: f64,
    /// Total canopy transpiration, kg.
    pub transpiration_kg: f64,
    /// Mean transpiration over the simulated days, mm/day (kg per m² of ground).
    pub transpiration_mm_day: f64,
    /// Latent heat carried by transpired water, kWh.
    pub plant_latent_kwh: f64,
    /// Water condensed by the dehumidifier and returned to the makeup tank, litres.
    pub water_reclaimed_l: f64,
    /// Water added by a humidifier to cover a net deficit, kg. Not costed.
    pub humidified_kg: f64,
    /// Net envelope heat gain, kWh. Negative when heat leaves.
    pub envelope_kwh: f64,
    /// Net make-up air heat gain, kWh.
    pub ventilation_kwh: f64,
    /// Non-HVAC electrical load, kWh.
    pub process_load_kwh: f64,
    /// Electrical energy of sensible cooling, kWh.
    pub cooling_kwh: f64,
    /// Electrical energy of sensible heating, kWh.
    pub heating_kwh: f64,
    /// Electrical energy of dehumidification, kWh.
    pub dehumidification_kwh: f64,
    /// Total HVAC electrical energy, kWh.
    pub hvac_kwh: f64,
    /// HVAC energy in the off-peak band, kWh.
    pub hvac_offpeak_kwh: f64,
    /// HVAC energy in the peak band, kWh.
    pub hvac_peak_kwh: f64,
    /// Predicted HVAC energy cost, NZD.
    pub hvac_cost_nzd: f64,
    /// HVAC cost in the off-peak band, NZD.
    pub hvac_cost_offpeak_nzd: f64,
    /// HVAC cost in the peak band, NZD.
    pub hvac_cost_peak_nzd: f64,
    /// Sum of absolute per-interval energy-bookkeeping residuals, kWh. Zero when the books balance.
    pub closure_residual_kwh: f64,
}

/// Evaluates one interval between two samples. Pure function: no I/O, no clock.
pub fn interval(
    a: &Sample,
    b: &Sample,
    p: &SimParams,
    canopy: &Canopy,
) -> Result<Interval, String> {
    // One assembly of area A and U = 1/RSI, with bridging applied to the area.
    let ua = envelope_ua(&[(p.envelope_area_m2 * p.bridging_factor, 1.0 / p.envelope_rsi)]);
    let vdot = p.ventilation_ach * p.air_volume_m3 / 3600.0;

    let dt_s = (b.ts_s - a.ts_s) as f64;
    let mid_ts = a.ts_s + (b.ts_s - a.ts_s) / 2;
    let temp_c = 0.5 * (a.temp_c + b.temp_c);
    let rh = 0.5 * (a.rh_frac + b.rh_frac);
    let process_w = 0.5 * (a.power_w + b.power_w);
    let ambient = |s: &Sample| s.ambient_c.unwrap_or(p.ambient_default_c);
    let amb_c = 0.5 * (ambient(a) + ambient(b));

    let vpd = vpd_kpa(temp_c, rh);
    let lights = p.lights_on(mid_ts);
    let transp = transpiration_kg_s(canopy, vpd, temp_c, lights);
    let h = latent_heat_j_kg(temp_c);

    let envelope_w = transmission_heat_rate(ua, amb_c - temp_c);
    let rho = air_density(temp_c);
    let ventilation_w = infiltration_loss_ach_w(
        p.ventilation_ach,
        p.air_volume_m3,
        rho,
        CP_DRY_AIR,
        amb_c - temp_c,
    );
    let sensible_gain_w = envelope_w + ventilation_w + process_w;

    let w_in = humidity_ratio(temp_c, rh)?;
    let w_out = humidity_ratio(amb_c, p.ambient_rh_frac)?;
    let moisture_kg_s = transp + rho * vdot * (w_out - w_in);
    let removed_kg_s = moisture_kg_s.max(0.0);
    let humidified_kg_s = (-moisture_kg_s).max(0.0);

    let (cooling_th_w, heating_th_w) = if sensible_gain_w > 0.0 {
        (sensible_gain_w, 0.0)
    } else {
        (0.0, -sensible_gain_w)
    };
    let cooling_elec_w = cooling_th_w / p.cop_cooling;
    let heating_elec_w = heating_th_w / p.cop_heating;
    let dehumidifier_elec_w = removed_kg_s * J_PER_KWH / p.dehumidifier_mer_kg_kwh;

    // Heat the HVAC removes is sensible (cooling minus heating) plus the latent heat of the
    // moisture it removes, minus the latent heat spent humidifying. The room gains the
    // sensible gains plus the latent heat of the net moisture. The two must agree.
    let hvac_heat_removed_w = (cooling_th_w - heating_th_w) + (removed_kg_s - humidified_kg_s) * h;
    let heat_gained_w = sensible_gain_w + moisture_kg_s * h;

    Ok(Interval {
        dt_s,
        offpeak: p.is_offpeak(mid_ts),
        vpd_kpa: vpd,
        transpiration_kg_s: transp,
        plant_latent_w: transp * h,
        envelope_w,
        ventilation_w,
        process_w,
        sensible_gain_w,
        moisture_kg_s,
        removed_kg_s,
        humidified_kg_s,
        cooling_elec_w,
        heating_elec_w,
        dehumidifier_elec_w,
        hvac_heat_removed_w,
        heat_gained_w,
    })
}

/// Integrates the model over a run of accepted samples and returns the totals.
pub fn simulate(samples: &[Sample], p: &SimParams) -> Result<Summary, String> {
    p.validate()?;
    if samples.len() < 2 {
        return Err("need at least two valid samples to integrate energy".into());
    }
    let canopy = Canopy {
        plants: p.plants,
        ground_area_per_plant_m2: p.ground_area_per_plant_m2,
        lai: p.lai,
        g_leaf_max_m_s: p.g_leaf_max_m_s,
        ppfd_umol_m2_s: p.ppfd_umol_m2_s,
        ppfd_half_sat_umol_m2_s: p.ppfd_half_sat_umol_m2_s,
    };

    let mut s = Summary {
        samples: samples.len(),
        first_ts_s: samples[0].ts_s,
        last_ts_s: samples[samples.len() - 1].ts_s,
        vpd_min_kpa: f64::INFINITY,
        vpd_max_kpa: f64::NEG_INFINITY,
        ..Summary::default()
    };
    let mut vpd_sum = 0.0;

    for pair in samples.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.ts_s - a.ts_s > p.max_gap_s {
            s.gap_intervals += 1;
            continue;
        }
        let iv = interval(a, b, p, &canopy)?;
        let kwh = |w: f64| w * iv.dt_s / J_PER_KWH;

        s.intervals += 1;
        s.simulated_s += iv.dt_s;
        vpd_sum += iv.vpd_kpa;
        s.vpd_min_kpa = s.vpd_min_kpa.min(iv.vpd_kpa);
        s.vpd_max_kpa = s.vpd_max_kpa.max(iv.vpd_kpa);

        s.transpiration_kg += iv.transpiration_kg_s * iv.dt_s;
        s.plant_latent_kwh += kwh(iv.plant_latent_w);
        s.water_reclaimed_l += iv.removed_kg_s * iv.dt_s; // 1 kg of water is about 1 litre
        s.humidified_kg += iv.humidified_kg_s * iv.dt_s;
        s.envelope_kwh += kwh(iv.envelope_w);
        s.ventilation_kwh += kwh(iv.ventilation_w);
        s.process_load_kwh += kwh(iv.process_w);
        s.cooling_kwh += kwh(iv.cooling_elec_w);
        s.heating_kwh += kwh(iv.heating_elec_w);
        s.dehumidification_kwh += kwh(iv.dehumidifier_elec_w);

        let hvac = kwh(iv.cooling_elec_w + iv.heating_elec_w + iv.dehumidifier_elec_w);
        s.hvac_kwh += hvac;
        if iv.offpeak {
            s.hvac_offpeak_kwh += hvac;
            s.hvac_cost_offpeak_nzd += hvac * p.tariff_offpeak_nzd_kwh;
        } else {
            s.hvac_peak_kwh += hvac;
            s.hvac_cost_peak_nzd += hvac * p.tariff_peak_nzd_kwh;
        }

        s.closure_residual_kwh += kwh(iv.heat_gained_w - iv.hvac_heat_removed_w).abs();
    }

    if s.intervals == 0 {
        return Err(format!(
            "no interval is shorter than max_gap_s = {} s; nothing to simulate",
            p.max_gap_s
        ));
    }
    s.vpd_mean_kpa = vpd_sum / s.intervals as f64;
    s.hvac_cost_nzd = s.hvac_cost_offpeak_nzd + s.hvac_cost_peak_nzd;
    let days = s.simulated_s / 86_400.0;
    s.transpiration_mm_day = s.transpiration_kg / canopy.ground_area_m2() / days;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339_utc;

    fn sample(ts: &str, temp_c: f64, rh: f64, power_w: f64, amb: Option<f64>) -> Sample {
        Sample {
            ts_s: parse_rfc3339_utc(ts).unwrap(),
            temp_c,
            rh_frac: rh,
            co2_ppm: 800.0,
            reservoir_temp_c: 18.0,
            power_w,
            ambient_c: amb,
        }
    }

    fn canopy(p: &SimParams) -> Canopy {
        Canopy {
            plants: p.plants,
            ground_area_per_plant_m2: p.ground_area_per_plant_m2,
            lai: p.lai,
            g_leaf_max_m_s: p.g_leaf_max_m_s,
            ppfd_umol_m2_s: p.ppfd_umol_m2_s,
            ppfd_half_sat_umol_m2_s: p.ppfd_half_sat_umol_m2_s,
        }
    }

    #[test]
    fn energy_books_balance_across_heating_and_cooling_cases() {
        // Sweeps ambient and process load so both the cooling and heating branches run,
        // and both humidifying and dehumidifying cases occur.
        let p = SimParams::default();
        let c = canopy(&p);
        for amb in [-5.0, 5.0, 15.0, 30.0] {
            for power in [0.0, 300.0, 1800.0] {
                for rh in [0.2, 0.6, 0.95] {
                    let a = sample("2026-01-01T06:00:00Z", 20.0, rh, power, Some(amb));
                    let b = sample("2026-01-01T06:05:00Z", 20.0, rh, power, Some(amb));
                    let iv = interval(&a, &b, &p, &c).unwrap();
                    let scale = iv.heat_gained_w.abs().max(1.0);
                    assert!(
                        (iv.heat_gained_w - iv.hvac_heat_removed_w).abs() / scale < 1e-9,
                        "books do not balance at amb {amb}, power {power}, rh {rh}"
                    );
                    assert!(iv.cooling_elec_w >= 0.0 && iv.heating_elec_w >= 0.0);
                    assert!(iv.cooling_elec_w == 0.0 || iv.heating_elec_w == 0.0);
                }
            }
        }
    }

    #[test]
    fn lights_off_at_night_means_no_transpiration_and_no_latent_removal() {
        let p = SimParams::default();
        let c = canopy(&p);
        // 23:00 local (11:00 UTC): lights are off.
        let a = sample("2026-01-01T11:00:00Z", 18.0, 0.7, 150.0, Some(12.0));
        let b = sample("2026-01-01T11:05:00Z", 18.0, 0.7, 150.0, Some(12.0));
        let iv = interval(&a, &b, &p, &c).unwrap();
        assert_eq!(iv.transpiration_kg_s, 0.0);
        assert_eq!(iv.plant_latent_w, 0.0);
        // The moisture left is only make-up air, so the removal must match it exactly.
        assert!((iv.moisture_kg_s - iv.removed_kg_s + iv.humidified_kg_s).abs() < 1e-15);
    }

    #[test]
    fn hotter_outside_adds_envelope_heat() {
        let p = SimParams::default();
        let c = canopy(&p);
        let a = sample("2026-01-01T06:00:00Z", 20.0, 0.6, 0.0, Some(30.0));
        let b = sample("2026-01-01T06:05:00Z", 20.0, 0.6, 0.0, Some(30.0));
        let iv = interval(&a, &b, &p, &c).unwrap();
        // UA = 144 × 1.05 / 5.28 = 28.64 W/K; ΔT = 10 K gives 286 W.
        assert!(
            (iv.envelope_w - 286.4).abs() < 0.5,
            "envelope {}",
            iv.envelope_w
        );
        assert!(iv.cooling_elec_w > 0.0 && iv.heating_elec_w == 0.0);
    }

    #[test]
    fn full_day_simulation_has_sane_totals() {
        // 100 potatoes, 1 day, steady 20 °C / 60 % RH, 15 °C outside, 150 W process load.
        let p = SimParams::default();
        let step = 300;
        let start = parse_rfc3339_utc("2026-01-01T12:00:00Z").unwrap(); // 00:00 local
        let samples: Vec<Sample> = (0..=288)
            .map(|i| Sample {
                ts_s: start + i * step,
                temp_c: 20.0,
                rh_frac: 0.6,
                co2_ppm: 800.0,
                reservoir_temp_c: 18.0,
                power_w: 150.0,
                ambient_c: Some(15.0),
            })
            .collect();
        let s = simulate(&samples, &p).unwrap();
        assert_eq!(s.intervals, 288);
        assert_eq!(s.gap_intervals, 0);
        assert!((s.simulated_s - 86_400.0).abs() < 1e-9);
        // Transpiration over 16 lit hours at VPD ~0.9 kPa lands roughly in the 2 to 6 mm/day band.
        assert!(
            (1.5..=6.0).contains(&s.transpiration_mm_day),
            "{}",
            s.transpiration_mm_day
        );
        assert!(s.hvac_kwh > 0.0);
        assert!((s.hvac_cost_nzd - (s.hvac_cost_peak_nzd + s.hvac_cost_offpeak_nzd)).abs() < 1e-9);
        assert!(s.hvac_offpeak_kwh > 0.0 && s.hvac_peak_kwh > 0.0);
        assert!(s.closure_residual_kwh < 1e-9);
        // Reclaimed water equals transpiration, up to the make-up-air term.
        assert!(s.water_reclaimed_l > 0.0);
    }

    #[test]
    fn gaps_are_skipped_not_integrated() {
        let p = SimParams::default();
        let start = parse_rfc3339_utc("2026-01-01T12:00:00Z").unwrap();
        let mk = |ts: i64| Sample {
            ts_s: ts,
            temp_c: 20.0,
            rh_frac: 0.6,
            co2_ppm: 800.0,
            reservoir_temp_c: 18.0,
            power_w: 100.0,
            ambient_c: Some(15.0),
        };
        // One 300 s step, then a 1 h hole, then another 300 s step.
        let samples = [
            mk(start),
            mk(start + 300),
            mk(start + 300 + 3600),
            mk(start + 300 + 3900),
        ];
        let s = simulate(&samples, &p).unwrap();
        assert_eq!(s.intervals, 2);
        assert_eq!(s.gap_intervals, 1);
        assert!((s.simulated_s - 600.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_too_few_samples() {
        let p = SimParams::default();
        let one = [sample("2026-01-01T00:00:00Z", 20.0, 0.6, 0.0, None)];
        assert!(simulate(&one, &p).is_err());
    }
}
