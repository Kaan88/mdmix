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
use mdshared_ui::settings::pages::Form;

use crate::facts::{self, DASH};
use crate::icons;
use crate::kit::{self, Env, Hit, Hits, Tone, chrome_pal};
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
    /// Where the pointer is, for the tip over a control that is read only.
    pub pointer: Option<(f32, f32)>,
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

pub const READ_ONLY_TIP: &str =
    "Read only for now: this version shows the G6 and doesn't change it.";

pub fn draw_frame(p: &mut Painter, input: &FrameIn, dialog: Option<&mut Form>) -> FrameOut {
    let (w, h) = input.size;
    let dialog_is_none = dialog.is_none();
    let pal = input.pal;
    let mut env = Env {
        pal,
        model: input.model,
        hits: Hits::default(),
        hover: input.hover,
        pressed: input.pressed,
    };
    p.fill(Rect::new(0.0, 0.0, w, h), pal.page);

    let title = Rect::new(0.0, 0.0, w, TITLE_H);
    let main = Rect::new(
        SIDE_W,
        TITLE_H,
        (w - SIDE_W).max(0.0),
        (h - TITLE_H).max(0.0),
    );
    let (view, max_scroll) = draw_main(p, &mut env, input, main);

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

    if let Some(form) = dialog {
        form.draw(p, &pal, Rect::new(0.0, 0.0, w, h), false);
    }
    // A device control that does nothing yet says so, under the pointer.
    if input.pointer.is_some()
        && dialog_is_none
        && let Some((x, y)) = input.pointer
        && let Some((Hit::Inert, r)) = env.hits.at_rect(x, y)
    {
        Bubble::tip(p, READ_ONLY_TIP, r, w, h).draw(p, &pal);
    }
    // The toast is on top of everything, the dialog too.
    if let Some(t) = input.toast {
        Bubble::toast(p, t, main).draw(p, &pal);
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

    // Output: two equal halves, the one in use in the accent. Read only: inert, in full colour.
    let out_r = Rect::new(x, y, inner, kit::SEG_H);
    kit::draw_seg(
        p,
        &live,
        out_r,
        &facts::OUTPUTS,
        facts::output_index(&m.status),
        None,
        kit::SegKind {
            small: false,
            fill: true,
        },
    );
    env.hits.add(Hit::Inert, out_r);
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
    kit::slider(
        p,
        &live,
        track,
        vol.map(|v| f32::from(v) / 100.0),
        false,
        live.pane,
    );
    env.hits.add(Hit::Inert, Rect::new(x, y, inner, 28.0));
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
    kit::draw_seg(
        p,
        &live,
        lay_r,
        &facts::LAYOUTS,
        chosen,
        None,
        kit::SegKind {
            small: true,
            fill: false,
        },
    );
    env.hits.add(Hit::Inert, lay_r);
    y += kit::SEG_H_SM + 10.0;

    // The pages: 34 high, 2 apart, the current one on the `on` fill with the accent mark.
    y += 6.0;
    for page in Page::ALL {
        if page == Page::Chain {
            y += 4.0;
            p.fill(Rect::new(r.x + 8.0, y, r.w - 16.0 - px, px), live.line);
            y += 5.0;
        }
        let row = Rect::new(r.x + 6.0, y, r.w - 12.0 - px, NAV_H);
        let hit = Hit::Nav(page);
        let selected = page == input.page;
        pane::draw_row_fill(
            p,
            &live,
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
            if selected { live.accent } else { ink },
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
    let mut tags = pages::tags(page, input.model);
    tags.push(("Device read only".into(), Tone::Quiet));
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
    let used = if page == Page::Chain {
        pages::draw_chain(p, env, Rect::new(x, y, w, 0.0))
    } else {
        let cards = pages::cards(page, input.model, input.prefs);
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
        render_at(page, sample, pal, size, 0.0)
    }

    fn render_at(
        page: Page,
        sample: Sample,
        pal: mdshared_ui::theme::Palette,
        size: (f32, f32),
        scroll: f32,
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
        assert!(!Hit::Inert.acts() && !Hit::Drag.acts());
        assert!(Hit::Retry.acts() && Hit::Nav(Page::Sbx).acts() && Hit::Theme(0).acts());
        // The controls of the page are there to be found, as inert regions.
        let found = (0..1100).step_by(6).any(|x| {
            (60..720)
                .step_by(6)
                .any(|y| matches!(out.hits.at(x as f32, y as f32), Some(Hit::Inert)))
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
            pressed: None,
            toast: None,
            frosted: false,
        };
        draw_frame(&mut p, &input, form.as_mut());
        bmp.pixels
    }

    #[test]
    fn the_read_only_tip_shows_over_a_control_and_never_over_the_dialog() {
        let (_, out) = render(Page::Playback, Sample::Live, LIGHT, (1100.0, 720.0));
        let at = (0..1100)
            .step_by(4)
            .flat_map(|x| (60..720).step_by(4).map(move |y| (x as f32, y as f32)))
            .find(|&(x, y)| matches!(out.hits.at(x, y), Some(Hit::Inert)))
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
}
