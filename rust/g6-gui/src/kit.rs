//! The drawing kit of the pages: controls in the toolkit's look, cards of rows, and the grid the cards flow in.
//!
//! A device control is bound ([`Bind`]) to the [`Setting`] it writes, and its regions then carry the value a click
//! asks for ([`Hit::Set`]), the slider to take hold of ([`Hit::Slide`]) or the dropdown to open ([`Hit::Open`]). One
//! the window doesn't write is inert, and its region says why ([`Hit::Inert`] and [`Why`]), for the pointer's tip.

use std::hash::Hash;

use mdshared_ui::anim::{Animator, Id, fluent};
use mdshared_ui::chrome::button_fill;
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb, mix};
use mdshared_ui::glyph;
use mdshared_ui::theme::Palette;

use crate::intent::{Setting, Val};
use crate::model::{Model, Page};

/// What a click on a region does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hit {
    Nav(Page),
    Settings,
    Retry,
    Refresh,
    Min,
    Max,
    Close,
    Theme(usize),
    Backdrop(usize),
    Corners(usize),
    /// The empty part of the title row: drags the window.
    Drag,
    /// A click writes this value of this setting to the G6.
    Set(Setting, Val),
    /// A slider of this setting: pressed, it is taken hold of and dragged.
    Slide(Setting),
    /// A dropdown of this setting: a click opens its choices.
    Open(Setting),
    /// A device control the window doesn't change: the pointer says so and a tip says why.
    Inert(Why),
}

/// Why a control doesn't write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Why {
    /// Windows' volume or mute (Core Audio): read, never changed.
    Windows,
    /// The speaker setup: how SoundBlaster Command writes it isn't known (step G9).
    SpeakerSetup,
    /// The lighting effects run from this computer and need their thread (step G7).
    Effects,
    /// The command line has no write for this choice.
    NoWrite,
    /// The G6 is away.
    Away,
    /// The G6 hasn't been read yet.
    NotRead,
    /// The light can't be turned on without a colour, and the G6 can't be asked for its colour.
    NoColour,
    /// The preset's curve depends on the output, which isn't known.
    NoOutput,
}

impl Why {
    /// The tip over the control.
    pub fn text(self) -> &'static str {
        match self {
            Why::Windows => {
                "Read only: this is Windows' setting (Core Audio). Change it in Windows' sound settings."
            }
            Why::SpeakerSetup => {
                "Read only: how SoundBlaster Command writes the speaker setup isn't known yet."
            }
            Why::Effects => {
                "Not built yet: effects run from this computer. The command line has them (g6 --lighting-effect)."
            }
            Why::NoWrite => "Read only: the command line has no write for this choice.",
            Why::Away => "The G6 is not connected.",
            Why::NotRead => "Not read from the G6 yet.",
            Why::NoColour => {
                "The G6 can't be asked which colour it had: pick a colour below to turn the light on."
            }
            Why::NoOutput => "The output isn't known, so the preset's curve can't be chosen.",
        }
    }
}

/// What a control is bound to: the setting it writes, or why it writes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    Live(Setting),
    Off(Why),
}

/// The regions the last frame drew, in drawing order; the one drawn last is on top.
#[derive(Clone, Debug, Default)]
pub struct Hits(Vec<(Hit, Rect)>);

impl Hit {
    /// Whether a click on the region does anything: the title row drags the window and a read-only control is inert.
    pub fn acts(self) -> bool {
        !matches!(self, Hit::Drag | Hit::Inert(_))
    }
}

impl Hits {
    pub fn add(&mut self, hit: Hit, r: Rect) {
        self.0.push((hit, r));
    }

    /// How many regions there are; a mark for `clip_from`.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The regions added since `start` are only where `view` is: each is cut to it, and one entirely outside it is
    /// dropped. A page that scrolls draws its controls where they would be; the part scrolled out of sight is not
    /// there to be hit.
    pub fn clip_from(&mut self, start: usize, view: Rect) {
        let tail = self.0.split_off(start.min(self.0.len()));
        for (hit, r) in tail {
            let (x0, y0) = (r.x.max(view.x), r.y.max(view.y));
            let (x1, y1) = (r.right().min(view.right()), r.bottom().min(view.bottom()));
            if x1 > x0 && y1 > y0 {
                self.0.push((hit, Rect::new(x0, y0, x1 - x0, y1 - y0)));
            }
        }
    }

    /// The region under the point and what it does.
    pub fn at_rect(&self, x: f32, y: f32) -> Option<(Hit, Rect)> {
        self.0.iter().rev().find(|(_, r)| r.contains(x, y)).copied()
    }

    pub fn at(&self, x: f32, y: f32) -> Option<Hit> {
        self.0
            .iter()
            .rev()
            .find(|(_, r)| r.contains(x, y))
            .map(|(h, _)| *h)
    }

    /// Every region, in drawing order.
    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = (Hit, Rect)> + '_ {
        self.0.iter().copied()
    }

    pub fn rect_of(&self, hit: Hit) -> Option<Rect> {
        self.0
            .iter()
            .rev()
            .find(|(h, _)| *h == hit)
            .map(|(_, r)| *r)
    }
}

/// Everything a page needs to draw: the colours, the device state, the pointer, and the animator that moves what
/// changes.
pub struct Env<'a> {
    pub pal: Palette,
    pub model: &'a Model,
    pub hits: Hits,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
    /// The window's animated values. Every widget that moves asks it for its current value while it draws.
    pub anim: &'a mut Animator,
    /// The slider in hand: it follows the pointer without gliding.
    pub dragging: Option<Setting>,
    /// What the page's widgets are named under, so a switch of page never moves one page's knob into the next's.
    scope: Id,
    /// How many widgets of the page asked for a name so far this frame: the order they are drawn in is their name.
    seq: u32,
}

