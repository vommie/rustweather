//! Is someone there? No fetching behind a locked screen, and fullscreen programs are not interrupted.

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

/// True while the screen saver or lock screen runs.
pub fn away() -> bool {
    let Ok(bus) = zbus::blocking::Connection::session() else {
        return false;
    };
    for (service, path) in [
        ("org.freedesktop.ScreenSaver", "/ScreenSaver"),
        ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver"),
    ] {
        let reply = bus.call_method(
            Some(service),
            path,
            Some("org.freedesktop.ScreenSaver"),
            "GetActive",
            &(),
        );
        if let Ok(message) = reply
            && let Ok(active) = message.body().deserialize::<bool>()
        {
            return active;
        }
    }
    false
}

/// True when the active X11 window is fullscreen. Wayland has no way to ask; there Plasma's own setting applies.
pub fn fullscreen() -> bool {
    check().unwrap_or(false)
}

fn check() -> Option<bool> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atom = |name: &[u8]| Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom);
    let (active, state, full) = (
        atom(b"_NET_ACTIVE_WINDOW")?,
        atom(b"_NET_WM_STATE")?,
        atom(b"_NET_WM_STATE_FULLSCREEN")?,
    );
    let window = conn
        .get_property(false, root, active, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let id = window.value32()?.next()?;
    let states = conn
        .get_property(false, id, state, AtomEnum::ATOM, 0, 32)
        .ok()?
        .reply()
        .ok()?;
    Some(states.value32()?.any(|a| a == full))
}
