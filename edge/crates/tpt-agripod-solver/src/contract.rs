// SPDX-License-Identifier: MIT OR Apache-2.0

//! Actuator contracts: `requires` is checked against the sensed state, `ensures` against the
//! state after the command is applied. Any error, including an unknown variable or an
//! overflow, denies the command. Denial is the safe default.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::dsl::parse_formula;
use crate::expr::{Bindings, EvalError};
use crate::formula::Formula;

/// A parsed contract: the conditions before and after an actuator command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    /// Must hold on the current state for the command to be allowed.
    pub requires: Formula,
    /// Must hold on the state after the command is applied.
    pub ensures: Formula,
}

/// A contract text that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    /// 1-based line in the contract text.
    pub line: usize,
    /// Byte offset within that line.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

/// Parses contract text. Each non-blank line that does not start with `#` must begin with
/// `requires:` or `ensures:`. Several lines of the same kind are combined with AND.
pub fn parse_contract(text: &str) -> Result<Contract, ContractError> {
    let mut requires: Vec<Formula> = Vec::new();
    let mut ensures: Vec<Formula> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = idx + 1;
        let t = raw.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let leading = raw.len() - t.len();
        let (target, prefix) = if t.starts_with("requires:") {
            (&mut requires, "requires:")
        } else if t.starts_with("ensures:") {
            (&mut ensures, "ensures:")
        } else {
            return Err(ContractError {
                line,
                offset: leading,
                message: "line must start with 'requires:' or 'ensures:'".to_string(),
            });
        };
        let body = &t[prefix.len()..];
        let f = parse_formula(body).map_err(|e| ContractError {
            line,
            offset: leading + prefix.len() + e.offset,
            message: e.message,
        })?;
        target.push(f);
    }
    Ok(Contract {
        requires: conjoin(requires),
        ensures: conjoin(ensures),
    })
}

fn conjoin(mut parts: Vec<Formula>) -> Formula {
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        Formula::And(parts)
    }
}

/// Which part of the contract a denial came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The `requires` clause failed, or could not be evaluated, on the current state.
    Requires,
    /// The `ensures` clause failed, or could not be evaluated, after the command.
    Ensures,
}

/// The outcome of checking one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Both clauses hold. The command may be executed.
    Allow,
    /// The command must not be executed. Reasons are human-readable, one per failed atom,
    /// or a description of the evaluation error.
    Deny {
        /// Which clause denied it.
        phase: Phase,
        /// Why.
        reasons: Vec<String>,
    },
}

/// Checks `command` against `contract` in the given `state`.
pub fn check_command(contract: &Contract, state: &Bindings, command: &Bindings) -> Decision {
    match contract.requires.eval(state) {
        Ok(true) => {}
        Ok(false) => {
            return Decision::Deny {
                phase: Phase::Requires,
                reasons: explain(&contract.requires, state),
            };
        }
        Err(e) => {
            return Decision::Deny {
                phase: Phase::Requires,
                reasons: alloc::vec![describe(&e)],
            };
        }
    }

    let post = state.overlay(command);
    match contract.ensures.eval(&post) {
        Ok(true) => Decision::Allow,
        Ok(false) => Decision::Deny {
            phase: Phase::Ensures,
            reasons: explain(&contract.ensures, &post),
        },
        Err(e) => Decision::Deny {
            phase: Phase::Ensures,
            reasons: alloc::vec![describe(&e)],
        },
    }
}

fn explain(f: &Formula, env: &Bindings) -> Vec<String> {
    match f.violations(env) {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => alloc::vec![String::from("condition is false")],
        Err(e) => alloc::vec![describe(&e)],
    }
}

