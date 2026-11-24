//! The order settings are written in. This is the only place that holds it (Python: `device_set_audio_effects`).
//!
//! A [`Request`] holds the settings to change. [`Request::actions`] lists them in write order, and each
//! [`Action`] turns into the batches to send only when it is applied. A bad value then fails at the same point
//! as in Python, after the writes before it went out.

use crate::packet::{Batch, ProtoError, err};
use crate::pyfmt::{py_float, py_str_repr};
use crate::types::{
    Channels, DecoderMode, MicEqPreset, MixerSource, Output, PlaybackFilter, ProcessingMode,
    SbxEffect, SmartVolumeSpecial,
};
use crate::{decoder, eq_presets, equalizer, lighting, mixer, modes, playback, recording, sbx};

/// A volume and the channels it applies to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Volume {
    pub percent: i64,
    pub channels: Channels,
}

/// The switch and level of one SBX effect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SbxSetting {
    pub enabled: Option<bool>,
    pub level: Option<i64>,
}

/// The mute and volume of one mixer source.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MixerSetting {
    pub mute: Option<bool>,
    pub volume: Option<Volume>,
}

/// Every setting the CLI can change; `None` / `false` leaves it alone. Mutes take `true` for muted.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Request {
    pub toggle_output: bool,
    pub set_output: Option<Output>,
    pub playback_mute: Option<bool>,
    pub playback_volume: Option<Volume>,
    pub speakers_to_stereo: bool,
    pub speakers_to_5_1: bool,
    pub speakers_to_7_1: bool,
    pub headphones_to_stereo: bool,
    pub headphones_to_5_1: bool,
    pub headphones_to_7_1: bool,
    pub direct_mode: Option<bool>,
    pub spdif_out_direct_mode: Option<bool>,
    pub playback_filter: Option<PlaybackFilter>,
    pub decoder_mode: Option<DecoderMode>,
    pub lighting_disable: bool,
    /// Checked when applied, as Python does.
    pub lighting_rgb: Option<[i64; 3]>,
    pub lighting_volume_ring: Option<bool>,
    pub mixer_playback_mute: Option<bool>,
    /// In [`MixerSource::ALL`] order.
    pub mixer: [MixerSetting; 7],
    pub recording_mute: Option<bool>,
    pub mic_recording_volume: Option<Volume>,
    pub mic_boost_db: Option<i64>,
    pub mic_monitoring_mute: Option<bool>,
    pub mic_monitoring_volume: Option<Volume>,
    pub voice_clarity: Option<bool>,
    pub noise_reduction_level: Option<i64>,
    pub aec: Option<bool>,
    pub mic_smart_volume: Option<bool>,
    pub mic_eq: Option<bool>,
    pub mic_eq_preset: Option<MicEqPreset>,
    /// In [`SbxEffect::ALL`] order.
    pub sbx_effects: [SbxSetting; 5],
    pub smart_volume_special: Option<SmartVolumeSpecial>,
    pub sbx: Option<bool>,
    pub scout_mode: Option<bool>,
    pub eq: Option<bool>,
    /// A preset id (see [`eq_presets::find_eq_preset`]) and the output whose curve to use; without one, the
    /// current output.
    pub eq_preset: Option<(String, Option<Output>)>,
    pub eq_preamp: Option<f64>,
    /// Band label and gain as typed; both are checked when applied, as Python does.
    pub eq_bands: Vec<(String, String)>,
    pub line_out_headphone_virtualization: Option<bool>,
}

/// One API call of Python's `G6Api`, apart from [`Action::EqPreset`], which stands for the preamp and ten band
/// calls Python makes for a preset.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    ToggleOutput,
    SetOutput(Output),
    PlaybackMute(bool),
    PlaybackVolume(Volume),
    SpeakersToStereo,
    SpeakersTo5_1,
    SpeakersTo7_1,
    HeadphonesToStereo,
    HeadphonesTo5_1,
    HeadphonesTo7_1,
    DirectMode(bool),
    SpdifOutDirectMode(bool),
    PlaybackFilter(PlaybackFilter),
    DecoderMode(DecoderMode),
    LightingDisable,
    LightingRgb([i64; 3]),
    LightingVolumeRing(bool),
    MixerPlaybackMute(bool),
    MixerMute(MixerSource, bool),
    MixerVolume(MixerSource, Volume),
    RecordingMute(bool),
    MicRecordingVolume(Volume),
    MicBoost(i64),
    MicMonitoringMute(bool),
    MicMonitoringVolume(Volume),
    VoiceClarity(bool),
    NoiseReductionLevel(i64),
    Aec(bool),
    MicSmartVolume(bool),
    MicEq(bool),
    MicEqPreset(MicEqPreset),
    SbxToggle(SbxEffect, bool),
    SbxLevel(SbxEffect, i64),
    SmartVolumeSpecial(SmartVolumeSpecial),
    ProcessingMode(ProcessingMode, bool),
    Eq(bool),
    EqPreset(String, Option<Output>),
    EqPreamp(f64),
    EqBand(String, String),
    LineOutHeadphoneVirtualization(bool),
}

