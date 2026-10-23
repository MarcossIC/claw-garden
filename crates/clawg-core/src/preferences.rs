//! The user's preferences, kept in `<data>/preferences.json`: for now, the
//! language clawg saves and searches memories in and its agent speaks.
//!
//! Without the file, or with one that cannot be read, there is no preference
//! and clawg behaves as it did before preferences existed. Every value comes
//! from a closed set, so no preference is free text that reaches a prompt.
//! Parsing is pure; only [`load`] and [`save`] touch the filesystem.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::preferences_file;

/// The only file format this build reads and writes.
const VERSION: u32 = 1;

/// A language clawg can be told to use with the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Es,
}

impl Lang {
    /// The code `preferences.json`, the API and `memory.lang` spell it with.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
        }
    }

    /// Its code as [`Lang::as_str`] writes it, and nothing else.
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Lang::En),
            "es" => Some(Lang::Es),
            _ => None,
        }
    }

    /// The language a memory is in when it is not in this one.
    pub fn other(self) -> Self {
        match self {
            Lang::En => Lang::Es,
            Lang::Es => Lang::En,
        }
    }

    /// Its name in English, for the sentences clawg writes in English.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Spanish",
        }
    }
}

/// Why `preferences.json` could not be used. A reader that only needs the
/// language treats it as no preference ([`language_in`]); `doctor` and
/// `clawg preferences` show the message so the user can fix the file.
#[derive(Debug, thiserror::Error)]
pub enum PreferencesError {
    #[error("cannot read preferences.json: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot write preferences.json: {0}")]
    Write(std::io::Error),
    #[error("preferences.json is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("preferences.json has version {0}; this clawg reads version {VERSION}")]
    Version(u32),
}

/// The file as written. `deny_unknown_fields` because a misspelled key would
/// otherwise read as no preference, and the user who set one would get
/// English with no word about why.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<Lang>,
}

/// What the user chose; `None` where they chose nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Preferences {
    pub language: Option<Lang>,
}

impl Preferences {
    pub fn parse(text: &str) -> Result<Self, PreferencesError> {
        let on_disk: OnDisk = serde_json::from_str(text)?;
        if on_disk.version != VERSION {
            return Err(PreferencesError::Version(on_disk.version));
        }
        Ok(Self { language: on_disk.language })
    }

    pub fn to_json(&self) -> String {
        let on_disk = OnDisk { version: VERSION, language: self.language };
        // A number and an optional unit enum always serialize.
        let mut json = serde_json::to_string_pretty(&on_disk).unwrap_or_default();
        json.push('\n');
        json
    }
}

/// The preferences in `base_dir`. A missing file is no preference; anything
/// else that cannot be read is an error the caller decides how to show.
pub fn load(base_dir: &Path) -> Result<Preferences, PreferencesError> {
    match std::fs::read_to_string(preferences_file(base_dir)) {
        Ok(text) => Preferences::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}

/// The preferred language in `base_dir`, or `None` without one. A file that
/// cannot be read is logged and counts as none, so a typo never fails a
/// command, a tool or a hook.
pub fn language_in(base_dir: &Path) -> Option<Lang> {
    match load(base_dir) {
        Ok(p) => p.language,
        Err(e) => {
            log::warn!("{e}; clawg goes on without a language preference");
            None
        }
    }
}

/// Writes the preferences through a temporary file and a rename, so a reader
/// sees the old file or the new one, never half. Creates the data directory
/// when nothing has run in it yet.
pub fn save(base_dir: &Path, preferences: &Preferences) -> Result<(), PreferencesError> {
    let path = preferences_file(base_dir);
    let tmp = path.with_extension("json.tmp");
    crate::private_dir::ensure_private_dir(base_dir).map_err(PreferencesError::Write)?;
    std::fs::write(&tmp, preferences.to_json()).map_err(PreferencesError::Write)?;
    std::fs::rename(&tmp, &path).map_err(PreferencesError::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_no_preference() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), Preferences::default());
        assert_eq!(language_in(dir.path()), None);
    }

    #[test]
    fn a_preference_round_trips() {
        let p = Preferences { language: Some(Lang::Es) };
        assert_eq!(p.to_json(), "{\n  \"version\": 1,\n  \"language\": \"es\"\n}\n");
        assert_eq!(Preferences::parse(&p.to_json()).unwrap(), p);
        assert_eq!(Preferences::parse(r#"{"version":1}"#).unwrap(), Preferences::default());
        assert_eq!(Preferences::default().to_json(), "{\n  \"version\": 1\n}\n");
    }

    #[test]
    fn an_unknown_version_field_or_language_is_refused() {
        let e = Preferences::parse(r#"{"version":2,"language":"es"}"#).unwrap_err();
        assert!(matches!(e, PreferencesError::Version(2)), "{e}");
        let e = Preferences::parse(r#"{"version":1,"lenguaje":"es"}"#).unwrap_err();
        assert!(e.to_string().contains("lenguaje"), "the message must name the field: {e}");
        let e = Preferences::parse(r#"{"version":1,"language":"fr"}"#).unwrap_err();
        assert!(e.to_string().contains("fr"), "the message must name the value: {e}");
        assert!(e.to_string().contains("preferences.json"), "{e}");
    }

    #[test]
    fn an_invalid_file_reads_as_no_preference() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(preferences_file(dir.path()), "{ not json").unwrap();
        assert!(matches!(load(dir.path()), Err(PreferencesError::Json(_))));
        assert_eq!(language_in(dir.path()), None);
    }

    #[test]
    fn saving_goes_through_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        save(&data, &Preferences { language: Some(Lang::En) }).unwrap();
        assert_eq!(language_in(&data), Some(Lang::En));
        assert!(!preferences_file(&data).with_extension("json.tmp").exists(), "no temporary file is left behind");
        save(&data, &Preferences::default()).unwrap();
        assert_eq!(language_in(&data), None);
    }

    #[test]
    fn a_language_names_itself_and_the_other() {
        assert_eq!(Lang::parse("es"), Some(Lang::Es));
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("ES"), None);
        assert_eq!(Lang::Es.other(), Lang::En);
        assert_eq!(Lang::En.other(), Lang::Es);
        assert_eq!((Lang::En.as_str(), Lang::En.name()), ("en", "English"));
        assert_eq!((Lang::Es.as_str(), Lang::Es.name()), ("es", "Spanish"));
    }
}
