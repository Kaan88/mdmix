//! The write path on the UI side (step G3 of `doc/gui-plan.md`): intents in, worker commands out, read-backs
//! reconciled. No window and no thread here, so the tests drive it with a fake device.
//!
//! - **Intent:** one user action is one [`Setting`] and a [`Val`]; it becomes the CLI's action for that flag.
//! - **Pending:** what was asked for is shown until a read-back answers it ([`Desk::shown`]).
//! - **Throttle:** a slider being dragged sends at most one write per [`THROTTLE`]; the value in between waits, a newer
//!   one replaces it, and the release always sends the last value.
//! - **Reconcile:** a read-back equal to the pending value clears it with a toast ("SBX Surround set to 62 %"). A
//!   refused write, or a read-back with another value, drops the pending value, so the control springs back to what
//!   the device has, with an error toast. A side effect on another setting (turning off the last SBX effect turns SBX
//!   off) is just the read-back: nothing pending, nothing to reconcile.
//! - **Settle:** a read-back that leaves the setting unanswered (the G6 settling after an output switch) keeps the
//!   requested value pending and schedules reads at 1 s, 3 s and 7 s; at 8 s it stops and says once that the G6
//!   hasn't confirmed it. No "was not set" toast comes from silence.
//! - **Undo:** the last confirmed single setting, re-sent with the value from before as a new write.
//! - **Away:** a turn whose status read fails drops every pending value; the model says the G6 is away.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::intent::{Setting, Val};
use crate::model::{Conn, Model};
use crate::worker::{Cmd, ReadResult, Write};

/// The least time between two writes of a dragged slider (the mockup's figure; to tune on the device).
pub const THROTTLE: Duration = Duration::from_millis(80);

/// Where a value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A click: a switch, a segment, a dropdown's choice. Sent at once.
    Click,
    /// A slider moving under the pointer. Throttled.
    Drag,
    /// The slider let go: its last value is sent.
    Release,
}

/// A value asked for and not yet read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub val: Val,
    /// The device's value before the gesture started, for Undo.
    pub before: Option<Val>,
    /// It is Undo re-sending an old value.
    pub undo: bool,
}

/// The slider in hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    setting: Setting,
    /// When the last write of the drag went to the worker, and its value.
    sent: Option<(Instant, Val)>,
    /// A value that came too soon after the last write: sent when its time comes.
    waiting: Option<Val>,
}

/// What Undo would do: set `setting` back from `after` to `before`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Undo {
    pub setting: Setting,
    pub before: Val,
    pub after: Val,
}

/// A pending value whose read-back left the setting unanswered: follow-up reads at growing gaps, then one toast.
/// `start` is when the first read-back came back without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settle {
    start: Instant,
    reads: u8,
}

/// When each follow-up read is due, counted from `start`: the gaps grow (1 s, then 2 s, then 4 s).
const SETTLE_AT: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(7),
];
/// When waiting longer stops being useful: the desk says once that the G6 hasn't confirmed the change.
const SETTLE_GIVE_UP: Duration = Duration::from_secs(8);

#[derive(Debug, Default)]
pub struct Desk {
    pub pending: BTreeMap<Setting, Pending>,
    held: Option<Held>,
    pub undo: Option<Undo>,
    /// The pending values a read left unanswered, with their follow-up schedule.
    settling: BTreeMap<Setting, Settle>,
}

/// The toast after a write: what happened, worded for a person, and the setting it is about. A second toast about
/// the same setting replaces the first in place; one about nothing (a read, a general refusal) stands on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub setting: Option<Setting>,
}

impl Toast {
    /// A toast not about one setting.
    pub fn plain(text: impl Into<String>) -> Toast {
        Toast {
            text: text.into(),
            setting: None,
        }
    }

    /// A toast about what was done to one setting.
    pub fn about(setting: Setting, text: impl Into<String>) -> Toast {
        Toast {
            text: text.into(),
            setting: Some(setting),
        }
    }
}

impl From<String> for Toast {
    fn from(text: String) -> Toast {
        Toast::plain(text)
    }
}

impl From<&str> for Toast {
    fn from(text: &str) -> Toast {
        Toast::plain(text)
    }
}

impl Desk {
    /// The model as drawn: the device's values with the pending ones over them.
    pub fn shown(&self, m: &Model) -> Model {
        let mut out = m.clone();
        for (s, p) in &self.pending {
            s.put(p.val, &mut out);
        }
        out
    }

    /// The slider being dragged, if any.
    pub fn dragging(&self) -> Option<Setting> {
        self.held.map(|h| h.setting)
    }

    /// The window stopped a drag without a release (a colour picker closed under the marker it held): a value that
    /// was waiting out the throttle is dropped, so it cannot go out after the fact and overwrite what went since.
    pub fn abandon(&mut self, s: Setting) {
        if self.held.is_some_and(|h| h.setting == s) {
            self.held = None;
        }
    }

    /// A user action. Returns the command for the worker, if one is to go now, or the reason nothing can be sent.
    pub fn intent(
        &mut self,
        m: &Model,
        s: Setting,
        v: Val,
        phase: Phase,
        now: Instant,
    ) -> Result<Option<Cmd>, Toast> {
        self.intent_as(m, s, v, phase, now, false)
    }

    fn intent_as(
        &mut self,
        m: &Model,
        s: Setting,
        v: Val,
        phase: Phase,
        now: Instant,
        undo: bool,
    ) -> Result<Option<Cmd>, Toast> {
        if m.conn != Conn::Connected || !m.have_status {
            self.held = None;
            return Err(Toast::about(
                s,
                "The G6 is not connected, so nothing was changed.",
            ));
        }
        // The action is built now, so a value that can't be written is said at once and never shown as pending.
        s.action(v, &self.shown(m))
            .map_err(|e| Toast::about(s, e))?;
        if phase == Phase::Click && !self.pending.contains_key(&s) && s.current(m) == Some(v) {
            return Ok(None);
        }
        let before = self
            .pending
            .get(&s)
            .map_or_else(|| s.current(m), |p| p.before);
        self.pending.insert(
            s,
            Pending {
                val: v,
                before,
                undo,
            },
        );
        // A new value replaces whatever the last one was waiting for.
        self.settling.remove(&s);
        match phase {
            Phase::Click => {
                self.held = None;
                self.send(m, s, v).map(Some)
            }
            Phase::Drag => {
                let held = match self.held {
                    Some(h) if h.setting == s => h,
                    _ => Held {
                        setting: s,
                        sent: None,
                        waiting: None,
                    },
                };
                self.held = Some(held);
                let due = held.sent.is_none_or(|(at, _)| now >= at + THROTTLE);
                if due {
                    self.sent_now(s, v, now);
                    self.send(m, s, v).map(Some)
                } else {
                    if let Some(h) = &mut self.held {
                        h.waiting = Some(v);
                    }
                    Ok(None)
                }
            }
            Phase::Release => {
                // The last value always goes, even when the drag sent it already: its read-back settles the control.
                self.held = None;
                self.send(m, s, v).map(Some)
            }
        }
    }

