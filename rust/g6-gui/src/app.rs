//! The window: winit for the window and its events, softbuffer for the pixels, mdshared-ui for everything drawn.
//! The loop is this app's own, as mdview's and mdedit's are; the shared frame isn't built yet.
//!
//! Motion: a frame is drawn between `Animator::begin_frame` and `end_frame`; the deadline `end_frame` gives (the
//! toasts' own times and the tooltip's delay included, through `wake_at`) is what the loop waits for, and with nothing
//! moving it waits for an event, as before. See `doc/gui-plan.md`, "Motion".
//!
//! Writes: a click on a bound control, a slider's drag (taken by the pointer until release) or a dropdown's choice is
//! an intent for `desk::Desk`, which hands the worker the CLI's write for it and reconciles the read-back. Ctrl+Z
//! undoes the last single setting.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mdshared_text::Bitmap;
use mdshared_ui::anim::{Clock, SystemClock};
use mdshared_ui::colour_picker;
use mdshared_ui::frost::{self, Frost};
use mdshared_ui::gfx::{Gfx, Painter, Rect};
use mdshared_ui::keys::{self, Mods};
use mdshared_ui::menu::{Item as MenuItem, Menu};
use mdshared_ui::platform;
use mdshared_ui::scroll::SmoothScroll;
use mdshared_ui::settings::pages::Form;
use mdshared_ui::settings::{Outcome, Value};
use mdshared_ui::slider::Slider;
use mdshared_ui::tooltip::Over;
use mdshared_ui::window;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use mdshared_app::backdrop::Backdrop;
use mdshared_ui::corners::Corners;

use crate::cache::{self, Cached};
use crate::desk::{Desk, Phase, Toast};
use crate::dialog;
use crate::frame::{FrameIn, FrameOut, draw_frame, popup_area};
use crate::fx::{Fx, TOAST_FOR};
use crate::intent::{Setting, Val};
use crate::kit::{Hit, Hits};
use crate::levels::{self, LevelsHandle, MeterEvent, Meters};
use crate::model::{Conn, Model, Page};
use crate::pick::{self, ColourPick, Press};
use crate::prefs::{self, Prefs, Theme};
use crate::worker::{Cmd, ReadResult, Worker};

/// The slider in hand: its setting, the rectangle its thumb travels in, and the toolkit's slider doing the arithmetic.
struct Capture {
    setting: Setting,
    track: Rect,
    slider: Slider,
}

/// The rectangle the toolkit's slider is given for a slider the kit drew in `r`: the kit's knob travels 7 px in from
/// the ends, the toolkit's half its 18 px thumb, so the toolkit's rectangle is 2 px longer at each end.
fn travel_rect(s: Setting, r: Rect) -> Rect {
    if vertical(s) {
        Rect::new(r.x, r.y - 2.0, r.w, r.h + 4.0)
    } else {
        Rect::new(r.x - 2.0, r.y, r.w + 4.0, r.h)
    }
}

/// The EQ's preamp and bands stand upright.
fn vertical(s: Setting) -> bool {
    matches!(s, Setting::EqPreamp | Setting::EqBand(_))
}

/// The toolkit's slider for a setting at the value shown.
fn slider_for(s: Setting, shown: Val) -> Option<Slider> {
    let (lo, hi, step) = s.range()?;
    let sl = Slider::new(lo, hi, s.unit_of(shown)).step(step);
    Some(if vertical(s) { sl.vertical() } else { sl })
}

/// What wakes the loop from another thread (or from the toolkit's watchers).
pub enum UserEvent {
    // Boxed: a read is much larger than the motion notice, and the queue hands it over once.
    Read(Box<ReadResult>),
    /// The system's wish for less motion may have changed (`platform::watch_motion_reduced`).
    Motion,
    /// The live playback meter's thread: it opened the endpoint's meter, a reading, or no meter at all.
    Levels(MeterEvent),
}

const WHEEL_LINE: f32 = 48.0;

pub fn run(page: Option<Page>) -> i32 {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("g6-gui: no event loop: {e}");
            return 1;
        }
    };
    let proxy = event_loop.create_proxy();
    // The system's "Animation effects" is watched from the start (on Windows a hidden window of the toolkit's hears
    // it change); a change wakes the loop, which reads it again.
    let motion = proxy.clone();
    platform::watch_motion_reduced(move || {
        let _ = motion.send_event(UserEvent::Motion);
    });
    let meter_proxy = proxy.clone();
    let worker = Worker::start(move |r| {
        let _ = proxy.send_event(UserEvent::Read(Box::new(r)));
    });
    // The meter's own thread opens the endpoint's meter when the Mixer page needs it: never the HID worker, never the
    // UI thread (see `g6_osaudio::OutputMeter`).
    let levels = LevelsHandle::start(meter_proxy);
    let mut prefs = Prefs::load();
    if let Some(p) = page {
        prefs.page = p;
    }
    let mut app = App::new(prefs, worker, levels, cache::load());
    // The first read starts at once, before the window exists: the window opens while it runs.
    app.ask_read(false);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("g6-gui: {e}");
        return 1;
    }
    0
}