impl<'a> Env<'a> {
    pub fn new(
        pal: Palette,
        model: &'a Model,
        hover: Option<Hit>,
        pressed: Option<Hit>,
        anim: &'a mut Animator,
    ) -> Env<'a> {
        Env {
            pal,
            model,
            hits: Hits::default(),
            hover,
            pressed,
            anim,
            dragging: None,
            scope: Id::new("env"),
            seq: 0,
        }
    }

    /// Starts a page: its widgets are named from 1 again, under the page.
    pub fn begin_scope(&mut self, scope: impl Hash) {
        self.scope = Id::new(scope);
        self.seq = 0;
    }

    /// The name of the next widget of the page. A page draws its widgets in the same order every frame, so the
    /// order is a stable name; a layout that changes (a row that appears) only makes the ones after it start fresh,
    /// at rest, which is never wrong.
    fn next_id(&mut self) -> Id {
        self.seq += 1;
        self.scope.with(self.seq)
    }

    /// Whether a bound control may write now: the G6 must be there and read.
    pub fn gate(&self, bind: Bind) -> Result<Setting, Why> {
        match bind {
            Bind::Off(why) => Err(why),
            Bind::Live(_) if matches!(self.model.conn, crate::model::Conn::Away(_)) => {
                Err(Why::Away)
            }
            Bind::Live(_) if !self.model.have_status => Err(Why::NotRead),
            Bind::Live(s) => Ok(s),
        }
    }

    /// The region of a switch: a click asks for the other state.
    pub fn toggle_hit(&self, on: Option<bool>, bind: Bind) -> Hit {
        match (self.gate(bind), on) {
            (Err(why), _) => Hit::Inert(why),
            (Ok(_), None) => Hit::Inert(Why::NotRead),
            (Ok(s), Some(v)) => Hit::Set(s, Val::On(!v)),
        }
    }

    pub fn is_hover(&self, hit: Hit) -> bool {
        self.hover == Some(hit)
    }

    pub fn is_pressed(&self, hit: Hit) -> bool {
        self.pressed == Some(hit)
    }

    /// A hover fade: 0 to 1 over 83 ms while `on`, back while not. A name first seen is where it is going.
    pub fn fade(&mut self, name: impl Hash, on: bool) -> f32 {
        self.anim.toggle(name, on, fluent::FADE)
    }

    /// The switch of a row: the knob slides and the fill changes colour when the value changes. A value first
    /// seen (the first read of a page) is drawn at rest.
    pub fn toggle(&mut self, p: &mut Painter, pal: &Palette, r: Rect, on: Option<bool>) {
        let id = self.next_id();
        match on {
            Some(on) => {
                let t = self.anim.toggle(id, on, fluent::MOVE);
                toggle_at(p, pal, r, t);
            }
            None => toggle(p, pal, r, None),
        }
    }

    /// A slider's fraction, gliding to a new reading. `None` (not read) stays `None`. The slider in hand (`held`)
    /// is where the pointer put it, at once.
    pub fn glide(&mut self, frac: Option<f32>, held: bool) -> Option<f32> {
        let id = self.next_id();
        frac.map(|f| {
            if held {
                self.anim.set(id, f);
                f
            } else {
                self.anim.value(id, f, fluent::MOVE)
            }
        })
    }

    /// Whether this bound slider is the one in hand.
    pub fn held(&self, bind: Bind) -> bool {
        matches!(bind, Bind::Live(s) if self.dragging == Some(s))
    }

    /// A segmented choice with motion: the chosen fill slides, the hover fades.
    #[allow(clippy::too_many_arguments)]
    pub fn seg(
        &mut self,
        p: &mut Painter,
        pal: &Palette,
        r: Rect,
        labels: &[&str],
        sel: Option<usize>,
        hovered: Option<usize>,
        kind: SegKind,
    ) -> Vec<Rect> {
        let id = self.next_id();
        draw_seg_with(
            p,
            pal,
            Some((&mut *self.anim, id)),
            r,
            labels,
            sel,
            hovered,
            kind,
        )
    }

    /// A push button whose fill fades between rest, hover and pressed.
    pub fn button(&mut self, p: &mut Painter, pal: &Palette, r: Rect, label: &str, hit: Hit) {
        let target = button_fill_of(pal, self.is_hover(hit), self.is_pressed(hit));
        let fill = self.anim.value(("button", hit), target, fluent::FADE);
        button(p, pal, r, label, fill);
    }
}

/// The palette of a control that is out of use (its effect is bypassed or its switch is off): text, accent and edges
/// drawn part of the way to the ground they sit on, as the mockup's `.off` (opacity .45).
pub fn dimmed(pal: &Palette, ground: Rgb) -> Palette {
    let f = |c: Rgb, t: f32| mix(c, ground, t);
    Palette {
        text: f(pal.text, 0.50),
        muted: f(pal.muted, 0.50),
        accent: f(pal.accent, 0.55),
        on_accent: f(pal.on_accent, 0.30),
        field: f(pal.field, 0.30),
        line: f(pal.line, 0.25),
        popup_line: f(pal.popup_line, 0.25),
        hover: pal.field,
        ..*pal
    }
}

/// The chrome's palette. Over a backdrop the chrome's ground is no longer one known colour, so its secondary text is
/// written in the full text colour (the mockup does the same under `data-frost`).
pub fn chrome_pal(pal: &Palette, frosted: bool) -> Palette {
    if frosted {
        Palette {
            muted: pal.text,
            ..*pal
        }
    } else {
        *pal
    }
}

// ---- the mockup's type scale ------------------------------------------------------------------

pub const SIZE_H2: f32 = 17.0;
pub const SIZE_CARD: f32 = 13.5;
pub const SIZE_LABEL: f32 = 13.5;
pub const SIZE_SMALL: f32 = 11.5;
pub const SIZE_VALUE: f32 = 12.5;
pub const SIZE_BODY: f32 = 13.0;

// ---- controls ---------------------------------------------------------------------------------

pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 20.0;
pub const SEG_H: f32 = 28.0;
pub const SEG_H_SM: f32 = 24.0;
pub const CTL_H: f32 = 28.0;

/// The switch of the settings dialog: a pill with the knob at the end that says on. `None` is a state not read: a dash.
pub fn toggle(p: &mut Painter, pal: &Palette, r: Rect, on: Option<bool>) {
    match on {
        Some(on) => toggle_at(p, pal, r, if on { 1.0 } else { 0.0 }),
        None => p.text_center(Face::Ui, 13.0, r, "—", pal.muted),
    }
}

/// The switch `t` of the way from off (0) to on (1): the knob has moved and the fill has changed colour by as much.
/// At 0 and 1 it is exactly the resting look.
pub fn toggle_at(p: &mut Painter, pal: &Palette, r: Rect, t: f32) {
    let knob = 12.0;
    let t = t.clamp(0.0, 1.0);
    let off_edge = mix(pal.muted, pal.popup, 0.35);
    if t >= 1.0 {
        p.rbox(r, r.h / 2.0, pal.accent);
        p.rbox(
            Rect::new(r.right() - 4.0 - knob, r.cy() - knob / 2.0, knob, knob),
            knob / 2.0,
            pal.on_accent,
        );
        return;
    }
    if t <= 0.0 {
        p.frame(r, r.h / 2.0, pal.field, off_edge);
        p.rbox(
            Rect::new(r.x + 4.0, r.cy() - knob / 2.0, knob, knob),
            knob / 2.0,
            pal.muted,
        );
        return;
    }
    p.frame(
        r,
        r.h / 2.0,
        mix(pal.field, pal.accent, t),
        mix(off_edge, pal.accent, t),
    );
    let (x0, x1) = (r.x + 4.0, r.right() - 4.0 - knob);
    p.rbox(
        Rect::new(x0 + (x1 - x0) * t, r.cy() - knob / 2.0, knob, knob),
        knob / 2.0,
        mix(pal.muted, pal.on_accent, t),
    );
}

/// The mockup's segmented choice: a field with a line edge and 1 px of room inside, a button per option (13 px of
/// room each side, 9 for the small kind), the chosen one filled with the accent in the semibold face.
fn seg_metrics(small: bool) -> (f32, f32, f32) {
    // (text size, side room, height)
    if small {
        (12.0, 9.0, SEG_H_SM)
    } else {
        (SIZE_BODY, 13.0, SEG_H)
    }
}

/// The kind of a segmented choice: the small one is 24 high; a filled one has equal buttons.
#[derive(Clone, Copy, Debug)]
pub struct SegKind {
    pub small: bool,
    pub fill: bool,
}

/// The width a segmented choice wants: every label measured in the semibold face plus its side room, and the edge.
pub fn seg_width(p: &mut Painter, labels: &[&str], small: bool) -> f32 {
    let (size, pad, _) = seg_metrics(small);
    4.0 + labels
        .iter()
        .map(|l| p.measure(Face::Semibold, size, l) + 2.0 * pad)
        .sum::<f32>()
}

/// [`draw_seg_with`] with no motion: the control at rest. The tests' reference for what motion must come to.
#[cfg(test)]
pub fn draw_seg(
    p: &mut Painter,
    pal: &Palette,
    r: Rect,
    labels: &[&str],
    sel: Option<usize>,
    hovered: Option<usize>,
    kind: SegKind,
) -> Vec<Rect> {
    draw_seg_with(p, pal, None, r, labels, sel, hovered, kind)
}

