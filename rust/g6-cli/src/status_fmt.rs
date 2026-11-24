//! `--status` output, as text or JSON (Python: `format_status`, `DeviceStatus.to_dict`).
//!
//! The mixer part (volumes from the OS audio stack) isn't ported yet, so it is always `null` / left out.

use crate::pyval::{PyVal, dumps_indent2};
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

/// `DeviceStatus.to_dict()`.
pub fn status_to_pyval(st: &DeviceStatus) -> PyVal {
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
        ("mixer", PyVal::None),
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

/// `format_status(status, as_json)`.
pub fn format_status(st: &DeviceStatus, as_json: bool) -> String {
    if as_json {
        return dumps_indent2(&status_to_pyval(st));
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
    rows.iter()
        .map(|(label, cell)| format!("{label:<13} {}", cell.render()))
        .collect::<Vec<_>>()
        .join("\n")
}