    fn sent_now(&mut self, s: Setting, v: Val, now: Instant) {
        if let Some(h) = &mut self.held
            && h.setting == s
        {
            h.sent = Some((now, v));
            h.waiting = None;
        }
    }

    fn send(&self, m: &Model, s: Setting, v: Val) -> Result<Cmd, Toast> {
        let action = s
            .action(v, &self.shown(m))
            .map_err(|e| Toast::about(s, e))?;
        Ok(Cmd::Write(Write {
            setting: s,
            val: v,
            action,
        }))
    }

    /// When the waiting value of a drag is due, for the window's timer.
    pub fn due(&self) -> Option<Instant> {
        let h = self.held?;
        h.waiting?;
        Some(h.sent.map_or_else(Instant::now, |(at, _)| at + THROTTLE))
    }

    /// The waiting value of a drag, once its time has come.
    pub fn tick(&mut self, m: &Model, now: Instant) -> Option<Cmd> {
        let h = self.held?;
        let v = h.waiting?;
        if h.sent.is_some_and(|(at, _)| now < at + THROTTLE) {
            return None;
        }
        self.sent_now(h.setting, v, now);
        self.send(m, h.setting, v).ok()
    }

    /// Ctrl+Z: the last confirmed setting goes back to the value it had, as a new write.
    pub fn undo(&mut self, m: &Model, now: Instant) -> Result<Option<Cmd>, Toast> {
        let Some(u) = self.undo.take() else {
            return Err(Toast::plain("Nothing to undo."));
        };
        if u.setting.current(m) != Some(u.after) {
            return Err(Toast::about(
                u.setting,
                format!(
                    "{} has changed since, so there is nothing to undo.",
                    u.setting.label()
                ),
            ));
        }
        self.intent_as(m, u.setting, u.before, Phase::Click, now, true)
    }

    /// A turn of the worker came back: the model takes the read, and each write is settled against it. Returns the
    /// toast to show, if any (an error wins over a confirmation). A read-back that leaves a setting unanswered keeps
    /// its requested value pending and schedules follow-up reads (`follow_up`); nothing is said until one answers.
    pub fn reconcile(&mut self, m: &mut Model, r: ReadResult, now: Instant) -> Option<Toast> {
        let why = r.status.as_ref().err().cloned();
        let wrote = !r.writes.is_empty();
        let about = r.writes.first().map(|w| w.setting);
        m.apply(r.status, r.mixer);
        if let Some(why) = why {
            let had = !self.pending.is_empty();
            self.pending.clear();
            self.held = None;
            self.settling.clear();
            return if wrote || had {
                Some(Toast {
                    text: format!("The G6 is not connected, so the change was not made. {why}"),
                    setting: about,
                })
            } else {
                // A read the user asked for: its answer is the reason.
                r.user.then(|| Toast::plain(why))
            };
        }
        let mut good = None;
        let mut bad = None;
        for w in r.writes {
            let Some(p) = self.pending.get(&w.setting).copied() else {
                continue;
            };
            // A newer value was asked for since this one was sent: it is still on its way.
            if p.val != w.val {
                continue;
            }
            if let Err(e) = w.result {
                let label = w.setting.label();
                self.pending.remove(&w.setting);
                self.settling.remove(&w.setting);
                bad = Some(Toast::about(
                    w.setting,
                    format!("{label} was not changed: {e}"),
                ));
                continue;
            }
            if self.dragging() == Some(w.setting) {
                // The drag goes on; its release settles it.
                continue;
            }
            self.answer(m, w.setting, p, now, &mut good, &mut bad);
        }
        // A follow-up read carries no writes: it is here to answer the values still settling.
        self.settle_unanswered(m, now, &mut good, &mut bad);
        if r.user && bad.is_none() && good.is_none() {
            good = Some(Toast::plain("Status read from the G6."));
        }
        bad.or(good)
    }

    /// One pending value against the model: confirmed, refused, or still unanswered. A readable setting the G6 did
    /// not answer for stays pending and starts (or keeps) its follow-up schedule; no toast is made for that.
    fn answer(
        &mut self,
        m: &mut Model,
        s: Setting,
        p: Pending,
        now: Instant,
        good: &mut Option<Toast>,
        bad: &mut Option<Toast>,
    ) {
        match s.confirmed(p.val, m) {
            Some(true) => {
                self.pending.remove(&s);
                self.settling.remove(&s);
                *good = Some(Toast::about(s, confirmation(s, p.val, p.undo, false)));
                self.remember(s, p, p.val);
            }
            None if s.unreadable() => {
                s.put(p.val, m);
                self.pending.remove(&s);
                self.settling.remove(&s);
                *good = Some(Toast::about(s, confirmation(s, p.val, p.undo, true)));
                self.remember(s, p, p.val);
            }
            None => {
                self.settling.entry(s).or_insert(Settle {
                    start: now,
                    reads: 0,
                });
            }
            Some(false) => {
                let has = s
                    .current(m)
                    .map_or_else(|| "something else".into(), |c| s.text(c));
                self.pending.remove(&s);
                self.settling.remove(&s);
                *bad = Some(Toast::about(
                    s,
                    format!(
                        "{} was not set to {}: the G6 has {has}.",
                        s.label(),
                        s.text(p.val)
                    ),
                ));
            }
        }
    }

    /// Re-checks every value still waiting for an answer against a read that just came in.
    fn settle_unanswered(
        &mut self,
        m: &mut Model,
        now: Instant,
        good: &mut Option<Toast>,
        bad: &mut Option<Toast>,
    ) {
        for s in self.settling.keys().copied().collect::<Vec<_>>() {
            if self.dragging() == Some(s) {
                continue;
            }
            let Some(p) = self.pending.get(&s).copied() else {
                self.settling.remove(&s);
                continue;
            };
            self.answer(m, s, p, now, good, bad);
        }
    }

