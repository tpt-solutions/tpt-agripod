// SPDX-License-Identifier: MIT OR Apache-2.0

//! Fault injection on the edge loop, on the host. Each scenario is one failure from the todo:
//! sensor loss, stuck sensor, out-of-range sensor, contract denial, actuator error, and a pump
//! that fails open. Plus a seeded property run over random inputs.

mod common;

use tpt_agripod_edge::{
    Actuators, Command, ControlLoop, Event, Fault, Inputs, Journal, Outcome, Sample,
};

#[derive(Default)]
struct FakeActuators {
    applied: Vec<Command>,
    isolated: u32,
    fail_apply: bool,
    fail_isolate: bool,
}

impl Actuators for FakeActuators {
    fn apply(&mut self, cmd: &Command) -> Result<(), String> {
        if self.fail_apply {
            return Err(String::from("bus timeout"));
        }
        self.applied.push(*cmd);
        Ok(())
    }

    fn isolate(&mut self) -> Result<(), String> {
        if self.fail_isolate {
            return Err(String::from("relay stuck"));
        }
        self.isolated += 1;
        Ok(())
    }
}

#[derive(Default)]
struct VecJournal(Vec<Event>);

impl Journal for VecJournal {
    fn record(&mut self, event: Event) {
        self.0.push(event);
    }
}

type Loop = ControlLoop<FakeActuators, VecJournal>;

fn new_loop() -> Loop {
    ControlLoop::new(
        &crate::common::test_site(),
        FakeActuators::default(),
        VecJournal::default(),
    )
    .expect("shipped contracts parse")
}

fn sample(value: f64, t: u64) -> Option<Sample> {
    Some(Sample { value, at_s: t })
}

/// A healthy cycle at time `t` with the given moisture and reservoir level.
fn healthy(t: u64, moisture: f64, level: f64) -> Inputs {
    Inputs {
        now_s: t,
        moisture: sample(moisture, t),
        reservoir_l: sample(level, t),
        headroom_w: sample(2500.0, t),
    }
}

fn last_hardware_command(l: &Loop) -> Option<Command> {
    l.actuators().applied.last().copied()
}

fn is_safe(o: &Outcome) -> bool {
    !matches!(o, Outcome::Applied(_))
}

#[test]
fn dry_soil_irrigates_within_limits() {
    let mut l = new_loop();
    let out = l.step(&healthy(0, 0.2, 80.0));
    let expected = Command {
        valve_open_s: 10.0,
        pump_duty_frac: 0.12,
    };
    assert_eq!(out, Outcome::Applied(expected));
    assert_eq!(last_hardware_command(&l), Some(expected));
}

#[test]
fn wet_soil_stays_off() {
    let mut l = new_loop();
    assert_eq!(
        l.step(&healthy(0, 0.6, 80.0)),
        Outcome::Applied(Command::SAFE)
    );
}

#[test]
fn missing_moisture_reading_goes_safe_and_closes_the_valve() {
    let mut l = new_loop();
    l.step(&healthy(0, 0.2, 80.0)); // irrigating
    let inputs = Inputs {
        now_s: 10,
        moisture: None,
        ..healthy(10, 0.2, 80.0)
    };
    let out = l.step(&inputs);
    assert!(matches!(out, Outcome::Safe { .. }), "{out:?}");
    assert_eq!(last_hardware_command(&l), Some(Command::SAFE));
    assert!(l.journal().0.iter().any(|e| matches!(
        e,
        Event::SensorFault {
            channel: "moisture_frac",
            fault: Fault::Missing
        }
    )));
}

#[test]
fn stale_reading_is_refused() {
    let mut l = new_loop();
    let inputs = Inputs {
        now_s: 100,
        moisture: sample(0.2, 60), // 40 s old, limit 30 s
        ..healthy(100, 0.2, 80.0)
    };
    assert!(is_safe(&l.step(&inputs)));
    assert!(l.journal().0.iter().any(|e| matches!(
        e,
        Event::SensorFault {
            fault: Fault::Stale,
            ..
        }
    )));
}

#[test]
fn out_of_range_reading_is_refused() {
    let mut l = new_loop();
    assert!(is_safe(&l.step(&healthy(0, 1.4, 80.0))));
    assert!(is_safe(&l.step(&healthy(1, f64::NAN, 80.0))));
}

