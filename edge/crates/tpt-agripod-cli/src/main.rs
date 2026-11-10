// SPDX-License-Identifier: MIT OR Apache-2.0

//! `agripod` command-line tool. Skeleton: prints the version and the profile schema it supports.

fn main() {
    println!(
        "agripod {} (profile schema {})",
        env!("CARGO_PKG_VERSION"),
        tpt_agripod_core::PROFILE_SCHEMA_VERSION
    );
}
