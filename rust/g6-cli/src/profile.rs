//! Settings profiles (Python: `g6_profile.py`): what the G6 reports, saved as a JSON map of CLI settings, and turned
//! back into the same command-line arguments when loaded. Only the settings in [`PROFILE_FLAGS`] are accepted, so
//! a profile can't switch the output, reload drivers or start an effect.

use std::path::Path;

use crate::pyval::{LoadError, PyVal, decode_text_file, dumps_indent2, loads};
use crate::{CliError, os_error};
use g6_proto::equalizer::EQ_BAND_FREQS;
use g6_proto::status::DeviceStatus;
use g6_proto::types::{MicEqPreset, PlaybackFilter, SbxEffect};

pub const PROFILE_VERSION: i64 = 1;

/// Every setting a profile may hold, in the order it is applied.
pub const PROFILE_FLAGS: [&str; 30] = [
    "playback-direct-mode",
    "playback-spdif-out-direct-mode",
    "playback-filter",
    "playback-line-out-headphone-virtualization",
    "decoder-mode",
    "sbx",
    "scout-mode",
    "sbx-surround",
    "sbx-surround-value",
    "sbx-crystalizer",
    "sbx-crystalizer-value",
    "sbx-bass",
    "sbx-bass-value",
    "sbx-smart-volume",
    "sbx-smart-volume-value",
    "sbx-dialog-plus",
    "sbx-dialog-plus-value",
    "sbx-smart-volume-special-value",
    "eq",
    "eq-preamp",
    "eq-band",
    "recording-mic-boost-db",
    "recording-voice-clarity",
    "recording-voice-clarity-noise-reduction",
    "recording-voice-clarity-aec",
    "recording-voice-clarity-smart-volume",
    "recording-voice-clarity-mic-eq",
    "recording-voice-clarity-mic-eq-preset",
    "lighting-disable",
    "lighting-volume-ring",
];

/// Flags without a value.
const SWITCHES: [&str; 1] = ["lighting-disable"];

fn ed(v: bool) -> PyVal {
    PyVal::str(if v { "Enabled" } else { "Disabled" })
}

/// The CLI flag of an SBX effect ("sbx-smart-volume").
fn sbx_flag(effect: SbxEffect) -> String {
    format!("sbx-{}", effect.key().replace('_', "-"))
}

/// The profile of what the G6 reported; values it couldn't read are left out.
pub fn profile_from_status(st: &DeviceStatus) -> PyVal {
    let mut s: Vec<(String, PyVal)> = Vec::new();
    let mut put = |flag: &str, v: PyVal| s.push((flag.to_string(), v));
    let put_bool = |put: &mut dyn FnMut(&str, PyVal), flag: &str, v: Option<bool>| {
        if let Some(v) = v {
            put(flag, ed(v));
        }
    };

    put_bool(&mut put, "playback-direct-mode", st.direct_mode);
    put_bool(&mut put, "playback-spdif-out-direct-mode", st.spdif_direct);
    if let Some(f) = st
        .dac_filter
        .as_deref()
        .filter(|f| PlaybackFilter::from_name(f).is_some())
    {
        put("playback-filter", PyVal::str(f));
    }
    put_bool(
        &mut put,
        "playback-line-out-headphone-virtualization",
        st.line_out_headphone_virtualization,
    );
    if let Some(m) = st
        .decoder_mode
        .as_deref()
        .filter(|m| matches!(*m, "Normal" | "Full" | "Night"))
    {
        put("decoder-mode", PyVal::str(m));
    }
    put_bool(&mut put, "sbx", st.sbx_enabled);
    put_bool(&mut put, "scout-mode", st.scout_mode);
    if let Some(effects) = &st.sbx {
        for (effect, state) in SbxEffect::ALL.into_iter().zip(effects) {
            let flag = sbx_flag(effect);
            put_bool(&mut put, &flag, state.enabled);
            if let Some(level) = state.level {
                put(&format!("{flag}-value"), PyVal::int(level));
            }
        }
    }
    if let Some(m) = st
        .smart_volume_mode
        .as_deref()
        .filter(|m| matches!(*m, "Loud" | "Night"))
    {
        put("sbx-smart-volume-special-value", PyVal::str(m));
    }
    put_bool(&mut put, "eq", st.eq_enabled);
    if let Some(db) = st.eq_preamp_db {
        put("eq-preamp", PyVal::Float(db));
    }
    if let Some(bands) = &st.eq_bands_db {
        let pairs = EQ_BAND_FREQS
            .iter()
            .zip(bands)
            .filter_map(|(f, db)| db.map(|db| PyVal::List(vec![PyVal::str(*f), PyVal::Float(db)])))
            .collect();
        put("eq-band", PyVal::List(pairs));
    }
    if let Some(db) = st.mic_boost_db.filter(|db| [0, 10, 20, 30].contains(db)) {
        put("recording-mic-boost-db", PyVal::int(db.into()));
    }
    put_bool(&mut put, "recording-voice-clarity", st.voice_clarity);
    let rec = st.recording.unwrap_or_default();
    if let Some(level) = rec
        .noise_reduction_level
        .filter(|l| (0..=100).contains(l) && l % 20 == 0)
    {
        put("recording-voice-clarity-noise-reduction", PyVal::int(level));
    }
    put_bool(
        &mut put,
        "recording-voice-clarity-aec",
        rec.acoustic_echo_cancellation,
    );
    put_bool(
        &mut put,
        "recording-voice-clarity-smart-volume",
        rec.smart_volume,
    );
    put_bool(&mut put, "recording-voice-clarity-mic-eq", rec.mic_eq);
    if let Some(p) = st
        .mic_eq_preset
        .as_deref()
        .filter(|p| MicEqPreset::from_name(p).is_some())
    {
        put("recording-voice-clarity-mic-eq-preset", PyVal::str(p));
    }
    if st.rgb_enabled == Some(false) {
        put("lighting-disable", PyVal::Bool(true));
    }
    put_bool(&mut put, "lighting-volume-ring", st.ring_enabled);
    PyVal::Dict(vec![
        ("g6_profile".into(), PyVal::int(PROFILE_VERSION)),
        ("settings".into(), PyVal::Dict(s)),
    ])
}