/// Draws a segmented choice into `r` and returns the rectangle of each button. Buttons are as wide as their text and
/// room; extra width goes to all of them alike (`fill`: they are equal, as the mockup's `.seg.fill`). With motion when
/// it is given an animator and a name: the accent fill slides from the button that was
/// chosen to the one that is (167 ms), the hover fill fades (83 ms), and a label takes its colour by how much of its
/// button the fill covers. The fill is kept relative to the control, so a control that moves with its window does not
/// set it sliding. At rest, and without an animator, the pixels are those of [`draw_seg`].
///
/// A choice that appears when the first read arrives (it was `None`) is shown at once, not faded in.
#[allow(clippy::too_many_arguments)]
pub fn draw_seg_with(
    p: &mut Painter,
    pal: &Palette,
    mut anim: Option<(&mut Animator, Id)>,
    r: Rect,
    labels: &[&str],
    sel: Option<usize>,
    hovered: Option<usize>,
    kind: SegKind,
) -> Vec<Rect> {
    let SegKind { small, fill } = kind;
    if labels.is_empty() {
        return Vec::new();
    }
    let (size, pad, _) = seg_metrics(small);
    p.frame(r, 6.0, pal.field, pal.line);
    let inner = r.inset(2.0);
    let n = labels.len() as f32;
    let want: Vec<f32> = labels
        .iter()
        .map(|l| p.measure(Face::Semibold, size, l) + 2.0 * pad)
        .collect();
    let total: f32 = want.iter().sum();
    let extra = (inner.w - total) / n;
    let mut x = inner.x;
    let mut rects = Vec::new();
    for w_want in &want {
        let w = if fill {
            inner.w / n
        } else {
            (w_want + extra).max(0.0)
        };
        rects.push(Rect::new(x, inner.y, w, inner.h));
        x += w;
    }

    // What is on its way: how much each hover is there, where the fill is, and how solid it is.
    let chosen = sel.filter(|&k| k < rects.len());
    let (hover, fill_at, there) = match anim.as_mut() {
        None => (
            (0..rects.len())
                .map(|k| f32::from(u8::from(hovered == Some(k) && sel != Some(k))))
                .collect::<Vec<_>>(),
            chosen.map(|k| rects[k]),
            f32::from(u8::from(chosen.is_some())),
        ),
        Some((a, id)) => {
            let id = *id;
            let hover = (0..rects.len())
                .map(|k| {
                    a.toggle(
                        id.with(("hover", k)),
                        hovered == Some(k) && sel != Some(k),
                        fluent::FADE,
                    )
                })
                .collect::<Vec<_>>();
            let on = id.with("on");
            // A choice read for the first time was not there before: it does not fade in.
            if chosen.is_some() && a.settled(on) && a.get::<f32>(on) == Some(0.0) {
                a.set(on, 1.0);
            }
            let there = a.toggle(on, chosen.is_some(), fluent::FADE);
            let place = id.with("fill");
            let at = match chosen {
                Some(k) => {
                    let c = rects[k];
                    let rel = Rect::new(c.x - r.x, c.y - r.y, c.w, c.h);
                    Some(a.value(place, rel, fluent::MOVE))
                }
                // A choice taken away fades out where it was.
                None if there > 0.0 => {
                    a.keep(place);
                    a.get::<Rect>(place)
                }
                None => None,
            }
            .map(|rel| Rect::new(r.x + rel.x, r.y + rel.y, rel.w, rel.h));
            (hover, at, there)
        }
    };
    for (seg, &h) in rects.iter().zip(&hover) {
        if h >= 1.0 {
            p.rbox(*seg, 4.0, pal.hover);
        } else if h > 0.0 {
            p.rbox_a(*seg, 4.0, pal.hover, h);
        }
    }
    if let Some(f) = fill_at {
        if there >= 1.0 {
            p.rbox(f, 4.0, pal.accent);
        } else if there > 0.0 {
            p.rbox_a(f, 4.0, pal.accent, there);
        }
    }
    for (label, seg) in labels.iter().zip(&rects) {
        // How much of this button the fill covers: its label is the on-accent colour by as much.
        let covered = fill_at.map_or(0.0, |f| {
            let overlap = f.right().min(seg.right()) - f.x.max(seg.x);
            if seg.w > 0.0 {
                (overlap / seg.w).clamp(0.0, 1.0) * there
            } else {
                0.0
            }
        });
        let face = if covered >= 0.5 {
            Face::Semibold
        } else {
            Face::Ui
        };
        p.text_center(
            face,
            size,
            *seg,
            label,
            mix(pal.text, pal.on_accent, covered),
        );
    }
    rects
}

/// A horizontal slider: a groove, a fill from the left (or from the middle) and a knob. `frac` is 0 to 1; `None`
/// draws the groove alone.
pub fn slider(
    p: &mut Painter,
    pal: &Palette,
    r: Rect,
    frac: Option<f32>,
    from_middle: bool,
    ground: Rgb,
) {
    let (x0, x1) = (r.x + 7.0, r.right() - 7.0);
    let groove = Rect::new(x0, r.cy() - 2.0, (x1 - x0).max(0.0), 4.0);
    p.frame(groove, 2.0, pal.field, mix(pal.muted, ground, 0.55));
    let Some(f) = frac else { return };
    let f = f.clamp(0.0, 1.0);
    let kx = x0 + f * (x1 - x0);
    let from = if from_middle { (x0 + x1) / 2.0 } else { x0 };
    let (a, b) = if kx >= from { (from, kx) } else { (kx, from) };
    if b - a > 0.5 {
        p.rbox(Rect::new(a, groove.y, b - a, groove.h), 2.0, pal.accent);
    }
    p.rbox(Rect::new(kx - 9.0, r.cy() - 9.0, 18.0, 18.0), 9.0, ground);
    p.rbox(
        Rect::new(kx - 7.0, r.cy() - 7.0, 14.0, 14.0),
        7.0,
        pal.accent,
    );
}

/// The width of a slider's reading ("34", "+2.5 dB"): tabular figures, at least 34 wide.
pub fn reading_w(p: &mut Painter, reading: &str) -> f32 {
    p.measure(Face::Ui, SIZE_VALUE, reading).max(34.0)
}

/// A dropdown as it looks closed: the value and a chevron.
pub fn dropdown(p: &mut Painter, pal: &Palette, r: Rect, text: &str) {
    p.frame(r, 6.0, pal.field, pal.line);
    p.text(
        Face::Ui,
        SIZE_BODY,
        r.x + 10.0,
        r.cy(),
        text,
        pal.text,
        r.w - 36.0,
    );
    p.icon(glyph::DOWN, 10.0, r.right() - 14.0, r.cy(), pal.muted);
}

/// The width a dropdown wants for `text`: the text, 10 px before it and 26 after it for the chevron.
pub fn dropdown_w(p: &mut Painter, text: &str) -> f32 {
    (p.measure(Face::Ui, SIZE_BODY, text) + 36.0 + 2.0).ceil()
}

/// The fill of a button at rest, hovered or pressed.
pub fn button_fill_of(pal: &Palette, hovered: bool, pressed: bool) -> Rgb {
    button_fill(pal, hovered, pressed, false, false).unwrap_or(mix(pal.popup, pal.text, 0.04))
}

/// A button filled with `fill`.
pub fn button(p: &mut Painter, pal: &Palette, r: Rect, label: &str, fill: Rgb) {
    p.frame(r, 5.0, fill, pal.popup_line);
    p.text_center(Face::Ui, SIZE_BODY, r, label, pal.text);
}

pub fn button_w(p: &mut Painter, label: &str) -> f32 {
    (p.measure(Face::Ui, SIZE_BODY, label) + 28.0 + 2.0).round()
}

/// The width of a badge with `text`.
pub fn badge_w(p: &mut Painter, text: &str) -> f32 {
    (p.measure(Face::Semibold, 10.5, text) + 14.0).ceil()
}

/// A small label: "In use", "SBX", "Not connected". Lit is the accent filled; warn the warning colours.
pub fn badge(p: &mut Painter, pal: &Palette, x: f32, cy: f32, text: &str, tone: Tone) -> f32 {
    let w = badge_w(p, text);
    let r = Rect::new(x, cy - 9.0, w, 18.0);
    let (fill, edge, ink) = match tone {
        Tone::Quiet => (pal.popup, pal.popup_line, pal.muted),
        Tone::Lit => (pal.accent, pal.accent, pal.on_accent),
        Tone::Warn => (pal.warn_bg, pal.warn, pal.warn),
    };
    p.frame(r, 4.0, fill, edge);
    p.text_center(Face::Semibold, 10.5, r, text, ink);
    w
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Lit,
    Warn,
}

