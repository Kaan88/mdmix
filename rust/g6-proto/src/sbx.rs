//! SBX effect switches and levels (Python: `g6_spec/sbx.py`).

use crate::packet::{HidReport, ProtoError, err, report};
use crate::types::{
    MODE_COMMIT, MODE_DATA, PLAYBACK_INTERMEDIATE, SMART_VOLUME_SPECIAL_ID, SbxEffect,
    SmartVolumeSpecial, ZERO4, slider_percent_bytes,
};

fn param(id: u8, value: [u8; 4]) -> Vec<HidReport> {
    vec![
        report(MODE_DATA, PLAYBACK_INTERMEDIATE, id, value),
        report(MODE_COMMIT, PLAYBACK_INTERMEDIATE, id, ZERO4),
    ]
}

pub fn sbx_toggle(effect: SbxEffect, activate: bool) -> Vec<HidReport> {
    let value = slider_percent_bytes(if activate { 100 } else { 0 }).expect("in table");
    param(effect.ids().0, value)
}

pub fn sbx_slider(effect: SbxEffect, value: i64) -> Result<Vec<HidReport>, ProtoError> {
    if !(0..=100).contains(&value) {
        return err(format!(
            "Slider value must be between 0 and 100, got {value}"
        ));
    }
    let bytes = slider_percent_bytes(value).expect("table covers 0..=100");
    Ok(param(effect.ids().1, bytes))
}

pub fn sbx_smart_volume_special(mode: SmartVolumeSpecial) -> Vec<HidReport> {
    param(SMART_VOLUME_SPECIAL_ID, mode.value())
}
