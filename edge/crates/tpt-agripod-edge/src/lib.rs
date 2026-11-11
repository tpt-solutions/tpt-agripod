// SPDX-License-Identifier: MIT OR Apache-2.0

//! Edge control loop. Sense, estimate, propose, check contracts, actuate, log. Safe state on
//! any failure.
//!
//! * [`estimate`]: sensor validation (missing, stale, out of range, stuck). No holding over.
//! * [`control`]: single-zone irrigation with hysteresis, a pulse limit and a cooldown.
//! * [`cycle`]: the loop. Contracts from `edge/contracts/` gate every command.
//!
//! The fleet link is not a safety input. The loop runs offline (spec section 4.1).

pub mod control;
pub mod cycle;
pub mod estimate;
pub mod json;
pub mod profile;

pub use control::{IrrigationPolicy, Irrigator, Phase};
pub use cycle::{Actuators, Command, ControlLoop, Event, Inputs, Journal, LoopConfig, Outcome};
pub use estimate::{Channel, Fault, Sample};
pub use profile::{ActiveProfile, ApplyOutcome, ProfileStore};
