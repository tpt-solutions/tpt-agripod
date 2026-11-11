# Upstream dependency pins

Written for: engineers adding or updating a dependency on the TPT upstream crates.

## Policy

1. **crates.io first.** Use a crates.io release when one exists. Pin to a minor version (`"0.1"`), so patch fixes arrive and breaking changes do not.
2. **Git only when not on crates.io.** Pin to an exact commit (`rev = "<full sha>"`), never a branch or tag. Record the reason in `edge/Cargo.toml`.
3. **One pin per upstream repo.** Every crate from `tpt-solutions/tpt-thermodynamics` uses the same `rev`, so the workspace never mixes two states of that repo.
4. **Only allowed git sources.** `edge/deny.toml` lists the git repositories allowed in `allow-git`. Add a repository there only through an ADR.
5. **Licence check on every move.** When a pin changes, run `cargo deny check` from `edge/` and confirm the new licences are on the allow-list.

## Current pins

| Crate | Source | Pin | Notes |
|---|---|---|---|
| `tpt-eng-props-air` | crates.io | `0.1` (0.1.1 at time of writing) | ASHRAE psychrometrics. `no_std`. |
| `tpt-eng-props-water` | crates.io | `0.1` (0.1.1 at time of writing) | IAPWS-IF97 Regions 1, 2 and 4. Region 3 unsupported. `no_std`. Not used by any member until Phase 3. |
| `tpt-math-units` | crates.io | `0.1` | Re-exports `uom`. Needed to build and read the typed quantities the props-air API takes and returns. `MIT OR Apache-2.0`. |
| `tpt-eng-building-sys` | crates.io | `0.2` | Envelope UA, transmission and infiltration load formulas, used by the Phase 1 load model. Pulls in `tpt-eng-heat-transfer` 0.2 transitively. `MIT OR Apache-2.0`. Evaluated in `todo.md`, Phase 0. |
| `tpt-thermo-core` | git `tpt-solutions/tpt-thermodynamics` | `67c773ac38b75d6198e8152bc62a8b40102353ec` (master, 2026-09-04) | Only `no_std` crate in that repo. |
| `tpt-thermo-electrolyte` | same | same rev | Pitzer, eNRTL, HKF. No EC or pH compensation (open issue in `todo.md`). |
| `tpt-thermo-transport` | same | same rev | Viscosity, thermal conductivity, diffusivity. |

The `tpt-thermo-*` pins are declared under `[workspace.dependencies]` in `edge/Cargo.toml`. A member crate opts in with `tpt-thermo-core = { workspace = true }`. No member uses them yet, so they are not in `Cargo.lock`.

## Verification of the current pins

- Compiled in a scratch crate outside the repo: all five resolve and `cargo check` passes.
- The `tpt-thermo-*` licences are `MIT OR Apache-2.0`, inherited from the workspace.
- `cargo deny` passes the transitive licences. `Unicode-3.0` (needed by `unicode-ident` via `serde_derive`) is on the allow-list by maintainer decision.

## Licence audit

Checked against the policy in `todo.md` (MIT, BSD-2/3, ISC, Zlib, Unlicense, or MIT/Apache-2.0 dual; no Apache-only, copyleft, MPL or EPL). Licences are from crates.io metadata unless noted.

