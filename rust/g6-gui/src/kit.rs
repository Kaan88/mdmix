//! The drawing kit of the pages: controls in the toolkit's look, cards of rows, and the grid the cards flow in.
//!
//! Step G2 draws every device control as it is on the device and inert: no hit is recorded for it. The few live
//! controls (pages, Retry, Refresh, the appearance choices) carry a [`Hit`].

use mdshared_ui::chrome::button_fill;
use mdshared_ui::gfx::{Face, Painter, Rect, Rgb, mix};
use mdshared_ui::glyph;
use mdshared_ui::segments;
use mdshared_ui::theme::Palette;

use crate::model::{Model, Page};

/// What a click on a region does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

/// The regions the last frame drew, in drawing order; the one drawn last is on top.
#[derive(Clone, Debug, Default)]
pub struct Hits(Vec<(Hit, Rect)>);

impl Hits {
    pub fn add(&mut self, hit: Hit, r: Rect) {
        self.0.push((hit, r));
    }

    pub fn at(&self, x: f32, y: f32) -> Option<Hit> {
        self.0
            .iter()
            .rev()
            .find(|(_, r)| r.contains(x, y))
            .map(|(h, _)| *h)
    }

    #[cfg(test)]
    pub fn rect_of(&self, hit: Hit) -> Option<Rect> {
        self.0
            .iter()
            .rev()
            .find(|(h, _)| *h == hit)
            .map(|(_, r)| *r)
    }
}

/// Everything a page needs to draw: the colours, the device state, the pointer.
pub struct Env<'a> {
    pub pal: Palette,
    pub model: &'a Model,
    pub hits: Hits,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
}

impl Env<'_> {
    pub fn is_hover(&self, hit: Hit) -> bool {
        self.hover == Some(hit)
    }

    pub fn is_pressed(&self, hit: Hit) -> bool {
        self.pressed == Some(hit)
    }
}

/// The palette of a disabled control: text, accent and edges drawn part of the way to the ground they sit on.
pub fn dimmed(pal: &Palette, ground: Rgb) -> Palette {
    let f = |c: Rgb, t: f32| mix(c, ground, t);
    Palette {
        text: f(pal.text, 0.42),
        muted: f(pal.muted, 0.40),
        accent: f(pal.accent, 0.55),
        field: f(pal.field, 0.30),
        line: f(pal.line, 0.25),
        popup_line: f(pal.popup_line, 0.25),
        hover: pal.field,
        ..*pal
    }
}

// ---- controls ---------------------------------------------------------------------------------

pub const TOGGLE_W: f32 = 40.0;
pub const TOGGLE_H: f32 = 20.0;
pub const SEG_H: f32 = segments::SEGMENT_H;
pub const CTL_H: f32 = 28.0;

