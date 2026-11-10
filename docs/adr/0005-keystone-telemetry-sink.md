# 0005: TPT Keystone as the telemetry sink

Status: Accepted
Date: 2026-11-11

## Context

Sensor series feed back into refining the physics models. TPT Keystone DB is the team's time-series platform. It speaks the Postgres wire protocol and offers a `COPY` bulk path and `USING TIME` tables (Chronos). Keystone is self-described as not production-hardened.

## Decision

`tpt-agripod-telemetry` writes to Keystone through a pluggable sink interface. Keystone is one implementation. A stdout and file sink exists for tests and for running without Keystone. Writes are batched, use `COPY`, and are idempotent across reconnects.

## Consequences

- Telemetry keeps working with other databases, or none, which limits the blast radius of a Keystone problem.
- The Keystone Go client's licence and API stability must be checked before Phase 3 (open question in `todo.md`).