| Component | Candidate use | Licence | Verdict |
|---|---|---|---|
| `esp-hal` | ESP32 firmware | MIT OR Apache-2.0 | Allowed |
| `tokio-modbus` | Modbus on the edge host (std only) | MIT OR Apache-2.0 | Allowed. Not `no_std`, so the RTU framer in `tpt-agripod-actuators` stays in-house |
| `tokio-serial` | Serial port for Modbus RTU (host) | MIT | Allowed |
| `rumqttc` | MQTT client (edge) | Apache-2.0 only | **Denied** |
| `paho-mqtt` | MQTT client (edge) | EPL-2.0 | **Denied** |
| `mqttrs` | MQTT codec | Apache-2.0 only | **Denied** |
| `rumqttd` | MQTT broker (Rust) | Apache-2.0 only | **Denied** |
| `minimq` | MQTT client, `no_std` | MIT | Allowed. Candidate; needs a transport layer |
| `mqtt-protocol` | MQTT codec | MIT/Apache-2.0 | Allowed. Candidate |
| `tokio-postgres` | Postgres driver (Rust, if needed) | MIT OR Apache-2.0 | Allowed |
| `pgx` (Go) | Postgres driver for telemetry to Keystone | MIT | Allowed |
| `yosssi/gmq` v0.0.1 (Go) | Fleet MQTT client (ADR 0010) | MIT. Stale since 2018; behind one file | Allowed |
| `mochi-mqtt/server/v2` v2.7.9 (Go) | In-process broker for tests only | MIT | Allowed. Test-only, so not in the shipped build |
| `gorilla/websocket` v1.5.0 (Go) | Transitive, via mochi | BSD-2-Clause | Allowed |
| `rs/xid` v1.4.0 (Go) | Transitive, via mochi | MIT | Allowed |
| `jinzhu/copier` v0.3.5 (Go) | Transitive, via mochi | MIT (`License` file in module) | Allowed |
| `gopkg.in/check.v1` (Go) | Transitive, via mochi | BSD-2-Clause style (gocheck) | Allowed |
| `gopkg.in/yaml.v3` v3.0.1 (Go) | Transitive, via mochi | MIT and Apache-2.0 (dual) | Allowed |
| `json` 0.12.4 (Rust) | Considered for profile parsing | MIT/Apache-2.0, but RUSTSEC-2022-0081 (unmaintained, soundness issue) | **Denied** (ADR 0011) |
| `serde_json` / `miniserde` (Rust) | Considered for profile parsing | MIT OR Apache-2.0, but depend on `ryu` (Apache-2.0 OR BSL-1.0) | **Denied** (ADR 0011) |
| `minimq` 0.13.5 (Rust) | Edge MQTT client (ADR 0006, 0013) | MIT OR Apache-2.0 | Allowed. Default features off (no defmt) |
| `embassy-time` 0.5 (Rust) | Timer queue for minimq | MIT OR Apache-2.0 | Allowed. `std` and `generic-queue-64` features (ADR 0013) |
| `embedded-io-adapters` 0.7 (Rust) | Adapts tokio TcpStream to minimq's transport traits | MIT OR Apache-2.0 | Allowed. Version 0.7 matches embedded-io-async 0.7 |
| `tokio` 1 (Rust) | Runtime for the edge node | MIT | Allowed. Features: rt, net, time, macros, io-util, sync |
| `getrandom` 0.3 (Rust) | Random source for message UUIDs | MIT OR Apache-2.0 | Allowed |
| Vendored Keystone Go SDK (`fleet/third_party/keystone-sdk-go`, commit `53e2017`) | Telemetry writes to Keystone | MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE in the copy). go-licenses reads it as Apache-2.0 alone, so the CI licence job excludes it by name, with the reason in `ci.yml` | Allowed (dual). Reviewed here |
| `mochi-mqtt/server` (Go) test broker `fleet/testbroker` | Integration tests and CI | MIT | Allowed |
| Keystone Go SDK (`tpt-keystone-db/sdk-go`) | Telemetry writes to Keystone (ADR 0009) | Standard library only. Root licence `LICENSE-MIT` and `LICENSE-APACHE` (GitHub reports NOASSERTION) | Allowed. Not yet imported |
| `eclipse/paho.mqtt.golang` (Go) | Considered, not used | EPL-2.0 / EDL-1.0 | **Excluded** by policy (ADR 0010) |

The Mosquitto broker is EPL-2.0 / EDL-1.0 (from memory, not verified here). Run as a separate service, not linked. Treat it the same as the Go client until checked.

Keystone Go client: `github.com/tpt-solutions/tpt-keystone-db/sdk-go` (see the row above). Checked; not yet imported. Its module path is `github.com/tpt/sdk-go`, which needs resolving before import (ADR 0009).

## Updating a pin

1. Pick the new commit on `master` of the upstream repo and confirm its tests pass there.
2. Change every `rev` in `edge/Cargo.toml` at once.
3. Run `cargo update -p tpt-thermo-core` (and the other `tpt-thermo-*` names), then `cargo check --workspace`.
4. Run `cargo deny check` from `edge/`.
5. Update the table above and record the new commit in the commit message.
