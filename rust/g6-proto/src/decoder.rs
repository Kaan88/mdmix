//! Decoder mode (Python: `g6_spec/decoder.py`).

use crate::packet::{HidReport, report};
use crate::types::{DECODER_INTERMEDIATE, DecoderMode, MODE_COMMIT, MODE_DATA, ZERO4};

pub const DECODER_FEATURE: u8 = 0x02;

pub fn decoder_mode(mode: DecoderMode) -> Vec<HidReport> {
    vec![
        report(
            MODE_DATA,
            DECODER_INTERMEDIATE,
            DECODER_FEATURE,
            mode.value(),
        ),
        report(MODE_COMMIT, DECODER_INTERMEDIATE, DECODER_FEATURE, ZERO4),
    ]
}
