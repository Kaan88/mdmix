//! `--shot`: one page rendered to a PNG without a window and without a device, from a fixed device state.
//!
//! With a backdrop the frosted chrome is simulated: the frame is keyed the way the window's copy is
//! ([`mdshared_ui::frost::key_out`]) and laid over a sample wallpaper, so a shot shows what the chrome lets through.
//! The real thing is the system's blur and is only seen in the running app.

use std::path::{Path, PathBuf};
use std::time::Instant;

use mdshared_app::backdrop::Backdrop;
use mdshared_text::Bitmap;
use mdshared_ui::frost::{self, Key};
use mdshared_ui::gfx::{Gfx, Painter};
use mdshared_ui::theme::{DARK, LIGHT};

use crate::args::ShotArgs;
use crate::dialog;
use crate::frame::{FrameIn, FrameOut, draw_frame, popup_area};
use crate::fx::Fx;
use crate::intent::Setting;
use crate::kit::Hit;
use crate::levels::Meters;
use crate::model::Page;
use crate::pick::{self, ColourPick};
use crate::prefs::{Prefs, Theme};
use crate::sample::Sample;
use mdshared_ui::gfx::Rgb;

/// Renders a shot to a bitmap.
pub fn render(args: &ShotArgs) -> Bitmap {
    let mut model = args.sample.model();
    if args.picker && model.colour.is_none() {
        // The picker shot shows a colour set from here: the G6 can't be asked which colour it shows.
        model.colour = Some([0, 200, 255]);
    }
    let prefs = Prefs {
        theme: if args.dark { Theme::Dark } else { Theme::Light },
        backdrop: args.backdrop,
        page: args.page,
        ..Prefs::default()
    };
    let pal = if args.dark { DARK } else { LIGHT };
    let (w, h) = (args.size.0 as f32, args.size.1 as f32);
    let mut bmp = Bitmap::new(
        (w * args.scale).round() as u32,
        (h * args.scale).round() as u32,
    );
    let mut gfx = Gfx::new();
    let mut fx = Fx::still();
    let mut form = args
        .dialog
        .map(|page| dialog::form_at(&prefs, &model, page));
    // No audio runs in a shot. The busy sample shows the meter at fixed readings at fixed instants (a held peak and
    // a quieter level), so the picture is the same every time; the live and away samples have no source and say
    // "no meter", as the window does when the endpoint's meter can't be opened.
    let now = Instant::now();
    let meters = match args.sample {
        Sample::Busy => Meters::busy(now),
        Sample::Live | Sample::Away => Meters::none(),
    };
    let input = FrameIn {
        model: &model,
        prefs: &prefs,
        pal,
        page: args.page,
        size: (w, h),
        scroll: 0.0,
        maximized: false,
        hover: None,
        pointer: None,
        now,
        pressed: None,
        dragging: None,
        frosted: args.backdrop != Backdrop::Off,
        meters: &meters,
    };
    let mut areas;
    {
        let mut p = Painter::new(&mut bmp, &mut gfx, args.scale);
        let out = draw_frame(&mut p, &input, form.as_mut(), &mut fx);
        areas = chrome_areas(&p, &out);
        // The colour picker is placed by the row the frame just recorded, as the window places it on its next frame.
        if args.picker
            && let Some(anchor) = out.hits.rect_of(Hit::Open(Setting::Colour))
        {
            let original = Setting::Colour.current(&model);
            let colour: Rgb = pick::rgb_or_fallback(original);
            let mut pk = ColourPick::open(anchor, colour, original);
            pk.place(popup_area((w, h)));
            fx.pick = Some(pk);
            let out = draw_frame(&mut p, &input, form.as_mut(), &mut fx);
            areas = chrome_areas(&p, &out);
        }
    }
    if args.backdrop == Backdrop::Off && !args.desk {
        return bmp;
    }
    over_wallpaper(
        &bmp,
        &areas,
        Key::new(frost::key_colour(&pal), frost::veil(args.backdrop)),
        Sim {
            tint: match args.backdrop {
                // The system's own tint of Mica, as the mockup has it: the key colour at .80 over the blurred wallpaper.
                Backdrop::Mica => Some((frost::key_colour(&pal), 0.80)),
                _ => None,
            },
            margin: if args.desk {
                (DESK_MARGIN * args.scale).round() as usize
            } else {
                0
            },
            scale: args.scale,
            chrome: args.backdrop != Backdrop::Off,
        },
    )
}

/// The chrome's rectangles in device pixels, for the frost simulation.
fn chrome_areas(p: &Painter, out: &FrameOut) -> Vec<(usize, usize, usize, usize)> {
    out.chrome
        .iter()
        .map(|r| {
            let d = p.dev_rect(*r);
            (
                d.0.max(0) as usize,
                d.1.max(0) as usize,
                d.2.max(0) as usize,
                d.3.max(0) as usize,
            )
        })
        .collect()
}

