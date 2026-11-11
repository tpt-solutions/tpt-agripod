// SPDX-License-Identifier: MIT OR Apache-2.0

//! A profile applied at runtime changes what the loop sends, and a refused profile changes
//! nothing. Uses the same example profile the orchestrator ships.

mod common;

use tpt_agripod_edge::{
    Actuators, ApplyOutcome, Command, ControlLoop, Event, Inputs, Journal, Outcome, ProfileStore,
    Sample,
};

const EXAMPLE: &str = include_str!("../../../../profiles/examples/example-plant-v0.json");

#[derive(Default)]
struct Hw(Vec<Command>);

impl Actuators for Hw {
    fn apply(&mut self, cmd: &Command) -> Result<(), String> {
        self.0.push(*cmd);
        Ok(())
    }

    fn isolate(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
struct Log(Vec<Event>);

impl Journal for Log {
    fn record(&mut self, e: Event) {
        self.0.push(e);
    }
}

fn dry_cycle(t: u64) -> Inputs {
    Inputs {
        now_s: t,
        moisture: Some(Sample {
            value: 0.2,
            at_s: t,
        }),
        reservoir_l: Some(Sample {
            value: 80.0,
            at_s: t,
        }),
        headroom_w: Some(Sample {
            value: 2500.0,
            at_s: t,
        }),
    }
}

#[test]
fn applied_profile_sets_the_pump_duty_sent_to_hardware() {
    let mut store = ProfileStore::new();
    let ApplyOutcome::Applied(active) = store.apply(EXAMPLE) else {
        panic!("example profile must apply");
    };
    let mut l = ControlLoop::new(&common::test_site(), Hw::default(), Log::default()).unwrap();
    l.apply_profile(&active);

    // The example sets pump_duty to 0.10, not the built-in 0.12.
    let out = l.step(&dry_cycle(0));
    assert_eq!(
        out,
        Outcome::Applied(Command {
            valve_open_s: 10.0,
            pump_duty_frac: 0.10
        })
    );
}

#[test]
fn refused_profile_leaves_the_loop_on_its_previous_setpoint() {
    let mut store = ProfileStore::new();
    let ApplyOutcome::Applied(active) = store.apply(EXAMPLE) else {
        panic!("example profile must apply");
    };
    let mut l = ControlLoop::new(&common::test_site(), Hw::default(), Log::default()).unwrap();
    l.apply_profile(&active);

    // A pump max above the contract is refused. The store keeps the good profile, and the
    // loop keeps its setpoint.
    let bad = EXAMPLE.replacen(r#""max": 0.15,"#, r#""max": 0.9,"#, 1);
    match store.apply(&bad) {
        ApplyOutcome::Rejected(problems) => assert!(!problems.is_empty()),
        ApplyOutcome::Applied(_) => panic!("unsafe profile applied"),
    }
    assert_eq!(store.active().map(|p| p.pump_duty_frac), Some(0.10));

    let out = l.step(&dry_cycle(0));
    assert_eq!(
        out,
        Outcome::Applied(Command {
            valve_open_s: 10.0,
            pump_duty_frac: 0.10
        })
    );
}
