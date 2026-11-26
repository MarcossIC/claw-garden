//! The reviewer's model as the user prefers it: `<data>/review.json`, one
//! model for the profile and one per project, apart from `preferences.json`
//! so that a clawg from before it, which refuses a key it does not know in
//! that file, keeps reading the language.
//!
//! Everything here is pure but [`load`] and [`save`]: the file's text, the
//! rule a model id follows, the list the Cursor CLI prints, a choice from it,
//! which model a review uses, and every sentence the commands print.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clawg_core::preferences::Lang;
use serde::{Deserialize, Serialize};

/// The model a review uses when nobody chose one.
pub const DEFAULT_MODEL: &str = "gpt-5.3-codex-high";

/// The file, in the profile's data directory.
pub const FILE: &str = "review.json";

const VERSION: u32 = 1;

/// The longest model id kept, as the daemon keeps it.
const MAX_MODEL_LEN: usize = 64;

/// Why `review.json` could not be used.
#[derive(Debug, thiserror::Error)]
pub enum ReviewPrefsError {
    #[error("cannot read {FILE}: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot write {FILE}: {0}")]
    Write(std::io::Error),
    #[error("{FILE} is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{FILE} has version {0}; this clawg reads version {VERSION}")]
    Version(u32),
    #[error("{FILE} names {0:?}, which is not a model id")]
    NotAModel(String),
}

/// The file as written. `deny_unknown_fields` because a misspelled key would
/// otherwise read as no choice, and the review would run a model nobody chose.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    projects: BTreeMap<String, String>,
}

/// What the user chose: a model for the profile and one per project id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewPrefs {
    pub model: Option<String>,
    pub projects: BTreeMap<String, String>,
}

impl ReviewPrefs {
    pub fn parse(text: &str) -> Result<Self, ReviewPrefsError> {
        let on_disk: OnDisk = serde_json::from_str(text)?;
        if on_disk.version != VERSION {
            return Err(ReviewPrefsError::Version(on_disk.version));
        }
        // A model edited by hand reaches the CLI as an argument: only an id does.
        if let Some(bad) = on_disk.model.iter().chain(on_disk.projects.values()).find(|m| model_id(m).as_deref() != Some(m.as_str())) {
            return Err(ReviewPrefsError::NotAModel(bad.clone()));
        }
        Ok(Self { model: on_disk.model, projects: on_disk.projects })
    }

    pub fn to_json(&self) -> String {
        let on_disk = OnDisk { version: VERSION, model: self.model.clone(), projects: self.projects.clone() };
        serde_json::to_string_pretty(&on_disk).expect("strings and a number serialize") + "\n"
    }
}

pub fn file_in(base_dir: &Path) -> PathBuf {
    base_dir.join(FILE)
}

/// The preferences in `base_dir`; a missing file is no choice.
pub fn load(base_dir: &Path) -> Result<ReviewPrefs, ReviewPrefsError> {
    match std::fs::read_to_string(file_in(base_dir)) {
        Ok(text) => ReviewPrefs::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReviewPrefs::default()),
        Err(e) => Err(e.into()),
    }
}

/// Writes the preferences through a temporary file and a rename.
pub fn save(base_dir: &Path, prefs: &ReviewPrefs) -> Result<(), ReviewPrefsError> {
    let path = file_in(base_dir);
    let tmp = path.with_extension("json.tmp");
    clawg_core::private_dir::ensure_private_dir(base_dir).map_err(ReviewPrefsError::Write)?;
    std::fs::write(&tmp, prefs.to_json()).map_err(ReviewPrefsError::Write)?;
    std::fs::rename(&tmp, &path).map_err(ReviewPrefsError::Write)?;
    Ok(())
}

/// `raw` in lowercase when it is a model id: 1 to 64 characters of
/// `[a-z0-9._-]`, the rule the daemon keeps a model by.
pub fn model_id(raw: &str) -> Option<String> {
    let model = raw.trim().to_ascii_lowercase();
    let id = !model.is_empty()
        && model.len() <= MAX_MODEL_LEN
        && model.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'));
    id.then_some(model)
}