struct App {
    prefs: Prefs,
    model: Model,
    worker: Worker,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    bmp: Bitmap,
    gfx: Gfx,
    frost: Frost,
    /// Logical px per device px's inverse: the window's scale factor.
    scale: f32,
    /// Window size in device px.
    size: (u32, u32),
    maximized: bool,
    focused: bool,
    system_dark: bool,
    page: Page,
    /// The page's scroll: the wheel and the keys move a target, the drawing follows it.
    scroll: SmoothScroll,
    max_scroll: f32,
    view: Rect,
    hits: Hits,
    pointer: Option<(f32, f32)>,
    pressed: Option<Hit>,
    mods: Mods,
    last_click: Option<(Instant, (f32, f32), u8)>,
    dialog: Option<Form>,
    /// The write path: what was asked for and not read back yet, the drag's throttle, Undo.
    desk: Desk,
    /// The slider in hand, which has the pointer until it is let go.
    capture: Option<Capture>,
    /// The live playback meter's thread, and the ballistics of the two channels it feeds.
    levels: LevelsHandle,
    meters: Meters,
    /// The window's state as the meter decides on it: a minimised or occluded window polls no level.
    minimized: bool,
    occluded: bool,
    /// Motion: the animator and what is kept while it leaves (the toasts' texts, the tooltip's last anchor, the closed
    /// dialog, an open dropdown).
    fx: Fx,
    clock: SystemClock,
    /// When the next frame is due, as the last one said: `None` is "sleep until an event".
    deadline: Option<Instant>,
    drawn_once: bool,
}

impl App {
    fn new(prefs: Prefs, worker: Worker, levels: LevelsHandle, cached: Option<Cached>) -> App {
        App {
            page: prefs.page,
            prefs,
            // The last status read is on screen at once, inert, until this run's first read replaces it.
            model: cached.map_or_else(Model::starting, |c| Model::cached(c.status, c.mixer)),
            worker,
            window: None,
            surface: None,
            bmp: Bitmap::new(1, 1),
            gfx: Gfx::new(),
            frost: Frost::default(),
            scale: 1.0,
            size: (1, 1),
            maximized: false,
            focused: true,
            system_dark: platform::system_is_dark(),
            scroll: SmoothScroll::new(),
            max_scroll: 0.0,
            view: Rect::default(),
            hits: Hits::default(),
            pointer: None,
            pressed: None,
            mods: Mods::default(),
            last_click: None,
            dialog: None,
            desk: Desk::default(),
            capture: None,
            levels,
            meters: Meters::none(),
            minimized: false,
            occluded: false,
            fx: Fx::new(),
            clock: SystemClock,
            deadline: None,
            drawn_once: false,
        }
    }

    fn pal(&self) -> mdshared_ui::theme::Palette {
        self.prefs.theme.palette(self.system_dark)
    }

