//! The window's motion: the toolkit's animator and the things that outlive a frame because they are still moving.
//!
//! Drawing is immediate and the window sleeps between events (`doc/gui-plan.md`, "Motion"). The animator remembers
//! the animated values by name from frame to frame and says, at the end of each, whether and when another is needed.
//! This type holds it together with what has to be kept for an exit: the toast, the dialog and the tooltip's anchor,
//! and the page switch, which is driven by a picture of the old page.

use mdshared_ui::anim::{Animator, Edge, PageEffect, PageTransition, Policy, Transient};
use mdshared_ui::gfx::{Painter, Rect};
use mdshared_ui::settings::pages::Form;

/// The new page rises into place from this far below, in logical px (WinUI's entrance moves a page about this far).
pub const PAGE_RISE: f32 = 20.0;

pub struct Fx {
    pub anim: Animator,
    /// The switch of the page area from one page to another.
    pub page: PageTransition,
    /// The toast. Its text stays until it has faded out.
    pub toast: Transient<String>,
    /// The settings dialog, after it was closed and while it fades out.
    pub leaving: Option<Form>,
    /// Where the tooltip was last shown, kept for its fade-out.
    pub tip: Option<Rect>,
}

impl Fx {
    /// The motion of the running window: it animates unless the system asks for less.
    pub fn new() -> Fx {
        Fx {
            anim: Animator::new(),
            // 250 ms, decelerating: `fluent::PAGE`.
            page: PageTransition::new("page"),
            toast: Transient::new(),
            leaving: None,
            tip: None,
        }
    }

    /// Nothing moves: every value is its target at once. For `--shot` and the tests that draw a frame, which must
    /// not depend on time.
    pub fn still() -> Fx {
        let mut fx = Fx::new();
        fx.anim.set_policy(Policy::Never);
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
}

impl Default for Fx {
    fn default() -> Fx {
        Fx::new()
    }
}
