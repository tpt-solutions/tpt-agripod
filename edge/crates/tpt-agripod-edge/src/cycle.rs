// SPDX-License-Identifier: MIT OR Apache-2.0

//! One control cycle: validate sensors, watch for a pump that will not stop, propose a command,
//! check it against the contracts, then actuate. Any failure goes to safe state: valve
//! closed, pump off. A suspected fail-open pump latches until an operator resets the loop.
//!
//! Hardware and logging are injected through [`Actuators`] and [`Journal`], so every path can
//! be exercised on the host.

use tpt_agripod_solver::{Bindings, Contract, Decision, Rational, check_command, parse_contract};

use crate::control::Irrigator;
use crate::estimate::{Channel, Fault, Sample};
use crate::site::SiteConfig;

/// Contract for the pump, shipped in `edge/contracts/`.
pub const PUMP_CONTRACT: &str = include_str!("../../../contracts/pump.contract");

/// Actuator settings for one cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Command {
    /// How long the valve should be open this cycle, seconds.
    pub valve_open_s: f64,
    /// Pump duty cycle, fraction 0 to 1.
    pub pump_duty_frac: f64,
}

impl Command {
    /// Valve closed, pump off.
    pub const SAFE: Command = Command {
        valve_open_s: 0.0,
        pump_duty_frac: 0.0,
    };
}

/// Hardware the loop drives. Implementations talk to Modbus or to the ESP32 leaf tier.
pub trait Actuators {
    /// Applies a command. An error means the command may not have taken effect.
    fn apply(&mut self, cmd: &Command) -> Result<(), String>;
    /// Cuts power to the pump and valve drivers. Used when the pump may be running uncommanded.
    fn isolate(&mut self) -> Result<(), String>;
}

/// Something the loop tells the outside world about. Structured, so logs can be queried.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A command was applied.
    Applied(Command),
    /// A sensor channel refused its reading.
    SensorFault {
        /// Which channel.
        channel: &'static str,
        /// Why.
        fault: Fault,
    },
    /// A contract refused the proposed command.
    Denied {
        /// `"pump"` or `"valve"`.
        contract: &'static str,
        /// Explanations from the solver.
        reasons: Vec<String>,
    },
    /// The loop went to safe state.
    SafeState {
        /// Why.
        reason: String,
    },
    /// Hardware reported an error.
    ActuatorError {
        /// What it reported.
        reason: String,
    },
    /// The reservoir level fell while the pump and valve were commanded off.
    PumpFailOpenSuspected {
        /// Level drop since the last cycle, litres.
        drop_l: f64,
    },
    /// The loop is latched and will not actuate until reset.
    Latched {
        /// The latching reason.
        reason: String,
    },
    /// An operator reset the latch.
    Reset,
}

/// Sink for [`Event`]s.
pub trait Journal {
    /// Records one event.
    fn record(&mut self, event: Event);
}

/// The sensed inputs for one cycle. `None` means the sensor reported nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inputs {
    /// Current time, Unix seconds.
    pub now_s: u64,
    /// Root-zone moisture, fraction 0 to 1.
    pub moisture: Option<Sample>,
    /// Nutrient reservoir level, litres.
    pub reservoir_l: Option<Sample>,
    /// Grid power headroom, watts.
    pub headroom_w: Option<Sample>,
}

/// What the loop did this cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The command was applied.
    Applied(Command),
    /// The loop is in safe state for this cycle, for the given reason.
    Safe {
        /// Why.
        reason: String,
    },
    /// The loop is latched. It stays safe until [`ControlLoop::reset`].
    Latched {
        /// Why it latched.
        reason: String,
    },
}

/// The edge control loop.
pub struct ControlLoop<A: Actuators, J: Journal> {
    pump: Contract,
    valve: Contract,
    irrigator: Irrigator,
    moisture: Channel,
    reservoir: Channel,
    headroom: Channel,
    drain_limit_l: f64,
    prev_level_l: Option<f64>,
    last_applied: Command,
    latched: Option<String>,
    actuators: A,
    journal: J,
}

impl<A: Actuators, J: Journal> ControlLoop<A, J> {
    /// Builds the loop for one site. The valve contract is rendered from the site's limit, so the
    /// loop enforces the open-time limit the site file states.
    pub fn new(site: &SiteConfig, actuators: A, journal: J) -> Result<Self, String> {
        let pump = parse_contract(PUMP_CONTRACT).map_err(|e| format!("pump contract: {e:?}"))?;
        let valve_text = site.valve_contract();
        let valve = parse_contract(&valve_text).map_err(|e| format!("valve contract: {e:?}"))?;
        Ok(Self {
            pump,
            valve,
            irrigator: Irrigator::new(site.irrigation.clone()),
            // Ranges are physical limits. Moisture freezes are suspicious. Level holds still
            // legitimately, so it has no stuck check.
            moisture: Channel::new("moisture_frac", 0.0, 1.0, 30, Some(6)),
            reservoir: Channel::new("reservoir_level_l", 0.0, 500.0, 30, None),
            headroom: Channel::new("grid_power_headroom_w", 0.0, 50_000.0, 30, None),
            drain_limit_l: site.drain_limit_l,
            prev_level_l: None,
            last_applied: Command::SAFE,
            latched: None,
            actuators,
            journal,
        })
    }

