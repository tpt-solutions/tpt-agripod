// SPDX-License-Identifier: MIT OR Apache-2.0

//! Moist-air properties, delegated to `tpt-eng-props-air` (ASHRAE Hyland–Wexler).
//! This module only adapts units and adds the few quantities the models need on top.

use tpt_eng_props_air::{humidity_ratio as ashrae_humidity_ratio, saturation_pressure_water};
use tpt_math_units::uom::si::f64::{Pressure, ThermodynamicTemperature};
use tpt_math_units::uom::si::pressure::{kilopascal, pascal};
use tpt_math_units::uom::si::thermodynamic_temperature::kelvin;

/// Standard atmospheric pressure, kPa. The container is assumed to sit at sea-level pressure.
pub const STANDARD_PRESSURE_KPA: f64 = 101.325;
/// Specific gas constant of water vapour, J/(kg·K).
pub const R_VAPOUR: f64 = 461.5;
/// Specific gas constant of dry air, J/(kg·K).
pub const R_DRY_AIR: f64 = 287.05;
/// Specific heat capacity of dry air at constant pressure, J/(kg·K).
pub const CP_DRY_AIR: f64 = 1006.0;

const KELVIN_OFFSET: f64 = 273.15;

/// Saturation vapour pressure of water over liquid (or ice below 0 °C), in pascals.
pub fn saturation_pressure_pa(temp_c: f64) -> f64 {
    let t = ThermodynamicTemperature::new::<kelvin>(temp_c + KELVIN_OFFSET);
    saturation_pressure_water(t).get::<pascal>()
}

/// Vapour pressure deficit in kPa for dry-bulb `temp_c` (°C) and relative humidity `rh_frac` (0 to 1).
///
/// VPD is the gap between what the air could hold at saturation and what it holds now.
/// It is zero at 100 % RH and grows with temperature at fixed RH.
pub fn vpd_kpa(temp_c: f64, rh_frac: f64) -> f64 {
    (1.0 - rh_frac) * saturation_pressure_pa(temp_c) / 1000.0
}

/// Humidity ratio (kg water per kg dry air) at dry-bulb `temp_c` and relative humidity `rh_frac`,
/// at standard pressure.
pub fn humidity_ratio(temp_c: f64, rh_frac: f64) -> Result<f64, String> {
    let p_w = rh_frac * saturation_pressure_pa(temp_c);
    ashrae_humidity_ratio(
        Pressure::new::<pascal>(p_w),
        Pressure::new::<kilopascal>(STANDARD_PRESSURE_KPA),
    )
    .map_err(|e| format!("humidity ratio at {temp_c} °C, RH {rh_frac}: {e:?}"))
}

/// Density of dry air at standard pressure and `temp_c` (°C), kg/m³.
///
/// Ignores the small humidity correction (about 1 %). Good enough for the air-side loads here.
pub fn air_density(temp_c: f64) -> f64 {
    STANDARD_PRESSURE_KPA * 1000.0 / (R_DRY_AIR * (temp_c + KELVIN_OFFSET))
}

/// Latent heat of vaporisation of water at `temp_c` (°C), J/kg.
///
/// Linear fit `h_fg = 2501 − 2.361·T` kJ/kg (ASHRAE Fundamentals). Accurate to about 0.1 %
/// between 0 and 40 °C.
pub fn latent_heat_j_kg(temp_c: f64) -> f64 {
    (2501.0 - 2.361 * temp_c) * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference saturation pressures (kPa) from the ASHRAE Fundamentals psychrometric table.
    const PSAT_TABLE_KPA: [(f64, f64); 5] = [
        (0.0, 0.6113),
        (10.0, 1.2281),
        (20.0, 2.3392),
        (25.0, 3.1690),
        (30.0, 4.2470),
    ];

    #[test]
    fn saturation_pressure_matches_reference_table() {
        for (t, expected_kpa) in PSAT_TABLE_KPA {
            let got = saturation_pressure_pa(t) / 1000.0;
            let rel = (got - expected_kpa).abs() / expected_kpa;
            assert!(
                rel < 0.005,
                "psat({t} C) = {got} kPa, table {expected_kpa} kPa"
            );
        }
    }

    #[test]
    fn vpd_matches_hand_calculation() {
        // 20 °C, 50 % RH: VPD = 0.5 × 2.3392 kPa = 1.1696 kPa.
        assert!((vpd_kpa(20.0, 0.5) - 1.1696).abs() < 0.005);
        // Saturated air has no deficit.
        assert_eq!(vpd_kpa(25.0, 1.0), 0.0);
    }

    #[test]
    fn vpd_rises_with_temperature_and_falls_with_humidity() {
        for rh in [0.3, 0.5, 0.7, 0.9] {
            let mut prev = f64::NEG_INFINITY;
            for step in 0..=40 {
                let t = -5.0 + step as f64; // -5 .. 35 °C
                let v = vpd_kpa(t, rh);
                assert!(v > prev, "VPD not increasing in T at RH {rh}, T {t}");
                prev = v;
            }
        }
        for t in [5.0, 15.0, 25.0] {
            let mut prev = f64::INFINITY;
            for step in 0..=20 {
                let rh = step as f64 / 20.0;
                let v = vpd_kpa(t, rh);
                assert!(v <= prev, "VPD not decreasing in RH at T {t}, RH {rh}");
                prev = v;
            }
        }
    }

    #[test]
    fn humidity_ratio_matches_reference_point() {
        // 20 °C, 50 % RH at 101.325 kPa: W ≈ 0.00726 kg/kg (ASHRAE table ~0.0073).
        let w = humidity_ratio(20.0, 0.5).unwrap();
        assert!((w - 0.00726).abs() < 0.0001, "W = {w}");
    }

    #[test]
    fn latent_heat_matches_steam_table_within_a_fraction_of_a_percent() {
        // Steam tables: h_fg(20 °C) = 2453.5 kJ/kg, h_fg(0 °C) = 2501.3 kJ/kg.
        assert!((latent_heat_j_kg(20.0) / 1000.0 - 2453.5).abs() / 2453.5 < 0.002);
        assert!((latent_heat_j_kg(0.0) / 1000.0 - 2501.0).abs() < 0.5);
    }
}