#[test]
fn frozen_moisture_sensor_is_detected_before_it_floods() {
    // A sensor stuck at "dry" would keep irrigating forever. It must be refused.
    let mut l = new_loop();
    let mut outcomes = Vec::new();
    for t in 0..10 {
        outcomes.push(l.step(&healthy(t, 0.2, 80.0)));
    }
    let first_safe = outcomes
        .iter()
        .position(is_safe)
        .expect("stuck sensor must trip");
    assert_eq!(first_safe, 5, "trips on the 6th identical reading");
    assert!(
        outcomes[..5]
            .iter()
            .all(|o| matches!(o, Outcome::Applied(_)))
    );
}

#[test]
fn low_reservoir_denies_the_pump_before_it_runs() {
    let mut l = new_loop();
    let out = l.step(&healthy(0, 0.2, 50.0)); // requires is "> 50": 50 is refused
    assert!(matches!(out, Outcome::Safe { .. }), "{out:?}");
    assert!(l.journal().0.iter().any(|e| matches!(
        e,
        Event::Denied { contract: "pump", reasons } if reasons.iter().any(|r| r.contains("reservoir_level_l > 50"))
    )));
    assert!(l.actuators().applied.iter().all(|c| *c == Command::SAFE));
}

#[test]
fn low_power_headroom_denies_the_pump() {
    let mut l = new_loop();
    let inputs = Inputs {
        headroom_w: sample(1500.0, 0),
        ..healthy(0, 0.2, 80.0)
    };
    assert!(is_safe(&l.step(&inputs)));
    assert!(l.journal().0.iter().any(|e| matches!(
        e,
        Event::Denied {
            contract: "pump",
            ..
        }
    )));
}

#[test]
fn actuator_error_goes_safe() {
    let mut l = new_loop();
    l.actuators_mut().fail_apply = true;
    let out = l.step(&healthy(0, 0.2, 80.0));
    assert!(matches!(out, Outcome::Safe { .. }), "{out:?}");
    assert!(
        l.journal()
            .0
            .iter()
            .any(|e| matches!(e, Event::ActuatorError { .. }))
    );
}

#[test]
fn draining_while_idle_latches_as_a_fail_open_pump() {
    let mut l = new_loop();
    // Wet soil, so the loop is idle and sends SAFE.
    assert_eq!(
        l.step(&healthy(0, 0.6, 80.0)),
        Outcome::Applied(Command::SAFE)
    );
    // The reservoir drops 5 L with nothing commanded: the pump is running uncommanded.
    let out = l.step(&healthy(10, 0.6, 75.0));
    assert!(matches!(out, Outcome::Latched { .. }), "{out:?}");
    assert_eq!(l.actuators().isolated, 1, "power cut to the pump");
    assert!(l.is_latched());
    // Stays latched, even when the sensors look healthy and dry.
    assert!(matches!(
        l.step(&healthy(20, 0.2, 75.0)),
        Outcome::Latched { .. }
    ));
    // Operator reset resumes control. The same 5 L drop must not re-trip the latch.
    l.reset();
    let expected = Command {
        valve_open_s: 10.0,
        pump_duty_frac: 0.12,
    };
    assert_eq!(l.step(&healthy(30, 0.2, 75.0)), Outcome::Applied(expected));
    assert!(!l.is_latched());
}

#[test]
fn a_pump_that_is_still_failing_trips_again_after_reset() {
    let mut l = new_loop();
    l.step(&healthy(0, 0.6, 80.0));
    assert!(matches!(
        l.step(&healthy(10, 0.6, 75.0)),
        Outcome::Latched { .. }
    ));
    l.reset();
    // The reservoir keeps falling while idle: a genuine new drop.
    assert!(matches!(
        l.step(&healthy(20, 0.6, 75.0)),
        Outcome::Applied(_)
    ));
    assert!(matches!(
        l.step(&healthy(30, 0.6, 68.0)),
        Outcome::Latched { .. }
    ));
}

#[test]
fn draining_while_irrigating_is_not_a_fail_open() {
    // The pump is supposed to be running, so the level falling is expected.
    let mut l = new_loop();
    assert!(matches!(
        l.step(&healthy(0, 0.2, 80.0)),
        Outcome::Applied(_)
    ));
    let out = l.step(&healthy(10, 0.2, 70.0));
    assert!(matches!(out, Outcome::Applied(_)), "{out:?}");
    assert!(!l.is_latched());
}

