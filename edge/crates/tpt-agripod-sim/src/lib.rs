// SPDX-License-Identifier: MIT OR Apache-2.0

//! Deterministic simulation of the container: plant transpiration, latent and sensible
//! heat loads, and HVAC energy with a predicted cost. Phase 1 of `todo.md`.
//!
//! Pipeline: [`sensor::parse_csv`] validates input, [`simulate::simulate`] integrates the
//! model, and [`report`] formats the result. Parameters are assumptions; see
//! `docs/phase1-model.md`.

pub mod mock;
pub mod params;
pub mod plant;
pub mod psychro;
pub mod report;
pub mod sensor;
pub mod simulate;
pub mod time;

pub use params::SimParams;
pub use sensor::{Ingest, Rejection, Sample, parse_csv};
pub use simulate::{Summary, simulate};
