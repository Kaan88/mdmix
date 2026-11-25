//! The live playback level: a thread of its own polls Windows' peak meter of the G6's playback endpoint and hands the
//! window one reading at a time through its event-loop proxy.
//!
//! The thread is the only place the meter is opened and read: `g6_osaudio::OutputMeter` holds COM pointers and is
//! neither `Send` nor `Sync`, and opening it joins that thread to COM's multithreaded apartment.
//!
//! It runs only while the Mixer page is shown and the window is visible (not minimised or hidden): the window starts
//! it on a page switch and on a restore, and stops it when the page is left or the window goes away, so the endpoint
//! isn't polled for a page nobody sees. While the level is silent the thread keeps polling; the window just stops
//! asking for frames once the meters are at rest ([`Meters::next_wake`], folded into the loop's `WaitUntil`).
//!
//! Reduced motion does not stop the meter: a level is data, not decoration.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mdshared_ui::meter::LevelMeter;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;
use crate::model::Page;

/// How often the thread reads the endpoint's peak: 30 Hz, as the plan's audio thread does.
pub const POLL: Duration = Duration::from_millis(33);

/// How long the thread waits before trying to open a meter that was not there (the G6 plugged in after the page was
/// opened, or another program holding the endpoint).
const RETRY: Duration = Duration::from_secs(2);

/// The caption over a strip whose level has no source.
pub const NO_METER: &str = "no meter";

/// The live meter runs only while the Mixer page is shown in a window a person can see.
pub fn should_run(page: Page, visible: bool) -> bool {
    page == Page::Mixer && visible
}

/// What the meter thread tells the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeterEvent {
    /// The endpoint's meter is open: the strips may show levels from now on.
    Opened,
    /// The peak of the left and right channels, 0.0..=1.0 linear. The same reading for both on a mono endpoint.
    Peaks { left: f32, right: f32 },
    /// There is no meter: it could not be opened, the thread was stopped, or the endpoint went away.
    Unavailable,
}

/// The window's side of the meter: whether a source is open, and the ballistics of the two channels.
///
/// The ballistics are the toolkit's [`LevelMeter`]: a level that jumps up to a reading and falls back, and a held
/// peak that falls after a while. Every reading is stamped with the moment it arrived, so a frame drawn between
/// readings shows the level as it has fallen since.
#[derive(Clone, Debug)]
pub struct Meters {
    live: bool,
    left: LevelMeter,
    right: LevelMeter,
}

impl Meters {
    /// No source: a strip that has no meter at all (this platform, the G6 away, the thread stopped). The window draws
    /// a flat track with the words [`NO_METER`], never a level.
    pub fn none() -> Meters {
        Meters {
            live: false,
            left: LevelMeter::new(),
            right: LevelMeter::new(),
        }
    }

    /// A fixed state for `--shot`: two readings at two fixed moments, a held peak 300 ms ago and a quieter level at
    /// `now`, so the picture does not depend on audio and shows the held line above the bar.
    pub fn busy(now: Instant) -> Meters {
        let mut m = Meters::none();
        m.live = true;
        let held = now.checked_sub(Duration::from_millis(300)).unwrap_or(now);
        m.left.update(held, 0.8);
        m.right.update(held, 0.6);
        m.left.update(now, 0.45);
        m.right.update(now, 0.3);
        m
    }

    /// Takes one event from the thread at `now`. True when the picture changes: a source appeared or went, or a
    /// meter is still moving. A silent reading once the meters are at rest changes nothing, so the window does not
    /// redraw for it.
    pub fn apply(&mut self, event: MeterEvent, now: Instant) -> bool {
        match event {
            MeterEvent::Opened => {
                let was = self.live;
                self.live = true;
                !was
            }
            MeterEvent::Peaks { left, right } => {
                let was = self.live;
                self.live = true;
                self.left.update(now, left);
                self.right.update(now, right);
                !was || !self.at_rest(now)
            }
            MeterEvent::Unavailable => {
                let was = self.live;
                self.live = false;
                self.left.reset();
                self.right.reset();
                was
            }
        }
    }

    /// The two channels to draw with, or `None` when there is no source.
    pub fn source(&self) -> Option<(&LevelMeter, &LevelMeter)> {
        self.live.then_some((&self.left, &self.right))
    }

    /// Both channels at the floor at `now`: nothing will move until the next reading.
    pub fn at_rest(&self, now: Instant) -> bool {
        self.left.at_rest(now) && self.right.at_rest(now)
    }

