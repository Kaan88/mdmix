//! `--status` output, as text or JSON (Python: `format_status`, `DeviceStatus.to_dict`).
//!
//! The mixer part (volumes from the OS audio stack, `g6-osaudio`) is passed in separately; without it it is `null` /
//! left out.

use crate::pyval::{PyVal, dumps_indent2};
use g6_osaudio::{EndpointEntry, Mixer, format_mixer_rows};
use g6_proto::equalizer::EQ_BAND_FREQS;
use g6_proto::pyfmt::py_g;
use g6_proto::status::DeviceStatus;
use g6_proto::types::{PlaybackFilter, SbxEffect};

fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> PyVal) -> PyVal {
    v.map_or(PyVal::None, f)
}

fn s(v: &Option<String>) -> PyVal {
    opt(v.as_ref(), PyVal::str)
}

fn b(v: Option<bool>) -> PyVal {
    opt(v, PyVal::Bool)
}

fn dict(items: Vec<(&str, PyVal)>) -> PyVal {
    PyVal::Dict(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// `DeviceStatus.to_dict()` with no mixer.
pub fn status_to_pyval(st: &DeviceStatus) -> PyVal {
    status_to_pyval_with_mixer(st, None)
}

fn endpoint_dict(items: &[(String, EndpointEntry)]) -> PyVal {
    PyVal::Dict(
        items
            .iter()
            .map(|(name, e)| {
                let v = dict(vec![
                    ("level_percent", PyVal::int(e.level_percent)),
                    ("level_db", PyVal::Float(e.level_db)),
                    ("muted", PyVal::Bool(e.muted)),
                    ("default", PyVal::Bool(e.default)),
                ]);
                (name.clone(), v)
            })
            .collect(),
    )
}

/// The `mixer` dict of `read_mixer()`.
pub fn mixer_to_pyval(m: &Mixer) -> PyVal {
    let monitoring = PyVal::Dict(
        m.monitoring
            .iter()
            .map(|(name, e)| {
                let v = dict(vec![
                    ("level_db", opt(e.level_db, PyVal::Float)),
                    ("muted", b(e.muted)),
                ]);
                (name.clone(), v)
            })
            .collect(),
    );
    dict(vec![
        ("playback", endpoint_dict(&m.playback)),
        ("recording", endpoint_dict(&m.recording)),
        ("monitoring", monitoring),
        ("speaker_layout", s(&m.speaker_layout)),
    ])
}

/// `DeviceStatus.to_dict()` with the mixer read separately.
pub fn status_to_pyval_with_mixer(st: &DeviceStatus, mixer: Option<&Mixer>) -> PyVal {
    let sbx = opt(st.sbx.as_ref(), |effects| {
        PyVal::Dict(
            SbxEffect::ALL
                .iter()
                .zip(effects)
                .map(|(e, state)| {
                    let v = dict(vec![
                        ("enabled", b(state.enabled)),
                        ("level", opt(state.level, PyVal::int)),
                    ]);
                    (e.key().to_string(), v)
                })
                .collect(),
        )
    });
    let bands = opt(st.eq_bands_db.as_ref(), |bands| {
        PyVal::Dict(
            EQ_BAND_FREQS
                .iter()
                .zip(bands)
                .map(|(f, db)| (f.to_string(), opt(*db, PyVal::Float)))
                .collect(),
        )
    });
    let recording = opt(st.recording.as_ref(), |r| {
        dict(vec![
            ("noise_reduction", b(r.noise_reduction)),
            (
                "noise_reduction_level",
                opt(r.noise_reduction_level, PyVal::int),
            ),
            (
                "acoustic_echo_cancellation",
                b(r.acoustic_echo_cancellation),
            ),
            ("smart_volume", b(r.smart_volume)),
            ("mic_eq", b(r.mic_eq)),
        ])
    });
    dict(vec![
        ("firmware", s(&st.firmware)),
        ("output", s(&st.output)),
        ("decoder_mode", s(&st.decoder_mode)),
        ("dac_filter", s(&st.dac_filter)),
        ("direct_mode", b(st.direct_mode)),
        ("spdif_direct", b(st.spdif_direct)),
        (
            "line_out_headphone_virtualization",
            b(st.line_out_headphone_virtualization),
        ),
        ("rgb_enabled", b(st.rgb_enabled)),
        ("ring_enabled", b(st.ring_enabled)),
        (
            "mode_flags",
            opt(st.mode_flags, |(x, y)| {
                PyVal::List(vec![
                    PyVal::str(format!("0x{x:02x}")),
                    PyVal::str(format!("0x{y:02x}")),
                ])
            }),
        ),
        ("sbx", sbx),
        ("smart_volume_mode", s(&st.smart_volume_mode)),
        ("eq_enabled", b(st.eq_enabled)),
        ("eq_preamp_db", opt(st.eq_preamp_db, PyVal::Float)),
        ("eq_bands_db", bands),
        (
            "mic_boost_db",
            opt(st.mic_boost_db, |v| PyVal::int(v.into())),
        ),
        ("sbx_enabled", b(st.sbx_enabled)),
        ("scout_mode", b(st.scout_mode)),
        ("voice_clarity", b(st.voice_clarity)),
        ("recording", recording),
        ("mic_eq_preset", s(&st.mic_eq_preset)),
        ("mixer", mixer.map_or(PyVal::None, mixer_to_pyval)),
    ])
}

/// A row value: unknown, a switch, or text.
enum Cell {
    Unknown,
    Switch(bool),
    Text(String),
}

impl Cell {
    fn render(&self) -> String {
        match self {
            Cell::Unknown => "?".into(),
            Cell::Switch(true) => "On".into(),
            Cell::Switch(false) => "Off".into(),
            Cell::Text(t) => t.clone(),
        }
    }
}

fn text(v: &Option<String>) -> Cell {
    v.as_ref().map_or(Cell::Unknown, |t| Cell::Text(t.clone()))
}

fn switch(v: Option<bool>) -> Cell {
    v.map_or(Cell::Unknown, Cell::Switch)
}

/// `fmt` for a switch inside a longer value.
fn on_off(v: Option<bool>) -> String {
    switch(v).render()
}

fn level(v: Option<i64>) -> String {
    v.map_or("?".into(), |l| l.to_string())
}

/// `format_status(status, as_json)` with no mixer.
pub fn format_status(st: &DeviceStatus, as_json: bool) -> String {
    format_status_with_mixer(st, None, as_json)
}

/// `format_status(status, as_json)` for a status whose `mixer` is `mixer`.
pub fn format_status_with_mixer(st: &DeviceStatus, mixer: Option<&Mixer>, as_json: bool) -> String {
    if as_json {
        return dumps_indent2(&status_to_pyval_with_mixer(st, mixer));
    }
    let dac = st.dac_filter.as_ref().map(|code| {
        PlaybackFilter::from_name(code).map_or(code.clone(), |f| f.label().to_string())
    });
    let mut rows: Vec<(&str, Cell)> = vec![
        ("Firmware", text(&st.firmware)),
        ("Output", text(&st.output)),
        ("Decoder mode", text(&st.decoder_mode)),
        ("DAC filter", text(&dac)),
        ("Direct mode", switch(st.direct_mode)),
        ("SPDIF direct", switch(st.spdif_direct)),
        (
            "Line Out virt",
            switch(st.line_out_headphone_virtualization),
        ),
        ("RGB light", switch(st.rgb_enabled)),
        ("Volume ring", switch(st.ring_enabled)),
        ("SBX", switch(st.sbx_enabled)),
        ("Scout mode", switch(st.scout_mode)),
    ];
    if let Some(effects) = &st.sbx {
        for (effect, state) in SbxEffect::ALL.iter().zip(effects) {
            let mut value = format!("{}, {}", on_off(state.enabled), level(state.level));
            // Python checks the mode is a non-empty string
            if *effect == SbxEffect::SmartVolume
                && let Some(mode) = st.smart_volume_mode.as_ref().filter(|m| !m.is_empty())
            {
                value += &format!(", {mode}");
            }
            let label = match effect {
                SbxEffect::Surround => "Surround",
                SbxEffect::Crystalizer => "Crystalizer",
                SbxEffect::Bass => "Bass",
                SbxEffect::SmartVolume => "Smart volume",
                SbxEffect::DialogPlus => "Dialog+",
            };
            rows.push((label, Cell::Text(value)));
        }
    }
    rows.push(("Equalizer", switch(st.eq_enabled)));
    if let Some(bands) = &st.eq_bands_db {
        let db = |v: Option<f64>| v.map_or("?".into(), py_g);
        let bands: Vec<String> = EQ_BAND_FREQS
            .iter()
            .zip(bands)
            .map(|(f, v)| format!("{f}:{}", db(*v)))
            .collect();
        rows.push((
            "EQ (dB)",
            Cell::Text(format!(
                "preamp {} | {}",
                db(st.eq_preamp_db),
                bands.join(" ")
            )),
        ));
    }
    rows.push((
        "Mic boost",
        st.mic_boost_db
            .map_or(Cell::Unknown, |v| Cell::Text(format!("{v} dB"))),
    ));
    rows.push(("Voice clarity", switch(st.voice_clarity)));
    if let Some(rec) = &st.recording {
        rows.push((
            "Noise reduct.",
            Cell::Text(format!(
                "{}, {}",
                on_off(rec.noise_reduction),
                level(rec.noise_reduction_level)
            )),
        ));
        rows.push(("Echo cancel.", switch(rec.acoustic_echo_cancellation)));
        rows.push(("Mic smart vol", switch(rec.smart_volume)));
        let mut mic_eq = on_off(rec.mic_eq);
        if let Some(p) = st.mic_eq_preset.as_ref().filter(|p| !p.is_empty()) {
            mic_eq += &format!(", {p}");
        }
        rows.push(("Mic EQ", Cell::Text(mic_eq)));
    }
    let mut lines: Vec<String> = rows
        .iter()
        .map(|(label, cell)| format!("{label:<13} {}", cell.render()))
        .collect();
    if let Some(mixer) = mixer {
        lines.extend(
            format_mixer_rows(mixer)
                .iter()
                .map(|(label, text)| format!("{label:<13} {text}")),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod mixer_tests {
    use super::*;
    use g6_osaudio::MonitoringEntry;

    fn mixer() -> Mixer {
        let ep = |pct, db, default| EndpointEntry {
            level_percent: pct,
            level_db: db,
            muted: false,
            default,
        };
        Mixer {
            playback: vec![("Speakers".into(), ep(96, -0.61, true))],
            recording: vec![("Line In".into(), ep(68, 0.0, false))],
            monitoring: vec![(
                "External Mic".into(),
                MonitoringEntry {
                    level_db: Some(8.08),
                    muted: Some(true),
                },
            )],
            speaker_layout: Some("Stereo".into()),
        }
    }

    #[test]
    fn json_mixer_matches_python_layout() {
        let json = format_status_with_mixer(&DeviceStatus::default(), Some(&mixer()), true);
        let tail = json.split("\"mixer\": ").nth(1).unwrap();
        let expected = r#"{
    "playback": {
      "Speakers": {
        "level_percent": 96,
        "level_db": -0.61,
        "muted": false,
        "default": true
      }
    },
    "recording": {
      "Line In": {
        "level_percent": 68,
        "level_db": 0.0,
        "muted": false,
        "default": false
      }
    },
    "monitoring": {
      "External Mic": {
        "level_db": 8.08,
        "muted": true
      }
    },
    "speaker_layout": "Stereo"
  }
}"#;
        assert_eq!(tail, expected);
    }

    #[test]
    fn text_mixer_rows_follow_the_status_rows() {
        let text = format_status_with_mixer(&DeviceStatus::default(), Some(&mixer()), false);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[lines.len() - 4..],
            [
                "Playback vol  Speakers: 96% -0.6 dB",
                "Recording     Line In: 68% +0.0 dB",
                "Monitoring    External Mic: +8.1 dB (muted)",
                "Speaker setup Stereo (Windows)",
            ]
        );
    }

    #[test]
    fn no_mixer_means_null_and_no_rows() {
        let st = DeviceStatus::default();
        assert!(format_status(&st, true).contains("\"mixer\": null"));
        assert!(!format_status(&st, false).contains("Playback vol"));
    }
}
