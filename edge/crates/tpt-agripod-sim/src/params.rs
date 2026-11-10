// SPDX-License-Identifier: MIT OR Apache-2.0

//! Model parameters for the Phase 1 container simulation.
//!
//! Every default is an assumption, not a measurement. The basis for each is in
//! `docs/phase1-model.md`. Replace them with measured values as the hardware arrives.

/// All parameters the Phase 1 model reads.
#[derive(Debug, Clone, PartialEq)]
pub struct SimParams {
    /// Number of potato plants in the container.
    pub plants: u32,
    /// Ground area per plant, m². Potato spacing of 0.3 m × 0.9 m.
    pub ground_area_per_plant_m2: f64,
    /// Leaf area index of the canopy, m²/m².
    pub lai: f64,
    /// Maximum leaf conductance to water vapour, m/s.
    pub g_leaf_max_m_s: f64,
    /// Canopy PPFD while lights are on, µmol/(m²·s).
    pub ppfd_umol_m2_s: f64,
    /// PPFD at half-maximum light response, µmol/(m²·s).
    pub ppfd_half_sat_umol_m2_s: f64,
    /// Start of the lit period, local clock hour.
    pub photoperiod_start_h: f64,
    /// Length of the lit period, hours.
    pub photoperiod_h: f64,
    /// Local time offset from UTC, hours. New Zealand standard time is +12. Daylight saving is ignored.
    pub utc_offset_h: f64,
    /// Envelope surface area, m². A 40 ft high-cube container is about 144 m² external.
    pub envelope_area_m2: f64,
    /// Envelope thermal resistance, m²·K/W. R-30 imperial is RSI 5.28.
    pub envelope_rsi: f64,
    /// Multiplier on envelope conductance for thermal bridging and framing.
    pub bridging_factor: f64,
    /// Interior air volume, m³. A 40 ft high-cube interior is about 73 m³ (2.35 m wide, 2.6 m high).
    pub air_volume_m3: f64,
    /// Filtered make-up air, air changes per hour.
    pub ventilation_ach: f64,
    /// Outdoor air temperature used when the input has no `ambient_c` column, °C.
    pub ambient_default_c: f64,
    /// Outdoor air relative humidity, fraction. Used to set the moisture of make-up air.
    pub ambient_rh_frac: f64,
    /// Coefficient of performance for sensible cooling (heat pump), dimensionless.
    pub cop_cooling: f64,
    /// Coefficient of performance for sensible heating (heat pump), dimensionless.
    pub cop_heating: f64,
    /// Dehumidifier moisture removal efficiency, kg of water removed per kWh of electricity.
    pub dehumidifier_mer_kg_kwh: f64,
    /// Electricity price in the peak band, NZD/kWh. Placeholder: replace with the real tariff.
    pub tariff_peak_nzd_kwh: f64,
    /// Electricity price in the off-peak band, NZD/kWh. Placeholder: replace with the real tariff.
    pub tariff_offpeak_nzd_kwh: f64,
    /// Start of the off-peak band, local clock hour.
    pub offpeak_start_h: f64,
    /// End of the off-peak band, local clock hour. A value below the start wraps past midnight.
    pub offpeak_end_h: f64,
    /// Largest time gap between accepted samples that is still integrated, seconds.
    pub max_gap_s: i64,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            plants: 100,
            ground_area_per_plant_m2: 0.27,
            lai: 3.0,
            g_leaf_max_m_s: 0.004,
            ppfd_umol_m2_s: 300.0,
            ppfd_half_sat_umol_m2_s: 150.0,
            photoperiod_start_h: 6.0,
            photoperiod_h: 16.0,
            utc_offset_h: 12.0,
            envelope_area_m2: 144.0,
            envelope_rsi: 5.28,
            bridging_factor: 1.05,
            air_volume_m3: 73.0,
            ventilation_ach: 0.2,
            ambient_default_c: 15.0,
            ambient_rh_frac: 0.7,
            cop_cooling: 3.0,
            cop_heating: 3.0,
            dehumidifier_mer_kg_kwh: 1.2,
            tariff_peak_nzd_kwh: 0.30,
            tariff_offpeak_nzd_kwh: 0.15,
            offpeak_start_h: 21.0,
            offpeak_end_h: 7.0,
            max_gap_s: 900,
        }
    }
}

