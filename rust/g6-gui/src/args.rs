//! The command line: open the window, or render shots without one.

use std::path::PathBuf;

use mdshared_app::backdrop::Backdrop;

use crate::model::Page;
use crate::prefs::Theme;
use crate::sample::Sample;

pub const USAGE: &str = "\
g6-gui: the Sound BlasterX G6 control panel (step G2: reads the device, writes nothing)

USAGE
  g6-gui [--page <page>]
      Open the window. --page picks the page to start on (sbx, eq, playback, mixer, mic, lighting, chain, system).

  g6-gui --shot <page> <light|dark> [options]
      Render one page to a PNG without a window and without a device, from a fixed device state.
        --sample <live|busy|away>    live: the recorded G6 (default); busy: invented values; away: no G6
        --backdrop <off|mica|acrylic> the frosted chrome over a sample wallpaper (default mica, the app's default)
        --desk                       the desktop round the window, with its corners and shadow
        --dialog <appearance|device> the settings dialog over the page
        --out <file.png>             default: target/g6-gui-shots/<name>.png
        --size <WxH>                 logical px (default 1180x780)
        --scale <n>                  device pixels per logical px (default 1)

  g6-gui --shots <dir>
      Render every page in both themes, and the other states, into <dir>.

  g6-gui --help
";

#[derive(Clone, Debug, PartialEq)]
pub struct ShotArgs {
    pub page: Page,
    pub dark: bool,
    pub backdrop: Backdrop,
    pub sample: Sample,
    /// 0 Appearance, 1 Device.
    pub dialog: Option<usize>,
    pub out: Option<PathBuf>,
    pub size: (u32, u32),
    pub scale: f32,
    /// Desktop round the window, with its rounded corners and shadow, as the mockup shows it.
    pub desk: bool,
}

impl ShotArgs {
    pub fn new(page: Page, dark: bool) -> ShotArgs {
        ShotArgs {
            page,
            dark,
            backdrop: Backdrop::Mica,
            sample: Sample::Live,
            dialog: None,
            out: None,
            size: (1180, 780),
            scale: 1.0,
            desk: false,
        }
    }

    /// The file name a shot gets when `--out` isn't given.
    pub fn file_name(&self) -> String {
        let mut s = String::new();
        if self.sample != Sample::Live {
            s += self.sample.name();
            s += "-";
        }
        // Mica is the default; the others are named.
        match self.backdrop {
            Backdrop::Mica => {}
            Backdrop::Off => s += "flat-",
            Backdrop::Acrylic => s += "acrylic-",
        }
        if self.desk {
            s += "desk-";
        }
        if let Some(d) = self.dialog {
            s += if d == 0 {
                "dialog-appearance-"
            } else {
                "dialog-device-"
            };
        }
        s += self.page.key();
        s += if self.dark { "-dark" } else { "-light" };
        if self.size != (1180, 780) {
            s += &format!("-{}x{}", self.size.0, self.size.1);
        }
        s + ".png"
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Run { page: Option<Page> },
    Shot(ShotArgs),
    Shots(PathBuf),
    Help,
}

pub fn parse(args: &[String]) -> Result<Mode, String> {
    let mut it = args.iter().peekable();
    let mut page = None;
    let mut shot: Option<ShotArgs> = None;
    let mut shots: Option<PathBuf> = None;
    let value = |it: &mut std::iter::Peekable<std::slice::Iter<String>>, flag: &str| {
        it.next()
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => return Ok(Mode::Help),
            "--page" => {
                let v = value(&mut it, a)?;
                page = Some(Page::from_key(&v).ok_or_else(|| format!("unknown page {v:?}"))?);
            }
            "--shot" => {
                let pg = value(&mut it, a)?;
                let th = value(&mut it, a)?;
                let pg = Page::from_key(&pg).ok_or_else(|| format!("unknown page {pg:?}"))?;
                let dark = match Theme::from_name(&th) {
                    Some(Theme::Dark) => true,
                    Some(Theme::Light) => false,
                    _ => return Err(format!("the theme of a shot is light or dark, not {th:?}")),
                };
                shot = Some(ShotArgs::new(pg, dark));
            }
            "--desk" => {
                shot.as_mut()
                    .ok_or_else(|| format!("{a} goes after --shot <page> <theme>"))?
                    .desk = true
            }
            "--shots" => shots = Some(PathBuf::from(value(&mut it, a)?)),
            "--backdrop" | "--sample" | "--dialog" | "--out" | "--size" | "--scale" => {
                let v = value(&mut it, a)?;
                let s = shot
                    .as_mut()
                    .ok_or_else(|| format!("{a} goes after --shot <page> <theme>"))?;
                match a.as_str() {
                    "--backdrop" => {
                        s.backdrop =
                            Backdrop::parse(&v).ok_or_else(|| format!("unknown backdrop {v:?}"))?
                    }
                    "--sample" => {
                        s.sample =
                            Sample::from_name(&v).ok_or_else(|| format!("unknown sample {v:?}"))?
                    }
                    "--dialog" => {
                        s.dialog = Some(match v.to_ascii_lowercase().as_str() {
                            "appearance" => 0,
                            "device" => 1,
                            _ => return Err(format!("unknown dialog page {v:?}")),
                        })
                    }
                    "--out" => s.out = Some(PathBuf::from(v)),
                    "--size" => s.size = parse_size(&v)?,
                    _ => {
                        s.scale = v
                            .parse::<f32>()
                            .ok()
                            .filter(|x| (0.5..=4.0).contains(x))
                            .ok_or_else(|| format!("--scale is 0.5 to 4, not {v:?}"))?
                    }
                }
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(match (shot, shots) {
        (Some(_), Some(_)) => return Err("--shot and --shots don't go together".into()),
        (Some(s), None) => Mode::Shot(s),
        (None, Some(d)) => Mode::Shots(d),
        (None, None) => Mode::Run { page },
    })
}

fn parse_size(v: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--size is WxH, like 1180x780, not {v:?}");
    let (w, h) = v.split_once(['x', 'X']).ok_or_else(bad)?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    if (400..=4000).contains(&w) && (300..=3000).contains(&h) {
        Ok((w, h))
    } else {
        Err(format!("--size {v:?} is out of range"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn no_arguments_open_the_window() {
        assert_eq!(parse(&[]), Ok(Mode::Run { page: None }));
        assert_eq!(
            parse(&args("--page eq")),
            Ok(Mode::Run {
                page: Some(Page::Equalizer)
            })
        );
    }

    #[test]
    fn a_shot_takes_its_options() {
        let Ok(Mode::Shot(s)) = parse(&args(
            "--shot mixer dark --backdrop acrylic --sample busy --size 800x600 --scale 2",
        )) else {
            panic!("not a shot");
        };
        assert_eq!(
            (s.page, s.dark, s.backdrop, s.sample),
            (Page::Mixer, true, Backdrop::Acrylic, Sample::Busy)
        );
        assert_eq!((s.size, s.scale), ((800, 600), 2.0));
        assert_eq!(s.file_name(), "busy-acrylic-mixer-dark-800x600.png");
    }

    #[test]
    fn bad_input_is_said_plainly() {
        for bad in [
            "--shot nope dark",
            "--shot sbx system",
            "--backdrop mica",
            "--shot sbx dark --size 5x5",
            "--shot sbx dark --scale 9",
            "--shot sbx dark --shots out",
            "--wat",
            "--page",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_default_name_is_the_page_and_the_theme() {
        assert_eq!(
            ShotArgs::new(Page::Settings, false).file_name(),
            "system-light.png"
        );
    }
}
