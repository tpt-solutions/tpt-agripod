# TPT Agripod: Project Todo

Software-defined, physics-informed, modular controlled environment agriculture (CEA).
Maintainer: Solo (TPT Solutions). Licence: **MIT OR Apache-2.0** (dual). Source of truth: `spec.txt`.

**Legend:** `[ ]` todo, `[x]` done. Tick as you go. **Gate** items must pass before the next phase starts.

**Standing rules (apply to every phase)**
- Every dependency (Rust crate, Go module, firmware crate) must be MIT, BSD, ISC, Zlib, Unlicense, or MIT/Apache dual. **No Apache-only, GPL/LGPL/AGPL, MPL or EPL.**
- No gRPC/protobuf, NATS or other Apache-only stacks. Edge to fleet is MQTT + JSON.
- Deterministic, offline-capable edge. Safety logic is never allowed to depend on the network.

| Phase | Name | Spec timing | Status |
|---|---|---|---|
| 0 | Foundation | before wk 1 | [ ] |
| 1 | Proof of Physics | wk 1-4 | [ ] |
| 2 | Single-Zone Prototype | wk 5-12 | [ ] |
| 3 | Full Stack Integration | wk 13-20 | [ ] |
| 4 | Modular Scaling | wk 21+ | [ ] |
| 5 | Hardening & Rollout | after Phase 4 | [ ] |
| A | Hardware track (parallel) | wk 1+ | [ ] |

---

## Phase 0: Foundation

### Repo & licensing
- [x] Init monorepo (git) with layout: `/edge` (Cargo workspace), `/fleet` (Go modules), `/esp32` (no_std firmware), `/profiles`, `/docs`, `/hardware`
- [x] Add `LICENSE-MIT` and `LICENSE-APACHE` (copyright TPT Solutions)
- [x] README with dual-licence statement ("MIT OR Apache-2.0, at your option") and contribution clause
- [x] `license = "MIT OR Apache-2.0"` in every `Cargo.toml`; SPDX header convention for Go and Rust files
- [x] CONTRIBUTING (issues only), `.gitignore`, `.editorconfig`
- [x] ADR log in `/docs/adr` (record decisions: monorepo, MQTT+JSON, own QF_LRA solver, ESP32 leaf tier, Keystone sink)

### Licence enforcement (no Apache-only deps)
- [x] `deny.toml` for `cargo-deny`: allowlist MIT, BSD-2/3, ISC, Zlib, Unlicense, and `MIT OR Apache-2.0` expressions; deny Apache-only, copyleft, MPL, EPL
- [x] Go: `go-licenses` check with the same allowlist
- [x] Add a CI job that fails the build on any disallowed licence (`cargo deny` gate in `.github/workflows/ci.yml`, plus a `go-licenses` job; not yet run on GitHub)
- [ ] Audit `esp-hal` and chosen Modbus, MQTT and Postgres-driver crates and modules against the policy
- [x] Check licence of the Keystone Go client and of the chosen MQTT client and broker (paho is EPL/EDL; verify or pick an alternative). MQTT: ADR 0006, 0007, all MIT. Keystone Go SDK (`tpt-keystone-db/sdk-go`): standard library only, MIT OR Apache-2.0 (`docs/dependency-pins.md`). Its module path needs resolving before import (ADR 0009)

### Upstream TPT crates
- [x] Locate upstream: `tpt-solutions/tpt-engineering` (43 crates; includes `tpt-eng-props-air`, `tpt-eng-props-water`, `tpt-eng-props-mixture`, `tpt-eng-controls`, `tpt-eng-timeseries-*`; all `tpt-eng-props-*` are no_std) and `tpt-solutions/tpt-thermodynamics` (`tpt-thermo-*`, 12 crates)
- [x] Check whether the crates are on crates.io; otherwise pin as git dependencies (by commit) and document the version policy
- [ ] Both repos are MIT OR Apache-2.0 (workspace level); confirm per-crate `license` fields and transitive deps (`tpt-math` from crates.io) pass `deny.toml`
- [ ] Reuse the upstream `deny.toml`, `rust-toolchain.toml` (edition 2024, MSRV 1.85), `rustfmt.toml` and `cargo xtask` conventions as the starting point for `/edge`
- [x] Evaluate `tpt-eng-controls`, `tpt-eng-timeseries-*` and `tpt-eng-heat-transfer` / `-thermal-mgmt` / `-building-sys` for reuse in Phase 1 (container load model). Result: adopted `tpt-eng-building-sys` 0.2 (envelope UA, transmission, infiltration; formulas match the hand model). Not adopted for Phase 1: `tpt-eng-controls` (PID, Phase 2), `timeseries-*` (Phase 3 telemetry), `thermal-mgmt` (heat sinks, not relevant). Note: this evaluation was done after the Phase 1 model was first written, not before, and the model was then switched over.
- [x] Check that `tpt-thermo-electrolyte` (Pitzer/eNRTL/HKF) and `tpt-thermo-transport` can compute solution density and temperature-compensated EC/pH. Result: no. Read at rev `67c773a`: no conductivity or pH functions in the electrolyte crate; see Open questions
- [ ] Note that only `tpt-thermo-core` is no_std; keep thermo usage on the edge PC, not the ESP32
- [ ] Record any other missing API (IAPWS-IF97 enthalpy, mixture model) as upstream issues

