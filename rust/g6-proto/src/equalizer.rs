//! Graphic equalizer: on/off, preamp and ten bands, each a float32 parameter (Python: `g6_spec/equalizer.py`).

use crate::packet::{HidReport, ProtoError, err, report};
use crate::pyfmt::{py_g, py_str_repr};
use crate::types::{MODE_COMMIT, MODE_DATA, PLAYBACK_INTERMEDIATE, ZERO4};

pub const EQ_ENABLE_ID: u8 = 0x09;
pub const EQ_PREAMP_ID: u8 = 0x0a;
/// The bands as SBC labels them.
pub const EQ_BAND_FREQS: [&str; 10] = [
    "31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k",
];
/// Band ids, in the order of [`EQ_BAND_FREQS`].
pub const EQ_BAND_IDS: [u8; 10] = [0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14];
pub const EQ_DB_MIN: f64 = -12.0;
pub const EQ_DB_MAX: f64 = 12.0;

fn param(id: u8, value: f64) -> Vec<HidReport> {
    // Python packs the double with struct '<f', which rounds to the nearest float32
    let bytes = (value as f32).to_le_bytes();
    vec![
        report(MODE_DATA, PLAYBACK_INTERMEDIATE, id, bytes),
        report(MODE_COMMIT, PLAYBACK_INTERMEDIATE, id, ZERO4),
    ]
}

fn check_db(db: f64) -> Result<f64, ProtoError> {
    // NaN is outside the range too, as in Python
    if !(EQ_DB_MIN..=EQ_DB_MAX).contains(&db) {
        return err(format!(
            "EQ gain must be between {} and {} dB, got {}",
            py_g(EQ_DB_MIN),
            py_g(EQ_DB_MAX),
            py_g(db)
        ));
    }
    Ok(db)
}

pub fn equalizer_enabled(enable: bool) -> Vec<HidReport> {
    param(EQ_ENABLE_ID, if enable { 1.0 } else { 0.0 })
}

pub fn equalizer_preamp(db: f64) -> Result<Vec<HidReport>, ProtoError> {
    Ok(param(EQ_PREAMP_ID, check_db(db)?))
}

pub fn band_id(freq: &str) -> Option<u8> {
    EQ_BAND_FREQS
        .iter()
        .position(|f| *f == freq)
        .map(|i| EQ_BAND_IDS[i])
}

pub fn equalizer_band(freq: &str, db: f64) -> Result<Vec<HidReport>, ProtoError> {
    let Some(id) = band_id(freq) else {
        return err(format!(
            "unknown EQ band {}, expected one of {}",
            py_str_repr(freq),
            EQ_BAND_FREQS.join(", ")
        ));
    };
    Ok(param(id, check_db(db)?))
}
