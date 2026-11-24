//! Monitoring and recording mixer: a mute and a volume per source (Python: `g6_spec/mixer.py`).

use crate::packet::{ProtoError, UsbRequest};
use crate::types::{
    Channels, MUTE_W_VALUE, MixerSource, PLAYBACK_PLAYBACK, VOLUME_VR, check_range,
    mic_recording_volume_bytes, monitoring_volume_bytes,
};

/// The mixer's playback mute: the same request as `playback::playback_mute`.
pub fn playback_mute(mute: bool) -> Vec<UsbRequest> {
    vec![UsbRequest::set_cur(
        MUTE_W_VALUE,
        PLAYBACK_PLAYBACK,
        &[u8::from(mute)],
    )]
}

pub fn mute(source: MixerSource, mute: bool) -> Vec<UsbRequest> {
    vec![UsbRequest::set_cur(
        MUTE_W_VALUE,
        source.w_index(),
        &[u8::from(mute)],
    )]
}

pub fn volume(
    source: MixerSource,
    percent: i64,
    channels: Channels,
) -> Result<Vec<UsbRequest>, ProtoError> {
    check_range(
        percent,
        VOLUME_VR,
        "Volume percentage must be between 0 and 100",
        "Volume percentage must be a multiple of 10",
    )?;
    // monitoring and recording use different dB curves
    let bytes = if source.is_monitoring() {
        monitoring_volume_bytes(percent)
    } else {
        mic_recording_volume_bytes(percent)
    }
    .expect("table covers every step");
    Ok(channels
        .in_order()
        .iter()
        .map(|c| UsbRequest::set_cur(c.w_value(), source.w_index(), &bytes))
        .collect())
}