/// A status lamp.
pub fn led(p: &mut Painter, cx: f32, cy: f32, color: Rgb) {
    p.rbox(Rect::new(cx - 3.5, cy - 3.5, 7.0, 7.0), 3.5, color);
}

// ---- text -------------------------------------------------------------------------------------

/// `text` broken at spaces into lines no wider than `w`. A word wider than a line stands alone, and the painter
/// elides it.
pub fn wrap(p: &mut Painter, face: Face, size: f32, text: &str, w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if line.is_empty() || p.measure(face, size, &next) <= w {
            line = next;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ---- cards ------------------------------------------------------------------------------------

/// What sits at the right of a row.
#[derive(Clone, Debug)]
pub enum Ctl {
    Toggle(Option<bool>, Bind),
    /// A segmented choice. `live` makes each segment a hit (`Hit` built from its index) for the app's own settings;
    /// `bind` makes each segment write its value (`Setting::seg_val`).
    Seg {
        labels: Vec<String>,
        sel: Option<usize>,
        live: Option<fn(usize) -> Hit>,
        small: bool,
        bind: Bind,
    },
    /// A slider with its reading. `frac` is 0 to 1, `None` when the value is not known.
    Slider {
        frac: Option<f32>,
        reading: String,
        from_middle: bool,
        bind: Bind,
    },
    /// A dropdown showing `text`, wide enough for the widest of `options` as well.
    Dropdown {
        text: String,
        options: Vec<String>,
        bind: Bind,
    },
    /// Text, right aligned: a number, a version.
    Value {
        text: String,
        mono: bool,
    },
    Button {
        label: String,
        hit: Hit,
    },
}

#[derive(Clone, Debug)]
pub struct RowSpec {
    pub label: String,
    pub help: String,
    pub ctl: Ctl,
    /// A setting that matters only while the row above is on.
    pub child: bool,
    pub badge: Option<(String, Tone)>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Row(RowSpec),
    /// A small heading inside a card.
    Sub(String),
    /// A line of small text.
    Note(String),
    /// Labelled controls side by side, wrapping onto more lines when the card is narrow (the mockup's `.inserts`).
    Inline(Vec<(String, Ctl)>),
}

/// Breaks the controls of an [`Item::Inline`] into lines: each entry's width, label and control, 20 px apart.
fn inline_lines(p: &mut Painter, parts: &[(String, Ctl)], w: f32) -> Vec<Vec<(usize, f32)>> {
    let mut lines: Vec<Vec<(usize, f32)>> = vec![Vec::new()];
    let mut x = 0.0;
    for (i, (label, ctl)) in parts.iter().enumerate() {
        let (want, _) = ctl_want(p, ctl, w);
        let lw = p.measure(Face::Ui, SIZE_VALUE, label);
        let ew = lw + 8.0 + want;
        let line = lines.last_mut().expect("one line");
        if !line.is_empty() && x + ew > w {
            lines.push(Vec::new());
            x = 0.0;
        }
        lines.last_mut().expect("one line").push((i, ew));
        x += ew + 20.0;
    }
    lines
}

fn inline_height(n: usize) -> f32 {
    n as f32 * 28.0 + n.saturating_sub(1) as f32 * 8.0
}

pub fn row(label: &str, ctl: Ctl) -> RowSpec {
    RowSpec {
        label: label.into(),
        help: String::new(),
        ctl,
        child: false,
        badge: None,
    }
}

impl RowSpec {
    pub fn help(mut self, h: &str) -> RowSpec {
        self.help = h.into();
        self
    }
    pub fn child(mut self) -> RowSpec {
        self.child = true;
        self
    }
    pub fn badge(mut self, text: &str, tone: Tone) -> RowSpec {
        self.badge = Some((text.into(), tone));
        self
    }
    pub fn item(self) -> Item {
        Item::Row(self)
    }
}

fn seg_ctl(
    labels: &[&str],
    sel: Option<usize>,
    live: Option<fn(usize) -> Hit>,
    small: bool,
    bind: Bind,
) -> Ctl {
    Ctl::Seg {
        labels: labels.iter().map(|s| s.to_string()).collect(),
        sel,
        live,
        small,
        bind,
    }
}

pub fn seg(labels: &[&str], sel: Option<usize>, bind: Bind) -> Ctl {
    seg_ctl(labels, sel, None, false, bind)
}

/// The small kind (24 high): the mockup's `.seg.sm`.
pub fn seg_sm(labels: &[&str], sel: Option<usize>, bind: Bind) -> Ctl {
    seg_ctl(labels, sel, None, true, bind)
}

pub fn live_seg(labels: &[&str], sel: Option<usize>, hit: fn(usize) -> Hit) -> Ctl {
    seg_ctl(labels, sel, Some(hit), false, Bind::Off(Why::NoWrite))
}

/// A dropdown showing `text` that is wide enough for the longest of `options`.
pub fn dd(text: impl Into<String>, options: &[&str], bind: Bind) -> Ctl {
    Ctl::Dropdown {
        text: text.into(),
        options: options.iter().map(|s| s.to_string()).collect(),
        bind,
    }
}

/// The region of segment `k` of a bound choice: the value it writes, or why it writes nothing.
pub fn seg_hit(env: &Env, bind: Bind, k: usize) -> Hit {
    match env.gate(bind) {
        Err(why) => Hit::Inert(why),
        Ok(s) => match s.seg_val(k) {
            // The chosen one is there already: a click on it asks for what the G6 has, which sends nothing.
            Some(v) => Hit::Set(s, v),
            None => Hit::Inert(Why::NoWrite),
        },
    }
}

/// The width a dropdown wants: the widest of its shown text and its options, measured.
fn dropdown_want(p: &mut Painter, text: &str, options: &[String]) -> f32 {
    options
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(text))
        .map(|t| dropdown_w(p, t))
        .fold(90.0_f32, f32::max)
}

pub fn value(text: impl Into<String>) -> Ctl {
    Ctl::Value {
        text: text.into(),
        mono: false,
    }
}

type DrawFn<'a> = Box<dyn Fn(&mut Painter, &mut Env, Rect) + 'a>;

/// A piece of a card's body that is not rows: a curve, a rack of strips. `h` is its height.
pub struct Draw<'a> {
    pub h: f32,
    pub f: DrawFn<'a>,
}

pub enum Part<'a> {
    Items(Vec<Item>),
    Draw(Draw<'a>),
}

/// A mixer group: a card that is as wide as its strips (the mockup's `.grp`), with a small capital title.
pub struct Rack {
    /// The names of the strips; the card is wide enough for the widest on one line.
    pub names: Vec<String>,
    /// A wide strip (the playback master) is at least [`MASTER_STRIP_W`].
    pub master: bool,
}

pub struct Card<'a> {
    pub title: String,
    /// A switch in the header, as the mockup's cards have.
    pub head: Option<Option<bool>>,
    /// What the header's switch writes.
    pub head_bind: Bind,
    /// What is written before the header's switch ("Mute mix").
    pub head_label: Option<String>,
    pub parts: Vec<Part<'a>>,
    /// Takes the whole row of the grid.
    pub span: bool,
    /// The whole card is out of use (its effect is bypassed): drawn fainter.
    pub off: bool,
    pub rack: Option<Rack>,
}

impl<'a> Card<'a> {
    pub fn new(title: &str, items: Vec<Item>) -> Card<'a> {
        Card {
            title: title.into(),
            head: None,
            head_bind: Bind::Off(Why::NoWrite),
            head_label: None,
            parts: vec![Part::Items(items)],
            span: false,
            off: false,
            rack: None,
        }
    }
    pub fn head(mut self, on: Option<bool>, bind: Bind) -> Card<'a> {
        self.head = Some(on);
        self.head_bind = bind;
        self
    }
    pub fn head_label(mut self, label: &str) -> Card<'a> {
        self.head_label = Some(label.into());
        self
    }
    pub fn span(mut self) -> Card<'a> {
        self.span = true;
        self
    }
    pub fn off(mut self, off: bool) -> Card<'a> {
        self.off = off;
        self
    }
    pub fn rack(mut self, names: Vec<String>, master: bool) -> Card<'a> {
        self.rack = Some(Rack { names, master });
        self
    }
    pub fn then(mut self, part: Part<'a>) -> Card<'a> {
        self.parts.push(part);
        self
    }
}

