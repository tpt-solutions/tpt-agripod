// SPDX-License-Identifier: MIT OR Apache-2.0

//! Boolean formulas over linear atoms, their evaluation, and explanations of failures.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::expr::{Env, EvalError, LinExpr};
use crate::rational::Rational;

/// Comparison operator in an atom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    /// `<=`
    Le,
    /// `<`
    Lt,
    /// `>=`
    Ge,
    /// `>`
    Gt,
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

impl Cmp {
    /// The source spelling of the operator.
    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Le => "<=",
            Cmp::Lt => "<",
            Cmp::Ge => ">=",
            Cmp::Gt => ">",
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
        }
    }
}

/// `lhs op rhs`, with the source text kept for explanations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// Left-hand side.
    pub lhs: LinExpr,
    /// Operator.
    pub op: Cmp,
    /// Right-hand side.
    pub rhs: LinExpr,
    /// The atom as written in the contract, e.g. `pump_duty_cycle <= 0.15`.
    pub text: String,
}

impl Atom {
    /// Truth value of the atom under `env`.
    pub fn eval(&self, env: &impl Env) -> Result<bool, EvalError> {
        let l = self.lhs.eval(env)?;
        let r = self.rhs.eval(env)?;
        let ord = l.checked_cmp(r)?;
        use core::cmp::Ordering::{Equal, Greater, Less};
        Ok(match self.op {
            Cmp::Le => ord != Greater,
            Cmp::Lt => ord == Less,
            Cmp::Ge => ord != Less,
            Cmp::Gt => ord == Greater,
            Cmp::Eq => ord == Equal,
            Cmp::Ne => ord != Equal,
        })
    }
}

/// A boolean combination of atoms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Formula {
    /// A linear comparison.
    Atom(Atom),
    /// True when every child is true. An empty `And` is true.
    And(Vec<Formula>),
    /// True when some child is true. An empty `Or` is false.
    Or(Vec<Formula>),
    /// Logical negation.
    Not(Box<Formula>),
}

impl Formula {
    /// Truth value under `env`.
    ///
    /// Every child is evaluated, even after the answer is known. An error in any part of the
    /// formula is returned, so a bad variable cannot be hidden by a short-circuit.
    pub fn eval(&self, env: &impl Env) -> Result<bool, EvalError> {
        match self {
            Formula::Atom(a) => a.eval(env),
            Formula::And(children) => {
                let mut all = true;
                for c in children {
                    all &= c.eval(env)?;
                }
                Ok(all)
            }
            Formula::Or(children) => {
                let mut any = false;
                for c in children {
                    any |= c.eval(env)?;
                }
                Ok(any)
            }
            Formula::Not(inner) => Ok(!inner.eval(env)?),
        }
    }

    /// Explanations for why a false formula is false: one line per false atom, with the
    /// values it compared. Empty if the formula is true.
    pub fn violations(&self, env: &impl Env) -> Result<Vec<String>, EvalError> {
        let mut out = Vec::new();
        self.collect_violations(env, &mut out)?;
        Ok(out)
    }

    fn collect_violations(&self, env: &impl Env, out: &mut Vec<String>) -> Result<(), EvalError> {
        if self.eval(env)? {
            return Ok(());
        }
        match self {
            Formula::Atom(a) => {
                let l = a.lhs.eval(env)?;
                let r = a.rhs.eval(env)?;
                out.push(format!(
                    "{} (left {}, right {})",
                    a.text,
                    fmt_value(l),
                    fmt_value(r)
                ));
            }
            Formula::And(children) => {
                for c in children {
                    c.collect_violations(env, out)?;
                }
            }
            Formula::Or(children) => {
                for c in children {
                    c.collect_violations(env, out)?;
                }
            }
            Formula::Not(_) => out.push(String::from("a negated condition holds")),
        }
        Ok(())
    }

    /// Every atom in the formula, in source order.
    pub fn atoms(&self) -> Vec<&Atom> {
        let mut out = Vec::new();
        self.push_atoms(&mut out);
        out
    }

    fn push_atoms<'a>(&'a self, out: &mut Vec<&'a Atom>) {
        match self {
            Formula::Atom(a) => out.push(a),
            Formula::And(cs) | Formula::Or(cs) => cs.iter().for_each(|c| c.push_atoms(out)),
            Formula::Not(c) => c.push_atoms(out),
        }
    }
}

/// Display form of a value. Four decimals, for humans reading an explanation.
fn fmt_value(v: Rational) -> String {
    format!("{:.4}", v.to_f64())
}
