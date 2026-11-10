// SPDX-License-Identifier: MIT OR Apache-2.0

//! Exact rational arithmetic on `i128`. Every operation is checked: an overflow is an error,
//! never a wrap or a silent float rounding. Callers treat an error as "cannot decide", which
//! fails closed.

use core::cmp::Ordering;

/// Failure of an exact operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithError {
    /// A value or intermediate result does not fit in `i128`.
    Overflow,
    /// Division by zero, or a zero denominator.
    DivideByZero,
    /// A decimal literal has too many fractional digits to represent exactly.
    TooManyDigits,
    /// A decimal literal is not a plain signed decimal number.
    BadLiteral,
}

/// A rational number `num / den` in lowest terms with `den > 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    num: i128,
    den: i128,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

fn mul(a: i128, b: i128) -> Result<i128, ArithError> {
    a.checked_mul(b).ok_or(ArithError::Overflow)
}

fn add(a: i128, b: i128) -> Result<i128, ArithError> {
    a.checked_add(b).ok_or(ArithError::Overflow)
}

impl Rational {
    /// Zero.
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    /// One.
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    /// Builds `num / den` in lowest terms.
    pub fn new(num: i128, den: i128) -> Result<Self, ArithError> {
        if den == 0 {
            return Err(ArithError::DivideByZero);
        }
        // i128::MIN has no positive counterpart; refuse it so `abs` and `neg` are total.
        if num == i128::MIN || den == i128::MIN {
            return Err(ArithError::Overflow);
        }
        let g = gcd(num, den);
        let (mut n, mut d) = (num / g, den / g);
        if d < 0 {
            n = -n;
            d = -d;
        }
        Ok(Self { num: n, den: d })
    }

    /// An integer.
    pub fn from_int(v: i64) -> Self {
        Self {
            num: i128::from(v),
            den: 1,
        }
    }

    /// Parses a plain signed decimal such as `-12.345` or `0.15`. No exponents, no signs
    /// other than a leading minus, and no unit suffixes.
    pub fn parse_decimal(s: &str) -> Result<Self, ArithError> {
        let (negative, body) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let mut num: i128 = 0;
        let mut den: i128 = 1;
        let mut seen_dot = false;
        let mut digits = 0usize;
        for c in body.chars() {
            match c {
                '0'..='9' => {
                    digits += 1;
                    let d = i128::from(c as u8 - b'0');
                    num = add(mul(num, 10)?, d)?;
                    if seen_dot {
                        if den >= i128::MAX / 10 {
                            return Err(ArithError::TooManyDigits);
                        }
                        den *= 10;
                    }
                }
                '.' if !seen_dot => seen_dot = true,
                _ => return Err(ArithError::BadLiteral),
            }
        }
        if digits == 0 {
            return Err(ArithError::BadLiteral);
        }
        if negative {
            num = -num;
        }
        Self::new(num, den)
    }

    /// Numerator in lowest terms (sign carried here).
    pub fn num(&self) -> i128 {
        self.num
    }

    /// Denominator in lowest terms (always positive).
    pub fn den(&self) -> i128 {
        self.den
    }

    /// True for zero.
    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    /// Negation.
    pub fn negate(self) -> Self {
        // Safe: `new` never stores i128::MIN, so the numerator can be negated.
        Self {
            num: -self.num,
            den: self.den,
        }
    }

    /// Sum.
    pub fn checked_add(self, o: Self) -> Result<Self, ArithError> {
        let g = gcd(self.den, o.den);
        let lcm = mul(self.den, o.den / g)?;
        let a = mul(self.num, lcm / self.den)?;
        let c = mul(o.num, lcm / o.den)?;
        Self::new(add(a, c)?, lcm)
    }

    /// Difference.
    pub fn checked_sub(self, o: Self) -> Result<Self, ArithError> {
        self.checked_add(o.negate())
    }

