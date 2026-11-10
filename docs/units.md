# Units and sign conventions

Written for: engineers writing edge, fleet or profile code.

## Principle

Store and exchange values in SI-derived units. Convert only at the boundary (sensor input, display, reporting). Every field name carries its unit, so a value's unit is never implied.

## Units

| Quantity | Unit | Field suffix | Notes |
|---|---|---|---|
| Temperature | degrees Celsius | `_c` | Kelvin is used inside `tpt-eng-props-*`. Convert at that boundary only. |
| Pressure | kilopascal | `_kpa` | The props crates use pascals. Convert at the boundary. |
| Vapour pressure deficit | kilopascal | `_kpa` | Output of VPD calculation. |
| Relative humidity | fraction, 0 to 1 | `_frac` | Not percent. |
| Humidity ratio | kg water per kg dry air | `_kg_kg` | |
| Mass | kilogram | `_kg` | |
| Volume | litre | `_l` | Litres are used for reservoirs and drip volumes. |
| Flow | litre per second | `_l_s` | |
| Power | watt | `_w` | |
| Energy | kilowatt-hour | `_kwh` | Billing and reports. |
| Time (duration) | second | `_s` | Minute and hour are allowed only in profile schedules, with explicit suffixes `_min` and `_h`. |
| Timestamp | RFC 3339, UTC, `Z` suffix | `ts` | Never local time on the wire. Local time is for display only. |
| Light intensity | micromole per square metre per second | `_umol_m2_s` | PPFD. |
| Daily light integral | mole per square metre per day | `_mol_m2_day` | DLI. |
| Electrical conductivity | siemens per metre | `_s_m` | Reported in mS/cm in UI, stored in S/m. |
| pH | dimensionless | `ph` | |
| Duty cycle | fraction, 0 to 1 | `_frac` | Pump and LED duty. |

## Sign conventions

- Energy and power flowing **into** a system are positive. Heat **removed** from the reservoir is negative.
- Water flowing **into** the container is positive. Drainage out of the container is negative.
- Setpoint offsets are signed: positive raises the target.

## Numerical rules

- Use `f64` throughout the edge simulation. Fixed-point is not used.
- Compare floats with an explicit tolerance named in the code, never with `==`.
- Reject NaN and infinity at the sensor-ingest boundary.
