//! What the window shows: the last status read from the G6, whether the G6 is there, and the pages.
//!
//! The model is replaced by each whole status read. What a write asked for and hasn't been read back yet is kept apart,
//! in `desk::Desk`, and drawn over it.

use g6_proto::status::DeviceStatus;

use crate::{facts, icons};

/// The pages of the sidebar, in the order of the mockup. `Chain` and `Settings` come after a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Sbx,
    Equalizer,
    Playback,
    Mixer,
    Microphone,
    Lighting,
    Chain,
    Settings,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Sbx,
        Page::Equalizer,
        Page::Playback,
        Page::Mixer,
        Page::Microphone,
        Page::Lighting,
        Page::Chain,
        Page::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Sbx => "SBX Acoustic Engine",
            Page::Equalizer => "Equalizer",
            Page::Playback => "Playback and decoder",
            Page::Mixer => "Mixer",
            Page::Microphone => "Microphone",
            Page::Lighting => "Lighting",
            Page::Chain => "Signal chain",
            Page::Settings => "Settings",
        }
    }

    /// The line under the page's title.
    pub fn about(self) -> &'static str {
        match self {
            Page::Sbx => "Surround, Crystalizer, Bass, Smart Volume and Dialog+, and Scout Mode.",
            Page::Equalizer => "10 bands, ±12 dB.",
            Page::Playback => "Output, volume, speaker setup, decoder, DAC filter and Direct mode.",
            Page::Mixer => "Levels, faders and mutes for monitoring, recording and playback.",
            Page::Microphone => "Mic boost, recording and monitoring volume, Voice Clarity.",
            Page::Lighting => "RGB colour, volume ring and effects.",
            Page::Chain => "The path of the sound. A stage opens its page.",
            Page::Settings => "Device status and appearance.",
        }
    }

    /// The name on the command line and in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Page::Sbx => "sbx",
            Page::Equalizer => "eq",
            Page::Playback => "playback",
            Page::Mixer => "mixer",
            Page::Microphone => "mic",
            Page::Lighting => "lighting",
            Page::Chain => "chain",
            Page::Settings => "system",
        }
    }

    pub fn from_key(s: &str) -> Option<Page> {
        let s = s.trim().to_ascii_lowercase();
        Page::ALL.into_iter().find(|p| p.key() == s)
    }

    pub fn icon(self) -> &'static str {
        match self {
            Page::Sbx => icons::SPARK,
            Page::Equalizer => icons::WAVE,
            Page::Playback => icons::CHIP,
            Page::Mixer => icons::MIXER,
            Page::Microphone => icons::MIC,
            Page::Lighting => icons::BULB,
            Page::Chain => icons::CHAIN,
            Page::Settings => icons::SLIDERS,
        }
    }
}

/// Whether the G6 is there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conn {
    /// A status read is running; nothing has come back yet.
    Reading,
    /// A cache of the last status read is on screen; no read has answered yet. Its values are drawn but the
    /// controls are inert until the first read replaces them.
    Cached,
    Connected,
    /// The last read failed. The text says why.
    Away(String),
}

/// The level and mute of one mixer strip, as Core Audio reports them. `None` is unknown. Playback and recording
/// strips have a Windows percent; monitoring strips only have a level in dB (Core Audio gives no range for them, so
/// no position on a fader).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Strip {
    pub volume: Option<u8>,
    pub db: Option<f64>,
    pub muted: Option<bool>,
}

impl Strip {
    pub fn percent(volume: u8, muted: bool) -> Strip {
        Strip {
            volume: Some(volume),
            db: None,
            muted: Some(muted),
        }
    }
}

/// The mixer values the pages show: monitoring (Line In, External Mic, SPDIF In, Mic), recording sources
/// (Line In, External Mic, SPDIF In, What U Hear, Mic) and the playback master. They come from Windows
/// (`g6_osaudio::read_mixer`), read on the worker thread; a strip Windows doesn't list stays unknown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MixerView {
    pub master: Strip,
    pub monitoring: [Strip; 4],
    pub recording: [Strip; 5],
    pub mix_muted: Option<bool>,
    /// Windows' speaker setup of the default output ("Stereo", "5.1", "7.1", or another name or a hex mask).
    pub speaker_layout: Option<String>,
}

pub const MONITORING: [&str; 4] = ["Line In", "External Mic", "SPDIF In", "Mic"];
pub const RECORDING: [&str; 5] = ["Line In", "External Mic", "SPDIF In", "What U Hear", "Mic"];