### Toolchain & CI
- [x] Rust workspace skeleton: `tpt-agripod-core`, `-sim`, `-solver`, `-actuators`, `-edge`, `-cli`
- [x] Go module skeleton: `tpt-agripod-orchestrator`, `tpt-agripod-telemetry`
- [ ] ESP32 firmware crate skeleton (`esp-hal`, no_std)
- [ ] CI matrix: x86_64 (NUC), aarch64 (Raspberry Pi), Jetson (aarch64) cross-builds; ESP32 target build
- [x] CI: `cargo fmt`, `clippy -D warnings`, `cargo test`, `go vet`, `go test -race` (defined in `ci.yml`; fmt, clippy, test, deny and vet pass locally; `-race` runs on Linux in CI)
- [x] Windows dev setup notes (cross-compile to aarch64, espup for ESP32)

### Contracts
- [x] Crop Profile JSON Schema v0 (VPD target, DLI, N-P-K, photoperiod, root-zone temp, drip schedule, light spectrum), with a `kind` discriminator (`plant` now, `culture` later) and a `controls` block of named setpoints so the schema stays biology-agnostic
- [x] MQTT topic map and message schemas (profile push, telemetry, commands, status/last-will)
- [x] Units and sign conventions doc (SI throughout)

**Gate 0:** CI green on all targets; licence check passes; upstream crates resolve.

---

## Phase 1: Proof of Physics (wk 1-4)

- [x] Mock sensor data format (CSV: temperature, RH, CO2, reservoir temperature, power, optional ambient) and generator (`agripod generate-mock`)
- [x] Sensor ingest and validation (range checks, timestamps, missing data; bad rows rejected with line numbers)
- [x] VPD calculation via `tpt-eng-props-air` (ASHRAE psychrometrics)
- [ ] Plant transpiration model for 100 potato plants (leaf area, VPD, light, growth stage). Partial: LAI and light are constant; no growth-stage dependence yet. Calibration of `g_leaf_max` is a guess, not measured
- [x] Latent and sensible heat load model of the container (envelope R-30+, lighting, equipment)
- [x] HVAC and dehumidifier energy model; predicted energy cost output (NZ tariff, off-peak aware). Tariff is a placeholder
- [x] CLI tool: ingest mock data, simulate, output report (human and JSON)
- [ ] Determinism: fixed seeds, no wall-clock; identical output across platforms. Fixed seed and golden test pass on Windows. Cross-platform identity is confirmed only when CI's Linux job runs
- [x] Golden-file tests and property tests (energy books balance, VPD monotonicity). Note: the balance test checks HVAC sign and branch bookkeeping, not the physics
- [x] Validate against reference psychrometric tables and a hand calculation (reference values from the ASHRAE table as recalled; re-check against the printed table)
- [x] Short write-up of the model, assumptions and known limits in `/docs` (`docs/phase1-model.md`)

**Gate 1 (deliverable):** verified, deterministic thermodynamic model of the container, with predicted HVAC energy cost.

---

## Phase 2: Single-Zone Prototype (wk 5-12)

### Actuator HAL (`tpt-agripod-actuators`)
- [ ] `no_std` Modbus RTU over RS-485 (master and slave); CRC, framing, timeouts. Partial: master done and built for `thumbv7em-none-eabihf`, with CRC-16 vectors and exception handling. Slave (ESP32 side) not written
- [ ] Device abstractions: solenoid/relay, peristaltic pump, VFD, LED driver, moisture sensor. Blocked: needs the register maps from the device datasheets
- [ ] Mock transport for host tests; hardware transport for Linux serial. Partial: mock done. Linux serial transport not written
- [ ] Fuzz and property tests on the frame parser. Partial: seeded property tests (20,000 random frames) done. No coverage-guided fuzzing yet

