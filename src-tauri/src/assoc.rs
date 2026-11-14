//! File types: Natendo Data Edit as a Windows handler for table files, for
//! the current user only (no administrator rights). The app is listed in
//! "Open with" and in Windows Settings > Default apps; it becomes a type's
//! default only when no other program claims that type at all. It never
//! overrides another app's choice and never touches the user's own
//! default-app choices: Windows asks the user to confirm those, so the app
//! opens the right Settings page instead.

use serde::Serialize;

/// Table types offered (default on in setup).
pub const TABLE_TYPES: &[&str] = &["csv", "tsv", "tab", "psv", "ssv"];
/// Plain text types offered (default off in setup).
pub const TEXT_TYPES: &[&str] = &["txt", "dat", "log", "prn"];
pub const PROG_ID: &str = "NatendoDataEdit.Table";
pub const APP_NAME: &str = "Natendo Data Edit";
#[cfg(windows)]
const CAPS: &str = r"Software\Natendo\Data Edit\Capabilities";

/// One file type and whether this app handles it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TypeState {
    pub ext: String,
    /// Listed for this app ("Open with").
    pub on: bool,
    /// Plain text type (off unless chosen).
    pub text: bool,
}

pub fn known(ext: &str) -> bool {
    TABLE_TYPES.contains(&ext) || TEXT_TYPES.contains(&ext)
}

/// The Windows Settings page where the user picks default apps for this app.
pub const SETTINGS_URI: &str = "ms-settings:defaultapps?registeredAppUser=Natendo%20Data%20Edit";

