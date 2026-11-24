//! One frame of the window: the title row, the sidebar, the page, and the dialog over them.
//!
//! The frame is drawn whole every time, into the painter it is given, in logical px. The title row and the sidebar
//! are the chrome that a backdrop shows through: after each is drawn it is marked for [`mdshared_ui::frost`], and
//! the areas come back in [`FrameOut`].

use mdshared_ui::chrome::{self, TITLE_H, WinButton};
use mdshared_ui::frost;
use mdshared_ui::gfx::{Face, Painter, Rect, mix};
use mdshared_ui::glyph;
use mdshared_ui::overlay::Bubble;
use mdshared_ui::pane;
use mdshared_ui::scroll;
use mdshared_ui::segments;
use mdshared_ui::settings::pages::Form;

use crate::facts::{self, DASH};
use crate::icons;
use crate::kit::{self, Env, Hit, Hits, Tone, dimmed};
use crate::model::{Conn, Model, Page};
use crate::pages;
use crate::prefs::Prefs;

pub const SIDE_W: f32 = 236.0;
const PAD: f32 = 20.0;
const NAV_H: f32 = 34.0;

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
    pub pressed: Option<Hit>,
    pub toast: Option<&'a str>,
    /// Draw the window's own caption buttons (not in `--shot` of a bare page? always true today).
    pub frosted: bool,
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

pub fn draw_frame(p: &mut Painter, input: &FrameIn, dialog: Option<&mut Form>) -> FrameOut {
    let (w, h) = input.size;
    let pal = input.pal;
    let mut env = Env {
        pal,
        model: input.model,
        hits: Hits::default(),
        hover: input.hover,
        pressed: input.pressed,
    };
    p.fill(Rect::new(0.0, 0.0, w, h), pal.page);

    // The title row first in the hit list, so everything drawn later is on top of it.
    let title = Rect::new(0.0, 0.0, w, TITLE_H);
    env.hits.add(Hit::Drag, title);

    let main = Rect::new(
        SIDE_W,
        TITLE_H,
        (w - SIDE_W).max(0.0),
        (h - TITLE_H).max(0.0),
    );
    let (view, max_scroll) = draw_main(p, &mut env, input, main);

    let side = Rect::new(0.0, TITLE_H, SIDE_W, (h - TITLE_H).max(0.0));
    draw_sidebar(p, &mut env, input, side);
    if input.frosted {
        frost::mark(p.bmp, p.dev_rect(side));
    }
    draw_title(p, &mut env, input, title);
    if input.frosted {
        frost::mark(p.bmp, p.dev_rect(title));
    }

    if let Some(t) = input.toast {
        Bubble::toast(p, t, main).draw(p, &pal);
    }
    if let Some(form) = dialog {
        form.draw(p, &pal, Rect::new(0.0, 0.0, w, h), false);
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
    let pal = env.pal;
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
        chrome::draw_win_button(
            p,
            &pal,
            kind,
            b,
            input.maximized,
            env.is_hover(hit),
            env.is_pressed(hit),
        );
        env.hits.add(hit, b);
    }
    // The settings button, left of them.
    let b = Rect::new(r.right() - 3.0 * chrome::WIN_BTN_W - 38.0, 5.0, 32.0, 30.0);
    let open = false;
    if let Some(fill) = chrome::button_fill(
        &pal,
        env.is_hover(Hit::Settings),
        env.is_pressed(Hit::Settings),
        false,
        open,
    ) {
        p.rbox(b, 6.0, fill);
    }
    p.icon(icons::SLIDERS, 15.0, b.cx(), b.cy(), pal.text);
    env.hits.add(Hit::Settings, b);
}

// ---- sidebar -----------------------------------------------------------------------------------

