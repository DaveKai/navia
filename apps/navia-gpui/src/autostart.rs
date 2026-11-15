//! The Open at login setting. On Linux it's an XDG autostart entry, on Windows a per-user
//! Run key value. The setting reads what is there, so changes made in the desktop's own
//! startup settings show up too.

/// The argument that starts Navia in the tray with no window.
pub const HIDDEN_ARG: &str = "--hidden";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenAtLogin {
    Off,
    On,
    /// Opens in the tray, with no window.
    Hidden,
}

impl OpenAtLogin {
    pub fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Hidden => "hidden",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "on" => Self::On,
            "hidden" => Self::Hidden,
            _ => Self::Off,
        }
    }
}

/// The app's name for files and the Run key. The packaged name, or "navia".
pub fn app_name() -> &'static str {
    option_env!("NAVIA_PACKAGE_NAME").unwrap_or("navia")
}

/// Whether Navia was started hidden at login.
pub fn started_hidden() -> bool {
    std::env::args().skip(1).any(|a| a == HIDDEN_ARG)
}

/// The program to start. An AppImage's own file rather than its temporary mount, else this binary.
fn program() -> Option<String> {
    if let Some(image) = std::env::var_os("APPIMAGE").filter(|p| !p.is_empty()) {
        return Some(image.to_string_lossy().into_owned());
    }
    std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::path::{Path, PathBuf};

    /// `$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart`.
    fn autostart_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("autostart"))
    }

    fn entry_path(dir: &Path) -> PathBuf {
        dir.join(format!("{}.desktop", app_name()))
    }

    /// Quotes a path for a desktop entry's Exec key, then doubles `%`.
    pub(super) fn exec_quote(path: &str) -> String {
        let plain = !path.is_empty() && path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+,:@".contains(c));
        if plain {
            return path.to_string();
        }
        let mut out = String::from("\"");
        for c in path.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out.replace('%', "%%")
    }

    pub(super) fn entry_text(program: &str, hidden: bool) -> String {
        let exec = if hidden { format!("{} {HIDDEN_ARG}", exec_quote(program)) } else { exec_quote(program) };
        format!(
            "[Desktop Entry]\nType=Application\nName={display}\nExec={exec}\nIcon={icon}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-after=panel\n",
            display = navia_core::client::APP_DISPLAY_NAME,
            icon = navia_core::client::APP_ID
        )
    }

    /// Parses an entry. It's off when missing, hidden or disabled.
    pub(super) fn parse(text: &str) -> OpenAtLogin {
        let mut exec = None;
        let mut in_entry = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            if !in_entry {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            match (key.trim(), value.trim()) {
                ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => return OpenAtLogin::Off,
                ("Exec", v) => exec = Some(v.to_string()),
                _ => {}
            }
        }
        match exec {
            Some(e) if e.split_whitespace().any(|w| w == HIDDEN_ARG) => OpenAtLogin::Hidden,
            Some(_) => OpenAtLogin::On,
            None => OpenAtLogin::Off,
        }
    }

    pub(super) fn read_in(dir: &Path) -> OpenAtLogin {
        std::fs::read_to_string(entry_path(dir)).map(|t| parse(&t)).unwrap_or(OpenAtLogin::Off)
    }

    pub(super) fn write_in(dir: &Path, choice: OpenAtLogin, program: &str) -> std::io::Result<()> {
        let path = entry_path(dir);
        if choice == OpenAtLogin::Off {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(dir)?;
        std::fs::write(path, entry_text(program, choice == OpenAtLogin::Hidden))
    }

    pub fn current() -> OpenAtLogin {
        autostart_dir().map(|d| read_in(&d)).unwrap_or(OpenAtLogin::Off)
    }

    pub fn set(choice: OpenAtLogin) -> std::io::Result<()> {
        let dir = autostart_dir().ok_or_else(|| std::io::Error::other("no config folder"))?;
        let program = program().ok_or_else(|| std::io::Error::other("can't tell where Navia is"))?;
        write_in(&dir, choice, &program)
    }
}

