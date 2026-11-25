//! The G6's volumes, mutes, speaker setup and output peak level, read through the operating system's audio stack
//! (Python: `g6_os_audio.py`). Nothing here sets a value.
//!
//! On Windows the reads go through Core Audio (`IMMDeviceEnumerator`, `IAudioEndpointVolume`, the endpoints'
//! `IDeviceTopology`, `PKEY_AudioEndpoint_PhysicalSpeakers`, `IAudioMeterInformation`). Every other platform gets
//! `None` ("not available"); the Linux `amixer` reader is a later step.
//!
//! The mapping and formatting code is pure and tested; only `win.rs` talks to COM, and it is the only place in the
//! workspace with `unsafe`. Each public read initialises COM (MTA) on the calling thread and uses its interface
//! pointers on that thread only. [`OutputMeter`] holds COM pointers, so it is neither `Send` nor `Sync`: open it on
//! the thread that polls it.

#![deny(unsafe_code)]

#[cfg(windows)]
#[allow(unsafe_code)]
mod win;

/// Python's `G6_NAME_MARKERS`.
pub const G6_NAME_MARKERS: [&str; 2] = ["BlasterX G6", "Blaster X G6"];

/// `is_g6_endpoint`: a case-insensitive substring match.
pub fn is_g6_endpoint(name: &str) -> bool {
    let name = name.to_lowercase();
    G6_NAME_MARKERS
        .iter()
        .any(|m| name.contains(&m.to_lowercase()))
}

/// `short_name`: 'External Mic (Sound BlasterX G6)' -> 'External Mic'.
pub fn short_name(endpoint_name: &str) -> &str {
    endpoint_name
        .split_once(" (")
        .map_or(endpoint_name, |(head, _)| head)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Playback,
    Recording,
}

/// One G6 endpoint (`EndpointVolume`).
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointVolume {
    /// Windows endpoint name, e.g. 'Speakers (Sound BlasterX G6)'.
    pub name: String,
    pub flow: Flow,
    pub level_db: f64,
    /// Windows' scalar volume, as the Windows mixer and SBC show it.
    pub level_percent: i64,
    pub muted: bool,
    pub is_default: bool,
}

/// A volume and/or mute control inside the device topology (`TopologyControl`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopologyControl {
    pub name: String,
    /// Per channel.
    pub level_db: Vec<f64>,
    pub muted: Option<bool>,
}

/// A playback or recording entry of the mixer (`level_percent`, `level_db`, `muted`, `default`).
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointEntry {
    pub level_percent: i64,
    pub level_db: f64,
    pub muted: bool,
    pub default: bool,
}

/// A monitoring entry (`level_db`, `muted`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MonitoringEntry {
    pub level_db: Option<f64>,
    pub muted: Option<bool>,
}

/// The `mixer` dict of the status. Each list keeps insertion order, like a Python dict.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mixer {
    pub playback: Vec<(String, EndpointEntry)>,
    pub recording: Vec<(String, EndpointEntry)>,
    pub monitoring: Vec<(String, MonitoringEntry)>,
    pub speaker_layout: Option<String>,
}

/// Driver part names on the playback path -> SBC Mixer "Monitoring" labels.
pub const MONITORING_LABELS: [(&str, &str); 3] = [
    ("Microphone", "External Mic"),
    ("Auxiliary", "Line In"),
    ("SPDIF-In", "SPDIF In"),
];
/// The output itself; already reported as the playback endpoint.
pub const OUTPUT_PART: &str = "Speaker";

/// `d[key] = value` on an insertion-ordered dict: an existing key keeps its place.
fn set<V>(items: &mut Vec<(String, V)>, key: &str, value: V) {
    match items.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value,
        None => items.push((key.to_string(), value)),
    }
}

