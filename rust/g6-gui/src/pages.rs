//! The cards of each page, from the device state. Every device control is drawn as it is on the G6 and inert.
//!
//! A value the G6 did not give shows as "—", a choice with no answer has nothing chosen, and a slider with no
//! value has its groove alone.

use g6_proto::equalizer::{EQ_BAND_FREQS, EQ_DB_MAX, EQ_DB_MIN};
use g6_proto::status::DeviceStatus;
use g6_proto::types::SbxEffect;
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb, mix};
use mdshared_ui::glyph;

use crate::facts::{self, DASH};
use crate::icons;
use crate::kit::{
    self, Card, Ctl, Draw, Env, Hit, Item, Part, Tone, dimmed, live_seg, row, seg, value,
};
use crate::model::{MixerView, Model, Page, Strip};
use crate::prefs::{Prefs, Theme};
use mdshared_app::backdrop::Backdrop;
use mdshared_ui::corners::Corners;

fn frac(level: Option<i64>, max: f32) -> Option<f32> {
    level.map(|v| (v as f32 / max).clamp(0.0, 1.0))
}

fn toggle_row(label: &str, on: Option<bool>) -> kit::RowSpec {
    row(label, Ctl::Toggle(on))
}

fn percent_slider(level: Option<i64>) -> Ctl {
    Ctl::Slider {
        frac: frac(level, 100.0),
        reading: facts::percent(level),
        from_middle: false,
    }
}

/// A banner above a page: why part of it is out of use.
pub struct Notice {
    pub warn: bool,
    pub text: String,
}

pub fn notices(page: Page, m: &Model) -> Vec<Notice> {
    let mut out = Vec::new();
    if page == Page::Sbx {
        if m.direct() {
            out.push(Notice {
                warn: true,
                text: "Direct mode is on: SBX and Scout Mode are bypassed.".into(),
            });
        } else if m.status.sbx_enabled == Some(false) {
            out.push(Notice {
                warn: false,
                text: if m.status.scout_mode == Some(true) {
                    "Scout Mode is on, so SBX is off and the effects below are not applied.".into()
                } else {
                    "SBX is off: the effects below are not applied.".into()
                },
            });
        }
    }
    out
}

/// The badges at the right of a page's title.
pub fn tags(page: Page, m: &Model) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    let s = &m.status;
    match page {
        Page::Sbx => {
            if s.direct_mode == Some(true) {
                out.push(("Direct mode".into(), Tone::Lit));
            }
            if s.sbx_enabled == Some(true) {
                out.push(("SBX".into(), Tone::Lit));
            }
            if s.scout_mode == Some(true) {
                out.push(("Scout Mode".into(), Tone::Lit));
            }
        }
        Page::Equalizer if s.eq_enabled == Some(true) => out.push(("On".into(), Tone::Lit)),
        Page::Playback if s.direct_mode == Some(true) => {
            out.push(("Direct mode".into(), Tone::Lit))
        }
        _ => {}
    }
    out
}

/// The cards of a page. The signal chain isn't made of cards: see [`draw_chain`].
pub fn cards(page: Page, m: &Model, prefs: &Prefs) -> Vec<Card<'static>> {
    match page {
        Page::Sbx => sbx(m),
        Page::Equalizer => equalizer(m),
        Page::Playback => playback(m),
        Page::Mixer => mixer(m),
        Page::Microphone => microphone(m),
        Page::Lighting => lighting(m),
        Page::Settings => settings(m, prefs),
        Page::Chain => Vec::new(),
    }
}

// ---- SBX ---------------------------------------------------------------------------------------

const EFFECTS: [(SbxEffect, &str, &str); 5] = [
    (SbxEffect::Surround, "Surround", "Widens the sound stage."),
    (
        SbxEffect::Crystalizer,
        "Crystalizer",
        "Brings back detail in compressed music.",
    ),
    (SbxEffect::Bass, "Bass", "Adds weight to low notes."),
    (
        SbxEffect::SmartVolume,
        "Smart Volume",
        "Evens out loud and quiet parts.",
    ),
    (
        SbxEffect::DialogPlus,
        "Dialog+",
        "Lifts voices over the background.",
    ),
];

fn sbx(m: &Model) -> Vec<Card<'static>> {
    let s = &m.status;
    let bypassed = m.direct();
    let mut out = vec![
        Card::new(
            "Processing mode",
            vec![
                toggle_row("SBX", s.sbx_enabled)
                    .help("Master switch for the five effects below. Turns Scout Mode off.")
                    .item(),
                toggle_row("Scout Mode", s.scout_mode)
                    .help("Makes footsteps and quiet cues easier to hear. Turns SBX off.")
                    .item(),
            ],
        )
        .span()
        .off(bypassed),
    ];
    let effects_off = bypassed || s.sbx_enabled == Some(false);
    for (effect, name, help) in EFFECTS {
        let st = s.sbx_effect(effect);
        let mut items = vec![row("Level", percent_slider(st.and_then(|e| e.level))).item()];
        if effect == SbxEffect::SmartVolume {
            let sel = facts::index_of(&facts::SMART_MODES, s.smart_volume_mode.as_deref());
            items.push(row("Mode", seg(&facts::SMART_MODES, sel)).item());
        }
        items.push(Item::Note(help.into()));
        out.push(
            Card::new(name, items)
                .head(st.and_then(|e| e.enabled))
                .off(effects_off),
        );
    }
    out
}

