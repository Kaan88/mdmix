//! What a device control asks for: a typed [`Setting`] and a [`Val`], and the one CLI write it stands for.
//!
//! Every setting maps to exactly one `g6` flag ([`Setting::flag`], [`argv`]) and to the same
//! `g6_proto::apply::Action` the CLI builds for that flag, so the reports sent are the ones `tests/vectors/writes.json`
//! checks. Nothing here builds bytes: `Action::batches` does. Settings the CLI writes only through USB Audio Control
//! (Windows' volumes and mutes), the speaker setup and the lighting effects have no setting here and stay read only.

use g6_proto::apply::Action;
use g6_proto::eq_presets;
use g6_proto::equalizer::{EQ_BAND_FREQS, EQ_DB_MAX, EQ_DB_MIN};
use g6_proto::status::DeviceStatus;
use g6_proto::types::{
    DecoderMode, MicEqPreset, Output, PlaybackFilter, ProcessingMode, SbxEffect, SmartVolumeSpecial,
};

use crate::facts;
use crate::model::Model;

/// One device setting the window can write. Effects and bands are by index into `SbxEffect::ALL` and
/// `EQ_BAND_FREQS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Setting {
    Sbx,
    ScoutMode,
    EffectOn(u8),
    EffectLevel(u8),
    SmartMode,
    Output,
    DirectMode,
    SpdifDirect,
    LineOutVirt,
    DecoderMode,
    DacFilter,
    Eq,
    EqPreset,
    EqPreamp,
    EqBand(u8),
    MicBoost,
    VoiceClarity,
    NoiseReduction,
    Aec,
    MicSmartVolume,
    MicEq,
    MicEqPreset,
    Lighting,
    Colour,
    VolumeRing,
}

/// A setting's value: a switch, or a number whose meaning is the setting's (a level, an index into its choices, mic
/// boost in dB, an EQ gain in tenths of a dB, a colour as 0xRRGGBB — see [`rgb_val`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Val {
    On(bool),
    Int(i64),
}

/// A colour as [`Setting::Colour`]'s value: 0xRRGGBB packed into `Val::Int` (the picker may land on any of the
/// 16,777,216 colours, so an index into a list of swatches would not do).
pub const fn rgb_val(rgb: [u8; 3]) -> Val {
    Val::Int(((rgb[0] as i64) << 16) | ((rgb[1] as i64) << 8) | rgb[2] as i64)
}

