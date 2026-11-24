//! Fixed device states for `--shot`, so a page can be drawn without a G6, and for tests.
//!
//! `live` is the parsed transcript in `tests/vectors/status_live.json` (a real G6, Speakers, Direct mode on, effects
//! off), written out here; a test feeds the transcript through `collect_status` and compares. `busy` is the same G6
//! with Direct mode off and things switched on, so a shot shows the lit states too: it is invented, not read.

use g6_proto::status::{DeviceStatus, RecordingStatus, SbxState};

use crate::model::{MixerView, Model, Strip};

pub fn live_status() -> DeviceStatus {
    let sbx = |level| SbxState {
        enabled: Some(false),
        level: Some(level),
    };
    let mut bands = [None; 10];
    for (b, v) in bands
        .iter_mut()
        .zip([0.0, 0.0, 2.0, 2.5, 1.5, 2.5, 0.0, -2.5, -1.0, -1.5])
    {
        *b = Some(v);
    }
    DeviceStatus {
        firmware: Some("2.1.250903.1324".into()),
        output: Some("Speakers".into()),
        decoder_mode: Some("Normal".into()),
        dac_filter: Some("SLOW_ROLL_OFF_MINIMUM_PHASE".into()),
        direct_mode: Some(true),
        spdif_direct: Some(false),
        line_out_headphone_virtualization: Some(false),
        rgb_enabled: Some(false),
        ring_enabled: Some(false),
        mode_flags: Some((0x20, 0x40)),
        sbx: Some([sbx(50), sbx(50), sbx(50), sbx(74), sbx(50)]),
        smart_volume_mode: Some("Normal".into()),
        eq_enabled: Some(false),
        eq_preamp_db: Some(0.0),
        eq_bands_db: Some(bands),
        mic_boost_db: Some(0),
        sbx_enabled: Some(false),
        scout_mode: Some(false),
        voice_clarity: Some(false),
        recording: Some(RecordingStatus {
            noise_reduction: Some(false),
            noise_reduction_level: Some(0),
            acoustic_echo_cancellation: Some(false),
            smart_volume: Some(false),
            mic_eq: Some(false),
        }),
        mic_eq_preset: Some("PRESET_1".into()),
    }
}

pub fn busy_status() -> DeviceStatus {
    let mut s = live_status();
    s.direct_mode = Some(false);
    s.output = Some("Headphones".into());
    s.rgb_enabled = Some(true);
    s.ring_enabled = Some(true);
    s.sbx_enabled = Some(true);
    s.eq_enabled = Some(true);
    s.eq_preamp_db = Some(-1.5);
    s.mic_boost_db = Some(20);
    s.voice_clarity = Some(true);
    s.decoder_mode = Some("Night".into());
    s.mic_eq_preset = Some("PRESET_4".into());
    let on = |level| SbxState {
        enabled: Some(true),
        level: Some(level),
    };
    let off = |level| SbxState {
        enabled: Some(false),
        level: Some(level),
    };
    s.sbx = Some([on(67), on(50), off(50), off(70), off(50)]);
    s.recording = Some(RecordingStatus {
        noise_reduction: Some(true),
        noise_reduction_level: Some(60),
        acoustic_echo_cancellation: Some(true),
        smart_volume: Some(false),
        mic_eq: Some(true),
    });
    s
}

/// Mixer values for a shot of how the Mixer page looks with values. Invented: no real Core Audio read is kept in the
/// repository, so the Live sample has none.
pub fn busy_mixer() -> MixerView {
    let s = Strip::percent;
    let mon = |db, muted| Strip {
        volume: None,
        db: Some(db),
        muted: Some(muted),
    };
    MixerView {
        master: s(34, false),
        monitoring: [
            mon(-9.0, true),
            mon(8.1, true),
            mon(0.0, false),
            mon(-3.5, true),
        ],
        recording: [
            s(50, true),
            s(80, false),
            s(50, true),
            s(100, true),
            s(70, false),
        ],
        mix_muted: Some(false),
        speaker_layout: Some("5.1".into()),
    }
}

/// Which fixed state a shot draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// The real transcript. It has no mixer: Windows' levels were not recorded with it, so they show "—".
    Live,
    /// Invented values in every field, mixer included.
    Busy,
    /// No G6: the last status stays and the banner shows.
    Away,
}

impl Sample {
    pub fn from_name(s: &str) -> Option<Sample> {
        match s.to_ascii_lowercase().as_str() {
            "live" => Some(Sample::Live),
            "busy" => Some(Sample::Busy),
            "away" => Some(Sample::Away),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sample::Live => "live",
            Sample::Busy => "busy",
            Sample::Away => "away",
        }
    }

    pub fn model(self) -> Model {
        match self {
            Sample::Live => Model::with_status(live_status(), None),
            Sample::Busy => Model::with_status(busy_status(), Some(busy_mixer())),
            Sample::Away => {
                let mut m = Model::with_status(live_status(), None);
                m.conn = crate::model::Conn::Away(
                    "No G6 could be found. Is it connected to this computer?".into(),
                );
                m
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g6_proto::packet::{pad_report, unhex};
    use g6_proto::status::collect_status;

    /// `live_status` is what the recorded transcript parses to.
    #[test]
    fn the_live_sample_is_the_parsed_transcript() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/vectors/status_live.json");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let exchanges = v["exchanges"].as_array().unwrap();
        let mut i = 0;
        let got = collect_status(|_| {
            let ex = &exchanges[i];
            i += 1;
            ex["response"]
                .as_str()
                .map(|r| pad_report(&unhex(r).unwrap()).unwrap())
        });
        assert_eq!(got, live_status());
    }

    #[test]
    fn names_round_trip() {
        for s in [Sample::Live, Sample::Busy, Sample::Away] {
            assert_eq!(Sample::from_name(s.name()), Some(s));
        }
    }
}