/// What applying some actions needs to know about the host and device.
pub trait Host {
    /// For `--toggle-output`: Python's `determine_toggle_state`, the state it records for next time.
    fn next_toggle_state(&mut self) -> Result<Output, ProtoError>;
    /// For a preset without an output: the output the G6 reports, Headphones when unknown.
    fn current_output(&mut self) -> Output;
}

impl Request {
    /// The actions for this request, in the order they are written.
    pub fn actions(&self) -> Vec<Action> {
        use Action as A;
        let mut v = Vec::new();
        let mut opt = |a: Option<Action>| v.extend(a);

        // playback
        if self.toggle_output {
            opt(Some(A::ToggleOutput));
        } else {
            opt(self.set_output.map(A::SetOutput));
        }
        opt(self.playback_mute.map(A::PlaybackMute));
        opt(self.playback_volume.map(A::PlaybackVolume));
        for (on, a) in [
            (self.speakers_to_stereo, A::SpeakersToStereo),
            (self.speakers_to_5_1, A::SpeakersTo5_1),
            (self.speakers_to_7_1, A::SpeakersTo7_1),
            (self.headphones_to_stereo, A::HeadphonesToStereo),
            (self.headphones_to_5_1, A::HeadphonesTo5_1),
            (self.headphones_to_7_1, A::HeadphonesTo7_1),
        ] {
            opt(on.then_some(a));
        }
        opt(self.direct_mode.map(A::DirectMode));
        opt(self.spdif_out_direct_mode.map(A::SpdifOutDirectMode));
        opt(self.playback_filter.map(A::PlaybackFilter));
        opt(self.decoder_mode.map(A::DecoderMode));

        // lighting
        opt(self.lighting_disable.then_some(A::LightingDisable));
        opt(self.lighting_rgb.map(A::LightingRgb));
        opt(self.lighting_volume_ring.map(A::LightingVolumeRing));

        // mixer: each source's mute, then its volume
        opt(self.mixer_playback_mute.map(A::MixerPlaybackMute));
        for (src, m) in MixerSource::ALL.into_iter().zip(self.mixer) {
            opt(m.mute.map(|x| A::MixerMute(src, x)));
            opt(m.volume.map(|x| A::MixerVolume(src, x)));
        }

        // recording
        opt(self.recording_mute.map(A::RecordingMute));
        opt(self.mic_recording_volume.map(A::MicRecordingVolume));
        opt(self.mic_boost_db.map(A::MicBoost));
        opt(self.mic_monitoring_mute.map(A::MicMonitoringMute));
        opt(self.mic_monitoring_volume.map(A::MicMonitoringVolume));
        opt(self.voice_clarity.map(A::VoiceClarity));
        opt(self.noise_reduction_level.map(A::NoiseReductionLevel));
        opt(self.aec.map(A::Aec));
        opt(self.mic_smart_volume.map(A::MicSmartVolume));
        opt(self.mic_eq.map(A::MicEq));
        opt(self.mic_eq_preset.map(A::MicEqPreset));

        // SBX: switch then level per effect; smart volume's Night/Loud right after its level
        for (effect, s) in SbxEffect::ALL.into_iter().zip(self.sbx_effects) {
            opt(s.enabled.map(|x| A::SbxToggle(effect, x)));
            opt(s.level.map(|x| A::SbxLevel(effect, x)));
            if effect == SbxEffect::SmartVolume {
                opt(self.smart_volume_special.map(A::SmartVolumeSpecial));
            }
        }

        // SBX and Scout Mode exclude each other: switch one off before the other goes on. Python sorts by
        // "is Enabled" (stable), so a mode being disabled or left alone comes first.
        let mut pm = [
            (ProcessingMode::Sbx, self.sbx),
            (ProcessingMode::ScoutMode, self.scout_mode),
        ];
        pm.sort_by_key(|(_, value)| *value == Some(true));
        for (mode, value) in pm {
            opt(value.map(|x| A::ProcessingMode(mode, x)));
        }

        // equalizer: a preset first, so an explicit preamp or band given with it wins
        opt(self.eq.map(A::Eq));
        opt(self
            .eq_preset
            .clone()
            .map(|(id, output)| A::EqPreset(id, output)));
        opt(self.eq_preamp.map(A::EqPreamp));
        for (freq, db) in &self.eq_bands {
            opt(Some(A::EqBand(freq.clone(), db.clone())));
        }

        opt(self
            .line_out_headphone_virtualization
            .map(A::LineOutHeadphoneVirtualization));
        v
    }
}

fn hid(reports: Vec<crate::packet::HidReport>) -> Vec<Batch> {
    vec![Batch::Hid(reports)]
}