### Constraint solver (QF_LRA)
- [ ] Core: exact rational arithmetic (no GPL big-num crates) and simplex-based QF_LRA decision procedure. Partial: checked `i128` rationals done; Fourier–Motzkin used first (ADR 0008); simplex not written
- [x] Contract DSL: `requires:` / `ensures:` with linear constraints and boolean connectives (AND, OR, NOT, parentheses; unit suffixes rejected)
- [x] Contract evaluator: check actuator command against state before execution; produce a violation explanation
- [ ] Differential tests against a reference solver run offline (not shipped as a dependency). Partial: checked against brute-force grid search. No reference solver is installed
- [ ] Bounded runtime (worst-case time budget) and `no_std` feasibility check. Partial: decision procedure has a constraint budget; the solver builds for `thumbv7em-none-eabihf` with `alloc`. Worst-case timing not measured
- [x] Contracts for: flood prevention (pump duty, reservoir level), power headroom, LED dimming limits (`edge/contracts/`; the solenoid and LED thresholds are placeholders, see `docs/phase2-edge-loop.md`)

### Edge control loop
- [ ] Edge daemon: sensor read, state estimate, controller proposal, solver check, actuate, log. Partial: the control cycle is written and tested with fake I/O. The daemon process, real sensor reads and real actuation are not
- [x] Moisture-driven irrigation controller for a single zone (hysteresis, pulse limit, cooldown; thresholds are placeholders)
- [x] Safe state on solver failure, sensor loss or comms loss (valves close, pump off). Comms loss means the actuator bus; the fleet link is not a safety input (`docs/phase2-edge-loop.md`)
- [ ] Fault injection: pump fails open, valve sticks, sensor reads stuck or out of range. Partial: pump fail-open, stuck, stale, out-of-range, missing and actuator-error cases tested. A valve stuck open with the pump off is not detectable with the current sensors
- [ ] Structured local logging and a local event journal. Partial: structured `Event` type and a `Journal` trait. No durable on-disk journal yet

### ESP32 leaf tier
- [ ] `esp-hal` firmware: Modbus RTU slave, moisture ADC, solenoid and pump outputs
- [ ] Hardware watchdog: on loss of master heartbeat, all valves fail closed
- [ ] Independent flood guard on the node (max valve-open time, hard-coded)
- [ ] Flash and OTA-less update procedure documented

### Prototype integration (see Appendix A for the physical build)
- [ ] Raspberry Pi bring-up; edge daemon running under systemd
- [ ] Hardware-in-the-loop test: moisture read, contract evaluated, solenoid triggered
- [ ] Soak test (multi-day) with no flooding; record results

**Gate 2 (deliverable):** Rust code reads moisture, evaluates the QF_LRA contract and triggers the solenoid without flooding, including under fault injection.

---

## Phase 3: Full Stack Integration (wk 13-20)

### Orchestrator (Go)
- [x] State machine for the ~120-day grow cycle (stages, transitions, timers, manual overrides). Stage durations are a placeholder template
- [x] Persistent state (file or embedded store with a permissive licence). Atomic JSON file, no database
- [ ] Crop Profile loader and validator (JSON Schema). Partial: hand-written check of the schema's rules, unknown fields rejected, pump limit checked against the edge contract. Not a full JSON Schema validator
- [ ] MQTT client and profile push to edge nodes; ack, retry and version tracking. Not started: needs `gmqtt` added to `go.mod` (ADR 0007)
- [ ] Node registry, health and last-will handling. Not started
- [ ] Lettuce profile v1 and Potato profile v1. Not started: agronomic inputs needed, not invented here

### Edge (Rust)
- [ ] MQTT client (permissive licence), TLS and auth
- [ ] Apply a Crop Profile at runtime: targets feed controllers and contracts (no recompile)
- [ ] Offline operation: continue on the last profile; buffer telemetry to disk; resync on reconnect
- [ ] Profile validation on the edge (reject unsafe or malformed profiles)

### Physics integration
- [ ] Real-time VPD from the sensor array (`tpt-eng-props-air`)
- [ ] Reservoir thermal model: enthalpy required to heat/cool via `tpt-eng-props-water` (IAPWS-IF97)
- [ ] Predictive thermal-battery scheduling against off-peak hours
- [ ] Nutrient mixture model: EC and pH temperature and density compensation (`tpt-thermodynamics`)

