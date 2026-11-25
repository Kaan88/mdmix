//! The window's motion: the toolkit's animator and the things that outlive a frame because they are still moving.
//!
//! Drawing is immediate and the window sleeps between events (`doc/gui-plan.md`, "Motion"). The animator remembers
//! the animated values by name from frame to frame and says, at the end of each, whether and when another is needed.
//! This type holds it together with what has to be kept for an exit: the stack of toasts, the dialog and the
//! tooltip's last anchor, and the page switch, which is driven by a picture of the old page.
//!
//! The timers outside the animator (a toast's own time, the tooltip's delay) are folded into the same deadline by
//! [`Fx::wake`], so one wake-up serves them all.

use std::time::{Duration, Instant};

use mdshared_ui::anim::{Animator, Edge, PageEffect, PageTransition, Policy, Stack};
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::menu::Menu;
use mdshared_ui::settings::pages::Form;
use mdshared_ui::tooltip::Tooltip;

use crate::desk::Toast;
use crate::kit::Hit;
use crate::pick::ColourPick;

/// The new page rises into place from this far below, in logical px (WinUI's entrance moves a page about this far).
pub const PAGE_RISE: f32 = 20.0;

/// How long a toast stands before it goes (the mockup's figure: long enough to read one line, short enough not to
/// stand in the way).
pub const TOAST_FOR: Duration = Duration::from_millis(3500);

pub struct Fx {
    pub anim: Animator,
    /// The switch of the page area from one page to another.
    pub page: PageTransition,
    /// The toasts: several may stand at once, each with its own time to leave.
    pub toasts: Stack<Toast>,
    /// The settings dialog, after it was closed and while it fades out.
    pub leaving: Option<Form>,
    /// The tooltip's timing: the delay from the pointer coming to rest, the control just pressed, the next wake.
    pub tip: Tooltip<Rect>,
    /// The tip last drawn, its words and where: kept for its fade-out once it is gone from [`tip`](Self::tip).
    pub tip_last: Option<(Rect, String)>,
    /// A dropdown's open list of choices; each item is the click it stands for.
    pub menu: Option<Menu<Hit>>,
    /// The Lighting page's colour picker, while it is open.
    pub pick: Option<ColourPick>,
}

impl Fx {
    /// The motion of the running window: it animates unless the system asks for less.
    pub fn new() -> Fx {
        Fx {
            anim: Animator::new(),
            // 250 ms, decelerating: `fluent::PAGE`.
            page: PageTransition::new("page"),
            toasts: Stack::new().with_max_drawn(3),
            leaving: None,
            tip: Tooltip::new(),
            tip_last: None,
            menu: None,
            pick: None,
        }
    }

    /// Nothing moves: every value is its target at once. For `--shot` and the tests that draw a frame, which must
    /// not depend on time. The tip shows at once, too: a still frame is the picture at rest.
    pub fn still() -> Fx {
        let mut fx = Fx::new();
        fx.anim.set_policy(Policy::Never);
        fx.tip = Tooltip::new().with_delay(Duration::ZERO);
        fx
    }

    /// A page switch has been decided and the painter's bitmap still holds the old page: the new one will rise into
    /// place over `view` and the old one fade away.
    pub fn begin_page(&mut self, p: &Painter, view: Rect) {
        self.page.begin(
            &mut self.anim,
            p,
            view,
            PageEffect::SlideFade {
                from: Edge::Bottom,
                distance: PAGE_RISE,
            },
        );
    }

    /// A toast to show until `until`: one about the same setting takes the place of that setting's own (in place,
    /// with no second entrance), anything else joins the stack at the bottom.
    pub fn announce(&mut self, toast: Toast, until: Instant) {
        if let Some(setting) = toast.setting
            && let Some((key, _)) = self.toasts.find(|t| t.setting == Some(setting))
        {
            self.toasts.replace(key, toast, Some(until));
        } else {
            // `show` keeps an equal toast already there instead of standing a second one beside it.
            self.toasts.show(toast, Some(until));
        }
    }

    /// The end of a frame: the next moment the loop must wake, or `None` when nothing moves and no timer waits. The
    /// toast times and the tooltip's delay join the animator's own deadline, so one `WaitUntil` serves them all.
    pub fn wake(&mut self) -> Option<Instant> {
        for at in [self.toasts.deadline(), self.tip.deadline()]
            .into_iter()
            .flatten()
        {
            self.anim.wake_at(at);
        }
        self.anim.end_frame()
    }
}

impl Default for Fx {
    fn default() -> Fx {
        Fx::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::Setting;

    fn texts(fx: &Fx) -> Vec<(u64, String)> {
        fx.toasts
            .current()
            .map(|(k, t)| (k, t.text.clone()))
            .collect()
    }

    #[test]
    fn a_second_write_on_the_same_setting_replaces_its_toast_and_another_setting_stacks() {
        let mut fx = Fx::new();
        let t = Instant::now();
        let until = t + Duration::from_secs(3);
        fx.announce(Toast::about(Setting::Sbx, "SBX turned on."), until);
        fx.announce(Toast::about(Setting::Eq, "Equalizer turned on."), until);
        let both = texts(&fx);
        assert_eq!(both.len(), 2, "two settings, two toasts: {both:?}");
        assert_eq!(both[0].1, "SBX turned on.");
        assert_eq!(both[1].1, "Equalizer turned on.");
        // The same setting again: the same toast, in its place, with the new words.
        fx.announce(Toast::about(Setting::Sbx, "SBX turned off."), until);
        let after = texts(&fx);
        assert_eq!(after.len(), 2, "{after:?}");
        assert_eq!(after[0], (both[0].0, "SBX turned off.".to_string()));
        assert_eq!(after[1], both[1]);
        // A toast about no setting is its own; saying it twice does not stand it twice.
        fx.announce(Toast::plain("Status read from the G6."), until);
        fx.announce(Toast::plain("Status read from the G6."), until);
        let all = texts(&fx);
        assert_eq!(all.len(), 3, "{all:?}");
        assert_eq!(all[2].1, "Status read from the G6.");
        // At most three stand: a fourth sends the oldest away.
        fx.announce(
            Toast::about(Setting::MicBoost, "Mic boost set to 30 dB."),
            until,
        );
        let shown = texts(&fx);
        assert_eq!(shown.len(), 3, "{shown:?}");
        assert!(!shown.iter().any(|(_, t)| t == "SBX turned off."));
    }
}