fn usb(requests: Vec<crate::packet::UsbRequest>) -> Vec<Batch> {
    vec![Batch::Usb(requests)]
}

impl Action {
    /// The batches this action sends, one per Python API call.
    pub fn batches(&self, host: &mut dyn Host) -> Result<Vec<Batch>, ProtoError> {
        use Action as A;
        Ok(match self {
            A::ToggleOutput => {
                // Python sends the opposite of the state determine_toggle_state returns (see doc/rust-port.md)
                hid(match host.next_toggle_state()? {
                    Output::Speakers => playback::toggle_to_headphones(),
                    Output::Headphones => playback::toggle_to_speakers(),
                })
            }
            A::SetOutput(Output::Speakers) => hid(playback::toggle_to_speakers()),
            A::SetOutput(Output::Headphones) => hid(playback::toggle_to_headphones()),
            A::PlaybackMute(m) => usb(playback::playback_mute(*m)),
            A::PlaybackVolume(v) => usb(playback::playback_volume(v.percent, v.channels)?),
            A::SpeakersToStereo
            | A::SpeakersTo5_1
            | A::SpeakersTo7_1
            | A::HeadphonesToStereo
            | A::HeadphonesTo5_1
            | A::HeadphonesTo7_1 => usb(playback::speaker_layout()),
            A::DirectMode(e) => hid(playback::enable_direct_mode(*e)),
            A::SpdifOutDirectMode(e) => hid(playback::enable_spdif_out_direct_mode(*e)),
            A::PlaybackFilter(f) => hid(playback::playback_filter(*f)),
            A::DecoderMode(m) => hid(decoder::decoder_mode(*m)),
            A::LightingDisable => hid(lighting::lighting_disable()),
            A::LightingRgb([r, g, b]) => hid(lighting::lighting_enable_set_rgb(*r, *g, *b)?),
            A::LightingVolumeRing(e) => hid(lighting::lighting_volume_ring(*e)),
            A::MixerPlaybackMute(m) => usb(mixer::playback_mute(*m)),
            A::MixerMute(src, m) => usb(mixer::mute(*src, *m)),
            A::MixerVolume(src, v) => usb(mixer::volume(*src, v.percent, v.channels)?),
            A::RecordingMute(m) => usb(recording::recording_mute(*m)),
            A::MicRecordingVolume(v) => {
                usb(recording::mic_recording_volume(v.percent, v.channels)?)
            }
            A::MicBoost(db) => hid(recording::mic_boost(*db)?),
            A::MicMonitoringMute(m) => usb(recording::mic_monitoring_mute(*m)),
            A::MicMonitoringVolume(v) => {
                usb(recording::mic_monitoring_volume(v.percent, v.channels)?)
            }
            A::VoiceClarity(e) => hid(recording::voice_clarity_enabled(*e)),
            A::NoiseReductionLevel(l) => hid(recording::voice_clarity_noise_reduction_level(*l)?),
            A::Aec(e) => hid(recording::voice_clarity_acoustic_echo_cancellation_enabled(
                *e,
            )),
            A::MicSmartVolume(e) => hid(recording::voice_clarity_smart_volume_enabled(*e)),
            A::MicEq(e) => hid(recording::voice_clarity_mic_equalizer_enabled(*e)),
            A::MicEqPreset(p) => hid(recording::voice_clarity_mic_equalizer_preset(*p)),
            A::SbxToggle(effect, on) => hid(sbx::sbx_toggle(*effect, *on)),
            A::SbxLevel(effect, v) => hid(sbx::sbx_slider(*effect, *v)?),
            A::SmartVolumeSpecial(m) => hid(sbx::sbx_smart_volume_special(*m)),
            A::ProcessingMode(mode, on) => hid(modes::processing_mode(*mode, *on)),
            A::Eq(e) => hid(equalizer::equalizer_enabled(*e)),
            A::EqPreset(id, output) => {
                let output = match output {
                    Some(o) => *o,
                    None => host.current_output(),
                };
                let (preamp, bands) = eq_presets::eq_preset_values(id, output)?;
                let mut v = hid(equalizer::equalizer_preamp(preamp)?);
                for (freq, db) in bands {
                    v.push(Batch::Hid(equalizer::equalizer_band(freq, db)?));
                }
                v
            }
            A::EqPreamp(db) => hid(equalizer::equalizer_preamp(*db)?),
            A::EqBand(freq, db) => {
                // Python converts the gain before it looks at the band
                let Some(db) = py_float(db) else {
                    return err(format!(
                        "could not convert string to float: {}",
                        py_str_repr(db)
                    ));
                };
                hid(equalizer::equalizer_band(freq, db)?)
            }
            A::LineOutHeadphoneVirtualization(e) => {
                hid(playback::enable_line_out_headphone_virtualization(*e))
            }
        })
    }
}
