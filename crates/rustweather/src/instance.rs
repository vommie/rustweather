//! One running app: a second start hands over to the first, which opens its window.

use std::io::{Read as _, Write as _};

use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName as _};

const NAME: &str = "rustweather.instance";

/// True when another instance took the message.
pub fn signal_running() -> bool {
    let Ok(name) = NAME.to_ns_name::<GenericNamespaced>() else {
        return false;
    };
    match Stream::connect(name) {
        Ok(mut stream) => stream.write_all(b"open").is_ok(),
        Err(_) => false,
    }
}

/// Listens for later starts and calls `on_open` for each.
pub fn listen(on_open: impl Fn() + Send + 'static) {
    let Ok(name) = NAME.to_ns_name::<GenericNamespaced>() else {
        return;
    };
    let Ok(listener) = ListenerOptions::new().name(name).create_sync() else {
        tracing::warn!("[App] Could not listen for a second start");
        return;
    };
    let spawned = std::thread::Builder::new().name("instance".into()).spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = [0u8; 8];
            let _ = stream.read(&mut buffer);
            on_open();
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "[App] No instance thread");
    }
}