// ---- Equalizer ---------------------------------------------------------------------------------

fn equalizer(m: &Model) -> Vec<Card<'static>> {
    let s = &m.status;
    let curve = facts::eq_curve(s);
    let preset = match (curve, facts::eq_preset_name(s)) {
        (None, _) => DASH.to_string(),
        (Some(_), Some(name)) => name.to_string(),
        (Some(_), None) => "Custom".to_string(),
    };
    let which = s.output.clone().unwrap_or_else(|| "current".into());
    let on = s.eq_enabled;
    let bands = s.eq_bands_db.unwrap_or([None; 10]);
    let pre = s.eq_preamp_db;
    let off = on == Some(false);
    let preset_row = row("Preset", Ctl::Dropdown(preset))
        .help(&format!(
            "A preset has one curve per output. This is its {which} curve."
        ))
        .item();
    let curve_part = Part::Draw(Draw {
        h: 150.0,
        f: Box::new(move |p, env, r| eq_curve(p, env, r, pre, bands, off)),
    });
    let bands_part = Part::Draw(Draw {
        h: 210.0,
        f: Box::new(move |p, env, r| eq_bands(p, env, r, pre, bands, off)),
    });
    vec![
        Card::new("Equalizer", vec![preset_row])
            .head(on)
            .span()
            .then(curve_part)
            .then(bands_part),
    ]
}

fn db_frac(v: f64) -> f32 {
    ((v - EQ_DB_MIN) / (EQ_DB_MAX - EQ_DB_MIN)).clamp(0.0, 1.0) as f32
}

fn eq_curve(
    p: &mut Painter,
    env: &mut Env,
    r: Rect,
    pre: Option<f64>,
    bands: [Option<f64>; 10],
    off: bool,
) {
    let pal = env.pal;
    let dim = dimmed(&pal, pal.popup);
    let line = if off { dim.muted } else { pal.accent };
    let area = Rect::new(r.x + 34.0, r.y + 8.0, r.w - 34.0 - 4.0, r.h - 16.0);
    p.frame(area, 6.0, pal.field, pal.popup_line);
    let px = p.px().max(1.0);
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = area.bottom() - 4.0 - db_frac(db) * (area.h - 8.0);
        let c = if db == 0.0 {
            pal.popup_line
        } else {
            mix(pal.field, pal.popup_line, 0.5)
        };
        p.fill(Rect::new(area.x + 1.0, y, area.w - 2.0, px), c);
        let label = if db > 0.0 {
            format!("+{db:.0}")
        } else {
            format!("{db:.0}")
        };
        p.text_right(Face::Ui, 10.5, area.x - 6.0, y, &label, dim.muted);
    }
    let pts: Vec<Option<(f32, f32)>> = bands
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let x = area.x + 20.0 + (area.w - 40.0) * i as f32 / 9.0;
            b.map(|v| {
                let v = v + pre.unwrap_or(0.0);
                (x, area.bottom() - 4.0 - db_frac(v) * (area.h - 8.0))
            })
        })
        .collect();
    let old = p.clip(area.inset(1.0));
    for w in pts.windows(2) {
        if let [Some(a), Some(b)] = w {
            segment(p, *a, *b, 1.0, line);
        }
    }
    for pt in pts.iter().flatten() {
        p.rbox(
            Rect::new(pt.0 - 5.5, pt.1 - 5.5, 11.0, 11.0),
            5.5,
            pal.field,
        );
        p.rbox(Rect::new(pt.0 - 4.0, pt.1 - 4.0, 8.0, 8.0), 4.0, line);
    }
    p.restore(old);
}

/// A line segment of width `2 * half`, as a filled quad.
fn segment(p: &mut Painter, a: (f32, f32), b: (f32, f32), half: f32, c: Rgb) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(0.001);
    let (nx, ny) = (-dy / len * half, dx / len * half);
    p.polygon(
        &[
            (a.0 + nx, a.1 + ny),
            (b.0 + nx, b.1 + ny),
            (b.0 - nx, b.1 - ny),
            (a.0 - nx, a.1 - ny),
        ],
        c,
    );
}

pub fn vslider(
    p: &mut Painter,
    pal: &mdshared_ui::theme::Palette,
    r: Rect,
    frac: Option<f32>,
    from_middle: bool,
    ground: Rgb,
) {
    let (y0, y1) = (r.y + 7.0, r.bottom() - 7.0);
    let groove = Rect::new(r.cx() - 2.0, y0, 4.0, (y1 - y0).max(0.0));
    p.frame(groove, 2.0, pal.field, mix(pal.muted, ground, 0.55));
    let Some(f) = frac else { return };
    let ky = y1 - f.clamp(0.0, 1.0) * (y1 - y0);
    let from = if from_middle { (y0 + y1) / 2.0 } else { y1 };
    let (a, b) = if ky <= from { (ky, from) } else { (from, ky) };
    if b - a > 0.5 {
        p.rbox(Rect::new(groove.x, a, groove.w, b - a), 2.0, pal.accent);
    }
    p.rbox(Rect::new(r.cx() - 9.0, ky - 9.0, 18.0, 18.0), 9.0, ground);
    p.rbox(
        Rect::new(r.cx() - 7.0, ky - 7.0, 14.0, 14.0),
        7.0,
        pal.accent,
    );
}

