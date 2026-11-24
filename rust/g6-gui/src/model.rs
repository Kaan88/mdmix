//! What the window shows: the last status read from the G6, whether the G6 is there, and the pages.
//!
//! In step G2 the model is only ever replaced by a whole status read. Nothing here writes.

use g6_proto::status::DeviceStatus;

use crate::icons;

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
    Connected,
    /// The last read failed. The text says why.
    Away(String),
}

/// The level and mute of one mixer strip, as Core Audio reports them. `None` is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strip {
    pub volume: Option<u8>,
    pub muted: Option<bool>,
}

/// The mixer values the pages show: monitoring (Line In, External Mic, SPDIF In, Mic), recording sources
/// (Line In, External Mic, SPDIF In, What U Hear, Mic) and the playback master.
///
/// Step G2 has no source for it: `DeviceStatus` has no mixer field on this branch, so the model holds `None` and the
/// Mixer page and the sidebar volume show "—". When `g6-osaudio` fills `DeviceStatus.mixer`, map it onto this in
/// `Model::apply` and nothing else changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MixerView {
    pub master: Strip,
    pub monitoring: [Strip; 4],
    pub recording: [Strip; 5],
    pub mix_muted: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub status: DeviceStatus,
    /// A status has been read at least once, so `status` is the device's and not the empty start.
    pub have_status: bool,
    pub conn: Conn,
    pub mixer: Option<MixerView>,
}

impl Model {
    /// Before the first read.
    pub fn starting() -> Model {
        Model {
            status: DeviceStatus::default(),
            have_status: false,
            conn: Conn::Reading,
            mixer: None,
        }
    }

    pub fn with_status(status: DeviceStatus, mixer: Option<MixerView>) -> Model {
        Model {
            status,
            have_status: true,
            conn: Conn::Connected,
            mixer,
        }
    }

    /// A read finished. A failed read keeps the last status on screen and says the G6 is away.
    pub fn apply(&mut self, read: Result<DeviceStatus, String>) {
        match read {
            Ok(status) => {
                self.status = status;
                self.have_status = true;
                self.conn = Conn::Connected;
            }
            Err(why) => self.conn = Conn::Away(why),
        }
    }

    /// Direct mode is on: SBX and Scout Mode are bypassed.
    pub fn direct(&self) -> bool {
        self.status.direct_mode == Some(true)
    }
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
        m.apply(Err("no G6".into()));
        assert_eq!(m.conn, Conn::Away("no G6".into()));
        let s = DeviceStatus {
            firmware: Some("1".into()),
            ..DeviceStatus::default()
        };
        m.apply(Ok(s));
        assert!(m.conn == Conn::Connected && m.have_status);
        m.apply(Err("gone".into()));
        assert!(m.conn != Conn::Connected);
        assert_eq!(m.status.firmware.as_deref(), Some("1"));
    }
}
