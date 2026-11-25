//! One frame of the window: the title row, the sidebar, the page, and the dialog over them.
//!
//! The frame is drawn whole every time, into the painter it is given, in logical px. The title row and the sidebar
//! are the chrome that a backdrop shows through: after each is drawn it is marked for [`mdshared_ui::frost`], and
//! the areas come back in [`FrameOut`].
//!
//! What moves is asked of the animator in [`Fx`] while it is drawn (see `doc/gui-plan.md`, "Motion"). A frame drawn
//! with [`Fx::still`] has nothing in flight and is the picture at rest.

use mdshared_ui::anim::{Id, fluent, pop};
use mdshared_ui::chrome::{self, TITLE_H, WinButton};
use mdshared_ui::frost;
use mdshared_ui::gfx::{Face, Painter, Rect, mix};
use mdshared_ui::glyph;
use mdshared_ui::overlay::{Bubble, TOAST_H};
use mdshared_ui::scroll;
use mdshared_ui::settings::pages::Form;
use mdshared_ui::tooltip::Over;
use std::time::Instant;

use crate::facts::{self, DASH};
use crate::fx::Fx;
use crate::icons;
use crate::intent::Setting;
use crate::kit::{self, Bind, Env, Hit, Hits, Tone, Why, chrome_pal};
use crate::levels::Meters;
use crate::model::{Conn, Model, Page};
use crate::pages;
use crate::prefs::Prefs;

pub const SIDE_W: f32 = 236.0;
const PAD: f32 = 20.0;
const NAV_H: f32 = 34.0;

/// The part of the window a flyout may use: everything under the title row. A menu and the colour picker are placed
/// in it, so neither leaves the window at any edge.
pub fn popup_area(size: (f32, f32)) -> Rect {
    Rect::new(0.0, TITLE_H, size.0.max(0.0), (size.1 - TITLE_H).max(0.0))
}

/// The gap between two toasts of the stack.
const TOAST_GAP: f32 = 8.0;

/// The tip over the read-only control at (`x`, `y`) in `hits`: the control's rectangle, which names it to the
/// toolkit's tooltip (a neighbour's tip takes over at once) and places the bubble, and the reason's words.
pub fn tip_over(hits: &Hits, x: f32, y: f32) -> Option<Over<Rect>> {
    match hits.at_rect(x, y) {
        Some((Hit::Inert(why), r)) => Some(Over {
            key: r,
            text: why.text().to_string(),
            anchor: r,
        }),
        _ => None,
    }
}

/// What a frame is drawn from.
pub struct FrameIn<'a> {
    pub model: &'a Model,
    pub prefs: &'a Prefs,
    pub pal: mdshared_ui::theme::Palette,
    pub page: Page,
    /// Logical window size.
    pub size: (f32, f32),
    pub scroll: f32,
    pub maximized: bool,
    pub hover: Option<Hit>,
    /// Where the pointer is, for the tip over a control that is read only.
    pub pointer: Option<(f32, f32)>,
    /// The frame's time, for the tooltip's delay and its next wake.
    pub now: Instant,
    pub pressed: Option<Hit>,
    /// The slider in hand.
    pub dragging: Option<Setting>,
    /// Draw the window's own caption buttons (not in `--shot` of a bare page? always true today).
    pub frosted: bool,
    /// The live playback meter's channels, or no source at all: the Mixer's strips draw a level, or [`levels::NO_METER`].
    pub meters: &'a Meters,
}

/// What a frame learned while it was drawn.
pub struct FrameOut {
    pub hits: Hits,
    /// Chrome regions, logical px: where a backdrop shows through.
    pub chrome: Vec<Rect>,
    /// The most the page can scroll.
    pub max_scroll: f32,
    /// The page's viewport, for the wheel.
    pub view: Rect,
}

