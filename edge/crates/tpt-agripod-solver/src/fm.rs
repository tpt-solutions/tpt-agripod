// SPDX-License-Identifier: MIT OR Apache-2.0

//! Feasibility of a conjunction of linear constraints over the rationals, by Fourier–Motzkin
//! elimination.
//!
//! This is the first decision procedure. It is exact, and small enough to audit line by line.
//! Its cost can grow doubly exponentially, so it runs under a constraint budget: when the
//! budget is exceeded the answer is `Unknown`, which callers must treat as "not proven safe".
//! The simplex procedure in ADR 0003 is the planned replacement for large systems. The two
//! must agree on every system in the differential tests.

use alloc::vec::Vec;

use crate::expr::LinExpr;
use crate::formula::{Atom, Cmp, Formula};
use crate::rational::{ArithError, Rational};

/// `Σ coeffs[i] · x_i  (<= or <)  bound`, over variables indexed `0..n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// Coefficient of each variable.
    pub coeffs: Vec<Rational>,
    /// Right-hand side.
    pub bound: Rational,
    /// `true` for strict `<`, `false` for `<=`.
    pub strict: bool,
}

/// A conjunction of constraints, with the variable names by index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    /// Variable names. Index `i` is the variable `vars[i]`.
    pub vars: Vec<alloc::string::String>,
    /// The constraints.
    pub constraints: Vec<Constraint>,
}

/// Why the procedure could not give a definite answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unknown {
    /// The constraint count exceeded the budget.
    Budget,
    /// Exact arithmetic overflowed.
    Arithmetic,
    /// The formula is outside what this procedure handles: disjunction, negation, or `!=`.
    Unsupported,
}

/// The answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feasibility {
    /// Some rational assignment satisfies every constraint.
    Sat,
    /// No rational assignment satisfies every constraint.
    Unsat,
    /// No answer. Not proven either way.
    Unknown(Unknown),
}

/// Builds a constraint system from a formula that is a conjunction of atoms.
pub fn conjunction(f: &Formula) -> Result<System, Unknown> {
    let mut atoms: Vec<&Atom> = Vec::new();
    collect_conjuncts(f, &mut atoms)?;
    let mut vars: Vec<alloc::string::String> = Vec::new();
    for a in &atoms {
        for e in [&a.lhs, &a.rhs] {
            for (name, _) in &e.terms {
                if !vars.contains(name) {
                    vars.push(name.clone());
                }
            }
        }
    }
    vars.sort();
    let mut constraints = Vec::new();
    for a in &atoms {
        // lhs - rhs  op  0
        let d = a
            .lhs
            .add(
                &a.rhs
                    .scale(Rational::from_int(-1))
                    .map_err(|_| Unknown::Arithmetic)?,
            )
            .map_err(|_| Unknown::Arithmetic)?;
        match a.op {
            Cmp::Le => constraints.push(to_constraint(&d, &vars, false, false)?),
            Cmp::Lt => constraints.push(to_constraint(&d, &vars, false, true)?),
            Cmp::Ge => constraints.push(to_constraint(&d, &vars, true, false)?),
            Cmp::Gt => constraints.push(to_constraint(&d, &vars, true, true)?),
            Cmp::Eq => {
                constraints.push(to_constraint(&d, &vars, false, false)?);
                constraints.push(to_constraint(&d, &vars, true, false)?);
            }
            Cmp::Ne => return Err(Unknown::Unsupported),
        }
    }
    Ok(System { vars, constraints })
}

fn collect_conjuncts<'a>(f: &'a Formula, out: &mut Vec<&'a Atom>) -> Result<(), Unknown> {
    match f {
        Formula::Atom(a) => out.push(a),
        Formula::And(cs) => {
            for c in cs {
                collect_conjuncts(c, out)?;
            }
        }
        Formula::Or(_) | Formula::Not(_) => return Err(Unknown::Unsupported),
    }
    Ok(())
}

/// Turns `d op 0` into `Σ c·x (<=|<) b`. `negate` flips the sign of `d` (for `>=`, `>`), and
/// `strict` selects `<` over `<=`.
fn to_constraint(
    d: &LinExpr,
    vars: &[alloc::string::String],
    negate: bool,
    strict: bool,
) -> Result<Constraint, Unknown> {
    let sign = Rational::from_int(if negate { -1 } else { 1 });
    let mut coeffs = alloc::vec![Rational::ZERO; vars.len()];
    for (name, c) in &d.terms {
        let idx = vars
            .iter()
            .position(|v| v == name)
            .ok_or(Unknown::Arithmetic)?;
        coeffs[idx] = c.checked_mul(sign).map_err(|_| Unknown::Arithmetic)?;
    }
    // sign·(Σc·x + k) (<|<=) 0   becomes   sign·Σc·x (<|<=) −sign·k
    let bound = d
        .constant
        .checked_mul(sign)
        .map_err(|_| Unknown::Arithmetic)?
        .negate();
    Ok(Constraint {
        coeffs,
        bound,
        strict,
    })
}