/// `group_monitoring`: merge '<X> Volume' and '<X> Mute' parts. Unknown names keep the driver's name.
pub fn group_monitoring(controls: &[TopologyControl]) -> Vec<(String, MonitoringEntry)> {
    let mut out: Vec<(String, MonitoringEntry)> = Vec::new();
    for ctl in controls {
        // Python strips ' Volume' and then ' Mute' (each at most once, in that order)
        let mut base = ctl.name.as_str();
        for suffix in [" Volume", " Mute"] {
            if let Some(b) = base.strip_suffix(suffix) {
                base = b;
            }
        }
        if base == OUTPUT_PART {
            continue;
        }
        let label = MONITORING_LABELS
            .iter()
            .find(|(k, _)| *k == base)
            .map_or(base, |(_, v)| v);
        let idx = match out.iter().position(|(k, _)| k == label) {
            Some(i) => i,
            None => {
                out.push((label.to_string(), MonitoringEntry::default()));
                out.len() - 1
            }
        };
        let entry = &mut out[idx].1;
        // Python: `if ctl.level_db:` tests the list, so a lone 0.0 still counts
        if let Some(first) = ctl.level_db.first() {
            entry.level_db = Some(*first);
        }
        if ctl.muted.is_some() && ctl.name.ends_with(" Mute") {
            entry.muted = ctl.muted;
        }
    }
    out
}

type Entries = Vec<(String, EndpointEntry)>;

/// `endpoints_by_flow`: (playback, recording) keyed by short name; a repeated name replaces the earlier entry.
pub fn endpoints_by_flow(endpoints: &[EndpointVolume]) -> (Entries, Entries) {
    let (mut playback, mut recording) = (Vec::new(), Vec::new());
    for ep in endpoints {
        let entry = EndpointEntry {
            level_percent: ep.level_percent,
            level_db: ep.level_db,
            muted: ep.muted,
            default: ep.is_default,
        };
        let list = match ep.flow {
            Flow::Playback => &mut playback,
            Flow::Recording => &mut recording,
        };
        set(list, short_name(&ep.name), entry);
    }
    (playback, recording)
}

/// `SPEAKER_LAYOUTS`: KSAUDIO_SPEAKER_* channel masks -> the names Windows' "Configure speakers" dialog uses.
/// Unknown masks come back as hex so they stay visible.
pub fn speaker_layout(mask: Option<u32>) -> Option<String> {
    let mask = mask?;
    Some(match mask {
        0x4 => "Mono".to_string(),
        0x3 => "Stereo".to_string(),
        0x33 => "Quadraphonic".to_string(),
        0x107 => "Surround".to_string(),
        0x3F | 0x60F => "5.1".to_string(),
        0xFF | 0x63F => "7.1".to_string(),
        _ => format!("0x{mask:x}"),
    })
}

/// What `read_mixer` does with the Core Audio reads.
pub fn build_mixer(
    endpoints: &[EndpointVolume],
    monitoring: &[TopologyControl],
    speaker_mask: Option<u32>,
) -> Mixer {
    let (playback, recording) = endpoints_by_flow(endpoints);
    Mixer {
        playback,
        recording,
        monitoring: group_monitoring(monitoring),
        speaker_layout: speaker_layout(speaker_mask),
    }
}

/// Python's `round(x, 2)`: the exact decimal value rounded half to even, which is what `{:.2}` does.
pub fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// Python's `round(scalar * 100)`: half to even. `None` where Python's `round` raises (NaN, infinity).
pub fn round_percent(scalar: f64) -> Option<i64> {
    let v = scalar * 100.0;
    v.is_finite().then(|| v.round_ties_even() as i64)
}

fn db_text(v: f64) -> String {
    format!("{v:+.1} dB")
}

fn endpoint_text(e: &EndpointEntry) -> String {
    let mut s = format!("{}% {}", e.level_percent, db_text(e.level_db));
    if e.muted {
        s += " (muted)";
    }
    s
}

fn monitoring_text(e: &MonitoringEntry) -> String {
    match (e.level_db, e.muted) {
        (Some(v), muted) => db_text(v) + if muted == Some(true) { " (muted)" } else { "" },
        (None, Some(true)) => "muted".into(),
        (None, Some(false)) => "not muted".into(),
        (None, None) => "?".into(),
    }
}

