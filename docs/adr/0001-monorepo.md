# 0001: Single monorepo with per-language workspaces

Status: Accepted
Date: 2026-11-11

## Context

Agripod spans a Rust edge workspace, Go fleet services, ESP32 firmware, profile schemas and hardware notes. A solo maintainer needs one place for review, CI and versioning, and the edge and fleet must agree on schemas.

## Decision

One git repository with these top-level areas:

- `edge/`: Cargo workspace (`tpt-agripod-core`, `-sim`, `-solver`, `-actuators`, `-edge`, `-cli`)
- `fleet/`: Go modules (`tpt-agripod-orchestrator`, `tpt-agripod-telemetry`)
- `esp32/`: standalone `no_std` crate, excluded from the edge workspace because it needs a different target and toolchain
- `profiles/`, `docs/`, `hardware/`

## Consequences

- One CI pipeline covers all languages, and schema changes land atomically with their consumers.
- The ESP32 crate builds separately and needs its own CI job.
- Each language keeps its native tooling (Cargo, Go modules), so there is no build-system layer to maintain.
