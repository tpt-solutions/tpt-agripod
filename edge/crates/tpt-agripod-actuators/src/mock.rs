// SPDX-License-Identifier: MIT OR Apache-2.0

//! In-memory transport for host tests. Records what the master wrote and replays scripted
//! bytes. A read that finds too few bytes times out, as a silent bus would.

extern crate std;

use std::collections::VecDeque;
use std::vec::Vec;

use crate::transport::{IoError, Transport};

/// Scripted transport. Feed it the bytes a slave would send, then run the master.
#[derive(Debug, Default)]
pub struct MockTransport {
    /// Every request the master wrote, one entry per `write_all`.
    pub written: Vec<Vec<u8>>,
    rx: VecDeque<u8>,
    /// When set, the next `write_all` fails with [`IoError::Fault`].
    pub fail_next_write: bool,
}

impl MockTransport {
    /// Creates an empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues bytes the next reads will return.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.rx.extend(bytes.iter().copied());
    }
}

impl Transport for MockTransport {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        if self.fail_next_write {
            self.fail_next_write = false;
            return Err(IoError::Fault);
        }
        self.written.push(bytes.to_vec());
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8], _timeout_ms: u32) -> Result<(), IoError> {
        if self.rx.len() < buf.len() {
            self.rx.clear();
            return Err(IoError::Timeout);
        }
        for slot in buf.iter_mut() {
            *slot = self.rx.pop_front().ok_or(IoError::Timeout)?;
        }
        Ok(())
    }
}