/// The colour in a [`Setting::Colour`] value.
pub fn rgb_of(v: Val) -> Option<[u8; 3]> {
    let Val::Int(n) = v else {
        return None;
    };
    if !(0..=0xFF_FFFF).contains(&n) {
        return None;
    }
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// A colour as the window shows it: `#1a2b3c`, the form the picker's own hex field uses.
pub fn hex_of(rgb: [u8; 3]) -> String {
    let [r, g, b] = rgb;
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The SBX effects' names, in `SbxEffect::ALL` order.
pub const EFFECT_NAMES: [&str; 5] = ["Surround", "Crystalizer", "Bass", "Smart Volume", "Dialog+"];
#[cfg(test)]
const EFFECT_FLAGS: [&str; 5] = [
    "--sbx-surround",
    "--sbx-crystalizer",
    "--sbx-bass",
    "--sbx-smart-volume",
    "--sbx-dialog-plus",
];

/// The static colours the Lighting page offers as quick picks (`--lighting-rgb R G B`); the picker can land on any
/// colour. The G6 can't be asked for its colour.
pub const SWATCHES: [(&str, [u8; 3]); 8] = [
    ("Red", [255, 0, 0]),
    ("Orange", [255, 96, 0]),
    ("Yellow", [255, 200, 0]),
    ("Green", [0, 255, 0]),
    ("Cyan", [0, 200, 255]),
    ("Blue", [0, 0, 255]),
    ("Purple", [160, 0, 255]),
    ("White", [255, 255, 255]),
];

fn effect(i: u8) -> SbxEffect {
    SbxEffect::ALL[usize::from(i).min(4)]
}

#[cfg(test)]
fn on_off(on: bool) -> &'static str {
    if on { "Enabled" } else { "Disabled" }
}

/// Tenths of a dB as the CLI takes them: "-12", "3.5".
fn tenths_text(t: i64) -> String {
    if t % 10 == 0 {
        format!("{}", t / 10)
    } else {
        format!("{:.1}", t as f64 / 10.0)
    }
}

fn tenths(db: f64) -> i64 {
    (db * 10.0).round() as i64
}

fn mic_eq_presets() -> Vec<MicEqPreset> {
    MicEqPreset::all().collect()
}

impl Setting {
    /// Every setting, for the tests.
    #[cfg(test)]
    pub fn all() -> Vec<Setting> {
        use Setting as S;
        let mut v = vec![S::Sbx, S::ScoutMode];
        for i in 0..5 {
            v.push(S::EffectOn(i));
            v.push(S::EffectLevel(i));
        }
        v.extend([
            S::SmartMode,
            S::Output,
            S::DirectMode,
            S::SpdifDirect,
            S::LineOutVirt,
            S::DecoderMode,
            S::DacFilter,
            S::Eq,
            S::EqPreset,
            S::EqPreamp,
        ]);
        v.extend((0..10).map(S::EqBand));
        v.extend([
            S::MicBoost,
            S::VoiceClarity,
            S::NoiseReduction,
            S::Aec,
            S::MicSmartVolume,
            S::MicEq,
            S::MicEqPreset,
            S::Lighting,
            S::Colour,
            S::VolumeRing,
        ]);
        v
    }

    /// The name in a toast: "SBX Surround set to 62 %".
    pub fn label(self) -> String {
        use Setting as S;
        match self {
            S::Sbx => "SBX".into(),
            S::ScoutMode => "Scout Mode".into(),
            S::EffectOn(i) | S::EffectLevel(i) => {
                format!("SBX {}", EFFECT_NAMES[usize::from(i).min(4)])
            }
            S::SmartMode => "Smart Volume mode".into(),
            S::Output => "Output".into(),
            S::DirectMode => "Direct mode".into(),
            S::SpdifDirect => "SPDIF-Out Direct".into(),
            S::LineOutVirt => "Headphone virtualization to Line Out".into(),
            S::DecoderMode => "Decoder dynamic range".into(),
            S::DacFilter => "DAC filter".into(),
            S::Eq => "Equalizer".into(),
            S::EqPreset => "EQ preset".into(),
            S::EqPreamp => "EQ preamp".into(),
            S::EqBand(i) => format!("EQ band {}", facts::band_labels()[usize::from(i).min(9)]),
            S::MicBoost => "Mic boost".into(),
            S::VoiceClarity => "Voice Clarity".into(),
            S::NoiseReduction => "Noise reduction".into(),
            S::Aec => "Acoustic echo cancellation".into(),
            S::MicSmartVolume => "Mic Smart Volume".into(),
            S::MicEq => "Mic EQ".into(),
            S::MicEqPreset => "Mic EQ preset".into(),
            S::Lighting => "Lighting".into(),
            S::Colour => "Lighting colour".into(),
            S::VolumeRing => "Volume ring light".into(),
        }
    }

    /// The `g6` flag the setting is written with. Each setting has its own (checked by the tests, which also
    /// compare the reports with what the CLI sends for it).
    #[cfg(test)]
    pub fn flag(self) -> String {
        use Setting as S;
        match self {
            S::Sbx => "--sbx".into(),
            S::ScoutMode => "--scout-mode".into(),
            S::EffectOn(i) => EFFECT_FLAGS[usize::from(i).min(4)].into(),
            S::EffectLevel(i) => format!("{}-value", EFFECT_FLAGS[usize::from(i).min(4)]),
            S::SmartMode => "--sbx-smart-volume-special-value".into(),
            S::Output => "--set-output".into(),
            S::DirectMode => "--playback-direct-mode".into(),
            S::SpdifDirect => "--playback-spdif-out-direct-mode".into(),
            S::LineOutVirt => "--playback-line-out-headphone-virtualization".into(),
            S::DecoderMode => "--decoder-mode".into(),
            S::DacFilter => "--playback-filter".into(),
            S::Eq => "--eq".into(),
            S::EqPreset => "--eq-preset".into(),
            S::EqPreamp => "--eq-preamp".into(),
            S::EqBand(i) => format!("--eq-band {}", EQ_BAND_FREQS[usize::from(i).min(9)]),
            S::MicBoost => "--recording-mic-boost-db".into(),
            S::VoiceClarity => "--recording-voice-clarity".into(),
            S::NoiseReduction => "--recording-voice-clarity-noise-reduction".into(),
            S::Aec => "--recording-voice-clarity-aec".into(),
            S::MicSmartVolume => "--recording-voice-clarity-smart-volume".into(),
            S::MicEq => "--recording-voice-clarity-mic-eq".into(),
            S::MicEqPreset => "--recording-voice-clarity-mic-eq-preset".into(),
            // Off is --lighting-disable; on again re-sends the colour (--lighting-rgb).
            S::Lighting => "--lighting-disable".into(),
            S::Colour => "--lighting-rgb".into(),
            S::VolumeRing => "--lighting-volume-ring".into(),
        }
    }

    /// Whether Undo may re-send the value from before. Not for the output switch, Direct mode or a preset: what Undo
    /// should re-send for those is an open device question (`doc/gui-plan.md`).
    pub fn undoable(self) -> bool {
        !matches!(
            self,
            Setting::Output | Setting::DirectMode | Setting::EqPreset
        )
    }

    /// The G6 can't be asked for this one: what was written is kept as the value.
    pub fn unreadable(self) -> bool {
        self == Setting::Colour
    }

    /// A slider's ends and step in the unit it shows (percent or dB). `Val::Int` is that number, in tenths for dB.
    pub fn range(self) -> Option<(f32, f32, f32)> {
        use Setting as S;
        match self {
            S::EffectLevel(_) => Some((0.0, 100.0, 1.0)),
            S::NoiseReduction => Some((0.0, 100.0, 20.0)),
            S::EqPreamp | S::EqBand(_) => Some((EQ_DB_MIN as f32, EQ_DB_MAX as f32, 0.5)),
            _ => None,
        }
    }

    /// A slider's value in its unit as a `Val`.
    pub fn val_of(self, v: f32) -> Val {
        match self {
            Setting::EqPreamp | Setting::EqBand(_) => Val::Int(tenths(f64::from(v))),
            _ => Val::Int(v.round() as i64),
        }
    }

    /// A `Val` of a slider in its unit.
    pub fn unit_of(self, v: Val) -> f32 {
        match (self, v) {
            (Setting::EqPreamp | Setting::EqBand(_), Val::Int(t)) => t as f32 / 10.0,
            (_, Val::Int(n)) => n as f32,
            (_, Val::On(b)) => f32::from(u8::from(b)),
        }
    }

    /// The value segment `k` of this setting's segmented choice writes; `None` when the CLI has no write for it.
    pub fn seg_val(self, k: usize) -> Option<Val> {
        match self {
            // The CLI writes Night and Loud only (--sbx-smart-volume-special-value).
            Setting::SmartMode => (k == 1 || k == 2).then_some(Val::Int(k as i64)),
            Setting::Output => (k < 2).then_some(Val::Int(k as i64)),
            Setting::DecoderMode => (k < 3).then_some(Val::Int(k as i64)),
            Setting::MicBoost => facts::MIC_BOOSTS
                .get(k)
                .and_then(|b| b.parse().ok())
                .map(Val::Int),
            _ => None,
        }
    }

    /// The choices of a dropdown, as (label, value).
    pub fn options(self) -> Vec<(String, Val)> {
        match self {
            Setting::DacFilter => PlaybackFilter::ALL
                .iter()
                .enumerate()
                .map(|(i, f)| (f.label().to_string(), Val::Int(i as i64)))
                .collect(),
            Setting::EqPreset => eq_presets::preset_ids()
                .enumerate()
                .map(|(i, id)| (id.to_string(), Val::Int(i as i64)))
                .collect(),
            Setting::MicEqPreset => mic_eq_presets()
                .iter()
                .enumerate()
                .map(|(i, p)| (facts::mic_eq_name(p.name()), Val::Int(i as i64)))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The value as words: "62 %", "On", "Headphones", "+2.5 dB".
    pub fn text(self, v: Val) -> String {
        use Setting as S;
        let pick = |list: &[&str], n: i64| {
            usize::try_from(n)
                .ok()
                .and_then(|i| list.get(i))
                .map_or_else(|| n.to_string(), |s| s.to_string())
        };
        match (self, v) {
            (_, Val::On(b)) => if b { "On" } else { "Off" }.into(),
            (S::EffectLevel(_) | S::NoiseReduction, Val::Int(n)) => format!("{n} %"),
            (S::EqPreamp | S::EqBand(_), Val::Int(t)) => facts::db(t as f64 / 10.0),
            (S::MicBoost, Val::Int(n)) => format!("{n} dB"),
            (S::SmartMode, Val::Int(n)) => pick(&facts::SMART_MODES, n),
            (S::Output, Val::Int(n)) => pick(&facts::OUTPUTS, n),
            (S::DecoderMode, Val::Int(n)) => pick(&facts::DECODER_MODES, n),
            (S::DacFilter | S::EqPreset | S::MicEqPreset, Val::Int(n)) => self
                .options()
                .into_iter()
                .find(|(_, o)| *o == Val::Int(n))
                .map_or_else(|| n.to_string(), |(l, _)| l),
            (S::Colour, v) => rgb_of(v).map_or_else(|| format!("{v:?}"), hex_of),
            (_, Val::Int(n)) => n.to_string(),
        }
    }

    /// What the device (or, for the colour, this session) says the setting is. `None`: not read, not known, or not
    /// one of the values the window writes.
    pub fn current(self, m: &Model) -> Option<Val> {
        use Setting as S;
        let s = &m.status;
        let on = |b: Option<bool>| b.map(Val::On);
        let rec = s.recording.unwrap_or_default();
        let index = |i: Option<usize>| i.map(|i| Val::Int(i as i64));
        match self {
            S::Sbx => on(s.sbx_enabled),
            S::ScoutMode => on(s.scout_mode),
            S::EffectOn(i) => on(s.sbx_effect(effect(i)).and_then(|e| e.enabled)),
            S::EffectLevel(i) => s.sbx_effect(effect(i)).and_then(|e| e.level).map(Val::Int),
            S::SmartMode => index(facts::index_of(
                &facts::SMART_MODES,
                s.smart_volume_mode.as_deref(),
            )),
            S::Output => index(facts::output_index(s)),
            S::DirectMode => on(s.direct_mode),
            S::SpdifDirect => on(s.spdif_direct),
            S::LineOutVirt => on(s.line_out_headphone_virtualization),
            S::DecoderMode => index(facts::index_of(
                &facts::DECODER_MODES,
                s.decoder_mode.as_deref(),
            )),
            S::DacFilter => index(
                s.dac_filter
                    .as_deref()
                    .and_then(PlaybackFilter::from_name)
                    .and_then(|f| PlaybackFilter::ALL.iter().position(|x| *x == f)),
            ),
            S::Eq => on(s.eq_enabled),
            S::EqPreset => index(
                facts::eq_preset_name(s)
                    .and_then(|n| eq_presets::preset_ids().position(|id| id == n)),
            ),
            S::EqPreamp => s.eq_preamp_db.map(|d| Val::Int(tenths(d))),
            S::EqBand(i) => s
                .eq_bands_db
                .and_then(|b| b[usize::from(i).min(9)])
                .map(|d| Val::Int(tenths(d))),
            S::MicBoost => s.mic_boost_db.map(|d| Val::Int(i64::from(d))),
            S::VoiceClarity => on(s.voice_clarity),
            S::NoiseReduction => rec.noise_reduction_level.map(Val::Int),
            S::Aec => on(rec.acoustic_echo_cancellation),
            S::MicSmartVolume => on(rec.smart_volume),
            S::MicEq => on(rec.mic_eq),
            S::MicEqPreset => index(
                s.mic_eq_preset
                    .as_deref()
                    .and_then(MicEqPreset::from_name)
                    .and_then(|p| mic_eq_presets().iter().position(|x| *x == p)),
            ),
            S::Lighting => on(s.rgb_enabled),
            S::Colour => m.colour.map(rgb_val),
            S::VolumeRing => on(s.ring_enabled),
        }
    }

    /// Whether the device now has `v`: `Some(true)` it does, `Some(false)` it has something else, `None` it didn't
    /// say. A preset counts as there when the curve is its curve, whichever name the curve is found under.
    pub fn confirmed(self, v: Val, m: &Model) -> Option<bool> {
        if self == Setting::EqPreset {
            let (Val::Int(i), Some(out)) = (v, facts::output_of(&m.status)) else {
                return None;
            };
            let id = eq_presets::preset_ids().nth(usize::try_from(i).ok()?)?;
            let (pre, bands) = eq_presets::eq_preset_values(id, out).ok()?;
            let (have_pre, have) = facts::eq_curve(&m.status)?;
            let close = |a: f64, b: f64| (a - b).abs() < 0.01;
            return Some(
                close(pre, have_pre) && bands.iter().zip(have).all(|((_, a), b)| close(*a, b)),
            );
        }
        if self.unreadable() {
            return None;
        }
        self.current(m).map(|c| c == v)
    }

    /// Shows `v` in the model as if the device had it: the pending value drawn until the read-back comes.
    pub fn put(self, v: Val, m: &mut Model) {
        use Setting as S;
        let s: &mut DeviceStatus = &mut m.status;
        let int = match v {
            Val::Int(n) => n,
            Val::On(b) => i64::from(b),
        };
        let on = match v {
            Val::On(b) => b,
            Val::Int(n) => n != 0,
        };
        let idx = usize::try_from(int).unwrap_or(usize::MAX);
        match self {
            S::Sbx => s.sbx_enabled = Some(on),
            S::ScoutMode => s.scout_mode = Some(on),
            S::EffectOn(i) | S::EffectLevel(i) => {
                let mut all = s.sbx.unwrap_or_default();
                let e = &mut all[usize::from(i).min(4)];
                if matches!(self, S::EffectOn(_)) {
                    e.enabled = Some(on);
                } else {
                    e.level = Some(int);
                }
                s.sbx = Some(all);
            }
            S::SmartMode => {
                s.smart_volume_mode = facts::SMART_MODES.get(idx).map(|n| n.to_string());
            }
            S::Output => s.output = facts::OUTPUTS.get(idx).map(|n| n.to_string()),
            S::DirectMode => s.direct_mode = Some(on),
            S::SpdifDirect => s.spdif_direct = Some(on),
            S::LineOutVirt => s.line_out_headphone_virtualization = Some(on),
            S::DecoderMode => {
                s.decoder_mode = facts::DECODER_MODES.get(idx).map(|n| n.to_string());
            }
            S::DacFilter => {
                s.dac_filter = PlaybackFilter::ALL.get(idx).map(|f| f.name().to_string())
            }
            S::Eq => s.eq_enabled = Some(on),
            S::EqPreset => {
                let out = facts::output_of(s);
                let id = eq_presets::preset_ids().nth(idx);
                if let (Some(out), Some(id)) = (out, id)
                    && let Ok((pre, bands)) = eq_presets::eq_preset_values(id, out)
                {
                    s.eq_preamp_db = Some(pre);
                    s.eq_bands_db = Some(bands.map(|(_, db)| Some(db)));
                }
            }
            S::EqPreamp => s.eq_preamp_db = Some(int as f64 / 10.0),
            S::EqBand(i) => {
                let mut b = s.eq_bands_db.unwrap_or([None; 10]);
                b[usize::from(i).min(9)] = Some(int as f64 / 10.0);
                s.eq_bands_db = Some(b);
            }
            S::MicBoost => s.mic_boost_db = u8::try_from(int).ok(),
            S::VoiceClarity => s.voice_clarity = Some(on),
            S::NoiseReduction | S::Aec | S::MicSmartVolume | S::MicEq => {
                let mut r = s.recording.unwrap_or_default();
                match self {
                    S::NoiseReduction => r.noise_reduction_level = Some(int),
                    S::Aec => r.acoustic_echo_cancellation = Some(on),
                    S::MicSmartVolume => r.smart_volume = Some(on),
                    _ => r.mic_eq = Some(on),
                }
                s.recording = Some(r);
            }
            S::MicEqPreset => {
                s.mic_eq_preset = mic_eq_presets().get(idx).map(|p| p.name().to_string());
            }
            S::Lighting => s.rgb_enabled = Some(on),
            S::Colour => {
                if let Some(rgb) = rgb_of(v) {
                    m.colour = Some(rgb);
                    m.status.rgb_enabled = Some(true);
                }
            }
            S::VolumeRing => s.ring_enabled = Some(on),
        }
    }

    /// The CLI action that writes `v`: the one `g6 <argv>` builds. An error says why there is none.
    pub fn action(self, v: Val, m: &Model) -> Result<Action, String> {
        use Setting as S;
        let bad = || format!("{} can't be set to {v:?}.", self.label());
        let idx = |v: Val| match v {
            Val::Int(n) => usize::try_from(n).ok(),
            Val::On(_) => None,
        };
        let int = |v: Val| match v {
            Val::Int(n) => Some(n),
            Val::On(_) => None,
        };
        let on = |v: Val| match v {
            Val::On(b) => Some(b),
            Val::Int(_) => None,
        };
        Ok(match self {
            S::Sbx => A::ProcessingMode(ProcessingMode::Sbx, on(v).ok_or_else(bad)?),
            S::ScoutMode => A::ProcessingMode(ProcessingMode::ScoutMode, on(v).ok_or_else(bad)?),
            S::EffectOn(i) => A::SbxToggle(effect(i), on(v).ok_or_else(bad)?),
            S::EffectLevel(i) => A::SbxLevel(effect(i), int(v).ok_or_else(bad)?),
            S::SmartMode => A::SmartVolumeSpecial(match idx(v) {
                Some(1) => SmartVolumeSpecial::Loud,
                Some(2) => SmartVolumeSpecial::Night,
                _ => {
                    return Err(
                        "The command line has no write for Smart Volume's Normal mode.".into(),
                    );
                }
            }),
            S::Output => A::SetOutput(match idx(v) {
                Some(0) => Output::Speakers,
                Some(1) => Output::Headphones,
                _ => return Err(bad()),
            }),
            S::DirectMode => A::DirectMode(on(v).ok_or_else(bad)?),
            S::SpdifDirect => A::SpdifOutDirectMode(on(v).ok_or_else(bad)?),
            S::LineOutVirt => A::LineOutHeadphoneVirtualization(on(v).ok_or_else(bad)?),
            S::DecoderMode => A::DecoderMode(
                idx(v)
                    .and_then(|i| facts::DECODER_MODES.get(i))
                    .and_then(|n| DecoderMode::from_name(n))
                    .ok_or_else(bad)?,
            ),
            S::DacFilter => A::PlaybackFilter(
                idx(v)
                    .and_then(|i| PlaybackFilter::ALL.get(i).copied())
                    .ok_or_else(bad)?,
            ),
            S::Eq => A::Eq(on(v).ok_or_else(bad)?),
            S::EqPreset => {
                let id = idx(v)
                    .and_then(|i| eq_presets::preset_ids().nth(i))
                    .ok_or_else(bad)?;
                // The curve for the output in use, named, as `--eq-preset ID --eq-preset-output OUT` writes it.
                let out = facts::output_of(&m.status)
                    .ok_or("The output isn't known, so the preset's curve can't be chosen.")?;
                A::EqPreset(id.to_string(), Some(out))
            }
            S::EqPreamp => A::EqPreamp(int(v).ok_or_else(bad)? as f64 / 10.0),
            S::EqBand(i) => A::EqBand(
                EQ_BAND_FREQS[usize::from(i).min(9)].to_string(),
                tenths_text(int(v).ok_or_else(bad)?),
            ),
            S::MicBoost => A::MicBoost(int(v).ok_or_else(bad)?),
            S::VoiceClarity => A::VoiceClarity(on(v).ok_or_else(bad)?),
            S::NoiseReduction => A::NoiseReductionLevel(int(v).ok_or_else(bad)?),
            S::Aec => A::Aec(on(v).ok_or_else(bad)?),
            S::MicSmartVolume => A::MicSmartVolume(on(v).ok_or_else(bad)?),
            S::MicEq => A::MicEq(on(v).ok_or_else(bad)?),
            S::MicEqPreset => A::MicEqPreset(
                idx(v)
                    .and_then(|i| mic_eq_presets().get(i).copied())
                    .ok_or_else(bad)?,
            ),
            S::Lighting => match on(v).ok_or_else(bad)? {
                false => A::LightingDisable,
                true => {
                    let [r, g, b] = m.colour.ok_or(
                        "The G6 can't be asked which colour it had: pick a colour to turn the light on.",
                    )?;
                    A::LightingRgb([r, g, b].map(i64::from))
                }
            },
            S::Colour => A::LightingRgb(rgb_of(v).ok_or_else(bad)?.map(i64::from)),
            S::VolumeRing => A::LightingVolumeRing(on(v).ok_or_else(bad)?),
        })
    }
}

use Action as A;

/// The `g6` arguments that write `v`: what the same change is on the command line.
#[cfg(test)]
pub fn argv(s: Setting, v: Val, m: &Model) -> Result<Vec<String>, String> {
    let action = s.action(v, m)?;
    let flag = |f: &str, rest: &[String]| {
        let mut a = vec![f.to_string()];
        a.extend_from_slice(rest);
        a
    };
    Ok(match action {
        A::LightingDisable => vec!["--lighting-disable".into()],
        A::LightingRgb(c) => flag("--lighting-rgb", &c.map(|x| x.to_string())),
        A::EqBand(freq, db) => vec!["--eq-band".into(), freq, db],
        A::EqPreset(id, Some(out)) => {
            vec![
                "--eq-preset".into(),
                id,
                "--eq-preset-output".into(),
                out.name().into(),
            ]
        }
        A::MicEqPreset(p) => flag(&s.flag(), &[p.name().into()]),
        A::SmartVolumeSpecial(m) => flag(
            &s.flag(),
            &[match m {
                SmartVolumeSpecial::Night => "Night".into(),
                SmartVolumeSpecial::Loud => "Loud".into(),
            }],
        ),
        A::PlaybackFilter(f) => flag(&s.flag(), &[f.name().into()]),
        A::DecoderMode(d) => flag(&s.flag(), &[d.name().into()]),
        A::SetOutput(o) => flag(&s.flag(), &[o.name().into()]),
        A::EqPreamp(_) | A::SbxLevel(..) | A::MicBoost(_) | A::NoiseReductionLevel(_) => {
            let n = match v {
                Val::Int(n) => n,
                Val::On(_) => return Err("not a number".into()),
            };
            let text = if s == Setting::EqPreamp {
                tenths_text(n)
            } else {
                n.to_string()
            };
            flag(&s.flag(), &[text])
        }
        _ => match v {
            Val::On(b) => flag(&s.flag(), &[on_off(b).into()]),
            Val::Int(_) => return Err("not a switch".into()),
        },
    })
}

#[cfg(test)]
pub(crate) mod test_device {
    use super::*;
    use g6_proto::apply::Host;
    use g6_proto::packet::{Batch, ProtoError, hex};
    use g6_proto::types::Output;

    pub struct NoHost;

    impl Host for NoHost {
        fn next_toggle_state(&mut self) -> Result<Output, ProtoError> {
            Err(ProtoError("no toggle".into()))
        }
        fn current_output(&mut self) -> Output {
            Output::Headphones
        }
    }

    /// The HID reports of a setting, hex without trailing zeros, as `writes.json` keeps them.
    pub fn hex_reports(s: Setting, v: Val, m: &Model) -> Vec<String> {
        let mut out = Vec::new();
        for b in s.action(v, m).unwrap().batches(&mut NoHost).unwrap() {
            let Batch::Hid(rs) = b else {
                panic!("{s:?} is not HID")
            };
            for r in rs {
                out.push(hex(&r).trim_end_matches('0').to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_device::hex_reports;
    use super::*;
    use crate::sample::Sample;
    use std::collections::HashSet;

    fn writes() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/vectors/writes.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// Every value a control of the setting can ask for: both ways of a switch, each step of a slider, every
    /// segment and every choice of a dropdown (for the colour: colours, which `writes.json` has cases for).
    fn values(s: Setting) -> Vec<Val> {
        if s == Setting::Colour {
            return [
                [0, 0, 0],
                [12, 34, 56],
                [17, 34, 51],
                [255, 0, 0],
                [255, 255, 255],
            ]
            .map(rgb_val)
            .to_vec();
        }
        let mut v: Vec<Val> = if let Some((lo, hi, step)) = s.range() {
            let n = ((hi - lo) / step).round() as i32;
            (0..=n).map(|k| s.val_of(lo + step * k as f32)).collect()
        } else if !s.options().is_empty() {
            s.options().into_iter().map(|(_, v)| v).collect()
        } else if matches!(
            s,
            Setting::SmartMode | Setting::Output | Setting::DecoderMode | Setting::MicBoost
        ) {
            (0..4).filter_map(|k| s.seg_val(k)).collect()
        } else {
            vec![Val::On(true), Val::On(false)]
        };
        let mut m = Sample::Live.model();
        m.colour = Some([255, 0, 0]);
        v.retain(|x| s.action(*x, &m).is_ok());
        v
    }

    #[test]
    fn every_setting_has_its_own_flag() {
        let all = Setting::all();
        let flags: HashSet<String> = all.iter().map(|s| s.flag()).collect();
        assert_eq!(flags.len(), all.len());
        for s in all {
            assert!(s.flag().starts_with("--"), "{s:?}");
        }
    }

    /// Each `writes.json` case that one setting stands for is sent byte for byte the same.
    #[test]
    fn the_reports_are_the_ones_the_cli_sends() {
        let mut m = Sample::Live.model();
        m.colour = Some([255, 0, 0]);
        let cases = writes();
        let cases = cases["cases"].as_array().unwrap();
        let mut matched = 0;
        for out in [0, 1] {
            Setting::Output.put(Val::Int(out), &mut m);
            for s in Setting::all() {
                for v in values(s) {
                    let args = argv(s, v, &m).unwrap();
                    let Some(case) = cases.iter().find(|c| {
                        c["argv"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|a| a.as_str().unwrap())
                            .eq(args.iter().map(String::as_str))
                    }) else {
                        continue;
                    };
                    let want: Vec<String> = case["hid"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|h| h.as_str().unwrap().trim_end_matches('0').to_string())
                        .collect();
                    assert_eq!(hex_reports(s, v, &m), want, "{args:?}");
                    matched += 1;
                }
            }
        }
        // Every HID case but --toggle-output and the combined Scout Mode case.
        assert!(matched >= 155, "{matched} cases matched");
    }

    #[test]
    fn a_shown_value_reads_back_as_itself() {
        for s in Setting::all() {
            for v in values(s) {
                let mut m = Sample::Live.model();
                m.colour = Some([255, 0, 0]);
                s.put(v, &mut m);
                if s == Setting::EqPreset {
                    assert_eq!(s.confirmed(v, &m), Some(true), "{s:?} {v:?}");
                } else if !s.unreadable() {
                    assert_eq!(s.current(&m), Some(v), "{s:?} {v:?}");
                }
            }
        }
    }

    #[test]
    fn smart_volume_normal_has_no_write_and_the_colour_is_needed_to_light_up() {
        let m = Sample::Live.model();
        assert_eq!(Setting::SmartMode.seg_val(0), None);
        assert!(Setting::SmartMode.action(Val::Int(0), &m).is_err());
        assert!(Setting::Lighting.action(Val::On(true), &m).is_err());
        assert!(Setting::Lighting.action(Val::On(false), &m).is_ok());
        assert_eq!(Setting::EffectLevel(0).text(Val::Int(62)), "62 %");
        assert_eq!(Setting::EffectLevel(0).label(), "SBX Surround");
        assert_eq!(Setting::EqBand(5).text(Val::Int(25)), "+2.5 dB");
    }
}
