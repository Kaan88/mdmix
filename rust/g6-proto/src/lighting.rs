//! RGB light and volume ring (Python: `g6_spec/lighting.py`).

use crate::packet::{HidReport, ProtoError, err, report, report_with_payload};
use crate::types::ZERO4;

pub fn lighting_disable() -> Vec<HidReport> {
    vec![report([0x3a, 0x02], [0x06, 0x00], 0x00, ZERO4)]
}

/// The LED ring under the volume knob, independent of the RGB light. The device takes 0x00 for on, 0x01 for off.
pub fn lighting_volume_ring(enable: bool) -> Vec<HidReport> {
    vec![
        report(
            [0x39, 0x03],
            [0x00, 0x0e],
            if enable { 0x00 } else { 0x01 },
            ZERO4,
        ),
        report([0x39, 0x01], [0x01, 0x00], 0x00, ZERO4),
    ]
}

fn check_byte(name: &str, v: i64) -> Result<u8, ProtoError> {
    u8::try_from(v).or_else(|_| err(format!("{name} must be between 0 and 255, got {v}")))
}

fn pre_1() -> HidReport {
    report([0x3a, 0x02], [0x06, 0x01], 0x00, ZERO4)
}

fn pre_2() -> HidReport {
    report([0x3a, 0x06], [0x04, 0x00], 0x03, [0x01, 0x00, 0x01, 0x00])
}

/// `5a 3a09 0a00 03 01 01 <brightness> <blue> <green> <red>`
fn colour(r: u8, g: u8, b: u8, brightness: u8) -> HidReport {
    report_with_payload(
        [0x3a, 0x09],
        [0x0a, 0x00],
        0x03,
        [0x01, 0x01, brightness, b],
        &[g, r],
    )
}

/// Turns the light on in this colour: the same three reports three times, at full brightness, as SBC does.
pub fn lighting_enable_set_rgb(
    red: i64,
    green: i64,
    blue: i64,
) -> Result<Vec<HidReport>, ProtoError> {
    let r = check_byte("red", red)?;
    let g = check_byte("green", green)?;
    let b = check_byte("blue", blue)?;
    Ok((0..3)
        .flat_map(|_| [pre_1(), pre_2(), colour(r, g, b, 0xff)])
        .collect())
}

/// One animation frame: the first two reports of [`lighting_enable_set_rgb`] and the colour at this brightness.
pub fn lighting_frame(
    red: i64,
    green: i64,
    blue: i64,
    brightness: i64,
) -> Result<Vec<HidReport>, ProtoError> {
    let r = check_byte("red", red)?;
    let g = check_byte("green", green)?;
    let b = check_byte("blue", blue)?;
    let br = check_byte("brightness", brightness)?;
    Ok(vec![pre_1(), pre_2(), colour(r, g, b, br)])
}
