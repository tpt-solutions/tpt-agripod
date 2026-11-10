// SPDX-License-Identifier: MIT OR Apache-2.0

//! Linear expressions over named variables, and the bindings that give them values.

use alloc::string::String;
use alloc::vec::Vec;

use crate::rational::{ArithError, Rational};

/// Why a formula could not be evaluated. Every variant means "do not act".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// The formula names a variable with no value in the state or command.
    UnknownVariable(String),
    /// Exact arithmetic overflowed or divided by zero.
    Arithmetic(ArithError),
}

impl From<ArithError> for EvalError {
    fn from(e: ArithError) -> Self {
        EvalError::Arithmetic(e)
    }
}

/// Source of variable values.
pub trait Env {
    /// The value of `name`, or `None` if it is not bound.
    fn value(&self, name: &str) -> Option<Rational>;
}

/// A set of named values. Used for the sensed state and for a proposed command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    entries: Vec<(String, Rational)>,
}

impl Bindings {
    /// An empty set of bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `name` to `value`, replacing any earlier value.
    pub fn set(&mut self, name: &str, value: Rational) {
        match self.entries.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((String::from(name), value)),
        }
    }

    /// A copy of `self` with every binding in `other` applied on top.
    pub fn overlay(&self, other: &Bindings) -> Bindings {
        let mut out = self.clone();
        for (name, value) in &other.entries {
            out.set(name, *value);
        }
        out
    }
}

impl Env for Bindings {
    fn value(&self, name: &str) -> Option<Rational> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| *v)
    }
}

/// `constant + Σ coeff · var`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinExpr {
    /// Coefficient of each variable. Names are unique and sorted.
    pub terms: Vec<(String, Rational)>,
    /// The constant part.
    pub constant: Rational,
}

impl LinExpr {
    /// A constant expression.
    pub fn constant(c: Rational) -> Self {
        Self {
            terms: Vec::new(),
            constant: c,
        }
    }

    /// The expression `1 · name`.
    pub fn var(name: &str) -> Self {
        Self {
            terms: alloc::vec![(String::from(name), Rational::ONE)],
            constant: Rational::ZERO,
        }
    }

    /// True when the expression has no variables.
    pub fn is_constant(&self) -> bool {
        self.terms.is_empty()
    }

    /// Sum of two expressions.
    pub fn add(&self, o: &LinExpr) -> Result<LinExpr, ArithError> {
        let mut terms = self.terms.clone();
        for (name, coeff) in &o.terms {
            match terms.iter_mut().find(|(n, _)| n == name) {
                Some(t) => t.1 = t.1.checked_add(*coeff)?,
                None => terms.push((name.clone(), *coeff)),
            }
        }
        terms.retain(|(_, c)| !c.is_zero());
        terms.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(LinExpr {
            terms,
            constant: self.constant.checked_add(o.constant)?,
        })
    }

    /// Expression multiplied by a constant.
    pub fn scale(&self, k: Rational) -> Result<LinExpr, ArithError> {
        let mut terms = Vec::with_capacity(self.terms.len());
        for (name, coeff) in &self.terms {
            terms.push((name.clone(), coeff.checked_mul(k)?));
        }
        terms.retain(|(_, c)| !c.is_zero());
        Ok(LinExpr {
            terms,
            constant: self.constant.checked_mul(k)?,
        })
    }

    /// Value of the expression under `env`.
    pub fn eval(&self, env: &impl Env) -> Result<Rational, EvalError> {
        let mut acc = self.constant;
        for (name, coeff) in &self.terms {
            let v = env
                .value(name)
                .ok_or_else(|| EvalError::UnknownVariable(name.clone()))?;
            acc = acc.checked_add(coeff.checked_mul(v)?)?;
        }
        Ok(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_linear_expressions() {
        let mut env = Bindings::new();
        env.set("a", Rational::from_int(2));
        env.set("b", Rational::parse_decimal("0.5").unwrap());
        // 3·a − b + 1 = 3·2 − 0.5 + 1 = 6.5
        let e = LinExpr::var("a")
            .scale(Rational::from_int(3))
            .unwrap()
            .add(&LinExpr::var("b").scale(Rational::from_int(-1)).unwrap())
            .unwrap()
            .add(&LinExpr::constant(Rational::ONE))
            .unwrap();
        assert_eq!(
            e.eval(&env).unwrap(),
            Rational::parse_decimal("6.5").unwrap()
        );
    }

    #[test]
    fn unknown_variable_is_an_error() {
        let env = Bindings::new();
        assert_eq!(
            LinExpr::var("missing").eval(&env),
            Err(EvalError::UnknownVariable(String::from("missing")))
        );
    }

    #[test]
    fn add_merges_and_drops_cancelled_terms() {
        let e = LinExpr::var("x")
            .add(&LinExpr::var("x").scale(Rational::from_int(-1)).unwrap())
            .unwrap();
        assert!(e.is_constant());
    }

    #[test]
    fn overlay_replaces_values() {
        let mut state = Bindings::new();
        state.set("duty", Rational::from_int(0));
        state.set("level", Rational::from_int(60));
        let mut cmd = Bindings::new();
        cmd.set("duty", Rational::parse_decimal("0.2").unwrap());
        let post = state.overlay(&cmd);
        assert_eq!(
            post.value("duty"),
            Some(Rational::parse_decimal("0.2").unwrap())
        );
        assert_eq!(post.value("level"), Some(Rational::from_int(60)));
        assert_eq!(state.value("duty"), Some(Rational::ZERO));
    }
}
