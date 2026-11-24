//! Output switch, playback volume and mute, Direct mode, DAC filter (Python: `g6_spec/playback.py`).

use crate::packet::{HidReport, ProtoError, UsbRequest, err, report};
use crate::types::{
    Channels, MODE_COMMIT, MODE_DATA, MUTE_W_VALUE, PLAYBACK_INTERMEDIATE, PLAYBACK_PLAYBACK,
    PlaybackFilter, ZERO4, playback_volume_bytes,
};

const DIRECT_MODE: [u8; 2] = [0x00, 0x05];
const SPDIF_OUT_DIRECT_MODE: [u8; 2] = [0x00, 0x0d];
// SBC: Speakers settings -> "Apply Headphone Virtualization to Line Out"
const LINE_OUT_HEADPHONE_VIRTUALIZATION: [u8; 2] = [0x00, 0x0a];

pub fn playback_mute(mute: bool) -> Vec<UsbRequest> {
    vec![UsbRequest::set_cur(
        MUTE_W_VALUE,
        PLAYBACK_PLAYBACK,
        &[u8::from(mute)],
    )]
}

/// The switch itself, then the zeroed parameters SBC sends after it. The two outputs differ only in the code and
/// in an extra 0x06 pair for headphones.
fn toggle(code: u8, extra_06: bool) -> Vec<HidReport> {
    let mut v = vec![
        report([0x2c, 0x05], [0x00, code], 0x00, ZERO4),
        report([0x2c, 0x01], [0x01, 0x00], 0x00, ZERO4),
    ];
    let mut ids: Vec<u8> = (0x0a..=0x14).collect();
    ids.push(0x09);
    if extra_06 {
        ids.push(0x06);
    }
    ids.push(0x09);
    for id in ids {
        v.push(report(MODE_DATA, PLAYBACK_INTERMEDIATE, id, ZERO4));
        v.push(report(MODE_COMMIT, PLAYBACK_INTERMEDIATE, id, ZERO4));
    }
    v
}

pub fn toggle_to_speakers() -> Vec<HidReport> {
    toggle(0x02, false)
}

pub fn toggle_to_headphones() -> Vec<HidReport> {
    toggle(0x04, true)
}

/// Stereo, 5.1 and 7.1 send the same requests, for speakers and headphones alike. SBC does the same; the layout
/// itself is presumably set through the OS.
pub fn speaker_layout() -> Vec<UsbRequest> {
    Channels::Both
        .in_order()
        .iter()
        .map(|c| UsbRequest::set_cur(c.w_value(), PLAYBACK_PLAYBACK, &[0x00, 0xf4]))
        .collect()
}

pub fn playback_volume(percent: i64, channels: Channels) -> Result<Vec<UsbRequest>, ProtoError> {
    if !(0..=100).contains(&percent) {
        return err(format!(
            "Volume percentage must be between 0 and 100, got {percent}"
        ));
    }
    if percent % 10 != 0 {
        return err(format!(
            "Volume percentage must be a multiple of 10, got {percent}"
        ));
    }
    let bytes = playback_volume_bytes(percent).expect("table covers every step");
    Ok(channels
        .in_order()
        .iter()
        .map(|c| UsbRequest::set_cur(c.w_value(), PLAYBACK_PLAYBACK, &bytes))
        .collect())
}

fn switch_39(intermediate: [u8; 2], enable: bool) -> Vec<HidReport> {
    vec![
        report([0x39, 0x03], intermediate, u8::from(enable), ZERO4),
        report([0x39, 0x01], [0x01, 0x00], 0x00, ZERO4),
    ]
}

pub fn enable_direct_mode(enable: bool) -> Vec<HidReport> {
    switch_39(DIRECT_MODE, enable)
}

pub fn enable_spdif_out_direct_mode(enable: bool) -> Vec<HidReport> {
    switch_39(SPDIF_OUT_DIRECT_MODE, enable)
}

/// Only flips the device switch; SBC also re-sends its stored headphone SBX values when this changes.
pub fn enable_line_out_headphone_virtualization(enable: bool) -> Vec<HidReport> {
    switch_39(LINE_OUT_HEADPHONE_VIRTUALIZATION, enable)
}

pub fn playback_filter(filter: PlaybackFilter) -> Vec<HidReport> {
    vec![
        report([0x6c, 0x03], filter.code(), 0x00, ZERO4),
        report([0x6c, 0x01], [0x01, 0x00], 0x00, ZERO4),
    ]
}