fn eq_bands(
    p: &mut Painter,
    env: &mut Env,
    r: Rect,
    pre: Option<f64>,
    bands: [Option<f64>; 10],
    off: bool,
) {
    let pal = env.pal;
    let dim = if off {
        dimmed(&dimmed(&pal, pal.popup), pal.popup)
    } else {
        dimmed(&pal, pal.popup)
    };
    let labels = facts::band_labels();
    let n = 11;
    let col_w = r.w / n as f32;
    for i in 0..n {
        let x = r.x + col_w * i as f32;
        let (name, v) = if i == 0 {
            ("Pre".to_string(), pre)
        } else {
            (labels[i - 1].clone(), bands[i - 1])
        };
        let col = Rect::new(x, r.y, col_w, r.h);
        p.text_center(
            Face::Ui,
            11.0,
            Rect::new(col.x, col.y, col.w, 18.0),
            &name,
            dim.muted,
        );
        vslider(
            p,
            &dim,
            Rect::new(col.x, col.y + 18.0, col.w, col.h - 18.0 - 20.0),
            v.map(db_frac),
            true,
            pal.popup,
        );
        let text = v.map_or(DASH.to_string(), facts::db_short);
        p.text_center(
            Face::Ui,
            11.0,
            Rect::new(col.x, col.bottom() - 20.0, col.w, 20.0),
            &text,
            dim.text,
        );
        if i == 0 {
            let px = p.px().max(1.0);
            p.fill(
                Rect::new(col.right() - px, col.y + 22.0, px, col.h - 28.0),
                pal.popup_line,
            );
        }
    }
    debug_assert_eq!(EQ_BAND_FREQS.len(), 10);
}

// ---- Playback and decoder ----------------------------------------------------------------------

fn playback(m: &Model) -> Vec<Card<'static>> {
    let s = &m.status;
    let out_idx = facts::output_index(s);
    let layout_row = |i: usize| {
        let name = facts::OUTPUTS[i];
        let r = row(name, seg(&facts::LAYOUTS, None));
        if out_idx == Some(i) {
            r.badge("In use", Tone::Quiet)
        } else {
            r
        }
        .item()
    };
    let dec = facts::index_of(&facts::DECODER_MODES, s.decoder_mode.as_deref());
    let filter = facts::filter_label(s).unwrap_or_else(|| DASH.into());
    let mut decoder = vec![
        row("Dynamic range", seg(&facts::DECODER_MODES, dec))
            .help("For Dolby Digital input.")
            .item(),
        row("DAC filter", Ctl::Dropdown(filter)).item(),
    ];
    if m.direct() && (s.decoder_mode.is_none() || s.dac_filter.is_none()) {
        decoder.push(Item::Note(
            "The G6 reports no decoder mode or filter while Direct mode is on.".into(),
        ));
    }
    vec![
        Card::new(
            "Output",
            vec![
                row("Output", seg(&facts::OUTPUTS, out_idx)).help("Keeps the volume.").item(),
                row(
                    "Volume",
                    Ctl::Slider {
                        frac: m.mixer.as_ref().and_then(|x| x.master.volume).map(|v| f32::from(v) / 100.0),
                        reading: m.mixer.as_ref().and_then(|x| x.master.volume).map_or(DASH.into(), |v| v.to_string()),
                        from_middle: false,
                    },
                )
                .item(),
                row("Mute", Ctl::Toggle(m.mixer.as_ref().and_then(|x| x.master.muted))).item(),
                Item::Sub("Speaker setup".into()),
                layout_row(0),
                layout_row(1),
                Item::Note("Kept per output. The G6 gives no read-back of the speaker setup yet, so none is chosen.".into()),
            ],
        ),
        Card::new("Decoder", decoder),
        Card::new(
            "Signal path",
            vec![
                toggle_row("Direct mode", s.direct_mode)
                    .help("The shortest path to the DAC. Turns off SBX, Scout Mode and SPDIF-Out Direct.")
                    .item(),
                toggle_row("SPDIF-Out Direct", s.spdif_direct)
                    .help("Passes the stream to the optical output untouched. Turns off Direct mode.")
                    .item(),
                toggle_row("Headphone virtualization to Line Out", s.line_out_headphone_virtualization)
                    .help("For Line Out: the Speakers output.")
                    .item(),
            ],
        )
        .span(),
    ]
}

// ---- Mixer -------------------------------------------------------------------------------------

const MONITORING: [&str; 4] = ["Line In", "External Mic", "SPDIF In", "Mic"];
const RECORDING: [&str; 5] = ["Line In", "External Mic", "SPDIF In", "What U Hear", "Mic"];