/// The model a review uses: `--model`, else the preference of `project`,
/// else the profile's, else [`DEFAULT_MODEL`].
pub fn resolve(flag: Option<&str>, prefs: &ReviewPrefs, project: Option<&str>) -> String {
    if let Some(flag) = flag.map(str::trim).filter(|f| !f.is_empty()) {
        return flag.to_ascii_lowercase();
    }
    project
        .and_then(|p| prefs.projects.get(p))
        .or(prefs.model.as_ref())
        .cloned()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

/// One model as `agent --list-models` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedModel {
    pub id: String,
    pub name: String,
}

/// The models of `agent --list-models`: each `<id> - <name>` line whose id is
/// a model id, in the order printed.
pub fn parse_model_list(text: &str) -> Vec<ListedModel> {
    text.lines()
        .filter_map(|line| {
            let (id, name) = line.split_once(" - ")?;
            let id = model_id(id)?;
            let name = name.trim().trim_matches('\u{200b}').trim().to_string();
            Some(ListedModel { id, name })
        })
        .collect()
}

/// What `input` picks from `list`: its number, counted from 1, or an id in it.
pub fn choose(input: &str, list: &[ListedModel]) -> Option<String> {
    let input = input.trim();
    if let Ok(n) = input.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| list.get(i)).map(|m| m.id.clone());
    }
    let id = model_id(input)?;
    list.iter().find(|m| m.id == id).map(|m| m.id.clone())
}