#[test]
fn sensor_loss_mid_irrigation_closes_the_valve_at_once() {
    let mut l = new_loop();
    assert!(matches!(
        l.step(&healthy(0, 0.2, 80.0)),
        Outcome::Applied(_)
    ));
    let out = l.step(&Inputs {
        reservoir_l: None,
        ..healthy(10, 0.2, 80.0)
    });
    assert!(is_safe(&out));
    assert_eq!(last_hardware_command(&l), Some(Command::SAFE));
}

#[test]
fn recovers_on_its_own_after_a_transient_sensor_fault() {
    let mut l = new_loop();
    assert!(is_safe(&l.step(&Inputs {
        moisture: None,
        ..healthy(0, 0.2, 80.0)
    })));
    assert!(matches!(
        l.step(&healthy(1, 0.2, 80.0)),
        Outcome::Applied(_)
    ));
    assert!(!l.is_latched());
}

#[test]
fn random_faults_never_leave_the_valve_open_without_a_passing_contract() {
    // Property run. Seeded, so failures reproduce. Every cycle, either the command was applied
    // and the contracts hold for it, or the last command the hardware received is SAFE.
    // The reservoir follows a random walk with rare injected drains, so the loop spends time
    // in every state: irrigating, cooling down, safe on faults, and latched then reset.
    let mut state: u64 = 0xA6A1_90D0_0000_0001;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let unit = |x: u64| (x % 10_000) as f64 / 10_000.0;
    let mut applied_cycles = 0;
    let mut safe_cycles = 0;
    let mut latches = 0;

    for _run in 0..100 {
        let mut l = new_loop();
        let mut level = 80.0_f64;
        for t in 0..300u64 {
            // Random walk of ±0.5 L per cycle, clamped; a 1 in 150 chance of a 10 L drain.
            level = (level + (unit(next()) - 0.5) * 1.0).clamp(40.0, 120.0);
            if next() % 150 == 0 {
                level -= 10.0;
            }
            let maybe = |drop: u64, v: f64| {
                if drop % 11 == 0 { None } else { sample(v, t) }
            };
            let inputs = Inputs {
                now_s: t,
                moisture: maybe(next(), unit(next()) * 1.2 - 0.1),
                reservoir_l: maybe(next(), level),
                headroom_w: maybe(next(), 1000.0 + unit(next()) * 3000.0),
            };
            if l.is_latched() && next() % 4 == 0 {
                l.reset();
            }
            let out = l.step(&inputs);
            match out {
                Outcome::Applied(cmd) => {
                    applied_cycles += 1;
                    let lvl = inputs.reservoir_l.map(|s| s.value).unwrap_or(0.0);
                    let headroom = inputs.headroom_w.map(|s| s.value).unwrap_or(0.0);
                    assert!(lvl > 50.0, "applied with reservoir {lvl} at t={t}");
                    assert!(
                        headroom > 2000.0,
                        "applied with headroom {headroom} at t={t}"
                    );
                    assert!(
                        cmd.pump_duty_frac <= 0.15,
                        "pump {} at t={t}",
                        cmd.pump_duty_frac
                    );
                    assert!(
                        cmd.valve_open_s <= 900.0,
                        "valve {} at t={t}",
                        cmd.valve_open_s
                    );
                    assert!(
                        cmd.pump_duty_frac <= common::test_site().irrigation.pump_duty_frac + 1e-12
                    );
                    assert_eq!(last_hardware_command(&l), Some(cmd));
                }
                Outcome::Latched { .. } => {
                    latches += 1;
                    safe_cycles += 1;
                    assert_eq!(
                        last_hardware_command(&l).unwrap_or(Command::SAFE),
                        Command::SAFE,
                        "hardware not safe while latched at t={t}"
                    );
                }
                Outcome::Safe { .. } => {
                    safe_cycles += 1;
                    assert_eq!(
                        last_hardware_command(&l).unwrap_or(Command::SAFE),
                        Command::SAFE,
                        "hardware not safe after {out:?} at t={t}"
                    );
                }
            }
        }
    }
    assert!(
        applied_cycles > 1000,
        "only {applied_cycles} applied cycles: test proves little"
    );
    assert!(
        safe_cycles > 1000,
        "only {safe_cycles} safe cycles: test proves little"
    );
    assert!(
        latches > 10,
        "only {latches} latched cycles: drain path barely exercised"
    );
}
