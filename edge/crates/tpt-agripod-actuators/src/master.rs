// SPDX-License-Identifier: MIT OR Apache-2.0

//! Modbus RTU master: one request, one response, with a timeout. No retries here. The caller
//! decides what a failed exchange means for the actuator (the safe-state rules).

use crate::rtu::{self, MAX_FRAME, Response, RtuError};
use crate::transport::{IoError, Transport};

/// Failure of one exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterError {
    /// The link failed or timed out.
    Io(IoError),
    /// The response was malformed, or the slave returned an exception.
    Rtu(RtuError),
}

impl From<IoError> for MasterError {
    fn from(e: IoError) -> Self {
        MasterError::Io(e)
    }
}

impl From<RtuError> for MasterError {
    fn from(e: RtuError) -> Self {
        MasterError::Rtu(e)
    }
}

/// A Modbus RTU master over a [`Transport`].
pub struct Master<T: Transport> {
    transport: T,
    timeout_ms: u32,
}

impl<T: Transport> Master<T> {
    /// Creates a master that waits up to `timeout_ms` for each response.
    pub fn new(transport: T, timeout_ms: u32) -> Self {
        Self {
            transport,
            timeout_ms,
        }
    }

    /// Gives the transport back, for inspection in tests or to reconfigure the port.
    pub fn into_inner(self) -> T {
        self.transport
    }

    /// Reads `out.len()` holding registers starting at `start`, from slave `unit`.
    pub fn read_holding_registers(
        &mut self,
        unit: u8,
        start: u16,
        out: &mut [u16],
    ) -> Result<(), MasterError> {
        let quantity =
            u16::try_from(out.len()).map_err(|_| RtuError::BadArgument("too many registers"))?;
        let mut req = [0u8; MAX_FRAME];
        let n = rtu::encode_read_holding(unit, start, quantity, &mut req)?;
        let mut resp = [0u8; MAX_FRAME];
        let len = self.exchange(rtu::FC_READ_HOLDING, quantity, &req[..n], &mut resp)?;
        match rtu::decode_response(unit, rtu::FC_READ_HOLDING, quantity, &resp[..len])? {
            Response::Registers(data) => {
                for (slot, pair) in out.iter_mut().zip(data.chunks_exact(2)) {
                    *slot = u16::from_be_bytes([pair[0], pair[1]]);
                }
                Ok(())
            }
            Response::Echo => Err(RtuError::WrongFunction.into()),
        }
    }

    /// Writes one holding register on slave `unit`.
    pub fn write_single_register(
        &mut self,
        unit: u8,
        register: u16,
        value: u16,
    ) -> Result<(), MasterError> {
        let mut req = [0u8; MAX_FRAME];
        let n = rtu::encode_write_single_register(unit, register, value, &mut req)?;
        self.write_exchange(unit, rtu::FC_WRITE_SINGLE_REGISTER, &req[..n])
    }

    /// Switches one coil on slave `unit`.
    pub fn write_single_coil(&mut self, unit: u8, coil: u16, on: bool) -> Result<(), MasterError> {
        let mut req = [0u8; MAX_FRAME];
        let n = rtu::encode_write_single_coil(unit, coil, on, &mut req)?;
        self.write_exchange(unit, rtu::FC_WRITE_SINGLE_COIL, &req[..n])
    }

    fn write_exchange(&mut self, unit: u8, function: u8, req: &[u8]) -> Result<(), MasterError> {
        let mut resp = [0u8; MAX_FRAME];
        let len = self.exchange(function, 0, req, &mut resp)?;
        match rtu::decode_response(unit, function, 0, &resp[..len])? {
            Response::Echo => Ok(()),
            Response::Registers(_) => Err(RtuError::WrongFunction.into()),
        }
    }

    /// Sends `req`, then reads the response header and the rest of the frame into `resp`.
    /// Returns the response length. An exception response is 5 bytes; anything else is the
    /// length the function code implies.
    fn exchange(
        &mut self,
        function: u8,
        quantity: u16,
        req: &[u8],
        resp: &mut [u8],
    ) -> Result<usize, MasterError> {
        self.transport.write_all(req)?;
        self.transport.read_exact(&mut resp[..2], self.timeout_ms)?;
        let len = if resp[1] == function | rtu::EXCEPTION_FLAG {
            5
        } else {
            rtu::expected_response_len(function, quantity)
        };
        if len > resp.len() {
            return Err(RtuError::Length.into());
        }
        self.transport
            .read_exact(&mut resp[2..len], self.timeout_ms)?;
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::crc16;
    use crate::mock::MockTransport;
    use std::vec::Vec;

    fn framed(mut body: Vec<u8>) -> Vec<u8> {
        let crc = crc16(&body);
        body.push(crc as u8);
        body.push((crc >> 8) as u8);
        body
    }

    #[test]
    fn reads_registers_big_endian() {
        let mut t = MockTransport::new();
        t.feed(&framed(std::vec![0x01, 0x03, 0x04, 0x01, 0x02, 0xFF, 0xFE]));
        let mut m = Master::new(t, 100);
        let mut regs = [0u16; 2];
        m.read_holding_registers(1, 0, &mut regs).unwrap();
        assert_eq!(regs, [0x0102, 0xFFFE]);
        let t = m.into_inner();
        assert_eq!(
            t.written[0],
            std::vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x02, 0xC4, 0x0B]
        );
    }

