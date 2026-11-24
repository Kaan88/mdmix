//! Reading the device state: the HID queries and their parsers (Python: `g6_status.py`).
//!
//! [`collect_status`] sends every query through a caller-supplied function, so the same code runs against a real
//! device, a recorded transcript or a test.

use crate::equalizer::{EQ_BAND_FREQS, EQ_BAND_IDS, EQ_ENABLE_ID, EQ_PREAMP_ID};
use crate::modes::{MODE_BIT_SBX, MODE_BIT_SCOUT, MODE_BIT_VOICE_CLARITY, MODE_QUERY};
use crate::packet::HidReport;
use crate::pyfmt::{py_g, py_repr, py_round, py_round_int};
use crate::recording::{
    AEC_ID, MIC_EQ_BAND_IDS, MIC_EQ_ID, NOISE_REDUCTION_LEVEL_ID, SMART_VOLUME_ID, VOICE_CLARITY_ID,
};
use crate::types::{MicEqPreset, PlaybackFilter, SMART_VOLUME_SPECIAL_ID, SbxEffect};

pub const Q_FIRMWARE: &[u8] = &[0x5a, 0x07, 0x01, 0x02];
pub const Q_OUTPUT: &[u8] = &[0x5a, 0x2c, 0x01, 0x01];
/// Two ways to ask for the flags; the second is tried when the first gets no usable answer.
pub const Q_MODE_FLAGS: [&[u8]; 2] = [
    &[0x5a, 0x02, 0x0a, 0x39, 0x81, 0x00],
    &[0x5a, 0x39, 0x01, 0x01],
];
pub const Q_DECODER: &[u8] = &[0x5a, 0x11, 0x03, 0x01, 0x97, 0x02];
pub const Q_DAC_FILTER: &[u8] = &[0x5a, 0x6c, 0x01, 0x01];
pub const Q_RGB: [&[u8]; 2] = [
    &[0x5a, 0x02, 0x0a, 0x3a, 0x00, 0x06, 0x01, 0x00],
    &[0x5a, 0x3a, 0x02, 0x07, 0x01],
];
pub const Q_RING: [&[u8]; 2] = [
    &[0x5a, 0x02, 0x0a, 0x3a, 0x00, 0x0e, 0x01, 0x00],
    &[0x5a, 0x3a, 0x02, 0x10, 0x01],
];
pub const Q_MIC_BOOST: &[u8] = &[0x5a, 0x3c, 0x02, 0x01];

const R_MODE_FLAGS_PREFIXES: [[u8; 4]; 2] = [[0x5a, 0x39, 0x05, 0x01], [0x5a, 0x39, 0x09, 0x02]];
const R_DECODER_PREFIX: [u8; 7] = [0x5a, 0x11, 0x08, 0x01, 0x00, 0x97, 0x02];
const R_DAC_FILTER_PREFIX: [u8; 4] = [0x5a, 0x6c, 0x03, 0x01];
const R_RGB_PREFIX: [u8; 4] = [0x5a, 0x3a, 0x02, 0x07];
const R_RING_PREFIX: [u8; 4] = [0x5a, 0x3a, 0x02, 0x10];
const R_MIC_BOOST_PREFIX: [u8; 4] = [0x5a, 0x3c, 0x04, 0x01];
const R_PROCESSING_MODE_PREFIX: [u8; 6] = [0x5a, 0x26, 0x0b, 0x08, 0xff, 0xff];

pub const MIC_BOOST_DB: [u8; 4] = [0, 10, 20, 30];
/// Parameter groups: playback (SBX, EQ) and recording (voice clarity, mic EQ).
pub const PLAYBACK_PARAM: u8 = 0x96;
pub const RECORDING_PARAM: u8 = 0x95;

/// The state of one SBX effect. The level is a percentage.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SbxState {
    pub enabled: Option<bool>,
    pub level: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RecordingStatus {
    pub noise_reduction: Option<bool>,
    pub noise_reduction_level: Option<i64>,
    pub acoustic_echo_cancellation: Option<bool>,
    pub smart_volume: Option<bool>,
    pub mic_eq: Option<bool>,
}

