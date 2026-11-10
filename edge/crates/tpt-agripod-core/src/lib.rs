// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared types for the Agripod edge node.
//!
//! This crate holds types that every other edge crate depends on. It must stay
//! `no_std`-compatible in spirit: no I/O, no wall-clock, no randomness.

/// Version of the Crop Profile schema this build understands (`profiles/crop-profile.schema.json`).
pub const PROFILE_SCHEMA_VERSION: &str = "0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_schema_version_matches_schema_file() {
        let schema = include_str!("../../../../profiles/crop-profile.schema.json");
        assert!(schema.contains(&format!("\"const\": \"{PROFILE_SCHEMA_VERSION}\"")));
    }
}
