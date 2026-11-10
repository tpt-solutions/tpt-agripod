# Phase 1 model: container load and HVAC energy

Written for: the maintainer and engineers changing `edge/crates/tpt-agripod-sim`, and anyone
judging whether the predicted energy cost can be trusted yet.

## What it does

`agripod simulate --input FILE` reads sensor readings (indoor temperature, RH, CO2, reservoir
temperature, non-HVAC power, and optionally outdoor temperature) and reports:

- VPD over the run, from `tpt-eng-props-air`.
- Canopy transpiration for the crop, and the latent heat and water it carries.
- Envelope, make-up air and process heat gains.
- HVAC electricity: sensible heating and cooling, and dehumidification.
- Predicted HVAC energy cost, split by off-peak and peak band.

It does **not** predict indoor temperature. Indoor state comes from the measurements. The
model answers "what would the HVAC have to do to hold these conditions", not "what would
the room do without control".

## Equations

Interval `i` runs between two accepted samples. Rates use the mean of the two end values.

| Quantity | Equation |
|---|---|
| Vapour pressure deficit | `VPD = (1 − RH) · p_sat(T)` (ASHRAE Hyland–Wexler, via `tpt-eng-props-air`) |
| Canopy transpiration | `ṁ = LAI · g_leaf · Δρ_v · A_ground`, with `Δρ_v = VPD / (R_v · T)` and `g_leaf = g_max · PPFD / (PPFD + K)` while lit, zero in darkness |
| Envelope gain | `Q_env = UA · (T_amb − T)`, with `UA = A · f_bridge / RSI`. Computed by `tpt_eng_building_sys::envelope_ua` and `transmission_heat_rate` |
| Make-up air gain | `Q_vent = ρ · c_p · V̇ · (T_amb − T)`, with `V̇ = ACH · V / 3600`. Computed by `tpt_eng_building_sys::infiltration_loss_ach_w` |
| Process gain | `Q_proc = P_nonHVAC` (all electrical load becomes room heat) |
| Net sensible | `Q_s = Q_env + Q_vent + Q_proc`. Positive means cooling is needed |
| Moisture to remove | `ṁ_net = ṁ_transp + ρ · V̇ · (W_out − W_in)`. A positive value is removed by the dehumidifier |
| HVAC electricity | Cooling `Q_s / COP_c`, heating `−Q_s / COP_h`, dehumidifier `ṁ_removed · 3.6×10⁶ / MER` |
| Cost | Electricity per interval times the band's tariff, band taken at the interval midpoint |

Latent heat of vaporisation is `h_fg = 2501 − 2.361·T` kJ/kg (ASHRAE). Air density is dry-air
density at standard pressure.

## Parameters (all assumptions)

Defaults are in `edge/crates/tpt-agripod-sim/src/params.rs`. Every one can be overridden on the
CLI or in code.

| Parameter | Default | Basis | Status |
|---|---|---|---|
| Plants | 100 | Spec, section 6 | Specified |
| Ground area per plant | 0.27 m² | Potato spacing 0.3 m × 0.9 m | Assumption. 100 plants need about 27 m² at this spacing, which is more than the floor of a 20 ft container (about 14 m²). A 40 ft container is needed, or the spacing must be tighter. |
| LAI | 3.0 | Typical mid-season potato canopy | Assumption, constant across the cycle |
| `g_leaf_max` | 0.004 m/s | Chosen so that a 16 h lit day at 1 kPa VPD gives about 3 to 4 mm/day, the field potato ET band | **Calibration guess. Not measured.** |
| PPFD / half-saturation | 300 / 150 µmol/(m²·s) | Typical canopy PPFD for a lit crop | Assumption |
| Photoperiod | 06:00 to 22:00 local (16 h) | Lettuce-style schedule; potato in the spec is not photoperiod-sensitive | Assumption |
| Time zone | UTC+12 (NZ standard time) | Summer daylight saving is ignored | Assumption |
| Envelope area / RSI | 144 m² / 5.28 m²K/W | 40 ft high-cube exterior estimate; R-30 imperial is RSI 5.28 | Estimate |
| Bridging factor | 1.05 | Spray-foam or PIR, so little thermal bridging | Assumption |
| Air volume / ventilation | 73 m³ / 0.2 ACH | Interior estimate; filtered make-up air | Estimate |
| Outdoor RH | 0.70 | Sets the moisture in make-up air | Assumption |
| COP cooling / heating | 3.0 / 3.0 | Horticultural heat-pump range | Assumption |
| Dehumidifier MER | 1.2 kg/kWh | Conservative for a horticultural dehumidifier; check the supplier's rating at the right temperature and RH | Assumption |
| Tariff peak / off-peak | 0.30 / 0.15 NZD/kWh | Placeholder | **Placeholder. Replace with the real tariff.** |
| Off-peak window | 21:00 to 07:00 local | Common NZ night-rate window | Assumption; confirm with the retailer |
| Max gap | 900 s | Two missed 5-minute readings | Assumption |

