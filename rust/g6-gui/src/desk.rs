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

#[derive(Debug, Default)]
pub struct Desk {
    pub pending: BTreeMap<Setting, Pending>,
    held: Option<Held>,
    pub undo: Option<Undo>,
}

/// The toast after a write: what happened, worded for a person.
pub type Toast = String;

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
            return Err("The G6 is not connected, so nothing was changed.".into());
        }
        // The action is built now, so a value that can't be written is said at once and never shown as pending.
        s.action(v, &self.shown(m))?;
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
        let action = s.action(v, &self.shown(m))?;
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
            return Err("Nothing to undo.".into());
        };
        if u.setting.current(m) != Some(u.after) {
            return Err(format!(
                "{} has changed since, so there is nothing to undo.",
                u.setting.label()
            ));
        }
        self.intent_as(m, u.setting, u.before, Phase::Click, now, true)
    }

    /// A turn of the worker came back: the model takes the read, and each write is settled against it. Returns the
    /// toast to show, if any (an error wins over a confirmation).
    pub fn reconcile(&mut self, m: &mut Model, r: ReadResult) -> Option<Toast> {
        let why = r.status.as_ref().err().cloned();
        let wrote = !r.writes.is_empty();
        m.apply(r.status, r.mixer);
        if let Some(why) = why {
            let had = !self.pending.is_empty();
            self.pending.clear();
            self.held = None;
            return if wrote || had {
                Some(format!(
                    "The G6 is not connected, so the change was not made. {why}"
                ))
            } else {
                // A read the user asked for: its answer is the reason.
                r.user.then_some(why)
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
            let label = w.setting.label();
            let held = self.dragging() == Some(w.setting);
            if let Err(e) = w.result {
                self.pending.remove(&w.setting);
                bad = Some(format!("{label} was not changed: {e}"));
                continue;
            }
            if held {
                // The drag goes on; its release settles it.
                continue;
            }
            self.pending.remove(&w.setting);
            match w.setting.confirmed(w.val, m) {
                Some(true) => {
                    good = Some(confirmation(w.setting, w.val, p.undo, false));
                    self.remember(w.setting, p, w.val);
                }
                None => {
                    if w.setting.unreadable() {
                        w.setting.put(w.val, m);
                        self.remember(w.setting, p, w.val);
                    }
                    good = Some(confirmation(w.setting, w.val, p.undo, true));
                }
                Some(false) => {
                    let now = w
                        .setting
                        .current(m)
                        .map_or_else(|| "something else".into(), |c| w.setting.text(c));
                    bad = Some(format!(
                        "{label} was not set to {}: the G6 has {now}.",
                        w.setting.text(w.val)
                    ));
                }
            }
        }
        if r.user && bad.is_none() && good.is_none() {
            good = Some("Status read from the G6.".into());
        }
        bad.or(good)
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
                Ok(self.status.clone())
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
            self.toast = self.desk.reconcile(&mut self.m, r);
        }

        fn act(&mut self, s: Setting, v: Val, phase: Phase) -> Option<Cmd> {
            match self.desk.intent(&self.m, s, v, phase, self.t) {
                Ok(c) => c,
                Err(t) => {
                    self.toast = Some(t);
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
    }

    fn hex(r: &HidReport) -> String {
        g6_proto::packet::hex(r).trim_end_matches('0').to_string()
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
        let cases = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/vectors/writes.json"),
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&cases).unwrap();
        let want_of = |argv: &[&str]| -> Vec<String> {
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
                .unwrap();
            c["hid"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h.as_str().unwrap().trim_end_matches('0').to_string())
                .collect()
        };
        let mut want = want_of(&["--playback-direct-mode", "Enabled"]);
        want.extend(want_of(&["--sbx-surround-value", "42"]));
        want.extend(want_of(&["--recording-mic-boost-db", "30"]));
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
    fn the_colour_is_kept_as_written_because_the_g6_cannot_say() {
        let mut rig = Rig::new();
        // With no colour known, the light can't be turned on: there is no colour to send.
        rig.m.status.rgb_enabled = Some(false);
        assert!(
            rig.act(Setting::Lighting, Val::On(true), Phase::Click)
                .is_none()
        );
        assert!(rig.toast.as_deref().unwrap().contains("pick a colour"));
        rig.click(Setting::Colour, Val::Int(7));
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

    use crate::frame::{FrameIn, FrameOut, draw_frame};
    use crate::fx::Fx;
    use crate::kit::{Hit, Why};
    use crate::model::Page;
    use crate::prefs::Prefs;
    use mdshared_text::Bitmap;
    use mdshared_ui::gfx::{Gfx, Painter};

    fn frame_of(m: &Model, page: Page) -> FrameOut {
        let prefs = Prefs::default();
        let mut bmp = Bitmap::new(1100, 2400);
        let mut gfx = Gfx::new();
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
            pressed: None,
            dragging: None,
            frosted: false,
        };
        draw_frame(&mut p, &input, None, &mut Fx::still())
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
        for (m, why) in [(away, Why::Away), (starting, Why::NotRead)] {
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
}
