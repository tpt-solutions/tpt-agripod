# 0008: Fourier–Motzkin as the first decision procedure; simplex to follow

Status: Accepted
Date: 2026-11-11

## Context

ADR 0003 names a simplex-based QF_LRA decision procedure. Phase 2 needs a checker that is
correct before it is fast. Runtime contract checks in the edge loop do not need a decision
procedure at all: they evaluate concrete sensor values with exact arithmetic (`tpt-agripod-solver`
`formula` and `contract` modules). A decision procedure is needed for offline consistency
checks (is an `ensures` clause satisfiable at all?) and for any future symbolic check.

No reference SMT solver is installed in the build environment, so differential testing is
against brute-force grid search.

## Decision

Implement Fourier–Motzkin elimination over exact rationals (`fm` module) as the first
decision procedure, under an explicit constraint budget. An exceeded budget or an arithmetic
overflow returns `Unknown`, never a guess. Treat `Unknown` as "not proven safe".

Simplex remains the planned replacement for large systems. It must agree with Fourier–Motzkin
on every system in the differential tests before it replaces it.

## Consequences

- The procedure is small enough to audit line by line. That matters more than speed at this
  scale: contracts have a handful of variables.
- Fourier–Motzkin can blow up doubly exponentially. The budget bounds the cost, and the
  budget is the worst-case time bound until simplex lands. Measured timings are not yet
  recorded.
- Disjunction, negation and `!=` are not handled by the decision procedure. Callers get
  `Unknown(Unsupported)`. Full QF_LRA needs a DPLL(T) layer, which is follow-up work under a
  new ADR.
- Differential tests cover conjunctions of linear atoms only, over small integer systems.
