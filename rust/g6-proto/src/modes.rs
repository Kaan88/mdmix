//! Processing modes: the SBX master switch and Scout Mode (Python: `g6_spec/modes.py`).

use crate::packet::{HidReport, report};
use crate::types::{ProcessingMode, ZERO4};

/// Bits of the mode register, byte 6 of the answer to [`MODE_QUERY`].
pub const MODE_BIT_SBX: u8 = 0x01;
pub const MODE_BIT_SCOUT: u8 = 0x02;
/// Follows the recording effects; it has no write of its own.
pub const MODE_BIT_VOICE_CLARITY: u8 = 0x04;

pub const MODE_QUERY: [u8; 6] = [0x5a, 0x26, 0x03, 0x08, 0xff, 0xff];

/// The write, then the register read SBC sends after it (the same bytes as [`MODE_QUERY`]).
pub fn processing_mode(mode: ProcessingMode, enable: bool) -> Vec<HidReport> {
    vec![
        report(
            [0x26, 0x05],
            [0x07, mode as u8],
            0x00,
            [u8::from(enable), 0, 0, 0],
        ),
        report([0x26, 0x03], [0x08, 0xff], 0xff, ZERO4),
    ]
}
