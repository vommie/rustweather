//! Windows come back the way they were: size, position and whether they were maximized. Wayland lets no program place a window, so
//! there the position is left to the compositor.

use std::rc::Rc;
use std::time::Duration;

use rustweather_core::ui_state::WindowState;
use slint::winit_030::WinitWindowAccessor as _;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, winit};
use slint::{ComponentHandle, PhysicalPosition, PhysicalSize};

use crate::app::App;

/// Long enough for a window manager to finish maximizing, so a half-done state is never kept.
const SETTLE: Duration = Duration::from_millis(400);
/// The window manager places a new window itself, so the position is set once more when the window is mapped.
const MAPPED: Duration = Duration::from_millis(60);

/// True when the place lies on a screen that still exists.
fn on_a_screen(window: &slint::Window, state: &WindowState) -> bool {
    let inside = |m: winit::monitor::MonitorHandle| {
        let (p, s) = (m.position(), m.size());
        let (right, bottom) = (
            p.x + i32::try_from(s.width).unwrap_or(0),
            p.y + i32::try_from(s.height).unwrap_or(0),
        );
        state.x + 40 > p.x && state.y + 10 > p.y && state.x < right - 40 && state.y < bottom - 40
    };
    window
        .with_winit_window(|w| w.available_monitors().any(inside))
        .unwrap_or(true)
}

/// Shows a window at its remembered place. An open window only comes to the front.
pub fn show_restored<T: ComponentHandle + 'static>(
    component: &T,
    saved: Option<WindowState>,
) -> Result<(), slint::PlatformError> {
    let window = component.window();
    let hidden = !window.is_visible();
    let saved = saved.filter(|_| hidden);
    if let Some(state) = saved.filter(|s| s.width > 0 && s.height > 0) {
        window.set_size(PhysicalSize::new(state.width, state.height));
    }
    component.show()?;
    window.with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
    if let Some(state) = saved {
        let weak = component.as_weak();
        let place = on_a_screen(window, &state) && state.width > 0;
        slint::Timer::single_shot(MAPPED, move || {
            if let Some(component) = weak.upgrade() {
                if place {
                    component.window().set_position(PhysicalPosition::new(state.x, state.y));
                }
                if state.maximized {
                    component.window().set_maximized(true);
                }
            }
        });
    }
    Ok(())
}

/// The state of a visible window; a fullscreen window keeps the state it came from.
fn capture(window: &slint::Window, old: Option<WindowState>) -> Option<WindowState> {
    if !window.is_visible() || window.is_fullscreen() {
        return old;
    }
    if window.is_maximized() {
        return Some(WindowState {
            maximized: true,
            ..old.unwrap_or_default()
        });
    }
    let (position, size) = (window.position(), window.size());
    Some(WindowState {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        maximized: false,
    })
}

fn remember(app: &App) {
    let changed = {
        let mut ui = app.ui.borrow_mut();
        let before = (ui.main_window, ui.mini_window);
        ui.main_window = capture(app.main.window(), ui.main_window);
        ui.mini_window = capture(app.mini.window(), ui.mini_window);
        before != (ui.main_window, ui.mini_window)
    };
    if changed {
        app.save_ui();
    }
}

fn watch(window: &slint::Window, app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    window.on_winit_window_event(move |_, event| {
        if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_))
            && let Some(app) = weak.upgrade()
        {
            let later = Rc::downgrade(&app);
            app.window_timer.start(slint::TimerMode::SingleShot, SETTLE, move || {
                if let Some(app) = later.upgrade() {
                    remember(&app);
                }
            });
        }
        EventResult::Propagate
    });
}

pub fn wire(app: &Rc<App>) {
    watch(app.main.window(), app);
    watch(app.mini.window(), app);
}
