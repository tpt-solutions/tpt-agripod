# 0011: Edge profile parsing uses an in-house strict JSON parser

Status: Accepted
Date: 2026-11-11

## Context

The edge must validate Crop Profiles itself (docs/mqtt-topics.md: an unsafe or malformed profile
is rejected and the previous one stays active). That needs a JSON parser. Candidates checked on
crates.io, the RustSec advisory list, and cargo-deny:

| Crate | Licence | Result |
|---|---|---|
| `json` 0.12.4 | MIT/Apache-2.0 | **Denied.** Advisory RUSTSEC-2022-0081: unmaintained, with an open soundness issue. cargo-deny fails on it |
| `serde_json` 1.0.154 | MIT OR Apache-2.0 | **Denied.** Depends on `ryu`, which is Apache-2.0 OR BSL-1.0. BSL-1.0 is not on the allowlist |
| `miniserde` 0.1.46 | MIT OR Apache-2.0 | **Denied.** Depends on `ryu` for float output |
| `nanoserde` 0.2.1 | MIT OR Apache-2.0 | Possible. Ignores unknown fields, so the schema's strictness would be hand-rebuilt on top of it |

Adding BSL-1.0 to the allowlist would be a policy change, and the maintainer has not made it.

## Decision

Write a small RFC 8259 parser in `edge/crates/tpt-agripod-edge/src/json.rs`, with no
dependencies. It rejects duplicate keys, trailing data, control characters in strings, invalid
escapes and unpaired surrogates, non-finite numbers, and nesting deeper than 32. Its tests include
a seeded fuzz pass over random structural bytes. The profile size is capped at 64 KiB before
parsing.

## Consequences

- The parser is code we own and must audit. It is about 400 lines, and it has no dependencies to
  track.
- The profile validator, which is the part that matters for safety, is separate from the parser
  and has its own tests. Its rules mirror the Go validator's and the schema's. The mirroring is
  checked by hand, not by a shared test vector yet. A shared vector file is a follow-up.
- If the policy later allows BSL-1.0, `serde_json` could replace this. That would be a new ADR.
