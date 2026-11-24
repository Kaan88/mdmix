//! The protocol crate against the vectors the Python code exports (`tests/vectors/`, see doc/rust-port.md).

use std::path::PathBuf;

use g6_proto::effects::{Effect, LevelFollower, MUSIC_FLOOR_DB, effect_frame, level_to_brightness};
use g6_proto::eq_presets;
use g6_proto::equalizer::EQ_BAND_FREQS;
use g6_proto::packet::{pad_report, unhex};
use g6_proto::status::{DeviceStatus, collect_status};
use g6_proto::types::SbxEffect;
use serde_json::{Value, json};

fn vectors(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/vectors")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("valid JSON")
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

#[test]
fn effect_frames() {
    let v = vectors("effects.json");
    let frames = v["frames"].as_array().unwrap();
    assert!(!frames.is_empty());
    for f in frames {
        let effect = Effect::from_name(f["effect"].as_str().unwrap()).unwrap();
        let rgb = f["rgb"].as_array().map(|c| {
            let c: Vec<u8> = c.iter().map(|x| x.as_u64().unwrap() as u8).collect();
            (c[0], c[1], c[2])
        });
        let got = effect_frame(
            effect,
            f["t"].as_f64().unwrap(),
            f["period"].as_f64().unwrap(),
            rgb,
            f["level"].as_f64(),
        )
        .unwrap();
        let want: Vec<u64> = f["frame"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .collect();
        assert_eq!(
            [got.0, got.1, got.2, got.3].map(u64::from).to_vec(),
            want,
            "{f}"
        );
    }
}

#[test]
fn effect_levels() {
    let v = vectors("effects.json");
    for c in v["level_to_brightness"].as_array().unwrap() {
        let got = level_to_brightness(c["peak"].as_f64().unwrap(), MUSIC_FLOOR_DB);
        assert!(close(got, c["out"].as_f64().unwrap()), "{c}: {got}");
    }
    let lf = &v["level_follower"];
    let mut follower = LevelFollower::default();
    assert!(close(follower.decay_s, lf["decay_s"].as_f64().unwrap()));
    for step in lf["steps"].as_array().unwrap() {
        let got = follower.update(
            step["level"].as_f64().unwrap(),
            step["dt"].as_f64().unwrap(),
        );
        assert!(close(got, step["out"].as_f64().unwrap()), "{step}: {got}");
    }
}

#[test]
fn effect_period_must_be_positive() {
    let e = effect_frame(Effect::Cycle, 0.0, 0.0, None, None).unwrap_err();
    assert_eq!(e.to_string(), "period must be > 0, got 0.0");
    assert!(effect_frame(Effect::Cycle, 0.0, f64::NAN, None, None).is_err());
}

#[test]
fn eq_presets_match_json() {
    let v = vectors("eq_presets.json");
    let presets = v["presets"].as_object().unwrap();
    let ours = eq_presets::all();
    // serde_json sorts object keys, so check SBC's order (which the CLI help lists) on the text itself
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/vectors/eq_presets.json"),
    )
    .unwrap();
    let positions: Vec<usize> = ours
        .iter()
        .map(|(id, _, _)| {
            text.find(&format!("\"{id}\": {{"))
                .unwrap_or_else(|| panic!("{id} not in the JSON"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "preset order differs from the JSON"
    );
    assert_eq!(ours.len(), presets.len());
    for (id, speakers, headphones) in ours {
        let p = &presets[*id];
        let as_vec = |x: &Value| -> Vec<f64> {
            x.as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_f64().unwrap())
                .collect()
        };
        assert_eq!(speakers.to_vec(), as_vec(&p["speakers"]), "{id} speakers");
        assert_eq!(
            headphones.to_vec(),
            as_vec(&p["headphones"]),
            "{id} headphones"
        );
    }
}

#[test]
fn eq_preset_lookup() {
    assert_eq!(eq_presets::find_eq_preset("hip hop").unwrap(), "HipHop");
    assert_eq!(eq_presets::find_eq_preset("ROCK").unwrap(), "Rock");
    let e = eq_presets::find_eq_preset("nope").unwrap_err().to_string();
    assert!(
        e.starts_with("unknown EQ preset 'nope', expected one of Acoustic, Classical, "),
        "{e}"
    );
}

/// The JSON Python's `DeviceStatus.to_dict()` gives, without the OS mixer.
fn status_json(s: &DeviceStatus) -> Value {
    let sbx = s.sbx.map(|states| {
        let mut m = serde_json::Map::new();
        for (e, st) in SbxEffect::ALL.iter().zip(states) {
            m.insert(
                e.key().into(),
                json!({"enabled": st.enabled, "level": st.level}),
            );
        }
        Value::Object(m)
    });
    let bands = s.eq_bands_db.map(|b| {
        let mut m = serde_json::Map::new();
        for (f, v) in EQ_BAND_FREQS.iter().zip(b) {
            m.insert((*f).into(), json!(v));
        }
        Value::Object(m)
    });
    json!({
        "firmware": s.firmware,
        "output": s.output,
        "decoder_mode": s.decoder_mode,
        "dac_filter": s.dac_filter,
        "direct_mode": s.direct_mode,
        "spdif_direct": s.spdif_direct,
        "line_out_headphone_virtualization": s.line_out_headphone_virtualization,
        "rgb_enabled": s.rgb_enabled,
        "ring_enabled": s.ring_enabled,
        "mode_flags": s.mode_flags.map(|(a, b)| vec![format!("0x{a:02x}"), format!("0x{b:02x}")]),
        "sbx": sbx,
        "smart_volume_mode": s.smart_volume_mode,
        "eq_enabled": s.eq_enabled,
        "eq_preamp_db": s.eq_preamp_db,
        "eq_bands_db": bands,
        "mic_boost_db": s.mic_boost_db,
        "sbx_enabled": s.sbx_enabled,
        "scout_mode": s.scout_mode,
        "voice_clarity": s.voice_clarity,
        "recording": s.recording.map(|r| json!({
            "noise_reduction": r.noise_reduction,
            "noise_reduction_level": r.noise_reduction_level,
            "acoustic_echo_cancellation": r.acoustic_echo_cancellation,
            "smart_volume": r.smart_volume,
            "mic_eq": r.mic_eq,
        })),
        "mic_eq_preset": s.mic_eq_preset,
    })
}

#[test]
fn status_live_transcript() {
    let v = vectors("status_live.json");
    let exchanges = v["exchanges"].as_array().unwrap();
    let mut i = 0;
    // feed the answers in order and check every query is the one Python sent at that point
    let status = collect_status(|q| {
        let ex = exchanges
            .get(i)
            .unwrap_or_else(|| panic!("query {i} beyond the transcript"));
        i += 1;
        assert_eq!(
            g6_proto::packet::hex(q),
            ex["query"].as_str().unwrap(),
            "query {}",
            i - 1
        );
        ex["response"]
            .as_str()
            .map(|r| pad_report(&unhex(r).unwrap()).unwrap())
    });
    assert_eq!(
        i,
        exchanges.len(),
        "Rust sent fewer queries than the transcript holds"
    );
    assert_eq!(status_json(&status), v["parsed"]);
}