fn draw_sidebar(p: &mut Painter, env: &mut Env, input: &FrameIn, r: Rect) {
    let pal = env.pal;
    let m = input.model;
    pane::draw_head(p, &pal, r, "DEVICE");
    let px = p.px().max(1.0);
    p.fill(Rect::new(r.right() - px, r.y, px, r.h), pal.line);
    let dim = dimmed(&pal, pal.pane);
    let x = r.x + 14.0;
    let inner = r.w - 28.0;
    let mut y = r.y + pane::HEAD + 4.0;

    // Name and state.
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
        y + 10.0,
        "SoundBlaster X G6",
        pal.text,
        inner - 18.0,
    );
    p.text(
        Face::Ui,
        12.0,
        x + 18.0,
        y + 28.0,
        sub,
        sub_color,
        inner - 18.0,
    );
    y += 46.0;

    // Output.
    let out_r = Rect::new(x, y, inner, segments::SEGMENT_H);
    segments::draw_segments(
        p,
        &dim,
        out_r,
        &facts::OUTPUTS,
        facts::output_index(&m.status),
        None,
    );
    y += segments::SEGMENT_H + 10.0;

    // Master volume: it is Core Audio's, so it is a dash until that is read.
    let master = m.mixer.as_ref().map(|v| v.master);
    let muted = master.and_then(|s| s.muted) == Some(true);
    p.icon(
        if muted { icons::MUTED } else { icons::SPEAKER },
        15.0,
        x + 8.0,
        y + 14.0,
        dim.muted,
    );
    let vol = master.and_then(|s| s.volume);
    let reading = vol.map_or(DASH.to_string(), |v| v.to_string());
    let rw = 36.0;
    kit::slider(
        p,
        &dim,
        Rect::new(x + 24.0, y, inner - 24.0 - rw, 28.0),
        vol.map(|v| f32::from(v) / 100.0),
        false,
        pal.pane,
    );
    p.text_right(Face::Ui, 13.0, x + inner, y + 14.0, &reading, dim.text);
    y += 38.0;

    // Speaker setup: no read-back is known, so nothing is chosen.
    p.text(
        Face::Semibold,
        11.0,
        x,
        y + 6.0,
        "Speaker setup",
        pal.muted,
        inner,
    );
    y += 16.0;
    segments::draw_segments(
        p,
        &dim,
        Rect::new(x, y, inner, segments::SEGMENT_H),
        &facts::LAYOUTS,
        None,
        None,
    );
    y += segments::SEGMENT_H + 12.0;

    // The pages.
    for (i, page) in Page::ALL.into_iter().enumerate() {
        if page == Page::Chain {
            y += 6.0;
            p.fill(Rect::new(r.x + 12.0, y, r.w - 24.0, px), pal.line);
            y += 7.0;
        }
        let row = Rect::new(r.x + 6.0, y, r.w - 13.0, NAV_H);
        let hit = Hit::Nav(page);
        let selected = page == input.page;
        pane::draw_row_fill(
            p,
            &pal,
            row,
            env.is_hover(hit),
            env.is_pressed(hit),
            selected,
        );
        let ink = if selected { pal.text } else { pal.muted };
        p.icon(
            page.icon(),
            15.0,
            row.x + 24.0,
            row.cy(),
            if selected { pal.accent } else { ink },
        );
        p.text(
            Face::Ui,
            13.0,
            row.x + 42.0,
            row.cy(),
            page.title(),
            ink,
            row.w - 52.0,
        );
        env.hits.add(hit, row);
        y += NAV_H + 2.0;
        let _ = i;
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
            kit::button(
                p,
                &pal,
                b,
                "Retry",
                env.is_hover(Hit::Retry),
                env.is_pressed(Hit::Retry),
            );
            env.hits.add(Hit::Retry, b);
            top = r.bottom() + 4.0;
        }
    }
    let view = Rect::new(main.x, top, main.w, (main.bottom() - top).max(0.0));
    let old = p.clip(view);
    let x = view.x + PAD;
    let w = (view.w - 2.0 * PAD).max(100.0);
    let mut y = view.y + 14.0 - input.scroll;

    // Header: the title, what the page is for, and its badges.
    let page = input.page;
    p.text(Face::Semibold, 17.0, x, y + 10.0, page.title(), pal.text, w);
    let mut tag_x = x + w;
    let mut tags = pages::tags(page, input.model);
    tags.push(("Device read only".into(), Tone::Quiet));
    for (text, tone) in tags.iter().rev() {
        let bw = (p.measure(Face::Semibold, 10.5, text) + 12.0).ceil();
        tag_x -= bw;
        kit::badge(p, &pal, tag_x, y + 10.0, text, *tone);
        tag_x -= 6.0;
    }
    p.text(Face::Ui, 12.0, x, y + 32.0, page.about(), pal.muted, w);
    y += 52.0;

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
    let used = if page == Page::Chain {
        pages::draw_chain(p, env, Rect::new(x, y, w, 0.0))
    } else {
        let cards = pages::cards(page, input.model, input.prefs);
        kit::flow(p, env, Rect::new(x, y, w, 0.0), &cards)
    };
    let content = (start + used + 24.0) - (view.y - input.scroll);
    let max_scroll = (content - view.h).max(0.0);
    p.restore(old);
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
    use crate::sample::Sample;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::Gfx;
    use mdshared_ui::theme::{DARK, LIGHT};

    fn render(
        page: Page,
        sample: Sample,
        pal: mdshared_ui::theme::Palette,
        size: (f32, f32),
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
            scroll: 0.0,
            maximized: false,
            hover: None,
            pressed: None,
            toast: None,
            frosted: false,
        };
        let out = draw_frame(&mut p, &input, None);
        (bmp, out)
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
}
