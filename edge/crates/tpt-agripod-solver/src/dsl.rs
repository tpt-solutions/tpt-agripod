// SPDX-License-Identifier: MIT OR Apache-2.0

//! Parser for contract formulas.
//!
//! Grammar (lowest precedence first):
//!
//! ```text
//! formula  := and ( "OR" and )*
//! and      := unary ( "AND" unary )*
//! unary    := "NOT" unary | group | atom
//! group    := "(" formula ")"          (only when not followed by a comparison)
//! atom     := expr cmp expr
//! cmp      := "<=" | "<" | ">=" | ">" | "==" | "!="
//! expr     := term ( ("+" | "-") term )*
//! term     := factor ( ("*" | "/") factor )*   (at least one side of "*" is constant)
//! factor   := number | identifier | "-" factor | "(" expr ")"
//! ```
//!
//! Keywords are upper case. Numbers are plain decimals. A unit suffix such as `50L` is
//! rejected: put the unit in the variable name (`reservoir_level_l > 50`), so a unit
//! mistake cannot pass silently.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::expr::LinExpr;
use crate::formula::{Atom, Cmp, Formula};
use crate::rational::Rational;

/// A parse failure, with a byte offset into the text that was parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset of the problem within the parsed text.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ident,
    Number,
    And,
    Or,
    Not,
    Cmp(Cmp),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

#[derive(Debug, Clone, Copy)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

fn tokenize(src: &str) -> Result<Vec<Token>, ParseError> {
    let bytes = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let two = bytes.get(i..i + 2);
        let (kind, len) = match (two, c) {
            (Some(b"<="), _) => (Kind::Cmp(Cmp::Le), 2),
            (Some(b">="), _) => (Kind::Cmp(Cmp::Ge), 2),
            (Some(b"=="), _) => (Kind::Cmp(Cmp::Eq), 2),
            (Some(b"!="), _) => (Kind::Cmp(Cmp::Ne), 2),
            (_, b'<') => (Kind::Cmp(Cmp::Lt), 1),
            (_, b'>') => (Kind::Cmp(Cmp::Gt), 1),
            (_, b'+') => (Kind::Plus, 1),
            (_, b'-') => (Kind::Minus, 1),
            (_, b'*') => (Kind::Star, 1),
            (_, b'/') => (Kind::Slash, 1),
            (_, b'(') => (Kind::LParen, 1),
            (_, b')') => (Kind::RParen, 1),
            (_, b'0'..=b'9') | (_, b'.') => {
                let mut j = i;
                while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                    return Err(ParseError {
                        offset: j,
                        message: "unit suffix on a number is not allowed; put the unit in the variable name"
                            .to_string(),
                    });
                }
                (Kind::Number, j - i)
            }
            (_, b'a'..=b'z') | (_, b'A'..=b'Z') | (_, b'_') => {
                let mut j = i;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let word = &src[i..j];
                let kind = match word {
                    "AND" => Kind::And,
                    "OR" => Kind::Or,
                    "NOT" => Kind::Not,
                    _ => Kind::Ident,
                };
                (kind, j - i)
            }
            _ => {
                return Err(ParseError {
                    offset: i,
                    message: "unexpected character".to_string(),
                });
            }
        };
        toks.push(Token {
            kind,
            start,
            end: start + len,
        });
        i += len;
    }
    Ok(toks)
}

