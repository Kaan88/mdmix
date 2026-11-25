//! The app's own settings (theme, backdrop, corners, the page, the window's size and place), kept in the toolkit's settings
//! file layer. Nothing here is a device setting.

use std::path::PathBuf;

use mdshared_app::backdrop::Backdrop;
use mdshared_app::settings;
use mdshared_ui::corners::Corners;
use mdshared_ui::theme::{DARK, LIGHT, Palette};

use crate::model::Page;

pub const APP: &str = "g6-gui";
const SECTION: &str = "g6-gui";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const LABELS: [&'static str; 3] = ["System", "Light", "Dark"];

    pub fn index(self) -> usize {
        match self {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        }
    }

    pub fn from_index(i: usize) -> Theme {
        match i {
            1 => Theme::Light,
            2 => Theme::Dark,
            _ => Theme::System,
        }
    }

    pub fn from_name(s: &str) -> Option<Theme> {
        match s.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// The palette; `system_dark` is what the system says, used for `System`.
    pub fn palette(self, system_dark: bool) -> Palette {
        match self {
            Theme::Light => LIGHT,
            Theme::Dark => DARK,
            Theme::System if system_dark => DARK,
            Theme::System => LIGHT,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub theme: Theme,
    pub backdrop: Backdrop,
    pub corners: Corners,
    pub page: Page,
    /// The window's inner size in logical px, when it was saved.
    pub size: Option<(u32, u32)>,
    /// The window's top-left corner on the desktop in physical px, when it was saved.
    pub pos: Option<(i32, i32)>,
    /// The window was maximized when it closed.
    pub maximized: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: Theme::System,
            // The mockup's default. Where the system shows no backdrop the toolkit falls back to the flat chrome.
            backdrop: Backdrop::Mica,
            corners: Corners::Round,
            page: Page::Sbx,
            size: None,
            pos: None,
            maximized: false,
        }
    }
}

impl Prefs {
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        let (mut w, mut h) = (None, None);
        let (mut x, mut y) = (None, None);
        for (k, v) in settings::pairs(text) {
            let v = v.trim();
            match k {
                "theme" => p.theme = Theme::from_name(v).unwrap_or(p.theme),
                "backdrop" => p.backdrop = Backdrop::parse(v).unwrap_or(p.backdrop),
                "corners" => {
                    p.corners = v.parse().map_or(p.corners, Corners::from_setting);
                }
                "page" => p.page = Page::from_key(v).unwrap_or(p.page),
                "width" => w = v.parse().ok(),
                "height" => h = v.parse().ok(),
                "x" => x = v.parse().ok(),
                "y" => y = v.parse().ok(),
                "maximized" => p.maximized = v == "true",
                _ => {}
            }
        }
        if let (Some(x), Some(y)) = (x, y) {
            p.pos =
                Some((x, y)).filter(|&(x, y): &(i32, i32)| x.abs() <= 65536 && y.abs() <= 65536);
        }
        if let (Some(w), Some(h)) = (w, h) {
            p.size = Some((w, h))
                .filter(|&(w, h)| (MIN.0..=16384).contains(&w) && (MIN.1..=16384).contains(&h));
        }
        p
    }

    pub fn to_text(&self) -> String {
        let mut w = settings::Writer::new(SECTION);
        w.line("theme", self.theme.key());
        w.line("backdrop", self.backdrop.key());
        w.line("corners", &self.corners.to_setting().to_string());
        w.line("page", self.page.key());
        if let Some((width, height)) = self.size {
            w.line("width", &width.to_string());
            w.line("height", &height.to_string());
        }
        if let Some((x, y)) = self.pos {
            w.line("x", &x.to_string());
            w.line("y", &y.to_string());
        }
        if self.maximized {
            w.line("maximized", "true");
        }
        w.finish()
    }

    /// Where the window opens: the saved place, if a monitor still shows enough of its caption to grab it.
    /// `monitors` are (x, y, width, height) in physical px.
    pub fn place_on(&self, monitors: &[(i32, i32, u32, u32)]) -> Option<(i32, i32)> {
        self.pos.filter(|&p| on_screen(p, monitors))
    }

    pub fn path() -> Option<PathBuf> {
        settings::default_settings_path(APP)
    }

    /// The saved settings; the defaults when there are none or the file can't be read.
    pub fn load() -> Prefs {
        Prefs::path()
            .and_then(|p| settings::read(&p).ok().flatten())
            .map(|t| Prefs::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = Prefs::path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = settings::write_atomic(&path, &self.to_text()) {
                eprintln!("g6-gui: settings not saved: {e}");
            }
        }
    }
}

/// The smallest window, in logical px: the sidebar and a page narrow enough to be read.
pub const MIN: (u32, u32) = (760, 600);

/// Whether a window whose top-left is at `pos` has a grabbable piece of its caption (100 by 24 px from its corner,
/// after the left 40) on one of `monitors`.
fn on_screen(pos: (i32, i32), monitors: &[(i32, i32, u32, u32)]) -> bool {
    let (x, y) = (i64::from(pos.0) + 40, i64::from(pos.1));
    monitors.iter().any(|&(mx, my, mw, mh)| {
        let (mx, my) = (i64::from(mx), i64::from(my));
        x >= mx && x + 100 <= mx + i64::from(mw) && y >= my && y + 24 <= my + i64::from(mh)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_backdrop_off_beats_the_default() {
        assert_eq!(Prefs::default().backdrop, Backdrop::Mica);
        assert_eq!(
            Prefs::parse(
                "[g6-gui]
backdrop=off
"
            )
            .backdrop,
            Backdrop::Off
        );
    }

    #[test]
    fn settings_round_trip() {
        let p = Prefs {
            theme: Theme::Dark,
            backdrop: Backdrop::Acrylic,
            corners: Corners::Small,
            page: Page::Mixer,
            size: Some((1000, 700)),
            pos: Some((-1200, 40)),
            maximized: true,
        };
        assert_eq!(Prefs::parse(&p.to_text()), p);
    }

    #[test]
    fn nonsense_falls_back_to_the_defaults() {
        let p = Prefs::parse(
            "[g6-gui]\ntheme=blue\nbackdrop=glass\npage=zzz\nwidth=3\nheight=3\ncorners=x\n",
        );
        assert_eq!(p, Prefs::default());
        assert_eq!(Prefs::parse(""), Prefs::default());
    }

    #[test]
    fn a_place_is_kept_only_where_a_monitor_shows_its_caption() {
        let mons = [(0, 0, 1920, 1080), (-1280, 100, 1280, 1024)];
        assert!(on_screen((100, 100), &mons));
        assert!(on_screen((-1200, 200), &mons));
        // A monitor that was unplugged.
        assert!(!on_screen((2500, 100), &mons));
        // Only the very edge of the caption showing is not enough to grab it.
        assert!(!on_screen((1900, 100), &mons));
        assert!(!on_screen((100, -100), &mons));
    }

    #[test]
    fn system_theme_follows_the_system() {
        assert!(Theme::System.palette(true).dark);
        assert!(!Theme::System.palette(false).dark);
        assert!(Theme::Dark.palette(false).dark);
        assert!(!Theme::Light.palette(true).dark);
    }
}
