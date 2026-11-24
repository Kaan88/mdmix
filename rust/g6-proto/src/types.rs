//! Enums and value lookups shared by the builders and parsers (Python: `g6_spec/__init__.py` and friends).

use crate::packet::{ProtoError, err};
use crate::tables;

/// Report modes: write a value, then commit (the same `5a 11 03` form also reads a value back).
pub const MODE_DATA: [u8; 2] = [0x12, 0x07];
pub const MODE_COMMIT: [u8; 2] = [0x11, 0x03];

/// Parameter groups in the `intermediate` field.
pub const RECORDING_INTERMEDIATE: [u8; 2] = [0x01, 0x95];
pub const PLAYBACK_INTERMEDIATE: [u8; 2] = [0x01, 0x96];
pub const DECODER_INTERMEDIATE: [u8; 2] = [0x01, 0x97];

pub const ZERO4: [u8; 4] = [0; 4];

/// Speakers (line out) or headphones.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    Speakers,
    Headphones,
}

impl Output {
    pub fn name(self) -> &'static str {
        match self {
            Output::Speakers => "Speakers",
            Output::Headphones => "Headphones",
        }
    }

    pub fn from_name(s: &str) -> Option<Output> {
        match s {
            "Speakers" => Some(Output::Speakers),
            "Headphones" => Some(Output::Headphones),
            _ => None,
        }
    }
}

/// One audio channel, as the `wValue` of a volume request.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Channel {
    Left,
    Right,
}

impl Channel {
    pub fn w_value(self) -> [u8; 2] {
        match self {
            Channel::Left => [0x01, 0x02],
            Channel::Right => [0x02, 0x02],
        }
    }
}

/// The `--...-channels` choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Channels {
    #[default]
    Both,
    Left,
    Right,
}

impl Channels {
    /// Left first. Python once iterated a set here and the order changed from run to run.
    pub fn in_order(self) -> &'static [Channel] {
        match self {
            Channels::Both => &[Channel::Left, Channel::Right],
            Channels::Left => &[Channel::Left],
            Channels::Right => &[Channel::Right],
        }
    }

    pub fn from_name(s: &str) -> Option<Channels> {
        match s {
            "Both" => Some(Channels::Both),
            "Left" => Some(Channels::Left),
            "Right" => Some(Channels::Right),
            _ => None,
        }
    }
}

/// The DAC filter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaybackFilter {
    FastRollOffMinimumPhase,
    SlowRollOffMinimumPhase,
    FastRollOffLinearPhase,
    SlowRollOffLinearPhase,
}

impl PlaybackFilter {
    pub const ALL: [PlaybackFilter; 4] = [
        PlaybackFilter::FastRollOffMinimumPhase,
        PlaybackFilter::SlowRollOffMinimumPhase,
        PlaybackFilter::FastRollOffLinearPhase,
        PlaybackFilter::SlowRollOffLinearPhase,
    ];

    /// The enum name, which is also the CLI value and the status JSON value.
    pub fn name(self) -> &'static str {
        match self {
            PlaybackFilter::FastRollOffMinimumPhase => "FAST_ROLL_OFF_MINIMUM_PHASE",
            PlaybackFilter::SlowRollOffMinimumPhase => "SLOW_ROLL_OFF_MINIMUM_PHASE",
            PlaybackFilter::FastRollOffLinearPhase => "FAST_ROLL_OFF_LINEAR_PHASE",
            PlaybackFilter::SlowRollOffLinearPhase => "SLOW_ROLL_OFF_LINEAR_PHASE",
        }
    }

    /// The label `--status` prints.
    pub fn label(self) -> &'static str {
        match self {
            PlaybackFilter::FastRollOffMinimumPhase => "Fast Roll-off / Minimum Phase",
            PlaybackFilter::SlowRollOffMinimumPhase => "Slow Roll-off / Minimum Phase",
            PlaybackFilter::FastRollOffLinearPhase => "Fast Roll-off / Linear Phase",
            PlaybackFilter::SlowRollOffLinearPhase => "Slow Roll-off / Linear Phase",
        }
    }

    pub fn code(self) -> [u8; 2] {
        match self {
            PlaybackFilter::FastRollOffMinimumPhase => [0x00, 0x01],
            PlaybackFilter::SlowRollOffMinimumPhase => [0x00, 0x02],
            PlaybackFilter::FastRollOffLinearPhase => [0x00, 0x04],
            PlaybackFilter::SlowRollOffLinearPhase => [0x00, 0x05],
        }
    }

    pub fn from_name(s: &str) -> Option<PlaybackFilter> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}