fn mixer(m: &Model) -> Vec<Card<'static>> {
    let s = &m.status;
    let mx = m.mixer.clone();
    let strip_of = |f: &dyn Fn(&MixerView) -> Strip| mx.as_ref().map(f);
    let mon: Vec<(String, Option<Strip>)> = (0..4)
        .map(|i| (MONITORING[i].to_string(), strip_of(&|v| v.monitoring[i])))
        .collect();
    let rec: Vec<(String, Option<Strip>)> = (0..5)
        .map(|i| (RECORDING[i].to_string(), strip_of(&|v| v.recording[i])))
        .collect();
    let master = vec![(
        s.output.clone().unwrap_or_else(|| "Playback".into()),
        strip_of(&|v| v.master),
    )];
    let mix_muted = mx.as_ref().and_then(|v| v.mix_muted);
    let group = |title: &str,
                 strips: Vec<(String, Option<Strip>)>,
                 head: Option<Option<bool>>,
                 wide: bool| {
        let mut c = Card::new(title, Vec::new());
        c.parts.clear();
        if let Some(h) = head {
            c = c.head(h);
        }
        c.then(Part::Draw(Draw {
            h: 240.0,
            f: Box::new(move |p, env, r| strips_rack(p, env, r, &strips, wide)),
        }))
    };
    vec![
        group("Monitoring", mon, None, false),
        group("Recording sources", rec, None, false),
        group("Playback · mute mix", master, Some(mix_muted), true),
        Card::new(
            "Output and inputs",
            vec![
                row("Output", seg(&facts::OUTPUTS, facts::output_index(s))).item(),
                row("Speaker setup", seg(&facts::LAYOUTS, None)).item(),
                row("Mic boost (dB)", seg(&facts::MIC_BOOSTS, facts::mic_boost_index(s))).item(),
                toggle_row("Direct mode", s.direct_mode).item(),
                Item::Note(if m.mixer.is_some() {
                    "Levels and mutes are the Windows ones (Core Audio).".into()
                } else {
                    "Levels and mutes are the Windows ones (Core Audio). They show — until they are read from it.".into()
                }),
            ],
        )
        .span(),
    ]
}

fn strips_rack(
    p: &mut Painter,
    env: &mut Env,
    r: Rect,
    strips: &[(String, Option<Strip>)],
    wide: bool,
) {
    let pal = env.pal;
    let dim = dimmed(&pal, pal.popup);
    let w = (r.w / strips.len().max(1) as f32)
        .min(if wide { 92.0 } else { 64.0 })
        .floor();
    let total = w * strips.len() as f32;
    let x0 = r.x + ((r.w - total) / 2.0).max(0.0);
    for (i, (name, st)) in strips.iter().enumerate() {
        let col = Rect::new(x0 + w * i as f32, r.y + 4.0, w, r.h - 4.0);
        if i > 0 {
            let px = p.px().max(1.0);
            p.fill(Rect::new(col.x, col.y, px, col.h), pal.popup_line);
        }
        let lines = kit::wrap(p, Face::Ui, 11.5, name, w - 6.0);
        for (k, l) in lines.iter().take(2).enumerate() {
            let y = col.y + 9.0 + 13.0 * k as f32 + if lines.len() == 1 { 6.0 } else { 0.0 };
            p.text_center(
                Face::Ui,
                11.5,
                Rect::new(col.x + 3.0, y - 7.0, w - 6.0, 14.0),
                l,
                dim.text,
            );
        }
        // Meter (no source yet: empty), fader.
        let fd = Rect::new(col.x, col.y + 32.0, w, col.h - 32.0 - 56.0);
        let meter_w = 6.0;
        let fader_w = 24.0;
        let gap = 5.0;
        let stereo = if wide { 2.0 } else { 1.0 };
        let span_w = meter_w * stereo + gap * stereo + fader_w;
        let mut x = fd.x + (w - span_w) / 2.0;
        for _ in 0..stereo as usize {
            p.rbox(
                Rect::new(x, fd.y + 4.0, meter_w, fd.h - 8.0),
                3.0,
                pal.field,
            );
            x += meter_w + gap;
        }
        let vol = st.and_then(|s| s.volume);
        vslider(
            p,
            &dim,
            Rect::new(x, fd.y, fader_w, fd.h),
            vol.map(|v| f32::from(v) / 100.0),
            false,
            pal.popup,
        );
        let reading = vol.map_or(DASH.to_string(), |v| v.to_string());
        let (face, size) = if wide {
            (Face::Semibold, 16.0)
        } else {
            (Face::Ui, 13.0)
        };
        p.text_center(
            face,
            size,
            Rect::new(col.x, fd.bottom() + 4.0, w, 20.0),
            &reading,
            dim.text,
        );
        let muted = st.and_then(|s| s.muted);
        let btn = Rect::new(col.x + (w - 44.0) / 2.0, col.bottom() - 30.0, 44.0, 24.0);
        let (fill, ink) = if muted == Some(true) {
            (mix(pal.popup, pal.warn, 0.35), dim.text)
        } else {
            (mix(pal.popup, pal.text, 0.04), dim.muted)
        };
        p.frame(btn, 5.0, fill, dim.popup_line);
        p.text_center(Face::Ui, 12.0, btn, "Mute", ink);
    }
}

// ---- Microphone --------------------------------------------------------------------------------

