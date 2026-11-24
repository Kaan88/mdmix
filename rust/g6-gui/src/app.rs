//! The window: winit for the window and its events, softbuffer for the pixels, mdshared-ui for everything drawn.
//! The loop is this app's own, as mdview's and mdedit's are; the shared frame isn't built yet.
//!
//! Reading only. The one thing this window sends to the G6 is what the worker's status read sends.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mdshared_text::Bitmap;
use mdshared_ui::frost::{self, Frost};
use mdshared_ui::gfx::{Gfx, Painter, Rect};
use mdshared_ui::keys::{self, Mods};
use mdshared_ui::platform;
use mdshared_ui::settings::pages::Form;
use mdshared_ui::settings::{Outcome, Value};
use mdshared_ui::window;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use mdshared_app::backdrop::Backdrop;
use mdshared_ui::corners::Corners;

use crate::dialog;
use crate::frame::{FrameIn, FrameOut, draw_frame};
use crate::kit::{Hit, Hits};
use crate::model::{Conn, Model, Page};
use crate::prefs::{self, Prefs, Theme};
use crate::worker::{Cmd, ReadResult, Worker};

/// What wakes the loop from another thread.
pub enum UserEvent {
    Read(ReadResult),
}

const WHEEL_LINE: f32 = 48.0;
const TOAST_FOR: Duration = Duration::from_millis(3500);

pub fn run(page: Option<Page>) -> i32 {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("g6-gui: no event loop: {e}");
            return 1;
        }
    };
    let proxy = event_loop.create_proxy();
    let worker = Worker::start(move |r| {
        let _ = proxy.send_event(UserEvent::Read(r));
    });
    let mut prefs = Prefs::load();
    if let Some(p) = page {
        prefs.page = p;
    }
    let mut app = App::new(prefs, worker);
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
    scroll: f32,
    max_scroll: f32,
    view: Rect,
    hits: Hits,
    pointer: Option<(f32, f32)>,
    pressed: Option<Hit>,
    mods: Mods,
    last_click: Option<(Instant, (f32, f32), u8)>,
    dialog: Option<Form>,
    toast: Option<(String, Instant)>,
    drawn_once: bool,
}

impl App {
    fn new(prefs: Prefs, worker: Worker) -> App {
        App {
            page: prefs.page,
            prefs,
            model: Model::starting(),
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
            scroll: 0.0,
            max_scroll: 0.0,
            view: Rect::default(),
            hits: Hits::default(),
            pointer: None,
            pressed: None,
            mods: Mods::default(),
            last_click: None,
            dialog: None,
            toast: None,
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

    fn say(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now() + TOAST_FOR));
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
        let why = r.status.as_ref().err().cloned();
        self.model.apply(r.status, r.mixer);
        self.sync_dialog();
        // The result says what it answers: a read the user asked for, merged or not.
        if r.user {
            self.say(why.unwrap_or_else(|| "Status read from the G6.".into()));
        }
        self.redraw();
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

    fn set_page(&mut self, page: Page) {
        if page != self.page {
            self.page = page;
            self.scroll = 0.0;
            self.prefs.page = page;
            self.prefs.save();
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
        let pal = self.pal();
        let logical = self.logical();
        self.scroll = self.scroll.clamp(0.0, self.max_scroll.max(0.0));
        let toast = self.toast.as_ref().map(|(t, _)| t.clone());
        let hover = if self.dialog.is_some() {
            None
        } else {
            self.pointer.and_then(|(x, y)| self.hits.at(x, y))
        };
        let input = FrameIn {
            model: &self.model,
            prefs: &self.prefs,
            pal,
            page: self.page,
            size: logical,
            scroll: self.scroll,
            maximized: self.maximized,
            hover,
            pressed: self.pressed,
            toast: toast.as_deref(),
            frosted: self.frost.key().is_some(),
        };
        let mut p = Painter::new(&mut self.bmp, &mut self.gfx, self.scale);
        let out: FrameOut = draw_frame(&mut p, &input, self.dialog.as_mut());
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
        self.max_scroll = out.max_scroll;
        self.view = out.view;
        self.present();
        if !self.drawn_once {
            self.drawn_once = true;
            window.set_visible(true);
        }
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
        let before = self.pointer.and_then(|(x, y)| self.hits.at(x, y));
        self.pointer = Some((x, y));
        let mut changed = false;
        if let Some(f) = &mut self.dialog {
            changed |= f.pointer_move(x, y);
        } else {
            changed |= before != self.hits.at(x, y);
        }
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
                None => CursorIcon::Default,
            };
            w.set_cursor(icon);
        }
        if changed {
            self.redraw();
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
        if let (Some(dir), Some(w)) = (self.edge(x, y), &self.window) {
            let _ = w.drag_resize_window(dir);
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
            Hit::Drag => {}
        }
        self.redraw();
    }

    fn open_dialog(&mut self, page: usize) {
        self.dialog = Some(dialog::form_at(&self.prefs, &self.model, page));
        self.pressed = None;
    }

    fn dialog_outcome(&mut self, event_loop: &ActiveEventLoop, out: Outcome) {
        match out {
            Outcome::Changed { id, value } => self.apply_setting(id, &value),
            Outcome::Action(id) if id == dialog::READ => self.ask_read(true),
            Outcome::Closed => self.dialog = None,
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
        if let Some(w) = &self.window
            && !w.is_maximized()
            && w.is_minimized() != Some(true)
        {
            let s = w.inner_size().to_logical::<f64>(w.scale_factor());
            let size = (s.width.round() as u32, s.height.round() as u32);
            if size.0 >= prefs::MIN.0 && size.1 >= prefs::MIN.1 {
                self.prefs.size = Some(size);
            }
        }
        self.prefs.save();
        event_loop.exit();
    }

    fn scroll_by(&mut self, dy: f32) {
        let old = self.scroll;
        self.scroll = (self.scroll - dy).clamp(0.0, self.max_scroll.max(0.0));
        if self.scroll != old {
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
        match (name, self.mods.ctrl) {
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
            ));
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
        self.window = Some(window);
        self.sync_style();
        self.sync_frost();
        self.draw();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Read(r) => self.read_done(r),
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
                }
                self.redraw();
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
                self.sync_style();
                self.sync_frost();
                self.redraw();
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
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
                if let Some(f) = &mut self.dialog {
                    let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
                    if f.wheel(x, y, dy) {
                        self.redraw();
                    }
                } else {
                    self.scroll_by(dy);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(event_loop, &event),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match &self.toast {
            Some((_, until)) if Instant::now() >= *until => {
                self.toast = None;
                self.redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some((_, until)) => event_loop.set_control_flow(ControlFlow::WaitUntil(*until)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}