/// Decoder mode, sent as a float32 LE (2.0, 1.0, 3.0).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecoderMode {
    Normal,
    Full,
    Night,
}

impl DecoderMode {
    pub fn value(self) -> [u8; 4] {
        match self {
            DecoderMode::Normal => [0x00, 0x00, 0x00, 0x40],
            DecoderMode::Full => [0x00, 0x00, 0x80, 0x3f],
            DecoderMode::Night => [0x00, 0x00, 0x40, 0x40],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DecoderMode::Normal => "Normal",
            DecoderMode::Full => "Full",
            DecoderMode::Night => "Night",
        }
    }

    pub fn from_name(s: &str) -> Option<DecoderMode> {
        match s {
            "Normal" => Some(DecoderMode::Normal),
            "Full" => Some(DecoderMode::Full),
            "Night" => Some(DecoderMode::Night),
            _ => None,
        }
    }
}

/// The five SBX effects, each with a switch and a level parameter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SbxEffect {
    Surround,
    Crystalizer,
    Bass,
    SmartVolume,
    DialogPlus,
}

impl SbxEffect {
    /// In the order Python applies and reports them.
    pub const ALL: [SbxEffect; 5] = [
        SbxEffect::Surround,
        SbxEffect::Crystalizer,
        SbxEffect::Bass,
        SbxEffect::SmartVolume,
        SbxEffect::DialogPlus,
    ];

    /// (switch id, level id).
    pub fn ids(self) -> (u8, u8) {
        match self {
            SbxEffect::Surround => (0x00, 0x01),
            SbxEffect::Crystalizer => (0x07, 0x08),
            SbxEffect::Bass => (0x18, 0x19),
            SbxEffect::SmartVolume => (0x04, 0x05),
            SbxEffect::DialogPlus => (0x02, 0x03),
        }
    }

    /// The key in the status JSON.
    pub fn key(self) -> &'static str {
        match self {
            SbxEffect::Surround => "surround",
            SbxEffect::Crystalizer => "crystalizer",
            SbxEffect::Bass => "bass",
            SbxEffect::SmartVolume => "smart_volume",
            SbxEffect::DialogPlus => "dialog_plus",
        }
    }
}

/// The smart volume mode parameter id.
pub const SMART_VOLUME_SPECIAL_ID: u8 = 0x06;

/// Smart volume's Night and Loud modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SmartVolumeSpecial {
    Night,
    Loud,
}

impl SmartVolumeSpecial {
    pub fn value(self) -> [u8; 4] {
        match self {
            SmartVolumeSpecial::Night => [0x00, 0x00, 0x00, 0x40],
            SmartVolumeSpecial::Loud => [0x00, 0x00, 0x80, 0x3f],
        }
    }
}

/// A mic EQ preset, by index into [`tables::MIC_EQ_PRESETS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MicEqPreset(usize);

impl MicEqPreset {
    pub fn all() -> impl Iterator<Item = MicEqPreset> {
        (0..tables::MIC_EQ_PRESETS.len()).map(MicEqPreset)
    }

    pub fn from_name(s: &str) -> Option<MicEqPreset> {
        tables::MIC_EQ_PRESETS
            .iter()
            .position(|(n, _)| *n == s)
            .map(MicEqPreset)
    }

    pub fn name(self) -> &'static str {
        tables::MIC_EQ_PRESETS[self.0].0
    }

    /// The float32 LE gains for band ids 0x14..=0x1b.
    pub fn values(self) -> &'static [[u8; 4]; 8] {
        &tables::MIC_EQ_PRESETS[self.0].1
    }
}

/// The SBX master switch and Scout Mode (they exclude each other).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessingMode {
    Sbx = 0x01,
    ScoutMode = 0x02,
}

/// The mixer's sources, in the order Python applies them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MixerSource {
    MonitoringLineIn,
    MonitoringExternalMic,
    MonitoringSpdifIn,
    RecordingLineIn,
    RecordingExternalMic,
    RecordingSpdifIn,
    RecordingWhatUHear,
}

impl MixerSource {
    pub const ALL: [MixerSource; 7] = [
        MixerSource::MonitoringLineIn,
        MixerSource::MonitoringExternalMic,
        MixerSource::MonitoringSpdifIn,
        MixerSource::RecordingLineIn,
        MixerSource::RecordingExternalMic,
        MixerSource::RecordingSpdifIn,
        MixerSource::RecordingWhatUHear,
    ];

