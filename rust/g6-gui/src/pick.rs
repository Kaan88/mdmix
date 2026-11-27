//! The Lighting page's colour picker: the toolkit's [`colour_picker`] in a [`flyout`] over the page, opened from the
//! Colour row's swatch.
//!
//! The window owns the events and the drawing, so this is only the picker and the few things the window keeps with it:
//! where its card was placed last, the colour the row showed when it opened (for Escape and a press outside), the
//! last colour sent to the G6 during this open, and how the keyboard was used.
//!
//! A drag sends like a dragged slider (`Desk`, [`crate::desk::THROTTLE`]): the window passes every colour the marker
//! lands on in, the desk sends at most one write per interval and the last one on release. What a press, a drag, a
//! release and a key ask to be written is returned, as the `Setting::Colour` value the desk takes (0xRRGGBB).
//! Nothing here writes, and nothing here reads a clock.

use mdshared_ui::colour_picker::{self, Cache, Layout, Look, Part, Picker};
use mdshared_ui::flyout::{self, Flyout};
use mdshared_ui::gfx::{Painter, Rect, Rgb};
use mdshared_ui::keys::{KeyName, Mods};
use mdshared_ui::theme::Palette;

use crate::intent::{Val, rgb_val};

/// The colour the picker opens on when nothing has been set from here in this session: a mid grey, as the toolkit's
/// own colour rows open. The G6 can't be asked which colour it shows.
pub const FALLBACK: Rgb = [128, 128, 128];

/// The window's colour picker, open.
pub struct ColourPick {
    picker: Picker,
    flyout: Flyout,
    layout: Layout,
    cache: Cache,
    /// The colour the row showed when the picker opened: what a cancel writes back.
    pub original: Option<Val>,
    /// The last colour sent to the G6 during this open; `None` when nothing was written.
    pub sent: Option<Val>,
    /// A key was used since it opened: the part with the focus is ringed.
    ring: bool,
    /// A press in the hex field is selecting its text.
    text_drag: bool,
}

/// What a press on the open picker did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Not on the card (or on the row that opened it): the picker closes as a cancel.
    Away,
    /// On the card, and the colour changed: send it.
    Colour(Val),
    /// On the card: a marker moved within its colour, a part took the focus, the caret moved.
    Taken,
}

impl ColourPick {
    /// A picker open on `rgb`, anchored to `anchor`; `original` is what the row showed.
    pub fn open(anchor: Rect, rgb: Rgb, original: Option<Val>) -> ColourPick {
        ColourPick {
            picker: Picker::new(rgb, colour_picker::HexForm::DEFAULT),
            flyout: colour_picker::flyout(anchor),
            layout: Layout::default(),
            cache: Cache::new(),
            original,
            sent: None,
            ring: false,
            text_drag: false,
        }
    }

    /// Places the card in `area` (the window, under its title row) and lays the picker out in it. The window does
    /// this every frame, so a scroll or a resize under an open picker moves it along and keeps it inside.
    pub fn place(&mut self, area: Rect) -> Rect {
        let rect = self.flyout.place_in(area);
        self.layout = Layout::new(self.flyout.content_rect());
        rect
    }

    /// Anchors the card to a new rectangle (the row moved).
    pub fn set_anchor(&mut self, anchor: Rect) {
        self.flyout.set_anchor(anchor);
    }

    /// The parts of the picker as last placed: for a test that presses a marker's place.
    #[cfg(test)]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The colour picked now.
    pub fn colour(&self) -> Val {
        rgb_val(self.picker.rgb())
    }

    /// How the picker is drawn: a ring on the part with the keyboard once a key was used, the caret on while the
    /// hex field has the focus (there is no blink timer: the window sleeps while the picker is open).
    pub fn look(&self) -> Look {
        Look {
            ring: self.ring,
            caret_on: self.picker.focus() == Part::Hex,
        }
    }

    /// Draws the card and the picker in it, `t` of the way in as it opens (1 at rest). The card fades and rises from
    /// the row as a menu does; under reduced motion, and in a shot, `t` is 1 and this is the picture at rest.
    pub fn draw(&mut self, p: &mut Painter, pal: &Palette, t: f32) {
        let fly = self.flyout;
        let layout = self.layout;
        let look = self.look();
        let (picker, cache) = (&mut self.picker, &mut self.cache);
        flyout::draw_card_animated(p, pal, &fly, t, |p, _| {
            colour_picker::draw_picker(p, pal, fly.rect(), &layout, picker, cache, look);
        });
    }