/// Parses one formula. The whole text must be consumed.
pub fn parse_formula(src: &str) -> Result<Formula, ParseError> {
    let toks = tokenize(src)?;
    let mut p = Parser { src, toks, pos: 0 };
    if p.toks.is_empty() {
        return Err(ParseError {
            offset: 0,
            message: "empty formula".to_string(),
        });
    }
    let f = p.parse_or()?;
    if let Some(t) = p.peek() {
        return Err(ParseError {
            offset: t.start,
            message: "unexpected input after the formula".to_string(),
        });
    }
    Ok(f)
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.toks.get(self.pos).copied()
    }

    fn peek_kind(&self) -> Option<Kind> {
        self.peek().map(|t| t.kind)
    }

    fn here(&self) -> usize {
        self.peek().map(|t| t.start).unwrap_or(self.src.len())
    }

    fn eat(&mut self, kind: Kind) -> bool {
        if self.peek_kind() == Some(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: Kind, what: &str) -> Result<Token, ParseError> {
        match self.peek() {
            Some(t) if t.kind == kind => {
                self.pos += 1;
                Ok(t)
            }
            _ => Err(ParseError {
                offset: self.here(),
                message: format_expected(what),
            }),
        }
    }

    fn parse_or(&mut self) -> Result<Formula, ParseError> {
        let mut items = alloc::vec![self.parse_and()?];
        while self.eat(Kind::Or) {
            items.push(self.parse_and()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Formula::Or(items)
        })
    }

    fn parse_and(&mut self) -> Result<Formula, ParseError> {
        let mut items = alloc::vec![self.parse_unary()?];
        while self.eat(Kind::And) {
            items.push(self.parse_unary()?);
        }
        Ok(if items.len() == 1 {
            items.remove(0)
        } else {
            Formula::And(items)
        })
    }

    fn parse_unary(&mut self) -> Result<Formula, ParseError> {
        if self.eat(Kind::Not) {
            return Ok(Formula::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Formula, ParseError> {
        if self.peek_kind() == Some(Kind::LParen) {
            // Try a parenthesised formula. If it is really the left side of a comparison,
            // such as `(a + b) <= c`, undo and parse it as an atom.
            let save = self.pos;
            self.pos += 1;
            if let Ok(inner) = self.parse_or() {
                if self.eat(Kind::RParen) && !matches!(self.peek_kind(), Some(Kind::Cmp(_))) {
                    return Ok(inner);
                }
            }
            self.pos = save;
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Formula, ParseError> {
        let first = self.pos;
        let lhs = self.parse_expr()?;
        let op = match self.peek_kind() {
            Some(Kind::Cmp(op)) => {
                self.pos += 1;
                op
            }
            _ => {
                return Err(ParseError {
                    offset: self.here(),
                    message: "expected a comparison (<=, <, >=, >, ==, !=)".to_string(),
                });
            }
        };
        let rhs = self.parse_expr()?;
        let last = self.pos - 1;
        let text = self.src[self.toks[first].start..self.toks[last].end].to_string();
        Ok(Formula::Atom(Atom { lhs, op, rhs, text }))
    }

    fn parse_expr(&mut self) -> Result<LinExpr, ParseError> {
        let mut acc = self.parse_term()?;
        loop {
            let negate = match self.peek_kind() {
                Some(Kind::Plus) => false,
                Some(Kind::Minus) => true,
                _ => return Ok(acc),
            };
            let at = self.here();
            self.pos += 1;
            let t = self.parse_term()?;
            let t = if negate {
                t.scale(Rational::from_int(-1)).map_err(|_| arith_err(at))?
            } else {
                t
            };
            acc = acc.add(&t).map_err(|_| arith_err(at))?;
        }
    }

    fn parse_term(&mut self) -> Result<LinExpr, ParseError> {
        let mut acc = self.parse_factor()?;
        loop {
            match self.peek_kind() {
                Some(Kind::Star) => {
                    let at = self.here();
                    self.pos += 1;
                    let f = self.parse_factor()?;
                    acc = if acc.is_constant() {
                        f.scale(acc.constant).map_err(|_| arith_err(at))?
                    } else if f.is_constant() {
                        acc.scale(f.constant).map_err(|_| arith_err(at))?
                    } else {
                        return Err(ParseError {
                            offset: at,
                            message: "product of two variables is not linear".to_string(),
                        });
                    };
                }
                Some(Kind::Slash) => {
                    let at = self.here();
                    self.pos += 1;
                    let f = self.parse_factor()?;
                    if !f.is_constant() {
                        return Err(ParseError {
                            offset: at,
                            message: "division by a variable is not linear".to_string(),
                        });
                    }
                    let inv = Rational::ONE
                        .checked_div(f.constant)
                        .map_err(|_| arith_err(at))?;
                    acc = acc.scale(inv).map_err(|_| arith_err(at))?;
                }
                _ => return Ok(acc),
            }
        }
    }

    fn parse_factor(&mut self) -> Result<LinExpr, ParseError> {
        let t = self.peek().ok_or(ParseError {
            offset: self.src.len(),
            message: "expected a value".to_string(),
        })?;
        match t.kind {
            Kind::Number => {
                self.pos += 1;
                let text = &self.src[t.start..t.end];
                let v = Rational::parse_decimal(text).map_err(|_| ParseError {
                    offset: t.start,
                    message: format_bad_number(text),
                })?;
                Ok(LinExpr::constant(v))
            }
            Kind::Ident => {
                self.pos += 1;
                Ok(LinExpr::var(&self.src[t.start..t.end]))
            }
            Kind::Minus => {
                self.pos += 1;
                let f = self.parse_factor()?;
                f.scale(Rational::from_int(-1))
                    .map_err(|_| arith_err(t.start))
            }
            Kind::LParen => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect(Kind::RParen, "')'")?;
                Ok(e)
            }
            _ => Err(ParseError {
                offset: t.start,
                message: "expected a number, a variable, '-' or '('".to_string(),
            }),
        }
    }
}

fn arith_err(offset: usize) -> ParseError {
    ParseError {
        offset,
        message: "number is too large for exact arithmetic".to_string(),
    }
}

fn format_expected(what: &str) -> String {
    let mut s = String::from("expected ");
    s.push_str(what);
    s
}

fn format_bad_number(text: &str) -> String {
    let mut s = String::from("'");
    s.push_str(text);
    s.push_str("' is not a plain decimal number");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::Bindings;

    fn env(pairs: &[(&str, &str)]) -> Bindings {
        let mut b = Bindings::new();
        for (n, v) in pairs {
            b.set(n, Rational::parse_decimal(v).unwrap());
        }
        b
    }

    #[test]
    fn parses_the_spec_contract_clauses() {
        let f = parse_formula("reservoir_level > 50 AND grid_power_headroom > 2000").unwrap();
        let ok = env(&[("reservoir_level", "60"), ("grid_power_headroom", "2500")]);
        let low = env(&[("reservoir_level", "40"), ("grid_power_headroom", "2500")]);
        assert_eq!(f.eval(&ok), Ok(true));
        assert_eq!(f.eval(&low), Ok(false));

        let g = parse_formula("pump_duty_cycle <= 0.15 AND led_dimming >= 0.8").unwrap();
        assert_eq!(
            g.eval(&env(&[("pump_duty_cycle", "0.2"), ("led_dimming", "1")])),
            Ok(false)
        );
        assert_eq!(
            g.eval(&env(&[("pump_duty_cycle", "0.15"), ("led_dimming", "0.8")])),
            Ok(true)
        );
    }

    #[test]
    fn precedence_is_not_then_and_then_or() {
        // a OR b AND c  ==  a OR (b AND c)
        let f = parse_formula("a > 0 OR b > 0 AND c > 0").unwrap();
        assert_eq!(
            f.eval(&env(&[("a", "1"), ("b", "-1"), ("c", "-1")])),
            Ok(true)
        );
        assert_eq!(
            f.eval(&env(&[("a", "-1"), ("b", "1"), ("c", "-1")])),
            Ok(false)
        );
        let n = parse_formula("NOT a > 0").unwrap();
        assert_eq!(n.eval(&env(&[("a", "-1")])), Ok(true));
    }

    #[test]
    fn parenthesised_arithmetic_on_the_left_of_a_comparison() {
        let f = parse_formula("(a + b) * 2 <= 10").unwrap();
        assert_eq!(f.eval(&env(&[("a", "3"), ("b", "2")])), Ok(true));
        assert_eq!(f.eval(&env(&[("a", "4"), ("b", "2")])), Ok(false));
        let g = parse_formula("(a > 0) AND (b > 0)").unwrap();
        assert_eq!(g.eval(&env(&[("a", "1"), ("b", "1")])), Ok(true));
    }

    #[test]
    fn linear_products_and_division_by_constants_are_allowed() {
        let f = parse_formula("3 * x - y / 4 == 1").unwrap();
        assert_eq!(f.eval(&env(&[("x", "1"), ("y", "8")])), Ok(true));
    }

    #[test]
    fn rejects_nonlinear_and_malformed_input() {
        assert!(
            parse_formula("x * y > 0")
                .unwrap_err()
                .message
                .contains("not linear")
        );
        assert!(
            parse_formula("x / y > 0")
                .unwrap_err()
                .message
                .contains("not linear")
        );
        assert!(parse_formula("").is_err());
        assert!(parse_formula("x >").is_err());
        assert!(parse_formula("x > 1 extra").is_err());
        assert!(parse_formula("x > 1 @ 2").is_err());
        assert!(parse_formula("x > 1e3").is_err());
        assert!(parse_formula("x > 1.2.3").is_err());
    }

    #[test]
    fn rejects_unit_suffixes_so_units_cannot_slip_through() {
        let err = parse_formula("reservoir_level > 50L").unwrap_err();
        assert!(err.message.contains("unit suffix"));
        assert_eq!(err.offset, 20);
    }

    #[test]
    fn atom_text_is_kept_for_explanations() {
        let f = parse_formula("pump_duty_cycle <= 0.15").unwrap();
        let atoms = f.atoms();
        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms[0].text, "pump_duty_cycle <= 0.15");
    }
}