### Telemetry (Go)
- [ ] `tpt-agripod-telemetry`: ingest from MQTT, batch, back-pressure. Partial: batching and bounded back-pressure done; ingest reads JSON Lines from stdin, not MQTT
- [x] Sink interface (pluggable) with file and in-memory sinks for tests (no stdout sink as such; `-out` takes a file path)
- [ ] Keystone DB sink: Postgres wire protocol, `COPY` bulk path, `USING TIME` (Chronos) table schema. Partial: batched INSERT, transactions and the `USING TIME` DDL are written. No `COPY` (SDK lacks it, ADR 0009). Not run against a live server. SDK not yet imported
- [ ] Retention and rollup policy for sensor series. Partial: retention is a placeholder parameter in the DDL. No rollup
- [x] Dedup and idempotent writes after reconnect. Resent points collide on `(node, metric, ts)` and are skipped. Tested against a fake that enforces the key; live check pending (ADR 0009)

### Integration
- [ ] End-to-end test: orchestrator pushes profile, edge applies it, telemetry lands in Keystone
- [ ] Docker Compose (or equivalent) dev stack: broker, orchestrator, telemetry, Keystone
- [ ] Closed-loop single-cartridge run (multi-day)

**Gate 3 (deliverable):** fully automated, closed-loop single cartridge with telemetry flowing into the database.

---

## Phase 4: Modular Scaling (wk 21+)

- [ ] Cartridge abstraction: Type A (deep substrate), B (vertical trellis), C (shallow tray) as data, not code
- [ ] Cartridge descriptor schema (depth, zones, valves, sensors, spool motors) and auto-detect or declare on insert; model it as a generic vessel/bed descriptor (capabilities, sensors, actuators, contracts) so a future bioreactor is just another descriptor
- [ ] Profile-driven control: VPD targets, drip frequency and volume, and light spectrum all from the profile
- [ ] Carrot profile v1 (fine substrate, precise seeding parameters)
- [ ] Swap test: shallow tray (lettuce) to deep trough (carrots) with only the profile and cartridge changed
- [ ] Verify the edge adjusts VPD, drip and lighting automatically with **no code changes**
- [ ] Regression suite: replay recorded runs across all three profiles
- [ ] Update contracts per cartridge type (flow rates, max volumes)
- [ ] Docs: how to author a new profile and cartridge

**Gate 4 (deliverable):** genericity proven by a cartridge and profile swap with no code changes.

---

## Phase 5: Hardening & Rollout

### Remaining crop profiles (all 10 archetypes)
- [ ] Tomatoes (trellis, high DLI, dehumidification)
- [ ] Strawberries (VPD 0.8-1.0 kPa, elevated gutters)
- [ ] Onions (photoperiod triggering, dry curing phase)
- [ ] Cucumbers (transpiration, trellis load)
- [ ] Bell peppers (stable gradients, high potassium)
- [ ] Kumara (warm root zone 20°C+, bi-directional vine spooling)
- [ ] Microgreens/Herbs (7-14 day cycle, flood/drain)
- [ ] Profile test matrix: every profile validates and simulates in Phase 1 CLI

### Resource efficiency laws
- [ ] Express the laws as per-profile data (a list of metrics and limits), not three hard-coded laws, so culture profiles can define their own (e.g. yield per kWh, media recycling)
- [ ] Water Law: metrics for net water input vs. water bound in biomass; condensate reclamation accounting
- [ ] Power Law: peak-draw cap, and non-critical load shifting to off-peak NZ grid hours
- [ ] Light Law: inter-canopy lighting control and DLI delivery; measure the 30% wattage reduction
- [ ] Efficiency dashboards and alerts when a law is violated

### Safety & reliability
- [ ] Failure-mode analysis (FMEA) for pumps, valves, heat pumps, sensors, comms, power loss
- [ ] Second independent safety layer (hardware float switch and flood-sensor cut-off)
- [ ] Solver contract coverage report; contracts reviewed per actuator
- [ ] Long-run soak and chaos testing

### Security & operations
- [ ] Signed Crop Profiles and signed edge updates
- [ ] MQTT mutual TLS; secrets handling; role separation
- [ ] Security review and dependency audit (`cargo audit`, `govulncheck`)
- [ ] Observability: logs, metrics, alerting
- [ ] Fleet UI (operator view of nodes, profiles, cycle status)

