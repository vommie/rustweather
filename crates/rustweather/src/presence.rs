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

/// The pointer's position on the X11 screen. A connection is passed in, because this runs several times a second.
pub fn pointer(conn: &x11rb::rust_connection::RustConnection, root: u32) -> Option<(i32, i32)> {
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;
    Some((i32::from(reply.root_x), i32::from(reply.root_y)))
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    /// A stand-in for the desktop's screen saver service: it is active while a file exists.
    struct FakeScreenSaver(PathBuf);

    #[zbus::interface(name = "org.freedesktop.ScreenSaver")]
    impl FakeScreenSaver {
        fn get_active(&self) -> bool {
            self.0.exists()
        }
    }

    /// Run by hand on a private bus: `dbus-run-session`, with `RW_FAKE_LOCK` naming the file.
    #[test]
    #[ignore = "serves a fake screen saver for a test by hand"]
    fn serve_fake_screen_saver() {
        let file = PathBuf::from(std::env::var("RW_FAKE_LOCK").unwrap());
        let _bus = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.ScreenSaver")
            .unwrap()
            .serve_at("/ScreenSaver", FakeScreenSaver(file))
            .unwrap()
            .build()
            .unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }

    /// Run by hand while a fullscreen window has the focus.
    #[test]
    #[ignore = "needs a fullscreen window"]
    fn sees_a_fullscreen_window() {
        assert!(fullscreen());
    }

    /// Run by hand while a normal window has the focus.
    #[test]
    #[ignore = "needs a normal window"]
    fn sees_a_normal_window() {
        assert!(!fullscreen());
    }
}