    /// The unit (`wIndex`) the request goes to.
    pub fn w_index(self) -> [u8; 2] {
        match self {
            MixerSource::MonitoringLineIn => MONITORING_LINE_IN,
            MixerSource::MonitoringExternalMic => MONITORING_EXTERNAL_MIC,
            MixerSource::MonitoringSpdifIn => MONITORING_SPDIF_IN,
            MixerSource::RecordingLineIn => RECORDING_LINE_IN,
            MixerSource::RecordingExternalMic => RECORDING_EXTERNAL_MIC,
            MixerSource::RecordingSpdifIn => RECORDING_SPDIF_IN,
            MixerSource::RecordingWhatUHear => RECORDING_WHAT_U_HEAR,
        }
    }

    /// Monitoring uses the playback volume scale, recording its own.
    pub fn is_monitoring(self) -> bool {
        matches!(
            self,
            MixerSource::MonitoringLineIn
                | MixerSource::MonitoringExternalMic
                | MixerSource::MonitoringSpdifIn
        )
    }
}

// Units (wIndex), as the Python builders store them.
pub const PLAYBACK_PLAYBACK: [u8; 2] = [0x00, 0x01];
pub const MONITORING_LINE_IN: [u8; 2] = [0x00, 0x09];
pub const MONITORING_EXTERNAL_MIC: [u8; 2] = [0x00, 0x0a];
pub const MONITORING_SPDIF_IN: [u8; 2] = [0x00, 0x0c];
pub const RECORDING_LINE_IN: [u8; 2] = [0x00, 0x03];
pub const RECORDING_EXTERNAL_MIC: [u8; 2] = [0x00, 0x04];
pub const RECORDING_SPDIF_IN: [u8; 2] = [0x00, 0x05];
pub const RECORDING_WHAT_U_HEAR: [u8; 2] = [0x00, 0x06];

/// `wValue` of a mute request.
pub const MUTE_W_VALUE: [u8; 2] = [0x00, 0x01];

/// A range with a step, as Python's `ValueRange`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ValueRange {
    pub min: i64,
    pub step: i64,
    pub max: i64,
}

impl ValueRange {
    pub fn contains(self, v: i64) -> bool {
        (self.min..=self.max).contains(&v) && v % self.step == 0
    }

    /// Every allowed value, for CLI choices.
    pub fn values(self) -> impl Iterator<Item = i64> {
        (self.min..=self.max).step_by(self.step as usize)
    }
}

pub const SLIDER_VR: ValueRange = ValueRange {
    min: 0,
    step: 1,
    max: 100,
};
pub const VOLUME_VR: ValueRange = ValueRange {
    min: 0,
    step: 10,
    max: 100,
};
pub const MIC_BOOST_VR: ValueRange = ValueRange {
    min: 0,
    step: 10,
    max: 30,
};
pub const NOISE_REDUCTION_VR: ValueRange = ValueRange {
    min: 0,
    step: 20,
    max: 100,
};
pub const RGB_VR: ValueRange = ValueRange {
    min: 0,
    step: 1,
    max: 255,
};

fn lookup<const N: usize>(table: &[(u8, [u8; N])], v: i64) -> Option<[u8; N]> {
    table
        .iter()
        .find(|(k, _)| i64::from(*k) == v)
        .map(|(_, b)| *b)
}

pub fn slider_percent_bytes(v: i64) -> Option<[u8; 4]> {
    lookup(&tables::SLIDER_PERCENT, v)
}

pub fn playback_volume_bytes(v: i64) -> Option<[u8; 2]> {
    lookup(&tables::PLAYBACK_VOLUME, v)
}

/// Monitoring shares the playback table (as in Python).
pub fn monitoring_volume_bytes(v: i64) -> Option<[u8; 2]> {
    playback_volume_bytes(v)
}

pub fn mic_recording_volume_bytes(v: i64) -> Option<[u8; 2]> {
    lookup(&tables::MIC_RECORDING_VOLUME, v)
}

pub fn mic_boost_bytes(v: i64) -> Option<[u8; 4]> {
    lookup(&tables::MIC_BOOST_DB, v)
}

pub fn voice_clarity_level_bytes(v: i64) -> Option<[u8; 4]> {
    lookup(&tables::VOICE_CLARITY_LEVEL, v)
}

/// Range check with Python's two messages ("between ..." and "multiple of ...").
pub(crate) fn check_range(
    v: i64,
    vr: ValueRange,
    between: &str,
    multiple: &str,
) -> Result<(), ProtoError> {
    if v < vr.min || v > vr.max {
        return err(format!("{between}, got {v}"));
    }
    if v % vr.step != 0 {
        return err(format!("{multiple}, got {v}"));
    }
    Ok(())
}