impl RecordingStatus {
    fn any_known(&self) -> bool {
        self.noise_reduction.is_some()
            || self.noise_reduction_level.is_some()
            || self.acoustic_echo_cancellation.is_some()
            || self.smart_volume.is_some()
            || self.mic_eq.is_some()
    }
}

/// What the G6 reports. `None` means unknown: no answer, or one the parsers don't recognise. Field order is the
/// order of Python's `DeviceStatus`, which the JSON output keeps.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DeviceStatus {
    pub firmware: Option<String>,
    /// "Speakers", "Headphones" or an unknown code as "0x..".
    pub output: Option<String>,
    pub decoder_mode: Option<String>,
    /// A `PlaybackFilter` name, or an unknown code as "0x..".
    pub dac_filter: Option<String>,
    pub direct_mode: Option<bool>,
    pub spdif_direct: Option<bool>,
    pub line_out_headphone_virtualization: Option<bool>,
    pub rgb_enabled: Option<bool>,
    pub ring_enabled: Option<bool>,
    pub mode_flags: Option<(u8, u8)>,
    /// One entry per effect, in [`SbxEffect::ALL`] order.
    pub sbx: Option<[SbxState; 5]>,
    pub smart_volume_mode: Option<String>,
    pub eq_enabled: Option<bool>,
    pub eq_preamp_db: Option<f64>,
    /// In [`EQ_BAND_FREQS`] order.
    pub eq_bands_db: Option<[Option<f64>; 10]>,
    pub mic_boost_db: Option<u8>,
    pub sbx_enabled: Option<bool>,
    pub scout_mode: Option<bool>,
    pub voice_clarity: Option<bool>,
    pub recording: Option<RecordingStatus>,
    /// A preset name, or the eight gains in dB when they match no preset.
    pub mic_eq_preset: Option<String>,
}

impl DeviceStatus {
    pub fn sbx_effect(&self, effect: SbxEffect) -> Option<SbxState> {
        let i = SbxEffect::ALL.iter().position(|e| *e == effect)?;
        self.sbx.map(|s| s[i])
    }
}

/// Firmware version, e.g. "2.1.201208.1030": length at byte 2, ASCII after it.
pub fn parse_firmware(r: &HidReport) -> Option<String> {
    let n = usize::from(r[2]);
    if n == 0 {
        return None;
    }
    let bytes = &r[3..(3 + n).min(r.len())];
    if !bytes.is_ascii() {
        return None;
    }
    let s = std::str::from_utf8(bytes).ok()?.trim_end_matches('\0');
    (!s.is_empty()).then(|| s.to_string())
}

pub fn parse_output(r: &HidReport) -> String {
    match r[4] {
        2 => "Speakers".to_string(),
        4 => "Headphones".to_string(),
        c => format!("0x{c:02x}"),
    }
}

/// The two flag bytes and what is known of them. Bit 0x40 of the second byte is set while the ring is off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModeFlags {
    pub flags: (u8, u8),
    pub direct_mode: bool,
    pub spdif_direct: bool,
    pub ring_enabled: bool,
    pub line_out_headphone_virtualization: bool,
}

pub fn parse_mode_flags(r: &HidReport) -> Option<ModeFlags> {
    if !R_MODE_FLAGS_PREFIXES.iter().any(|p| r[..4] == p[..]) {
        return None;
    }
    let (a, b) = (r[4], r[5]);
    Some(ModeFlags {
        flags: (a, b),
        direct_mode: a & 0x20 != 0,
        spdif_direct: b & 0x20 != 0,
        ring_enabled: b & 0x40 == 0,
        line_out_headphone_virtualization: b & 0x04 != 0,
    })
}

pub fn parse_decoder(r: &HidReport) -> Option<String> {
    if r[..7] != R_DECODER_PREFIX {
        return None;
    }
    let raw = u32::from_le_bytes([r[7], r[8], r[9], r[10]]);
    // the mode is a float32: 1.0, 2.0 or 3.0
    Some(match raw {
        0x3f80_0000 => "Full".to_string(),
        0x4000_0000 => "Normal".to_string(),
        0x4040_0000 => "Night".to_string(),
        _ => format!("0x{raw:08x}"),
    })
}