## Input contract

CSV with header `ts,temp_c,rh_frac,co2_ppm,reservoir_temp_c,power_w`, plus optional
`ambient_c`. Column order is free. Timestamps are RFC 3339 UTC with a `Z` suffix.

- `power_w` is read as the **non-HVAC** electrical load (lighting, pumps, controls). Metered
  total draw that includes the HVAC would double count. Revisit when a meter is fitted.
- Rows that fail validation are rejected with a line number and reason. They are never
  silently fixed. Range limits are in `sensor.rs`.
- Gaps longer than `max_gap_s` are skipped, not interpolated, and counted in the report.

## Validation done

| Check | Where | Result |
|---|---|---|
| Saturation pressure at 0, 10, 20, 25, 30 °C against the ASHRAE table | `psychro.rs` tests | Within 0.5 %. The reference values are from memory of the ASHRAE table, so re-check them against the printed table before relying on this. |
| VPD hand calculation (20 °C, 50 % RH = 1.170 kPa) | `psychro.rs` | Pass |
| Humidity ratio at 20 °C, 50 % RH (≈ 0.00726) | `psychro.rs` | Pass |
| Latent heat against steam tables | `psychro.rs` | Within 0.2 % |
| Transpiration hand calculation (1 kPa, 20 °C, lit): 1.5966×10⁻³ kg/s | `plant.rs` | Pass to 10⁻⁹ relative against the formula |
| Transpiration plausibility: 2 to 6 mm/day at 1 kPa, 16 h light | `plant.rs` | Pass (3.4 mm/day). **A bound, not a validation against data.** |
| VPD rises with temperature and falls with RH | `psychro.rs` | Pass over a grid |
| Heat-balance bookkeeping: heat gained equals heat removed by the HVAC, across heating and cooling, dry and humid cases | `simulate.rs` | Pass to 10⁻⁹. This checks the sign and branch logic. It does not check the physics. |
| Lights, off-peak window, and midnight wrap | `params.rs` | Pass |
| Timestamp parsing, including leap years and invalid dates | `time.rs` | Pass |
| Ingest rejects bad rows and keeps the rest | `sensor.rs` | Pass |
| Gap skipping | `simulate.rs` | Pass |
| Deterministic mock data; same seed gives same bytes | `mock.rs` | Pass |
| Golden report for 1 day, seed 42 | `tests/golden.rs` | Pass. Run on Windows here. CI runs it on Linux as well. Cross-platform identity is confirmed only when CI runs. |

Determinism: the model uses no wall clock and no randomness outside the seeded generator. The
psychrometric functions use the pure-Rust `libm` through `tpt-math-numeric`, not the platform
math library. The golden test formats numbers to fixed precision.

## Known limits

1. **No dynamics.** Indoor state is measured. Thermal mass, the time to reach setpoint, and
   overshoot are not modelled. Those belong to the Phase 3 reservoir and thermal-battery work.
2. **Dehumidifier heat rejection is not modelled.** Dehumidifiers reject their condensing heat
   somewhere. If that heat returns to the room, the sensible cooling load is understated.
3. **Transpiration has no radiation balance and no aerodynamic term** (driving-force model, not
   Penman–Monteith). `g_leaf_max` is the main calibration risk.
4. **No night transpiration.** Stomata are treated as closed in darkness.
5. **Canopy is static.** LAI and PPFD do not change with growth stage. Growth-stage dependence
   comes with the crop profile in Phase 3.
6. **Humidity correction in air density** is ignored (about 1 %).
7. **Daylight saving is ignored.** Use the correct offset per season when reporting costs.
8. **Tariffs are placeholders.** No demand charges, fixed charges, or GST are modelled.
9. **Humidification** (net moisture deficit) is recorded but not costed.
10. **CO2 and reservoir temperature** are validated and kept, but no model uses them yet.
11. **Process load is a measured input.** No lighting or pump model.
12. **Off-peak shifting** (the Power Law) is not implemented. The report only shows what each
    band costs as measured. It does not move load.

## Gate 1 status

The model runs, is tested, and produces a predicted HVAC energy cost. It has **not** been
checked against measured container data, because there is no hardware yet. Treat the cost
figure as an order-of-magnitude estimate until:

- transpiration is checked against a weighed-pot or lysimeter trial, or against published
  potato ET data for the local climate, and `g_leaf_max` is set from that;
- the tariff is replaced with the real NZ tariff; and
- the dehumidifier's MER is taken from the supplier's rating at the operating point.