pub fn draw_frame(
    p: &mut Painter,
    input: &FrameIn,
    dialog: Option<&mut Form>,
    fx: &mut Fx,
) -> FrameOut {
    let (w, h) = input.size;
    let dialog_is_none = dialog.is_none();
    let pal = input.pal;
    let Fx {
        anim,
        page,
        toasts,
        leaving,
        tip,
        tip_last,
        menu,
        pick,
    } = fx;
    let mut env = Env::new(pal, input.model, input.hover, input.pressed, anim);
    env.dragging = input.dragging;
    p.fill(Rect::new(0.0, 0.0, w, h), pal.page);

    let title = Rect::new(0.0, 0.0, w, TITLE_H);
    let main = Rect::new(
        SIDE_W,
        TITLE_H,
        (w - SIDE_W).max(0.0),
        (h - TITLE_H).max(0.0),
    );
    let (view, max_scroll) = draw_main(p, &mut env, input, main);
    // A page switch blends the old page into the area of the page, over what it drew and under everything else.
    page.apply(env.anim, p, view);

    let side = Rect::new(0.0, TITLE_H, SIDE_W, (h - TITLE_H).max(0.0));
    draw_sidebar(p, &mut env, input, side);
    // The empty title row drags the window. It goes in after the page, which is clipped to its own view anyway, and
    // before the title's own buttons, which are on top of it.
    env.hits.add(Hit::Drag, title);
    if input.frosted {
        frost::mark(p.bmp, p.dev_rect(side));
    }
    draw_title(p, &mut env, input, title);
    if input.frosted {
        frost::mark(p.bmp, p.dev_rect(title));
    }

    // The dialog: it comes in over the window and goes out the same way, as one picture. (The toolkit's form draws
    // its own dimming, so the whole window's worth fades and nothing rises: a shifted dimming would leave a strip.)
    // Inside it, the form's own motion: switches and chosen segments slide, its hover fills fade.
    let shown = dialog.is_some();
    if shown {
        *leaving = None;
    }
    if let Some(form) = dialog.or(leaving.as_mut()) {
        let t = env
            .anim
            .presence("dialog", shown, fluent::ENTER, fluent::EXIT);
        let window = Rect::new(0.0, 0.0, w, h);
        let anim = &mut *env.anim;
        pop(p, window, t, (0.0, 0.0), |p| {
            form.draw_animated(p, &pal, window, false, anim, "settings")
        });
        if !shown && t <= 0.0 {
            *leaving = None;
        }
    }
    // A dropdown's choices, over the page and the chrome.
    if let Some(m) = menu.as_mut().filter(|_| dialog_is_none) {
        m.draw_in(p, &pal, popup_area((w, h)));
    }
    // The colour picker and its card: over the page and the chrome, under the dialog and the toast. It is anchored to
    // the Colour row the page has just recorded, and placed inside the window every frame, so a resize or a scroll
    // under it moves it along.
    let picking = pick.is_some() && dialog_is_none && leaving.is_none();
    if let Some(pk) = pick.as_mut().filter(|_| picking) {
        if let Some(anchor) = env.hits.rect_of(Hit::Open(Setting::Colour)) {
            pk.set_anchor(anchor);
        }
        let card = pk.place(popup_area((w, h)));
        let t = env
            .anim
            .presence("colour-picker", true, fluent::ENTER, fluent::FADE);
        if t > 0.0 {
            pk.draw(p, &pal, t);
            // Nothing under the card is there to be hit while it is open.
            env.hits.add(Hit::Blocked, card);
        }
    }
    // A device control that writes nothing says why, under the pointer. The toolkit's tooltip holds the wait (counted
    // from the pointer coming to rest) and what shows; the tip fades in and out.
    let tips_allowed = dialog_is_none && leaving.is_none() && menu.is_none() && pick.is_none();
    let over = input
        .pointer
        .filter(|_| tips_allowed)
        .and_then(|(x, y)| tip_over(&env.hits, x, y));
    let at = input
        .pointer
        .unwrap_or((f32::NEG_INFINITY, f32::NEG_INFINITY));
    tip.hover_at(input.now, over, at);
    tip.tick(input.now);
    let showing = tip.shown().map(|o| (o.anchor, o.text.clone()));
    let is_showing = showing.is_some();
    if let Some((anchor, text)) = &showing {
        *tip_last = Some((*anchor, text.clone()));
    }
    let t = env
        .anim
        .presence("tip", is_showing, fluent::ENTER, fluent::FADE);
    if t > 0.0 {
        if let Some((anchor, text)) = &tip_last {
            Bubble::tip(p, text, *anchor, w, h).draw_presence(p, &pal, t);
        }
    } else {
        *tip_last = None;
    }
    // The toasts, newest at the bottom: a stack, each leaving on its own timer, the ones above it lifting into the
    // room it leaves. It is on top of everything, the dialog too.
    for s in toasts.frame(env.anim, "toasts", |_| TOAST_H, TOAST_GAP) {
        let mut b = Bubble::toast(p, &s.item.text, main);
        b.rect.y -= s.lift;
        b.draw_presence(p, &pal, s.t);
    }
    FrameOut {
        hits: env.hits,
        chrome: vec![title, side],
        max_scroll,
        view,
    }
}

// ---- title row ---------------------------------------------------------------------------------

fn draw_title(p: &mut Painter, env: &mut Env, input: &FrameIn, r: Rect) {
    let pal = chrome_pal(&env.pal, input.frosted);
    p.fill(r, pal.title);
    // The mark: a ring with a dot.
    p.rbox(Rect::new(14.0, 12.0, 16.0, 16.0), 8.0, pal.accent);
    p.rbox(Rect::new(16.5, 14.5, 11.0, 11.0), 5.5, pal.title);
    p.rbox(Rect::new(19.0, 17.0, 6.0, 6.0), 3.0, pal.muted);
    let mut x = 40.0;
    x += p.text(
        Face::Semibold,
        12.5,
        x,
        r.cy(),
        "SoundBlaster X G6",
        pal.text,
        260.0,
    );
    let fw = input
        .model
        .status
        .firmware
        .as_deref()
        .filter(|_| input.model.have_status);
    if let Some(fw) = fw
        && r.w > 620.0
    {
        x += 10.0;
        x += p.text(
            Face::Ui,
            12.0,
            x,
            r.cy(),
            &format!("firmware {fw}"),
            pal.muted,
            240.0,
        );
    }
    if matches!(input.model.conn, Conn::Away(_)) {
        x += 10.0;
        kit::badge(p, &pal, x, r.cy(), "Not connected", Tone::Warn);
    }

    // Caption buttons from the right: close, maximise, minimise.
    for (n, kind, hit) in [
        (0, WinButton::Close, Hit::Close),
        (1, WinButton::Max, Hit::Max),
        (2, WinButton::Min, Hit::Min),
    ] {
        let b = chrome::win_button_rect(r.right(), n);
        caption_button(p, env, &pal, kind, b, hit, input.maximized);
        env.hits.add(hit, b);
    }
    // The settings button, left of them.
    let b = Rect::new(r.right() - 3.0 * chrome::WIN_BTN_W - 38.0, 5.0, 32.0, 30.0);
    let hot = env.fade(("hover", Hit::Settings), env.is_hover(Hit::Settings));
    let pressed = env.is_pressed(Hit::Settings) && env.is_hover(Hit::Settings);
    if let Some(fill) = chrome::button_fill(&pal, true, pressed, false, false) {
        fill_fade(p, b, 6.0, fill, hot);
    }
    p.icon(icons::SLIDERS, 15.0, b.cx(), b.cy(), pal.text);
    env.hits.add(Hit::Settings, b);
}

/// A rounded box at `alpha` of its colour; at 1 it is the plain box and at 0 nothing.
fn fill_fade(p: &mut Painter, r: Rect, radius: f32, c: mdshared_ui::gfx::Rgb, alpha: f32) {
    if alpha >= 1.0 {
        p.rbox(r, radius, c);
    } else if alpha > 0.0 {
        p.rbox_a(r, radius, c, alpha);
    }
}