    /// When the loop should next wake to draw the fall, or `None` when there is no source or it is at rest.
    pub fn next_wake(&self, now: Instant, interval: Duration) -> Option<Instant> {
        if !self.live {
            return None;
        }
        match (
            self.left.next_wake(now, interval),
            self.right.next_wake(now, interval),
        ) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }
}

impl Default for Meters {
    fn default() -> Meters {
        Meters::none()
    }
}

/// What the window asks of the meter thread.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    /// Open the endpoint's meter and poll it.
    Start,
    /// Stop polling and let the meter go.
    Stop,
}

/// The window's handle to the meter thread.
pub struct LevelsHandle {
    tx: Option<Sender<Cmd>>,
    join: Option<JoinHandle<()>>,
    running: bool,
}

impl LevelsHandle {
    /// Starts the thread. It waits until it is asked to run, so nothing is opened at start-up.
    pub fn start(proxy: EventLoopProxy<UserEvent>) -> LevelsHandle {
        let (tx, rx) = mpsc::channel();
        let join = match thread::Builder::new()
            .name("g6-meter".into())
            .spawn(move || run(rx, proxy))
        {
            Ok(j) => Some(j),
            Err(e) => {
                eprintln!("g6-gui: the meter thread did not start: {e}");
                None
            }
        };
        LevelsHandle {
            tx: Some(tx),
            join,
            running: false,
        }
    }

    /// Starts or stops the polling. A request the thread is already on is not sent again, so a page drawn at 60 Hz
    /// doesn't fill its channel.
    pub fn set_wanted(&mut self, want: bool) {
        if want == self.running {
            return;
        }
        self.running = want;
        if let Some(tx) = &self.tx {
            let _ = tx.send(if want { Cmd::Start } else { Cmd::Stop });
        }
    }