/// What the commands about the review model print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Said<'a> {
    /// The profile's model, or the default, and each project's.
    Shown { prefs: &'a ReviewPrefs },
    /// A model was set, for the profile or the folder's project.
    Set { model: &'a str, project: bool },
    Unset { project: bool },
    /// The line above the numbered list.
    ListHeader,
    /// A model the CLI's list does not have.
    NotInList { model: &'a str },
    /// The list could not be read, and the model was kept anyway.
    ListUnread { why: &'a str },
    /// The question after the list.
    Prompt,
    /// No terminal or no list to choose from: the model has to be given.
    NeedsModel,
    /// What was typed is not a model.
    NotAModel { raw: &'a str },
    /// The folder has no project to keep a model for.
    NoProject,
    /// `review.json` could not be read, so it was left as it was.
    Unchanged { why: &'a str },
    /// The line at the top of the picker.
    PickerTitle,
    /// What marks the family, the version and the model in use.
    InUse,
    /// Above the families.
    PickFamily,
    /// After a family's name, above its versions.
    PickVersion,
    /// The keys of a screen whose rows open; `top` is the first, which
    /// going back leaves.
    KeysToOpen { top: bool },
    /// The keys of the screen of models.
    KeysToMark,
    /// That a model takes two presses: one marks it, another confirms it.
    TwoPresses,
    /// How many rows do not fit, above the ones shown or below them.
    More { above: bool, rows: usize },
    /// The picker was left without a model confirmed.
    NotChosen,
}

/// `said` in `lang`.
pub fn says(lang: Lang, said: &Said<'_>) -> String {
    match said {
        Said::Shown { prefs } => {
            let mut lines = vec![match (lang, &prefs.model) {
                (Lang::En, Some(model)) => format!("review model: {model}"),
                (Lang::En, None) => format!("review model: {DEFAULT_MODEL} (the default)"),
                (Lang::Es, Some(model)) => format!("modelo del revisor: {model}"),
                (Lang::Es, None) => format!("modelo del revisor: {DEFAULT_MODEL} (el de por defecto)"),
            }];
            for (project, model) in &prefs.projects {
                lines.push(match lang {
                    Lang::En => format!("review model of project {project}: {model}"),
                    Lang::Es => format!("modelo del revisor del proyecto {project}: {model}"),
                });
            }
            lines.join("\n")
        }
        Said::Set { model, project: false } => match lang {
            Lang::En => format!("Review model: {model}. `clawg review` uses it unless --model or a project's choice says otherwise."),
            Lang::Es => format!("Modelo del revisor: {model}. Lo usa `clawg review` salvo que --model o la elección de un proyecto digan otro."),
        },
        Said::Set { model, project: true } => match lang {
            Lang::En => format!("Review model of this folder's project: {model}."),
            Lang::Es => format!("Modelo del revisor del proyecto de esta carpeta: {model}."),
        },
        Said::Unset { project: false } => match lang {
            Lang::En => format!("No review model chosen: `clawg review` uses {DEFAULT_MODEL}."),
            Lang::Es => format!("Sin modelo del revisor elegido: `clawg review` usa {DEFAULT_MODEL}."),
        },
        Said::Unset { project: true } => match lang {
            Lang::En => "This folder's project has no review model of its own now: it uses the profile's.".to_string(),
            Lang::Es => "El proyecto de esta carpeta ya no tiene un modelo del revisor propio: usa el del perfil.".to_string(),
        },
        Said::ListHeader => match lang {
            Lang::En => "Models of the Cursor CLI (* is the one in use):".to_string(),
            Lang::Es => "Modelos del CLI de Cursor (* es el que se usa):".to_string(),
        },
        Said::NotInList { model } => match lang {
            Lang::En => format!(
                "{model} is not a model of the Cursor CLI; nothing was changed. `clawg preferences set review-model` without a model lists them."
            ),
            Lang::Es => format!(
                "{model} no es un modelo del CLI de Cursor; no se cambió nada. `clawg preferences set review-model` sin modelo los lista."
            ),
        },
        Said::ListUnread { why } => match lang {
            Lang::En => format!("The Cursor CLI's list of models could not be read ({why}); the model is kept without checking it."),
            Lang::Es => format!("No se pudo leer la lista de modelos del CLI de Cursor ({why}); el modelo se guarda sin comprobarlo."),
        },
        Said::Prompt => match lang {
            Lang::En => "Number or id of the model: ".to_string(),
            Lang::Es => "Número o id del modelo: ".to_string(),
        },
        Said::NeedsModel => match lang {
            Lang::En => "Give the model: `clawg preferences set review-model MODEL`; nothing was changed.".to_string(),
            Lang::Es => "Indicá el modelo: `clawg preferences set review-model MODELO`; no se cambió nada.".to_string(),
        },
        Said::NotAModel { raw } => match lang {
            Lang::En => format!("{raw:?} is not a model id (1 to 64 characters of a-z, 0-9, '.', '_', '-'); nothing was changed."),
            Lang::Es => format!("{raw:?} no es un id de modelo (1 a 64 caracteres de a-z, 0-9, '.', '_', '-'); no se cambió nada."),
        },
        Said::NoProject => match lang {
            Lang::En => "This folder has no project the daemon can name; nothing was changed.".to_string(),
            Lang::Es => "Esta carpeta no tiene un proyecto que el daemon pueda nombrar; no se cambió nada.".to_string(),
        },
        Said::Unchanged { why } => match lang {
            Lang::En => format!("{why}; nothing was changed"),
            Lang::Es => format!("{why}; no se cambió nada"),
        },
        Said::PickerTitle => match lang {
            Lang::En => "Review model".to_string(),
            Lang::Es => "Modelo del revisor".to_string(),
        },
        Said::InUse => match lang {
            Lang::En => "in use".to_string(),
            Lang::Es => "en uso".to_string(),
        },
        Said::PickFamily => match lang {
            Lang::En => "Choose a family".to_string(),
            Lang::Es => "Elegí una familia".to_string(),
        },
        Said::PickVersion => match lang {
            Lang::En => "choose a version".to_string(),
            Lang::Es => "elegí una versión".to_string(),
        },
        Said::KeysToOpen { top: true } => match lang {
            Lang::En => "↑↓ move · Enter or Space: open · Esc or q: leave".to_string(),
            Lang::Es => "↑↓ mover · Enter o Espacio: abrir · Esc o q: salir".to_string(),
        },
        Said::KeysToOpen { top: false } => match lang {
            Lang::En => "↑↓ move · Enter or Space: open · Esc: back · q: leave".to_string(),
            Lang::Es => "↑↓ mover · Enter o Espacio: abrir · Esc: volver · q: salir".to_string(),
        },
        Said::KeysToMark => match lang {
            Lang::En => "↑↓ move · Enter or Space: mark · Esc: back · q: leave".to_string(),
            Lang::Es => "↑↓ mover · Enter o Espacio: marcar · Esc: volver · q: salir".to_string(),
        },
        Said::TwoPresses => match lang {
            Lang::En => "Two steps: the first press marks (●), another on the marked one confirms.".to_string(),
            Lang::Es => "Dos pasos: la primera pulsación marca (●), otra sobre el marcado confirma.".to_string(),
        },
        Said::More { above, rows } => {
            let arrow = if *above { '↑' } else { '↓' };
            match lang {
                Lang::En => format!("{arrow} {rows} more"),
                Lang::Es => format!("{arrow} {rows} más"),
            }
        }
        Said::NotChosen => match lang {
            Lang::En => "No model was chosen; nothing was changed.".to_string(),
            Lang::Es => "No se eligió ningún modelo; no se cambió nada.".to_string(),
        },
    }
}

/// The numbered list, `*` on the model in use.
pub fn numbered(list: &[ListedModel], in_use: &str) -> String {
    list.iter()
        .enumerate()
        .map(|(i, m)| format!("{}{:>3}. {} - {}", if m.id == in_use { "*" } else { " " }, i + 1, m.id, m.name))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "Available models\n\nauto - Auto (default)\ngpt-5.3-codex-high - Codex 5.3 High\ngrok-4.7-low-fast - Grok 4.7  Low Fast\u{200b}\u{200b}\ngemini-3.8-flash-low - Gemini 3.8 Flash Low\n";

    #[test]
    fn the_review_preferences_round_trip() {
        let mut prefs = ReviewPrefs { model: Some("gpt-5.3-codex-high".into()), ..ReviewPrefs::default() };
        prefs.projects.insert("repo-7142".into(), "gemini-3.8-flash-low".into());
        assert_eq!(ReviewPrefs::parse(&prefs.to_json()).unwrap(), prefs);
        assert_eq!(ReviewPrefs::default().to_json(), "{\n  \"version\": 1\n}\n");
        assert_eq!(ReviewPrefs::parse(r#"{"version":1}"#).unwrap(), ReviewPrefs::default());

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), ReviewPrefs::default(), "a missing file is no choice");
        save(dir.path(), &prefs).unwrap();
        assert_eq!(load(dir.path()).unwrap(), prefs);
        assert!(!file_in(dir.path()).with_extension("json.tmp").exists());
    }

    #[test]
    fn a_broken_or_newer_file_is_refused() {
        assert!(matches!(ReviewPrefs::parse("{ not json"), Err(ReviewPrefsError::Json(_))));
        assert!(matches!(ReviewPrefs::parse(r#"{"version":2,"model":"x"}"#), Err(ReviewPrefsError::Version(2))));
        let e = ReviewPrefs::parse(r#"{"version":1,"modelo":"x"}"#).unwrap_err();
        assert!(e.to_string().contains("modelo") && e.to_string().contains(FILE), "{e}");
        for edited in [r#"{"version":1,"model":"gpt 5 & more"}"#, r#"{"version":1,"projects":{"p":"GPT-5"}}"#] {
            assert!(matches!(ReviewPrefs::parse(edited), Err(ReviewPrefsError::NotAModel(_))), "{edited}");
        }
    }

    #[test]
    fn a_model_is_kept_only_as_an_id() {
        assert_eq!(model_id(" GPT-5.3-Codex-High ").as_deref(), Some("gpt-5.3-codex-high"));
        for bad in ["", "gpt 5", "C:/x", "claude-opus-4-8[context=1m]", &"m".repeat(65)] {
            assert_eq!(model_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_model_is_the_flag_then_the_project_then_the_profile_then_the_default() {
        let mut prefs = ReviewPrefs::default();
        assert_eq!(resolve(None, &prefs, Some("p")), DEFAULT_MODEL);
        prefs.model = Some("gemini-3.8-flash-low".into());
        assert_eq!(resolve(None, &prefs, Some("p")), "gemini-3.8-flash-low");
        prefs.projects.insert("p".into(), "gpt-5.6-sol-high".into());
        assert_eq!(resolve(None, &prefs, Some("p")), "gpt-5.6-sol-high");
        assert_eq!(resolve(None, &prefs, Some("q")), "gemini-3.8-flash-low", "another project's choice is not this one's");
        assert_eq!(resolve(None, &prefs, None), "gemini-3.8-flash-low");
        assert_eq!(resolve(Some("GPT-5.3-Codex"), &prefs, Some("p")), "gpt-5.3-codex");
        assert_eq!(resolve(Some("  "), &prefs, Some("p")), "gpt-5.6-sol-high", "a blank flag is none");
    }

    #[test]
    fn the_model_list_is_read_from_the_cursor_output() {
        let list = parse_model_list(LIST);
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["auto", "gpt-5.3-codex-high", "grok-4.7-low-fast", "gemini-3.8-flash-low"]);
        assert_eq!(list[2].name, "Grok 4.7  Low Fast", "without the zero-width spaces");
        assert_eq!(parse_model_list("Available models\n\n"), []);
        assert_eq!(parse_model_list("not logged in"), []);
    }

    #[test]
    fn a_choice_is_a_number_or_an_id_of_the_list() {
        let list = parse_model_list(LIST);
        assert_eq!(choose("2", &list).as_deref(), Some("gpt-5.3-codex-high"));
        assert_eq!(choose(" Gemini-3.8-Flash-Low \n", &list).as_deref(), Some("gemini-3.8-flash-low"));
        for bad in ["0", "5", "gpt-9", "", "-1"] {
            assert_eq!(choose(bad, &list), None, "{bad:?}");
        }
        let shown = numbered(&list, "gpt-5.3-codex-high");
        assert!(shown.contains("*  2. gpt-5.3-codex-high - Codex 5.3 High"), "{shown}");
        assert!(shown.contains("   1. auto - Auto (default)"), "{shown}");
    }

    #[test]
    fn every_sentence_is_in_both_languages() {
        let mut prefs = ReviewPrefs { model: Some("m".into()), ..ReviewPrefs::default() };
        prefs.projects.insert("p".into(), "n".into());
        let none = ReviewPrefs::default();
        let all = [
            Said::Shown { prefs: &prefs },
            Said::Shown { prefs: &none },
            Said::Set { model: "m", project: false },
            Said::Set { model: "m", project: true },
            Said::Unset { project: false },
            Said::Unset { project: true },
            Said::ListHeader,
            Said::NotInList { model: "m" },
            Said::ListUnread { why: "not found" },
            Said::Prompt,
            Said::NeedsModel,
            Said::NotAModel { raw: "x y" },
            Said::NoProject,
            Said::Unchanged { why: "review.json is broken" },
            Said::PickerTitle,
            Said::InUse,
            Said::PickFamily,
            Said::PickVersion,
            Said::KeysToOpen { top: true },
            Said::KeysToOpen { top: false },
            Said::KeysToMark,
            Said::TwoPresses,
            Said::More { above: true, rows: 3 },
            Said::More { above: false, rows: 3 },
            Said::NotChosen,
        ];
        for said in &all {
            let (en, es) = (says(Lang::En, said), says(Lang::Es, said));
            assert!(!en.is_empty() && !es.is_empty(), "{said:?}");
            assert_ne!(en, es, "{said:?} has a Spanish sentence of its own");
        }
        assert!(says(Lang::En, &Said::Shown { prefs: &none }).contains(DEFAULT_MODEL));
        assert!(says(Lang::Es, &Said::Shown { prefs: &prefs }).contains("proyecto p: n"));
        assert_eq!(says(Lang::Es, &Said::Unchanged { why: "roto" }), "roto; no se cambió nada");
    }
}