fn microphone(m: &Model) -> Vec<Card<'static>> {
    let s = &m.status;
    let rec = s.recording.unwrap_or_default();
    let vc_off = s.voice_clarity == Some(false);
    let mic_eq = rec.mic_eq;
    let mut preset_row = row(
        "Mic EQ preset",
        Ctl::Dropdown(facts::mic_eq_label(s).unwrap_or_else(|| DASH.into())),
    )
    .child();
    if mic_eq == Some(false) {
        preset_row = preset_row.help("Needs Mic EQ.");
    }
    vec![
        Card::new(
            "Microphone",
            vec![
                row("Mic boost (dB)", seg(&facts::MIC_BOOSTS, facts::mic_boost_index(s))).item(),
                row("Recording volume", Ctl::Slider { frac: None, reading: DASH.into(), from_middle: false }).item(),
                toggle_row("Mute recording", None).item(),
                row("Monitoring volume", Ctl::Slider { frac: None, reading: DASH.into(), from_middle: false }).item(),
                toggle_row("Mute monitoring", None).item(),
                Item::Note("Monitoring plays the microphone back on the output. Volumes and mutes show — until they are read from Windows.".into()),
            ],
        ),
        Card::new(
            "Voice Clarity",
            vec![
                row(
                    "Noise reduction",
                    Ctl::Slider {
                        frac: frac(rec.noise_reduction_level, 100.0),
                        reading: facts::percent(rec.noise_reduction_level),
                        from_middle: false,
                    },
                )
                .item(),
                toggle_row("Acoustic echo cancellation", rec.acoustic_echo_cancellation).item(),
                toggle_row("Smart Volume", rec.smart_volume).item(),
                toggle_row("Mic EQ", mic_eq).item(),
                preset_row.item(),
            ],
        )
        .head(s.voice_clarity)
        .off(vc_off),
    ]
}

// ---- Lighting ----------------------------------------------------------------------------------

fn lighting(m: &Model) -> Vec<Card<'static>> {
    let s = &m.status;
    let on = s.rgb_enabled;
    let ring_on = s.ring_enabled;
    vec![
        Card::new(
            "Lighting",
            vec![
                row("Colour", value(DASH)).help("The G6 can't be asked which colour it shows.").item(),
                toggle_row("Volume ring light", ring_on).help("The LED indicator round the knob.").item(),
            ],
        )
        .head(on)
        .then(Part::Draw(Draw {
            h: 112.0,
            f: Box::new(move |p, env, r| light_ring(p, env, r, on)),
        })),
        Card::new(
            "Effects",
            vec![
                row("Effect", seg(&["Off", "Pulsate", "Cycle", "Music"], None)).item(),
                row("Period", value(DASH)).help("Seconds for one pulse or colour turn. Needs an effect.").item(),
                toggle_row("Music: cycle colours", None).help("Needs the Music effect.").item(),
                Item::Note("Host-side effects: they run while this app is open, so there is nothing to read back. Music follows the playback level (Windows).".into()),
            ],
        ),
    ]
}

fn light_ring(p: &mut Painter, env: &mut Env, r: Rect, on: Option<bool>) {
    let pal = env.pal;
    let dim = dimmed(&pal, pal.popup);
    let c = (r.x + 56.0, r.cy());
    let disc = if pal.dark {
        [0x2a, 0x2a, 0x2e]
    } else {
        [0x4a, 0x4a, 0x50]
    };
    p.rbox(
        Rect::new(c.0 - 46.0, c.1 - 46.0, 92.0, 92.0),
        46.0,
        [0x18, 0x18, 0x1a],
    );
    p.rbox(Rect::new(c.0 - 40.0, c.1 - 40.0, 80.0, 80.0), 40.0, disc);
    let ring = match on {
        Some(true) => mix(pal.text, pal.popup, 0.2),
        _ => [0x3a, 0x3a, 0x40],
    };
    p.rbox(Rect::new(c.0 - 35.0, c.1 - 35.0, 70.0, 70.0), 35.0, ring);
    p.rbox(
        Rect::new(c.0 - 30.0, c.1 - 30.0, 60.0, 60.0),
        30.0,
        [0x18, 0x18, 0x1a],
    );
    let text = match on {
        Some(true) => "Lit. The G6 can't be asked which colour it shows.",
        Some(false) => "Off.",
        None => "State not read.",
    };
    let lines = kit::wrap(p, Face::Ui, 12.0, text, r.w - 120.0);
    for (i, line) in lines.iter().enumerate() {
        let y = c.1 - 8.0 * (lines.len() as f32 - 1.0) + 16.0 * i as f32;
        p.text(Face::Ui, 12.0, r.x + 120.0, y, line, dim.muted, r.w - 120.0);
    }
}

// ---- Settings ----------------------------------------------------------------------------------

fn dl(label: &str, text: impl Into<String>) -> Item {
    row(label, value(text)).item()
}

pub fn device_rows(s: &DeviceStatus) -> Vec<(&'static str, String)> {
    vec![
        (
            "Firmware",
            s.firmware.clone().unwrap_or_else(|| DASH.into()),
        ),
        ("Output", s.output.clone().unwrap_or_else(|| DASH.into())),
        ("Direct mode", facts::on_off(s.direct_mode)),
        ("SPDIF-Out Direct", facts::on_off(s.spdif_direct)),
        (
            "Decoder mode",
            s.decoder_mode.clone().unwrap_or_else(|| DASH.into()),
        ),
        (
            "DAC filter",
            facts::filter_label(s).unwrap_or_else(|| DASH.into()),
        ),
        ("SBX", facts::on_off(s.sbx_enabled)),
        ("Scout Mode", facts::on_off(s.scout_mode)),
        ("Equalizer", facts::on_off(s.eq_enabled)),
        (
            "Mic boost",
            s.mic_boost_db.map_or(DASH.into(), |d| format!("{d} dB")),
        ),
    ]
}

