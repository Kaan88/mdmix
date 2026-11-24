//! `--shot`: one page rendered to a PNG without a window and without a device, from a fixed device state.
//!
//! With a backdrop the frosted chrome is simulated: the frame is keyed the way the window's copy is
//! ([`mdshared_ui::frost::key_out`]) and laid over a sample wallpaper, so a shot shows what the chrome lets through.
//! The real thing is the system's blur and is only seen in the running app.

use std::path::{Path, PathBuf};

use mdshared_app::backdrop::Backdrop;
use mdshared_text::Bitmap;
use mdshared_ui::frost::{self, Key};
use mdshared_ui::gfx::{Gfx, Painter};
use mdshared_ui::theme::{DARK, LIGHT};

use crate::args::ShotArgs;
use crate::dialog;
use crate::frame::{FrameIn, draw_frame};
use crate::model::Page;
use crate::prefs::{Prefs, Theme};
use crate::sample::Sample;

/// Renders a shot to a bitmap.
pub fn render(args: &ShotArgs) -> Bitmap {
    let model = args.sample.model();
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
    let mut form = args
        .dialog
        .map(|page| dialog::form_at(&prefs, &model, page));
    let areas;
    {
        let mut p = Painter::new(&mut bmp, &mut gfx, args.scale);
        let input = FrameIn {
            model: &model,
            prefs: &prefs,
            pal,
            page: args.page,
            size: (w, h),
            scroll: 0.0,
            maximized: false,
            hover: None,
            pressed: None,
            toast: None,
            frosted: args.backdrop != Backdrop::Off,
        };
        let out = draw_frame(&mut p, &input, form.as_mut());
        areas = out
            .chrome
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
            .collect::<Vec<_>>();
    }
    if args.backdrop == Backdrop::Off {
        return bmp;
    }
    over_wallpaper(
        &bmp,
        &areas,
        Key::new(frost::key_colour(&pal), frost::veil(args.backdrop)),
    )
}

/// The frame as the window would hand it to the compositor, laid over a sample wallpaper: a soft diagonal blend of
/// two colours, standing in for what Mica or Acrylic shows.
fn over_wallpaper(bmp: &Bitmap, chrome: &[(usize, usize, usize, usize)], key: Key) -> Bitmap {
    let (w, h) = (bmp.width as usize, bmp.height as usize);
    let mut px: Vec<u32> = bmp
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    for a in chrome {
        frost::key_out(&key, &mut px, w, *a);
    }
    let mut out = Bitmap::new(bmp.width, bmp.height);
    let (from, to) = ([70.0_f32, 130.0, 200.0], [225.0_f32, 150.0, 170.0]);
    for y in 0..h {
        for x in 0..w {
            let t = (x as f32 / w as f32 * 0.6 + y as f32 / h as f32 * 0.4).clamp(0.0, 1.0);
            let wall = [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * t);
            let v = px[y * w + x];
            let a = (v >> 24) as f32 / 255.0;
            // Premultiplied: the pixel's colour plus what it lets through.
            let c = [(v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff].map(|c| c as f32);
            let o = (y * w + x) * 4;
            for i in 0..3 {
                let v = (c[i] + (1.0 - a) * wall[i]).round().clamp(0.0, 255.0) as u8;
                // The bitmap is BGRA.
                out.pixels[o + 2 - i] = v;
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
        // Frosted chrome, both kinds.
        for backdrop in [Backdrop::Mica, Backdrop::Acrylic] {
            shot(ShotArgs {
                backdrop,
                sample: Sample::Busy,
                ..ShotArgs::new(Page::Playback, dark)
            })?;
        }
        // The dialog's two pages.
        for d in [0, 1] {
            shot(ShotArgs {
                dialog: Some(d),
                ..ShotArgs::new(Page::Sbx, dark)
            })?;
        }
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
        let flat = render(&ShotArgs::new(Page::Sbx, true));
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
    fn an_unwritable_output_path_is_an_error_not_a_panic() {
        let a = ShotArgs {
            out: Some(PathBuf::from("")),
            ..ShotArgs::new(Page::Sbx, false)
        };
        // An empty path can't be a file: the error comes back, nothing panics.
        assert!(write(&a, Path::new("unused")).is_err());
    }
}
