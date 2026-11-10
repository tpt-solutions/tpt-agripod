// SPDX-License-Identifier: MIT OR Apache-2.0

//! QF_LRA constraint solver for actuator contracts (ADR 0003).
//!
//! Three layers:
//!
//! * [`rational`]: exact rationals on `i128`. Every operation is checked. Overflow is an error.
//! * [`dsl`] and [`formula`]: parse and evaluate linear boolean formulas, with explanations.
//! * [`contract`]: `requires` and `ensures` clauses, checked against the state and the
//!   post-command state. Any error denies the command.
//!
//! [`fm`] decides whether a conjunction of linear constraints is satisfiable, under a budget.
//!
//! `no_std` with `alloc`. There are no dependencies.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod contract;
pub mod dsl;
pub mod expr;
pub mod fm;
pub mod formula;
pub mod rational;

pub use contract::{Contract, Decision, Phase, check_command, parse_contract};
pub use expr::{Bindings, Env, EvalError};
pub use formula::Formula;
pub use rational::Rational;
