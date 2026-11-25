//! The Settings dialog: the toolkit's paged form with Appearance and Device.

use mdshared_app::backdrop::Backdrop;
use mdshared_ui::corners::{self, Corners};
use mdshared_ui::settings::Row;
use mdshared_ui::settings::pages::{Form, Item, Page};

use crate::facts::DASH;
use crate::model::{Conn, Model};
use crate::prefs::{Prefs, Theme};

pub const THEME: u32 = 1;
pub const BACKDROP: u32 = 2;
pub const CORNERS: u32 = 3;
pub const FIRMWARE: u32 = 10;
pub const CONNECTION: u32 = 11;
pub const TRANSPORT: u32 = 12;
pub const READ: u32 = 13;

pub fn pages(prefs: &Prefs, model: &Model) -> Vec<Page> {
    let appearance = Page::new(
        "Appearance",
        vec![
            Item::new(Row::choice(
                THEME,
                "Theme",
                &Theme::LABELS,
                prefs.theme.index(),
            )),
            Item::new(
                Row::choice(
                    BACKDROP,
                    Backdrop::TITLE,
                    &Backdrop::LABELS,
                    prefs.backdrop.index(),
                )
                .help(Backdrop::HELP),
            ),
            Item::new(
                Row::choice(
                    CORNERS,
                    Corners::SETTING_LABEL,
                    &Corners::LABELS,
                    prefs.corners.index(),
                )
                .help(corners::setting_help()),
            ),
        ],
    );
    let connection = match &model.conn {
        Conn::Connected => "Connected".to_string(),
        Conn::Reading => "Reading…".to_string(),
        Conn::Cached => "Reading… (the last values are shown)".to_string(),
        Conn::Away(why) => format!("Not connected: {why}"),
    };
    let device = Page::new(
        "Device",
        vec![
            Item::new(Row::text(
                FIRMWARE,
                "Firmware",
                model.status.firmware.clone().unwrap_or_else(|| DASH.into()),
            ))
            .disabled("Read from the G6."),
            Item::new(Row::text(CONNECTION, "Connection", connection))
                .disabled("Looked up when the window opens and on Read again."),
            Item::new(Row::text(TRANSPORT, "Transport", "HID, interface 4"))
                .disabled("USB Audio Control comes with the volume reads."),
            Item::gap(),
            Item::new(Row::action(READ, "Status", "Read again")),
        ],
    );
    vec![appearance, device]
}

pub fn form(prefs: &Prefs, model: &Model) -> Form {
    Form::new("Settings", pages(prefs, model))
}

/// Opens at `page` (0 Appearance, 1 Device).
pub fn form_at(prefs: &Prefs, model: &Model, page: usize) -> Form {
    let mut f = form(prefs, model);
    f.show_page(page);
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::Sample;
    use mdshared_text::Bitmap;
    use mdshared_ui::anim::{Animator, Policy};
    use mdshared_ui::gfx::{Gfx, Painter, Rect};
    use mdshared_ui::theme::LIGHT;
    use std::time::Instant;

    #[test]
    fn both_pages_fit_the_sizes_the_window_allows() {
        for sample in [Sample::Live, Sample::Away] {
            let f = form(&Prefs::default(), &sample.model());
            let mut bmp = Bitmap::new(8, 8);
            let mut gfx = Gfx::new();
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            for size in [(760.0, 600.0), (1100.0, 720.0)] {
                let fit = f.fit(&mut p, Rect::new(0.0, 0.0, size.0, size.1));
                assert!(fit.problems().is_empty(), "{size:?}: {:?}", fit.problems());
            }
        }
    }

    #[test]
    fn a_choice_comes_back_with_its_id() {
        let f = form(&Prefs::default(), &Sample::Live.model());
        assert_eq!(f.page_names(), ["Appearance", "Device"]);
        assert_eq!(f.page_of(READ), Some(1));
        assert_eq!(f.page_of(THEME), Some(0));
    }

    /// The dialog in motion, drawn where nothing moves (a shot, a test), is the plain draw pixel for pixel: its
    /// switches and segments stand where their values are, and no hover fill is left half in.
    #[test]
    fn the_animated_dialog_at_rest_is_the_plain_draw() {
        let prefs = Prefs::default();
        let model = Sample::Busy.model();
        let size = (900.0, 700.0);
        let window = Rect::new(0.0, 0.0, size.0, size.1);
        let pixels = |animated: bool| {
            let mut bmp = Bitmap::new(size.0 as u32, size.1 as u32);
            let mut gfx = Gfx::new();
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            let mut f = form(&prefs, &model);
            if animated {
                let mut anim = Animator::new();
                anim.set_policy(Policy::Never);
                anim.begin_frame(Instant::now());
                f.draw_animated(&mut p, &LIGHT, window, false, &mut anim, "settings");
                assert!(anim.end_frame().is_none(), "at rest asks for no frame");
            } else {
                f.draw(&mut p, &LIGHT, window, false);
            }
            bmp.pixels
        };
        assert_eq!(pixels(false), pixels(true));
    }
}