pub fn parse_dac_filter(r: &HidReport) -> Option<String> {
    if r[..4] != R_DAC_FILTER_PREFIX {
        return None;
    }
    let code = r[4];
    Some(
        PlaybackFilter::ALL
            .into_iter()
            .find(|f| f.code()[1] == code)
            .map_or_else(|| format!("0x{code:02x}"), |f| f.name().to_string()),
    )
}

fn parse_on_off(r: &HidReport, prefix: &[u8; 4]) -> Option<bool> {
    (r[..4] == prefix[..]).then_some(r[4] == 1)
}

pub fn parse_rgb(r: &HidReport) -> Option<bool> {
    parse_on_off(r, &R_RGB_PREFIX)
}

pub fn parse_ring(r: &HidReport) -> Option<bool> {
    parse_on_off(r, &R_RING_PREFIX)
}

/// `5a 11 03 01 <group> <id>`: read one float parameter.
pub fn param_query(param_id: u8, group: u8) -> [u8; 6] {
    [0x5a, 0x11, 0x03, 0x01, group, param_id]
}

/// The answer `5a 11 08 01 00 <group> <id> <float32 LE>`, rounded to 4 decimals as Python does. NaN is unknown.
pub fn parse_param(r: &HidReport, param_id: u8, group: u8) -> Option<f64> {
    if r[..7] != [0x5a, 0x11, 0x08, 0x01, 0x00, group, param_id] {
        return None;
    }
    let value = f64::from(f32::from_le_bytes([r[7], r[8], r[9], r[10]]));
    (!value.is_nan()).then(|| py_round(value, 4))
}

fn switch(value: Option<f64>) -> Option<bool> {
    value.map(|v| v != 0.0)
}

/// 0..1 to a percentage; an infinite or NaN reading is unknown, as in Python.
fn percent(value: Option<f64>) -> Option<i64> {
    value.and_then(|v| py_round_int(v * 100.0))
}

pub fn parse_mic_boost(r: &HidReport) -> Option<u8> {
    (r[..4] == R_MIC_BOOST_PREFIX && MIC_BOOST_DB.contains(&r[6])).then_some(r[6])
}

/// The mode register: SBX, Scout Mode and voice clarity bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProcessingModes {
    pub sbx_enabled: bool,
    pub scout_mode: bool,
    pub voice_clarity: bool,
}

pub fn parse_processing_mode(r: &HidReport) -> Option<ProcessingModes> {
    if r[..6] != R_PROCESSING_MODE_PREFIX {
        return None;
    }
    let mask = r[6];
    Some(ProcessingModes {
        sbx_enabled: mask & MODE_BIT_SBX != 0,
        scout_mode: mask & MODE_BIT_SCOUT != 0,
        voice_clarity: mask & MODE_BIT_VOICE_CLARITY != 0,
    })
}

/// The gains of a mic EQ preset in dB, rounded to 2 decimals.
pub fn mic_eq_preset_gains(preset: MicEqPreset) -> [f64; 8] {
    let v = preset.values();
    std::array::from_fn(|i| py_round(f64::from(f32::from_le_bytes(v[i])), 2))
}

