# Phase 2 edge loop: behaviour and placeholders

Written for: engineers wiring the edge loop to real sensors and actuators, and the maintainer
judging whether it is safe to run unattended.

## What it does

Each control cycle (`tpt-agripod-edge::cycle::ControlLoop::step`):

1. **Validate sensors.** Moisture, reservoir level and grid headroom are each checked for a
   missing reading, a stale one (older than 30 s), an out-of-range or non-finite value, and, for
   moisture only, a frozen value (the same reading six cycles running). Any fault goes to safe
   state. Nothing is held over or interpolated.
2. **Watch for a fail-open pump.** If the reservoir falls by more than 2 L between cycles while
   the pump and valve were commanded off, the loop cuts power (`Actuators::isolate`) and
   latches. It stays safe until an operator calls `reset`.
3. **Propose.** Single-zone hysteresis (`control::Irrigator`): start a pulse below 35 % moisture,
   stop above 45 %, pulse at most 60 s, rest 900 s between pulses.
4. **Check contracts.** `pump.contract` and `solenoid.contract` from `edge/contracts/` are
   evaluated with exact arithmetic. A denial by either goes to safe state, with the solver's
   reasons in the journal.
5. **Actuate.** A hardware error goes to safe state. Safe state is valve closed and pump off.

Every decision is recorded as a structured `Event`.

## What is not a safety input

The fleet link. The spec requires the edge to run offline, so losing the cloud connection does
not stop irrigation or trigger safe state. The actuator bus is different: if commands cannot be
delivered, the loop goes to safe state.

## Placeholders to replace before a field trial

| Value | Where | Basis |
|---|---|---|
| Moisture thresholds 35 % / 45 % | `IrrigationPolicy` | Assumption for the pumice/coir blend. Calibrate against the sensor |
| Pulse 60 s, cooldown 900 s | `IrrigationPolicy` | Assumption. Sets the water bound (about 66 open cycles per 10,000 s) |
| Valve open 10 s per cycle, pump duty 0.12 | `IrrigationPolicy` | Assumption. Must stay under the contract limits (0.15 and 900 s) |
| Drain limit 2 L per cycle | `LoopConfig` | Assumption. Set from the reservoir's normal evaporation and draw |
| Sensor max age 30 s, stuck after 6 cycles | `ControlLoop::new` | Assumption. Needs the real sensor's sample rate |
| Solenoid `valve_open_s <= 900` | `edge/contracts/solenoid.contract` | **Placeholder.** Confirm against the flood study |
| LED headroom `> 500 W` | `edge/contracts/led.contract` | **Placeholder** |
| Pump headroom `> 2000 W` and reservoir `> 50 L` | `edge/contracts/pump.contract` | From spec section 4.2 |

## Limits

- The loop assumes a 10 s cycle. The irrigation timing is in seconds and assumes the loop is
  stepped about every 10 s. Nothing enforces that yet.
- A valve that sticks open with the pump off is not detected. The sensors available cannot see
  it, because the water goes into the media, not the reservoir.
- Latching covers pump fail-open only. Other faults clear on their own once the sensors are
  good again.
- Cutting power on latch depends on hardware that is not built yet. Until it is, `isolate` is
  a call into an unimplemented driver.
- No daemon, persistence, or real I/O yet. The loop is a library, tested on the host with fake
  actuators and sensors.