/// Human-readable form of an evaluation error.
pub fn describe(e: &EvalError) -> String {
    match e {
        EvalError::UnknownVariable(name) => format!("unknown variable '{name}'"),
        EvalError::Arithmetic(a) => format!("arithmetic failed: {a:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rational::Rational;

    const SPEC: &str = "\
# Example from spec section 4.2.
requires: reservoir_level > 50 AND grid_power_headroom > 2000
ensures: pump_duty_cycle <= 0.15 AND led_dimming >= 0.8
";

    fn bind(pairs: &[(&str, &str)]) -> Bindings {
        let mut b = Bindings::new();
        for (n, v) in pairs {
            b.set(n, Rational::parse_decimal(v).unwrap());
        }
        b
    }

    fn healthy() -> Bindings {
        bind(&[
            ("reservoir_level", "60"),
            ("grid_power_headroom", "2500"),
            ("pump_duty_cycle", "0"),
            ("led_dimming", "1"),
        ])
    }

    #[test]
    fn allows_a_safe_command() {
        let c = parse_contract(SPEC).unwrap();
        let cmd = bind(&[("pump_duty_cycle", "0.1")]);
        assert_eq!(check_command(&c, &healthy(), &cmd), Decision::Allow);
    }

    #[test]
    fn denies_when_the_requires_clause_fails_and_explains_why() {
        let c = parse_contract(SPEC).unwrap();
        let state = bind(&[
            ("reservoir_level", "40"),
            ("grid_power_headroom", "2500"),
            ("pump_duty_cycle", "0"),
            ("led_dimming", "1"),
        ]);
        let cmd = bind(&[("pump_duty_cycle", "0.1")]);
        match check_command(&c, &state, &cmd) {
            Decision::Deny { phase, reasons } => {
                assert_eq!(phase, Phase::Requires);
                assert_eq!(reasons.len(), 1);
                assert!(reasons[0].contains("reservoir_level > 50"), "{reasons:?}");
                assert!(reasons[0].contains("left 40.0000"), "{reasons:?}");
            }
            other => panic!("expected denial, got {other:?}"),
        }
    }

    #[test]
    fn denies_a_pump_command_above_the_duty_limit() {
        // The flood-prevention case: a pump commanded past its limit is refused before it runs.
        let c = parse_contract(SPEC).unwrap();
        let cmd = bind(&[("pump_duty_cycle", "0.2")]);
        match check_command(&c, &healthy(), &cmd) {
            Decision::Deny { phase, reasons } => {
                assert_eq!(phase, Phase::Ensures);
                assert_eq!(
                    reasons,
                    alloc::vec![String::from(
                        "pump_duty_cycle <= 0.15 (left 0.2000, right 0.1500)"
                    )]
                );
            }
            other => panic!("expected denial, got {other:?}"),
        }
    }

    #[test]
    fn missing_sensor_denies_closed() {
        let c = parse_contract(SPEC).unwrap();
        let state = bind(&[("grid_power_headroom", "2500")]);
        let cmd = bind(&[("pump_duty_cycle", "0.1")]);
        match check_command(&c, &state, &cmd) {
            Decision::Deny { phase, reasons } => {
                assert_eq!(phase, Phase::Requires);
                assert!(reasons[0].contains("unknown variable 'reservoir_level'"));
            }
            other => panic!("expected denial, got {other:?}"),
        }
    }

    #[test]
    fn command_cannot_be_hidden_by_a_missing_led_value() {
        // ensures reads led_dimming, which is absent from both state and command.
        let c = parse_contract(SPEC).unwrap();
        let state = bind(&[("reservoir_level", "60"), ("grid_power_headroom", "2500")]);
        let cmd = bind(&[("pump_duty_cycle", "0.1")]);
        assert!(matches!(
            check_command(&c, &state, &cmd),
            Decision::Deny {
                phase: Phase::Ensures,
                ..
            }
        ));
    }

    #[test]
    fn parse_errors_report_line_and_offset() {
        let err = parse_contract("requires: x > 1\nensures: y >> 2\n").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(err.offset >= "ensures: ".len());
        let err = parse_contract("when: x > 1").unwrap_err();
        assert_eq!(err.line, 1);
        assert!(err.message.contains("requires:"));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let c = parse_contract("\n# note\n\nrequires: x > 0\n").unwrap();
        assert_eq!(c.ensures, Formula::And(alloc::vec![]));
    }
}
