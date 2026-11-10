# pH bench prototypes (ESP32)

Two bench sketches for continuous pH monitoring, one per front-end option. They are for bench and field trials only.

- The leaf-tier firmware is a Rust `no_std` crate (see [ADR 0004](../../docs/adr/0004-esp32-leaf-tier.md)). A pH reading path, if it becomes production, belongs there, not in these sketches.
- Arduino framework, built with Arduino IDE or PlatformIO. Target board: any ESP32 dev module.

| | A: analog board + ADS1115 | B: EZO-pH over I2C |
|---|---|---|
| Sketch | `ph_analog_ads1115/ph_analog_ads1115.ino` | `ph_ezo_i2c/ph_ezo_i2c.ino` |
| Front-end | Generic analog pH board (Gravity SEN0161 class) | Atlas Scientific EZO-pH circuit |
| Electrode interface | Board does the buffering; ADS1115 digitises it | Circuit does buffering, temperature math and calibration |
| Calibration stored | ESP32 flash (NVS) | EZO circuit (non-volatile) |
| Temperature compensation | Done in firmware (Nernst scaling) | Done by EZO, fed `T,xx.x` each cycle |
| Effort to get right | Higher: you own the math and validation | Lower: more of it is vendor-tested |
| Cost | Lower | Higher |
| Best for | Cost-sensitive nodes, you want full control | Fastest path to trustworthy readings |

## Shared hardware

- ESP32 I2C: SDA GPIO21, SCL GPIO22
- DS18B20 temperature sensor on GPIO4, 4.7 kOhm pull-up to 3.3 V
- Probe: refillable or double-junction electrode for hydroponics (see the main discussion on junction clogging)
- Keep the probe cable short and shielded. Keep it away from pump motors and mains wiring.

## Scenario A: analog board + ADS1115

Wiring:

| From | To |
|---|---|
| pH board VCC | 3.3 V |
| pH board GND | GND |
| pH board Po (analog out) | ADS1115 A0 |
| ADS1115 VDD / GND | 3.3 V / GND |
| ADS1115 SDA / SCL | GPIO21 / GPIO22 |
| ADS1115 ADDR | GND (address 0x48) |
| DS18B20 DQ | GPIO4 (with 4.7 kOhm to 3.3 V) |

Powering the board from 3.3 V keeps its output within the ADS1115's gain-1 range. Do not wire the board's output directly to an ESP32 pin unless you accept the ADC noise.

Calibration (serial monitor at 115200 baud):

1. Put the probe in pH 7.00 buffer, wait for a stable reading, send `cal7`.
2. Rinse, put it in pH 4.00 buffer, wait, send `cal4`.
3. The sketch computes the slope efficiency. It rejects values outside 0.85 to 1.05, which usually means a bad buffer or probe.
4. `status` shows the stored calibration. Re-calibrate weekly as a starting point.

Model used: `V = V7 + S(T) * (7 - pH)`, with `S(T) = eff * 59.16 mV * (T + 273.15) / 298.15`.

## Scenario B: EZO-pH over I2C

Setup (one time, before the sketch runs):

1. Connect the EZO circuit over UART and switch it to I2C mode per the Atlas datasheet (`I2C,99`, then power cycle). New circuits ship in UART mode.
2. Wire: VCC 3.3 V, GND, SDA GPIO21, SCL GPIO22. Use 4.7 kOhm pull-ups on SDA and SCL if the circuit's board does not already have them.

Calibration (serial monitor at 115200 baud):

1. `cal7`, probe in pH 7.00 buffer
2. `cal4`, probe in pH 4.00 buffer
3. Optional `cal10` for a third point
4. `slope` shows the EZO's acid/base/neutral diagnostics. Check the slope is close to 100%.
5. `clear` erases the EZO calibration.

## Known gaps

- Neither sketch implements a moving median or outlier rejection across cycles. Add that before using readings for control.
- Neither sketch checks probe health beyond the slope or efficiency check at calibration time.
- The EZO sketch assumes the EZO's default I2C address (0x63). Change `EZO_ADDR` if you have changed it.
- Neither sketch has been run on hardware yet. They were written against the datasheets and the standard ESP32 Arduino core, and need a bench test before being trusted.
