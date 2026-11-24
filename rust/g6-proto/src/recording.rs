//! Microphone: mute, volumes, boost and the voice clarity effects (Python: `g6_spec/recording.py`).

use crate::packet::{HidReport, ProtoError, UsbRequest, report};
use crate::types::{
    Channels, MIC_BOOST_VR, MODE_COMMIT, MODE_DATA, MONITORING_EXTERNAL_MIC, MUTE_W_VALUE,
    MicEqPreset, NOISE_REDUCTION_VR, RECORDING_EXTERNAL_MIC, RECORDING_INTERMEDIATE, VOLUME_VR,
    ZERO4, check_range, mic_boost_bytes, mic_recording_volume_bytes, monitoring_volume_bytes,
    slider_percent_bytes, voice_clarity_level_bytes,
};

pub const VOICE_CLARITY_ID: u8 = 0x04;
pub const NOISE_REDUCTION_LEVEL_ID: u8 = 0x05;
pub const AEC_ID: u8 = 0x00;
pub const SMART_VOLUME_ID: u8 = 0x2c;
pub const MIC_EQ_ID: u8 = 0x13;
/// The eight mic EQ bands, in the order a preset writes them.
pub const MIC_EQ_BAND_IDS: [u8; 8] = [0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b];

pub fn recording_mute(mute: bool) -> Vec<UsbRequest> {
    vec![UsbRequest::set_cur(
        MUTE_W_VALUE,
        RECORDING_EXTERNAL_MIC,
        &[u8::from(mute)],
    )]
}

pub fn mic_recording_volume(
    percent: i64,
    channels: Channels,
) -> Result<Vec<UsbRequest>, ProtoError> {
    check_range(
        percent,
        VOLUME_VR,
        "Mic recording volume must be between 0 and 100",
        "Mic recording volume must be a multiple of 10",
    )?;
    let bytes = mic_recording_volume_bytes(percent).expect("table covers every step");
    Ok(channels
        .in_order()
        .iter()
        .map(|c| UsbRequest::set_cur(c.w_value(), RECORDING_EXTERNAL_MIC, &bytes))
        .collect())
}

pub fn mic_boost(db: i64) -> Result<Vec<HidReport>, ProtoError> {
    check_range(
        db,
        MIC_BOOST_VR,
        "Mic boost must be between 0 and 30 dB",
        "Mic boost must be a multiple of 10 dB",
    )?;
    let value = mic_boost_bytes(db).expect("table covers every step");
    Ok(vec![
        report([0x3c, 0x04], [0x00, 0x00], 0x02, value),
        report([0x3c, 0x02], [0x01, 0x00], 0x00, ZERO4),
    ])
}

pub fn mic_monitoring_mute(mute: bool) -> Vec<UsbRequest> {
    vec![UsbRequest::set_cur(
        MUTE_W_VALUE,
        MONITORING_EXTERNAL_MIC,
        &[u8::from(mute)],
    )]
}

pub fn mic_monitoring_volume(
    percent: i64,
    channels: Channels,
) -> Result<Vec<UsbRequest>, ProtoError> {
    check_range(
        percent,
        VOLUME_VR,
        "Mic monitoring level must be between 0 and 100",
        "Mic monitoring level must be a multiple of 10",
    )?;
    let bytes = monitoring_volume_bytes(percent).expect("table covers every step");
    Ok(channels
        .in_order()
        .iter()
        .map(|c| UsbRequest::set_cur(c.w_value(), MONITORING_EXTERNAL_MIC, &bytes))
        .collect())
}

fn toggle(feature: u8, enable: bool) -> Vec<HidReport> {
    let on = slider_percent_bytes(100).expect("in table");
    let off = slider_percent_bytes(0).expect("in table");
    vec![
        report(
            MODE_DATA,
            RECORDING_INTERMEDIATE,
            feature,
            if enable { on } else { off },
        ),
        report(MODE_COMMIT, RECORDING_INTERMEDIATE, feature, off),
    ]
}

/// SBC calls this one "Noise Reduction".
pub fn voice_clarity_enabled(enable: bool) -> Vec<HidReport> {
    toggle(VOICE_CLARITY_ID, enable)
}

pub fn voice_clarity_noise_reduction_level(percent: i64) -> Result<Vec<HidReport>, ProtoError> {
    check_range(
        percent,
        NOISE_REDUCTION_VR,
        "Voice clarity level must be between 0 and 100",
        "Voice clarity level must be a multiple of 20",
    )?;
    let value = voice_clarity_level_bytes(percent).expect("table covers every step");
    Ok(vec![
        report(
            MODE_DATA,
            RECORDING_INTERMEDIATE,
            NOISE_REDUCTION_LEVEL_ID,
            value,
        ),
        report(
            MODE_COMMIT,
            RECORDING_INTERMEDIATE,
            NOISE_REDUCTION_LEVEL_ID,
            ZERO4,
        ),
    ])
}

pub fn voice_clarity_acoustic_echo_cancellation_enabled(enable: bool) -> Vec<HidReport> {
    toggle(AEC_ID, enable)
}

pub fn voice_clarity_smart_volume_enabled(enable: bool) -> Vec<HidReport> {
    toggle(SMART_VOLUME_ID, enable)
}

pub fn voice_clarity_mic_equalizer_enabled(enable: bool) -> Vec<HidReport> {
    toggle(MIC_EQ_ID, enable)
}

pub fn voice_clarity_mic_equalizer_preset(preset: MicEqPreset) -> Vec<HidReport> {
    MIC_EQ_BAND_IDS
        .iter()
        .zip(preset.values())
        .flat_map(|(&id, &value)| {
            [
                report(MODE_DATA, RECORDING_INTERMEDIATE, id, value),
                report(MODE_COMMIT, RECORDING_INTERMEDIATE, id, ZERO4),
            ]
        })
        .collect()
}