fn settings(m: &Model, prefs: &Prefs) -> Vec<Card<'static>> {
    let s = &m.status;
    let mut status: Vec<Item> = device_rows(s)
        .into_iter()
        .map(|(l, v)| {
            if l == "Firmware" {
                row(
                    l,
                    Ctl::Value {
                        text: v,
                        mono: true,
                    },
                )
                .item()
            } else {
                dl(l, v)
            }
        })
        .collect();
    status.push(dl("Speaker setup", DASH));
    status.push(dl("Transport", "HID, interface 4"));
    status.push(
        row(
            "Read the status again",
            Ctl::Button {
                label: "Read again".into(),
                hit: Hit::Refresh,
            },
        )
        .item(),
    );
    vec![
        Card::new(
            "Profiles",
            vec![Item::Note(
                "A profile loads many settings onto the G6 at once, so profiles come with the write path. The command line has them today: g6 --profile-save and --profile-load.".into(),
            )],
        ),
        Card::new("Device status", status),
        Card::new("Appearance", appearance_rows(prefs)).span(),
    ]
}

pub fn appearance_rows(prefs: &Prefs) -> Vec<Item> {
    vec![
        row(
            "Theme",
            live_seg(&Theme::LABELS, Some(prefs.theme.index()), Hit::Theme),
        )
        .item(),
        row(
            "Window backdrop",
            live_seg(
                &Backdrop::LABELS,
                Some(prefs.backdrop.index()),
                Hit::Backdrop,
            ),
        )
        .help(Backdrop::HELP)
        .item(),
        row(
            "Window corners",
            live_seg(&Corners::LABELS, Some(prefs.corners.index()), Hit::Corners),
        )
        .help(corners_help())
        .item(),
    ]
}

fn corners_help() -> &'static str {
    mdshared_ui::corners::setting_help()
}

// ---- Signal chain ------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Light {
    Live,
    Idle,
    Off,
}

struct Node {
    title: String,
    sub: String,
    light: Light,
    page: Option<Page>,
    switch: Option<Option<bool>>,
    icon: Option<&'static str>,
    color: Option<Rgb>,
}

fn node(title: &str, sub: impl Into<String>, light: Light, page: Option<Page>) -> Node {
    Node {
        title: title.into(),
        sub: sub.into(),
        light,
        page,
        switch: None,
        icon: None,
        color: None,
    }
}

impl Node {
    fn sw(mut self, on: Option<bool>) -> Node {
        self.switch = Some(on);
        self
    }
    fn icon(mut self, g: &'static str) -> Node {
        self.icon = Some(g);
        self
    }
}

const NODE_H: f32 = 62.0;
const WIRE: f32 = 20.0;
const NODE_MIN: f32 = 120.0;

fn playback_nodes(m: &Model) -> Vec<Node> {
    let s = &m.status;
    let direct = m.direct();
    let on = |v: Option<bool>| v == Some(true);
    let sbx_sub = if direct {
        "Bypassed".to_string()
    } else if on(s.scout_mode) {
        "Scout Mode".into()
    } else if on(s.sbx_enabled) {
        let names: Vec<&str> = EFFECTS
            .iter()
            .filter(|(e, _, _)| s.sbx_effect(*e).and_then(|x| x.enabled) == Some(true))
            .map(|(_, n, _)| *n)
            .collect();
        if names.is_empty() {
            "No effect on".into()
        } else {
            names.join(", ")
        }
    } else {
        "Off".into()
    };
    let sbx_light = if direct {
        Light::Idle
    } else if on(s.sbx_enabled) || on(s.scout_mode) {
        Light::Live
    } else {
        Light::Off
    };
    let eq_sub = match (s.eq_enabled, facts::eq_curve(s)) {
        (Some(true), Some(_)) => facts::eq_preset_name(s).unwrap_or("Custom").to_string(),
        (Some(true), None) => "On".into(),
        (Some(false), _) => "Off".into(),
        (None, _) => DASH.into(),
    };
    let out_name = s.output.clone().unwrap_or_else(|| "Output".into());
    let out_sub = match m.mixer.as_ref().map(|x| x.master) {
        Some(Strip {
            volume: Some(v),
            muted,
        }) if muted != Some(true) => format!("Volume {v}"),
        Some(Strip {
            muted: Some(true), ..
        }) => "Muted".into(),
        _ => "Volume —".into(),
    };
    vec![
        node("PC audio", "USB · SPDIF In", Light::Live, None).icon(glyph::RIGHT),
        node(
            "Decoder",
            s.decoder_mode
                .as_ref()
                .map_or(DASH.into(), |d| format!("{d} range")),
            Light::Live,
            Some(Page::Playback),
        ),
        node("SBX", sbx_sub, sbx_light, Some(Page::Sbx)).sw(s.sbx_enabled),
        node(
            "Equalizer",
            eq_sub,
            if on(s.eq_enabled) {
                Light::Live
            } else {
                Light::Off
            },
            Some(Page::Equalizer),
        )
        .sw(s.eq_enabled),
        node(
            "DAC",
            facts::filter_label(s).map_or(DASH.into(), |f| {
                f.replace(" Roll-off / ", ", ").replace(" Phase", "")
            }),
            Light::Live,
            Some(Page::Playback),
        ),
        node(&out_name, out_sub, Light::Live, Some(Page::Playback)).icon(icons::SPEAKER),
    ]
}

