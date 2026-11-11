// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared test fixtures. The site values are a test fixture, not defaults: production code has none.

use tpt_agripod_edge::SiteConfig;

const FIXTURE: &str = include_str!("../fixtures/site.test.json");

/// A validated test site.
pub fn test_site() -> SiteConfig {
    SiteConfig::parse(FIXTURE).expect("the test fixture must validate")
}
