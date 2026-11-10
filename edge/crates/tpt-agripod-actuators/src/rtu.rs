// SPDX-License-Identifier: MIT OR Apache-2.0

//! Modbus RTU frames: request encoding and response decoding. Pure functions over byte slices.
//!
//! Frame: unit (1 byte), function (1), data (n), CRC-16 (2, low byte first).

use crate::crc::crc16;

/// Largest Modbus RTU frame on the wire, bytes.
pub const MAX_FRAME: usize = 256;
/// Read Holding Registers.
pub const FC_READ_HOLDING: u8 = 0x03;
/// Write Single Coil.
pub const FC_WRITE_SINGLE_COIL: u8 = 0x05;
/// Write Single Register.
pub const FC_WRITE_SINGLE_REGISTER: u8 = 0x06;
/// Set on the function code of an exception response.
pub const EXCEPTION_FLAG: u8 = 0x80;
/// Coil value that switches a coil on in Write Single Coil.
pub const COIL_ON: u16 = 0xFF00;
/// Coil value that switches a coil off in Write Single Coil.
pub const COIL_OFF: u16 = 0x0000;
/// Most registers one Read Holding Registers request may ask for.
pub const MAX_READ_REGISTERS: u16 = 125;

/// Errors from encoding or decoding a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtuError {
    /// The output buffer is too small for the frame.
    BufferTooSmall,
    /// A request argument is outside the Modbus limits.
    BadArgument(&'static str),
    /// The frame is shorter or longer than its function code requires.
    Length,
    /// CRC does not match the frame contents.
    BadCrc,
    /// The response came from a different unit address.
    WrongUnit,
    /// The response carries a different function code.
    WrongFunction,
    /// The slave returned a Modbus exception with this code.
    Exception(u8),
    /// A byte count does not match the request.
    Mismatch(&'static str),
}

/// A decoded, CRC-checked response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response<'a> {
    /// Register values as raw big-endian bytes, `2 × quantity` long.
    Registers(&'a [u8]),
    /// The slave echoed the request (write functions).
    Echo,
}

/// Encodes Read Holding Registers. Returns the frame length.
pub fn encode_read_holding(
    unit: u8,
    start: u16,
    quantity: u16,
    out: &mut [u8],
) -> Result<usize, RtuError> {
    if quantity == 0 || quantity > MAX_READ_REGISTERS {
        return Err(RtuError::BadArgument("quantity must be 1 to 125"));
    }
    let body = [
        unit,
        FC_READ_HOLDING,
        (start >> 8) as u8,
        start as u8,
        (quantity >> 8) as u8,
        quantity as u8,
    ];
    finish(&body, out)
}

/// Encodes Write Single Register. Returns the frame length.
pub fn encode_write_single_register(
    unit: u8,
    register: u16,
    value: u16,
    out: &mut [u8],
) -> Result<usize, RtuError> {
    let body = [
        unit,
        FC_WRITE_SINGLE_REGISTER,
        (register >> 8) as u8,
        register as u8,
        (value >> 8) as u8,
        value as u8,
    ];
    finish(&body, out)
}

/// Encodes Write Single Coil. `on` maps to `0xFF00`, off to `0x0000`. Returns the frame length.
pub fn encode_write_single_coil(
    unit: u8,
    coil: u16,
    on: bool,
    out: &mut [u8],
) -> Result<usize, RtuError> {
    let value = if on { COIL_ON } else { COIL_OFF };
    let body = [
        unit,
        FC_WRITE_SINGLE_COIL,
        (coil >> 8) as u8,
        coil as u8,
        (value >> 8) as u8,
        value as u8,
    ];
    finish(&body, out)
}

/// Length of a normal (non-exception) response for `function` with `quantity` registers.
/// Read responses are `5 + 2n` bytes. Write responses echo the request, 8 bytes.
pub fn expected_response_len(function: u8, quantity: u16) -> usize {
    match function {
        FC_READ_HOLDING => 5 + 2 * usize::from(quantity),
        _ => 8,
    }
}

/// Checks and decodes a response to a request for `unit`, `function`, `quantity`.
pub fn decode_response(
    unit: u8,
    function: u8,
    quantity: u16,
    frame: &[u8],
) -> Result<Response<'_>, RtuError> {
    if frame.len() < 5 {
        return Err(RtuError::Length);
    }
    let (body, crc_bytes) = frame.split_at(frame.len() - 2);
    let crc = u16::from(crc_bytes[0]) | (u16::from(crc_bytes[1]) << 8);
    if crc16(body) != crc {
        return Err(RtuError::BadCrc);
    }
    if body[0] != unit {
        return Err(RtuError::WrongUnit);
    }
    if body[1] == function | EXCEPTION_FLAG {
        if frame.len() != 5 {
            return Err(RtuError::Length);
        }
        return Err(RtuError::Exception(body[2]));
    }
    if body[1] != function {
        return Err(RtuError::WrongFunction);
    }
    match function {
        FC_READ_HOLDING => {
            if frame.len() != expected_response_len(function, quantity) {
                return Err(RtuError::Length);
            }
            if usize::from(body[2]) != 2 * usize::from(quantity) {
                return Err(RtuError::Mismatch("byte count"));
            }
            Ok(Response::Registers(&body[3..]))
        }
        FC_WRITE_SINGLE_COIL | FC_WRITE_SINGLE_REGISTER => {
            if frame.len() != 8 {
                return Err(RtuError::Length);
            }
            Ok(Response::Echo)
        }
        _ => Err(RtuError::WrongFunction),
    }
}