/// Decides feasibility of `sys`. `budget` caps the number of constraints held at once.
pub fn decide(sys: &System, budget: usize) -> Feasibility {
    let nvars = sys.vars.len();
    let mut cons = sys.constraints.clone();
    for v in 0..nvars {
        if cons.len() > budget {
            return Feasibility::Unknown(Unknown::Budget);
        }
        let mut keep = Vec::new();
        let mut upper = Vec::new(); // coefficient of v > 0: gives an upper bound on v
        let mut lower = Vec::new(); // coefficient of v < 0: gives a lower bound on v
        for c in cons {
            let a = c.coeffs[v];
            if a.is_zero() {
                keep.push(c);
            } else if a.num() > 0 {
                upper.push(c);
            } else {
                lower.push(c);
            }
        }
        for u in &upper {
            for l in &lower {
                match combine(u, l, v) {
                    Ok(c) => keep.push(c),
                    Err(_) => return Feasibility::Unknown(Unknown::Arithmetic),
                }
                if keep.len() > budget {
                    return Feasibility::Unknown(Unknown::Budget);
                }
            }
        }
        // Constraints that bound v only one way are dropped: v can be pushed far enough.
        cons = keep;
    }
    // Every variable is gone. Each constraint now reads 0 (<|<=) bound.
    for c in &cons {
        let contradiction = if c.strict {
            c.bound.num() <= 0
        } else {
            c.bound.num() < 0
        };
        if contradiction {
            return Feasibility::Unsat;
        }
    }
    Feasibility::Sat
}

