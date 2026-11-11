# 0012: Raw sensor retention is 90 days

Status: Accepted
Date: 2026-11-11

## Context

The telemetry schema needs a retention period on its Chronos index. It was a placeholder. Raw
points are the only copy until rollups exist (rollups are Phase 5 work).

## Decision

Raw points are kept for 90 days (`telemetry.DefaultRetention`). `-init-schema` takes `-retention` if
an operator wants something else. The value is written into the Chronos index DDL.

## Consequences

- Ninety days of raw data covers a full season of diagnosis at the 5-minute sample interval.
- Longer history needs rollups, which are not built. Retention must not be raised without them
  because storage grows linearly with channels and sample rate.
- Keystone enforces retention. This repo cannot verify that until the live test runs against a
  server (`keystone_live_test.go`).
