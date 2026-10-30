//! Start with the desktop: a file in `~/.config/autostart`.

use std::path::PathBuf;

fn file() -> Option<PathBuf> {
    let dirs = rustweather_core::paths::Paths::standard().ok()?;
    Some(dirs.config.parent()?.join("autostart").join("rustweather.desktop"))
}

pub fn enabled() -> bool {
    file().is_some_and(|f| f.exists())
}

pub fn set(on: bool) -> std::io::Result<()> {
    let Some(path) = file() else {
        return Ok(());
    };
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let exe = std::env::current_exe()?;
    std::fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=RustWeather\nComment=Temperature in the tray and a weather radar\nExec={}\nIcon=rustweather\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        ),
    )
}