/// The switch of the settings dialog: a pill with the knob at the end that says on. `None` is a state not read: a dash.
pub fn toggle(p: &mut Painter, pal: &Palette, r: Rect, on: Option<bool>) {
    let Some(on) = on else {
        p.text_center(Face::Ui, 13.0, r, "—", pal.muted);
        return;
    };
    let knob = 12.0;
    if on {
        p.rbox(r, r.h / 2.0, pal.accent);
        p.rbox(
            Rect::new(r.right() - 4.0 - knob, r.cy() - knob / 2.0, knob, knob),
            knob / 2.0,
            pal.on_accent,
        );
    } else {
        p.frame(r, r.h / 2.0, pal.field, mix(pal.muted, pal.popup, 0.35));
        p.rbox(
            Rect::new(r.x + 4.0, r.cy() - knob / 2.0, knob, knob),
            knob / 2.0,
            pal.muted,
        );
    }
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

/// A dropdown as it looks closed: the value and a chevron.
pub fn dropdown(p: &mut Painter, pal: &Palette, r: Rect, text: &str) {
    p.frame(r, 6.0, pal.field, pal.line);
    p.text(
        Face::Ui,
        13.0,
        r.x + 10.0,
        r.cy(),
        text,
        pal.text,
        r.w - 36.0,
    );
    p.icon(glyph::DOWN, 10.0, r.right() - 14.0, r.cy(), pal.muted);
}

/// A button. `live` buttons answer the pointer; the others are drawn quiet.
pub fn button(p: &mut Painter, pal: &Palette, r: Rect, label: &str, hovered: bool, pressed: bool) {
    let fill =
        button_fill(pal, hovered, pressed, false, false).unwrap_or(mix(pal.popup, pal.text, 0.04));
    p.frame(r, 5.0, fill, pal.popup_line);
    p.text_center(Face::Ui, 13.0, r, label, pal.text);
}

pub fn button_w(p: &mut Painter, label: &str) -> f32 {
    (p.measure(Face::Ui, 13.0, label) + 28.0).round()
}

/// A small label: "In use", "SBX", "Not connected". `warn` draws it in the warning colours.
pub fn badge(p: &mut Painter, pal: &Palette, x: f32, cy: f32, text: &str, tone: Tone) -> f32 {
    let w = (p.measure(Face::Semibold, 10.5, text) + 12.0).ceil();
    let r = Rect::new(x, cy - 9.0, w, 18.0);
    let (fill, edge, ink) = match tone {
        Tone::Quiet => (pal.popup, pal.popup_line, pal.muted),
        Tone::Lit => (mix(pal.popup, pal.accent, 0.14), pal.accent, pal.accent),
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
    Toggle(Option<bool>),
    /// A segmented choice. `live` makes each segment a hit (`Hit` built from its index).
    Seg {
        labels: Vec<String>,
        sel: Option<usize>,
        live: Option<fn(usize) -> Hit>,
    },
    /// A slider with its reading. `frac` is 0 to 1, `None` when the value is not known.
    Slider {
        frac: Option<f32>,
        reading: String,
        from_middle: bool,
    },
    Dropdown(String),
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

pub fn seg(labels: &[&str], sel: Option<usize>) -> Ctl {
    Ctl::Seg {
        labels: labels.iter().map(|s| s.to_string()).collect(),
        sel,
        live: None,
    }
}

pub fn live_seg(labels: &[&str], sel: Option<usize>, hit: fn(usize) -> Hit) -> Ctl {
    Ctl::Seg {
        labels: labels.iter().map(|s| s.to_string()).collect(),
        sel,
        live: Some(hit),
    }
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

pub struct Card<'a> {
    pub title: String,
    /// A switch in the header, as the mockup's cards have.
    pub head: Option<Option<bool>>,
    pub parts: Vec<Part<'a>>,
    /// Takes the whole row of the grid.
    pub span: bool,
    /// The whole card is out of use (its effect is bypassed): drawn fainter still.
    pub off: bool,
}

impl<'a> Card<'a> {
    pub fn new(title: &str, items: Vec<Item>) -> Card<'a> {
        Card {
            title: title.into(),
            head: None,
            parts: vec![Part::Items(items)],
            span: false,
            off: false,
        }
    }
    pub fn head(mut self, on: Option<bool>) -> Card<'a> {
        self.head = Some(on);
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
    pub fn then(mut self, part: Part<'a>) -> Card<'a> {
        self.parts.push(part);
        self
    }
}

pub const CARD_PAD: f32 = 16.0;
pub const CARD_HEAD: f32 = 36.0;
pub const CARD_FOOT: f32 = 8.0;
pub const GRID_GAP: f32 = 12.0;
pub const COL_MIN: f32 = 280.0;
const LABEL_SIZE: f32 = 13.5;
const HELP_SIZE: f32 = 11.5;
const HELP_LINE: f32 = 14.0;
const CTL_GAP: f32 = 12.0;

/// The width a control wants, before it is cut to what the row can give.
fn ctl_width(p: &mut Painter, ctl: &Ctl) -> f32 {
    match ctl {
        Ctl::Toggle(_) => TOGGLE_W,
        Ctl::Seg { labels, .. } => {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            segments::switch_width(p, &refs)
        }
        Ctl::Slider { reading, .. } => 150.0 + 10.0 + p.measure(Face::Ui, 13.0, reading).max(40.0),
        Ctl::Dropdown(t) => (p.measure(Face::Ui, 13.0, t) + 46.0).clamp(150.0, 250.0),
        Ctl::Value { text, mono } => {
            let face = if *mono { Face::Mono } else { Face::Ui };
            p.measure(face, 13.0, text)
        }
        Ctl::Button { label, .. } => button_w(p, label),
    }
}

/// The room a row's control gets: what it wants, at most 62 % of the row.
fn ctl_room(p: &mut Painter, r: &RowSpec, inner_w: f32) -> f32 {
    let want = ctl_width(p, &r.ctl);
    let indent = if r.child { 16.0 } else { 0.0 };
    let label = p.measure(Face::Ui, LABEL_SIZE, &r.label) + indent + CTL_GAP;
    // The label keeps the room it needs; the control keeps at least 40 % of the row.
    let most = (inner_w - label).max(inner_w * 0.4);
    want.min(most).max(TOGGLE_W.min(want))
}

fn row_height(p: &mut Painter, r: &RowSpec, inner_w: f32) -> f32 {
    let indent = if r.child { 16.0 } else { 0.0 };
    let label_w = (inner_w - ctl_room(p, r, inner_w) - CTL_GAP - indent).max(40.0);
    let lines = if r.help.is_empty() {
        0
    } else {
        wrap(p, Face::Ui, HELP_SIZE, &r.help, label_w).len().min(3)
    };
    if lines == 0 {
        34.0
    } else {
        26.0 + HELP_LINE * lines as f32
    }
}

fn items_height(p: &mut Painter, items: &[Item], inner_w: f32) -> f32 {
    items
        .iter()
        .map(|it| match it {
            Item::Row(r) => row_height(p, r, inner_w),
            Item::Sub(_) => 28.0,
            Item::Note(t) => {
                8.0 + HELP_LINE * wrap(p, Face::Ui, HELP_SIZE, t, inner_w).len() as f32
            }
        })
        .sum()
}

fn card_height(p: &mut Painter, card: &Card, w: f32) -> f32 {
    let inner = w - 2.0 * CARD_PAD;
    CARD_HEAD
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

/// Draws a row's control into `r` (its room, right aligned by the caller) and records its hits.
fn draw_ctl(p: &mut Painter, env: &mut Env, pal: &Palette, ground: Rgb, ctl: &Ctl, r: Rect) {
    match ctl {
        Ctl::Toggle(on) => toggle(
            p,
            pal,
            Rect::new(
                r.right() - TOGGLE_W,
                r.cy() - TOGGLE_H / 2.0,
                TOGGLE_W,
                TOGGLE_H,
            ),
            *on,
        ),
        Ctl::Seg { labels, sel, live } => {
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let w = segments::switch_width(p, &refs).min(r.w);
            let seg_r = Rect::new(r.right() - w, r.cy() - SEG_H / 2.0, w, SEG_H);
            let hovered = live.and_then(|mk| (0..labels.len()).find(|&k| env.hover == Some(mk(k))));
            let live_pal = env.pal;
            let use_pal = if live.is_some() { &live_pal } else { pal };
            let rects = segments::draw_segments(p, use_pal, seg_r, &refs, *sel, hovered);
            if let Some(mk) = live {
                for (k, rc) in rects.iter().enumerate() {
                    env.hits.add(mk(k), *rc);
                }
            }
        }
        Ctl::Slider {
            frac,
            reading,
            from_middle,
        } => {
            let rw = p.measure(Face::Ui, 13.0, reading).max(40.0);
            let tw = (r.w - rw - 10.0).clamp(40.0, 150.0);
            let track = Rect::new(r.right() - rw - 10.0 - tw, r.y, tw, r.h);
            slider(p, pal, track, *frac, *from_middle, ground);
            p.text_right(Face::Ui, 13.0, r.right(), r.cy(), reading, pal.text);
        }
        Ctl::Dropdown(text) => {
            let w = ctl_width(p, ctl).min(r.w);
            dropdown(
                p,
                pal,
                Rect::new(r.right() - w, r.cy() - CTL_H / 2.0, w, CTL_H),
                text,
            );
        }
        Ctl::Value { text, mono } => {
            let face = if *mono { Face::Mono } else { Face::Ui };
            let w = p.measure(face, 13.0, text).min(r.w);
            p.text(face, 13.0, r.right() - w, r.cy(), text, pal.text, r.w);
        }
        Ctl::Button { label, hit } => {
            let w = button_w(p, label).min(r.w);
            let b = Rect::new(r.right() - w, r.cy() - 14.0, w, 28.0);
            let (hv, pr) = (env.is_hover(*hit), env.is_pressed(*hit));
            let live_pal = env.pal;
            button(p, &live_pal, b, label, hv, pr);
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
                let h = row_height(p, r, area.w);
                let indent = if r.child { 16.0 } else { 0.0 };
                let room = ctl_room(p, r, area.w);
                let label_w = (area.w - room - CTL_GAP - indent).max(40.0);
                let lx = area.x + indent;
                let lines = if r.help.is_empty() {
                    Vec::new()
                } else {
                    wrap(p, Face::Ui, HELP_SIZE, &r.help, label_w)
                };
                let label_cy = if lines.is_empty() {
                    y + h / 2.0
                } else {
                    y + 13.0
                };
                if r.child {
                    p.fill(
                        Rect::new(area.x + 5.0, y + 2.0, p.px().max(1.0), h - 4.0),
                        pal.popup_line,
                    );
                }
                let used = p.text(
                    Face::Ui,
                    LABEL_SIZE,
                    lx,
                    label_cy,
                    &r.label,
                    pal.text,
                    label_w,
                );
                if let Some((t, tone)) = &r.badge {
                    let bx = lx + used + 8.0;
                    if bx + 30.0 < lx + label_w {
                        badge(p, pal, bx, label_cy, t, *tone);
                    }
                }
                for (i, l) in lines.iter().take(3).enumerate() {
                    p.text(
                        Face::Ui,
                        HELP_SIZE,
                        lx,
                        y + 29.0 + HELP_LINE * i as f32,
                        l,
                        pal.muted,
                        label_w,
                    );
                }
                let ctl_r = Rect::new(area.right() - room, y, room, h);
                draw_ctl(p, env, pal, ground, &r.ctl, ctl_r);
                y += h;
            }
            Item::Sub(t) => {
                p.text(Face::Semibold, 11.0, area.x, y + 18.0, t, pal.muted, area.w);
                y += 28.0;
            }
            Item::Note(t) => {
                let lines = wrap(p, Face::Ui, HELP_SIZE, t, area.w);
                for (i, l) in lines.iter().enumerate() {
                    p.text(
                        Face::Ui,
                        HELP_SIZE,
                        area.x,
                        y + 11.0 + HELP_LINE * i as f32,
                        l,
                        pal.muted,
                        area.w,
                    );
                }
                y += HELP_LINE * lines.len() as f32 + 8.0;
            }
        }
    }
    y - area.y
}

/// Draws one card at `r`.
fn draw_card(p: &mut Painter, env: &mut Env, card: &Card, r: Rect) {
    let pal = env.pal;
    p.frame(r, 10.0, pal.popup, pal.popup_line);
    // Every control of a card is inert in step G2; a card that is bypassed is fainter again.
    let dim = if card.off {
        dimmed(&dimmed(&pal, pal.popup), pal.popup)
    } else {
        dimmed(&pal, pal.popup)
    };
    let title_color = if card.off { dim.text } else { pal.text };
    p.text(
        Face::Semibold,
        13.0,
        r.x + CARD_PAD,
        r.y + CARD_HEAD / 2.0,
        &card.title,
        title_color,
        r.w - 2.0 * CARD_PAD - 52.0,
    );
    if let Some(on) = card.head {
        toggle(
            p,
            &dim,
            Rect::new(
                r.right() - CARD_PAD - TOGGLE_W,
                r.y + (CARD_HEAD - TOGGLE_H) / 2.0,
                TOGGLE_W,
                TOGGLE_H,
            ),
            on,
        );
    }
    let mut y = r.y + CARD_HEAD;
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

/// Lays the cards out in a grid of columns at least [`COL_MIN`] wide that fill `area`'s width, and draws them. A
/// card that `span`s takes a row of its own. Returns the height used.
pub fn flow(p: &mut Painter, env: &mut Env, area: Rect, cards: &[Card]) -> f32 {
    let cols = (((area.w + GRID_GAP) / (COL_MIN + GRID_GAP)).floor() as usize).max(1);
    let col_w = ((area.w - GRID_GAP * (cols - 1) as f32) / cols as f32).floor();
    let mut y = area.y;
    let mut i = 0;
    while i < cards.len() {
        // The cards of this row: a spanning card alone, otherwise up to `cols` plain ones.
        let mut row: Vec<(usize, f32)> = Vec::new(); // (index, width)
        if cards[i].span {
            row.push((i, area.w));
            i += 1;
        } else {
            while i < cards.len() && !cards[i].span && row.len() < cols {
                row.push((i, col_w));
                i += 1;
            }
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
}
