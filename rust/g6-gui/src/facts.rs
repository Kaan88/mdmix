//! Words and numbers for what the status says: pure functions of a `DeviceStatus`, so the pages stay drawing.

use g6_proto::eq_presets;
use g6_proto::equalizer::EQ_BAND_FREQS;
use g6_proto::status::DeviceStatus;
use g6_proto::types::{Output, PlaybackFilter};

pub const DASH: &str = "—";

pub const OUTPUTS: [&str; 2] = ["Speakers", "Headphones"];
pub const DECODER_MODES: [&str; 3] = ["Normal", "Full", "Night"];
pub const MIC_BOOSTS: [&str; 4] = ["0", "10", "20", "30"];
pub const LAYOUTS: [&str; 3] = ["Stereo", "5.1", "7.1"];
pub const SMART_MODES: [&str; 3] = ["Normal", "Loud", "Night"];

/// Which of `options` the status value names, or `None` for no answer or a value that isn't one of them.
pub fn index_of(options: &[&str], value: Option<&str>) -> Option<usize> {
    let v = value?;
    options.iter().position(|o| o.eq_ignore_ascii_case(v))
}

pub fn output_index(s: &DeviceStatus) -> Option<usize> {
    index_of(&OUTPUTS, s.output.as_deref())
}

/// The output as the preset table wants it.
pub fn output_of(s: &DeviceStatus) -> Option<Output> {
    match output_index(s)? {
        0 => Some(Output::Speakers),
        _ => Some(Output::Headphones),
    }
}

/// What a decoder mode or filter reads as when the G6 isn't giving one (Direct mode on).
pub const NOT_REPORTED: &str = "Not reported in Direct mode";

/// A raw, unnamed answer that is all zeros ("0x00", "0x00000000"): what the G6 sends for the decoder mode and the
/// DAC filter while Direct mode is on. It is no setting.
pub fn is_raw_zero(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c == '0'))
}

/// In Direct mode a raw zero decoder mode or filter is "not reported": it becomes `None`.
pub fn drop_direct_zeros(s: &mut DeviceStatus) {
    if s.direct_mode != Some(true) {
        return;
    }
    for v in [&mut s.decoder_mode, &mut s.dac_filter] {
        if v.as_deref().is_some_and(is_raw_zero) {
            *v = None;
        }
    }
}

pub fn filter_label(s: &DeviceStatus) -> Option<String> {
    let name = s.dac_filter.as_deref()?;
    Some(match PlaybackFilter::from_name(name) {
        Some(f) => f.label().to_string(),
        None => name.to_string(),
    })
}

/// "PRESET_3" is "Preset 3"; the eight gains of a curve that matches no preset are "Custom".
pub fn mic_eq_label(s: &DeviceStatus) -> Option<String> {
    let v = s.mic_eq_preset.as_deref()?;
    Some(match v.strip_prefix("PRESET_") {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => format!("Preset {n}"),
        _ => "Custom".to_string(),
    })
}

pub fn mic_boost_index(s: &DeviceStatus) -> Option<usize> {
    let db = s.mic_boost_db?;
    MIC_BOOSTS.iter().position(|b| b.parse() == Ok(db))
}

/// "+2.5 dB", "-1 dB", "0 dB".
pub fn db(v: f64) -> String {
    let mut s = format!("{v:.1}");
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    if v > 0.0 {
        s.insert(0, '+');
    }
    format!("{s} dB")
}

/// The same without the unit, for the narrow band columns.
pub fn db_short(v: f64) -> String {
    db(v).trim_end_matches(" dB").to_string()
}

pub fn percent(v: Option<i64>) -> String {
    v.map_or_else(|| DASH.to_string(), |v| format!("{v} %"))
}

pub fn on_off(v: Option<bool>) -> String {
    match v {
        Some(true) => "On".into(),
        Some(false) => "Off".into(),
        None => DASH.into(),
    }
}

/// The curve the status holds: preamp and the ten bands, when all of them were read.
pub fn eq_curve(s: &DeviceStatus) -> Option<(f64, [f64; 10])> {
    let bands = s.eq_bands_db?;
    let mut out = [0.0; 10];
    for (o, b) in out.iter_mut().zip(bands) {
        *o = b?;
    }
    Some((s.eq_preamp_db?, out))
}

/// The name of the SBC preset whose curve for the current output is the one on the device.
pub fn eq_preset_name(s: &DeviceStatus) -> Option<&'static str> {
    let (pre, bands) = eq_curve(s)?;
    let output = output_of(s)?;
    eq_presets::preset_ids().find(|id| {
        eq_presets::eq_preset_values(id, output).is_ok_and(|(p, b)| {
            (p - pre).abs() < 0.01 && b.iter().zip(bands).all(|((_, a), c)| (a - c).abs() < 0.01)
        })
    })
}

pub fn band_labels() -> [String; 10] {
    std::array::from_fn(|i| {
        let f = EQ_BAND_FREQS[i];
        match f.strip_suffix('k') {
            Some(n) => format!("{n}k"),
            None => f.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample;

    #[test]
    fn the_live_sample_reads_as_the_g6_said() {
        let s = sample::live_status();
        assert_eq!(output_index(&s), Some(0));
        assert_eq!(
            filter_label(&s).as_deref(),
            Some("Slow Roll-off / Minimum Phase")
        );
        assert_eq!(mic_eq_label(&s).as_deref(), Some("Preset 1"));
        assert_eq!(mic_boost_index(&s), Some(0));
        assert_eq!(index_of(&DECODER_MODES, s.decoder_mode.as_deref()), Some(0));
    }

    #[test]
    fn db_text() {
        assert_eq!(db(2.5), "+2.5 dB");
        assert_eq!(db(-1.0), "-1 dB");
        assert_eq!(db(0.0), "0 dB");
        assert_eq!(db_short(-12.0), "-12");
    }

    #[test]
    fn a_preset_curve_is_found_by_its_values() {
        let (id, speakers, _) = eq_presets::all()[3];
        let mut s = sample::live_status();
        s.output = Some("Speakers".into());
        s.eq_preamp_db = Some(speakers[0]);
        let mut bands = [None; 10];
        for (b, v) in bands.iter_mut().zip(&speakers[1..]) {
            *b = Some(*v);
        }
        s.eq_bands_db = Some(bands);
        let found = eq_preset_name(&s).expect("a preset");
        // Two presets may share a curve; the one found must have these values.
        let (p, b) = eq_presets::eq_preset_values(found, Output::Speakers).unwrap();
        assert_eq!(p, speakers[0]);
        assert_eq!(b[4].1, speakers[5]);
        let _ = id;
        s.eq_preamp_db = Some(9.0);
        assert_eq!(eq_preset_name(&s), None);
    }

    #[test]
    fn unknown_values_are_none() {
        assert_eq!(index_of(&OUTPUTS, Some("Line")), None);
        assert_eq!(index_of(&OUTPUTS, None), None);
        assert_eq!(percent(None), DASH);
        assert_eq!(on_off(None), DASH);
    }
}
