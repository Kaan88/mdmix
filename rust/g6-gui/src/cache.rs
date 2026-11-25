//! The last status the G6 gave, kept next to the app's settings so the window's first frame can show it.
//!
//! It is not a device setting and it is never trusted as current: the model marks it [`Conn::Cached`](crate::model::Conn::Cached)
//! until the first fresh read answers. The file is written atomically by the toolkit's settings layer, the same one the
//! preferences use, and the (de)serialisation is written out here so `g6-proto` stays free of I/O and serde.

use std::path::{Path, PathBuf};

use g6_proto::status::{DeviceStatus, RecordingStatus, SbxState};
use serde_json::{Map, Value, json};

use crate::model::{MixerView, Strip};
use crate::prefs::Prefs;

const FILE: &str = "status-cache.json";
/// Bumped when the shape changes: an older or newer file is ignored rather than misread.
const VERSION: u64 = 1;

/// The last status read, with the mixer values Windows gave at the same time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cached {
    pub status: DeviceStatus,
    pub mixer: Option<MixerView>,
}

/// The file next to the preferences file.
pub fn path() -> Option<PathBuf> {
    Prefs::path().map(|p| p.with_file_name(FILE))
}

/// The saved status, if there is one and it can be read. Never an error: a missing or damaged file is simply no
/// cache, and the window waits for the read as it did before.
pub fn load() -> Option<Cached> {
    load_at(&path()?)
}

pub fn load_at(path: &Path) -> Option<Cached> {
    let text = std::fs::read_to_string(path).ok()?;
    from_json(&text)
}

/// Writes the status atomically, creating the directory. A failure only means the next start has no cache.
pub fn save(c: &Cached) {
    if let Some(path) = path() {
        save_at(&path, c);
    }
}

pub fn save_at(path: &Path, c: &Cached) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = mdshared_app::settings::write_atomic(path, &to_json(c)) {
        eprintln!("g6-gui: status cache not saved: {e}");
    }
}

pub fn to_json(c: &Cached) -> String {
    json!({
        "version": VERSION,
        "status": status_json(&c.status),
        "mixer": c.mixer.as_ref().map(mixer_json),
    })
    .to_string()
}

/// `None` for anything that isn't this cache: another version, a missing status, damaged JSON.
pub fn from_json(text: &str) -> Option<Cached> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("version")?.as_u64()? != VERSION {
        return None;
    }
    let status = status_of(v.get("status")?.as_object()?);
    // An empty status is not a cache: it would draw blank controls under a "Reading…" badge as if they were known.
    if status.firmware.is_none() && status.output.is_none() && status.sbx.is_none() {
        return None;
    }
    let mixer = v.get("mixer").and_then(Value::as_object).map(mixer_of);
    Some(Cached { status, mixer })
}

// ---- DeviceStatus ------------------------------------------------------------------------------

fn status_json(s: &DeviceStatus) -> Value {
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
        "mode_flags": s.mode_flags.map(|(a, b)| [a, b]),
        "sbx": s.sbx.map(|all| all.iter().map(sbx_json).collect::<Vec<_>>()),
        "smart_volume_mode": s.smart_volume_mode,
        "eq_enabled": s.eq_enabled,
        "eq_preamp_db": s.eq_preamp_db,
        "eq_bands_db": s.eq_bands_db,
        "mic_boost_db": s.mic_boost_db,
        "sbx_enabled": s.sbx_enabled,
        "scout_mode": s.scout_mode,
        "voice_clarity": s.voice_clarity,
        "recording": s.recording.map(|r| recording_json(&r)),
        "mic_eq_preset": s.mic_eq_preset,
    })
}

fn status_of(v: &Map<String, Value>) -> DeviceStatus {
    let mut s = DeviceStatus {
        firmware: string(v, "firmware"),
        output: string(v, "output"),
        decoder_mode: string(v, "decoder_mode"),
        dac_filter: string(v, "dac_filter"),
        direct_mode: boolean(v, "direct_mode"),
        spdif_direct: boolean(v, "spdif_direct"),
        line_out_headphone_virtualization: boolean(v, "line_out_headphone_virtualization"),
        rgb_enabled: boolean(v, "rgb_enabled"),
        ring_enabled: boolean(v, "ring_enabled"),
        mode_flags: v
            .get("mode_flags")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 2)
            .and_then(|a| Some((u8_of(&a[0])?, u8_of(&a[1])?))),
        smart_volume_mode: string(v, "smart_volume_mode"),
        eq_enabled: boolean(v, "eq_enabled"),
        eq_preamp_db: v.get("eq_preamp_db").and_then(Value::as_f64),
        mic_boost_db: v.get("mic_boost_db").and_then(u8_of),
        sbx_enabled: boolean(v, "sbx_enabled"),
        scout_mode: boolean(v, "scout_mode"),
        voice_clarity: boolean(v, "voice_clarity"),
        mic_eq_preset: string(v, "mic_eq_preset"),
        ..DeviceStatus::default()
    };
    s.sbx = v
        .get("sbx")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 5)
        .map(|a| std::array::from_fn(|i| sbx_of(&a[i])));
    s.eq_bands_db = v
        .get("eq_bands_db")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 10)
        .map(|a| std::array::from_fn(|i| a[i].as_f64()));
    s.recording = v
        .get("recording")
        .and_then(Value::as_object)
        .map(recording_of);
    s
}