pub const CARD_PAD: f32 = 16.0;
/// The card's header: 6 px of padding, the 40 px header with its line, and 4 px before the first row.
pub const CARD_HEAD: f32 = 50.0;
const CARD_HEAD_LINE: f32 = 47.0;
pub const CARD_FOOT: f32 = 8.0;
pub const GRID_GAP: f32 = 12.0;
pub const COL_MIN: f32 = 280.0;
const HELP_LINE: f32 = 14.0;
const CTL_GAP: f32 = 12.0;
const RACK_H: f32 = 340.0;
const RACK_HEAD: f32 = 34.0;
pub const STRIP_W: f32 = 60.0;
pub const MASTER_STRIP_W: f32 = 92.0;

/// The width of each strip of a rack: at least the mockup's 60 (92 for the master), and wide enough for the widest
/// name on one line.
pub fn strip_w(p: &mut Painter, names: &[String], master: bool) -> f32 {
    let widest = names
        .iter()
        .map(|n| p.measure(Face::Ui, SIZE_SMALL, n))
        .fold(0.0_f32, f32::max);
    (widest + 10.0)
        .ceil()
        .max(if master { MASTER_STRIP_W } else { STRIP_W })
}

/// What a control wants of a row: its width, and the least it can be squeezed to.
fn ctl_want(p: &mut Painter, ctl: &Ctl, inner_w: f32) -> (f32, f32) {
    match ctl {
        Ctl::Toggle(..) => (TOGGLE_W, TOGGLE_W),
        Ctl::Seg { labels, small, .. } => {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let w = seg_width(p, &refs, *small).ceil();
            (w, w)
        }
        Ctl::Slider { reading, .. } => {
            let rw = reading_w(p, reading);
            let track = (inner_w * 0.5).clamp(110.0, 240.0);
            (track + 10.0 + rw, 96.0 + 10.0 + rw)
        }
        Ctl::Dropdown { text, options, .. } => {
            let w = dropdown_want(p, text, options);
            (w, w.min(130.0))
        }
        Ctl::Value { text, mono } => {
            let face = if *mono { Face::Mono } else { Face::Ui };
            let w = p.measure(face, SIZE_VALUE, text).ceil();
            (w, w.min(60.0))
        }
        Ctl::Button { label, .. } => {
            let w = button_w(p, label);
            (w, w)
        }
    }
}

fn ctl_height(ctl: &Ctl) -> f32 {
    match ctl {
        Ctl::Toggle(..) => TOGGLE_H,
        Ctl::Seg { small: true, .. } => SEG_H_SM,
        Ctl::Seg { .. } | Ctl::Dropdown { .. } | Ctl::Button { .. } => 28.0,
        Ctl::Slider { .. } | Ctl::Value { .. } => 28.0,
    }
}

/// Where a row puts its label, help and control.
struct RowLay {
    h: f32,
    /// The control goes under the label (it does not fit beside it).
    stacked: bool,
    ctl_w: f32,
    /// Where the control begins in a stacked row, from the row's top.
    ctl_y: f32,
    lines: Vec<String>,
    label_cy: f32,
}

/// The width of a row's label and its badge, on one line.
fn label_w_of(p: &mut Painter, r: &RowSpec) -> f32 {
    p.measure(Face::Ui, SIZE_LABEL, &r.label) + badge_room(p, r)
}

fn badge_room(p: &mut Painter, r: &RowSpec) -> f32 {
    r.badge.as_ref().map_or(0.0, |(t, _)| badge_w(p, t) + 8.0)
}

/// The mockup's row: the label takes the room it needs on one line and the control the rest; a control that cannot
/// shrink to what is left, or a help text that would run past two lines beside it, goes under the label.
fn layout_row(p: &mut Painter, r: &RowSpec, inner_w: f32) -> RowLay {
    let indent = if r.child { 16.0 } else { 0.0 };
    let avail = inner_w - indent;
    let (want, min) = ctl_want(p, &r.ctl, avail);
    let help = |p: &mut Painter, w: f32| {
        if r.help.is_empty() {
            Vec::new()
        } else {
            wrap(p, Face::Ui, SIZE_SMALL, &r.help, w.max(40.0))
        }
    };
    let help_h = |n: usize| {
        if n == 0 {
            0.0
        } else {
            26.0 + HELP_LINE * n as f32
        }
    };
    if r.label.is_empty() {
        // A control with no label (an effect's level) has the whole row.
        let w = match r.ctl {
            Ctl::Slider { .. } | Ctl::Dropdown { .. } => avail,
            _ => want.min(avail),
        };
        return RowLay {
            h: 34.0,
            stacked: false,
            ctl_w: w,
            ctl_y: 0.0,
            lines: Vec::new(),
            label_cy: 17.0,
        };
    }
    let label_w = label_w_of(p, r);
    let room = avail - label_w - CTL_GAP;
    let inline = if want <= room {
        Some(want)
    } else if min <= room {
        Some(room)
    } else {
        None
    };
    if let Some(cw) = inline {
        let lines = help(p, avail - cw - CTL_GAP);
        if lines.len() <= 2 {
            let h = if lines.is_empty() {
                34.0
            } else {
                help_h(lines.len()).max(40.0)
            };
            return RowLay {
                h,
                stacked: false,
                ctl_w: cw,
                ctl_y: 0.0,
                label_cy: if lines.is_empty() { h / 2.0 } else { 13.0 },
                lines,
            };
        }
    }
    let lines = help(p, avail);
    let head = if lines.is_empty() {
        28.0
    } else {
        help_h(lines.len()) - 2.0
    };
    let ctl_h = ctl_height(&r.ctl).max(28.0);
    RowLay {
        h: head + ctl_h + 6.0,
        stacked: true,
        ctl_w: match r.ctl {
            Ctl::Slider { .. } | Ctl::Dropdown { .. } => avail,
            _ => want.min(avail),
        },
        ctl_y: head,
        label_cy: if lines.is_empty() { 15.0 } else { 13.0 },
        lines,
    }
}

const SUB_H: f32 = 35.0;
/// The top padding of a card with no header.
const BARE_TOP: f32 = 10.0;

fn items_height(p: &mut Painter, items: &[Item], inner_w: f32) -> f32 {
    items
        .iter()
        .map(|it| match it {
            Item::Row(r) => layout_row(p, r, inner_w).h,
            Item::Sub(_) => SUB_H,
            Item::Note(t) => 8.0 + 15.0 * wrap(p, Face::Ui, SIZE_SMALL, t, inner_w).len() as f32,
            Item::Inline(parts) => inline_height(inline_lines(p, parts, inner_w).len()),
        })
        .sum()
}

fn rack_w(p: &mut Painter, card: &Card, rack: &Rack) -> f32 {
    let strips = strip_w(p, &rack.names, rack.master) * rack.names.len() as f32 + 8.0;
    let mut head = 12.0 + p.measure(Face::Semibold, 11.0, &card.title.to_uppercase()) + 12.0;
    if card.head.is_some() {
        head += 8.0 + TOGGLE_W;
        if let Some(l) = &card.head_label {
            head += 8.0 + p.measure(Face::Semibold, 11.0, l);
        }
    }
    strips.max(head).ceil()
}

fn card_height(p: &mut Painter, card: &Card, w: f32) -> f32 {
    if card.rack.is_some() {
        return RACK_H;
    }
    let inner = w - 2.0 * CARD_PAD;
    let bare = card.title.is_empty() && card.head.is_none();
    (if bare { BARE_TOP } else { CARD_HEAD })
        + card
            .parts
            .iter()
            .map(|part| match part {
                Part::Items(items) => items_height(p, items, inner),
                Part::Draw(d) => d.h,
            })
            .sum::<f32>()
        + CARD_FOOT
}