    /// Product.
    pub fn checked_mul(self, o: Self) -> Result<Self, ArithError> {
        // Cross-cancel first so intermediate products stay small.
        let g1 = gcd(self.num, o.den).max(1);
        let g2 = gcd(o.num, self.den).max(1);
        let num = mul(self.num / g1, o.num / g2)?;
        let den = mul(self.den / g2, o.den / g1)?;
        Self::new(num, den)
    }

    /// Quotient. Fails on division by zero.
    pub fn checked_div(self, o: Self) -> Result<Self, ArithError> {
        if o.is_zero() {
            return Err(ArithError::DivideByZero);
        }
        let inverse = Self::new(o.den, o.num)?;
        self.checked_mul(inverse)
    }

    /// Ordering. Compares by cross-multiplication, which can overflow.
    pub fn checked_cmp(self, o: Self) -> Result<Ordering, ArithError> {
        let l = mul(self.num, o.den)?;
        let r = mul(o.num, self.den)?;
        Ok(l.cmp(&r))
    }

    /// Nearest `f64`, for display only. Never used in a decision.
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i128, d: i128) -> Rational {
        Rational::new(n, d).unwrap()
    }

    #[test]
    fn normalises_to_lowest_terms_with_positive_denominator() {
        assert_eq!(r(2, 4), r(1, 2));
        assert_eq!(r(3, -6), r(-1, 2));
        assert_eq!(r(0, -5), Rational::ZERO);
        assert_eq!(Rational::new(1, 0), Err(ArithError::DivideByZero));
    }

    #[test]
    fn parses_decimals_exactly() {
        assert_eq!(Rational::parse_decimal("0.15").unwrap(), r(3, 20));
        assert_eq!(Rational::parse_decimal("-12.5").unwrap(), r(-25, 2));
        assert_eq!(
            Rational::parse_decimal("50").unwrap(),
            Rational::from_int(50)
        );
        assert_eq!(Rational::parse_decimal("."), Err(ArithError::BadLiteral));
        assert_eq!(Rational::parse_decimal("1e3"), Err(ArithError::BadLiteral));
        assert_eq!(Rational::parse_decimal("50L"), Err(ArithError::BadLiteral));
        assert_eq!(Rational::parse_decimal("+1"), Err(ArithError::BadLiteral));
    }

    #[test]
    fn arithmetic_is_exact() {
        // 0.1 + 0.2 is exactly 3/10 here, unlike f64.
        let tenth = r(1, 10);
        assert_eq!(tenth.checked_add(r(1, 5)).unwrap(), r(3, 10));
        assert_eq!(r(1, 3).checked_mul(r(3, 7)).unwrap(), r(1, 7));
        assert_eq!(r(1, 2).checked_div(r(-1, 4)).unwrap(), r(-2, 1));
        assert_eq!(r(1, 2).checked_sub(r(1, 2)).unwrap(), Rational::ZERO);
        assert_eq!(
            r(1, 2).checked_div(Rational::ZERO),
            Err(ArithError::DivideByZero)
        );
    }

    #[test]
    fn comparisons_are_exact() {
        assert_eq!(r(1, 3).checked_cmp(r(1, 3)).unwrap(), Ordering::Equal);
        assert_eq!(r(1, 3).checked_cmp(r(3, 10)).unwrap(), Ordering::Greater);
        assert_eq!(
            r(-1, 2).checked_cmp(Rational::ZERO).unwrap(),
            Ordering::Less
        );
    }

    #[test]
    fn overflow_is_an_error_not_a_wrap() {
        let big = Rational::new(i128::MAX / 2, 1).unwrap();
        assert_eq!(
            big.checked_add(big).and_then(|x| x.checked_add(big)),
            Err(ArithError::Overflow)
        );
        assert_eq!(big.checked_mul(big), Err(ArithError::Overflow));
        assert_eq!(Rational::new(i128::MIN, 1), Err(ArithError::Overflow));
    }
}