fn sbx_json(e: &SbxState) -> Value {
    json!({ "enabled": e.enabled, "level": e.level })
}

fn sbx_of(v: &Value) -> SbxState {
    let o = v.as_object();
    SbxState {
        enabled: o.and_then(|o| boolean(o, "enabled")),
        level: o.and_then(|o| o.get("level")).and_then(Value::as_i64),
    }
}

fn recording_json(r: &RecordingStatus) -> Value {
    json!({
        "noise_reduction": r.noise_reduction,
        "noise_reduction_level": r.noise_reduction_level,
        "acoustic_echo_cancellation": r.acoustic_echo_cancellation,
        "smart_volume": r.smart_volume,
        "mic_eq": r.mic_eq,
    })
}

fn recording_of(v: &Map<String, Value>) -> RecordingStatus {
    RecordingStatus {
        noise_reduction: boolean(v, "noise_reduction"),
        noise_reduction_level: v.get("noise_reduction_level").and_then(Value::as_i64),
        acoustic_echo_cancellation: boolean(v, "acoustic_echo_cancellation"),
        smart_volume: boolean(v, "smart_volume"),
        mic_eq: boolean(v, "mic_eq"),
    }
}

// ---- the mixer ---------------------------------------------------------------------------------

fn strip_json(s: &Strip) -> Value {
    json!({ "volume": s.volume, "db": s.db, "muted": s.muted })
}

fn strip_of(v: &Value) -> Strip {
    let o = v.as_object();
    Strip {
        volume: o.and_then(|o| o.get("volume")).and_then(u8_of),
        db: o.and_then(|o| o.get("db")).and_then(Value::as_f64),
        muted: o.and_then(|o| boolean(o, "muted")),
    }
}

fn mixer_json(m: &MixerView) -> Value {
    json!({
        "master": strip_json(&m.master),
        "monitoring": m.monitoring.iter().map(strip_json).collect::<Vec<_>>(),
        "recording": m.recording.iter().map(strip_json).collect::<Vec<_>>(),
        "mix_muted": m.mix_muted,
        "speaker_layout": m.speaker_layout,
    })
}

fn mixer_of(v: &Map<String, Value>) -> MixerView {
    let strips = |key: &str, n: usize| -> Vec<Strip> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().take(n).map(strip_of).collect())
            .unwrap_or_default()
    };
    let mut monitoring = [Strip::default(); 4];
    for (d, s) in monitoring.iter_mut().zip(strips("monitoring", 4)) {
        *d = s;
    }
    let mut recording = [Strip::default(); 5];
    for (d, s) in recording.iter_mut().zip(strips("recording", 5)) {
        *d = s;
    }
    MixerView {
        master: v.get("master").map(strip_of).unwrap_or_default(),
        monitoring,
        recording,
        mix_muted: boolean(v, "mix_muted"),
        speaker_layout: v
            .get("speaker_layout")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

// ---- small readers -----------------------------------------------------------------------------

fn string(v: &Map<String, Value>, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn boolean(v: &Map<String, Value>, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

fn u8_of(v: &Value) -> Option<u8> {
    u8::try_from(v.as_u64()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{busy_mixer, busy_status, live_status};

    #[test]
    fn a_full_status_round_trips_with_and_without_the_mixer() {
        for (status, mixer) in [(live_status(), None), (busy_status(), Some(busy_mixer()))] {
            let c = Cached { status, mixer };
            let back = from_json(&to_json(&c)).expect("the cache parses");
            assert_eq!(back, c);
            // The identity is the whole struct: a field missed by the writer would show up as a difference.
            assert_eq!(back.status, c.status);
            assert_eq!(back.mixer, c.mixer);
        }
    }

    #[test]
    fn damaged_or_foreign_files_are_no_cache() {
        for text in [
            "",
            "not json",
            "{}",
            r#"{"version":2,"status":{}}"#,
            r#"{"version":1}"#,
            r#"{"version":1,"status":{}}"#,
            r#"{"status":{}}"#,
        ] {
            assert_eq!(from_json(text), None, "{text:?}");
        }
        assert_eq!(load_at(Path::new("no-such-status-cache.json")), None);
    }

    #[test]
    fn what_was_saved_is_what_is_loaded() {
        let dir = std::env::temp_dir().join(format!("g6-gui-cache-{}", std::process::id()));
        let path = dir.join(FILE);
        let c = Cached {
            status: busy_status(),
            mixer: Some(busy_mixer()),
        };
        save_at(&path, &c);
        assert_eq!(load_at(&path).as_ref(), Some(&c));
        // A damaged file on disk reads as no cache, not a panic.
        std::fs::write(&path, b"{oops").unwrap();
        assert_eq!(load_at(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