/// Draws a row's control into `r` and records it as inert unless it answers a click.
fn draw_ctl(
    p: &mut Painter,
    env: &mut Env,
    ground: Rgb,
    pal: &Palette,
    ctl: &Ctl,
    r: Rect,
    left: bool,
) {
    let place = |w: f32, h: f32| {
        let w = w.min(r.w);
        Rect::new(
            if left { r.x } else { r.right() - w },
            r.cy() - h / 2.0,
            w,
            h,
        )
    };
    match ctl {
        Ctl::Toggle(on, bind) => {
            let b = place(TOGGLE_W, TOGGLE_H);
            env.toggle(p, pal, b, *on);
            let hit = env.toggle_hit(*on, *bind);
            env.hits.add(hit, b);
        }
        Ctl::Seg {
            labels,
            sel,
            live,
            small,
            bind,
        } => {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let w = seg_width(p, &refs, *small).ceil();
            let seg_r = place(w, if *small { SEG_H_SM } else { SEG_H });
            let hits: Option<Vec<Hit>> = match (live, env.gate(*bind)) {
                (Some(mk), _) => Some((0..labels.len()).map(mk).collect()),
                (None, Ok(_)) => Some((0..labels.len()).map(|k| seg_hit(env, *bind, k)).collect()),
                (None, Err(_)) => None,
            };
            let hovered = hits
                .as_ref()
                .and_then(|hs| hs.iter().position(|h| h.acts() && env.hover == Some(*h)));
            let rects = env.seg(
                p,
                pal,
                seg_r,
                &refs,
                *sel,
                hovered,
                SegKind {
                    small: *small,
                    fill: false,
                },
            );
            match (hits, env.gate(*bind)) {
                (Some(hs), _) => {
                    for (h, rc) in hs.into_iter().zip(&rects) {
                        env.hits.add(h, *rc);
                    }
                }
                (None, Err(why)) => env.hits.add(Hit::Inert(why), seg_r),
                (None, Ok(_)) => {}
            }
        }
        Ctl::Slider {
            frac,
            reading,
            from_middle,
            bind,
        } => {
            let rw = reading_w(p, reading);
            let track = Rect::new(r.x, r.y, (r.w - rw - 10.0).max(40.0), r.h);
            let held = env.held(*bind);
            let frac = env.glide(*frac, held);
            slider(p, pal, track, frac, *from_middle, ground);
            p.text_right(Face::Ui, SIZE_VALUE, r.right(), r.cy(), reading, pal.muted);
            let hit = match (env.gate(*bind), frac) {
                (Ok(s), Some(_)) => Hit::Slide(s),
                (Ok(_), None) => Hit::Inert(Why::NotRead),
                (Err(why), _) => Hit::Inert(why),
            };
            // A live slider is taken by its track; a read-only one says why over its whole row.
            let area = if matches!(hit, Hit::Slide(_)) {
                track
            } else {
                Rect::new(r.x, r.y, r.w, r.h)
            };
            env.hits.add(hit, area);
        }
        Ctl::Dropdown {
            text,
            options,
            bind,
        } => {
            let w = dropdown_want(p, text, options);
            let b = place(if left { r.w } else { w }, CTL_H);
            dropdown(p, pal, b, text);
            let hit = match env.gate(*bind) {
                Ok(s) => Hit::Open(s),
                Err(why) => Hit::Inert(why),
            };
            env.hits.add(hit, b);
        }
        Ctl::Value { text, mono } => {
            let face = if *mono { Face::Mono } else { Face::Ui };
            let w = p.measure(face, SIZE_VALUE, text).min(r.w);
            let x = if left { r.x } else { r.right() - w };
            p.text(face, SIZE_VALUE, x, r.cy(), text, pal.muted, r.w);
        }
        Ctl::Button { label, hit } => {
            let b = place(button_w(p, label), CTL_H);
            let live_pal = env.pal;
            env.button(p, &live_pal, b, label, *hit);
            env.hits.add(*hit, b);
        }
    }
}

fn draw_items(
    p: &mut Painter,
    env: &mut Env,
    pal: &Palette,
    ground: Rgb,
    items: &[Item],
    area: Rect,
) -> f32 {
    let mut y = area.y;
    for it in items {
        match it {
            Item::Row(r) => {
                let lay = layout_row(p, r, area.w);
                let indent = if r.child { 16.0 } else { 0.0 };
                let lx = area.x + indent;
                let avail = area.w - indent;
                if r.child {
                    p.fill(
                        Rect::new(area.x + 5.0, y + 2.0, p.px().max(1.0), lay.h - 4.0),
                        pal.popup_line,
                    );
                }
                let label_cy = y + lay.label_cy;
                if !r.label.is_empty() {
                    let max = if lay.stacked {
                        avail
                    } else {
                        // A whole pixel of slack: the measured label must not come out a hair over its room.
                        (avail - lay.ctl_w - CTL_GAP).max(label_w_of(p, r) + 1.0)
                    };
                    let used = p.text(Face::Ui, SIZE_LABEL, lx, label_cy, &r.label, pal.text, max);
                    if let Some((t, tone)) = &r.badge {
                        badge(p, pal, lx + used + 8.0, label_cy, t, *tone);
                    }
                }
                for (i, l) in lay.lines.iter().enumerate() {
                    p.text(
                        Face::Ui,
                        SIZE_SMALL,
                        lx,
                        y + 29.0 + HELP_LINE * i as f32,
                        l,
                        pal.muted,
                        avail,
                    );
                }
                let ctl_r = if lay.stacked {
                    Rect::new(lx, y + lay.ctl_y, avail, lay.h - lay.ctl_y - 6.0)
                } else if r.label.is_empty() {
                    Rect::new(lx, y, lay.ctl_w, lay.h)
                } else {
                    Rect::new(area.right() - lay.ctl_w, y, lay.ctl_w, lay.h)
                };
                draw_ctl(p, env, ground, pal, &r.ctl, ctl_r, lay.stacked);
                y += lay.h;
            }
            Item::Sub(t) => {
                p.fill(
                    Rect::new(area.x, y + 10.0, area.w, p.px().max(1.0)),
                    pal.popup_line,
                );
                p.text(Face::Semibold, 11.0, area.x, y + 25.0, t, pal.muted, area.w);
                y += SUB_H;
            }
            Item::Inline(parts) => {
                let lines = inline_lines(p, parts, area.w);
                for (n, line) in lines.iter().enumerate() {
                    let cy = y + n as f32 * 36.0 + 14.0;
                    let mut x = area.x;
                    for &(i, ew) in line {
                        let (label, ctl) = &parts[i];
                        let lw = p.measure(Face::Ui, SIZE_VALUE, label);
                        p.text(Face::Ui, SIZE_VALUE, x, cy, label, pal.muted, lw + 1.0);
                        let c = Rect::new(x + lw + 8.0, cy - 14.0, ew - lw - 8.0, 28.0);
                        draw_ctl(p, env, ground, pal, ctl, c, true);
                        x += ew + 20.0;
                    }
                }
                y += inline_height(lines.len());
            }
            Item::Note(t) => {
                let lines = wrap(p, Face::Ui, SIZE_SMALL, t, area.w);
                for (i, l) in lines.iter().enumerate() {
                    p.text(
                        Face::Ui,
                        SIZE_SMALL,
                        area.x,
                        y + 11.0 + 15.0 * i as f32,
                        l,
                        pal.muted,
                        area.w,
                    );
                }
                y += 15.0 * lines.len() as f32 + 8.0;
            }
        }
    }
    y - area.y
}

