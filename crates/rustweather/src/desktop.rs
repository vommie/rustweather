//! Desktop integration without a package: the application menu entry, the icons and the autostart entry, all in the user's own
//! XDG folders. Status, install and uninstall live here; Settings > Desktop and `--desktop` call them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustweather_core::APP_ID;

const ENTRY: &str = include_str!("../../../packaging/io.github.vommie.rustweather.desktop");
const SCALABLE: &[u8] = include_bytes!("../../../packaging/rustweather.svg");
const ICONS: [(u32, &[u8]); 9] = [
    (16, include_bytes!("../../../packaging/icons/rustweather-16.png")),
    (22, include_bytes!("../../../packaging/icons/rustweather-22.png")),
    (24, include_bytes!("../../../packaging/icons/rustweather-24.png")),
    (32, include_bytes!("../../../packaging/icons/rustweather-32.png")),
    (48, include_bytes!("../../../packaging/icons/rustweather-48.png")),
    (64, include_bytes!("../../../packaging/icons/rustweather-64.png")),
    (128, include_bytes!("../../../packaging/icons/rustweather-128.png")),
    (256, include_bytes!("../../../packaging/icons/rustweather-256.png")),
    (512, include_bytes!("../../../packaging/icons/rustweather-512.png")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Missing,
    Installed,
    /// Parts are missing, or the entry starts a program in another place.
    Outdated,
    /// A system package provides it; nothing of ours to do.
    Package,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: State,
    pub entry: bool,
    pub entry_matches: bool,
    pub icons: usize,
    pub autostart: bool,
    pub program: PathBuf,
    pub entry_path: PathBuf,
}

impl Status {
    /// True when there is something of ours to remove.
    pub fn removable(&self) -> bool {
        self.entry || self.icons > 0 || (self.autostart && self.state != State::Package)
    }

    /// What belongs to the integration: name, whether it is there, a detail.
    pub fn rows(&self) -> Vec<(String, bool, String)> {
        let total = ICONS.len() + 1;
        let target = if self.entry_matches {
            self.entry_path.display().to_string()
        } else {
            "Starts another program".to_owned()
        };
        vec![
            (
                "Application menu entry".to_owned(),
                self.entry && self.entry_matches,
                if self.entry { target } else { String::new() },
            ),
            (
                "Icons".to_owned(),
                self.icons == total,
                format!("{} of {total}", self.icons),
            ),
            ("Starts with the desktop".to_owned(), self.autostart, String::new()),
            ("Program".to_owned(), true, self.program.display().to_string()),
        ]
    }
}

#[derive(Debug, Clone)]
pub struct Desktop {
    /// `~/.local/share`
    data: PathBuf,
    /// `~/.config`
    config: PathBuf,
    /// Where a package would have put the entry; empty under `--home`.
    system: Vec<PathBuf>,
}

fn exec_of(program: &Path) -> String {
    if program.starts_with("/usr/bin") || program.starts_with("/usr/local/bin") {
        return "rustweather".to_owned();
    }
    format!(
        "\"{}\"",
        program.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// The packaged entry, starting this very program.
fn entry_text(program: &Path) -> String {
    let exec = exec_of(program);
    ENTRY
        .lines()
        .map(|line| match line.strip_prefix("Exec=rustweather") {
            Some(rest) => format!("Exec={exec}{rest}\n"),
            None if line.starts_with("TryExec=") => format!("TryExec={}\n", program.display()),
            None => format!("{line}\n"),
        })
        .collect()
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, bytes)
}

/// Tells the desktop about the change. Each tool is optional: only what exists runs, and it runs detached from the app's window.
fn refresh(data: &Path) {
    let applications = data.join("applications");
    let icons = data.join("icons/hicolor");
    let calls: [(&str, Vec<String>); 3] = [
        (
            "update-desktop-database",
            vec!["-q".to_owned(), applications.display().to_string()],
        ),
        (
            "gtk-update-icon-cache",
            vec![
                "-q".to_owned(),
                "-t".to_owned(),
                "-f".to_owned(),
                icons.display().to_string(),
            ],
        ),
        ("kbuildsycoca6", Vec::new()),
    ];
    for (tool, args) in calls {
        let started = Command::new(tool)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match started {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => tracing::debug!(tool, error = %e, "[Desktop] Tool not available"),
        }
    }
}

impl Desktop {
    pub fn standard() -> Result<Self, Box<dyn std::error::Error>> {
        let dirs = directories::BaseDirs::new().ok_or("no home folder found")?;
        Ok(Self {
            data: dirs.data_dir().to_path_buf(),
            config: dirs.config_dir().to_path_buf(),
            system: vec!["/usr/share/applications".into(), "/usr/local/share/applications".into()],
        })
    }

    /// Everything inside one folder, for tests and the test environment.
    pub fn under(home: &Path) -> Self {
        Self {
            data: home.join("data"),
            config: home.join("config-home"),
            system: Vec::new(),
        }
    }

    fn entry_file(&self) -> PathBuf {
        self.data.join("applications").join(format!("{APP_ID}.desktop"))
    }

    fn autostart_file(&self) -> PathBuf {
        self.config.join("autostart").join("rustweather.desktop")
    }

    fn icon_files(&self) -> Vec<(PathBuf, &'static [u8])> {
        let hicolor = self.data.join("icons/hicolor");
        let mut files: Vec<(PathBuf, &'static [u8])> = ICONS
            .iter()
            .map(|(size, bytes)| (hicolor.join(format!("{size}x{size}/apps/rustweather.png")), *bytes))
            .collect();
        files.push((hicolor.join("scalable/apps/rustweather.svg"), SCALABLE));
        files
    }

    fn program() -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| "rustweather".into())
    }

    pub fn status(&self) -> Status {
        self.status_for(&Self::program())
    }

    fn status_for(&self, program: &Path) -> Status {
        let entry_path = self.entry_file();
        let text = fs::read_to_string(&entry_path).ok();
        let entry = text.is_some();
        let entry_matches = text.is_some_and(|t| t == entry_text(program));
        let files = self.icon_files();
        let icons = files.iter().filter(|(path, _)| path.exists()).count();
        let packaged = self
            .system
            .iter()
            .any(|dir| dir.join(format!("{APP_ID}.desktop")).exists());
        let state = if packaged {
            State::Package
        } else if !entry && icons == 0 {
            State::Missing
        } else if entry && entry_matches && icons == files.len() {
            State::Installed
        } else {
            State::Outdated
        };
        Status {
            state,
            entry,
            entry_matches,
            icons,
            autostart: self.autostart_enabled(),
            program: program.to_path_buf(),
            entry_path,
        }
    }

    pub fn install(&self) -> io::Result<()> {
        self.install_for(&Self::program())?;
        refresh(&self.data);
        Ok(())
    }

    fn install_for(&self, program: &Path) -> io::Result<()> {
        for (path, bytes) in self.icon_files() {
            write(&path, bytes)?;
        }
        write(&self.entry_file(), entry_text(program).as_bytes())?;
        if self.autostart_enabled() {
            self.write_autostart(program)?;
        }
        Ok(())
    }

    /// Removes only what the app itself wrote. Settings, places, keys and the cache stay.
    pub fn uninstall(&self) -> io::Result<()> {
        self.remove_files()?;
        refresh(&self.data);
        Ok(())
    }

    fn remove_files(&self) -> io::Result<()> {
        remove(&self.entry_file())?;
        remove(&self.autostart_file())?;
        for (path, _) in self.icon_files() {
            remove(&path)?;
        }
        Ok(())
    }

    pub fn autostart_enabled(&self) -> bool {
        self.autostart_file().exists()
    }

    pub fn set_autostart(&self, on: bool) -> io::Result<()> {
        if on {
            self.write_autostart(&Self::program())
        } else {
            remove(&self.autostart_file())
        }
    }

    fn write_autostart(&self, program: &Path) -> io::Result<()> {
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=RustWeather\nComment=Temperature in the tray and a weather radar\n\
             Exec={}\nIcon=rustweather\nX-GNOME-Autostart-enabled=true\n",
            exec_of(program)
        );
        write(&self.autostart_file(), text.as_bytes())
    }

    /// `--desktop install`, `--desktop uninstall`, `--desktop status`.
    pub fn run_action(&self, action: &str) -> Result<(), Box<dyn std::error::Error>> {
        match action {
            "install" => self.install()?,
            "uninstall" => self.uninstall()?,
            "status" => {}
            other => return Err(format!("unknown action {other}: use install, uninstall or status").into()),
        }
        let status = self.status();
        println!("{:?}", status.state);
        for (name, ok, detail) in status.rows() {
            println!("{} {name}: {detail}", if ok { "yes" } else { "no " });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> (Desktop, PathBuf) {
        let root = std::env::temp_dir().join(format!("rw-desktop-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (Desktop::under(&root), root)
    }

    #[test]
    fn install_status_uninstall() {
        let (d, root) = setup("cycle");
        let program = Path::new("/opt/rust weather/rustweather");
        assert_eq!(d.status_for(program).state, State::Missing);
        assert!(!d.status_for(program).removable());
        d.install_for(program).unwrap();
        let s = d.status_for(program);
        assert_eq!(s.state, State::Installed);
        assert_eq!(s.icons, ICONS.len() + 1);
        d.write_autostart(program).unwrap();
        d.remove_files().unwrap();
        let s = d.status_for(program);
        assert_eq!(s.state, State::Missing);
        assert!(!s.autostart);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_moved_program_or_a_missing_icon_needs_repair() {
        let (d, root) = setup("repair");
        d.install_for(Path::new("/old/rustweather")).unwrap();
        assert_eq!(d.status_for(Path::new("/new/rustweather")).state, State::Outdated);
        let (first, _) = d.icon_files().remove(0);
        fs::remove_file(first).unwrap();
        assert_eq!(d.status_for(Path::new("/old/rustweather")).state, State::Outdated);
        d.install_for(Path::new("/new/rustweather")).unwrap();
        assert_eq!(d.status_for(Path::new("/new/rustweather")).state, State::Installed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_package_wins() {
        let (mut d, root) = setup("package");
        let system = root.join("system");
        fs::create_dir_all(&system).unwrap();
        fs::write(system.join(format!("{APP_ID}.desktop")), "").unwrap();
        d.system = vec![system];
        assert_eq!(d.status_for(Path::new("/usr/bin/rustweather")).state, State::Package);
        d.write_autostart(Path::new("/usr/bin/rustweather")).unwrap();
        assert!(!d.status_for(Path::new("/usr/bin/rustweather")).removable());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_entry_starts_this_program_also_from_its_actions() {
        let text = entry_text(Path::new("/opt/rust weather/rustweather"));
        assert!(text.contains("Exec=\"/opt/rust weather/rustweather\"\n"));
        assert!(text.contains("Exec=\"/opt/rust weather/rustweather\" --open radar\n"));
        assert!(text.contains("TryExec=/opt/rust weather/rustweather\n"));
        assert!(entry_text(Path::new("/usr/bin/rustweather")).contains("Exec=rustweather --open settings\n"));
    }
}