    /// Stops the polling and waits for the thread to leave: a poll interval at most, or the end of an open.
    pub fn stop(&mut self) {
        self.set_wanted(false);
        // Hanging up also wakes a thread that is waiting for a start (it opened nothing).
        self.tx = None;
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// The thread: wait for `Start`, open the endpoint's meter on this thread, then read it 30 times a second until
/// `Stop`. An open that fails is retried every [`RETRY`] while it is still wanted, so plugging the G6 in shows the
/// meter without leaving the page.
fn run(rx: Receiver<Cmd>, proxy: EventLoopProxy<UserEvent>) {
    // The meter's COM pointers live on this thread, for as long as the meter is open.
    let mut meter: Option<g6_osaudio::OutputMeter> = None;
    loop {
        if meter.is_none() {
            // Stopped: wait for a start without opening anything.
            match rx.recv() {
                Ok(Cmd::Start) => {}
                Ok(Cmd::Stop) => continue,
                Err(_) => return,
            }
            // Open, and while it can't be opened say so and try again: the G6 may be plugged in while the page is
            // open. A Stop during that wait leaves the thread stopped, waiting for the next Start.
            loop {
                match g6_osaudio::open_output_meter() {
                    Some(m) => {
                        meter = Some(m);
                        if proxy
                            .send_event(UserEvent::Levels(MeterEvent::Opened))
                            .is_err()
                        {
                            return;
                        }
                        break;
                    }
                    None => {
                        if proxy
                            .send_event(UserEvent::Levels(MeterEvent::Unavailable))
                            .is_err()
                        {
                            return;
                        }
                        match rx.recv_timeout(RETRY) {
                            Ok(Cmd::Stop) => break,
                            Ok(Cmd::Start) | Err(RecvTimeoutError::Timeout) => continue,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                }
            }
            continue;
        }
        match rx.recv_timeout(POLL) {
            Ok(Cmd::Stop) => {
                // Dropping the meter here leaves COM's apartment before the thread does anything else with it.
                meter = None;
                if proxy
                    .send_event(UserEvent::Levels(MeterEvent::Unavailable))
                    .is_err()
                {
                    return;
                }
            }
            Ok(Cmd::Start) => {}
            Err(RecvTimeoutError::Timeout) => {
                let Some(m) = &meter else { continue };
                let [left, right] = m.peaks();
                let event = MeterEvent::Peaks {
                    left: left as f32,
                    right: right as f32,
                };
                if proxy.send_event(UserEvent::Levels(event)).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_runs_only_on_the_mixer_page_of_a_visible_window() {
        assert!(should_run(Page::Mixer, true));
        for page in Page::ALL.into_iter().filter(|p| *p != Page::Mixer) {
            assert!(!should_run(page, true), "{page:?}");
        }
        assert!(!should_run(Page::Mixer, false));
        assert!(!should_run(Page::Sbx, false));
    }

    /// The handle sends one command per change, which is what the window's page switch and minimise/restore call
    /// does: leaving the Mixer page sends the thread a Stop and nothing else.
    #[test]
    fn the_handle_starts_and_stops_the_thread_once_per_change() {
        let (tx, rx) = mpsc::channel();
        let mut handle = LevelsHandle {
            tx: Some(tx),
            join: None,
            running: false,
        };
        handle.set_wanted(false);
        handle.set_wanted(true);
        handle.set_wanted(true);
        handle.set_wanted(false);
        handle.set_wanted(false);
        let sent: Vec<Cmd> = rx.try_iter().collect();
        assert_eq!(sent, [Cmd::Start, Cmd::Stop]);
    }

    #[test]
    fn no_source_is_no_meter_and_never_a_deadline() {
        let t0 = Instant::now();
        let m = Meters::none();
        assert!(m.source().is_none());
        assert!(m.at_rest(t0));
        assert_eq!(m.next_wake(t0, POLL), None);
        assert_eq!(Meters::default().source(), None);
    }

    /// The app's update path: a reading starts the fall, silent readings keep it going, and once both the level and
    /// the held peak are on the floor the loop is given no deadline while the thread keeps polling.
    #[test]
    fn readings_through_the_app_then_silence_leave_no_deadline_once_at_rest() {
        let t0 = Instant::now();
        let mut m = Meters::none();
        // The first reading is the picture changing from "no meter" to a level: one frame.
        assert!(m.apply(
            MeterEvent::Peaks {
                left: 0.5,
                right: 0.5
            },
            t0
        ));
        assert!(m.source().is_some());
        assert!(m.next_wake(t0, POLL).is_some());

        // A louder, unequal reading moves both channels.
        let t1 = t0 + POLL;
        assert!(m.apply(
            MeterEvent::Peaks {
                left: 1.0,
                right: 0.25
            },
            t1
        ));
        assert!(m.next_wake(t1, POLL).is_some());

        // Silence from here: the thread keeps sending readings every 33 ms.
        let mut now = t1;
        let mut asked = 0;
        for _ in 0..400 {
            now += POLL;
            if m.apply(
                MeterEvent::Peaks {
                    left: 0.0,
                    right: 0.0,
                },
                now,
            ) {
                asked += 1;
            }
            if m.next_wake(now, POLL).is_none() {
                break;
            }
        }
        assert!(asked > 0, "the fall must be drawn");
        assert!(m.at_rest(now));
        assert_eq!(m.next_wake(now, POLL), None);
        // Further silent readings change nothing and ask for no frame.
        for _ in 0..10 {
            now += POLL;
            assert!(!m.apply(
                MeterEvent::Peaks {
                    left: 0.0,
                    right: 0.0
                },
                now
            ));
            assert_eq!(m.next_wake(now, POLL), None);
        }
    }

    #[test]
    fn the_source_appearing_and_going_changes_the_picture_once() {
        let t0 = Instant::now();
        let mut m = Meters::none();
        assert!(m.apply(MeterEvent::Opened, t0));
        assert!(!m.apply(MeterEvent::Opened, t0));
        assert!(m.source().is_some());
        // The thread stopped, or the endpoint went: no source, nothing to wake for.
        assert!(m.apply(MeterEvent::Unavailable, t0));
        assert!(!m.apply(MeterEvent::Unavailable, t0));
        assert!(m.source().is_none());
        assert_eq!(m.next_wake(t0, POLL), None);
    }

    #[test]
    fn a_fixed_state_for_a_shot_has_a_level_and_a_peak() {
        let t0 = Instant::now();
        let m = Meters::busy(t0);
        let (l, r) = m.source().expect("a source");
        assert!(l.fraction(t0) > 0.5 && r.fraction(t0) > 0.3);
        assert!(l.peak_fraction(t0) > l.fraction(t0), "the hold line shows");
        assert!(r.peak_fraction(t0) >= r.fraction(t0));
        assert!(m.next_wake(t0, POLL).is_some(), "a shot's meter is moving");
    }
}