impl SimParams {
    /// Checks that every parameter is finite and inside its physical range.
    pub fn validate(&self) -> Result<(), String> {
        check("plants", self.plants >= 1)?;
        check(
            "ground_area_per_plant_m2",
            pos(self.ground_area_per_plant_m2),
        )?;
        check("lai", pos(self.lai))?;
        check("g_leaf_max_m_s", pos(self.g_leaf_max_m_s))?;
        check("ppfd_umol_m2_s", pos(self.ppfd_umol_m2_s))?;
        check("ppfd_half_sat_umol_m2_s", pos(self.ppfd_half_sat_umol_m2_s))?;
        check(
            "photoperiod_start_h",
            in_range(self.photoperiod_start_h, 0.0, 24.0),
        )?;
        check("photoperiod_h", in_range(self.photoperiod_h, 0.0, 24.0))?;
        check("utc_offset_h", in_range(self.utc_offset_h, -12.0, 14.0))?;
        check("envelope_area_m2", pos(self.envelope_area_m2))?;
        check("envelope_rsi", pos(self.envelope_rsi))?;
        check("bridging_factor", in_range(self.bridging_factor, 1.0, 3.0))?;
        check("air_volume_m3", pos(self.air_volume_m3))?;
        check("ventilation_ach", in_range(self.ventilation_ach, 0.0, 10.0))?;
        check("ambient_rh_frac", in_range(self.ambient_rh_frac, 0.0, 1.0))?;
        check("cop_cooling", pos(self.cop_cooling))?;
        check("cop_heating", pos(self.cop_heating))?;
        check("dehumidifier_mer_kg_kwh", pos(self.dehumidifier_mer_kg_kwh))?;
        check(
            "tariff_peak_nzd_kwh",
            in_range(self.tariff_peak_nzd_kwh, 0.0, 10.0),
        )?;
        check(
            "tariff_offpeak_nzd_kwh",
            in_range(self.tariff_offpeak_nzd_kwh, 0.0, 10.0),
        )?;
        check("offpeak_start_h", in_range(self.offpeak_start_h, 0.0, 24.0))?;
        check("offpeak_end_h", in_range(self.offpeak_end_h, 0.0, 24.0))?;
        check("max_gap_s", self.max_gap_s > 0)?;
        Ok(())
    }

    /// Whether the lights are on at `ts_s`, given the photoperiod and time zone.
    pub fn lights_on(&self, ts_s: i64) -> bool {
        let hour = crate::time::local_hour(ts_s, self.utc_offset_h);
        let since_start = (hour - self.photoperiod_start_h).rem_euclid(24.0);
        since_start < self.photoperiod_h
    }

    /// Whether `ts_s` falls in the off-peak tariff band.
    pub fn is_offpeak(&self, ts_s: i64) -> bool {
        let hour = crate::time::local_hour(ts_s, self.utc_offset_h);
        let since_start = (hour - self.offpeak_start_h).rem_euclid(24.0);
        let width = (self.offpeak_end_h - self.offpeak_start_h).rem_euclid(24.0);
        since_start < width
    }
}

fn pos(v: f64) -> bool {
    v.is_finite() && v > 0.0
}

fn in_range(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

fn check(name: &str, ok: bool) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(format!("parameter {name} is out of range or not finite"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339_utc;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(SimParams::default().validate(), Ok(()));
    }

    #[test]
    fn rejects_nonsense() {
        let p = SimParams {
            cop_cooling: 0.0,
            ..SimParams::default()
        };
        assert!(p.validate().unwrap_err().contains("cop_cooling"));
        let p = SimParams {
            ambient_rh_frac: 1.2,
            ..SimParams::default()
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn lights_follow_the_local_photoperiod() {
        let p = SimParams::default(); // on 06:00-22:00 NZST (UTC+12)
        let at = |s: &str| parse_rfc3339_utc(s).unwrap();
        assert!(!p.lights_on(at("2026-01-01T17:59:00Z"))); // 05:59 local
        assert!(p.lights_on(at("2026-01-01T18:00:00Z"))); // 06:00 local
        assert!(p.lights_on(at("2026-01-02T09:59:00Z"))); // 21:59 local
        assert!(!p.lights_on(at("2026-01-02T10:00:00Z"))); // 22:00 local
    }

    #[test]
    fn offpeak_band_wraps_midnight() {
        let p = SimParams::default(); // 21:00-07:00 local
        let at = |s: &str| parse_rfc3339_utc(s).unwrap();
        assert!(p.is_offpeak(at("2026-01-01T09:00:00Z"))); // 21:00 local
        assert!(p.is_offpeak(at("2026-01-01T18:59:00Z"))); // 06:59 local
        assert!(!p.is_offpeak(at("2026-01-01T19:00:00Z"))); // 07:00 local
        assert!(!p.is_offpeak(at("2026-01-01T02:00:00Z"))); // 14:00 local
    }
}