impl MixerView {
    /// Maps what Core Audio read onto the strips by name. The master is the default playback endpoint (or the
    /// first one).
    pub fn from_os(m: &g6_osaudio::Mixer) -> MixerView {
        let find = |list: &[(String, g6_osaudio::EndpointEntry)], name: &str| {
            list.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, e)| strip_of(e))
        };
        let master = m
            .playback
            .iter()
            .find(|(_, e)| e.default)
            .or_else(|| m.playback.first())
            .map(|(_, e)| strip_of(e))
            .unwrap_or_default();
        let mut v = MixerView {
            master,
            mix_muted: master.muted,
            speaker_layout: m.speaker_layout.clone().filter(|l| !l.is_empty()),
            ..MixerView::default()
        };
        for (i, name) in RECORDING.iter().enumerate() {
            v.recording[i] = find(&m.recording, name).unwrap_or_default();
        }
        for (i, name) in MONITORING.iter().enumerate() {
            v.monitoring[i] = m
                .monitoring
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, e)| Strip {
                    volume: None,
                    db: e.level_db,
                    muted: e.muted,
                })
                .unwrap_or_default();
        }
        v
    }

    /// Which of `facts::LAYOUTS` Windows has set, when it is one of them.
    pub fn layout_index(&self) -> Option<usize> {
        crate::facts::index_of(&crate::facts::LAYOUTS, self.speaker_layout.as_deref())
    }
}

fn strip_of(e: &g6_osaudio::EndpointEntry) -> Strip {
    Strip::percent(e.level_percent.clamp(0, 100) as u8, e.muted)
}

#[derive(Clone, Debug)]
pub struct Model {
    pub status: DeviceStatus,
    /// A status has been read at least once, so `status` is the device's and not the empty start.
    pub have_status: bool,
    pub conn: Conn,
    pub mixer: Option<MixerView>,
    /// What the last read actually answered. A read can leave queries out (the G6 settling after a write, an output
    /// switch above all); `status` keeps the value known before for each of them, so nothing blanks, but a kept
    /// value is not an answer: `Setting::confirmed` looks here and says "unanswered" rather than "the device has
    /// another one", so a pending value keeps waiting for a real answer.
    pub heard: DeviceStatus,
    /// Some value on screen was kept from an earlier read because the last one left it out.
    pub stale: bool,
    /// The static colour written last in this session: the G6 can't be asked for it.
    pub colour: Option<[u8; 3]>,
}

impl Model {
    /// Before the first read.
    pub fn starting() -> Model {
        Model {
            status: DeviceStatus::default(),
            have_status: false,
            conn: Conn::Reading,
            mixer: None,
            heard: DeviceStatus::default(),
            stale: false,
            colour: None,
        }
    }

    /// The last status read is on screen while the first fresh read runs: the values the window had when it closed,
    /// with every device control inert until the read replaces them.
    pub fn cached(mut status: DeviceStatus, mixer: Option<MixerView>) -> Model {
        facts::drop_direct_zeros(&mut status);
        Model {
            heard: status.clone(),
            status,
            have_status: true,
            conn: Conn::Cached,
            mixer,
            stale: false,
            colour: None,
        }
    }

    pub fn with_status(mut status: DeviceStatus, mixer: Option<MixerView>) -> Model {
        facts::drop_direct_zeros(&mut status);
        Model {
            heard: status.clone(),
            status,
            have_status: true,
            conn: Conn::Connected,
            mixer,
            stale: false,
            colour: None,
        }
    }

    /// A read finished. A failed read keeps the last status on screen and says the G6 is away.
    /// The mixer is replaced with the status, so a read where Windows gave none shows "—" again.
    pub fn apply(&mut self, read: Result<DeviceStatus, String>, mixer: Option<MixerView>) {
        match read {
            Ok(mut status) => {
                let mut heard = status.clone();
                facts::drop_direct_zeros(&mut heard);
                // What this read left out keeps the value known before, so a control never blanks while the G6
                // settles; `heard` says what was really answered.
                self.stale = self.have_status && keep_known(&mut status, &self.status);
                facts::drop_direct_zeros(&mut status);
                self.heard = heard;
                self.status = status;
                self.have_status = true;
                self.conn = Conn::Connected;
                self.mixer = mixer;
            }
            Err(why) => self.conn = Conn::Away(why),
        }
    }

    /// The last read left the output out: the one shown is kept from before.
    #[cfg(test)]
    pub fn output_unanswered(&self) -> bool {
        self.have_status && self.heard.output.is_none()
    }

    /// Direct mode is on: SBX and Scout Mode are bypassed.
    pub fn direct(&self) -> bool {
        self.status.direct_mode == Some(true)
    }
}

