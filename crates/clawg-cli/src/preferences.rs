//! `clawg preferences`: show or change the user's preferences.
//!
//! Works on `<data>/preferences.json` alone, never the store, so it runs with
//! or without the daemon. Its sentences come in the language a change leaves
//! set, one `match` per sentence, so neither language can be left without one.

use std::path::Path;

use clawg_core::preferences::{self, Lang};

use crate::cli::PreferencesAction;

/// Applies `action` (none shows the preferences) and returns what to print.
pub fn run(base_dir: &Path, action: Option<&PreferencesAction>) -> Result<String, String> {
    // An unreadable file is never overwritten: it may be a hand edit halfway
    // done, and a later clawg may have written keys this one cannot read.
    let mut current = preferences::load(base_dir).map_err(|e| match action {
        None => e.to_string(),
        Some(_) => format!("{e}; nothing was changed"),
    })?;
    let language = match action {
        None => return Ok(shown(current.language)),
        Some(PreferencesAction::Set(s)) => Some(s.value),
        Some(PreferencesAction::Unset(_)) => None,
    };
    current.language = language;
    preferences::save(base_dir, &current).map_err(|e| e.to_string())?;
    Ok(applied(language))
}

fn shown(language: Option<Lang>) -> String {
    match language {
        None => "no language preference: clawg uses English",
        Some(Lang::En) => "language: English",
        Some(Lang::Es) => "idioma: español",
    }
    .to_string()
}

fn applied(language: Option<Lang>) -> String {
    match language {
        None => {
            "No language preference: clawg uses English. Claude Code sessions opened from now on use it; \
             `clawg setup claude-code` rewrites the protocol block of CLAUDE.md."
        }
        Some(Lang::En) => {
            "Language: English. Claude Code sessions opened from now on use it; \
             `clawg setup claude-code` rewrites the protocol block of CLAUDE.md."
        }
        Some(Lang::Es) => {
            "Idioma: español. Lo usan las sesiones de Claude Code que se abran a partir de ahora; \
             `clawg setup claude-code` reescribe el bloque del protocolo de CLAUDE.md."
        }
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{PreferenceKey, ProfileArg, SetPreference, UnsetPreference};
    use clawg_core::paths::preferences_file;

    fn set(lang: Lang) -> PreferencesAction {
        PreferencesAction::Set(SetPreference { key: PreferenceKey::Language, value: lang, profile: ProfileArg::default() })
    }

    fn unset() -> PreferencesAction {
        PreferencesAction::Unset(UnsetPreference { key: PreferenceKey::Language, profile: ProfileArg::default() })
    }

    #[test]
    fn the_preferences_command_speaks_the_language_it_sets() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run(dir.path(), None).unwrap(), "no language preference: clawg uses English");

        let es = run(dir.path(), Some(&set(Lang::Es))).unwrap();
        assert!(es.starts_with("Idioma: español."), "{es}");
        assert!(es.contains("sesiones de Claude Code") && es.contains("clawg setup claude-code"), "{es}");
        assert_eq!(run(dir.path(), None).unwrap(), "idioma: español");

        let en = run(dir.path(), Some(&set(Lang::En))).unwrap();
        assert!(en.starts_with("Language: English."), "{en}");
        assert!(en.contains("Claude Code sessions") && en.contains("clawg setup claude-code"), "{en}");
        assert_eq!(run(dir.path(), None).unwrap(), "language: English");

        let none = run(dir.path(), Some(&unset())).unwrap();
        assert!(none.starts_with("No language preference: clawg uses English."), "{none}");
        assert!(none.contains("clawg setup claude-code"), "{none}");
        assert_eq!(clawg_core::preferences::language_in(dir.path()), None);
    }

    #[test]
    fn a_change_on_an_invalid_file_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let hand_edit = r#"{"version":1,"language":"es",}"#;
        std::fs::write(preferences_file(dir.path()), hand_edit).unwrap();

        let shown = run(dir.path(), None).unwrap_err();
        assert!(shown.contains("preferences.json is not valid"), "{shown}");
        for action in [set(Lang::En), unset()] {
            let refused = run(dir.path(), Some(&action)).unwrap_err();
            assert!(refused.ends_with("nothing was changed"), "{refused}");
        }
        assert_eq!(std::fs::read_to_string(preferences_file(dir.path())).unwrap(), hand_edit);
    }
}
