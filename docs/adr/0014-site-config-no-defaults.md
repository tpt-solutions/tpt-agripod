# 0014: Site-specific values come from a site file, with no defaults

Status: Accepted
Date: 2026-11-11

## Context

Some values depend on the installation, not on the software: the valve open-time limit (flood
study), the LED headroom threshold (supply rating), the irrigation thresholds and pulse timing
(calibration), the drain limit, and the electricity tariff. The code had defaults for these, which
looked like real values and were not.

## Decision

- The edge reads these from a site file (`SiteConfig`). It has no defaults for them. Every field
  must be present and valid, unknown fields are refused, and the file must say `"status": "production"`.
  A file marked `"example"` is refused, so the example in `edge/config/site.example.json` cannot run.
- The solenoid and LED contracts are templates. The edge renders them from the site file, and an
  unrendered placeholder fails the contract parser, so a missing value cannot slip through.
- The simulator's electricity tariff is a required input. The CLI requires `--tariff-peak`,
  `--tariff-offpeak`, `--offpeak-start` and `--offpeak-end`, and the simulation refuses to run without one.
- The orchestrator refuses a stage template marked `"_status": "example"`.
- Spec-derived values stay fixed in the contracts: the pump duty limit 0.15, the reservoir above 50 L,
  and the grid headroom above 2000 W for the pump.

## Consequences

- The values still need to be supplied. Nothing runs on a made-up number, but the operator must
  fill in the flood study, supply rating, calibration and tariff before the node will start.
- Tests use a fixture file that is clearly labelled as a test fixture, not a default.
- The example files remain as templates and are refused by the software.