/// Fills what `new` left out with what `old` had. Returns whether anything was filled.
fn keep_known(new: &mut DeviceStatus, old: &DeviceStatus) -> bool {
    fn keep<T: Clone>(n: &mut Option<T>, o: &Option<T>) -> bool {
        let fill = n.is_none() && o.is_some();
        if fill {
            n.clone_from(o);
        }
        fill
    }
    let mut kept = false;
    // Inside a part that was answered, each value on its own.
    if let (Some(n), Some(o)) = (new.sbx.as_mut(), old.sbx.as_ref()) {
        for (n, o) in n.iter_mut().zip(o) {
            kept |= keep(&mut n.enabled, &o.enabled);
            kept |= keep(&mut n.level, &o.level);
        }
    }
    if let (Some(n), Some(o)) = (new.eq_bands_db.as_mut(), old.eq_bands_db.as_ref()) {
        for (n, o) in n.iter_mut().zip(o) {
            kept |= keep(n, o);
        }
    }
    if let (Some(n), Some(o)) = (new.recording.as_mut(), old.recording.as_ref()) {
        kept |= keep(&mut n.noise_reduction, &o.noise_reduction);
        kept |= keep(&mut n.noise_reduction_level, &o.noise_reduction_level);
        kept |= keep(
            &mut n.acoustic_echo_cancellation,
            &o.acoustic_echo_cancellation,
        );
        kept |= keep(&mut n.smart_volume, &o.smart_volume);
        kept |= keep(&mut n.mic_eq, &o.mic_eq);
    }
    kept |= keep(&mut new.firmware, &old.firmware);
    kept |= keep(&mut new.output, &old.output);
    kept |= keep(&mut new.decoder_mode, &old.decoder_mode);
    kept |= keep(&mut new.dac_filter, &old.dac_filter);
    kept |= keep(&mut new.direct_mode, &old.direct_mode);
    kept |= keep(&mut new.spdif_direct, &old.spdif_direct);
    kept |= keep(
        &mut new.line_out_headphone_virtualization,
        &old.line_out_headphone_virtualization,
    );
    kept |= keep(&mut new.rgb_enabled, &old.rgb_enabled);
    kept |= keep(&mut new.ring_enabled, &old.ring_enabled);
    kept |= keep(&mut new.mode_flags, &old.mode_flags);
    kept |= keep(&mut new.sbx, &old.sbx);
    kept |= keep(&mut new.smart_volume_mode, &old.smart_volume_mode);
    kept |= keep(&mut new.eq_enabled, &old.eq_enabled);
    kept |= keep(&mut new.eq_preamp_db, &old.eq_preamp_db);
    kept |= keep(&mut new.eq_bands_db, &old.eq_bands_db);
    kept |= keep(&mut new.mic_boost_db, &old.mic_boost_db);
    kept |= keep(&mut new.sbx_enabled, &old.sbx_enabled);
    kept |= keep(&mut new.scout_mode, &old.scout_mode);
    kept |= keep(&mut new.voice_clarity, &old.voice_clarity);
    kept |= keep(&mut new.recording, &old.recording);
    kept |= keep(&mut new.mic_eq_preset, &old.mic_eq_preset);
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_keys_round_trip() {
        for p in Page::ALL {
            assert_eq!(Page::from_key(p.key()), Some(p));
            assert_eq!(Page::from_key(&p.key().to_uppercase()), Some(p));
        }
        assert_eq!(Page::from_key("nope"), None);
    }

    #[test]
    fn a_failed_read_keeps_the_last_status() {
        let mut m = Model::starting();
        assert!(!m.have_status);
        m.apply(Err("no G6".into()), None);
        assert_eq!(m.conn, Conn::Away("no G6".into()));
        let s = DeviceStatus {
            firmware: Some("1".into()),
            ..DeviceStatus::default()
        };
        m.apply(Ok(s), None);
        assert!(m.conn == Conn::Connected && m.have_status);
        m.apply(Err("gone".into()), None);
        assert!(m.conn != Conn::Connected);
        assert_eq!(m.status.firmware.as_deref(), Some("1"));
    }

    #[test]
    fn windows_levels_map_onto_the_strips_by_name() {
        use g6_osaudio::{EndpointEntry, Mixer, MonitoringEntry};
        let ep = |pct, muted, default| EndpointEntry {
            level_percent: pct,
            level_db: -10.0,
            muted,
            default,
        };
        let os = Mixer {
            playback: vec![
                ("Headphones".into(), ep(10, false, false)),
                ("Speakers".into(), ep(37, true, true)),
            ],
            recording: vec![
                ("external mic".into(), ep(80, false, false)),
                ("What U Hear".into(), ep(100, true, true)),
            ],
            monitoring: vec![(
                "Line In".into(),
                MonitoringEntry {
                    level_db: Some(-9.0),
                    muted: Some(true),
                },
            )],
            speaker_layout: Some("5.1".into()),
        };
        let v = MixerView::from_os(&os);
        assert_eq!(v.master, Strip::percent(37, true));
        assert_eq!(v.mix_muted, Some(true));
        assert_eq!(v.recording[1], Strip::percent(80, false));
        assert_eq!(v.recording[3], Strip::percent(100, true));
        assert_eq!(v.recording[0], Strip::default());
        assert_eq!(v.monitoring[0].db, Some(-9.0));
        assert_eq!(v.monitoring[0].volume, None);
        assert_eq!(v.monitoring[1], Strip::default());
        assert_eq!(v.layout_index(), Some(1));
        let odd = MixerView {
            speaker_layout: Some("Quadraphonic".into()),
            ..MixerView::default()
        };
        assert_eq!(odd.layout_index(), None);
    }

    #[test]
    fn a_raw_zero_decoder_and_filter_in_direct_mode_are_not_settings() {
        let s = |direct, dec: &str, filter: &str| DeviceStatus {
            direct_mode: Some(direct),
            decoder_mode: Some(dec.into()),
            dac_filter: Some(filter.into()),
            ..DeviceStatus::default()
        };
        let m = Model::with_status(s(true, "0x00000000", "0x00"), None);
        assert_eq!(
            (&m.status.decoder_mode, &m.status.dac_filter),
            (&None, &None)
        );
        // Other raw codes stay visible, and so does a zero when Direct mode is off.
        let m = Model::with_status(s(true, "0x12345678", "0x07"), None);
        assert!(m.status.decoder_mode.is_some() && m.status.dac_filter.is_some());
        let m = Model::with_status(s(false, "0x00000000", "0x00"), None);
        assert!(m.status.decoder_mode.is_some() && m.status.dac_filter.is_some());
    }

    #[test]
    fn a_read_replaces_the_mixer_and_a_failure_keeps_it() {
        let mut m = Model::starting();
        let v = MixerView::default();
        let s = DeviceStatus {
            firmware: Some("1".into()),
            ..DeviceStatus::default()
        };
        m.apply(Ok(s.clone()), Some(v.clone()));
        assert_eq!(m.mixer, Some(v));
        m.apply(Err("gone".into()), None);
        assert!(m.mixer.is_some());
        m.apply(Ok(s), None);
        assert!(m.mixer.is_none());
    }

    #[test]
    fn a_read_without_the_output_keeps_the_one_known_and_marks_it_unanswered() {
        let mut m = Model::starting();
        let s = DeviceStatus {
            firmware: Some("1".into()),
            output: Some("Headphones".into()),
            ..DeviceStatus::default()
        };
        m.apply(Ok(s.clone()), None);
        assert!(!m.output_unanswered());
        m.apply(Ok(DeviceStatus { output: None, ..s }), None);
        assert_eq!(m.status.output.as_deref(), Some("Headphones"));
        // Kept for the picture, but not an answer from this read.
        assert!(m.output_unanswered());
        m.apply(
            Ok(DeviceStatus {
                output: Some("Speakers".into()),
                ..DeviceStatus::default()
            }),
            None,
        );
        assert!(!m.output_unanswered());
    }

    #[test]
    fn a_read_that_leaves_values_out_keeps_the_ones_known() {
        let mut m = Model::starting();
        let s = DeviceStatus {
            output: Some("Speakers".into()),
            decoder_mode: Some("Normal".into()),
            eq_bands_db: Some([Some(1.0); 10]),
            ..DeviceStatus::default()
        };
        m.apply(Ok(s), None);
        assert!(!m.stale);
        let mut bands = [Some(2.0); 10];
        bands[3] = None;
        m.apply(
            Ok(DeviceStatus {
                output: Some("Headphones".into()),
                eq_bands_db: Some(bands),
                ..DeviceStatus::default()
            }),
            None,
        );
        assert!(m.stale);
        assert_eq!(m.status.output.as_deref(), Some("Headphones"));
        assert_eq!(m.status.decoder_mode.as_deref(), Some("Normal"));
        assert_eq!(m.heard.decoder_mode, None);
        let b = m.status.eq_bands_db.unwrap();
        assert_eq!((b[0], b[3]), (Some(2.0), Some(1.0)));
    }

    #[test]
    fn a_cached_status_is_shown_but_not_confirmed_until_a_read() {
        let mut m = Model::cached(
            DeviceStatus {
                firmware: Some("1".into()),
                output: Some("Headphones".into()),
                ..DeviceStatus::default()
            },
            None,
        );
        assert_eq!(m.conn, Conn::Cached);
        assert!(m.have_status);
        assert_eq!(m.status.output.as_deref(), Some("Headphones"));
        m.apply(
            Ok(DeviceStatus {
                firmware: Some("1".into()),
                output: Some("Speakers".into()),
                ..DeviceStatus::default()
            }),
            None,
        );
        assert_eq!(m.conn, Conn::Connected);
        assert_eq!(m.status.output.as_deref(), Some("Speakers"));
    }
}