/// One of the window's caption buttons, as `chrome::draw_win_button` draws it but with the hover fading in and out
/// (83 ms). The close button's red and its white glyph fade together.
fn caption_button(
    p: &mut Painter,
    env: &mut Env,
    pal: &mdshared_ui::theme::Palette,
    kind: WinButton,
    b: Rect,
    hit: Hit,
    maximized: bool,
) {
    let g = match kind {
        WinButton::Close => glyph::WIN_CLOSE,
        WinButton::Max if maximized => glyph::WIN_RESTORE,
        WinButton::Max => glyph::WIN_MAX,
        WinButton::Min => glyph::WIN_MIN,
    };
    let hot = env.fade(("hover", hit), env.is_hover(hit));
    let pressed = env.is_pressed(hit) && env.is_hover(hit);
    let mut ink = pal.text;
    if hot > 0.0 {
        let fill = match kind {
            WinButton::Close if pressed => mix(pal.danger, [0, 0, 0], 0.15),
            WinButton::Close => pal.danger,
            _ if pressed => pal.press,
            _ => pal.hover,
        };
        if hot >= 1.0 {
            p.fill(b, fill);
        } else {
            p.fill_a(b, fill, hot);
        }
        if kind == WinButton::Close {
            ink = mix(pal.text, [255, 255, 255], hot);
        }
    }
    p.icon(g, 10.0, b.cx(), b.cy(), ink);
}

// ---- sidebar -----------------------------------------------------------------------------------

fn draw_sidebar(p: &mut Painter, env: &mut Env, input: &FrameIn, r: Rect) {
    let live = env.pal;
    // Over a backdrop the chrome's secondary text is the full text colour, as in the mockup.
    let pal = chrome_pal(&live, input.frosted);
    let m = input.model;
    p.fill(r, live.pane);
    let px = p.px().max(1.0);
    p.fill(Rect::new(r.right() - px, r.y, px, r.h), live.line);
    let x = r.x + 14.0;
    let inner = r.w - 28.0 - px;
    let mut y = r.y + 14.0;

    // Name and state: a lamp, the name at 14 px and what the G6 is doing under it.
    let (dot, sub, sub_color) = match &m.conn {
        Conn::Connected => ([0x3f, 0xb9, 0x50], "USB DAC and amp", pal.muted),
        Conn::Reading => (pal.faint, "Reading the G6…", pal.muted),
        Conn::Away(_) => (pal.warn, "Not connected", pal.warn),
    };
    kit::led(p, x + 4.0, y + 18.0, dot);
    p.text(
        Face::Semibold,
        14.0,
        x + 18.0,
        y + 9.0,
        "SoundBlaster X G6",
        pal.text,
        inner - 18.0,
    );
    p.text(
        Face::Ui,
        12.0,
        x + 18.0,
        y + 27.0,
        sub,
        sub_color,
        inner - 18.0,
    );
    y += 36.0 + 10.0;

    // Output: two equal halves, the one in use in the accent. A click on the other switches (--set-output).
    let out_r = Rect::new(x, y, inner, kit::SEG_H);
    let bind = Bind::Live(Setting::Output);
    let out_hits: Vec<Hit> = (0..facts::OUTPUTS.len())
        .map(|k| kit::seg_hit(env, bind, k))
        .collect();
    let hovered = out_hits
        .iter()
        .position(|h| h.acts() && env.hover == Some(*h));
    let rects = kit::draw_seg_with(
        p,
        &live,
        Some((&mut *env.anim, Id::new("side-output"))),
        out_r,
        &facts::OUTPUTS,
        facts::output_index(&m.status),
        hovered,
        kit::SegKind {
            small: false,
            fill: true,
        },
    );
    for (hit, rc) in out_hits.into_iter().zip(rects) {
        env.hits.add(hit, rc);
    }
    y += kit::SEG_H + 10.0;

    // Master volume: it is Core Audio's, so it is a dash until Windows gives it.
    let master = m.mixer.as_ref().map(|v| v.master);
    let muted = master.and_then(|s| s.muted) == Some(true);
    p.icon(
        if muted { icons::MUTED } else { icons::SPEAKER },
        15.0,
        x + 14.0,
        y + 14.0,
        pal.text,
    );
    let vol = master.and_then(|s| s.volume);
    let reading = vol.map_or(DASH.to_string(), |v| v.to_string());
    let val_w = 40.0;
    let track = Rect::new(x + 30.0, y, inner - 30.0 - val_w, 28.0);
    let frac = vol.map(|v| {
        env.anim
            .value("side-volume", f32::from(v) / 100.0, fluent::MOVE)
    });
    kit::slider(p, &live, track, frac, false, live.pane);
    env.hits
        .add(Hit::Inert(Why::Windows), Rect::new(x, y, inner, 28.0));
    p.text_right(
        Face::Ui,
        kit::SIZE_VALUE,
        x + inner,
        y + 14.0,
        &reading,
        pal.muted,
    );
    y += 28.0 + 10.0;

    // Speaker setup: Windows' setting of the output in use, on one line with its label. One that isn't stereo, 5.1 or
    // 7.1 is said in the label and nothing is chosen.
    let layout = m.mixer.as_ref().and_then(|v| v.speaker_layout.as_deref());
    let chosen = m.mixer.as_ref().and_then(|v| v.layout_index());
    let caption = match (layout, chosen) {
        (Some(l), None) => format!("Speaker setup · {l}"),
        _ => "Speaker setup".to_string(),
    };
    let sw = kit::seg_width(p, &facts::LAYOUTS, true).ceil();
    // The label shares the line with the control when it fits beside it; otherwise it has a line above.
    let beside = p.measure(Face::Ui, 12.0, &caption) <= inner - sw - 8.0;
    if !beside {
        p.text(Face::Ui, 12.0, x, y + 8.0, &caption, pal.muted, inner);
        y += 18.0;
    }
    let lay_r = Rect::new(x + inner - sw, y, sw, kit::SEG_H_SM);
    if beside {
        p.text(
            Face::Ui,
            12.0,
            x,
            y + kit::SEG_H_SM / 2.0,
            &caption,
            pal.muted,
            inner - sw - 8.0,
        );
    }
    kit::draw_seg_with(
        p,
        &live,
        Some((&mut *env.anim, Id::new("side-layout"))),
        lay_r,
        &facts::LAYOUTS,
        chosen,
        None,
        kit::SegKind {
            small: true,
            fill: false,
        },
    );
    env.hits.add(Hit::Inert(Why::SpeakerSetup), lay_r);
    y += kit::SEG_H_SM + 10.0;

    // The pages: 34 high, 2 apart, the current one on the `on` fill with the accent mark. The mark and its fill slide
    // from the page that was current to the one that is (167 ms), kept relative to the list so a sidebar that
    // reflows (the speaker caption wrapping) does not set them sliding; hover fades in and out (83 ms).
    y += 6.0;
    let list_top = y;
    let mut rows = Vec::new();
    for page in Page::ALL {
        if page == Page::Chain {
            y += 4.0;
            p.fill(Rect::new(r.x + 8.0, y, r.w - 16.0 - px, px), live.line);
            y += 5.0;
        }
        rows.push((page, Rect::new(r.x + 6.0, y, r.w - 12.0 - px, NAV_H)));
        y += NAV_H + 2.0;
    }
    for &(page, row) in &rows {
        let hit = Hit::Nav(page);
        let selected = page == input.page;
        let hot = env.fade(("nav-hover", page), env.is_hover(hit) && !selected);
        let fill = if env.is_pressed(hit) {
            live.press
        } else {
            live.hover
        };
        fill_fade(p, row, 5.0, fill, hot);
    }
    if let Some(&(_, row)) = rows.iter().find(|(pg, _)| *pg == input.page) {
        let rel = Rect::new(row.x - r.x, row.y - list_top, row.w, row.h);
        let at = env.anim.value("nav-selected", rel, fluent::MOVE);
        let at = Rect::new(r.x + at.x, list_top + at.y, at.w, at.h);
        p.rbox(at, 5.0, live.on);
        p.rbox(
            Rect::new(at.x, at.y + 7.0, 3.0, at.h - 14.0),
            1.5,
            live.accent,
        );
    }
    for (page, row) in rows {
        let hit = Hit::Nav(page);
        let lit = env.fade(("nav-ink", page), page == input.page);
        p.icon(
            page.icon(),
            15.0,
            row.x + 24.0,
            row.cy(),
            mix(pal.muted, live.accent, lit),
        );
        p.text(
            Face::Ui,
            13.0,
            row.x + 42.0,
            row.cy(),
            page.title(),
            mix(pal.muted, pal.text, lit),
            row.w - 52.0,
        );
        env.hits.add(hit, row);
    }
}