/// `format_mixer_rows`: (label, text) rows for the status text.
pub fn format_mixer_rows(mixer: &Mixer) -> Vec<(String, String)> {
    let sections: [(&str, Vec<(&str, String)>); 3] = [
        (
            "Playback vol",
            mixer
                .playback
                .iter()
                .map(|(n, e)| (n.as_str(), endpoint_text(e)))
                .collect(),
        ),
        (
            "Recording",
            mixer
                .recording
                .iter()
                .map(|(n, e)| (n.as_str(), endpoint_text(e)))
                .collect(),
        ),
        (
            "Monitoring",
            mixer
                .monitoring
                .iter()
                .map(|(n, e)| (n.as_str(), monitoring_text(e)))
                .collect(),
        ),
    ];
    let mut rows = Vec::new();
    for (label, items) in sections {
        for (i, (name, text)) in items.into_iter().enumerate() {
            let label = if i == 0 { label } else { "" };
            rows.push((label.to_string(), format!("{name}: {text}")));
        }
    }
    if let Some(layout) = mixer.speaker_layout.as_deref().filter(|l| !l.is_empty()) {
        rows.push(("Speaker setup".into(), format!("{layout} (Windows)")));
    }
    rows
}

/// Playback, recording and monitoring levels of the G6, or `None` when this platform has no reader or Core Audio
/// failed anywhere (Python's `read_mixer_safely`: the mixer is optional extra information).
pub fn read_mixer() -> Option<Mixer> {
    #[cfg(windows)]
    {
        win::read_mixer()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The current peak level (0..1, linear) of what the G6 is playing: Core Audio's `IAudioMeterInformation` on the
/// G6 playback endpoint. Not `Send`: keep it on the thread that opened it.
///
/// Opening one joins that thread to the COM multithreaded apartment for as long as the meter lives. A thread that
/// later needs a single-threaded apartment (a GUI thread initialising OLE or drag and drop, for instance) would then
/// fail with `RPC_E_CHANGED_MODE`. So open the meter on its own polling thread, or only after the UI has initialised
/// COM itself (then this crate just reuses that apartment).
#[cfg(windows)]
pub struct OutputMeter(win::Meter);

/// Not available on this platform; [`open_output_meter`] never returns one.
#[cfg(not(windows))]
pub struct OutputMeter(std::convert::Infallible);

impl OutputMeter {
    /// Peak over all channels, 0.0..=1.0. A failed read (device gone) gives 0.0.
    pub fn peak(&self) -> f64 {
        #[cfg(windows)]
        {
            self.0.peak()
        }
        #[cfg(not(windows))]
        {
            match self.0 {}
        }
    }

    /// The peak of each metering channel, 0.0..=1.0, left first: the endpoint's first two channels, or its one mono
    /// reading for both. A failed read (device gone) gives `[0.0, 0.0]`.
    pub fn peaks(&self) -> [f64; 2] {
        #[cfg(windows)]
        {
            self.0.peaks()
        }
        #[cfg(not(windows))]
        {
            match self.0 {}
        }
    }
}

/// `open_output_meter`: `None` when there is no G6 playback endpoint or this platform has no meter. Call it on the
/// thread that will poll the meter, not on a GUI thread that hasn't initialised COM yet (see [`OutputMeter`]).
pub fn open_output_meter() -> Option<OutputMeter> {
    #[cfg(windows)]
    {
        win::open_meter().map(OutputMeter)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(name: &str, db: &[f64], muted: Option<bool>) -> TopologyControl {
        TopologyControl {
            name: name.into(),
            level_db: db.to_vec(),
            muted,
        }
    }

    #[test]
    fn g6_names() {
        assert!(is_g6_endpoint("Speakers (Sound BlasterX G6)"));
        assert!(is_g6_endpoint("line in (SOUND BLASTER X G6)"));
        assert!(!is_g6_endpoint("Speakers (Realtek)"));
        assert_eq!(
            short_name("External Mic (Sound BlasterX G6)"),
            "External Mic"
        );
        assert_eq!(short_name("Speakers"), "Speakers");
    }

    #[test]
    fn monitoring_merges_volume_and_mute() {
        let parts = [
            ctl("Speaker Volume", &[-3.0], None),
            ctl("Microphone Volume", &[-12.5, -12.5], None),
            ctl("Microphone Mute", &[], Some(true)),
            ctl("Auxiliary Mute", &[], Some(false)),
            ctl("Auxiliary Volume", &[0.0], None),
            ctl("Weird", &[], None),
        ];
        let got = group_monitoring(&parts);
        let names: Vec<_> = got.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["External Mic", "Line In", "Weird"]);
        assert_eq!(got[0].1.level_db, Some(-12.5));
        assert_eq!(got[0].1.muted, Some(true));
        assert_eq!(got[1].1.level_db, Some(0.0));
        assert_eq!(got[1].1.muted, Some(false));
        assert_eq!(got[2].1, MonitoringEntry::default());
    }

    #[test]
    fn mute_flag_needs_the_mute_name() {
        let got = group_monitoring(&[ctl("Line", &[], Some(true))]);
        assert_eq!(got[0].1.muted, None);
    }

    #[test]
    fn endpoints_keyed_by_short_name() {
        let ep = |name: &str, flow, pct| EndpointVolume {
            name: name.into(),
            flow,
            level_db: -1.5,
            level_percent: pct,
            muted: false,
            is_default: true,
        };
        let (p, r) = endpoints_by_flow(&[
            ep("Speakers (Sound BlasterX G6)", Flow::Playback, 40),
            ep("External Mic (Sound BlasterX G6)", Flow::Recording, 70),
            ep("Speakers (2- Sound BlasterX G6)", Flow::Playback, 41),
        ]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].1.level_percent, 41);
        assert_eq!(r[0].0, "External Mic");
    }

    #[test]
    fn layouts() {
        assert_eq!(speaker_layout(None), None);
        assert_eq!(speaker_layout(Some(3)).as_deref(), Some("Stereo"));
        assert_eq!(speaker_layout(Some(0x60F)).as_deref(), Some("5.1"));
        assert_eq!(speaker_layout(Some(0x63F)).as_deref(), Some("7.1"));
        assert_eq!(speaker_layout(Some(0x1234)).as_deref(), Some("0x1234"));
    }

    #[test]
    fn rounding_follows_python() {
        assert_eq!(round2(-12.345_678), -12.35);
        assert_eq!(round2(0.125), 0.12); // exact tie: half to even
        // f32 values widened to f64, as Core Audio hands them over (0.425f32 is 42.5000011...); Python: 43
        assert_eq!(round_percent(f64::from(0.425f32)), Some(43));
        assert_eq!(round_percent(0.425), Some(42)); // an exact f64 tie goes to even
        assert_eq!(round_percent(1.0), Some(100));
        assert_eq!(round_percent(f64::NAN), None);
        assert_eq!(round_percent(f64::INFINITY), None);
    }

    fn sample() -> Mixer {
        let ep = |pct, db, muted, default| EndpointEntry {
            level_percent: pct,
            level_db: db,
            muted,
            default,
        };
        Mixer {
            playback: vec![("Speakers".into(), ep(37, -26.5, false, true))],
            recording: vec![
                ("External Mic".into(), ep(100, 0.0, true, false)),
                ("Line In".into(), ep(50, -0.04, false, true)),
            ],
            monitoring: vec![
                (
                    "Line In".into(),
                    MonitoringEntry {
                        level_db: Some(-9.0),
                        muted: Some(true),
                    },
                ),
                (
                    "SPDIF In".into(),
                    MonitoringEntry {
                        level_db: None,
                        muted: Some(false),
                    },
                ),
                ("Odd".into(), MonitoringEntry::default()),
            ],
            speaker_layout: Some("Stereo".into()),
        }
    }

    #[test]
    fn rows_match_python_format() {
        let rows: Vec<_> = format_mixer_rows(&sample())
            .iter()
            .map(|(a, b)| format!("{a}|{b}"))
            .collect();
        assert_eq!(
            rows,
            [
                "Playback vol|Speakers: 37% -26.5 dB",
                "Recording|External Mic: 100% +0.0 dB (muted)",
                "|Line In: 50% -0.0 dB",
                "Monitoring|Line In: -9.0 dB (muted)",
                "|SPDIF In: not muted",
                "|Odd: ?",
                "Speaker setup|Stereo (Windows)",
            ]
        );
    }

    #[test]
    fn empty_mixer_has_no_rows() {
        assert!(format_mixer_rows(&Mixer::default()).is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn not_available_off_windows() {
        assert!(read_mixer().is_none());
        assert!(open_output_meter().is_none());
    }
}