    /// When a follow-up read (or the give-up) is due, for the window's timer. `None` when nothing is settling, so an
    /// idle window waits for an event.
    pub fn settle_due(&self) -> Option<Instant> {
        let mut due: Option<Instant> = None;
        for st in self.settling.values() {
            if let Some(at) = SETTLE_AT.get(usize::from(st.reads)) {
                let at = st.start + *at;
                due = Some(due.map_or(at, |d| d.min(at)));
            }
            let at = st.start + SETTLE_GIVE_UP;
            due = Some(due.map_or(at, |d| d.min(at)));
        }
        due
    }

    /// The follow-up read when one is due now: one read serves every setting that is waiting. `None` before its time.
    pub fn follow_up(&mut self, now: Instant) -> Option<Cmd> {
        let mut due = false;
        for st in self.settling.values_mut() {
            if let Some(at) = SETTLE_AT.get(usize::from(st.reads))
                && now >= st.start + *at
            {
                st.reads += 1;
                due = true;
            }
        }
        due.then_some(Cmd::Read { user: false })
    }

    /// The G6 stayed silent past the last follow-up: drop the pending value and say so once. What the device last
    /// reported stays on screen; when it never reported one, the value that was asked for stands.
    pub fn timed_out(&mut self, m: &mut Model, now: Instant) -> Option<Toast> {
        let due: Vec<Setting> = self
            .settling
            .iter()
            .filter(|(_, st)| now >= st.start + SETTLE_GIVE_UP)
            .map(|(s, _)| *s)
            .collect();
        let mut toast = None;
        for s in due {
            self.settling.remove(&s);
            let Some(p) = self.pending.remove(&s) else {
                continue;
            };
            if s.current(m).is_none() {
                s.put(p.val, m);
            }
            toast = Some(Toast::about(
                s,
                format!("{}: the G6 hasn't confirmed the change yet.", s.label()),
            ));
        }
        toast
    }

    fn remember(&mut self, s: Setting, p: Pending, after: Val) {
        self.undo = match p.before {
            Some(before) if s.undoable() && !p.undo && before != after => Some(Undo {
                setting: s,
                before,
                after,
            }),
            // Undoing is not undone again; a setting without Undo leaves none behind.
            _ => None,
        };
    }
}