/// Eliminates variable `v` from an upper-bound constraint `u` (coefficient a > 0) and a
/// lower-bound constraint `l` (coefficient c < 0). Multiplies `u` by `−c` and `l` by `a`,
/// then adds, so the `v` terms cancel. Both multipliers are positive, so strictness is kept.
fn combine(u: &Constraint, l: &Constraint, v: usize) -> Result<Constraint, Unknown> {
    let arith = |_: ArithError| Unknown::Arithmetic;
    let mu = l.coeffs[v].negate();
    let ml = u.coeffs[v];
    let coeffs = u
        .coeffs
        .iter()
        .zip(&l.coeffs)
        .map(|(a, b)| {
            mu.checked_mul(*a)
                .and_then(|x| ml.checked_mul(*b).and_then(|y| x.checked_add(y)))
                .map_err(arith)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bound = mu
        .checked_mul(u.bound)
        .and_then(|x| ml.checked_mul(l.bound).and_then(|y| x.checked_add(y)))
        .map_err(arith)?;
    Ok(Constraint {
        coeffs,
        bound,
        strict: u.strict || l.strict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec;

    fn r(n: i64) -> Rational {
        Rational::from_int(n)
    }

    fn con(coeffs: &[i64], bound: i64, strict: bool) -> Constraint {
        Constraint {
            coeffs: coeffs.iter().map(|c| r(*c)).collect(),
            bound: r(bound),
            strict,
        }
    }

    fn sys(n: usize, cons: Vec<Constraint>) -> System {
        System {
            vars: (0..n)
                .map(|i| String::from(["x", "y", "z", "w"][i]))
                .collect(),
            constraints: cons,
        }
    }

    #[test]
    fn single_variable_cases() {
        // x <= 1 and x >= 2
        let s = sys(1, vec![con(&[1], 1, false), con(&[-1], -2, false)]);
        assert_eq!(decide(&s, 100), Feasibility::Unsat);
        // 0 < x < 1
        let s = sys(1, vec![con(&[1], 1, true), con(&[-1], 0, true)]);
        assert_eq!(decide(&s, 100), Feasibility::Sat);
        // x < 1 and x > 1
        let s = sys(1, vec![con(&[1], 1, true), con(&[-1], -1, true)]);
        assert_eq!(decide(&s, 100), Feasibility::Unsat);
        // x <= 0.15 and x > 0.15: exact, so the boundary is handled without rounding
        let b = Rational::parse_decimal("0.15").unwrap();
        let s = System {
            vars: vec![String::from("x")],
            constraints: vec![
                Constraint {
                    coeffs: vec![r(1)],
                    bound: b,
                    strict: false,
                },
                Constraint {
                    coeffs: vec![r(-1)],
                    bound: b.negate(),
                    strict: true,
                },
            ],
        };
        assert_eq!(decide(&s, 100), Feasibility::Unsat);
    }

    #[test]
    fn two_variable_cases() {
        // x + y <= 1, x >= 0, y >= 0: feasible
        let s = sys(
            2,
            vec![
                con(&[1, 1], 1, false),
                con(&[-1, 0], 0, false),
                con(&[0, -1], 0, false),
            ],
        );
        assert_eq!(decide(&s, 100), Feasibility::Sat);
        // x + y <= 1, x >= 1, y >= 1: infeasible
        let s = sys(
            2,
            vec![
                con(&[1, 1], 1, false),
                con(&[-1, 0], -1, false),
                con(&[0, -1], -1, false),
            ],
        );
        assert_eq!(decide(&s, 100), Feasibility::Unsat);
    }

    #[test]
    fn budget_exhaustion_is_unknown_not_a_guess() {
        let s = sys(
            2,
            vec![
                con(&[1, 1], 1, false),
                con(&[-1, 0], 0, false),
                con(&[0, -1], 0, false),
            ],
        );
        assert_eq!(decide(&s, 1), Feasibility::Unknown(Unknown::Budget));
    }

    #[test]
    fn builds_systems_from_formulas() {
        use crate::dsl::parse_formula;
        let f = parse_formula("pump <= 0.15 AND pump >= 0.2").unwrap();
        let s = conjunction(&f).unwrap();
        assert_eq!(decide(&s, 100), Feasibility::Unsat);
        let f = parse_formula("pump <= 0.15 AND pump >= 0.1").unwrap();
        assert_eq!(decide(&conjunction(&f).unwrap(), 100), Feasibility::Sat);
        let f = parse_formula("a > 0 OR b > 0").unwrap();
        assert_eq!(conjunction(&f), Err(Unknown::Unsupported));
        let f = parse_formula("a != 0").unwrap();
        assert_eq!(conjunction(&f), Err(Unknown::Unsupported));
    }

    /// Differential test against brute force on a grid. The grid holds real points, so:
    /// if the procedure says Unsat, no grid point may satisfy the system. If a grid point
    /// satisfies the system, the procedure must not say Unsat. Sat without a grid point is
    /// allowed, since the rational solution may fall between grid points.
    #[test]
    fn agrees_with_grid_search_on_random_systems() {
        let mut state: u64 = 0xC0FF_EE00_1234_5678;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut unsat_seen = 0;
        let mut sat_with_witness = 0;
        for _ in 0..400 {
            let n = 1 + (next() % 3) as usize;
            let m = 1 + (next() % 5) as usize;
            let mut cons = Vec::new();
            for _ in 0..m {
                let coeffs: Vec<i64> = (0..n).map(|_| (next() % 7) as i64 - 3).collect();
                let bound = (next() % 13) as i64 - 6;
                let strict = next() % 2 == 0;
                cons.push(con(&coeffs, bound, strict));
            }
            let s = sys(n, cons.clone());
            let verdict = decide(&s, 100_000);

            // Grid with step 1/2 over [-7, 7]^n.
            let steps: Vec<i64> = (-14..=14).collect(); // numerator over 2
            let mut witness = false;
            let total = steps.len().pow(n as u32);
            for idx in 0..total {
                let mut rem = idx;
                let mut point = Vec::with_capacity(n);
                for _ in 0..n {
                    point.push(steps[rem % steps.len()]);
                    rem /= steps.len();
                }
                // Coefficients and bounds are integers. A grid coordinate k stands for k/2.
                // Multiplying both sides by 2 keeps everything integral:
                // Σ a·(k/2) ≤ b  is  Σ a·k ≤ 2b.
                let ok = cons.iter().all(|c| {
                    let lhs2: i64 = c
                        .coeffs
                        .iter()
                        .zip(&point)
                        .map(|(a, k)| a.num() as i64 * k)
                        .sum();
                    let bound2 = c.bound.num() as i64 * 2;
                    if c.strict {
                        lhs2 < bound2
                    } else {
                        lhs2 <= bound2
                    }
                });
                if ok {
                    witness = true;
                    break;
                }
            }
            match verdict {
                Feasibility::Unsat => {
                    assert!(
                        !witness,
                        "procedure said Unsat but a grid point satisfies it: {cons:?}"
                    );
                    unsat_seen += 1;
                }
                Feasibility::Sat => {
                    if witness {
                        sat_with_witness += 1;
                    }
                }
                Feasibility::Unknown(_) => panic!("budget too small for a 3-variable test"),
            }
        }
        // The generator should produce both answers, or the test proves little.
        assert!(unsat_seen > 20, "only {unsat_seen} Unsat cases");
        assert!(
            sat_with_witness > 20,
            "only {sat_with_witness} Sat cases with a witness"
        );
    }
}