fn mic_nodes(m: &Model) -> Vec<Node> {
    let s = &m.status;
    let rec = s.recording.unwrap_or_default();
    let vc = s.voice_clarity == Some(true);
    let more = [rec.acoustic_echo_cancellation, rec.smart_volume, rec.mic_eq]
        .iter()
        .filter(|v| **v == Some(true))
        .count();
    let vc_sub = if vc {
        format!(
            "Noise reduction {}{}",
            rec.noise_reduction_level
                .map_or(DASH.into(), |v| v.to_string()),
            if more > 0 {
                format!(", {more} more")
            } else {
                String::new()
            }
        )
    } else if s.voice_clarity.is_none() {
        DASH.into()
    } else {
        "Off".into()
    };
    vec![
        node("Mic in", "External Mic", Light::Live, None).icon(icons::MIC),
        node(
            "Mic boost",
            s.mic_boost_db.map_or(DASH.into(), |d| format!("+{d} dB")),
            Light::Live,
            Some(Page::Microphone),
        ),
        node(
            "Voice Clarity",
            vc_sub,
            if vc { Light::Live } else { Light::Off },
            Some(Page::Microphone),
        )
        .sw(s.voice_clarity),
        node("Recording", "Volume —", Light::Live, Some(Page::Mixer)),
        node("Monitoring", "Volume —", Light::Idle, Some(Page::Mixer)),
        node("Mixer", "Levels —", Light::Live, Some(Page::Mixer)).icon(icons::MIXER),
    ]
}

fn device_nodes(m: &Model) -> Vec<Node> {
    let s = &m.status;
    let mut light = node(
        "Lighting",
        match s.rgb_enabled {
            Some(true) => "On",
            Some(false) => "Off",
            None => DASH,
        },
        if s.rgb_enabled == Some(true) {
            Light::Live
        } else {
            Light::Off
        },
        Some(Page::Lighting),
    )
    .sw(s.rgb_enabled)
    .icon(icons::BULB);
    light.color = None;
    vec![
        light,
        node(
            "G6",
            s.firmware.clone().unwrap_or_else(|| DASH.into()),
            Light::Live,
            Some(Page::Settings),
        )
        .icon(glyph::INFO),
    ]
}

/// The signal chain: three lanes of stages. Returns the height used. A lane wraps into more rows when the page is
/// too narrow for it (the column layout under 730 px is step G8).
pub fn draw_chain(p: &mut Painter, env: &mut Env, area: Rect) -> f32 {
    let mut y = area.y;
    let lanes: [(&str, Vec<Node>, bool); 3] = [
        ("Playback", playback_nodes(env.model), true),
        ("Microphone and inputs", mic_nodes(env.model), false),
        ("Device", device_nodes(env.model), false),
    ];
    for (title, nodes, playback) in &lanes {
        p.text(
            Face::Semibold,
            13.0,
            area.x,
            y + 12.0,
            title,
            env.pal.text,
            area.w,
        );
        y += 28.0;
        y += draw_lane(p, env, Rect::new(area.x, y, area.w, 0.0), nodes, *playback);
        y += 18.0;
    }
    y - area.y - 18.0
}