// ---- the page ----------------------------------------------------------------------------------

/// Draws the connection banner and the page. Returns the viewport and how far the page can scroll.
fn draw_main(p: &mut Painter, env: &mut Env, input: &FrameIn, main: Rect) -> (Rect, f32) {
    let pal = env.pal;
    let mut top = main.y;
    // The banner doesn't scroll: it is the first thing seen when the G6 is away.
    match &input.model.conn {
        Conn::Connected => {}
        Conn::Reading => {
            let r = Rect::new(main.x + PAD, top + 12.0, main.w - 2.0 * PAD, 36.0);
            p.rbox(r, 6.0, mix(pal.popup, pal.text, 0.05));
            p.icon(glyph::INFO, 15.0, r.x + 20.0, r.cy(), pal.accent);
            p.text(
                Face::Ui,
                12.5,
                r.x + 40.0,
                r.cy(),
                "Reading the G6…",
                pal.text,
                r.w - 56.0,
            );
            top = r.bottom() + 4.0;
        }
        Conn::Away(why) => {
            let r = Rect::new(main.x + PAD, top + 12.0, main.w - 2.0 * PAD, 36.0);
            p.shadow(r, 6.0, pal.dark);
            p.rbox(r, 6.0, pal.warn_bg);
            p.icon(glyph::WARN, 15.0, r.x + 20.0, r.cy(), pal.warn);
            let bw = kit::button_w(p, "Retry");
            let b = Rect::new(r.right() - bw - 8.0, r.cy() - 12.0, bw, 24.0);
            let text = format!("G6 not connected. {why} These are the last settings read from it.");
            let shown = if input.model.have_status {
                text
            } else {
                format!("G6 not connected. {why}")
            };
            p.text(
                Face::Ui,
                12.5,
                r.x + 40.0,
                r.cy(),
                &shown,
                pal.warn,
                b.x - r.x - 52.0,
            );
            env.button(p, &pal, b, "Retry", Hit::Retry);
            env.hits.add(Hit::Retry, b);
            top = r.bottom() + 4.0;
        }
    }
    let view = Rect::new(main.x, top, main.w, (main.bottom() - top).max(0.0));
    let old = p.clip(view);
    let hits_from = env.hits.len();
    let x = view.x + PAD;
    let w = (view.w - 2.0 * PAD).max(100.0);
    let mut y = view.y + 18.0 - input.scroll;

    // Header: the title, what the page is for beside it on the same line when it fits, and the badges at the right
    // (the mockup's `.phead`: h2 17 px, the description 11.5 px, 12 px apart).
    let page = input.page;
    let cy = y + 2.0 + 11.5;
    let tw = p.measure(Face::Semibold, kit::SIZE_H2, page.title());
    p.text(
        Face::Semibold,
        kit::SIZE_H2,
        x,
        cy,
        page.title(),
        pal.text,
        w,
    );
    let tags = pages::tags(page, input.model);
    let tags_w: f32 = tags
        .iter()
        .map(|(t, _)| kit::badge_w(p, t) + 6.0)
        .sum::<f32>()
        - 6.0;
    let mut tag_x = x + w - tags_w;
    for (text, tone) in &tags {
        tag_x += kit::badge(p, &pal, tag_x, cy, text, *tone) + 6.0;
    }
    let about = page.about();
    let aw = p.measure(Face::Ui, kit::SIZE_SMALL, about);
    let room = w - tw - 12.0 - tags_w - 12.0;
    if aw <= room {
        p.text(
            Face::Ui,
            kit::SIZE_SMALL,
            x + tw + 12.0,
            cy + 1.5,
            about,
            pal.muted,
            room,
        );
        y += 2.0 + 23.0 + 12.0;
    } else {
        // Too long for the line: it goes under the title, wrapped.
        let lines = kit::wrap(p, Face::Ui, kit::SIZE_SMALL, about, w);
        for (i, l) in lines.iter().enumerate() {
            p.text(
                Face::Ui,
                kit::SIZE_SMALL,
                x,
                y + 2.0 + 23.0 + 6.0 + 15.0 * i as f32,
                l,
                pal.muted,
                w,
            );
        }
        y += 2.0 + 23.0 + 6.0 + 15.0 * lines.len() as f32 + 10.0;
    }

    for n in pages::notices(page, input.model) {
        let r = Rect::new(x, y, w, 36.0);
        let (fill, ink, g) = if n.warn {
            (pal.warn_bg, pal.warn, glyph::WARN)
        } else {
            (mix(pal.popup, pal.text, 0.05), pal.text, glyph::INFO)
        };
        p.rbox(r, 6.0, fill);
        p.icon(g, 15.0, r.x + 20.0, r.cy(), ink);
        p.text(Face::Ui, 12.5, r.x + 40.0, r.cy(), &n.text, ink, r.w - 56.0);
        y += 48.0;
    }

    let start = y;
    env.begin_scope(page);
    let used = if page == Page::Chain {
        pages::draw_chain(p, env, Rect::new(x, y, w, 0.0))
    } else {
        let cards = pages::cards(page, input.model, input.prefs, input.meters, input.now);
        kit::flow(p, env, Rect::new(x, y, w, 0.0), &cards)
    };
    let content = (start + used + 24.0) - (view.y - input.scroll);
    let max_scroll = (content - view.h).max(0.0);
    p.restore(old);
    // What scrolled out of the view can't be hit: not the page's controls, and not over the banner or the title row.
    env.hits.clip_from(hits_from, view);
    if let Some((track, thumb)) =
        scroll::scrollbar(view, content, view.h, input.scroll.min(max_scroll))
    {
        scroll::draw_scrollbar(p, &pal, track, thumb, false);
    }
    (view, max_scroll)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::Toast;
    use crate::levels::NO_METER;
    use crate::sample::Sample;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    use mdshared_ui::theme::{DARK, LIGHT};
    use std::time::Duration;

    fn render(
        page: Page,
        sample: Sample,
        pal: mdshared_ui::theme::Palette,
        size: (f32, f32),
    ) -> (Bitmap, FrameOut) {
        render_at(page, sample, pal, size, 0.0)
    }

    fn render_at(
        page: Page,
        sample: Sample,
        pal: mdshared_ui::theme::Palette,
        size: (f32, f32),
        scroll: f32,
    ) -> (Bitmap, FrameOut) {
        render_meters(
            page,
            sample,
            pal,
            size,
            scroll,
            &Meters::none(),
            Instant::now(),
        )
    }

    /// A frame with a given meter state and clock, for the Mixer's strips.
    fn render_meters(
        page: Page,
        sample: Sample,
        pal: mdshared_ui::theme::Palette,
        size: (f32, f32),
        scroll: f32,
        meters: &Meters,
        now: Instant,
    ) -> (Bitmap, FrameOut) {
        let model = sample.model();
        let prefs = Prefs::default();
        let mut bmp = Bitmap::new(size.0 as u32, size.1 as u32);
        let mut gfx = Gfx::new();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let input = FrameIn {
            model: &model,
            prefs: &prefs,
            pal,
            page,
            size,
            scroll,
            maximized: false,
            hover: None,
            pointer: None,
            now,
            pressed: None,
            dragging: None,
            frosted: false,
            meters,
        };
        let out = draw_frame(&mut p, &input, None, &mut Fx::still());
        (bmp, out)
    }

    /// The Mixer's Playback strip draws a level with its held peak when there is a source, and a flat track that says
    /// "no meter" when there is none: the rest of the page is the same picture.
    #[test]
    fn the_mixer_draws_a_level_only_when_there_is_a_meter() {
        let now = Instant::now();
        let live = Meters::busy(now);
        let (with, _) = render_meters(
            Page::Mixer,
            Sample::Busy,
            DARK,
            (1100.0, 720.0),
            0.0,
            &live,
            now,
        );
        let none = Meters::none();
        let (without, _) = render_meters(
            Page::Mixer,
            Sample::Busy,
            DARK,
            (1100.0, 720.0),
            0.0,
            &none,
            now,
        );
        let mut changed = 0;
        let mut lowest = 0;
        for (i, (a, b)) in with
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(without.pixels.as_chunks::<4>().0)
            .enumerate()
        {
            if a != b {
                changed += 1;
                lowest = lowest.max(i / with.width as usize);
            }
        }
        assert!(changed > 100, "{changed} pixels changed");
        assert!(
            changed < 6000,
            "{changed} pixels changed: more than the meters"
        );
        assert!(lowest > 150, "a change at y {lowest} is above the strips");
        // The words are the plan's, and only a strip without a source draws them.
        assert_eq!(NO_METER, "no meter");
    }

    #[test]
    fn every_page_draws_in_both_themes_without_a_device() {
        for pal in [LIGHT, DARK] {
            for page in Page::ALL {
                for sample in [Sample::Live, Sample::Busy, Sample::Away] {
                    let (bmp, out) = render(page, sample, pal, (1100.0, 720.0));
                    assert_eq!(bmp.width, 1100);
                    // Every page and the settings button can be hit.
                    for pg in Page::ALL {
                        assert!(out.hits.rect_of(Hit::Nav(pg)).is_some());
                    }
                    assert!(out.hits.rect_of(Hit::Settings).is_some());
                    assert_eq!(
                        out.hits.rect_of(Hit::Retry).is_some(),
                        sample == Sample::Away
                    );
                }
            }
        }
    }

    /// The Settings page scrolled through: its controls (the appearance buttons, low on the page) pass behind the title
    /// row and the banner. They must not take the click there.
    #[test]
    fn scrolled_out_controls_do_not_cover_the_title_row_or_the_banner() {
        let size = (760.0, 600.0);
        for (page, sample) in [Page::Settings, Page::Mixer, Page::Playback]
            .into_iter()
            .flat_map(|pg| [Sample::Live, Sample::Away].map(move |s| (pg, s)))
        {
            let (_, top) = render(page, sample, LIGHT, size);
            assert!(
                top.max_scroll > 100.0,
                "the page has to scroll for this test"
            );
            let mut at_title = 0;
            let mut s = 0.0;
            // Past the end too: the window clamps the scroll, the frame must not depend on it.
            while s <= top.max_scroll + 700.0 {
                let (_, out) = render_at(page, sample, LIGHT, size, s);
                // The title row, left of the window buttons.
                for x in (250..570).step_by(10) {
                    assert_eq!(
                        out.hits.at(x as f32, 20.0),
                        Some(Hit::Drag),
                        "x {x}, scroll {s}"
                    );
                    at_title += 1;
                }
                if let Some(retry) = out.hits.rect_of(Hit::Retry) {
                    assert_eq!(
                        out.hits.at(retry.cx(), retry.cy()),
                        Some(Hit::Retry),
                        "scroll {s}"
                    );
                    // The page's view starts below the banner, so nothing of the page is under it.
                    assert!(out.view.y >= retry.bottom());
                    for x in (retry.x as i32..retry.right() as i32 - 60).step_by(10) {
                        let hit = out.hits.at(x as f32, retry.cy());
                        assert!(
                            matches!(hit, None | Some(Hit::Retry)),
                            "{page:?} x {x}, scroll {s}: {hit:?}"
                        );
                    }
                }
                s += 40.0;
            }
            assert!(at_title > 0);
        }
    }

    #[test]
    fn a_read_only_control_is_inert_and_a_click_on_it_does_nothing() {
        let (_, out) = render(Page::Playback, Sample::Live, LIGHT, (1100.0, 720.0));
        assert!(!Hit::Inert(Why::Windows).acts() && !Hit::Drag.acts());
        assert!(Hit::Retry.acts() && Hit::Nav(Page::Sbx).acts() && Hit::Theme(0).acts());
        // The controls of the page are there to be found, as inert regions.
        let found = (0..1100).step_by(6).any(|x| {
            (60..720)
                .step_by(6)
                .any(|y| matches!(out.hits.at(x as f32, y as f32), Some(Hit::Inert(_))))
        });
        assert!(found);
    }

    /// The pixels of a Playback frame with the pointer at `at`, with or without the settings dialog open.
    fn frame_pixels(at: Option<(f32, f32)>, dialog: bool) -> Vec<u8> {
        let model = Sample::Live.model();
        let prefs = Prefs::default();
        let size = (1100.0, 720.0);
        let mut bmp = Bitmap::new(1100, 720);
        let mut gfx = Gfx::new();
        let mut form = dialog.then(|| crate::dialog::form_at(&prefs, &model, 0));
        let meters = Meters::none();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let input = FrameIn {
            model: &model,
            prefs: &prefs,
            pal: LIGHT,
            page: Page::Playback,
            size,
            scroll: 0.0,
            maximized: false,
            hover: None,
            pointer: at,
            now: Instant::now(),
            pressed: None,
            dragging: None,
            frosted: false,
            meters: &meters,
        };
        draw_frame(&mut p, &input, form.as_mut(), &mut Fx::still());
        bmp.pixels
    }

    #[test]
    fn the_read_only_tip_shows_over_a_control_and_never_over_the_dialog() {
        let (_, out) = render(Page::Playback, Sample::Live, LIGHT, (1100.0, 720.0));
        let at = (0..1100)
            .step_by(4)
            .flat_map(|x| (60..720).step_by(4).map(move |y| (x as f32, y as f32)))
            .find(|&(x, y)| matches!(out.hits.at(x, y), Some(Hit::Inert(_))))
            .expect("an inert control");
        assert_ne!(frame_pixels(Some(at), false), frame_pixels(None, false));
        assert_eq!(frame_pixels(Some(at), true), frame_pixels(None, true));
    }

    #[test]
    fn the_smallest_window_draws_every_page() {
        for page in Page::ALL {
            let (_, out) = render(page, Sample::Busy, LIGHT, (760.0, 600.0));
            assert!(out.view.h > 0.0);
        }
    }

    #[test]
    fn the_sidebar_is_one_colour_pane_and_the_page_another() {
        let (bmp, _) = render(Page::Sbx, Sample::Live, LIGHT, (1100.0, 720.0));
        let at = |x: usize, y: usize| {
            let o = (y * bmp.width as usize + x) * 4;
            [bmp.pixels[o + 2], bmp.pixels[o + 1], bmp.pixels[o]]
        };
        assert_eq!(at(5, 700), LIGHT.pane);
        assert_eq!(at(1090, 700), LIGHT.page);
        assert_eq!(at(500, 5), LIGHT.title);
    }

    // ---- motion, on a clock moved by hand ----

    use mdshared_ui::anim::document::TIP_DELAY;
    use mdshared_ui::anim::{Clock, ManualClock};

    /// A window drawn frame by frame on a clock that stands still until it is moved.
    struct Rig {
        bmp: Bitmap,
        gfx: Gfx,
        fx: Fx,
        clock: ManualClock,
        model: crate::model::Model,
        prefs: Prefs,
        view: Rect,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                bmp: Bitmap::new(1100, 720),
                gfx: Gfx::new(),
                fx: Fx::new(),
                clock: ManualClock::new(),
                model: Sample::Live.model(),
                prefs: Prefs::default(),
                view: Rect::default(),
            }
        }

        /// A frame `ms` after the last, with the pointer's hover at `hover` and no pointer; the next moment the loop
        /// wants a frame, as the app's `draw` works it out.
        fn frame(
            &mut self,
            page: Page,
            ms: u64,
            hover: Option<Hit>,
        ) -> (Option<Instant>, FrameOut) {
            self.frame_pointer(page, ms, hover, None)
        }

        /// The same with the pointer at `pointer`: the tooltip's delay counts from where it comes to rest.
        fn frame_pointer(
            &mut self,
            page: Page,
            ms: u64,
            hover: Option<Hit>,
            pointer: Option<(f32, f32)>,
        ) -> (Option<Instant>, FrameOut) {
            self.clock.advance_ms(ms);
            let now = self.clock.now();
            self.fx.anim.begin_frame(now);
            let meters = Meters::none();
            let input = FrameIn {
                model: &self.model,
                prefs: &self.prefs,
                pal: LIGHT,
                page,
                size: (1100.0, 720.0),
                scroll: 0.0,
                maximized: false,
                hover,
                pointer,
                now,
                pressed: None,
                dragging: None,
                frosted: false,
                meters: &meters,
            };
            let mut p = Painter::new(&mut self.bmp, &mut self.gfx, 1.0);
            let out = draw_frame(&mut p, &input, None, &mut self.fx);
            self.view = out.view;
            (self.fx.wake(), out)
        }

        /// What the window does when a page is clicked: the picture of the old one is kept, then the page changes.
        fn switch(&mut self) {
            let p = Painter::new(&mut self.bmp, &mut self.gfx, 1.0);
            self.fx.begin_page(&p, self.view);
        }

        /// Frames until nothing is asked for; how many it took.
        fn settle(&mut self, page: Page) -> usize {
            for n in 1..200 {
                if self.frame(page, 16, None).0.is_none() {
                    return n;
                }
            }
            panic!("it never settles");
        }
    }

    #[test]
    fn a_window_at_rest_asks_for_no_frame_however_long_it_sleeps() {
        let mut rig = Rig::new();
        assert!(rig.frame(Page::Sbx, 0, None).0.is_none());
        assert!(rig.frame(Page::Sbx, 3_600_000, None).0.is_none());
        assert!(!rig.fx.anim.animating());
        assert!(!rig.fx.page.running());
    }

    #[test]
    fn a_page_switch_draws_frames_then_settles_on_the_page_at_rest() {
        let mut rig = Rig::new();
        rig.frame(Page::Sbx, 0, None);
        let old = rig.bmp.pixels.clone();
        rig.switch();
        assert!(rig.fx.page.running());
        // The frame that draws the new page after the switch already blends the old one in.
        let (more, _) = rig.frame(Page::Playback, 16, None);
        assert!(more.is_some());
        let first = rig.bmp.pixels.clone();
        let (more, _) = rig.frame(Page::Playback, 80, None);
        assert!(more.is_some());
        assert_ne!(rig.bmp.pixels, first);
        assert_ne!(rig.bmp.pixels, old);
        let frames = rig.settle(Page::Playback);
        assert!((4..=40).contains(&frames), "{frames} frames");
        assert!(!rig.fx.page.running());
        // Settled, the window is exactly the page drawn with nothing in flight.
        let mut still = Rig::new();
        still.fx = Fx::still();
        still.frame(Page::Playback, 0, None);
        assert_eq!(rig.bmp.pixels, still.bmp.pixels);
    }

    #[test]
    fn clicking_through_the_pages_quickly_never_stops_halfway() {
        let mut rig = Rig::new();
        rig.frame(Page::Sbx, 0, None);
        let mut page = Page::Sbx;
        for next in [
            Page::Equalizer,
            Page::Playback,
            Page::Mixer,
            Page::Sbx,
            Page::Lighting,
        ] {
            rig.switch();
            page = next;
            // A few frames of the one switch, then the next click arrives while it still runs.
            for _ in 0..3 {
                rig.frame(page, 16, None);
            }
            assert!(rig.fx.page.running());
        }
        rig.settle(page);
        assert!(!rig.fx.page.running());
        let mut still = Rig::new();
        still.fx = Fx::still();
        still.frame(page, 0, None);
        assert_eq!(rig.bmp.pixels, still.bmp.pixels);
    }

    /// The selected row's place in the animator: y relative to the list.
    fn highlight_y(rig: &Rig) -> f32 {
        rig.fx.anim.get::<Rect>("nav-selected").expect("drawn").y
    }

    #[test]
    fn the_nav_highlight_moves_down_to_its_row_without_going_back() {
        let mut rig = Rig::new();
        rig.frame(Page::Sbx, 0, None);
        let from = highlight_y(&rig);
        rig.frame(Page::Playback, 16, None);
        let mut ys = vec![from, highlight_y(&rig)];
        for _ in 0..40 {
            if rig.frame(Page::Playback, 16, None).0.is_none() {
                break;
            }
            ys.push(highlight_y(&rig));
        }
        ys.push(highlight_y(&rig));
        let to = *ys.last().unwrap();
        assert!(to > from + 30.0, "{from} -> {to}");
        assert!(ys.windows(2).all(|w| w[1] >= w[0] - 1e-3), "{ys:?}");
        assert!(ys.iter().any(|&y| y > from + 1.0 && y < to - 1.0), "{ys:?}");
        // It ends on the row the page's own frame puts it on.
        let (_, out) = rig.frame(Page::Playback, 16, None);
        let row = out.hits.rect_of(Hit::Nav(Page::Playback)).unwrap();
        let first = out.hits.rect_of(Hit::Nav(Page::Sbx)).unwrap();
        assert!((to - (row.y - first.y)).abs() < 1e-3);
    }

    #[test]
    fn reduced_motion_makes_a_switch_and_the_highlight_land_at_once() {
        let mut rig = Rig::new();
        rig.fx.anim.set_system_reduced(true);
        rig.frame(Page::Sbx, 0, None);
        rig.switch();
        assert!(!rig.fx.page.running());
        let (more, out) = rig.frame(Page::Mixer, 16, Some(Hit::Nav(Page::Equalizer)));
        assert!(more.is_none());
        let row = out.hits.rect_of(Hit::Nav(Page::Mixer)).unwrap();
        let first = out.hits.rect_of(Hit::Nav(Page::Sbx)).unwrap();
        assert!((highlight_y(&rig) - (row.y - first.y)).abs() < 1e-3);
        // And the same picture as a window that never animates.
        let mut still = Rig::new();
        still.fx = Fx::still();
        still.frame(Page::Mixer, 0, Some(Hit::Nav(Page::Equalizer)));
        assert_eq!(rig.bmp.pixels, still.bmp.pixels);
    }

    #[test]
    fn hovering_a_nav_row_fades_in_and_the_toast_rises_and_goes() {
        let mut rig = Rig::new();
        rig.frame(Page::Sbx, 0, None);
        let rest = rig.bmp.pixels.clone();
        rig.frame(Page::Sbx, 16, Some(Hit::Nav(Page::Mixer)));
        let (more, _) = rig.frame(Page::Sbx, 32, Some(Hit::Nav(Page::Mixer)));
        assert!(more.is_some());
        let mid = rig.bmp.pixels.clone();
        assert_ne!(mid, rest);
        rig.settle(Page::Sbx);
        assert_ne!(rig.bmp.pixels, mid);
        // The toast: in over 167 ms, out over 167 ms, then nothing to wait for.
        let until = rig.clock.now() + crate::fx::TOAST_FOR;
        rig.fx
            .announce(Toast::plain("Status read from the G6."), until);
        assert!(rig.frame(Page::Sbx, 16, None).0.is_some());
        // Frames until its entrance has played; its own time is then the next wake.
        let mut due = None;
        for _ in 0..30 {
            due = rig.frame(Page::Sbx, 16, None).0;
            if due == Some(until) {
                break;
            }
        }
        assert_eq!(due, Some(until), "the toast's time is the next wake");
        let shown = rig.bmp.pixels.clone();
        rig.fx.toasts.hide_all();
        assert!(rig.frame(Page::Sbx, 16, None).0.is_some());
        rig.settle(Page::Sbx);
        assert_ne!(rig.bmp.pixels, shown);
        assert!(rig.fx.toasts.is_empty());
    }

    #[test]
    fn a_tip_waits_for_the_pointer_to_rest_and_not_after_a_press() {
        let mut rig = Rig::new();
        let (_, out) = rig.frame(Page::Playback, 0, None);
        let (_, r) = out
            .hits
            .iter()
            .find(|(h, _)| matches!(h, Hit::Inert(_)))
            .expect("a read-only control to rest on");
        let at = (r.cx(), r.cy());
        let t0 = rig.clock.now();
        let ms = TIP_DELAY.as_millis() as u64;
        rig.frame_pointer(Page::Playback, 0, None, Some(at));
        assert!(rig.fx.tip.shown().is_none());
        assert_eq!(rig.fx.tip.deadline(), Some(t0 + TIP_DELAY));
        // A move on the same control starts the wait again: it counts from the pointer coming to rest.
        let moved = (at.0 + 2.0, at.1);
        rig.frame_pointer(Page::Playback, 300, None, Some(moved));
        assert_eq!(
            rig.fx.tip.deadline(),
            Some(t0 + Duration::from_millis(300) + TIP_DELAY)
        );
        rig.frame_pointer(Page::Playback, ms - 1, None, Some(moved));
        assert!(rig.fx.tip.shown().is_none(), "not a moment early");
        rig.frame_pointer(Page::Playback, 1, None, Some(moved));
        assert_eq!(rig.fx.tip.shown().map(|o| o.anchor), Some(r));
        // Pressed: it goes, and it stays away while the pointer rests on that control.
        rig.fx.tip.press(Some(r));
        rig.frame_pointer(Page::Playback, 16, None, Some(moved));
        assert!(rig.fx.tip.shown().is_none());
        rig.frame_pointer(Page::Playback, 4 * ms, None, Some(moved));
        assert!(rig.fx.tip.shown().is_none());
        // Away from it and back: it waits the whole delay again.
        rig.frame_pointer(Page::Playback, 16, None, Some((5.0, 400.0)));
        rig.frame_pointer(Page::Playback, 16, None, Some(at));
        assert!(rig.fx.tip.shown().is_none());
        rig.frame_pointer(Page::Playback, ms, None, Some(at));
        assert!(rig.fx.tip.shown().is_some());
    }

    #[test]
    fn toasts_expire_and_the_window_sleeps_when_nothing_moves() {
        let mut rig = Rig::new();
        rig.frame(Page::Sbx, 0, None);
        let t0 = rig.clock.now();
        let until = t0 + crate::fx::TOAST_FOR;
        rig.fx
            .announce(Toast::about(Setting::Sbx, "SBX turned on."), until);
        let mut due = None;
        for _ in 0..30 {
            due = rig.frame(Page::Sbx, 16, None).0;
            if due == Some(until) {
                break;
            }
        }
        assert_eq!(due, Some(until));
        // Its time comes: it goes, its exit plays, and then nothing waits at all.
        rig.clock
            .advance_ms(crate::fx::TOAST_FOR.as_millis() as u64);
        assert!(rig.fx.toasts.tick(rig.clock.now()));
        let frames = rig.settle(Page::Sbx);
        assert!(frames < 30, "{frames} frames");
        assert!(rig.fx.toasts.is_empty());
        assert!(rig.frame(Page::Sbx, 16, None).0.is_none());
    }
}