### Release
- [ ] User and operator documentation; wiring and commissioning guides
- [ ] Reproducible builds and release artefacts for x86_64, aarch64, Jetson, ESP32
- [ ] Final licence audit (SBOM, NOTICE if required)
- [ ] Tag v1.0.0

**Gate 5:** all 10 profiles validated; safety review done; v1.0.0 released.

---

## Appendix A: Hardware track (parallel to software)

### Single-zone prototype (supports Phase 2)
- [ ] BOM and procurement: Raspberry Pi, ESP32, RS-485 transceivers, capacitive moisture sensor, solenoid valve, submersible pump, relays, PSU
- [ ] 1 m² grow bed with 70% pumice / 30% coco coir
- [ ] Drip irrigation, pressure-compensated emitters, sump and return
- [ ] Wiring diagram, fusing, enclosure, mains safety checks
- [ ] Float switch and independent flood cut-off
- [ ] Commissioning checklist

### Cartridge & chassis (supports Phases 4-5)
- [ ] Roller-rail floor and cartridge dimensions (Types A, B, C)
- [ ] Type A: 40 cm troughs with mesh-bottom sifting
- [ ] Type B: motorised high-tensile spools (vertical trellis)
- [ ] Type C: 10 cm rolling benches
- [ ] Fluidics: central sump, UV-C sterilisation, 100-micron filter, recirculation
- [ ] Insulated main reservoir and water heat pump (thermal battery)
- [ ] Dehumidifier with condensate piped to the freshwater makeup tank
- [ ] Inter-canopy LED fixtures and drivers
- [ ] Positive-pressure HEPA/MERV-13 filtration
- [ ] 20 ft / 40 ft high-cube envelope, R-30+ insulation, condensation check
- [ ] Power: peak-draw metering and off-peak tariff data

---

## Open questions & risks
- [ ] `tpt-thermodynamics` may not cover EC/pH compensation out of the box (electrolyte crate only; unconfirmed); may need an Agripod-side model or upstream work
- [ ] Keystone Go client: licence resolved (MIT OR Apache-2.0, stdlib only). Still open: API stability (Keystone is self-described as not production-hardened), the SDK's module path (`github.com/tpt/sdk-go`) not matching its repo, no `COPY`, and no SQLSTATE on errors. See ADR 0009
- [x] Choice of MQTT client and broker that satisfies the licence policy. Decided: edge client `minimq` (ADR 0006); fleet client `gmqtt` and fleet broker `mochi-mqtt/server`, both MIT (ADR 0007). Neither is added to a `Cargo.toml` or `go.mod` yet.
- [ ] Jetson toolchain and CI runner availability
- [ ] Compliance and electrical safety requirements (NZ) for the container build
- [ ] Upstream `tpt-engineering/deny.toml` allows `Apache-2.0` and `MPL-2.0`, which breaks the standing rule. Our `edge/deny.toml` does not copy them. Confirm no upstream crate pulled in via the git pin needs an exception.
- [ ] `tpt-thermo-*` crates are not on crates.io (only `tpt-eng-props-*`, `tpt-eng-controls` and `tpt-math*` are, at 0.1.1). They are on GitHub (`tpt-solutions/tpt-thermodynamics`, master), so pin them as git dependencies by commit. The workspace and crates are `MIT OR Apache-2.0` (`license.workspace = true`) and both LICENSE files are present. GitHub's "Apache-2.0" label is only the detector picking one file.
- [ ] Upstream `tpt-thermo-electrolyte` and `tpt-thermo-transport` do not cover EC or pH temperature/density compensation. Confirmed at rev `67c773a`: no conductivity or pH functions; density and residual enthalpy come from `tpt-thermo-eos-cubic`, not IAPWS-IF97. File these as upstream issues (not done: no GitHub access from here).
- [x] `unicode-ident` is licensed `(MIT OR Apache-2.0) AND Unicode-3.0`. `Unicode-3.0` is now on the `edge/deny.toml` allow-list by maintainer decision; it is not in the original todo.md list.
- [ ] Go licence check uses `--allowed_licenses`. A dual-licensed Go dependency may be reported as a single licence, so it could be rejected even when it is MIT OR Apache-2.0. Test this when the first real dependency is added.
- [ ] Spec timings are for a solo maintainer; revisit after Gate 2
- [ ] Deferred: sterility (CIP/SIP), biosafety and NZ regulatory scope (MPI/FSANZ) if culture-based production is pursued; see spec section 7
