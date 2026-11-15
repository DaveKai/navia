//! Parts of the window that keep last frame's drawing until the shell changes.
//!
//! GPUI redraws a whole view on every notify, but the lyrics animate for a whole song while the
//! sidebar and main column stay the same. Each of those renders in its own cached view (`Slot`),
//! so a lyrics-only frame replays them instead of rebuilding them.
//!
//! Any `cx.notify()` on the shell redraws every slot, so shell code behaves as if nothing were
//! cached. Only the lyrics animation requests frames through `Ticker`, which redraws the shell's
//! own elements and nothing cached. `Shell::frame` counts full frames only, so per-frame
//! bookkeeping treats cached parts as still drawn.

use gpui::prelude::*;
use gpui::{AnyElement, AnyView, Context, Entity, Pixels, StyleRefinement, Subscription, WeakEntity, Window, div, px};

use crate::shell::{Layout, Shell};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Sidebar,
    Main,
}

pub struct Slot {
    shell: WeakEntity<Shell>,
    part: Part,
}

impl Render for Slot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.shell.update(cx, |shell, cx| shell.render_slot(part, window, cx)).unwrap_or_else(|_| div().into_any_element())
    }
}

/// An empty view, notified to get a frame that redraws the shell's own elements but no slots.
pub struct Ticker;

impl Render for Ticker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub struct Slots {
    sidebar: Entity<Slot>,
    main: Entity<Slot>,
    ticker: Entity<Ticker>,
    /// The layout of the frame being drawn, for the slots that render after the shell.
    layout: Option<Layout>,
    /// The shell was notified since its last render, so this frame redraws everything.
    full: bool,
    _observer: Subscription,
}

impl Slots {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        let shell = cx.entity().downgrade();
        let sidebar = cx.new(|_| Slot { shell: shell.clone(), part: Part::Sidebar });
        let main = cx.new(|_| Slot { shell, part: Part::Main });
        let ticker = cx.new(|_| Ticker);
        let observer = cx.observe_self(|shell, cx| {
            shell.slots.full = true;
            shell.slots.sidebar.update(cx, |_, cx| cx.notify());
            shell.slots.main.update(cx, |_, cx| cx.notify());
        });
        Self { sidebar, main, ticker, layout: None, full: true, _observer: observer }
    }

    /// Starts a render of the shell. Returns true for a full frame, when the shell itself changed.
    pub fn begin(&mut self, layout: &Layout) -> bool {
        self.layout = Some(layout.clone());
        std::mem::replace(&mut self.full, false)
    }

    /// Asks for a frame that redraws the shell's own elements and leaves the slots as they are.
    pub fn tick(&self, window: &mut Window) {
        let ticker = self.ticker.entity_id();
        window.on_next_frame(move |_, cx| cx.notify(ticker));
    }

    pub fn ticker_view(&self) -> AnyElement {
        let mut style = StyleRefinement::default();
        style.position = Some(gpui::Position::Absolute);
        AnyView::from(self.ticker.clone()).cached(style.size_0()).into_any_element()
    }

    /// The sidebar, cached. It places itself, so the slot covers the shell's root.
    pub fn sidebar_view(&self) -> AnyElement {
        let style = StyleRefinement::default().absolute().inset_0();
        AnyView::from(self.sidebar.clone()).cached(style).into_any_element()
    }

    /// The main column, cached. It takes the column's place in the root row, with the sidebar's
    /// width as its left margin.
    pub fn main_view(&self, margin: Pixels) -> AnyElement {
        let style = StyleRefinement::default().relative().flex_1().min_w(px(0.0)).h_full().ml(margin);
        AnyView::from(self.main.clone()).cached(style).into_any_element()
    }
}

impl Shell {
    /// Reacts to a new playback position. When only the dock or expanded player show it, cached
    /// parts stay as they are. The bottom bar lives in the main column, so then everything redraws.
    /// Skipped while hidden.
    pub fn notify_position(&mut self, cx: &mut Context<Self>) {
        if !self.window_shown {
            return;
        }
        let in_shell = self.slots.layout.as_ref().is_some_and(|l| l.player && (self.expanded || l.mode == crate::persist::PlayerMode::Dock));
        if in_shell {
            self.slots.ticker.update(cx, |_, cx| cx.notify());
        } else {
            cx.notify();
        }
    }

    fn render_slot(&mut self, part: Part, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(layout) = self.slots.layout.clone() else { return div().into_any_element() };
        match part {
            Part::Sidebar => {
                let element = crate::focus::scoped(crate::focus::Scope::Sidebar, crate::focus::SECTION_SIDEBAR, || self.render_sidebar(&layout, cx));
                div().relative().size_full().child(crate::testing::timed(["sidebar.layout", "sidebar.prepaint", "sidebar.paint"], element)).into_any_element()
            }
            Part::Main => {
                let element = crate::focus::scoped(crate::focus::Scope::Main, crate::focus::SECTION_PAGE, || self.render_main_column(&layout, false, window, cx));
                div().relative().size_full().child(crate::testing::timed(["main.layout", "main.prepaint", "main.paint"], element)).into_any_element()
            }
        }
    }
}