fn draw_lane(p: &mut Painter, env: &mut Env, area: Rect, nodes: &[Node], playback: bool) -> f32 {
    let pal = env.pal;
    let per_row = (((area.w + WIRE) / (NODE_MIN + WIRE)).floor() as usize).clamp(1, nodes.len());
    let in_row = per_row.min(nodes.len());
    let node_w = ((area.w - WIRE * (in_row - 1) as f32) / in_row as f32)
        .floor()
        .min(210.0);
    let direct = env.model.direct();
    let mut rows = 0;
    // Positions of SBX's neighbours' wires, for the bypass.
    let mut bypass: Option<(f32, f32, f32)> = None;
    for (i, n) in nodes.iter().enumerate() {
        let (row_i, col) = (i / per_row, i % per_row);
        rows = row_i + 1;
        let x = area.x + col as f32 * (node_w + WIRE);
        let y = area.y + row_i as f32 * (NODE_H + 28.0) + if playback { 22.0 } else { 0.0 };
        let r = Rect::new(x, y, node_w, NODE_H);
        draw_node(p, env, n, r);
        if col > 0 {
            // The wire between this stage and the one before: cold while Direct mode bypasses SBX.
            let cold = playback && direct && (n.title == "SBX" || n.title == "Equalizer");
            let c = if cold {
                pal.popup_line
            } else {
                mix(pal.muted, pal.page, 0.35)
            };
            let px = p.px().max(1.0) * 1.5;
            p.fill(
                Rect::new(x - WIRE + 2.0, y + NODE_H / 2.0 - px / 2.0, WIRE - 4.0, px),
                c,
            );
            p.fill(Rect::new(x - 6.0, y + NODE_H / 2.0 - 3.0, px, 6.0), c);
            if playback && n.title == "SBX" {
                bypass = Some((x - WIRE / 2.0, y, 0.0));
            }
            if playback
                && n.title == "Equalizer"
                && let Some((x0, y0, _)) = bypass
            {
                bypass = Some((x0, y0, x - WIRE / 2.0));
            }
        }
    }
    if playback
        && direct
        && let Some((x0, y0, x1)) = bypass.filter(|b| b.2 > 0.0)
    {
        // The bypass over SBX: Direct mode goes round it.
        let c = pal.accent;
        let px = p.px().max(1.0) * 1.5;
        let top = y0 - 12.0;
        p.fill(Rect::new(x0, top, px, y0 + NODE_H / 2.0 - top), c);
        p.fill(Rect::new(x1, top, px, y0 + NODE_H / 2.0 - top), c);
        p.fill(Rect::new(x0, top, x1 - x0 + px, px), c);
        let label = "Direct mode";
        let w = p.measure(Face::Semibold, 11.0, label) + 16.0;
        let b = Rect::new((x0 + x1) / 2.0 - w / 2.0, top - 10.0, w, 20.0);
        p.frame(b, 10.0, mix(pal.popup, pal.accent, 0.14), c);
        p.text_center(Face::Semibold, 11.0, b, label, pal.accent);
    }
    let lane_h = rows as f32 * (NODE_H + 28.0) - 28.0 + if playback { 22.0 } else { 0.0 };
    lane_h.max(NODE_H)
}

fn draw_node(p: &mut Painter, env: &mut Env, n: &Node, r: Rect) {
    let pal = env.pal;
    let hover = n.page.is_some_and(|pg| env.is_hover(Hit::Nav(pg)));
    let (fill, edge) = match n.light {
        Light::Live => (pal.popup, mix(pal.popup_line, pal.accent, 0.45)),
        Light::Idle => (pal.popup, pal.popup_line),
        Light::Off => (mix(pal.popup, pal.page, 0.45), pal.popup_line),
    };
    let fill = if hover {
        mix(fill, pal.text, 0.05)
    } else {
        fill
    };
    p.frame(r, 8.0, fill, edge);
    let faint = n.light != Light::Live;
    let ink = if faint { pal.muted } else { pal.text };
    let mut x = r.x + 12.0;
    if let Some(g) = n.icon {
        p.icon(g, 14.0, x + 7.0, r.y + 19.0, ink);
        x += 22.0;
    }
    let room = r.right()
        - x
        - 12.0
        - if n.switch.is_some() {
            TOGGLE_SMALL.0 + 8.0
        } else {
            18.0
        };
    p.text(Face::Semibold, 13.0, x, r.y + 19.0, &n.title, ink, room);
    p.text(
        Face::Ui,
        11.5,
        r.x + 12.0,
        r.y + 42.0,
        &n.sub,
        pal.muted,
        r.w - 24.0,
    );
    let led_c = match n.light {
        Light::Live => [0x3f, 0xb9, 0x50],
        Light::Idle => pal.warn,
        Light::Off => pal.faint,
    };
    match n.switch {
        Some(on) => {
            let dim = dimmed(&pal, pal.popup);
            kit::toggle(
                p,
                &dim,
                Rect::new(
                    r.right() - 12.0 - TOGGLE_SMALL.0,
                    r.y + 19.0 - TOGGLE_SMALL.1 / 2.0,
                    TOGGLE_SMALL.0,
                    TOGGLE_SMALL.1,
                ),
                on,
            );
        }
        None => kit::led(p, r.right() - 14.0, r.y + 19.0, led_c),
    }
    if let Some(pg) = n.page {
        env.hits.add(Hit::Nav(pg), r);
    }
}

const TOGGLE_SMALL: (f32, f32) = (34.0, 18.0);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::Sample;

    #[test]
    fn every_page_has_cards_but_the_chain() {
        let m = Sample::Live.model();
        for page in Page::ALL {
            assert_eq!(
                cards(page, &m, &Prefs::default()).is_empty(),
                page == Page::Chain,
                "{page:?}"
            );
        }
    }

    #[test]
    fn direct_mode_is_the_first_notice_on_the_sbx_page() {
        let m = Sample::Live.model();
        let n = notices(Page::Sbx, &m);
        assert_eq!(n.len(), 1);
        assert!(n[0].text.starts_with("Direct mode is on"));
        assert!(notices(Page::Equalizer, &m).is_empty());
        let mut busy = Sample::Busy.model();
        assert!(notices(Page::Sbx, &busy).is_empty());
        busy.status.sbx_enabled = Some(false);
        assert!(notices(Page::Sbx, &busy)[0].text.starts_with("SBX is off"));
    }

    #[test]
    fn the_device_rows_show_a_dash_for_what_was_not_read() {
        let rows = device_rows(&DeviceStatus::default());
        assert!(rows.iter().all(|(_, v)| v == DASH), "{rows:?}");
        let live = device_rows(&crate::sample::live_status());
        assert_eq!(live[0].1, "2.1.250903.1324");
    }
}
