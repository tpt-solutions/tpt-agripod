// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sensor validation. A channel accepts a reading only if it is present, finite, inside its
//! physical range, fresh, and not frozen. Anything else is a fault, and the loop goes to safe
//! state. Nothing is interpolated or held over, because a held value would hide a dead sensor.

/// One reading and the time it was taken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Measured value, in the channel's units.
    pub value: f64,
    /// When it was taken, Unix seconds.
    pub at_s: u64,
}

/// Why a channel refused a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// No reading this cycle.
    Missing,
    /// The reading is NaN, infinite, or outside the physical range.
    OutOfRange,
    /// The reading is older than the channel's limit.
    Stale,
    /// The same value has come back for too many cycles in a row.
    Stuck,
}

/// Validation rules for one sensor channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    name: &'static str,
    lo: f64,
    hi: f64,
    max_age_s: u64,
    /// Repeats of an identical value that count as stuck. `None` disables the check, for
    /// quantities that legitimately hold still, such as a full reservoir.
    stuck_after: Option<u32>,
    last: Option<f64>,
    repeats: u32,
}

impl Channel {
    /// A channel named `name` accepting values in `[lo, hi]`, fresh within `max_age_s`.
    pub fn new(
        name: &'static str,
        lo: f64,
        hi: f64,
        max_age_s: u64,
        stuck_after: Option<u32>,
    ) -> Self {
        Self {
            name,
            lo,
            hi,
            max_age_s,
            stuck_after,
            last: None,
            repeats: 0,
        }
    }

    /// The channel name, for logs.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Validates one cycle's reading. `now_s` is the current time.
    pub fn accept(&mut self, sample: Option<Sample>, now_s: u64) -> Result<f64, Fault> {
        let s = sample.ok_or(Fault::Missing)?;
        if !s.value.is_finite() || s.value < self.lo || s.value > self.hi {
            self.last = None;
            self.repeats = 0;
            return Err(Fault::OutOfRange);
        }
        if now_s.saturating_sub(s.at_s) > self.max_age_s {
            return Err(Fault::Stale);
        }
        if self.last == Some(s.value) {
            self.repeats = self.repeats.saturating_add(1);
        } else {
            self.last = Some(s.value);
            self.repeats = 1;
        }
        if let Some(limit) = self.stuck_after {
            if self.repeats >= limit {
                return Err(Fault::Stuck);
            }
        }
        Ok(s.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moisture() -> Channel {
        Channel::new("moisture_frac", 0.0, 1.0, 30, Some(4))
    }

    #[test]
    fn accepts_a_fresh_in_range_reading() {
        let mut c = moisture();
        assert_eq!(
            c.accept(
                Some(Sample {
                    value: 0.4,
                    at_s: 100
                }),
                110
            ),
            Ok(0.4)
        );
    }

    #[test]
    fn refuses_missing_stale_and_out_of_range() {
        let mut c = moisture();
        assert_eq!(c.accept(None, 0), Err(Fault::Missing));
        assert_eq!(
            c.accept(
                Some(Sample {
                    value: 0.4,
                    at_s: 0
                }),
                31
            ),
            Err(Fault::Stale)
        );
        assert_eq!(
            c.accept(
                Some(Sample {
                    value: 1.2,
                    at_s: 0
                }),
                0
            ),
            Err(Fault::OutOfRange)
        );
        assert_eq!(
            c.accept(
                Some(Sample {
                    value: f64::NAN,
                    at_s: 0
                }),
                0
            ),
            Err(Fault::OutOfRange)
        );
    }

    #[test]
    fn a_frozen_sensor_is_stuck_after_its_limit() {
        let mut c = moisture();
        for t in 0..3 {
            assert_eq!(
                c.accept(
                    Some(Sample {
                        value: 0.4,
                        at_s: t
                    }),
                    t
                ),
                Ok(0.4),
                "cycle {t}"
            );
        }
        assert_eq!(
            c.accept(
                Some(Sample {
                    value: 0.4,
                    at_s: 3
                }),
                3
            ),
            Err(Fault::Stuck)
        );
        // A change releases it.
        assert_eq!(
            c.accept(
                Some(Sample {
                    value: 0.41,
                    at_s: 4
                }),
                4
            ),
            Ok(0.41)
        );
    }

    #[test]
    fn channels_without_a_stuck_limit_may_hold_still() {
        let mut c = Channel::new("reservoir_level_l", 0.0, 500.0, 30, None);
        for t in 0..100 {
            assert_eq!(
                c.accept(
                    Some(Sample {
                        value: 80.0,
                        at_s: t
                    }),
                    t
                ),
                Ok(80.0)
            );
        }
    }
}
