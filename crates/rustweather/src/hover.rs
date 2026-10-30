//! The weather card while the pointer rests on the tray icon. The tray protocol reports clicks but no hovering, so on X11 the app
//! watches the pointer around the place of the last click. Wayland does not let a program see the pointer there.

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle as _;
use x11rb::connection::Connection as _;
use x11rb::rust_connection::RustConnection;

use crate::app::App;
use crate::presence;

const TICK: Duration = Duration::from_millis(150);
const OPEN_AFTER: Duration = Duration::from_millis(500);
const CLOSE_AFTER: Duration = Duration::from_millis(350);
/// Half the size of a tray icon, plus a little.
const ICON_REACH: i32 = 14;

/// The steps of the card, apart from the window: testable without a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Away,
    /// The pointer is on the icon, the card waits for its moment.
    Waiting,
    Open,
    /// The pointer left; the card stays a moment, so it can be reached.
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    None,
    Show,
    Hide,
}

#[derive(Default)]
pub struct Hover {
    phase: Phase,
    since: Option<Instant>,
    x11: Option<(RustConnection, u32)>,
}

impl Hover {
    fn step(&mut self, inside: bool, card_visible: bool, now: Instant) -> Action {
        // The user closed the card by a click: it may only come back after the pointer left.
        if matches!(self.phase, Phase::Open | Phase::Closing) && !card_visible {
            if inside {
                return Action::None;
            }
            self.phase = Phase::Away;
        }
        let elapsed = self.since.map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
        match (self.phase, inside) {
            (Phase::Away, true) => {
                self.phase = Phase::Waiting;
                self.since = Some(now);
            }
            (Phase::Waiting, false) => self.phase = Phase::Away,
            (Phase::Waiting, true) if elapsed >= OPEN_AFTER => {
                self.phase = Phase::Open;
                return Action::Show;
            }
            (Phase::Open, false) => {
                self.phase = Phase::Closing;
                self.since = Some(now);
            }
            (Phase::Closing, true) => self.phase = Phase::Open,
            (Phase::Closing, false) if elapsed >= CLOSE_AFTER => {
                self.phase = Phase::Away;
                return Action::Hide;
            }
            _ => {}
        }
        Action::None
    }
}

/// True where the app can see the pointer on the panel.
pub fn usable() -> bool {
    std::env::var("XDG_SESSION_TYPE").map_or(true, |s| s != "wayland") && std::env::var_os("DISPLAY").is_some()
}

fn inside(pointer: (i32, i32), icon: (i32, i32), card: Option<(i32, i32, i32, i32)>) -> bool {
    let on_icon = (pointer.0 - icon.0).abs() <= ICON_REACH && (pointer.1 - icon.1).abs() <= ICON_REACH;
    let on_card =
        card.is_some_and(|(x, y, w, h)| pointer.0 >= x && pointer.1 >= y && pointer.0 < x + w && pointer.1 < y + h);
    on_icon || on_card
}

fn tick(app: &App) {
    let enabled = app.settings.borrow().tray.hover_card;
    let Some(icon) = app.ui.borrow().tray_position.filter(|_| enabled) else {
        return;
    };
    let mut hover = app.hover.borrow_mut();
    if hover.x11.is_none() {
        hover.x11 = x11rb::connect(None).ok().and_then(|(conn, screen)| {
            let root = conn.setup().roots.get(screen)?.root;
            Some((conn, root))
        });
    }
    let Some(pointer) = hover
        .x11
        .as_ref()
        .and_then(|(conn, root)| presence::pointer(conn, *root))
    else {
        return;
    };
    let window = app.flyout.window();
    let visible = window.is_visible();
    let card = visible.then(|| {
        let (p, s) = (window.position(), window.size());
        (
            p.x,
            p.y,
            i32::try_from(s.width).unwrap_or(0),
            i32::try_from(s.height).unwrap_or(0),
        )
    });
    let action = hover.step(inside(pointer, icon, card), visible, Instant::now());
    drop(hover);
    match action {
        Action::Show if !visible => app.toggle_flyout(icon.0, icon.1),
        Action::Hide if visible => {
            let _ = app.flyout.hide();
            app.update_radar_activity();
        }
        _ => {}
    }
}

pub fn start(app: &Rc<App>) {
    if !usable() {
        return;
    }
    let weak = Rc::downgrade(app);
    app.hover_timer.start(slint::TimerMode::Repeated, TICK, move || {
        if let Some(app) = weak.upgrade() {
            tick(&app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_opens_after_a_moment_and_closes_after_leaving() {
        let mut h = Hover::default();
        let t0 = Instant::now();
        assert_eq!(h.step(true, false, t0), Action::None);
        assert_eq!(h.step(true, false, t0 + OPEN_AFTER / 2), Action::None);
        assert_eq!(h.step(true, false, t0 + OPEN_AFTER), Action::Show);
        let t1 = t0 + Duration::from_secs(2);
        assert_eq!(h.step(false, true, t1), Action::None);
        assert_eq!(h.step(true, true, t1 + CLOSE_AFTER / 2), Action::None);
        assert_eq!(h.step(false, true, t1 + CLOSE_AFTER), Action::None);
        assert_eq!(h.step(false, true, t1 + CLOSE_AFTER * 2 + CLOSE_AFTER), Action::Hide);
    }

    #[test]
    fn passing_over_the_icon_opens_nothing() {
        let mut h = Hover::default();
        let t0 = Instant::now();
        h.step(true, false, t0);
        assert_eq!(h.step(false, false, t0 + Duration::from_millis(100)), Action::None);
        assert_eq!(h.phase, Phase::Away);
    }

    #[test]
    fn a_card_closed_by_a_click_waits_until_the_pointer_left() {
        let mut h = Hover {
            phase: Phase::Open,
            ..Hover::default()
        };
        let t0 = Instant::now();
        assert_eq!(h.step(true, false, t0 + Duration::from_secs(3)), Action::None);
        assert_eq!(h.step(false, false, t0 + Duration::from_secs(4)), Action::None);
        assert_eq!(h.phase, Phase::Away);
    }

    #[test]
    fn the_card_counts_as_inside() {
        assert!(inside((100, 100), (105, 95), None));
        assert!(!inside((100, 100), (200, 200), None));
        assert!(inside((100, 100), (200, 200), Some((90, 90, 50, 50))));
    }
}
