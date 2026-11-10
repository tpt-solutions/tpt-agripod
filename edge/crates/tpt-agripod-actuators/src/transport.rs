// SPDX-License-Identifier: MIT OR Apache-2.0

//! The byte-level link the master runs over. An implementation owns the serial port (or
//! UART), the RS-485 direction control, and the timing.

/// Failure of the link itself, below Modbus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoError {
    /// The expected bytes did not arrive within the timeout.
    Timeout,
    /// The link failed (port closed, framing error, overrun).
    Fault,
}

/// A byte link. Implementations must put the whole request on the wire before returning from
/// `write_all`, and must return `Timeout` rather than block past `timeout_ms`.
pub trait Transport {
    /// Writes all of `bytes`, including turning the bus around for transmit.
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoError>;

    /// Fills `buf` completely, or fails once `timeout_ms` has passed.
    fn read_exact(&mut self, buf: &mut [u8], timeout_ms: u32) -> Result<(), IoError>;
}