/// Appends the CRC to `body` and copies the frame into `out`.
fn finish(body: &[u8], out: &mut [u8]) -> Result<usize, RtuError> {
    let len = body.len() + 2;
    if out.len() < len {
        return Err(RtuError::BufferTooSmall);
    }
    out[..body.len()].copy_from_slice(body);
    let crc = crc16(body);
    out[body.len()] = crc as u8;
    out[body.len() + 1] = (crc >> 8) as u8;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_crc(mut frame: std::vec::Vec<u8>) -> std::vec::Vec<u8> {
        let crc = crc16(&frame);
        frame.push(crc as u8);
        frame.push((crc >> 8) as u8);
        frame
    }

    #[test]
    fn encodes_read_holding_with_known_crc() {
        let mut out = [0u8; MAX_FRAME];
        let n = encode_read_holding(1, 0, 10, &mut out).unwrap();
        assert_eq!(&out[..n], &[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]);
    }

    #[test]
    fn encodes_write_single_coil_on_and_off() {
        let mut out = [0u8; 8];
        let n = encode_write_single_coil(2, 5, true, &mut out).unwrap();
        assert_eq!(&out[..6], &[0x02, 0x05, 0x00, 0x05, 0xFF, 0x00]);
        assert_eq!(n, 8);
        encode_write_single_coil(2, 5, false, &mut out).unwrap();
        assert_eq!(&out[4..6], &[0x00, 0x00]);
    }

    #[test]
    fn rejects_bad_quantities_and_small_buffers() {
        let mut out = [0u8; MAX_FRAME];
        assert!(encode_read_holding(1, 0, 0, &mut out).is_err());
        assert!(encode_read_holding(1, 0, 126, &mut out).is_err());
        let mut tiny = [0u8; 4];
        assert_eq!(
            encode_write_single_register(1, 0, 0, &mut tiny),
            Err(RtuError::BufferTooSmall)
        );
    }

    #[test]
    fn decodes_a_read_response() {
        // Unit 1 returns two registers: 0x0102 and 0x0304.
        let frame = with_crc(std::vec![0x01, 0x03, 0x04, 0x01, 0x02, 0x03, 0x04]);
        let r = decode_response(1, FC_READ_HOLDING, 2, &frame).unwrap();
        assert_eq!(r, Response::Registers(&[0x01, 0x02, 0x03, 0x04]));
    }

    #[test]
    fn decodes_exceptions_and_reports_the_code() {
        let frame = with_crc(std::vec![0x01, 0x83, 0x02]);
        assert_eq!(
            decode_response(1, FC_READ_HOLDING, 1, &frame),
            Err(RtuError::Exception(2))
        );
    }

    #[test]
    fn rejects_corrupt_and_mismatched_frames() {
        let good = with_crc(std::vec![0x01, 0x06, 0x00, 0x01, 0x00, 0x03]);
        assert_eq!(
            decode_response(1, FC_WRITE_SINGLE_REGISTER, 0, &good),
            Ok(Response::Echo)
        );
        let mut bad_crc = good.clone();
        bad_crc[7] ^= 0xFF;
        assert_eq!(
            decode_response(1, FC_WRITE_SINGLE_REGISTER, 0, &bad_crc),
            Err(RtuError::BadCrc)
        );
        assert_eq!(
            decode_response(9, FC_WRITE_SINGLE_REGISTER, 0, &good),
            Err(RtuError::WrongUnit)
        );
        assert_eq!(
            decode_response(1, FC_WRITE_SINGLE_COIL, 0, &good),
            Err(RtuError::WrongFunction)
        );
        assert_eq!(
            decode_response(1, FC_READ_HOLDING, 1, &good[..4]),
            Err(RtuError::Length)
        );
    }
}
