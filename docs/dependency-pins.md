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
| `tpt-eng-props-water` | crates.io | `0.1` (0.1.1 at time of writing) | IAPWS-IF97 Regions 1, 2 and 4. Region 3 unsupported. `no_std`. |
| `tpt-thermo-core` | git `tpt-solutions/tpt-thermodynamics` | `67c773ac38b75d6198e8152bc62a8b40102353ec` (master, 2026-09-04) | Only `no_std` crate in that repo. |
| `tpt-thermo-electrolyte` | same | same rev | Pitzer, eNRTL, HKF. No EC or pH compensation (open issue in `todo.md`). |
| `tpt-thermo-transport` | same | same rev | Viscosity, thermal conductivity, diffusivity. |

The `tpt-thermo-*` pins are declared under `[workspace.dependencies]` in `edge/Cargo.toml`. A member crate opts in with `tpt-thermo-core = { workspace = true }`. No member uses them yet, so they are not in `Cargo.lock`.

## Verification of the current pins

- Compiled in a scratch crate outside the repo: all five resolve and `cargo check` passes.
- The `tpt-thermo-*` licences are `MIT OR Apache-2.0`, inherited from the workspace.
- `cargo deny` passes the transitive licences. `Unicode-3.0` (needed by `unicode-ident` via `serde_derive`) is on the allow-list by maintainer decision.

## Updating a pin

1. Pick the new commit on `master` of the upstream repo and confirm its tests pass there.
2. Change every `rev` in `edge/Cargo.toml` at once.
3. Run `cargo update -p tpt-thermo-core` (and the other `tpt-thermo-*` names), then `cargo check --workspace`.
4. Run `cargo deny check` from `edge/`.
5. Update the table above and record the new commit in the commit message.
