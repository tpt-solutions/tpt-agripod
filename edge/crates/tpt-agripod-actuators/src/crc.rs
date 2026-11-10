// SPDX-License-Identifier: MIT OR Apache-2.0

//! CRC-16/MODBUS: reflected polynomial 0xA001, initial value 0xFFFF, no final XOR.
//! On the wire the low byte is sent first.

/// CRC-16/MODBUS of `data`.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_published_vectors() {
        // Read Holding Registers, unit 1, start 0, quantity 10. CRC bytes on the wire: C5 CD.
        assert_eq!(crc16(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]), 0xCDC5);
        // Read Holding Registers, unit 1, start 0, quantity 1. CRC bytes on the wire: 84 0A.
        assert_eq!(crc16(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x01]), 0x0A84);
    }

    #[test]
    fn appending_the_crc_gives_zero_residue() {
        // Property: data followed by its own CRC (low byte first) checks to zero.
        let data = [0x11, 0x06, 0x00, 0x01, 0x00, 0x03, 0x9A, 0x9B, 0x12];
        let c = crc16(&data);
        let mut framed = [0u8; 11];
        framed[..9].copy_from_slice(&data);
        framed[9] = (c & 0xFF) as u8;
        framed[10] = (c >> 8) as u8;
        assert_eq!(crc16(&framed), 0);
    }
}