#[cfg(windows)]
mod imp {
    //! The per-user Run key, with a value named "Navia" as startup lists show it.
    use super::*;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW};

    const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: u32) -> Option<HKEY> {
        let mut key: HKEY = std::ptr::null_mut();
        let ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), 0, access, &mut key) } == ERROR_SUCCESS;
        ok.then_some(key)
    }

    pub fn current() -> OpenAtLogin {
        let Some(key) = open(KEY_QUERY_VALUE) else { return OpenAtLogin::Off };
        let name = wide(navia_core::client::APP_DISPLAY_NAME);
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let found = unsafe { RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) } == ERROR_SUCCESS;
        unsafe { RegCloseKey(key) };
        if !found {
            return OpenAtLogin::Off;
        }
        let text = String::from_utf16_lossy(&buf[..(len as usize / 2)]);
        if text.split_whitespace().any(|w| w.trim_matches('\0') == HIDDEN_ARG) { OpenAtLogin::Hidden } else { OpenAtLogin::On }
    }

    pub fn set(choice: OpenAtLogin) -> std::io::Result<()> {
        let key = open(KEY_SET_VALUE).ok_or_else(|| std::io::Error::other("can't open the Run key"))?;
        let name = wide(navia_core::client::APP_DISPLAY_NAME);
        let status = if choice == OpenAtLogin::Off {
            let s = unsafe { RegDeleteValueW(key, name.as_ptr()) };
            if s == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND { ERROR_SUCCESS } else { s }
        } else {
            let program = program().ok_or_else(|| std::io::Error::other("can't tell where Navia is"))?;
            let command = if choice == OpenAtLogin::Hidden { format!("\"{program}\" {HIDDEN_ARG}") } else { format!("\"{program}\"") };
            let data = wide(&command);
            unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) }
        };
        unsafe { RegCloseKey(key) };
        if status == ERROR_SUCCESS { Ok(()) } else { Err(std::io::Error::from_raw_os_error(status as i32)) }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use super::*;
    pub fn current() -> OpenAtLogin {
        OpenAtLogin::Off
    }
    pub fn set(_: OpenAtLogin) -> std::io::Result<()> {
        Err(std::io::Error::other("not supported here"))
    }
}

pub use imp::{current, set};

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::imp::*;
    use super::*;

    #[test]
    fn entries_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_in(dir.path()), OpenAtLogin::Off);
        write_in(dir.path(), OpenAtLogin::On, "/usr/bin/navia").unwrap();
        assert_eq!(read_in(dir.path()), OpenAtLogin::On);
        write_in(dir.path(), OpenAtLogin::Hidden, "/opt/My Apps/navia").unwrap();
        assert_eq!(read_in(dir.path()), OpenAtLogin::Hidden);
        let text = std::fs::read_to_string(dir.path().join(format!("{}.desktop", app_name()))).unwrap();
        assert!(text.contains("Exec=\"/opt/My Apps/navia\" --hidden\n"), "{text}");
        write_in(dir.path(), OpenAtLogin::Off, "/usr/bin/navia").unwrap();
        assert_eq!(read_in(dir.path()), OpenAtLogin::Off);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        // Off twice is fine.
        write_in(dir.path(), OpenAtLogin::Off, "/usr/bin/navia").unwrap();
    }

    #[test]
    fn disabled_entries_are_off() {
        assert_eq!(parse("[Desktop Entry]\nExec=navia\nHidden=true\n"), OpenAtLogin::Off);
        assert_eq!(parse("[Desktop Entry]\nExec=navia --hidden\nX-GNOME-Autostart-enabled=false\n"), OpenAtLogin::Off);
        assert_eq!(parse("[Desktop Entry]\nExec=navia --hidden\n"), OpenAtLogin::Hidden);
        // Keys in other groups don't count.
        assert_eq!(parse("[Desktop Entry]\nExec=navia\n[Desktop Action x]\nExec=navia --hidden\n"), OpenAtLogin::On);
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(exec_quote("/usr/bin/navia"), "/usr/bin/navia");
        assert_eq!(exec_quote("/a b/100%/$x"), "\"/a b/100%%/\\$x\"");
    }
}