/// The settings of a profile made by [`profile_from_status`].
pub fn settings_len(profile: &PyVal) -> usize {
    match profile.get("settings") {
        Some(PyVal::Dict(items)) => items.len(),
        _ => 0,
    }
}

/// CLI arguments that apply `profile`; an error for anything that isn't a profile setting.
pub fn profile_to_argv(profile: &PyVal) -> Result<Vec<String>, String> {
    let settings = match (profile, profile.get("g6_profile"), profile.get("settings")) {
        (PyVal::Dict(_), Some(version), Some(PyVal::Dict(settings))) if version.equals_one() => {
            settings
        }
        _ => {
            return Err(format!(
                "not a G6 profile (expected {{\"g6_profile\": {PROFILE_VERSION}, \"settings\": {{...}}}})"
            ));
        }
    };
    let mut unknown: Vec<&str> = settings
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| !PROFILE_FLAGS.contains(k))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(format!("not a profile setting: {}", unknown.join(", ")));
    }
    let mut argv = Vec::new();
    for flag in PROFILE_FLAGS {
        let Some((_, value)) = settings.iter().find(|(k, _)| k == flag) else {
            continue;
        };
        if SWITCHES.contains(&flag) {
            match value {
                PyVal::Bool(true) => argv.push(format!("--{flag}")),
                PyVal::Bool(false) => {}
                v => return Err(format!("{flag}: expected true or false, got {}", v.repr())),
            }
        } else if flag == "eq-band" {
            let PyVal::List(pairs) = value else {
                return Err(format!(
                    "eq-band: expected a list of [band, dB] pairs, got {}",
                    value.repr()
                ));
            };
            for pair in pairs {
                match pair {
                    PyVal::List(p) if p.len() == 2 => {
                        argv.extend(["--eq-band".to_string(), p[0].py_str(), p[1].py_str()]);
                    }
                    _ => return Err(format!("eq-band: expected [band, dB], got {}", pair.repr())),
                }
            }
        } else {
            match value {
                PyVal::Str(_) | PyVal::Int(_) | PyVal::Float(_) => {
                    argv.extend([format!("--{flag}"), value.py_str()]);
                }
                v => return Err(format!("{flag}: unsupported value {}", v.repr())),
            }
        }
    }
    Ok(argv)
}

/// The file text `json.dump(profile, f, indent=2)` plus a newline writes in text mode: `\r\n` line ends on
/// Windows, as Python's text mode writes them there.
pub fn profile_file_text(profile: &PyVal) -> String {
    let text = dumps_indent2(profile) + "\n";
    if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

/// Write the profile of `status` to `path`; returns it.
pub fn save_profile(path: &str, status: &DeviceStatus) -> Result<PyVal, CliError> {
    let profile = profile_from_status(status);
    if settings_len(&profile) == 0 {
        return Err(CliError::msg(
            "the G6 reported no settings, nothing to save",
        ));
    }
    std::fs::write(Path::new(path), profile_file_text(&profile)).map_err(|e| os_error(&e, path))?;
    Ok(profile)
}

/// The CLI arguments of the profile in `path`.
pub fn load_profile_argv(path: &str) -> Result<Vec<String>, CliError> {
    let bytes = std::fs::read(Path::new(path)).map_err(|e| os_error(&e, path))?;
    let text = decode_text_file(&bytes).map_err(CliError::Msg)?;
    let profile = match loads(&text) {
        Ok(p) => p,
        Err(LoadError::Decode(e)) => {
            return Err(CliError::Msg(format!("{path}: not valid JSON ({e})")));
        }
        Err(LoadError::Value(e)) => return Err(CliError::Msg(e)),
    };
    profile_to_argv(&profile).map_err(CliError::Msg)
}
