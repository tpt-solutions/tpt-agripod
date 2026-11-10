# Contributing

Contributions are welcome as issues only. Open an issue to report a bug, propose a change, or ask a question. Pull requests are not accepted at this stage.

## What to include in an issue

- What you tried or what you think should change, and why.
- For a bug: steps to reproduce, the expected result, the actual result, and the commit or version.
- For a safety concern (anything that could cause a flood, fire, electrical fault or unsafe state, including in the solver, actuator HAL or ESP32 guard): say so in the title, so it is handled first.

## Standing rules

These apply to any change proposed in an issue:

1. **Licence.** New code is `MIT OR Apache-2.0`. Rust `Cargo.toml` files set `license = "MIT OR Apache-2.0"`. Source files carry the SPDX header:
   - Rust and Go: `// SPDX-License-Identifier: MIT OR Apache-2.0`
   - Python, shell, YAML and TOML: `# SPDX-License-Identifier: MIT OR Apache-2.0`
2. **Dependencies.** Only MIT, BSD-2/3, ISC, Zlib, Unlicense or MIT/Apache dual licences. No Apache-only, GPL/LGPL/AGPL, MPL or EPL. No gRPC, protobuf or NATS.
3. **Determinism.** Edge simulation code takes no wall-clock input and uses fixed seeds in tests.
4. **Safety.** Safety logic never depends on the network.
5. **Units.** SI throughout, with the conventions in [docs/units.md](docs/units.md).

Significant design decisions are recorded as ADRs in [docs/adr/](docs/adr/). Propose a new one in an issue.

## Licence of contributions

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 licence, is dual licensed as stated in [README.md](README.md), without any additional terms or conditions.