/// The room round the window in a `--desk` shot, as the mockup's desktop padding.
const DESK_MARGIN: f32 = 18.0;
/// The window's corner radius in a `--desk` shot (the mockup's Round).
const DESK_RADIUS: f32 = 8.0;

/// How a shot stands in for what the system does round the frame.
struct Sim {
    /// The tint the system lays over the blurred wallpaper (Mica).
    tint: Option<(mdshared_ui::gfx::Rgb, f32)>,
    /// Desktop showing round the window, with its rounded corners and shadow.
    margin: usize,
    /// The chrome is frosted: the wallpaper shows through it.
    chrome: bool,
    scale: f32,
}

/// The sample wallpaper at (`u`, `v`) in 0..1: the mockup's night one, blurred (a diagonal blend of dark blue, purple,
/// plum and rust, with a teal pool at the bottom left).
fn wallpaper(u: f32, v: f32) -> [f32; 3] {
    let hex = |c: u32| {
        [
            (c >> 16) as f32,
            ((c >> 8) & 0xff) as f32,
            (c & 0xff) as f32,
        ]
    };
    let stops = [
        (0.0, hex(0x0b1026)),
        (0.45, hex(0x27124a)),
        (0.75, hex(0x5a1d3c)),
        (1.0, hex(0xb3472c)),
    ];
    let t = (0.5 + (u - 0.5) * 0.342 + (v - 0.5) * 0.94).clamp(0.0, 1.0);
    let mut c = stops[3].1;
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let k = (t - w[0].0) / (w[1].0 - w[0].0);
            c = [0, 1, 2].map(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
            break;
        }
    }
    let d = ((u - 0.06) / 0.25).powi(2) + ((v - 0.7) / 0.7).powi(2);
    let pool = (1.0 - d).clamp(0.0, 1.0) * 0.85;
    let teal = hex(0x14506e);
    [0, 1, 2].map(|i| c[i] + (teal[i] - c[i]) * pool)
}

