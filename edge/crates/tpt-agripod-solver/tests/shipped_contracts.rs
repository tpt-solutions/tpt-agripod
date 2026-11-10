// SPDX-License-Identifier: MIT OR Apache-2.0

//! The contracts shipped in `edge/contracts/` must parse, must not be contradictory (a
//! contradictory `ensures` would block every command), and must behave as the spec describes.

use tpt_agripod_solver::fm::{Feasibility, conjunction, decide};
use tpt_agripod_solver::{Bindings, Decision, Phase, Rational, check_command, parse_contract};

const PUMP: &str = include_str!("../../../contracts/pump.contract");
const LED: &str = include_str!("../../../contracts/led.contract");
const SOLENOID: &str = include_str!("../../../contracts/solenoid.contract");

fn bind(pairs: &[(&str, &str)]) -> Bindings {
    let mut b = Bindings::new();
    for (n, v) in pairs {
        b.set(n, Rational::parse_decimal(v).unwrap());
    }
    b
}

#[test]
fn all_shipped_contracts_parse() {
    for text in [PUMP, LED, SOLENOID] {
        parse_contract(text).unwrap();
    }
}

#[test]
fn no_shipped_ensures_clause_is_contradictory() {
    // Lint: the ensures clause alone must be satisfiable, or no command could ever pass it.
    for (name, text) in [("pump", PUMP), ("led", LED), ("solenoid", SOLENOID)] {
        let c = parse_contract(text).unwrap();
        let sys = conjunction(&c.ensures).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            decide(&sys, 10_000),
            Feasibility::Sat,
            "{name} ensures is contradictory"
        );
    }
}

#[test]
fn a_contradictory_clause_is_detected() {
    // The lint must catch a real contradiction, or the test above proves nothing.
    let c = parse_contract(
        "requires: x > 0\nensures: pump_duty_cycle_frac <= 0.1 AND pump_duty_cycle_frac >= 0.2\n",
    )
    .unwrap();
    let sys = conjunction(&c.ensures).unwrap();
    assert_eq!(decide(&sys, 10_000), Feasibility::Unsat);
}

#[test]
fn pump_refuses_a_command_above_the_duty_limit_and_allows_one_at_it() {
    let c = parse_contract(PUMP).unwrap();
    let state = bind(&[
        ("reservoir_level_l", "60"),
        ("grid_power_headroom_w", "2500"),
    ]);
    assert_eq!(
        check_command(&c, &state, &bind(&[("pump_duty_cycle_frac", "0.15")])),
        Decision::Allow
    );
    match check_command(&c, &state, &bind(&[("pump_duty_cycle_frac", "0.16")])) {
        Decision::Deny { phase, .. } => assert_eq!(phase, Phase::Ensures),
        other => panic!("expected denial, got {other:?}"),
    }
}

#[test]
fn pump_refuses_when_the_reservoir_is_low() {
    let c = parse_contract(PUMP).unwrap();
    let state = bind(&[
        ("reservoir_level_l", "50"),
        ("grid_power_headroom_w", "2500"),
    ]);
    match check_command(&c, &state, &bind(&[("pump_duty_cycle_frac", "0.1")])) {
        Decision::Deny { phase, reasons } => {
            assert_eq!(phase, Phase::Requires);
            // Boundary: 50 is not > 50, so the command is refused.
            assert!(reasons[0].contains("reservoir_level_l > 50"), "{reasons:?}");
        }
        other => panic!("expected denial, got {other:?}"),
    }
}

#[test]
fn pump_contract_is_fail_closed_without_a_sensor() {
    let c = parse_contract(PUMP).unwrap();
    let state = bind(&[("grid_power_headroom_w", "2500")]);
    assert!(matches!(
        check_command(&c, &state, &bind(&[("pump_duty_cycle_frac", "0")])),
        Decision::Deny {
            phase: Phase::Requires,
            ..
        }
    ));
}

#[test]
fn led_contract_bounds_dimming_on_both_sides() {
    let c = parse_contract(LED).unwrap();
    let state = bind(&[("grid_power_headroom_w", "1000")]);
    for (v, ok) in [("0.79", false), ("0.8", true), ("1", true), ("1.01", false)] {
        let decision = check_command(&c, &state, &bind(&[("led_dimming_frac", v)]));
        assert_eq!(decision == Decision::Allow, ok, "dimming {v}: {decision:?}");
    }
}
