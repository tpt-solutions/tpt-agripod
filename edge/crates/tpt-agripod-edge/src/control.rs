// SPDX-License-Identifier: MIT OR Apache-2.0

//! Single-zone irrigation: hysteresis with a bounded pulse and a cooldown.
//!
//! The controller proposes; it does not decide. Every proposal still goes through the
//! contracts in `cycle.rs`. The pulse and cooldown bound how much water can be delivered
//! even if the moisture sensor misreads as dry, so the flood limit does not depend on the
//! sensor being right.

use crate::cycle::Command;

/// Thresholds and timing for irrigation.
#[derive(Debug, Clone, PartialEq)]
pub struct IrrigationPolicy {
    /// Start a pulse when moisture falls below this fraction.
    pub low_frac: f64,
    /// Stop a pulse when moisture rises above this fraction.
    pub high_frac: f64,
    /// Longest single pulse, seconds.
    pub pulse_s: u64,
    /// Rest between pulses, seconds.
    pub cooldown_s: u64,
    /// Valve open time proposed each cycle while pulsing, seconds.
    pub cycle_s: f64,
    /// Pump duty proposed while pulsing, fraction.
    pub pump_duty_frac: f64,
}

impl Default for IrrigationPolicy {
    fn default() -> Self {
        Self {
            low_frac: 0.35,
            high_frac: 0.45,
            pulse_s: 60,
            cooldown_s: 900,
            cycle_s: 10.0,
            pump_duty_frac: 0.12,
        }
    }
}

/// Where the controller is in its cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the moisture to drop.
    Idle,
    /// Irrigating until `until_s`.
    Pulsing {
        /// End of the pulse, Unix seconds.
        until_s: u64,
    },
    /// Resting until `until_s`.
    Cooldown {
        /// End of the rest, Unix seconds.
        until_s: u64,
    },
}

/// The single-zone irrigation controller.
#[derive(Debug, Clone, PartialEq)]
pub struct Irrigator {
    policy: IrrigationPolicy,
    phase: Phase,
}

impl Irrigator {
    /// A controller that starts idle.
    pub fn new(policy: IrrigationPolicy) -> Self {
        Self {
            policy,
            phase: Phase::Idle,
        }
    }

    /// The current phase, for logs and tests.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// One cycle's proposal given a validated moisture fraction and the current time.
    pub fn propose(&mut self, moisture_frac: f64, now_s: u64) -> Command {
        let p = &self.policy;
        match self.phase {
            Phase::Idle => {
                if moisture_frac < p.low_frac {
                    self.phase = Phase::Pulsing {
                        until_s: now_s + p.pulse_s,
                    };
                    self.open()
                } else {
                    Command::SAFE
                }
            }
            Phase::Pulsing { until_s } => {
                if now_s >= until_s || moisture_frac > p.high_frac {
                    self.phase = Phase::Cooldown {
                        until_s: now_s + p.cooldown_s,
                    };
                    Command::SAFE
                } else {
                    self.open()
                }
            }
            Phase::Cooldown { until_s } => {
                if now_s >= until_s {
                    self.phase = Phase::Idle;
                }
                Command::SAFE
            }
        }
    }

    fn open(&self) -> Command {
        Command {
            valve_open_s: self.policy.cycle_s,
            pump_duty_frac: self.policy.pump_duty_frac,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulses_when_dry_then_rests() {
        let mut c = Irrigator::new(IrrigationPolicy::default());
        assert_eq!(
            c.propose(0.30, 0),
            Command {
                valve_open_s: 10.0,
                pump_duty_frac: 0.12
            }
        );
        assert_eq!(c.propose(0.30, 10).valve_open_s, 10.0);
        // Pulse limit reached at t=60: stop and rest.
        assert_eq!(c.propose(0.30, 60), Command::SAFE);
        assert_eq!(c.phase(), Phase::Cooldown { until_s: 960 });
        assert_eq!(c.propose(0.30, 500), Command::SAFE);
        assert_eq!(c.propose(0.30, 960).valve_open_s, 0.0);
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn stops_early_when_wet_enough() {
        let mut c = Irrigator::new(IrrigationPolicy::default());
        c.propose(0.30, 0);
        assert_eq!(c.propose(0.46, 5), Command::SAFE);
        assert!(matches!(c.phase(), Phase::Cooldown { .. }));
    }

    #[test]
    fn water_is_bounded_even_if_the_sensor_stays_dry() {
        // Called once per 10 s control cycle, as the loop does. A pulse is 60 s, so it opens
        // the valve for 6 cycles. A period is the pulse plus the 900 s cooldown, about 970 s
        // with the cycle granularity, so 10,000 s holds at most 11 periods: at most 66 open cycles.
        let mut c = Irrigator::new(IrrigationPolicy::default());
        let mut open_cycles = 0;
        for t in (0..10_000u64).step_by(10) {
            if c.propose(0.0, t).valve_open_s > 0.0 {
                open_cycles += 1;
            }
        }
        assert!(open_cycles > 0);
        assert!(open_cycles <= 66, "{open_cycles} open cycles");
    }
}