/// The signed distance from (`x`, `y`) to a `w` x `h` rounded rectangle at the origin: negative inside.
fn rounded_sdf(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (qx, qy) = (
        (x - w / 2.0).abs() - (w / 2.0 - r),
        (y - h / 2.0).abs() - (h / 2.0 - r),
    );
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// The frame as the window would hand it to the compositor, laid over a sample wallpaper that stands in for what
/// Mica or Acrylic shows (the real one is the system's blur and is only seen in the running app).
fn over_wallpaper(
    bmp: &Bitmap,
    chrome: &[(usize, usize, usize, usize)],
    key: Key,
    sim: Sim,
) -> Bitmap {
    let (w, h) = (bmp.width as usize, bmp.height as usize);
    let m = sim.margin;
    let mut px: Vec<u32> = bmp
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    if sim.chrome {
        for a in chrome {
            frost::key_out(&key, &mut px, w, *a);
        }
    }
    let (cw, ch) = (w + 2 * m, h + 2 * m);
    let mut out = Bitmap::new(cw as u32, ch as u32);
    for y in 0..ch {
        for x in 0..cw {
            let mut wall = wallpaper(x as f32 / cw as f32, y as f32 / ch as f32);
            if let Some((k, a)) = sim.tint {
                wall = [0, 1, 2].map(|i| wall[i] + (f32::from(k[i]) - wall[i]) * a);
            }
            let o = (y * cw + x) * 4;
            let (wx, wy) = (x as f32 + 0.5 - m as f32, y as f32 + 0.5 - m as f32);
            let mut rgb = wall;
            if m > 0 {
                // The desktop under the window: darker towards the window, as its shadow (36 px, drop 6, peak .35).
                let k = sim.scale;
                let d = rounded_sdf(wx, wy - 6.0 * k, w as f32, h as f32, DESK_RADIUS * k).max(0.0);
                let shade = 0.35 * (-(d * d) / (2.0 * 12.0 * k * 12.0 * k)).exp();
                rgb = rgb.map(|c| c * (1.0 - shade));
            }
            let inside = rounded_sdf(
                wx,
                wy,
                w as f32,
                h as f32,
                if m > 0 { DESK_RADIUS * sim.scale } else { 0.0 },
            );
            let cover = if m > 0 {
                (0.5 - inside).clamp(0.0, 1.0)
            } else {
                1.0
            };
            // The rounded rectangle can reach a pixel past the frame at a scale that isn't whole: read only inside it.
            if cover > 0.0 && x >= m && x < m + w && y >= m && y < m + h {
                let v = px[(y - m) * w + (x - m)];
                let a = (v >> 24) as f32 / 255.0;
                // Premultiplied: the pixel's colour plus what it lets through.
                let c = [(v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff].map(|c| c as f32);
                let win = [0, 1, 2].map(|i| c[i] + (1.0 - a) * wall[i]);
                rgb = [0, 1, 2].map(|i| rgb[i] + (win[i] - rgb[i]) * cover);
            }
            for (i, c) in rgb.iter().enumerate() {
                // The bitmap is BGRA.
                out.pixels[o + 2 - i] = c.round().clamp(0.0, 255.0) as u8;
            }
            out.pixels[o + 3] = 255;
        }
    }
    out
}

pub fn write(args: &ShotArgs, default_dir: &Path) -> Result<PathBuf, String> {
    let path = args
        .out
        .clone()
        .unwrap_or_else(|| default_dir.join(args.file_name()));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    render(args)
        .write_png(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Every page in both themes, and the other states worth looking at.
pub fn all(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut made = Vec::new();
    let mut shot = |a: ShotArgs| -> Result<(), String> {
        made.push(write(&a, dir)?);
        Ok(())
    };
    for dark in [false, true] {
        for page in Page::ALL {
            shot(ShotArgs::new(page, dark))?;
            shot(ShotArgs {
                sample: Sample::Busy,
                ..ShotArgs::new(page, dark)
            })?;
        }
        // No G6: the banner and the last status.
        shot(ShotArgs {
            sample: Sample::Away,
            ..ShotArgs::new(Page::Sbx, dark)
        })?;
        shot(ShotArgs {
            sample: Sample::Away,
            ..ShotArgs::new(Page::Mixer, dark)
        })?;
        // The chrome without a backdrop, and Acrylic (Mica is the default of every other shot).
        for backdrop in [Backdrop::Off, Backdrop::Acrylic] {
            shot(ShotArgs {
                backdrop,
                sample: Sample::Busy,
                ..ShotArgs::new(Page::Playback, dark)
            })?;
        }
        // The window on the desktop, to set beside the mockup.
        for page in [Page::Playback, Page::Mixer, Page::Sbx] {
            shot(ShotArgs {
                desk: true,
                sample: Sample::Busy,
                ..ShotArgs::new(page, dark)
            })?;
        }
        // The dialog's two pages.
        for d in [0, 1] {
            shot(ShotArgs {
                dialog: Some(d),
                ..ShotArgs::new(Page::Sbx, dark)
            })?;
        }
        // The colour picker open over the Lighting page.
        shot(ShotArgs {
            picker: true,
            sample: Sample::Busy,
            ..ShotArgs::new(Page::Lighting, dark)
        })?;
        // The smallest window.
        for page in [Page::Chain, Page::Mixer, Page::Equalizer] {
            shot(ShotArgs {
                size: (760, 600),
                sample: Sample::Busy,
                ..ShotArgs::new(page, dark)
            })?;
        }
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shot_is_the_asked_size_and_not_blank() {
        let b = render(&ShotArgs {
            size: (800, 600),
            scale: 1.5,
            ..ShotArgs::new(Page::Equalizer, true)
        });
        assert_eq!((b.width, b.height), (1200, 900));
        let distinct: std::collections::HashSet<_> = b
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        assert!(distinct.len() > 8);
    }

    #[test]
    fn a_frosted_shot_lets_the_wallpaper_through_the_chrome_only() {
        let flat = render(&ShotArgs {
            backdrop: Backdrop::Off,
            ..ShotArgs::new(Page::Sbx, true)
        });
        let mica = render(&ShotArgs {
            backdrop: Backdrop::Mica,
            ..ShotArgs::new(Page::Sbx, true)
        });
        let at = |b: &Bitmap, x: usize, y: usize| {
            let o = (y * b.width as usize + x) * 4;
            [b.pixels[o + 2], b.pixels[o + 1], b.pixels[o]]
        };
        // The sidebar, the title row: changed. The page: as it was.
        assert_ne!(at(&flat, 5, 700), at(&mica, 5, 700));
        assert_ne!(at(&flat, 500, 5), at(&mica, 500, 5));
        assert_eq!(at(&flat, 1100, 700), at(&mica, 1100, 700));
    }

    #[test]
    fn a_desk_shot_at_a_scale_that_is_not_whole_does_not_panic() {
        for scale in [1.1, 1.5, 0.7] {
            let b = render(&ShotArgs {
                desk: true,
                size: (300, 200),
                scale,
                ..ShotArgs::new(Page::Sbx, true)
            });
            let m = (18.0 * scale).round() as u32;
            assert_eq!(b.width, (300.0 * scale).round() as u32 + 2 * m);
        }
    }

    #[test]
    fn an_unwritable_output_path_is_an_error_not_a_panic() {
        let a = ShotArgs {
            out: Some(PathBuf::from("")),
            ..ShotArgs::new(Page::Sbx, false)
        };
        // An empty path can't be a file: the error comes back, nothing panics.
        assert!(write(&a, Path::new("unused")).is_err());
    }
}