#[cfg(windows)]
mod imp {
    use super::*;
    use std::io;
    use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    fn classes() -> io::Result<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(r"Software\Classes", KEY_READ | KEY_WRITE)
    }

    fn exe() -> io::Result<String> {
        Ok(std::env::current_exe()?.to_string_lossy().to_string())
    }

    pub fn status() -> Vec<TypeState> {
        let hkcu = classes().ok();
        TABLE_TYPES
            .iter()
            .map(|e| (e, false))
            .chain(TEXT_TYPES.iter().map(|e| (e, true)))
            .map(|(e, text)| {
                let on = hkcu
                    .as_ref()
                    .and_then(|c| c.open_subkey(format!(r".{e}\OpenWithProgids")).ok())
                    .is_some_and(|k| k.enum_values().flatten().any(|(n, _)| n == PROG_ID));
                TypeState {
                    ext: (*e).to_string(),
                    on,
                    text,
                }
            })
            .collect()
    }

    fn register_app(c: &RegKey) -> io::Result<()> {
        let exe = exe()?;
        let (prog, _) = c.create_subkey(PROG_ID)?;
        prog.set_value("", &"Table (Natendo Data Edit)")?;
        prog.set_value("FriendlyTypeName", &"Table (Natendo Data Edit)")?;
        let (icon, _) = prog.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{exe}\",0"))?;
        let (cmd, _) = prog.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &format!("\"{exe}\" \"%1\""))?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (caps, _) = hkcu.create_subkey(CAPS)?;
        caps.set_value("ApplicationName", &APP_NAME)?;
        caps.set_value(
            "ApplicationDescription",
            &"Edit tables in CSV, TSV and other delimited text files.",
        )?;
        let (apps, _) = hkcu.create_subkey(r"Software\RegisteredApplications")?;
        apps.set_value(APP_NAME, &CAPS)?;
        Ok(())
    }

    /// Does any program (this user's or the machine's) claim the type?
    fn claimed(ext: &str) -> bool {
        RegKey::predef(HKEY_CLASSES_ROOT)
            .open_subkey(format!(".{ext}"))
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .is_some_and(|v| !v.is_empty() && v != PROG_ID)
    }

    fn add(c: &RegKey, ext: &str) -> io::Result<()> {
        let (k, _) = c.create_subkey(format!(r".{ext}\OpenWithProgids"))?;
        k.set_raw_value(
            PROG_ID,
            &winreg::RegValue {
                bytes: Vec::new().into(),
                vtype: winreg::enums::RegType::REG_NONE,
            },
        )?;
        let caps = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(format!(r"{CAPS}\FileAssociations"), KEY_READ | KEY_WRITE);
        let caps = match caps {
            Ok(k) => k,
            Err(_) => {
                RegKey::predef(HKEY_CURRENT_USER)
                    .create_subkey(format!(r"{CAPS}\FileAssociations"))?
                    .0
            }
        };
        caps.set_value(format!(".{ext}"), &PROG_ID)?;
        if !claimed(ext) {
            let (k, _) = c.create_subkey(format!(".{ext}"))?;
            k.set_value("", &PROG_ID)?;
        }
        Ok(())
    }

    fn remove(c: &RegKey, ext: &str) {
        if let Ok(k) = c.open_subkey_with_flags(format!(r".{ext}\OpenWithProgids"), KEY_WRITE) {
            let _ = k.delete_value(PROG_ID);
        }
        if let Ok(k) = c.open_subkey_with_flags(format!(".{ext}"), KEY_READ | KEY_WRITE) {
            if k.get_value::<String, _>("").is_ok_and(|v| v == PROG_ID) {
                let _ = k.delete_value("");
            }
        }
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(format!(r"{CAPS}\FileAssociations"), KEY_WRITE)
        {
            let _ = k.delete_value(format!(".{ext}"));
        }
        // Keys left empty are removed (never one that still holds anything).
        drop_if_empty(c, &format!(r".{ext}\OpenWithProgids"));
        drop_if_empty(c, &format!(".{ext}"));
    }

    /// Delete a key that has no values and no subkeys.
    fn drop_if_empty(parent: &RegKey, path: &str) {
        let empty = parent
            .open_subkey(path)
            .ok()
            .and_then(|k| k.query_info().ok())
            .is_some_and(|i| i.sub_keys == 0 && i.values == 0);
        if empty {
            let _ = parent.delete_subkey(path);
        }
    }

    /// Handle exactly these types (others offered are released).
    pub fn set(on: &[String]) -> io::Result<()> {
        let c = classes()?;
        if on.is_empty() {
            return remove_all();
        }
        register_app(&c)?;
        for e in TABLE_TYPES.iter().chain(TEXT_TYPES) {
            if on.iter().any(|x| x == e) {
                add(&c, e)?;
            } else {
                remove(&c, e);
            }
        }
        Ok(())
    }

    /// Release every type and remove the app's registration.
    pub fn remove_all() -> io::Result<()> {
        let c = classes()?;
        for e in TABLE_TYPES.iter().chain(TEXT_TYPES) {
            remove(&c, e);
        }
        let _ = c.delete_subkey_all(PROG_ID);
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(k) = hkcu.open_subkey_with_flags(r"Software\RegisteredApplications", KEY_WRITE) {
            let _ = k.delete_value(APP_NAME);
        }
        let _ = hkcu.delete_subkey_all(r"Software\Natendo\Data Edit");
        drop_if_empty(&hkcu, r"Software\Natendo");
        Ok(())
    }

    pub fn open_settings() -> io::Result<()> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("explorer.exe")
            .arg(SETTINGS_URI)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use std::io;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "file types are set by the system on this platform",
        )
    }
    pub fn status() -> Vec<TypeState> {
        Vec::new()
    }
    pub fn set(_on: &[String]) -> io::Result<()> {
        Err(unsupported())
    }
    pub fn remove_all() -> io::Result<()> {
        Err(unsupported())
    }
    pub fn open_settings() -> io::Result<()> {
        Err(unsupported())
    }
}

pub use imp::{open_settings, remove_all, set, status};

/// `--register-file-types csv,tsv` / `--unregister-file-types` (setup and
/// uninstall): do it and return the exit code, or `None` to start the app.
pub fn from_args(args: &[String]) -> Option<i32> {
    match args {
        [flag, list] if flag == "--register-file-types" => {
            let mut on: Vec<String> = status()
                .into_iter()
                .filter(|t| t.on)
                .map(|t| t.ext)
                .collect();
            for e in list
                .split(',')
                .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            {
                if known(&e) && !on.contains(&e) {
                    on.push(e);
                }
            }
            Some(i32::from(set(&on).is_err()))
        }
        [flag] if flag == "--unregister-file-types" => Some(i32::from(remove_all().is_err())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_types_are_offered() {
        assert!(known("csv") && known("txt"));
        assert!(!known("exe") && !known("") && !known("csv\\..\\x"));
        assert_eq!(from_args(&["--version".into()]), None);
        assert_eq!(from_args(&[]), None);
    }
}