/// The preset the eight mic EQ gains belong to, or the gains themselves ("1.5 -2.25 0 ...").
pub fn match_mic_eq_preset(gains: &[Option<f64>; 8]) -> Option<String> {
    let mut rounded = [0.0; 8];
    for (r, g) in rounded.iter_mut().zip(gains) {
        *r = py_round((*g)?, 2);
    }
    if let Some(p) = MicEqPreset::all().find(|p| mic_eq_preset_gains(*p) == rounded) {
        return Some(p.name().to_string());
    }
    Some(
        rounded
            .iter()
            .map(|g| py_g(*g))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Asks the G6 for everything [`DeviceStatus`] holds, in the order Python asks. `query` gets the bytes to send
/// (without report id or padding) and returns the 64-byte answer, or `None` for no answer.
pub fn collect_status(mut query: impl FnMut(&[u8]) -> Option<HidReport>) -> DeviceStatus {
    let mut s = DeviceStatus {
        firmware: query(Q_FIRMWARE).as_ref().and_then(parse_firmware),
        output: query(Q_OUTPUT).as_ref().map(parse_output),
        ..DeviceStatus::default()
    };
    if let Some(f) = first_match(&mut query, &Q_MODE_FLAGS, parse_mode_flags) {
        s.mode_flags = Some(f.flags);
        s.direct_mode = Some(f.direct_mode);
        s.spdif_direct = Some(f.spdif_direct);
        s.ring_enabled = Some(f.ring_enabled);
        s.line_out_headphone_virtualization = Some(f.line_out_headphone_virtualization);
    }
    s.decoder_mode = query(Q_DECODER).as_ref().and_then(parse_decoder);
    s.dac_filter = query(Q_DAC_FILTER).as_ref().and_then(parse_dac_filter);
    s.rgb_enabled = first_match(&mut query, &Q_RGB, parse_rgb);
    // the flags carry the ring state too; ask separately only when they gave no answer
    if s.ring_enabled.is_none() {
        s.ring_enabled = first_match(&mut query, &Q_RING, parse_ring);
    }

    let mut param = |id: u8, group: u8| -> Option<f64> {
        query(&param_query(id, group))
            .as_ref()
            .and_then(|r| parse_param(r, id, group))
    };
    let sbx = SbxEffect::ALL.map(|e| {
        let (sw, lv) = e.ids();
        let enabled = switch(param(sw, PLAYBACK_PARAM));
        SbxState {
            enabled,
            level: percent(param(lv, PLAYBACK_PARAM)),
        }
    });
    if sbx.iter().any(|e| e.enabled.is_some() || e.level.is_some()) {
        s.sbx = Some(sbx);
    }
    if let Some(mode) = param(SMART_VOLUME_SPECIAL_ID, PLAYBACK_PARAM) {
        // a value no mode uses is shown as Python's str() of the float
        s.smart_volume_mode = Some(if mode == 0.0 {
            "Normal".to_string()
        } else if mode == 1.0 {
            "Loud".to_string()
        } else if mode == 2.0 {
            "Night".to_string()
        } else {
            py_repr(mode)
        });
    }
    s.eq_enabled = switch(param(EQ_ENABLE_ID, PLAYBACK_PARAM));
    s.eq_preamp_db = param(EQ_PREAMP_ID, PLAYBACK_PARAM);
    let bands = EQ_BAND_IDS.map(|id| param(id, PLAYBACK_PARAM));
    debug_assert_eq!(bands.len(), EQ_BAND_FREQS.len());
    if bands.iter().any(Option::is_some) {
        s.eq_bands_db = Some(bands);
    }
    drop(param);

    s.mic_boost_db = query(Q_MIC_BOOST).as_ref().and_then(parse_mic_boost);
    if let Some(m) = query(&MODE_QUERY).as_ref().and_then(parse_processing_mode) {
        s.sbx_enabled = Some(m.sbx_enabled);
        s.scout_mode = Some(m.scout_mode);
        s.voice_clarity = Some(m.voice_clarity);
    }

    let mut rec = |id: u8| -> Option<f64> {
        query(&param_query(id, RECORDING_PARAM))
            .as_ref()
            .and_then(|r| parse_param(r, id, RECORDING_PARAM))
    };
    // field by field, since the queries go out in this order
    let noise_reduction = switch(rec(VOICE_CLARITY_ID));
    let noise_reduction_level = percent(rec(NOISE_REDUCTION_LEVEL_ID));
    let acoustic_echo_cancellation = switch(rec(AEC_ID));
    let smart_volume = switch(rec(SMART_VOLUME_ID));
    let mic_eq = switch(rec(MIC_EQ_ID));
    let recording = RecordingStatus {
        noise_reduction,
        noise_reduction_level,
        acoustic_echo_cancellation,
        smart_volume,
        mic_eq,
    };
    if recording.any_known() {
        s.recording = Some(recording);
    }
    let gains = MIC_EQ_BAND_IDS.map(&mut rec);
    s.mic_eq_preset = match_mic_eq_preset(&gains);
    s
}

fn first_match<T>(
    query: &mut impl FnMut(&[u8]) -> Option<HidReport>,
    candidates: &[&[u8]],
    parse: impl Fn(&HidReport) -> Option<T>,
) -> Option<T> {
    candidates
        .iter()
        .find_map(|q| query(q).as_ref().and_then(&parse))
}