/// "SBX Surround set to 62 %", "Direct mode turned on", with "Undone:" before an Undo and the Ctrl+Z hint after a
/// change that can be undone. `unread`: the G6 didn't say what it has now.
fn confirmation(s: Setting, v: Val, undo: bool, unread: bool) -> String {
    let mut what = match v {
        Val::On(true) => format!("{} turned on", s.label()),
        Val::On(false) => format!("{} turned off", s.label()),
        Val::Int(_) => format!("{} set to {}", s.label(), s.text(v)),
    };
    if unread {
        what.push_str(" (not read back: the G6 doesn't report it)");
    }
    if undo {
        format!("Undone: {what}.")
    } else if s.undoable() {
        format!("{what}. Ctrl+Z undoes it.")
    } else {
        format!("{what}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::Sample;
    use crate::worker::{Device, Queue, serve};
    use g6_proto::packet::HidReport;
    use g6_proto::status::DeviceStatus;

    /// A G6 that records every report, applies what it was asked (unless told not to) and can refuse or vanish.
    struct Fake {
        status: DeviceStatus,
        sent: Vec<HidReport>,
        writes: Vec<(Setting, Val)>,
        refuse: bool,
        stubborn: bool,
        gone: bool,
        /// The next reads leave the output out, as the G6 does while it settles after a switch.
        silent_output: usize,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                status: Sample::Busy.model().status,
                sent: Vec::new(),
                writes: Vec::new(),
                refuse: false,
                stubborn: false,
                gone: false,
                silent_output: 0,
            }
        }
    }

    impl Device for Fake {
        fn send(&mut self, w: &Write, reports: &[HidReport]) -> Result<(), String> {
            if self.gone {
                return Err("No G6 could be found.".into());
            }
            if self.refuse {
                return Err("the write failed".into());
            }
            self.sent.extend_from_slice(reports);
            self.writes.push((w.setting, w.val));
            if !self.stubborn {
                let mut m = Model::with_status(self.status.clone(), None);
                w.setting.put(w.val, &mut m);
                self.status = m.status;
            }
            Ok(())
        }
        fn read(&mut self) -> Result<DeviceStatus, String> {
            if self.gone {
                Err("No G6 could be found.".into())
            } else {
                let mut s = self.status.clone();
                if self.silent_output > 0 {
                    self.silent_output = self.silent_output.saturating_sub(1);
                    s.output = None;
                }
                Ok(s)
            }
        }
        fn mixer(&mut self) -> Option<crate::model::MixerView> {
            None
        }
    }

    /// The window and its worker, without either: a model, the desk and the fake.
    struct Rig {
        m: Model,
        desk: Desk,
        dev: Fake,
        t: Instant,
        toast: Option<String>,
    }

    impl Rig {
        fn new() -> Rig {
            let dev = Fake::new();
            Rig {
                m: Model::with_status(dev.status.clone(), None),
                desk: Desk::default(),
                dev,
                t: Instant::now(),
                toast: None,
            }
        }

        /// The worker's turn for these commands.
        fn run(&mut self, cmds: Vec<Cmd>) {
            if cmds.is_empty() {
                return;
            }
            let mut q = Queue::default();
            for c in cmds {
                q.add(c);
            }
            let r = serve(&mut self.dev, &q);
            self.toast = self.desk.reconcile(&mut self.m, r, self.t).map(|t| t.text);
        }

        fn act(&mut self, s: Setting, v: Val, phase: Phase) -> Option<Cmd> {
            match self.desk.intent(&self.m, s, v, phase, self.t) {
                Ok(c) => c,
                Err(t) => {
                    self.toast = Some(t.text);
                    None
                }
            }
        }

        fn click(&mut self, s: Setting, v: Val) {
            let c = self.act(s, v, Phase::Click);
            self.run(c.into_iter().collect());
        }

        fn later(&mut self, ms: u64) {
            self.t += Duration::from_millis(ms);
        }

        /// The window's `about_to_wait`: a follow-up read whose time has come goes now. True when one went.
        fn follow(&mut self) -> bool {
            let Some(cmd) = self.desk.follow_up(self.t) else {
                return false;
            };
            self.run(vec![cmd]);
            true
        }
    }

    fn hex(r: &HidReport) -> String {
        g6_proto::packet::hex(r).trim_end_matches('0').to_string()
    }

    /// The HID reports `tests/vectors/writes.json` keeps for this `g6` command line.
    fn cli_reports(argv: &[&str]) -> Vec<String> {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/vectors/writes.json"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let c = v["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap())
                    .eq(argv.iter().copied())
            })
            .unwrap_or_else(|| panic!("no case for {argv:?}"));
        c["hid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap().trim_end_matches('0').to_string())
            .collect()
    }

    #[test]
    fn a_click_sends_exactly_the_cli_reports() {
        let mut rig = Rig::new();
        rig.click(Setting::DirectMode, Val::On(true));
        rig.click(Setting::EffectLevel(0), Val::Int(42));
        rig.click(Setting::MicBoost, Val::Int(30));
        let got: Vec<String> = rig.dev.sent.iter().map(hex).collect();
        // From tests/vectors/writes.json: --playback-direct-mode Enabled, --sbx-surround-value 42,
        // --recording-mic-boost-db 20.
        let mut want = cli_reports(&["--playback-direct-mode", "Enabled"]);
        want.extend(cli_reports(&["--sbx-surround-value", "42"]));
        want.extend(cli_reports(&["--recording-mic-boost-db", "30"]));
        assert_eq!(got, want);
        assert!(rig.desk.pending.is_empty());
        assert_eq!(rig.m.status.mic_boost_db, Some(30));
        assert_eq!(
            rig.toast.as_deref(),
            Some("Mic boost set to 30 dB. Ctrl+Z undoes it.")
        );
    }

    #[test]
    fn the_pending_value_is_shown_until_the_read_back_clears_it() {
        let mut rig = Rig::new();
        let before = rig
            .m
            .status
            .sbx_effect(g6_proto::types::SbxEffect::Surround)
            .unwrap()
            .level;
        let c = rig.act(Setting::EffectLevel(0), Val::Int(62), Phase::Click);
        assert!(c.is_some());
        // Not read back yet: the model is the device's, the picture the pending value.
        assert_eq!(
            Setting::EffectLevel(0).current(&rig.m),
            before.map(Val::Int)
        );
        assert_eq!(
            Setting::EffectLevel(0).current(&rig.desk.shown(&rig.m)),
            Some(Val::Int(62))
        );
        rig.run(c.into_iter().collect());
        assert!(rig.desk.pending.is_empty());
        assert_eq!(Setting::EffectLevel(0).current(&rig.m), Some(Val::Int(62)));
        assert_eq!(
            rig.toast.as_deref(),
            Some("SBX Surround set to 62 %. Ctrl+Z undoes it.")
        );
    }

    #[test]
    fn a_toast_says_which_setting_it_is_about_so_it_can_take_that_one_s_place() {
        let mut rig = Rig::new();
        let c = rig.act(Setting::MicBoost, Val::Int(30), Phase::Click);
        let mut q = Queue::default();
        q.add(c.expect("a write"));
        let r = serve(&mut rig.dev, &q);
        let t = rig
            .desk
            .reconcile(&mut rig.m, r, rig.t)
            .expect("a confirmation");
        assert_eq!(t.setting, Some(Setting::MicBoost));
        assert!(t.text.starts_with("Mic boost set to 30 dB."), "{}", t.text);
        // A read is about no setting: it stands on its own.
        let mut q = Queue::default();
        q.add(Cmd::Read { user: true });
        let r = serve(&mut rig.dev, &q);
        let t = rig
            .desk
            .reconcile(&mut rig.m, r, rig.t)
            .expect("a read's answer");
        assert_eq!(t.setting, None);
        assert_eq!(t.text, "Status read from the G6.");
    }

    #[test]
    fn a_drag_is_throttled_and_the_release_sends_the_last_value() {
        let mut rig = Rig::new();
        let s = Setting::EffectLevel(1);
        let mut cmds = Vec::new();
        // 100 moves, 4 ms apart: 400 ms of dragging.
        for i in 0..100 {
            cmds.extend(rig.act(s, Val::Int(i), Phase::Drag));
            rig.later(4);
            cmds.extend(rig.desk.tick(&rig.m, rig.t));
        }
        let during = cmds.len();
        // At most one write per 80 ms (the first goes at once), and every one of them the latest value then.
        assert!((2..=6).contains(&during), "{during} writes");
        cmds.extend(rig.act(s, Val::Int(99), Phase::Release));
        let last = cmds.last().cloned();
        assert!(
            matches!(
                last,
                Some(Cmd::Write(Write {
                    val: Val::Int(99),
                    ..
                }))
            ),
            "{last:?}"
        );
        assert!(rig.desk.dragging().is_none());
        rig.run(cmds);
        // The worker merged what waited: one write, the last value, and it is read back.
        assert_eq!(rig.dev.writes, [(s, Val::Int(99))]);
        assert!(rig.desk.pending.is_empty());
        assert_eq!(s.current(&rig.m), Some(Val::Int(99)));
    }

    #[test]
    fn a_waiting_drag_value_is_sent_when_its_time_comes() {
        let mut rig = Rig::new();
        let s = Setting::EqBand(3);
        assert!(rig.act(s, Val::Int(10), Phase::Drag).is_some());
        assert!(rig.act(s, Val::Int(20), Phase::Drag).is_none());
        assert!(rig.desk.due().is_some());
        rig.later(40);
        assert!(rig.desk.tick(&rig.m, rig.t).is_none());
        rig.later(41);
        let c = rig.desk.tick(&rig.m, rig.t);
        assert!(matches!(
            c,
            Some(Cmd::Write(Write {
                val: Val::Int(20),
                ..
            }))
        ));
        assert!(rig.desk.due().is_none());
        // A read-back of a write made during the drag leaves the value pending: the drag goes on.
        rig.run(c.into_iter().collect());
        assert!(rig.desk.pending.contains_key(&s));
        // A plain read doesn't settle a pending value either.
        rig.run(vec![Cmd::Read { user: false }]);
        assert!(rig.desk.pending.contains_key(&s));
        // Released on the value already sent: it goes again, and its read-back settles the control.
        let c = rig.act(s, Val::Int(20), Phase::Release);
        assert!(c.is_some());
        rig.run(c.into_iter().collect());
        assert!(rig.desk.pending.is_empty());
        assert_eq!(s.current(&rig.m), Some(Val::Int(20)));
    }

    #[test]
    fn a_refused_write_springs_back_with_a_toast() {
        let mut rig = Rig::new();
        rig.dev.refuse = true;
        let was = Setting::Eq.current(&rig.m);
        rig.click(Setting::Eq, Val::On(was != Some(Val::On(true))));
        assert!(rig.desk.pending.is_empty());
        assert_eq!(Setting::Eq.current(&rig.desk.shown(&rig.m)), was);
        assert_eq!(
            rig.toast.as_deref(),
            Some("Equalizer was not changed: the write failed")
        );
        assert!(rig.desk.undo.is_none());
    }

    #[test]
    fn a_read_back_with_another_value_springs_back_with_a_toast() {
        let mut rig = Rig::new();
        rig.dev.stubborn = true;
        let s = Setting::NoiseReduction;
        let was = s.current(&rig.m);
        rig.click(s, Val::Int(80));
        assert!(rig.desk.pending.is_empty());
        assert_eq!(s.current(&rig.desk.shown(&rig.m)), was);
        let t = rig.toast.clone().unwrap();
        assert!(
            t.starts_with("Noise reduction was not set to 80 %: the G6 has "),
            "{t}"
        );
    }

    #[test]
    fn a_side_effect_on_another_setting_is_just_the_read_back() {
        // On the G6, turning off the last SBX effect turns SBX off: the read-back says so and nothing springs back.
        let mut rig = Rig::new();
        let mut st = rig.dev.status.clone();
        let mut all = st.sbx.unwrap();
        for e in &mut all {
            e.enabled = Some(false);
        }
        all[0].enabled = Some(true);
        st.sbx = Some(all);
        st.sbx_enabled = Some(true);
        rig.dev.status = st.clone();
        rig.m = Model::with_status(st, None);
        let c = rig.act(Setting::EffectOn(0), Val::On(false), Phase::Click);
        rig.dev.status.sbx_enabled = Some(false);
        rig.run(c.into_iter().collect());
        assert_eq!(
            rig.toast.as_deref(),
            Some("SBX Surround turned off. Ctrl+Z undoes it.")
        );
        assert_eq!(rig.m.status.sbx_enabled, Some(false));
        assert!(rig.desk.pending.is_empty());
    }

    #[test]
    fn the_g6_gone_drops_every_pending_write_and_shows_it_away() {
        let mut rig = Rig::new();
        let a = rig.act(Setting::EffectLevel(2), Val::Int(5), Phase::Drag);
        let b = rig.act(Setting::Sbx, Val::On(false), Phase::Click);
        assert_eq!(rig.desk.pending.len(), 2);
        rig.dev.gone = true;
        rig.run(a.into_iter().chain(b).collect());
        assert!(rig.desk.pending.is_empty());
        assert!(rig.desk.dragging().is_none());
        assert!(matches!(rig.m.conn, Conn::Away(_)));
        assert!(
            rig.toast
                .clone()
                .unwrap()
                .starts_with("The G6 is not connected")
        );
        // Away, nothing is sent and the control isn't shown changed.
        rig.dev.gone = false;
        assert!(rig.act(Setting::Sbx, Val::On(true), Phase::Click).is_none());
        assert!(rig.desk.pending.is_empty());
        assert!(rig.toast.clone().unwrap().contains("not connected"));
        assert_eq!(rig.dev.writes.len(), 0);
    }

    #[test]
    fn undo_re_sends_the_old_value_as_a_new_write() {
        let mut rig = Rig::new();
        let s = Setting::EffectLevel(4);
        let was = s.current(&rig.m).unwrap();
        rig.click(s, Val::Int(7));
        assert_eq!(
            rig.desk.undo,
            Some(Undo {
                setting: s,
                before: was,
                after: Val::Int(7)
            })
        );
        let c = rig.desk.undo(&rig.m, rig.t).unwrap();
        assert!(matches!(c, Some(Cmd::Write(Write { val, .. })) if val == was));
        rig.run(c.into_iter().collect());
        assert_eq!(s.current(&rig.m), Some(was));
        assert_eq!(rig.dev.writes, [(s, Val::Int(7)), (s, was)]);
        assert!(
            rig.toast
                .clone()
                .unwrap()
                .starts_with("Undone: SBX Dialog+ set to")
        );
        // One step: nothing more to undo.
        assert!(rig.desk.undo(&rig.m, rig.t).is_err());
    }

    #[test]
    fn undo_is_not_offered_for_the_output_or_direct_mode() {
        let mut rig = Rig::new();
        // The busy sample is on Headphones.
        rig.click(Setting::Output, Val::Int(0));
        assert_eq!(rig.m.status.output.as_deref(), Some("Speakers"));
        assert!(rig.desk.undo.is_none());
        assert_eq!(rig.toast.as_deref(), Some("Output set to Speakers."));
        rig.click(Setting::DirectMode, Val::On(true));
        assert!(rig.desk.undo.is_none());
    }

    #[test]
    fn a_click_on_the_value_already_there_sends_nothing() {
        let mut rig = Rig::new();
        let v = Setting::Sbx.current(&rig.m).unwrap();
        assert!(rig.act(Setting::Sbx, v, Phase::Click).is_none());
        assert!(rig.desk.pending.is_empty());
    }

    #[test]
    fn an_unanswered_output_keeps_the_requested_value_and_is_read_again() {
        let mut rig = Rig::new();
        // The busy sample is on Headphones.
        assert_eq!(rig.m.status.output.as_deref(), Some("Headphones"));
        rig.dev.silent_output = usize::MAX;
        let c = rig
            .act(Setting::Output, Val::Int(0), Phase::Click)
            .expect("a write");
        rig.run(vec![c]);
        // The read-back left the output out: the old one stays in the model for the picture, marked unanswered.
        assert_eq!(rig.m.status.output.as_deref(), Some("Headphones"));
        assert!(rig.m.output_unanswered);
        assert_eq!(Setting::Output.confirmed(Val::Int(0), &rig.m), None);
        // The control shows what was asked for, pending, and nothing is said: it is not a refusal yet.
        assert!(rig.desk.pending.contains_key(&Setting::Output));
        assert_eq!(
            Setting::Output.current(&rig.desk.shown(&rig.m)),
            Some(Val::Int(0))
        );
        assert_eq!(rig.toast, None);
        // The follow-ups are 1 s, 3 s and 7 s after the write.
        let start = rig.t;
        assert_eq!(rig.desk.settle_due(), Some(start + Duration::from_secs(1)));
        rig.later(1000);
        assert!(rig.follow(), "the 1 s read is due");
        assert!(rig.desk.pending.contains_key(&Setting::Output));
        assert_eq!(rig.toast, None);
        assert_eq!(rig.desk.settle_due(), Some(start + Duration::from_secs(3)));
        rig.later(2000);
        assert!(rig.follow(), "the 3 s read is due");
        assert!(rig.desk.pending.contains_key(&Setting::Output));
        assert_eq!(rig.toast, None);
        assert_eq!(rig.desk.settle_due(), Some(start + Duration::from_secs(7)));
        // The G6 answers on the last follow-up: the switch is confirmed, with the ordinary toast.
        rig.dev.silent_output = 0;
        rig.later(4000);
        assert!(rig.follow(), "the 7 s read is due");
        assert!(rig.desk.pending.is_empty());
        assert!(!rig.m.output_unanswered);
        assert_eq!(rig.m.status.output.as_deref(), Some("Speakers"));
        assert_eq!(rig.toast.as_deref(), Some("Output set to Speakers."));
        assert_eq!(rig.desk.settle_due(), None);
    }

    #[test]
    fn a_g6_that_never_answers_says_so_once_and_never_calls_it_refused() {
        let mut rig = Rig::new();
        rig.dev.silent_output = usize::MAX;
        let c = rig
            .act(Setting::Output, Val::Int(0), Phase::Click)
            .expect("a write");
        rig.run(vec![c]);
        // The three follow-up reads go, each with nothing said.
        for gap in [1000, 2000, 4000] {
            rig.later(gap);
            assert!(rig.follow(), "a follow-up read is due");
            assert!(rig.desk.pending.contains_key(&Setting::Output));
            assert_eq!(rig.toast, None);
        }
        // Nothing more is asked, and at 8 s the desk gives up once.
        assert_eq!(rig.desk.settle_due(), Some(rig.t + Duration::from_secs(1)));
        rig.later(1000);
        assert!(rig.desk.follow_up(rig.t).is_none());
        let t = rig
            .desk
            .timed_out(&mut rig.m, rig.t)
            .expect("one toast at the end");
        assert_eq!(t.text, "Output: the G6 hasn't confirmed the change yet.");
        assert_eq!(t.setting, Some(Setting::Output));
        assert!(!t.text.contains("was not set"));
        assert!(rig.desk.pending.is_empty());
        // What the device last reported stays; nothing is left pending.
        assert_eq!(
            Setting::Output.current(&rig.desk.shown(&rig.m)),
            Some(Val::Int(1))
        );
        assert!(rig.desk.timed_out(&mut rig.m, rig.t).is_none());
    }

    #[test]
    fn startup_with_a_cache_shows_its_values_inert_until_the_read() {
        let mut cached = Sample::Live.model();
        cached.conn = Conn::Cached;
        assert_eq!(Setting::Output.current(&cached), Some(Val::Int(0)));
        // Every device control is drawn but nothing can be written yet.
        for page in Page::ALL {
            let out = frame_of(&cached, page);
            assert!(
                out.hits.iter().all(|(h, _)| setting_of(h).is_none()),
                "{page:?}"
            );
            if page == Page::Playback {
                assert!(out.hits.iter().any(|(h, _)| h == Hit::Inert(Why::Reading)));
            }
        }
        // The first read replaces the cache and the controls come alive on the read's values.
        cached.apply(Ok(Sample::Busy.model().status), None);
        assert_eq!(cached.conn, Conn::Connected);
        let out = frame_of(&cached, Page::Playback);
        assert!(
            out.hits
                .rect_of(Hit::Set(Setting::Output, Val::Int(1)))
                .is_some()
        );
    }

    #[test]
    fn the_colour_is_kept_as_written_because_the_g6_cannot_say() {
        let mut rig = Rig::new();
        // With no colour known, the light can't be turned on: there is no colour to send.
        rig.m.status.rgb_enabled = Some(false);
        assert!(
            rig.act(Setting::Lighting, Val::On(true), Phase::Click)
                .is_none()
        );
        assert!(rig.toast.as_deref().unwrap().contains("pick a colour"));
        rig.click(Setting::Colour, rgb_val([255, 255, 255]));
        assert_eq!(rig.m.colour, Some([255, 255, 255]));
        assert_eq!(rig.m.status.rgb_enabled, Some(true));
        assert!(
            rig.toast.as_deref().unwrap().contains("not read back"),
            "{:?}",
            rig.toast
        );
        assert_eq!(rig.dev.sent.len(), 9);
        // Off, then on again: the colour written last is sent again.
        rig.click(Setting::Lighting, Val::On(false));
        rig.click(Setting::Lighting, Val::On(true));
        assert_eq!(rig.dev.sent.len(), 9 + 1 + 9);
        assert_eq!(rig.dev.sent[10..], rig.dev.sent[..9]);
        assert_eq!(rig.m.status.rgb_enabled, Some(true));
    }

    // ---- through the drawn window: the regions a frame records are what a click acts on ----

    use crate::frame::{FrameIn, FrameOut, draw_frame, popup_area};
    use crate::fx::Fx;
    use crate::intent::rgb_of;
    use crate::intent::rgb_val;
    use crate::intent::test_device::hex_reports;
    use crate::kit::{Hit, Why};
    use crate::levels::Meters;
    use crate::model::Page;
    use crate::pick::{self, ColourPick, Press};
    use crate::prefs::Prefs;
    use mdshared_text::Bitmap;
    use mdshared_ui::colour_picker::Outcome as ColourOutcome;
    use mdshared_ui::gfx::{Gfx, Painter};
    use mdshared_ui::keys::{KeyName, Mods};

    fn draw_page(m: &Model, page: Page, fx: &mut Fx) -> FrameOut {
        let prefs = Prefs::default();
        let mut bmp = Bitmap::new(1100, 2400);
        let mut gfx = Gfx::new();
        let meters = Meters::none();
        let mut p = Painter::new(&mut bmp, &mut gfx, 1.0);
        let input = FrameIn {
            model: m,
            prefs: &prefs,
            pal: mdshared_ui::theme::LIGHT,
            page,
            size: (1100.0, 2400.0),
            scroll: 0.0,
            maximized: false,
            hover: None,
            pointer: None,
            now: std::time::Instant::now(),
            pressed: None,
            dragging: None,
            frosted: false,
            meters: &meters,
        };
        draw_frame(&mut p, &input, None, fx)
    }

    fn frame_of(m: &Model, page: Page) -> FrameOut {
        draw_page(m, page, &mut Fx::still())
    }

    /// The setting a region writes, if it writes one.
    fn setting_of(h: Hit) -> Option<Setting> {
        match h {
            Hit::Set(s, _) | Hit::Slide(s) | Hit::Open(s) => Some(s),
            _ => None,
        }
    }

    #[test]
    fn every_setting_has_a_live_control_and_the_rest_say_why_not() {
        let m = Sample::Busy.model();
        let mut live = std::collections::HashSet::new();
        let mut why = std::collections::HashSet::new();
        for page in Page::ALL {
            for (h, _) in frame_of(&m, page).hits.iter() {
                live.extend(setting_of(h));
                if let Hit::Inert(w) = h {
                    why.insert(w);
                }
            }
        }
        for s in Setting::all() {
            assert!(live.contains(&s), "{s:?} has no control");
        }
        // Windows' volumes and mutes, the speaker setup, the effects and Smart Volume's Normal stay read only.
        for w in [Why::Windows, Why::SpeakerSetup, Why::Effects, Why::NoWrite] {
            assert!(why.contains(&w), "{w:?}");
        }
    }

    #[test]
    fn the_mixer_strips_stay_inert() {
        let m = Sample::Busy.model();
        let out = frame_of(&m, Page::Mixer);
        let written: std::collections::HashSet<Setting> =
            out.hits.iter().filter_map(|(h, _)| setting_of(h)).collect();
        // Only the HID controls of its lower row write; the strips are Windows' (Core Audio).
        assert_eq!(
            written,
            [Setting::Output, Setting::MicBoost, Setting::DirectMode]
                .into_iter()
                .collect()
        );
        assert!(
            out.hits
                .iter()
                .filter(|(h, _)| *h == Hit::Inert(Why::Windows))
                .count()
                >= 20
        );
    }

    #[test]
    fn away_or_not_read_nothing_writes() {
        let away = Sample::Away.model();
        let mut starting = Model::starting();
        starting.status = Sample::Live.model().status;
        for (m, why) in [(away, Why::Away), (starting, Why::Reading)] {
            for page in Page::ALL {
                let out = frame_of(&m, page);
                assert!(
                    out.hits.iter().all(|(h, _)| setting_of(h).is_none()),
                    "{page:?}"
                );
                if page == Page::Sbx {
                    assert!(out.hits.iter().any(|(h, _)| h == Hit::Inert(why)));
                }
            }
        }
    }

    #[test]
    fn a_click_on_the_drawn_switch_sends_the_cli_reports_and_the_switch_follows() {
        let mut rig = Rig::new();
        let out = frame_of(&rig.m, Page::Playback);
        // The busy sample has Direct mode off: its switch asks for on.
        let hit = Hit::Set(Setting::DirectMode, Val::On(true));
        assert!(out.hits.rect_of(hit).is_some());
        let Hit::Set(s, v) = hit else { unreachable!() };
        rig.click(s, v);
        assert_eq!(
            rig.dev.sent.iter().map(hex).collect::<Vec<_>>(),
            ["5a3903000501", "5a390101"]
        );
        // Drawn again from the read-back, the switch now asks for off.
        let out = frame_of(&rig.m, Page::Playback);
        assert!(out.hits.rect_of(Hit::Set(s, Val::On(false))).is_some());
        // A segment of the output, in the sidebar: the other output.
        assert!(
            out.hits
                .rect_of(Hit::Set(Setting::Output, Val::Int(0)))
                .is_some()
        );
    }

    // ---- the colour picker: the toolkit's picker over the Lighting page, its colours through the desk ----

    /// The picker open over the Lighting page, driven as `app::App` drives it: the toolkit's press, drag, release
    /// and keys, the desk's three phases, and `pick::on_close` for what a close writes.
    struct Open {
        rig: Rig,
        pick: ColourPick,
    }

    impl Open {
        /// With a red light set first, so a cancel has the colour from before the open to go back to.
        fn new() -> Open {
            let mut rig = Rig::new();
            rig.click(Setting::Colour, rgb_val([255, 0, 0]));
            let out = frame_of(&rig.m, Page::Lighting);
            let anchor = out
                .hits
                .rect_of(Hit::Open(Setting::Colour))
                .expect("the Colour row opens the picker");
            let original = Setting::Colour.current(&rig.m);
            let mut pick = ColourPick::open(anchor, pick::rgb_or_fallback(original), original);
            let card = pick.place(popup_area((1100.0, 2400.0)));
            assert!(card.w > 200.0 && card.h > 200.0, "{card:?}");
            Open { rig, pick }
        }

        /// The point of the picker's square at saturation `s` and brightness `v`, each 0 to 1.
        fn square(&self, s: f32, v: f32) -> (f32, f32) {
            let r = self.pick.layout().square;
            (r.x + s * r.w, r.y + (1.0 - v) * r.h)
        }

        /// A value on its way: the worker's turn, and its read-back taken.
        fn write(&mut self, v: Val, phase: Phase) {
            if let Some(c) = self.rig.act(Setting::Colour, v, phase) {
                self.pick.sent = Some(v);
                self.rig.run(vec![c]);
            }
        }

        /// The window's `about_to_wait`: a value that waited out the throttle goes.
        fn tick(&mut self) {
            let Some(c) = self.rig.desk.tick(&self.rig.m, self.rig.t) else {
                return;
            };
            if let Cmd::Write(w) = &c {
                self.pick.sent = Some(w.val);
            }
            self.rig.run(vec![c]);
        }

        fn press(&mut self, x: f32, y: f32) {
            match self.pick.press(x, y, false, 1) {
                Press::Away => self.close(false),
                Press::Colour(v) => self.write(v, Phase::Drag),
                Press::Taken => {}
            }
        }

        fn drag(&mut self, x: f32, y: f32) {
            if let (_, Some(v)) = self.pick.drag(x, y) {
                self.write(v, Phase::Drag);
            }
        }
        fn release(&mut self) {
            if let Some(v) = self.pick.release() {
                self.write(v, Phase::Release);
            }
        }

        fn key(&mut self, key: KeyName) {
            let (outcome, colour) = self.pick.key(&key, Mods::default(), None);
            if let Some(v) = colour {
                self.write(v, Phase::Click);
            }
            match outcome {
                ColourOutcome::Accept => self.close(true),
                ColourOutcome::Cancel => self.close(false),
                _ => {}
            }
        }

        /// The window's close: what `pick::on_close` says goes, then a write-back on a cancel.
        fn close(&mut self, accept: bool) {
            let write = pick::on_close(
                self.pick.sent,
                self.pick.colour(),
                self.pick.original,
                accept,
            );
            self.rig.desk.abandon(Setting::Colour);
            if let Some(v) = write {
                self.write(v, if accept { Phase::Release } else { Phase::Click });
            }
        }
    }

    #[test]
    fn dragging_in_the_picker_writes_the_colour_throttled_and_the_release_ends_it() {
        let mut o = Open::new();
        let before = o.rig.dev.writes.len();
        let (x, y) = o.square(0.1, 0.9);
        o.press(x, y);
        assert_eq!(o.rig.dev.writes.len(), before + 1, "a press writes at once");
        // A press is the start of a drag: the model takes the colour on release, and the ring follows the pending one
        // in between, as for a slider.
        assert_eq!(
            Setting::Colour.current(&o.rig.desk.shown(&o.rig.m)),
            Some(o.pick.colour())
        );
        // 100 moves 4 ms apart: 400 ms of dragging, as the slider test does.
        for k in 0..100 {
            let t = k as f32 / 100.0;
            let (x, y) = o.square(0.1 + 0.8 * t, 0.9 - 0.7 * t);
            o.drag(x, y);
            o.rig.later(4);
            o.tick();
        }
        let during = o.rig.dev.writes.len() - before;
        assert!((2..=7).contains(&during), "{during} writes in 400 ms");
        let last = o.pick.colour();
        o.release();
        assert_eq!(o.rig.dev.writes.last(), Some(&(Setting::Colour, last)));
        // The read-back is the colour written: the row and the ring follow it, nothing is left pending.
        assert_eq!(o.rig.m.colour, rgb_of(last));
        assert!(o.rig.desk.pending.is_empty());
        assert!(o.rig.desk.dragging().is_none());
        // Undo is one step: the colour from before the whole press, drag and release.
        assert_eq!(
            o.rig.desk.undo,
            Some(Undo {
                setting: Setting::Colour,
                before: rgb_val([255, 0, 0]),
                after: last,
            })
        );
        // Every report sent is the CLI's for `--lighting-rgb R G B`.
        let mut want = Vec::new();
        for (_, v) in &o.rig.dev.writes {
            want.extend(hex_reports(Setting::Colour, *v, &o.rig.m));
        }
        assert_eq!(o.rig.dev.sent.iter().map(hex).collect::<Vec<_>>(), want);
    }

    #[test]
    fn escape_after_a_drag_writes_the_colour_from_before_the_open_back() {
        let mut o = Open::new();
        let red = [255, 0, 0];
        let (x, y) = o.square(0.9, 0.2);
        o.press(x, y);
        o.release();
        let picked = o.rig.m.colour.expect("the drag was read back");
        assert_ne!(picked, red);
        assert_eq!(o.pick.sent, Some(rgb_val(picked)));
        o.key(KeyName::Escape);
        assert_eq!(
            o.rig.dev.writes.last(),
            Some(&(Setting::Colour, rgb_val(red)))
        );
        assert_eq!(o.rig.m.colour, Some(red));
        assert!(o.rig.desk.pending.is_empty());
    }

    #[test]
    fn a_press_away_from_the_picker_closes_it_without_a_write() {
        let mut o = Open::new();
        let (writes, sent) = (o.rig.dev.writes.len(), o.rig.dev.sent.len());
        assert!(o.pick.sent.is_none());
        // The title row: away from the card and from the row that opened it.
        o.press(20.0, 10.0);
        assert_eq!(o.rig.dev.writes.len(), writes);
        assert_eq!(o.rig.dev.sent.len(), sent);
        assert_eq!(o.rig.m.colour, Some([255, 0, 0]));
        assert!(o.rig.desk.pending.is_empty());
    }

    #[test]
    fn the_open_picker_covers_what_is_under_it_and_stays_inside_the_window() {
        let m = Sample::Busy.model();
        let mut fx = Fx::still();
        let closed = draw_page(&m, Page::Lighting, &mut fx);
        let anchor = closed
            .hits
            .rect_of(Hit::Open(Setting::Colour))
            .expect("the Colour row opens the picker");
        let mut pick = ColourPick::open(anchor, [0, 200, 255], None);
        pick.place(popup_area((1100.0, 2400.0)));
        fx.pick = Some(pick);
        let open = draw_page(&m, Page::Lighting, &mut fx);
        let card = open.hits.rect_of(Hit::Blocked).expect("the card is there");
        assert!(card.x >= 0.0 && card.right() <= 1100.0, "{card:?}");
        assert!(card.y >= 36.0 && card.bottom() <= 2400.0, "{card:?}");
        for x in (card.x as i32..card.right() as i32).step_by(11) {
            for y in (card.y as i32..card.bottom() as i32).step_by(11) {
                assert_eq!(
                    open.hits.at(x as f32, y as f32),
                    Some(Hit::Blocked),
                    "{x}, {y}"
                );
            }
        }
        assert_ne!(closed.hits.at(card.cx(), card.cy()), Some(Hit::Blocked));
    }

    #[test]
    fn a_quick_pick_colour_is_the_cli_write_for_lighting_rgb() {
        let m = Sample::Busy.model();
        let out = frame_of(&m, Page::Lighting);
        let green = rgb_val([0, 255, 0]);
        let swatch = out
            .hits
            .rect_of(Hit::Set(Setting::Colour, green))
            .expect("a quick-pick swatch");
        assert_eq!(
            out.hits.at(swatch.cx(), swatch.cy()),
            Some(Hit::Set(Setting::Colour, green))
        );
        let mut rig = Rig::new();
        rig.click(Setting::Colour, green);
        assert_eq!(
            rig.dev.sent.iter().map(hex).collect::<Vec<_>>(),
            hex_reports(Setting::Colour, green, &rig.m)
        );
        // A colour no swatch has is the CLI's own reports too (`--lighting-rgb 12 34 56`).
        rig.click(Setting::Colour, rgb_val([12, 34, 56]));
        assert_eq!(
            rig.dev.sent[9..].iter().map(hex).collect::<Vec<_>>(),
            cli_reports(&["--lighting-rgb", "12", "34", "56"])
        );
        assert_eq!(rig.m.colour, Some([12, 34, 56]));
        assert_eq!(Setting::Colour.text(rgb_val([12, 34, 56])), "#0c2238");
    }
}