    /// A press at `x`, `y`. On the card it is the picker's (a part takes the pointer and the keyboard, a marker jumps
    /// under it, the hex field takes the caret); anywhere else it is [`Press::Away`] and the window closes the picker
    /// as a cancel.
    pub fn press(&mut self, x: f32, y: f32, shift: bool, clicks: u8) -> Press {
        self.ring = false;
        self.text_drag = false;
        if self.flyout.pointer_down(x, y) != flyout::Outcome::Inside {
            return Press::Away;
        }
        let outcome = self.picker.press(&self.layout, x, y);
        let hex = self.layout.hex;
        if self.picker.focus() == Part::Hex && hex.contains(x, y) {
            let rel = x - (hex.x + 10.0);
            let stops = self.cache.hex_stops().to_vec();
            let field = self.picker.field_mut();
            // A double click takes the word under the pointer, a triple click (or more) the whole field.
            if clicks >= 3 {
                field.select_all();
            } else if clicks == 2 {
                field.select_word_at(rel, &stops);
            } else {
                field.press(rel, &stops, shift);
                self.text_drag = true;
            }
        }
        match outcome {
            colour_picker::Outcome::Colour => Press::Colour(self.colour()),
            _ => Press::Taken,
        }
    }

    /// The pointer moved with the picker open: a marker that is held follows it. Whether the picture changed, and
    /// the colour to write when it changed.
    pub fn drag(&mut self, x: f32, y: f32) -> (bool, Option<Val>) {
        if self.text_drag {
            let rel = x - (self.layout.hex.x + 10.0);
            let stops = self.cache.hex_stops().to_vec();
            let field = self.picker.field_mut();
            let before = field.caret();
            field.drag(rel, &stops);
            return (field.caret() != before, None);
        }
        match self.picker.drag(&self.layout, x, y) {
            colour_picker::Outcome::None => (false, None),
            colour_picker::Outcome::Colour => (true, Some(self.colour())),
            // A marker moved within the same colour (through black, say): the picture changed, the colour didn't.
            _ => (true, None),
        }
    }

    /// The button went up: the colour to send as the drag's last one, when a part of the picker was held.
    pub fn release(&mut self) -> Option<Val> {
        self.text_drag = false;
        self.picker.release().then(|| self.colour())
    }

    /// A key: the toolkit's answer and, when it changed the colour, the colour to write. Enter accepts and Escape
    /// cancels ([`colour_picker::Outcome`]'s `Accept` and `Cancel`), which the window acts on.
    pub fn key(
        &mut self,
        key: &KeyName,
        mods: Mods,
        typed: Option<&str>,
    ) -> (colour_picker::Outcome, Option<Val>) {
        self.ring = true;
        let outcome = self.picker.key(key, mods, typed);
        let changed = match outcome {
            colour_picker::Outcome::Colour => Some(self.colour()),
            _ => None,
        };
        (outcome, changed)
    }
}

/// The colour a [`crate::intent::Val`] of the colour setting stands for, or the fallback.
pub fn rgb_or_fallback(v: Option<Val>) -> Rgb {
    v.and_then(crate::intent::rgb_of).unwrap_or(FALLBACK)
}

/// What the window writes when the picker closes, if anything: on an accept the colour now, when the last one did not
/// go already; on a cancel the colour from before the open, when something was written during this open (the G6
/// can't be asked, so with nothing known before there is nothing to put back).
pub fn on_close(sent: Option<Val>, now: Val, original: Option<Val>, accept: bool) -> Option<Val> {
    if accept {
        (sent != Some(now)).then_some(now)
    } else if sent.is_some() && sent != original {
        original
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    use mdshared_ui::theme::LIGHT;

    /// A press in the hex field places the caret, a double click takes the word under it and a triple click the
    /// whole field, as in any text box.
    #[test]
    fn a_triple_click_in_the_hex_field_selects_all_of_it() {
        let mut pk = ColourPick::open(Rect::new(40.0, 80.0, 120.0, 28.0), [0x12, 0x34, 0x56], None);
        pk.place(Rect::new(0.0, 36.0, 900.0, 700.0));
        // A draw measures the field's caret stops.
        let mut bmp = Bitmap::new(900, 740);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        pk.draw(&mut p, &LIGHT, 1.0);
        let hex = pk.layout().hex;
        let (x, y) = (hex.x + 40.0, hex.cy());
        assert_eq!(pk.picker.field().text(), "#123456");

        assert_eq!(pk.press(x, y, false, 1), Press::Taken);
        assert_eq!(pk.picker.focus(), Part::Hex);
        assert_eq!(pk.picker.field().selected_text(), "");
        pk.release();
        pk.press(x, y, false, 2);
        assert_eq!(pk.picker.field().selected_text(), "123456");
        pk.release();
        pk.press(x, y, false, 3);
        assert_eq!(pk.picker.field().selected_text(), "#123456");
        pk.release();
        // A fourth press in the same run is still the whole field.
        pk.press(x, y, false, 4);
        assert_eq!(pk.picker.field().selected_text(), "#123456");
    }
}