    fn logical(&self) -> (f32, f32) {
        (
            self.size.0 as f32 / self.scale,
            self.size.1 as f32 / self.scale,
        )
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Shows a toast: one about the same setting takes the place of that setting's own, others join the stack.
    fn say(&mut self, toast: Toast) {
        self.fx.announce(toast, Instant::now() + TOAST_FOR);
        self.redraw();
    }

    /// Reads the system's wish for less motion: at start, and when the settings may have changed.
    fn sync_motion(&mut self) {
        self.fx.anim.set_system_reduced(platform::motion_reduced());
    }

    /// Asks the worker for a status read. `user` says a person asked, so the result is announced.
    fn ask_read(&mut self, user: bool) {
        if !self.model.have_status || matches!(self.model.conn, Conn::Away(_)) {
            self.model.conn = Conn::Reading;
        }
        if !self.worker.ask(Cmd::Read { user }) {
            self.model.conn = Conn::Away("The device worker is not running.".into());
        }
        self.sync_dialog();
    }

    fn read_done(&mut self, r: ReadResult) {
        // The writes of the turn are settled against its read; a read the user asked for says how it went.
        let read_ok = r.status.is_ok();
        if let Some(t) = self.desk.reconcile(&mut self.model, r, Instant::now()) {
            self.say(t);
        }
        // A full status read is what the next start opens on.
        if read_ok && self.model.conn == Conn::Connected {
            cache::save(&Cached {
                status: self.model.status.clone(),
                mixer: self.model.mixer.clone(),
            });
        }
        if matches!(self.model.conn, Conn::Away(_)) {
            self.capture = None;
            self.fx.pick = None;
        }
        self.sync_dialog();
        self.redraw();
    }

    /// A user action on a device control: the desk decides what goes to the worker.
    fn intent(&mut self, s: Setting, v: Val, phase: Phase) {
        match self.desk.intent(&self.model, s, v, phase, Instant::now()) {
            Ok(Some(cmd)) => self.send(cmd),
            Ok(None) => {}
            Err(t) => self.say(t),
        }
        self.redraw();
    }

    fn send(&mut self, cmd: Cmd) {
        if !self.worker.ask(cmd) {
            self.model.conn = Conn::Away("The device worker is not running.".into());
            self.desk.pending.clear();
        }
    }

    /// Ctrl+Z: the last single setting back to what it was, as a new write.
    fn undo(&mut self) {
        match self.desk.undo(&self.model, Instant::now()) {
            Ok(Some(cmd)) => self.send(cmd),
            Ok(None) => {}
            Err(t) => self.say(t),
        }
        self.redraw();
    }

    /// Takes hold of a slider at the press; the value jumps to the point unless the thumb was taken.
    fn grab(&mut self, s: Setting, r: Rect, x: f32, y: f32) {
        let shown = self.desk.shown(&self.model);
        let Some(mut slider) = s.current(&shown).and_then(|v| slider_for(s, v)) else {
            return;
        };
        let track = travel_rect(s, r);
        if !slider.press(track, x, y) {
            return;
        }
        let v = s.val_of(slider.value());
        self.capture = Some(Capture {
            setting: s,
            track,
            slider,
        });
        self.intent(s, v, Phase::Drag);
    }

    /// Lets go of the slider in hand: its last value is sent.
    fn let_go(&mut self) {
        if let Some(mut c) = self.capture.take() {
            c.slider.release();
            let v = c.setting.val_of(c.slider.value());
            self.intent(c.setting, v, Phase::Release);
        }
    }

    /// Opens a dropdown's choices under it, the one the G6 has ticked.
    fn open_menu(&mut self, s: Setting) {
        let Some(r) = self.hits.rect_of(Hit::Open(s)) else {
            return;
        };
        let now = s.current(&self.desk.shown(&self.model));
        let items: Vec<MenuItem<Hit>> = s
            .options()
            .into_iter()
            .map(|(label, v)| MenuItem::new(Hit::Set(s, v), label).checked(now == Some(v)))
            .collect();
        self.fx.menu = Some(Menu::new(items, r.x, r.bottom() + 2.0, false));
        self.redraw();
    }

    // ---- the colour picker ---------------------------------------------------------------------

    /// Opens the toolkit's colour picker on the Lighting page's Colour row, on the colour the row shows (or a mid
    /// grey when nothing has been set from here: the G6 can't be asked).
    fn open_picker(&mut self) {
        let Some(anchor) = self.hits.rect_of(Hit::Open(Setting::Colour)) else {
            return;
        };
        let original = Setting::Colour.current(&self.desk.shown(&self.model));
        let mut pk = ColourPick::open(anchor, pick::rgb_or_fallback(original), original);
        pk.place(popup_area(self.logical()));
        self.fx.pick = Some(pk);
        self.redraw();
    }

    /// Sends a colour through the desk, as a click at once, throttled while the pointer holds a marker, and with the
    /// last value always on release.
    fn write_colour(&mut self, v: Val, phase: Phase) {
        match self
            .desk
            .intent(&self.model, Setting::Colour, v, phase, Instant::now())
        {
            Ok(Some(cmd)) => {
                self.send(cmd);
                if let Some(pk) = &mut self.fx.pick {
                    pk.sent = Some(v);
                }
            }
            Ok(None) => {}
            Err(t) => self.say(t),
        }
    }

    /// Closes the colour picker. Accepting keeps the colour, sending it when the last one did not go; anything else is
    /// a cancel: the colour from before the open is written back, if something was written during this open.
    fn close_picker(&mut self, accept: bool) {
        let Some(pk) = self.fx.pick.take() else {
            return;
        };
        let write = pick::on_close(pk.sent, pk.colour(), pk.original, accept);
        // A drag left in hand must not send its waiting value after the fact: it would overwrite what goes now.
        self.desk.abandon(Setting::Colour);
        if let Some(v) = write {
            self.write_colour(v, if accept { Phase::Release } else { Phase::Click });
        }
        self.redraw();
    }

    /// A press with the colour picker open: it is the picker's on its card; anywhere else the picker closes as a
    /// cancel and nothing under it is pressed.
    fn picker_press(&mut self, x: f32, y: f32, clicks: u8) {
        let Some(pk) = self.fx.pick.as_mut() else {
            return;
        };
        let press = pk.press(x, y, self.mods.shift, clicks);
        let colour = match press {
            Press::Away => {
                self.close_picker(false);
                return;
            }
            Press::Colour(v) => Some(v),
            Press::Taken => None,
        };
        if let Some(v) = colour {
            // A press is the start of a drag, as a slider's press is: it goes at once, and the whole press, drag and
            // release is one step for Undo.
            self.write_colour(v, Phase::Drag);
        }
        self.redraw();
    }

    /// The pointer moved with the picker open: a held marker follows it, a selection in the hex field grows. True
    /// when the picture changed.
    fn picker_move(&mut self, x: f32, y: f32) -> bool {
        let Some(pk) = self.fx.pick.as_mut() else {
            return false;
        };
        let (redraw, colour) = pk.drag(x, y);
        if let Some(v) = colour {
            // Throttled like a dragged slider: the desk keeps the newest value and sends it when its time comes.
            self.write_colour(v, Phase::Drag);
        }
        redraw
    }

    /// The button went up over the open picker: the drag's last colour goes, whenever it was held.
    fn picker_release(&mut self) {
        let Some(pk) = self.fx.pick.as_mut() else {
            return;
        };
        let colour = pk.release();
        if let Some(v) = colour {
            self.write_colour(v, Phase::Release);
        }
        self.redraw();
    }

    /// A key with the picker open: Tab between the parts, the arrows (Shift for more), Enter accepts, Escape cancels.
    fn picker_key(&mut self, name: &keys::KeyName, typed: Option<&str>) {
        let Some(pk) = self.fx.pick.as_mut() else {
            return;
        };
        let (outcome, colour) = pk.key(name, self.mods, typed);
        if let Some(v) = colour {
            self.write_colour(v, Phase::Click);
        }
        match outcome {
            colour_picker::Outcome::Accept => self.close_picker(true),
            colour_picker::Outcome::Cancel => self.close_picker(false),
            colour_picker::Outcome::Redraw | colour_picker::Outcome::Colour => self.redraw(),
            colour_picker::Outcome::None | colour_picker::Outcome::Unused => {}
        }
    }

    /// The dialog's Device page shows the model, so it is rebuilt when the model changes.
    fn sync_dialog(&mut self) {
        if let Some(f) = &mut self.dialog {
            f.set_pages(dialog::pages(&self.prefs, &self.model));
        }
    }

    // ---- window ------------------------------------------------------------------------------

    fn sync_style(&self) {
        #[cfg(windows)]
        if let Some(w) = &self.window
            && let Some(h) = hwnd(w)
        {
            platform::style_window(h, self.pal().dark);
            mdshared_ui::corners::set_window_corners(h, self.prefs.corners);
        }
    }

    /// Asks the system for the backdrop when the setting or the focus changed.
    fn sync_frost(&mut self) {
        let want = self.prefs.backdrop;
        if !self.frost.concerns(want) {
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        #[cfg(windows)]
        let mut sys = frost::WindowSystem(hwnd(&window));
        #[cfg(not(windows))]
        let mut sys = frost::BlurSystem(&window);
        let pal = self.pal();
        if self.frost.sync(want, self.focused, &pal, &mut sys) {
            self.redraw();
        }
    }

    /// Starts or stops the live playback meter: it runs only while the Mixer page is shown and the window is there
    /// for a person to see. Called on a page switch, a minimise/restore, an occlusion, and after each frame while the
    /// window first appears; it sends nothing when the answer hasn't changed.
    fn sync_meter(&mut self) {
        let visible = self.window.is_some() && self.drawn_once && !self.minimized && !self.occluded;
        self.levels
            .set_wanted(levels::should_run(self.page, visible));
    }

    fn set_page(&mut self, page: Page) {
        if page != self.page {
            // The bitmap still holds the old page: the transition keeps a picture of its area before the next frame
            // draws the new one. A switch begun while one runs starts from what is on screen.
            let p = Painter::new(&mut self.bmp, &mut self.gfx, self.scale);
            self.fx.begin_page(&p, self.view);
            self.page = page;
            let (content, view) = (self.max_scroll + self.view.h, self.view.h);
            self.scroll.jump_to(0.0, content, view);
            self.prefs.page = page;
            self.prefs.save();
            self.sync_meter();
        }
    }

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let (w, h) = self.size;
        if w == 0 || h == 0 {
            return;
        }
        if (self.bmp.width, self.bmp.height) != (w, h) {
            self.bmp = Bitmap::new(w, h);
        }
        let now = self.clock.now();
        self.fx.anim.begin_frame(now);
        let pal = self.pal();
        let logical = self.logical();
        let (content, view) = (self.max_scroll + self.view.h, self.view.h);
        let scroll = self
            .scroll
            .offset(&mut self.fx.anim, "scroll", content, view)
            .round();
        let hover = if self.dialog.is_some() {
            None
        } else {
            self.pointer.and_then(|(x, y)| self.hits.at(x, y))
        };
        // The pending values are drawn over the device's until their read-back comes.
        let shown = self.desk.shown(&self.model);
        let input = FrameIn {
            model: &shown,
            prefs: &self.prefs,
            pal,
            page: self.page,
            size: logical,
            scroll,
            maximized: self.maximized,
            hover,
            pointer: if self.dialog.is_some() {
                None
            } else {
                self.pointer
            },
            now,
            pressed: self.pressed,
            dragging: self.capture.as_ref().map(|c| c.setting),
            frosted: self.frost.key().is_some(),
            meters: &self.meters,
        };
        let mut p = Painter::new(&mut self.bmp, &mut self.gfx, self.scale);
        let out: FrameOut = draw_frame(&mut p, &input, self.dialog.as_mut(), &mut self.fx);
        self.frost.areas = out
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
            .collect();
        self.hits = out.hits;
        // The page may have moved under a pointer that didn't: its shape follows what is under it now.
        if let Some((x, y)) = self.pointer {
            self.update_cursor(x, y);
        }
        self.max_scroll = out.max_scroll;
        self.view = out.view;
        self.present();
        if !self.drawn_once {
            self.drawn_once = true;
            window.set_visible(true);
        }
        // The toasts' own times and the tooltip's delay ride on the animator's deadline: one wake-up serves them all.
        self.deadline = self.fx.wake();
        // The first frame makes the window visible, which is what the meter waits for.
        self.sync_meter();
    }

    fn present(&mut self) {
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let (w, h) = self.size;
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else {
            return;
        };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        // BGRA bytes are the 0xAARRGGBB words of a little-endian machine.
        for (d, c) in buffer.iter_mut().zip(self.bmp.pixels.as_chunks::<4>().0) {
            *d = u32::from_le_bytes(*c);
        }
        if let Some(key) = self.frost.key() {
            for area in &self.frost.areas {
                frost::key_out(key, &mut buffer, w as usize, *area);
            }
        }
        let _ = buffer.present();
    }

    // ---- pointer -----------------------------------------------------------------------------

    fn edge(&self, x: f32, y: f32) -> Option<ResizeDirection> {
        if self.maximized {
            return None;
        }
        let (w, h) = self.logical();
        window::edge_at(x, y, w, h)
    }

    fn pointer_moved(&mut self, x: f32, y: f32) {
        let before = self.pointer.and_then(|(x, y)| self.hits.at_rect(x, y));
        self.pointer = Some((x, y));
        let mut changed = false;
        if let Some(c) = &mut self.capture {
            if c.slider.drag(c.track, x, y) {
                let (s, v) = (c.setting, c.setting.val_of(c.slider.value()));
                self.intent(s, v, Phase::Drag);
            }
            return;
        }
        if self.fx.pick.is_some() {
            if self.picker_move(x, y) {
                self.redraw();
            }
            self.update_cursor(x, y);
            return;
        }
        if let Some(m) = &mut self.fx.menu {
            if m.pointer(x, y) {
                self.redraw();
            }
            self.update_cursor(x, y);
            return;
        }
        if let Some(f) = &mut self.dialog {
            changed |= f.pointer_move(x, y);
        } else {
            changed |= before != self.hits.at_rect(x, y);
            // The tip's wait counts from the pointer coming to rest, so a move on the same control starts it again
            // even when nothing drawn changed.
            let over = self.tip_over(x, y);
            changed |= self.fx.tip.hover_at(Instant::now(), over, (x, y));
        }
        self.update_cursor(x, y);
        if changed {
            self.redraw();
        }
    }

    /// The tip over the read-only control at (`x`, `y`), from the regions the last frame recorded; nothing while a
    /// dialog, a menu, the colour picker or a closing dialog has the window.
    fn tip_over(&self, x: f32, y: f32) -> Option<Over<Rect>> {
        if self.dialog.is_some()
            || self.fx.leaving.is_some()
            || self.fx.menu.is_some()
            || self.fx.pick.is_some()
        {
            return None;
        }
        crate::frame::tip_over(&self.hits, x, y)
    }

    /// The pointer's shape at (`x`, `y`): a resize edge, not-allowed over a read-only control, or the arrow.
    fn update_cursor(&self, x: f32, y: f32) {
        if let Some(w) = &self.window {
            let icon = match self.edge(x, y) {
                Some(ResizeDirection::North | ResizeDirection::South) => CursorIcon::NsResize,
                Some(ResizeDirection::East | ResizeDirection::West) => CursorIcon::EwResize,
                Some(ResizeDirection::NorthWest | ResizeDirection::SouthEast) => {
                    CursorIcon::NwseResize
                }
                Some(ResizeDirection::NorthEast | ResizeDirection::SouthWest) => {
                    CursorIcon::NeswResize
                }
                None if self.fx.pick.is_some() => CursorIcon::Default,
                None if self.dialog.is_none()
                    && self.fx.menu.is_none()
                    && matches!(self.hits.at(x, y), Some(Hit::Inert(_))) =>
                {
                    CursorIcon::NotAllowed
                }
                None => CursorIcon::Default,
            };
            w.set_cursor(icon);
        }
    }

    fn clicks(&mut self, x: f32, y: f32) -> u8 {
        let now = Instant::now();
        let n = match self.last_click {
            Some((t, (px, py), n))
                if now.duration_since(t) <= platform::double_click_time()
                    && (px - x).abs() < 4.0
                    && (py - y).abs() < 4.0 =>
            {
                n.saturating_add(1)
            }
            _ => 1,
        };
        self.last_click = Some((now, (x, y), n));
        n
    }

    fn pressed_at(&mut self, event_loop: &ActiveEventLoop, x: f32, y: f32) {
        let clicks = self.clicks(x, y);
        if let Some(f) = &mut self.dialog {
            let out = f.press(x, y, self.mods, clicks);
            self.dialog_outcome(event_loop, out);
            return;
        }
        // An open dropdown takes the click: on it, the choice is made on release; beside it, it closes.
        if let Some(m) = &self.fx.menu {
            if !m.contains(x, y) {
                self.fx.menu = None;
                self.redraw();
            }
            return;
        }
        // The colour picker takes the press while it is open, wherever it is.
        if self.fx.pick.is_some() {
            self.picker_press(x, y, clicks);
            return;
        }
        // A control just pressed shows no tip until the pointer has left it.
        self.fx.tip.press(self.hits.at_rect(x, y).map(|(_, r)| r));
        if let (Some(dir), Some(w)) = (self.edge(x, y), &self.window) {
            let _ = w.drag_resize_window(dir);
            return;
        }
        if let Some((Hit::Slide(s), r)) = self.hits.at_rect(x, y) {
            self.grab(s, r, x, y);
            return;
        }
        match self.hits.at(x, y) {
            Some(Hit::Drag) => {
                if clicks >= 2 {
                    if let Some(w) = &self.window {
                        window::title_double_click(w);
                    }
                } else if let Some(w) = &self.window {
                    let _ = w.drag_window();
                }
            }
            Some(hit) => {
                self.pressed = Some(hit);
                self.redraw();
            }
            None => {}
        }
    }

    fn released_at(&mut self, event_loop: &ActiveEventLoop, x: f32, y: f32) {
        if self.capture.is_some() {
            self.let_go();
            return;
        }
        if self.fx.pick.is_some() {
            self.picker_release();
            return;
        }
        if let Some(m) = &mut self.fx.menu {
            if m.contains(x, y) {
                m.pointer(x, y);
                let chosen = m.chosen();
                if chosen.is_some() {
                    self.fx.menu = None;
                }
                if let Some(hit) = chosen {
                    self.activate(event_loop, hit);
                }
                self.redraw();
            }
            return;
        }
        if let Some(f) = &mut self.dialog {
            let out = f.release(x, y);
            self.dialog_outcome(event_loop, out);
            return;
        }
        let pressed = self.pressed.take();
        let over = self.hits.at(x, y);
        self.redraw();
        if let (Some(a), Some(b)) = (pressed, over)
            && a == b
        {
            self.activate(event_loop, a);
        }
    }

    fn activate(&mut self, event_loop: &ActiveEventLoop, hit: Hit) {
        if !hit.acts() {
            return;
        }
        match hit {
            Hit::Nav(page) => self.set_page(page),
            Hit::Settings => self.open_dialog(0),
            Hit::Retry | Hit::Refresh => self.ask_read(true),
            Hit::Min => {
                if let Some(w) = &self.window {
                    w.set_minimized(true);
                }
            }
            Hit::Max => {
                if let Some(w) = &self.window {
                    w.set_maximized(!w.is_maximized());
                }
            }
            Hit::Close => self.quit(event_loop),
            Hit::Theme(i) => self.apply_setting(dialog::THEME, &Value::Choice(i)),
            Hit::Backdrop(i) => self.apply_setting(dialog::BACKDROP, &Value::Choice(i)),
            Hit::Corners(i) => self.apply_setting(dialog::CORNERS, &Value::Choice(i)),
            Hit::Set(s, v) => self.intent(s, v, Phase::Click),
            Hit::Open(Setting::Colour) => self.open_picker(),
            Hit::Open(s) => self.open_menu(s),
            // A slider is taken on the press (`grab`), not on a click.
            Hit::Slide(_) | Hit::Drag | Hit::Inert(_) | Hit::Blocked => {}
        }
        self.redraw();
    }

    fn open_dialog(&mut self, page: usize) {
        // A settings dialog is above the picker: the picker closes (as a cancel) rather than stay under it.
        self.close_picker(false);
        self.dialog = Some(dialog::form_at(&self.prefs, &self.model, page));
        self.pressed = None;
        self.fx.menu = None;
    }

    fn dialog_outcome(&mut self, event_loop: &ActiveEventLoop, out: Outcome) {
        match out {
            Outcome::Changed { id, value } => self.apply_setting(id, &value),
            Outcome::Action(id) if id == dialog::READ => self.ask_read(true),
            // The closed form is kept while it fades out.
            Outcome::Closed => self.fx.leaving = self.dialog.take(),
            Outcome::None | Outcome::Unused | Outcome::Redraw | Outcome::Action(_) => {}
        }
        let _ = event_loop;
        self.redraw();
    }

    /// An appearance setting changed, in the dialog or on the Settings page. App settings only: the G6 isn't
    /// touched.
    fn apply_setting(&mut self, id: u32, value: &Value) {
        let Value::Choice(i) = value else { return };
        match id {
            dialog::THEME => self.prefs.theme = Theme::from_index(*i),
            dialog::BACKDROP => self.prefs.backdrop = Backdrop::from_index(*i),
            dialog::CORNERS => self.prefs.corners = Corners::from_index(*i),
            _ => return,
        }
        if let Some(f) = &mut self.dialog {
            f.set_value(id, value.clone());
        }
        self.prefs.save();
        self.sync_style();
        self.sync_frost();
        self.redraw();
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        // A minimised window reports a size of 0 or the icon's: only a normal window's size is kept, and only if it
        // is one the window can have.
        // A maximized window keeps the size and place it had before, and opens maximized again.
        if let Some(w) = &self.window
            && w.is_minimized() != Some(true)
        {
            self.prefs.maximized = w.is_maximized();
        }
        if let Some(w) = &self.window
            && !w.is_maximized()
            && w.is_minimized() != Some(true)
        {
            let s = w.inner_size().to_logical::<f64>(w.scale_factor());
            let size = (s.width.round() as u32, s.height.round() as u32);
            if size.0 >= prefs::MIN.0 && size.1 >= prefs::MIN.1 {
                self.prefs.size = Some(size);
            }
            if let Ok(p) = w.outer_position() {
                self.prefs.pos = Some((p.x, p.y));
            }
        }
        self.prefs.save();
        // The meter's thread is waited for: one poll interval at most, and the endpoint is let go before the window is.
        self.levels.stop();
        event_loop.exit();
    }

    /// The wheel and the keys: the target moves, the page glides to it.
    fn scroll_by(&mut self, dy: f32) {
        let (content, view) = (self.max_scroll + self.view.h, self.view.h);
        if self.scroll.scroll_by(-dy, content, view) {
            self.redraw();
        }
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, event: &winit::event::KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let name = keys::key_name(&event.logical_key);
        if let Some(f) = &mut self.dialog {
            if let Some(name) = name {
                let out = f.key(&name, self.mods, event.text.as_deref());
                self.dialog_outcome(event_loop, out);
            }
            return;
        }
        use keys::KeyName as K;
        if self.fx.pick.is_some() {
            if let Some(name) = name {
                self.picker_key(&name, event.text.as_deref());
            }
            return;
        }
        if let Some(m) = &mut self.fx.menu {
            match name {
                Some(K::Escape) => self.fx.menu = None,
                Some(K::Down) => m.step(true),
                Some(K::Up) => m.step(false),
                Some(K::Enter) => {
                    let chosen = m.chosen();
                    self.fx.menu = None;
                    if let Some(hit) = chosen {
                        self.activate(event_loop, hit);
                    }
                }
                _ => {}
            }
            self.redraw();
            return;
        }
        match (name, self.mods.ctrl) {
            (Some(K::Char('z')), true) => self.undo(),
            (Some(K::F(5)), _) | (Some(K::Char('r')), true) => self.ask_read(true),
            (Some(K::Char(',')), true) => self.open_dialog(0),
            (Some(K::Down), _) => self.scroll_by(-WHEEL_LINE / 2.0),
            (Some(K::Up), _) => self.scroll_by(WHEEL_LINE / 2.0),
            (Some(K::PageDown), _) => self.scroll_by(-self.view.h * 0.9),
            (Some(K::PageUp), _) => self.scroll_by(self.view.h * 0.9),
            (Some(K::Home), _) => self.scroll_by(f32::MAX),
            (Some(K::End), _) => self.scroll_by(f32::MIN),
            _ => {}
        }
    }
}

#[cfg(windows)]
fn hwnd(w: &Window) -> Option<isize> {
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
fn hwnd(w: &Window) -> Option<isize> {
    let _ = (w.window_handle().ok()?.as_raw(), RawWindowHandle::Xlib);
    None
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = self.prefs.size.unwrap_or((1180, 780));
        let attributes = Window::default_attributes()
            .with_title("SoundBlaster X G6")
            .with_decorations(false)
            .with_visible(false)
            .with_inner_size(LogicalSize::new(f64::from(w), f64::from(h)))
            .with_min_inner_size(LogicalSize::new(
                f64::from(prefs::MIN.0),
                f64::from(prefs::MIN.1),
            ))
            .with_maximized(self.prefs.maximized);
        // The place it closed at, while that is still on a monitor (one may have been unplugged since).
        let monitors: Vec<(i32, i32, u32, u32)> = event_loop
            .available_monitors()
            .map(|m| {
                let (p, s) = (m.position(), m.size());
                (p.x, p.y, s.width, s.height)
            })
            .collect();
        let attributes = match self.prefs.place_on(&monitors) {
            Some((x, y)) => attributes.with_position(PhysicalPosition::new(x, y)),
            None => attributes,
        };
        let attributes = window::identify(attributes, "g6-gui", None);
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_undecorated_shadow(true)
        };
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("g6-gui: no window: {e}");
                event_loop.exit();
                return;
            }
        };
        window::created(&window);
        self.scale = window.scale_factor() as f32;
        self.maximized = window.is_maximized();
        let s: PhysicalSize<u32> = window.inner_size();
        self.size = (s.width, s.height);
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|c| softbuffer::Surface::new(&c, window.clone()));
        match surface {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                eprintln!("g6-gui: no surface: {e}");
                event_loop.exit();
                return;
            }
        }
        // Frames come as fast as the screen shows them while something moves.
        if let Some(mhz) = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .filter(|mhz| (24_000..=500_000).contains(mhz))
        {
            self.fx
                .anim
                .set_frame_interval(Duration::from_nanos(1_000_000_000_000 / u64::from(mhz)));
        }
        self.window = Some(window);
        self.sync_motion();
        self.sync_style();
        self.sync_frost();
        self.draw();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Read(r) => self.read_done(*r),
            UserEvent::Motion => {
                self.sync_motion();
                self.redraw();
            }
            // A silent reading once the meters are at rest changes nothing: the window stays asleep while the thread
            // keeps polling (the plan: no fake levels, and no frames for a level that isn't moving).
            UserEvent::Levels(e) => {
                if self.meters.apply(e, Instant::now()) {
                    self.redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.quit(event_loop),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(s) => {
                self.size = (s.width, s.height);
                if let Some(w) = &self.window {
                    self.maximized = w.is_maximized();
                    self.minimized = w.is_minimized() == Some(true);
                }
                // A minimised window polls no level; a restored one starts again.
                self.sync_meter();
                self.redraw();
            }
            WindowEvent::Occluded(o) => {
                self.occluded = o;
                self.sync_meter();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                if let Some(w) = &self.window {
                    let s = w.inner_size();
                    self.size = (s.width, s.height);
                }
                self.redraw();
            }
            WindowEvent::ThemeChanged(t) => {
                self.system_dark = t == winit::window::Theme::Dark;
                self.sync_motion();
                self.sync_style();
                self.sync_frost();
                self.redraw();
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                // A slider held while the window loses the focus is let go there, and so is a colour marker: its
                // last colour goes, and the picker stays open without one in hand. A control pressed then is let go
                // with no click: winit sends no release for a press the window lost, and the press left behind would
                // act on a later release over the same control (one whose press hit nothing).
                if !f {
                    self.let_go();
                    self.picker_release();
                    self.pressed = None;
                }
                if f {
                    self.sync_motion();
                }
                self.sync_frost();
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = Mods::from_winit(m.state()),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer_moved(
                    position.x as f32 / self.scale,
                    position.y as f32 / self.scale,
                );
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer = None;
                self.redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let Some((x, y)) = self.pointer else { return };
                match state {
                    ElementState::Pressed => self.pressed_at(event_loop, x, y),
                    ElementState::Released => self.released_at(event_loop, x, y),
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * WHEEL_LINE,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.scale,
                };
                let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
                if let Some(f) = &mut self.dialog {
                    if f.wheel(x, y, dy) {
                        self.redraw();
                    }
                } else if self.fx.pick.is_some() {
                    // The colour picker holds the page still while it is open, as its row does in the toolkit.
                } else if let Some(m) = &mut self.fx.menu {
                    // Over its list the wheel scrolls it; elsewhere the page moves and the list closes.
                    if m.contains(x, y) {
                        if m.scroll_by(-dy) {
                            self.redraw();
                        }
                    } else {
                        self.fx.menu = None;
                        self.scroll_by(dy);
                    }
                } else if self.capture.is_none() {
                    self.scroll_by(dy);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(event_loop, &event),
            _ => {}
        }
    }

    /// The wait for a frame ended: draw it.
    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            self.redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // A dragged slider's value that waited out the throttle goes now.
        if let Some(cmd) = self.desk.tick(&self.model, now) {
            self.send(cmd);
            self.redraw();
        }
        // A write the G6 didn't answer for is asked again, at 1 s, 3 s and 7 s; at 8 s the desk gives up and says so.
        if let Some(cmd) = self.desk.follow_up(now) {
            self.send(cmd);
            self.redraw();
        }
        if let Some(t) = self.desk.timed_out(&mut self.model, now) {
            self.sync_dialog();
            self.say(t);
        }
        // Whichever toast's time is up goes, and its exit plays from the next frame.
        if self.fx.toasts.tick(now) {
            self.redraw();
        }
        // Something moving or a timer: wake for it. Nothing: sleep until an event, using no CPU.
        let due = [
            self.deadline,
            self.desk.due(),
            self.desk.settle_due(),
            self.fx.toasts.deadline(),
            self.fx.tip.deadline(),
            self.meters.next_wake(now, levels::POLL),
        ]
        .into_iter()
        .flatten()
        .min();
        match due {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolkit's slider, given `travel_rect`, puts the value where the kit draws the knob, so a press on the knob
    /// keeps the value and a press elsewhere takes the value under the pointer.
    #[test]
    fn the_slider_arithmetic_matches_the_drawn_knob() {
        let r = Rect::new(100.0, 50.0, 200.0, 28.0);
        let s = Setting::EffectLevel(0);
        let sl = slider_for(s, Val::Int(62)).unwrap();
        let t = travel_rect(s, r);
        // The kit's knob: 7 px in from each end.
        let kx = r.x + 7.0 + 0.62 * (r.w - 14.0);
        assert_eq!(s.val_of(sl.value_at(t, kx, r.cy())), Val::Int(62));
        assert_eq!(s.val_of(sl.value_at(t, r.x, r.cy())), Val::Int(0));
        assert_eq!(s.val_of(sl.value_at(t, r.right(), r.cy())), Val::Int(100));
        // Upright, in steps of half a dB: the top is +12.
        let band = Setting::EqBand(2);
        let v = slider_for(band, Val::Int(25)).unwrap();
        let vr = Rect::new(10.0, 10.0, 30.0, 172.0);
        let tv = travel_rect(band, vr);
        assert_eq!(band.val_of(v.value_at(tv, vr.cx(), vr.y)), Val::Int(120));
        let ky = vr.bottom() - 7.0 - (2.5 + 12.0) / 24.0 * (vr.h - 14.0);
        assert_eq!(band.val_of(v.value_at(tv, vr.cx(), ky)), Val::Int(25));
        // Noise reduction moves in steps of 20.
        let nr = slider_for(Setting::NoiseReduction, Val::Int(40)).unwrap();
        let x = r.x + 7.0 + 0.55 * (r.w - 14.0);
        assert_eq!(
            Setting::NoiseReduction.val_of(nr.value_at(t, x, r.cy())),
            Val::Int(60)
        );
    }

    #[test]
    fn a_drag_follows_the_pointer_and_ends_on_release() {
        let r = Rect::new(0.0, 0.0, 214.0, 28.0);
        let s = Setting::EffectLevel(3);
        let mut sl = slider_for(s, Val::Int(0)).unwrap();
        let t = travel_rect(s, r);
        assert!(sl.press(t, 107.0, 14.0));
        assert_eq!(s.val_of(sl.value()), Val::Int(50));
        assert!(sl.drag(t, 207.0, 14.0));
        assert_eq!(s.val_of(sl.value()), Val::Int(100));
        // Off the track the value stays at the end.
        sl.drag(t, 400.0, 90.0);
        assert_eq!(s.val_of(sl.value()), Val::Int(100));
        assert!(sl.release());
        assert!(!sl.drag(t, 7.0, 14.0));
    }
}
