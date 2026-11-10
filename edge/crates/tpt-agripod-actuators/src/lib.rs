// SPDX-License-Identifier: MIT OR Apache-2.0

//! Actuator hardware abstraction layer: Modbus RTU over RS-485.
//!
//! `no_std`, no allocation. The framer ([`rtu`]) and CRC ([`crc`]) are pure functions over
//! byte slices. A [`transport::Transport`] supplies bytes and timeouts, so the same master
//! runs on a Linux serial port, an ESP32 UART, or the test mock.
//!
//! Device abstractions (solenoid, pump, VFD, LED driver, moisture sensor) need register maps
//! from the device datasheets and are not written yet.

#![no_std]

#[cfg(any(test, feature = "mock"))]
extern crate std;

pub mod crc;
pub mod master;
pub mod rtu;
pub mod transport;

#[cfg(any(test, feature = "mock"))]
pub mod mock;