/// Draws one card at `r`.
fn draw_card(p: &mut Painter, env: &mut Env, card: &Card, r: Rect) {
    let pal = env.pal;
    p.frame(r, 10.0, pal.popup, pal.popup_line);
    // A card that is bypassed, or whose own switch is off, has its controls drawn faint, as the mockup's `.off`.
    let faded = card.off || card.head == Some(Some(false));
    let dim = if faded { dimmed(&pal, pal.popup) } else { pal };
    let px = p.px().max(1.0);
    let rack = card.rack.is_some();
    let bare = card.title.is_empty() && card.head.is_none() && !rack;
    let (head_h, title_x) = if rack {
        (RACK_HEAD, r.x + 12.0)
    } else {
        (CARD_HEAD_LINE, r.x + CARD_PAD)
    };
    let head_cy = if rack { r.y + head_h / 2.0 } else { r.y + 27.0 };
    if !bare {
        p.fill(
            Rect::new(r.x + 1.0, r.y + head_h, r.w - 2.0, px),
            pal.popup_line,
        );
    }
    let right = r.right() - if rack { 12.0 } else { CARD_PAD };
    let mut tx = right;
    if let Some(on) = card.head {
        tx -= TOGGLE_W;
        let b = Rect::new(tx, head_cy - TOGGLE_H / 2.0, TOGGLE_W, TOGGLE_H);
        env.toggle(p, &pal, b, on);
        let hit = env.toggle_hit(on, card.head_bind);
        env.hits.add(hit, b);
        if let Some(l) = &card.head_label {
            let w = p.measure(Face::Semibold, 11.0, l);
            tx -= 8.0 + w;
            p.text(Face::Semibold, 11.0, tx, head_cy, l, pal.muted, w + 1.0);
        }
        tx -= 8.0;
    }
    let title_max = (tx - title_x).max(20.0);
    if rack {
        p.text(
            Face::Semibold,
            11.0,
            title_x,
            head_cy,
            &card.title.to_uppercase(),
            pal.muted,
            title_max,
        );
    } else {
        let c = if card.off { dim.text } else { pal.text };
        p.text(
            Face::Semibold,
            SIZE_CARD,
            title_x,
            head_cy,
            &card.title,
            c,
            title_max,
        );
    }
    if rack {
        for part in &card.parts {
            if let Part::Draw(d) = part {
                (d.f)(
                    p,
                    env,
                    Rect::new(
                        r.x + 4.0,
                        r.y + head_h + 8.0,
                        r.w - 8.0,
                        r.h - head_h - 16.0,
                    ),
                );
            }
        }
        return;
    }
    let mut y = r.y + if bare { BARE_TOP } else { CARD_HEAD };
    let inner_x = r.x + CARD_PAD;
    let inner_w = r.w - 2.0 * CARD_PAD;
    for part in &card.parts {
        match part {
            Part::Items(items) => {
                let area = Rect::new(inner_x, y, inner_w, 0.0);
                y += draw_items(p, env, &dim, pal.popup, items, area);
            }
            Part::Draw(d) => {
                (d.f)(p, env, Rect::new(inner_x, y, inner_w, d.h));
                y += d.h;
            }
        }
    }
}

/// How many columns a run of `run` plain cards takes in `width`, and how wide each is: as many as fit (at least
/// [`COL_MIN`] each) but never more than there are cards, so none is left empty.
fn grid_cols(width: f32, run: usize) -> (usize, f32) {
    let max_cols = (((width + GRID_GAP) / (COL_MIN + GRID_GAP)).floor() as usize).max(1);
    let n = run.clamp(1, max_cols);
    (n, ((width - GRID_GAP * (n - 1) as f32) / n as f32).floor())
}