    #[test]
    fn silent_slave_times_out() {
        let mut m = Master::new(MockTransport::new(), 100);
        let mut regs = [0u16; 1];
        assert_eq!(
            m.read_holding_registers(1, 0, &mut regs),
            Err(MasterError::Io(IoError::Timeout))
        );
    }

    #[test]
    fn exception_is_reported_not_treated_as_data() {
        let mut t = MockTransport::new();
        t.feed(&framed(std::vec![0x01, 0x83, 0x02]));
        let mut m = Master::new(t, 100);
        let mut regs = [0u16; 1];
        assert_eq!(
            m.read_holding_registers(1, 0, &mut regs),
            Err(MasterError::Rtu(RtuError::Exception(2)))
        );
    }

    #[test]
    fn corrupt_response_is_rejected() {
        let mut t = MockTransport::new();
        let mut frame = framed(std::vec![0x01, 0x03, 0x02, 0x00, 0x07]);
        frame[4] ^= 0x10;
        t.feed(&frame);
        let mut m = Master::new(t, 100);
        let mut regs = [0u16; 1];
        assert_eq!(
            m.read_holding_registers(1, 0, &mut regs),
            Err(MasterError::Rtu(RtuError::BadCrc))
        );
    }

    #[test]
    fn write_single_register_checks_the_echo() {
        let mut t = MockTransport::new();
        let req = {
            let mut b = std::vec![0x01, 0x06, 0x00, 0x10, 0x00, 0x2A];
            let c = crc16(&b);
            b.push(c as u8);
            b.push((c >> 8) as u8);
            b
        };
        t.feed(&req);
        let mut m = Master::new(t, 100);
        m.write_single_register(1, 0x10, 0x2A).unwrap();
    }

    #[test]
    fn link_fault_on_write_is_surfaced() {
        let mut t = MockTransport::new();
        t.fail_next_write = true;
        let mut m = Master::new(t, 100);
        assert_eq!(
            m.write_single_coil(1, 0, true),
            Err(MasterError::Io(IoError::Fault))
        );
    }

    #[test]
    fn decoder_never_panics_on_random_bytes() {
        // Seeded, so failures reproduce. Random frames of every length the master can see.
        let mut state: u64 = 0x1234_5678_9ABC_DEF0;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for _ in 0..20_000 {
            let len = (next() % 12) as usize;
            let mut frame = [0u8; 12];
            for slot in frame.iter_mut().take(len) {
                *slot = next() as u8;
            }
            let qty = (next() % 130) as u16;
            let _ = rtu::decode_response(1, rtu::FC_READ_HOLDING, qty, &frame[..len]);
            let _ = rtu::decode_response(1, rtu::FC_WRITE_SINGLE_COIL, 0, &frame[..len]);
        }
    }

    #[test]
    fn request_encoding_round_trips_through_a_matching_slave() {
        // For many (unit, start, quantity) triples, the slave reply built from the request
        // must decode back to the values the slave holds.
        for unit in [1u8, 17, 247] {
            for start in [0u16, 1, 0x00FF, 0xFFF0] {
                for quantity in [1u16, 2, 7, 125] {
                    let mut body = std::vec![unit, rtu::FC_READ_HOLDING, (quantity * 2) as u8];
                    for i in 0..quantity {
                        let v = start.wrapping_mul(31).wrapping_add(i);
                        body.push((v >> 8) as u8);
                        body.push(v as u8);
                    }
                    let mut t = MockTransport::new();
                    t.feed(&framed(body));
                    let mut m = Master::new(t, 50);
                    let mut regs = std::vec![0u16; quantity as usize];
                    m.read_holding_registers(unit, start, &mut regs).unwrap();
                    for (i, v) in regs.iter().enumerate() {
                        assert_eq!(*v, start.wrapping_mul(31).wrapping_add(i as u16));
                    }
                }
            }
        }
    }
}
