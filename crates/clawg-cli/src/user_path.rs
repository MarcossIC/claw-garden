//! `clawg path` — putting the installed `clawg` on the user's PATH.
//!
//! # Why this lives here and not in the installer script
//!
//! NSIS has no PATH primitive; the usual recipe is string surgery on the
//! registry value, and NSIS strings are capped at `NSIS_MAX_STRLEN` (1024
//! characters in the stock build). A longer PATH comes back truncated and is
//! written back that way — the classic installer that wipes its user's PATH.
//! Rust has no such cap, and the logic becomes testable. The installer hook
//! only runs `clawg path add` / `clawg path remove`.
//!
//! # What is preserved
//!
//! Every entry other than ours, byte for byte, and the value's registry type.
//! The user PATH is normally `REG_EXPAND_SZ`; rewriting it as `REG_SZ` would
//! silently stop every `%VAR%` entry in it from expanding.

/// What a change to the PATH did.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    AlreadyPresent,
    Removed,
    NotPresent,
}

/// The PATH list separator on Windows, where this is used.
const SEPARATOR: char = ';';

/// `path` with `dir` appended, or `None` when an equivalent entry is already
/// there (or `dir` is blank) and nothing should be written.
pub fn with_entry(path: &str, dir: &str) -> Option<String> {
    if normalized(dir).is_empty() || path.split(SEPARATOR).any(|entry| same_dir(entry, dir)) {
        return None;
    }
    let mut out = path.to_string();
    if !out.is_empty() && !out.ends_with(SEPARATOR) {
        out.push(SEPARATOR);
    }
    out.push_str(dir);
    Some(out)
}

/// `path` without any entry naming `dir`, or `None` when there was none.
pub fn without_entry(path: &str, dir: &str) -> Option<String> {
    let kept: Vec<&str> = path.split(SEPARATOR).filter(|entry| !same_dir(entry, dir)).collect();
    // Splitting and rejoining untouched entries reproduces them exactly, so
    // the entry count is enough to know whether anything was removed.
    (kept.len() != path.split(SEPARATOR).count()).then(|| kept.join(";"))
}

/// Whether two PATH entries name the same directory, the way Windows reads
/// them: ignoring case, surrounding quotes and whitespace, and trailing slashes.
fn same_dir(a: &str, b: &str) -> bool {
    let a = normalized(a);
    !a.is_empty() && a == normalized(b)
}