/// Lays the cards out like the mockup's grid and draws them. Plain cards share the width in columns at least
/// [`COL_MIN`] wide, and a row with fewer cards than columns stretches them to fill it. A card that `span`s takes a
/// row of its own; mixer racks keep the width of their strips and wrap like text. Returns the height used.
pub fn flow(p: &mut Painter, env: &mut Env, area: Rect, cards: &[Card]) -> f32 {
    let mut y = area.y;
    let mut i = 0;
    while i < cards.len() {
        // The cards of this row, with their widths.
        let mut row: Vec<(usize, f32)> = Vec::new();
        if cards[i].span {
            row.push((i, area.w));
            i += 1;
        } else if let Some(rack) = &cards[i].rack {
            let mut x = 0.0;
            while i < cards.len() {
                let Some(rk) = &cards[i].rack else { break };
                let w = rack_w(p, &cards[i], rk).min(area.w);
                if !row.is_empty() && x + w > area.w {
                    break;
                }
                row.push((i, w));
                x += w + GRID_GAP;
                i += 1;
            }
            let _ = rack;
        } else {
            let mut run = 0;
            while i + run < cards.len() && !cards[i + run].span && cards[i + run].rack.is_none() {
                run += 1;
            }
            let (n, col_w) = grid_cols(area.w, run);
            for k in 0..n {
                row.push((i + k, col_w));
            }
            i += n;
        }
        let h = row
            .iter()
            .map(|&(k, w)| card_height(p, &cards[k], w))
            .fold(0.0_f32, f32::max);
        let mut x = area.x;
        for &(k, w) in &row {
            draw_card(p, env, &cards[k], Rect::new(x, y, w, h));
            x += w + GRID_GAP;
        }
        y += h + GRID_GAP;
    }
    (y - area.y - GRID_GAP).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;

    #[test]
    fn hits_answer_the_last_one_drawn() {
        let mut h = Hits::default();
        h.add(Hit::Drag, Rect::new(0.0, 0.0, 100.0, 40.0));
        h.add(Hit::Close, Rect::new(50.0, 0.0, 50.0, 40.0));
        assert_eq!(h.at(60.0, 10.0), Some(Hit::Close));
        assert_eq!(h.at(10.0, 10.0), Some(Hit::Drag));
        assert_eq!(h.at(10.0, 50.0), None);
        assert_eq!(h.rect_of(Hit::Close).map(|r| r.x), Some(50.0));
    }

    #[test]
    fn wrapping_keeps_words_whole() {
        let mut bmp = Bitmap::new(8, 8);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let lines = wrap(
            &mut p,
            Face::Ui,
            11.5,
            "one two three four five six seven",
            90.0,
        );
        assert!(lines.len() > 1);
        assert_eq!(lines.join(" "), "one two three four five six seven");
        assert!(wrap(&mut p, Face::Ui, 11.5, "", 90.0).is_empty());
    }

    #[test]
    fn a_dimmed_palette_keeps_what_is_not_a_control_colour() {
        let pal = mdshared_ui::theme::DARK;
        let d = dimmed(&pal, pal.popup);
        assert_eq!(d.popup, pal.popup);
        assert_ne!(d.text, pal.text);
        assert!(d.dark);
    }

    fn painter_parts() -> (Bitmap, Gfx) {
        (Bitmap::new(8, 8), Gfx::new())
    }

    #[test]
    fn a_segmented_choice_is_as_wide_as_its_text_and_room() {
        let (mut bmp, mut gfx) = painter_parts();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let labels = ["Normal", "Full", "Night"];
        let w = seg_width(&mut p, &labels, false).ceil();
        let mut big = Bitmap::new(300, 40);
        let mut p2 = Painter::new(&mut big, &mut gfx, 1.0);
        let pal = mdshared_ui::theme::DARK;
        let kind = SegKind {
            small: false,
            fill: false,
        };
        let r = Rect::new(2.0, 2.0, w, SEG_H);
        let rects = draw_seg(&mut p2, &pal, r, &labels, Some(2), None, kind);
        // The buttons tile the inside of the frame, and each is wider than its text by the room on both sides.
        assert!((rects[0].x - (r.x + 2.0)).abs() < 0.01);
        assert!((rects[2].right() - (r.right() - 2.0)).abs() < 0.01);
        let night = p2.measure(Face::Semibold, SIZE_BODY, "Night");
        assert!(rects[2].w >= night + 26.0 - 0.01, "{} {night}", rects[2].w);
    }

    #[test]
    fn a_control_that_does_not_fit_beside_its_label_goes_under_it() {
        let (mut bmp, mut gfx) = painter_parts();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let r = row(
            "Speakers",
            seg(
                &["Stereo", "5.1", "7.1"],
                None,
                Bind::Off(Why::SpeakerSetup),
            ),
        );
        assert!(!layout_row(&mut p, &r, 400.0).stacked);
        let narrow = layout_row(&mut p, &r, 200.0);
        assert!(narrow.stacked);
        assert!(narrow.h > 34.0);
        // A dropdown shrinks first, and only then stacks.
        let d = row(
            "DAC filter",
            dd(
                "Slow Roll-off / Minimum Phase",
                &[],
                Bind::Live(Setting::DacFilter),
            ),
        );
        let beside = layout_row(&mut p, &d, 330.0);
        assert!(!beside.stacked);
        assert!(beside.ctl_w >= 130.0);
        assert!(layout_row(&mut p, &d, 170.0).stacked);
    }

    #[test]
    fn cards_fill_the_width_with_no_empty_column() {
        // Room for three columns of 280: two cards share all of the width, three take a column each.
        assert_eq!(grid_cols(900.0, 2), (2, 444.0));
        assert_eq!(grid_cols(900.0, 3), (3, 292.0));
        assert_eq!(grid_cols(900.0, 5), (3, 292.0));
        // Narrow: one column.
        assert_eq!(grid_cols(300.0, 4), (1, 300.0));
    }

    // ---- motion ----

    use mdshared_ui::anim::{Clock, ManualClock};

    fn pixels(bmp: &Bitmap) -> Vec<u8> {
        bmp.pixels.clone()
    }

    /// One frame of an animated segmented choice, `ms` after the last.
    fn seg_frame(
        clock: &ManualClock,
        anim: &mut Animator,
        ms: u64,
        sel: Option<usize>,
        hovered: Option<usize>,
    ) -> (Vec<u8>, Option<std::time::Instant>) {
        clock.advance_ms(ms);
        anim.begin_frame(clock.now());
        let mut bmp = Bitmap::new(300, 40);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        p.fill(
            Rect::new(0.0, 0.0, 300.0, 40.0),
            mdshared_ui::theme::LIGHT.popup,
        );
        let kind = SegKind {
            small: false,
            fill: false,
        };
        let r = Rect::new(4.0, 4.0, 240.0, SEG_H);
        draw_seg_with(
            &mut p,
            &mdshared_ui::theme::LIGHT,
            Some((&mut *anim, Id::new("seg"))),
            r,
            &["Light", "Dark", "System"],
            sel,
            hovered,
            kind,
        );
        (pixels(&bmp), anim.end_frame())
    }

    fn seg_at_rest(sel: Option<usize>, hovered: Option<usize>) -> Vec<u8> {
        let mut bmp = Bitmap::new(300, 40);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        p.fill(
            Rect::new(0.0, 0.0, 300.0, 40.0),
            mdshared_ui::theme::LIGHT.popup,
        );
        let kind = SegKind {
            small: false,
            fill: false,
        };
        draw_seg(
            &mut p,
            &mdshared_ui::theme::LIGHT,
            Rect::new(4.0, 4.0, 240.0, SEG_H),
            &["Light", "Dark", "System"],
            sel,
            hovered,
            kind,
        );
        pixels(&bmp)
    }

    #[test]
    fn a_segmented_choice_slides_to_the_new_button_and_comes_to_rest() {
        let (clock, mut anim) = (ManualClock::new(), Animator::new());
        // First sight is at rest: no motion, no frame asked for.
        let (first, deadline) = seg_frame(&clock, &mut anim, 0, Some(0), None);
        assert_eq!(first, seg_at_rest(Some(0), None));
        assert!(deadline.is_none());
        // The choice changes: the frame that says so still shows the old one, then it is in between.
        let (same, deadline) = seg_frame(&clock, &mut anim, 16, Some(2), None);
        assert_eq!(same, seg_at_rest(Some(0), None));
        assert!(deadline.is_some());
        let (mid, deadline) = seg_frame(&clock, &mut anim, 64, Some(2), None);
        assert!(deadline.is_some());
        assert_ne!(mid, seg_at_rest(Some(0), None));
        assert_ne!(mid, seg_at_rest(Some(2), None));
        let mut last = (mid, deadline);
        for _ in 0..30 {
            last = seg_frame(&clock, &mut anim, 16, Some(2), None);
            if last.1.is_none() {
                break;
            }
        }
        assert!(last.1.is_none(), "the slide never ends");
        assert_eq!(last.0, seg_at_rest(Some(2), None));
    }

    #[test]
    fn a_choice_that_was_not_read_appears_at_once() {
        let (clock, mut anim) = (ManualClock::new(), Animator::new());
        let (none, deadline) = seg_frame(&clock, &mut anim, 0, None, None);
        assert_eq!(none, seg_at_rest(None, None));
        assert!(deadline.is_none());
        // The first read arrives: the choice is there, not faded in.
        let (read, deadline) = seg_frame(&clock, &mut anim, 16, Some(1), None);
        assert_eq!(read, seg_at_rest(Some(1), None));
        assert!(deadline.is_none());
    }

    #[test]
    fn the_hover_of_a_segmented_choice_fades_in_and_out() {
        let (clock, mut anim) = (ManualClock::new(), Animator::new());
        seg_frame(&clock, &mut anim, 0, Some(0), None);
        seg_frame(&clock, &mut anim, 16, Some(0), Some(1));
        let (mid, deadline) = seg_frame(&clock, &mut anim, 32, Some(0), Some(1));
        assert!(deadline.is_some());
        assert_ne!(mid, seg_at_rest(Some(0), None));
        assert_ne!(mid, seg_at_rest(Some(0), Some(1)));
        // A frame is never taken for more than 50 ms of motion, so it takes a few to arrive.
        let mut last = (mid, Some(clock.now()));
        for _ in 0..20 {
            last = seg_frame(&clock, &mut anim, 16, Some(0), Some(1));
            if last.1.is_none() {
                break;
            }
        }
        assert!(last.1.is_none());
        assert_eq!(last.0, seg_at_rest(Some(0), Some(1)));
    }

    #[test]
    fn reduced_motion_puts_a_segmented_choice_on_its_new_button_at_once() {
        let (clock, mut anim) = (ManualClock::new(), Animator::new());
        anim.set_system_reduced(true);
        seg_frame(&clock, &mut anim, 0, Some(0), None);
        let (now, deadline) = seg_frame(&clock, &mut anim, 16, Some(2), Some(1));
        assert_eq!(now, seg_at_rest(Some(2), Some(1)));
        assert!(deadline.is_none());
    }

    #[test]
    fn a_switch_is_its_resting_look_at_both_ends_and_between_in_the_middle() {
        let pal = mdshared_ui::theme::LIGHT;
        let draw = |t: f32| {
            let mut bmp = Bitmap::new(60, 30);
            let mut gfx = Gfx::new();
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            p.fill(Rect::new(0.0, 0.0, 60.0, 30.0), pal.popup);
            toggle_at(&mut p, &pal, Rect::new(10.0, 5.0, TOGGLE_W, TOGGLE_H), t);
            pixels(&bmp)
        };
        let plain = |on: bool| {
            let mut bmp = Bitmap::new(60, 30);
            let mut gfx = Gfx::new();
            let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
            p.fill(Rect::new(0.0, 0.0, 60.0, 30.0), pal.popup);
            toggle(
                &mut p,
                &pal,
                Rect::new(10.0, 5.0, TOGGLE_W, TOGGLE_H),
                Some(on),
            );
            pixels(&bmp)
        };
        assert_eq!(draw(0.0), plain(false));
        assert_eq!(draw(1.0), plain(true));
        assert_ne!(draw(0.5), plain(false));
        assert_ne!(draw(0.5), plain(true));
    }
}