    /// Runs one control cycle.
    pub fn step(&mut self, i: &Inputs) -> Outcome {
        if let Some(reason) = self.latched.clone() {
            self.enforce_safe();
            self.journal.record(Event::Latched {
                reason: reason.clone(),
            });
            return Outcome::Latched { reason };
        }

        let now = i.now_s;
        let m = self.moisture.accept(i.moisture, now);
        let r = self.reservoir.accept(i.reservoir_l, now);
        let h = self.headroom.accept(i.headroom_w, now);
        let (m, r, h) = match (m, r, h) {
            (Ok(m), Ok(r), Ok(h)) => (m, r, h),
            (m, r, h) => {
                let mut first = None;
                for (channel, result) in [
                    (self.moisture.name(), m),
                    (self.reservoir.name(), r),
                    (self.headroom.name(), h),
                ] {
                    if let Err(fault) = result {
                        self.journal.record(Event::SensorFault { channel, fault });
                        first.get_or_insert(format!("sensor {channel}: {fault:?}"));
                    }
                }
                let reason = first.unwrap_or_else(|| String::from("sensor fault"));
                return self.safe(reason);
            }
        };

        // Fail-open watch. The pump and valve are commanded off, yet the reservoir is draining.
        if let Some(prev) = self.prev_level_l {
            let idle = self.last_applied == Command::SAFE;
            let drop_l = prev - r;
            if idle && drop_l > self.drain_limit_l {
                self.journal.record(Event::PumpFailOpenSuspected { drop_l });
                let reason = format!("reservoir fell {drop_l:.2} L with pump and valve off");
                if self.actuators.isolate().is_err() {
                    self.journal.record(Event::ActuatorError {
                        reason: String::from("isolate failed"),
                    });
                }
                self.latched = Some(reason.clone());
                self.journal.record(Event::Latched {
                    reason: reason.clone(),
                });
                return Outcome::Latched { reason };
            }
        }
        self.prev_level_l = Some(r);

        let cmd = self.irrigator.propose(m, now);
        let (Some(pump_v), Some(valve_v)) = (
            to_rational(cmd.pump_duty_frac),
            to_rational(cmd.valve_open_s),
        ) else {
            return self.safe(String::from("command is not a finite decimal"));
        };
        let mut cmd_bindings = Bindings::new();
        cmd_bindings.set("pump_duty_cycle_frac", pump_v);
        cmd_bindings.set("valve_open_s", valve_v);
        let mut state = Bindings::new();
        state.set("reservoir_level_l", to_rational_exact(r));
        state.set("grid_power_headroom_w", to_rational_exact(h));

        // Find the first denial with a shared borrow, then record it once the borrow is gone.
        let denial = [("pump", &self.pump), ("valve", &self.valve)]
            .into_iter()
            .find_map(
                |(name, contract)| match check_command(contract, &state, &cmd_bindings) {
                    Decision::Deny { reasons, .. } => Some((name, reasons)),
                    Decision::Allow => None,
                },
            );
        if let Some((name, reasons)) = denial {
            self.journal.record(Event::Denied {
                contract: name,
                reasons,
            });
            return self.safe(format!("{name} contract denied the command"));
        }

        match self.actuators.apply(&cmd) {
            Ok(()) => {
                self.last_applied = cmd;
                self.journal.record(Event::Applied(cmd));
                Outcome::Applied(cmd)
            }
            Err(reason) => {
                self.journal.record(Event::ActuatorError {
                    reason: reason.clone(),
                });
                self.safe(format!("actuator error: {reason}"))
            }
        }
    }

    /// Operator reset after a latch. The loop resumes on the next cycle if the sensors are good.
    pub fn reset(&mut self) {
        self.latched = None;
        // The level reference is stale after a latch. Start the drain watch afresh, so a
        // reset does not re-trip on the drop that caused the latch. A pump that is still
        // failing will show a new drop and trip again.
        self.prev_level_l = None;
        self.journal.record(Event::Reset);
    }

    /// Applies the setpoints of a validated profile. The pump duty takes effect on the next pulse.
    /// Only a profile that passed `profile::validate` may be passed here.
    pub fn apply_profile(&mut self, profile: &crate::profile::ActiveProfile) {
        self.irrigator.set_pump_duty_frac(profile.pump_duty_frac);
    }

    /// Whether the loop is latched.
    pub fn is_latched(&self) -> bool {
        self.latched.is_some()
    }

    /// The journal, for inspection.
    pub fn journal(&self) -> &J {
        &self.journal
    }

    /// The actuators, for inspection.
    pub fn actuators_mut(&mut self) -> &mut A {
        &mut self.actuators
    }

    /// The actuators, for inspection.
    pub fn actuators(&self) -> &A {
        &self.actuators
    }

    fn safe(&mut self, reason: String) -> Outcome {
        self.enforce_safe();
        self.journal.record(Event::SafeState {
            reason: reason.clone(),
        });
        Outcome::Safe { reason }
    }

    /// Drives the hardware to safe state. If even that fails, cuts power as a last resort.
    fn enforce_safe(&mut self) {
        self.last_applied = Command::SAFE;
        if let Err(reason) = self.actuators.apply(&Command::SAFE) {
            self.journal.record(Event::ActuatorError {
                reason: reason.clone(),
            });
            if self.actuators.isolate().is_err() {
                self.journal.record(Event::ActuatorError {
                    reason: String::from("isolate failed after safe-state write failed"),
                });
            }
        }
    }
}

/// A controller float as an exact decimal with six places. Returns `None` if it is not finite.
fn to_rational(v: f64) -> Option<Rational> {
    if !v.is_finite() {
        return None;
    }
    Rational::parse_decimal(&format!("{v:.6}")).ok()
}

/// A sensor float as an exact rational, through the same six-place decimal form.
fn to_rational_exact(v: f64) -> Rational {
    to_rational(v).unwrap_or(Rational::ZERO)
}