fn normalized(entry: &str) -> String {
    entry
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// The user PATH in the registry, read and written as a raw value so its type
/// survives the round trip.
#[cfg(windows)]
pub mod registry {
    use std::io;

    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ};
    use winreg::{RegKey, RegValue};

    use super::{with_entry, without_entry, Change};

    /// Where the user PATH lives, under `HKEY_CURRENT_USER`.
    pub const ENVIRONMENT_KEY: &str = "Environment";

    const VALUE: &str = "Path";

    /// Appends `dir` to the `Path` value of `HKCU\<subkey>`.
    pub fn add(subkey: &str, dir: &str) -> io::Result<Change> {
        let key = open(subkey)?;
        // A missing value is an empty PATH, created the way Windows creates it.
        let (current, vtype) = read(&key)?.unwrap_or((String::new(), REG_EXPAND_SZ));
        match with_entry(&current, dir) {
            Some(updated) => write(&key, &updated, vtype).map(|()| Change::Added),
            None => Ok(Change::AlreadyPresent),
        }
    }

    /// Removes every entry naming `dir` from the `Path` value of `HKCU\<subkey>`.
    pub fn remove(subkey: &str, dir: &str) -> io::Result<Change> {
        let key = open(subkey)?;
        let Some((current, vtype)) = read(&key)? else {
            return Ok(Change::NotPresent);
        };
        match without_entry(&current, dir) {
            Some(updated) => write(&key, &updated, vtype).map(|()| Change::Removed),
            None => Ok(Change::NotPresent),
        }
    }

    fn open(subkey: &str) -> io::Result<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(subkey, KEY_QUERY_VALUE | KEY_SET_VALUE)
    }

    /// The value and its type, or `None` when there is no value. Refuses —
    /// rather than guesses — anything it could not write back faithfully.
    fn read(key: &RegKey) -> io::Result<Option<(String, RegType)>> {
        let raw = match key.get_raw_value(VALUE) {
            Ok(raw) => raw,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if raw.vtype != REG_SZ && raw.vtype != REG_EXPAND_SZ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the user PATH has registry type {:?}, not a string; leaving it alone", raw.vtype),
            ));
        }
        // A byte left after the last whole unit is half a unit, which no
        // reader takes for text, so dropping it rewrites what every reader sees.
        let units: Vec<u16> = raw
            .bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        let text = String::from_utf16(&units).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "the user PATH is not valid UTF-16; leaving it alone")
        })?;
        Ok(Some((text, raw.vtype)))
    }

    fn write(key: &RegKey, value: &str, vtype: RegType) -> io::Result<()> {
        let bytes = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        key.set_raw_value(VALUE, &RegValue { bytes, vtype })
    }

    /// Tells running programs — Explorer above all, which hands its
    /// environment to every terminal it starts — that the user environment
    /// changed, so a new terminal sees the new PATH without signing out.
    pub fn broadcast_environment_change() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };
        let area: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
        let mut result = 0usize;
        // With `HWND_BROADCAST` each top-level window gets the whole timeout,
        // so a few slow ones add up. It stays at 5 s anyway: a window that has
        // not taken the message when its time runs out never gets it, and an
        // Explorer that missed it hands the old PATH to every new terminal.
        //
        // SAFETY: `area` is a NUL-terminated UTF-16 string that outlives the
        // call; the timeout bounds the wait on windows that do not answer.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                area.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5_000,
                &mut result,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Users\dev\AppData\Local\Claw Garden";

    // ── with_entry ──────────────────────────────────────────────────────────

    #[test]
    fn an_empty_path_becomes_just_the_directory() {
        assert_eq!(with_entry("", DIR).as_deref(), Some(DIR));
    }

    #[test]
    fn the_directory_is_appended_after_a_separator() {
        assert_eq!(
            with_entry(r"%USERPROFILE%\bin;C:\tools", DIR).as_deref(),
            Some(r"%USERPROFILE%\bin;C:\tools;C:\Users\dev\AppData\Local\Claw Garden")
        );
    }

    #[test]
    fn a_trailing_separator_is_not_doubled() {
        assert_eq!(
            with_entry(r"C:\tools;", DIR).as_deref(),
            Some(r"C:\tools;C:\Users\dev\AppData\Local\Claw Garden")
        );
    }

    #[test]
    fn an_equivalent_entry_is_not_added_twice() {
        // Windows paths ignore case, a trailing backslash and surrounding quotes.
        let path = r#"C:\tools;"c:\users\DEV\appdata\local\claw garden\""#;
        assert_eq!(with_entry(path, DIR), None);
    }

    #[test]
    fn a_blank_directory_is_never_added() {
        assert_eq!(with_entry(r"C:\tools", "  "), None);
    }

    // ── without_entry ───────────────────────────────────────────────────────

    #[test]
    fn removing_keeps_every_other_entry_untouched() {
        let path = r"%USERPROFILE%\bin;c:\users\dev\appdata\local\claw garden\;D:\x y";
        assert_eq!(
            without_entry(path, DIR).as_deref(),
            Some(r"%USERPROFILE%\bin;D:\x y")
        );
    }

    #[test]
    fn every_copy_of_the_entry_is_removed() {
        let path = format!(r"{DIR};C:\tools;{DIR}\");
        assert_eq!(without_entry(&path, DIR).as_deref(), Some(r"C:\tools"));
    }

    #[test]
    fn removing_an_absent_entry_changes_nothing() {
        assert_eq!(without_entry(r"C:\tools;D:\x", DIR), None);
    }

    #[test]
    fn a_path_holding_only_the_entry_becomes_empty() {
        assert_eq!(without_entry(DIR, DIR).as_deref(), Some(""));
    }

    // ── the registry, against a throwaway key ───────────────────────────────

    #[cfg(windows)]
    mod registry_io {
        use super::super::registry::{add, remove};
        use super::super::Change;
        use super::DIR;
        use winreg::enums::{RegType, HKEY_CURRENT_USER, REG_EXPAND_SZ};
        use winreg::{RegKey, RegValue};

        /// A key directly under `HKCU\Software` that exists for one test and is
        /// deleted after it — the real `Environment` key is never touched by a
        /// test, and no two tests share a parent one of them could delete.
        struct ScratchKey(String);

        impl ScratchKey {
            fn new(tag: &str) -> Self {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default();
                let path = format!(r"Software\clawg-test-{tag}-{}-{nanos}", std::process::id());
                RegKey::predef(HKEY_CURRENT_USER).create_subkey(&path).expect("create scratch key");
                Self(path)
            }

            fn key(&self) -> RegKey {
                RegKey::predef(HKEY_CURRENT_USER).open_subkey(&self.0).expect("open scratch key")
            }

            fn set(&self, value: &str, vtype: RegType) {
                self.set_bytes(value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect(), vtype);
            }

            fn set_bytes(&self, bytes: Vec<u8>, vtype: RegType) {
                let key = RegKey::predef(HKEY_CURRENT_USER)
                    .open_subkey_with_flags(&self.0, winreg::enums::KEY_SET_VALUE)
                    .expect("open for write");
                key.set_raw_value("Path", &RegValue { bytes, vtype }).expect("seed value");
            }

            fn get(&self) -> (String, RegType) {
                let raw = self.key().get_raw_value("Path").expect("read value");
                let units: Vec<u16> = raw
                    .bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|&u| u != 0)
                    .collect();
                (String::from_utf16(&units).expect("utf-16"), raw.vtype)
            }
        }

        impl Drop for ScratchKey {
            fn drop(&mut self) {
                let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
            }
        }

        /// Windows deletes a key that another handle still has open, and that
        /// handle then fails every call: a test that removed the key above its
        /// own would break a parallel test creating its key there.
        #[test]
        fn a_finished_test_never_deletes_the_key_above_its_own() {
            let first = ScratchKey::new("first");
            let (parent, _) = first.0.rsplit_once('\\').expect("a key under a parent");
            let held = RegKey::predef(HKEY_CURRENT_USER).open_subkey(parent).expect("open parent");
            drop(first);
            held.query_info().expect("the parent outlives a finished test");
        }

        #[test]
        fn add_then_remove_round_trips_and_keeps_expand_sz() {
            let scratch = ScratchKey::new("roundtrip");
            scratch.set(r"%USERPROFILE%\bin;C:\tools", REG_EXPAND_SZ);

            assert_eq!(add(&scratch.0, DIR).expect("add"), Change::Added);
            let (value, vtype) = scratch.get();
            assert_eq!(value, format!(r"%USERPROFILE%\bin;C:\tools;{DIR}"));
            assert_eq!(vtype, REG_EXPAND_SZ, "the %VAR% entries must keep expanding");

            assert_eq!(add(&scratch.0, DIR).expect("add again"), Change::AlreadyPresent);

            assert_eq!(remove(&scratch.0, DIR).expect("remove"), Change::Removed);
            assert_eq!(scratch.get(), (r"%USERPROFILE%\bin;C:\tools".to_string(), REG_EXPAND_SZ));

            assert_eq!(remove(&scratch.0, DIR).expect("remove again"), Change::NotPresent);
        }

        /// The registry hands such a byte back as 0 (seen on Windows 11), and
        /// a UTF-16 reader counts whole units anyway: refusing the value would
        /// only keep `clawg path add` from a PATH every program reads.
        #[test]
        fn a_byte_left_after_the_last_unit_is_not_part_of_the_value() {
            for tail in [&[b'x'][..], &[0][..], &[0, 0, 0][..]] {
                let scratch = ScratchKey::new("odd");
                let mut bytes: Vec<u8> = r"C:\tools".encode_utf16().flat_map(u16::to_le_bytes).collect();
                bytes.extend_from_slice(tail);
                scratch.set_bytes(bytes, REG_EXPAND_SZ);

                assert_eq!(add(&scratch.0, DIR).expect("add"), Change::Added, "{tail:?}");
                assert_eq!(scratch.get(), (format!(r"C:\tools;{DIR}"), REG_EXPAND_SZ), "{tail:?}");
            }
        }

        #[test]
        fn a_missing_path_value_is_created_as_expand_sz() {
            let scratch = ScratchKey::new("missing");
            assert_eq!(add(&scratch.0, DIR).expect("add"), Change::Added);
            assert_eq!(scratch.get(), (DIR.to_string(), REG_EXPAND_SZ));
        }

        #[test]
        fn removing_from_a_missing_value_is_not_an_error() {
            let scratch = ScratchKey::new("remove-missing");
            assert_eq!(remove(&scratch.0, DIR).expect("remove"), Change::NotPresent);
        }
    }
}
