// SPDX-License-Identifier: MIT OR Apache-2.0

//! Canopy transpiration: how much water the crop evaporates into the air.
//!
//! Model: vapour flux per unit leaf area is the leaf conductance times the vapour-density
//! difference between the leaf and the air. The density difference is VPD divided by
//! `R_v·T` (ideal gas). Leaf conductance is a maximum scaled by a saturating light response.
//! Stomata are treated as closed in darkness. This is a driving-force model, not a
//! Penman–Monteith model: there is no radiation balance and no aerodynamic term.

use crate::psychro::R_VAPOUR;

/// Crop canopy in the container. All areas are per the whole crop, not per plant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canopy {
    /// Number of plants.
    pub plants: u32,
    /// Ground area occupied by each plant, m².
    pub ground_area_per_plant_m2: f64,
    /// Leaf area index: leaf area per unit ground area, m²/m².
    pub lai: f64,
    /// Maximum leaf conductance to water vapour at full light, m/s.
    pub g_leaf_max_m_s: f64,
    /// Canopy photon flux while lights are on, µmol/(m²·s).
    pub ppfd_umol_m2_s: f64,
    /// Photon flux at which the light response reaches half its maximum, µmol/(m²·s).
    pub ppfd_half_sat_umol_m2_s: f64,
}

impl Canopy {
    /// Total ground area the crop covers, m².
    pub fn ground_area_m2(&self) -> f64 {
        f64::from(self.plants) * self.ground_area_per_plant_m2
    }

    /// Leaf conductance at the current lighting, m/s.
    pub fn leaf_conductance_m_s(&self, lights_on: bool) -> f64 {
        if !lights_on {
            return 0.0;
        }
        let light = self.ppfd_umol_m2_s / (self.ppfd_umol_m2_s + self.ppfd_half_sat_umol_m2_s);
        self.g_leaf_max_m_s * light
    }
}

/// Transpiration rate of the whole canopy, kg of water per second.
///
/// `vpd_kpa` and `temp_c` describe the air at the canopy. `lights_on` gates the stomatal
/// response: with lights off the rate is zero.
pub fn transpiration_kg_s(canopy: &Canopy, vpd_kpa: f64, temp_c: f64, lights_on: bool) -> f64 {
    let vpd_pa = vpd_kpa.max(0.0) * 1000.0;
    let density_deficit_kg_m3 = vpd_pa / (R_VAPOUR * (temp_c + 273.15));
    let per_ground_kg_m2_s =
        canopy.lai * canopy.leaf_conductance_m_s(lights_on) * density_deficit_kg_m3;
    per_ground_kg_m2_s * canopy.ground_area_m2()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn potato_bed() -> Canopy {
        Canopy {
            plants: 100,
            ground_area_per_plant_m2: 0.27,
            lai: 3.0,
            g_leaf_max_m_s: 0.004,
            ppfd_umol_m2_s: 300.0,
            ppfd_half_sat_umol_m2_s: 150.0,
        }
    }

    #[test]
    fn matches_hand_calculation() {
        // VPD 1.0 kPa at 20 °C: deficit = 1000 / (461.5 × 293.15) ≈ 0.0073919 kg/m³.
        // Light factor = 300 / (300 + 150) = 2/3, so g = 0.004 × 2/3 ≈ 0.0026667 m/s.
        // Per ground: 3.0 × 0.0026667 × 0.0073919 ≈ 5.9135e-5 kg/(m²·s).
        // Ground area: 100 × 0.27 = 27 m². Total ≈ 1.5966e-3 kg/s.
        let expected = 3.0 * 0.004 * (2.0 / 3.0) * (1000.0 / (461.5 * 293.15)) * 27.0;
        let kg_s = transpiration_kg_s(&potato_bed(), 1.0, 20.0, true);
        assert!(
            (kg_s - expected).abs() / expected < 1e-9,
            "got {kg_s}, expected {expected}"
        );
        assert!((kg_s - 1.5966e-3).abs() < 1e-7);
    }

    #[test]
    fn daily_rate_is_in_the_plausible_potato_band() {
        // Field potato ET at peak canopy is roughly 3 to 5 mm/day. At VPD 1 kPa, 16 h of light,
        // the model should land inside 2 to 6 mm/day. This is a plausibility bound, not validation.
        let kg_s = transpiration_kg_s(&potato_bed(), 1.0, 20.0, true);
        let mm_per_day = kg_s * 16.0 * 3600.0 / potato_bed().ground_area_m2();
        assert!((2.0..=6.0).contains(&mm_per_day), "{mm_per_day} mm/day");
    }

    #[test]
    fn no_transpiration_in_darkness_or_at_zero_deficit() {
        assert_eq!(transpiration_kg_s(&potato_bed(), 1.0, 20.0, false), 0.0);
        assert_eq!(transpiration_kg_s(&potato_bed(), 0.0, 20.0, true), 0.0);
    }

    #[test]
    fn scales_linearly_with_plant_count() {
        let mut half = potato_bed();
        half.plants = 50;
        let full = transpiration_kg_s(&potato_bed(), 1.2, 22.0, true);
        let half_rate = transpiration_kg_s(&half, 1.2, 22.0, true);
        assert!((full - 2.0 * half_rate).abs() < 1e-12);
    }
}
