# TPT Agripod

Software-defined, physics-informed, modular controlled environment agriculture (CEA).

Agripod treats a shipping-container-scale growing bay as a configurable runtime.
Crop requirements are expressed as data (Crop Profiles) and enforced by a
formal constraint solver on a deterministic, offline-capable edge node. The
physical chassis stays the same; grow cartridges and profiles change.

The design is in [spec.txt](spec.txt). The phased build plan is in [todo.md](todo.md).

## Layout

| Path | Contents |
|---|---|
| `edge/` | Rust Cargo workspace: physics, constraint solver, actuators, edge daemon, CLI |
| `fleet/` | Go modules: orchestrator and telemetry services |
| `esp32/` | `no_std` leaf-node firmware (`esp-hal`) |
| `profiles/` | Crop Profile JSON Schema and example profiles |
| `docs/` | Architecture decision records, units, MQTT topic map, dev setup |
| `hardware/` | Physical build notes, wiring and BOM |

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Every dependency must be licensed under MIT, BSD, ISC, Zlib, Unlicense, or a
MIT/Apache-2.0 dual licence. Apache-only, GPL/LGPL/AGPL, MPL and EPL dependencies
are not accepted. `edge/deny.toml` enforces this for Rust, and CI enforces it for Go.

## Contributing

Contributions are by issue only. See [CONTRIBUTING.md](CONTRIBUTING.md). Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 licence, is dual licensed as above, without any additional terms or conditions.
