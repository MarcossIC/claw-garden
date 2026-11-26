//! The screen the reviewer's model is chosen on: the models of the Cursor CLI
//! by family, then by version, then one by one.
//!
//! Everything here is pure: how a list is grouped, what a key does and what
//! is drawn. The terminal it runs over is a trait, so a test can stand in for it.

use super::prefs::ListedModel;

/// The words an id ends with that say how hard and how fast a model works,
/// never which model it is.
const VARIANT_WORDS: [&str; 10] = ["none", "minimal", "low", "medium", "high", "xhigh", "extra", "max", "fast", "thinking"];

/// The models that are one model at different efforts and speeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub label: String,
    pub models: Vec<ListedModel>,
}

/// The versions of one maker's line of models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub label: String,
    pub versions: Vec<Version>,
}

impl Family {
    pub fn model_count(&self) -> usize {
        self.versions.iter().map(|v| v.models.len()).sum()
    }
}

/// The letters `model`'s name starts with, or the first word of its id for a
/// name that starts with none. A rule, not a table: the CLI's list changes
/// with each of its releases.
fn family_label(model: &ListedModel) -> String {
    let letters: String = model.name.trim_start().chars().take_while(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return model.id.split('-').next().unwrap_or_default().to_string();
    }
    letters
}

/// `id` without the effort and speed words it ends with, never without a word.
fn version_key(id: &str) -> String {
    let mut words: Vec<&str> = id.split('-').collect();
    while words.len() > 1 && words.last().is_some_and(|last| VARIANT_WORDS.contains(last)) {
        words.pop();
    }
    words.join("-")
}

/// The words every name of `models` starts with.
fn shared_words(models: &[ListedModel]) -> String {
    let mut names = models.iter().map(|m| m.name.split_whitespace().collect::<Vec<_>>());
    let Some(mut shared) = names.next() else { return String::new() };
    for name in names {
        let same = shared.iter().zip(&name).take_while(|(a, b)| a == b).count();
        shared.truncate(same);
    }
    shared.join(" ")
}

/// `list` by family and version, each in the order the list first names it.
pub fn families(list: &[ListedModel]) -> Vec<Family> {
    let mut grouped: Vec<(String, Vec<(String, Vec<ListedModel>)>)> = Vec::new();
    for model in list {
        let (family, key) = (family_label(model), version_key(&model.id));
        let at = grouped.iter().position(|(label, _)| *label == family).unwrap_or_else(|| {
            grouped.push((family, Vec::new()));
            grouped.len() - 1
        });
        let versions = &mut grouped[at].1;
        match versions.iter_mut().find(|(k, _)| *k == key) {
            Some((_, models)) => models.push(model.clone()),
            None => versions.push((key, vec![model.clone()])),
        }
    }
    grouped
        .into_iter()
        .map(|(label, versions)| Family {
            label,
            versions: versions
                .into_iter()
                .map(|(key, models)| {
                    let shared = shared_words(&models);
                    Version { label: if shared.is_empty() { key } else { shared }, models }
                })
                .collect(),
        })
        .collect()
}

/// A key the picker answers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    /// Enter or the space bar: open a family or a version, mark a model, or
    /// confirm the marked one.
    Pick,
    /// One step back, out of the picker from the families.
    Back,
    Quit,
}

/// What a key left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Still choosing.
    Open,
    /// The id of the model confirmed.
    Chosen(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Families,
    Versions { family: usize },
    Models { family: usize, version: usize },
}

/// Where a model is: its family, its version in that family, its place in
/// that version.
type Place = (usize, usize, usize);

fn place_of(families: &[Family], id: &str) -> Option<Place> {
    families.iter().enumerate().find_map(|(f, family)| {
        family.versions.iter().enumerate().find_map(|(v, version)| version.models.iter().position(|m| m.id == id).map(|m| (f, v, m)))
    })
}

/// The choice under way: the screen shown, the row the cursor is on and the
/// model marked. A model is chosen in two presses, one that marks it and one
/// on the marked model that confirms it, so that a key held down or pressed
/// on the wrong row does not change the reviewer.
#[derive(Debug, Clone)]
pub struct Picker {
    families: Vec<Family>,
    in_use: Option<Place>,
    screen: Screen,
    cursor: usize,
    marked: Option<usize>,
}

impl Picker {
    /// The picker of `list`, on the family of the model `in_use`.
    pub fn new(list: &[ListedModel], in_use: &str) -> Self {
        let families = families(list);
        let in_use = place_of(&families, in_use);
        Self { cursor: in_use.map_or(0, |(family, _, _)| family), families, in_use, screen: Screen::Families, marked: None }
    }

    fn rows(&self) -> usize {
        match self.screen {
            Screen::Families => self.families.len(),
            Screen::Versions { family } => self.families[family].versions.len(),
            Screen::Models { family, version } => self.families[family].versions[version].models.len(),
        }
    }

    /// What `key` does to the choice.
    pub fn press(&mut self, key: Key) -> Outcome {
        let rows = self.rows();
        match key {
            Key::Quit => return Outcome::Cancelled,
            Key::Pick => return self.pick(),
            Key::Back => return self.back(),
            Key::Up if rows > 0 => self.cursor = (self.cursor + rows - 1) % rows,
            Key::Down if rows > 0 => self.cursor = (self.cursor + 1) % rows,
            Key::Up | Key::Down => {}
        }
        Outcome::Open
    }

    fn pick(&mut self) -> Outcome {
        match self.screen {
            Screen::Families => match self.families.get(self.cursor).map(|family| family.versions.len()) {
                None => {}
                Some(1) => self.open_models(self.cursor, 0),
                Some(_) => {
                    let family = self.cursor;
                    self.screen = Screen::Versions { family };
                    self.cursor = self.in_use.filter(|(f, _, _)| *f == family).map_or(0, |(_, version, _)| version);
                }
            },
            Screen::Versions { family } => self.open_models(family, self.cursor),
            Screen::Models { family, version } => {
                if self.marked == Some(self.cursor) {
                    return Outcome::Chosen(self.families[family].versions[version].models[self.cursor].id.clone());
                }
                self.marked = Some(self.cursor);
            }
        }
        Outcome::Open
    }

    fn open_models(&mut self, family: usize, version: usize) {
        self.screen = Screen::Models { family, version };
        self.cursor = self.in_use.filter(|(f, v, _)| (*f, *v) == (family, version)).map_or(0, |(_, _, model)| model);
        self.marked = None;
    }

    fn back(&mut self) -> Outcome {
        match self.screen {
            Screen::Families => return Outcome::Cancelled,
            Screen::Versions { family } => (self.screen, self.cursor) = (Screen::Families, family),
            Screen::Models { family, version } => {
                self.marked = None;
                (self.screen, self.cursor) = match self.families[family].versions.len() {
                    1 => (Screen::Families, family),
                    _ => (Screen::Versions { family }, version),
                };
            }
        }
        Outcome::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::prefs::parse_model_list;

    /// A cut of what the CLI lists, in its order: a version's models are not
    /// always next to each other.
    const LIST: &str = "Available models

auto - Auto (default)
gpt-5.3-codex-low - Codex 5.3 Low
gpt-5.3-codex-high - Codex 5.3 High
gpt-5.3-codex-xhigh-fast - Codex 5.3 Extra High Fast
gpt-5.2 - GPT-5.2
claude-opus-5-thinking-high - Claude Opus 5 1M Thinking
cursor-grok-4.5-high - Grok 4.5
cursor-grok-4.5-high-fast - Grok 4.5 Fast
claude-opus-5-5-low - Claude Opus 5.5 1M Low
claude-opus-5-5-medium - Claude Opus 5.5 1M
claude-opus-5-5-max-fast - Claude Opus 5.5 1M Max Fast
claude-opus-5-low - Claude Opus 5 1M Low
gpt-5.5-none - GPT-5.5 1M None
gpt-5.5-extra-high-fast - GPT-5.5 Extra High Fast
gpt-5.2-fast - GPT-5.2 Fast
claude-4.6-sonnet-medium - Claude Sonnet 4.6 1M
claude-4.6-sonnet-medium-thinking - Claude Sonnet 4.6 1M Thinking
gemini-3.1-pro - Gemini 3.1 Pro
kimi-k2.7-code - Kimi K2.7 Code
";

    fn listed() -> Vec<ListedModel> {
        parse_model_list(LIST)
    }

    fn model(id: &str, name: &str) -> ListedModel {
        ListedModel { id: id.into(), name: name.into() }
    }

    fn family<'a>(all: &'a [Family], label: &str) -> &'a Family {
        all.iter().find(|f| f.label == label).unwrap_or_else(|| panic!("no family {label}"))
    }

    fn ids(version: &Version) -> Vec<&str> {
        version.models.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn a_family_is_named_by_the_letters_its_model_names_start_with() {
        let all = families(&listed());
        let labels: Vec<&str> = all.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["Auto", "Codex", "GPT", "Claude", "Grok", "Gemini", "Kimi"]);
        assert_eq!(family(&all, "Codex").model_count(), 3, "a Codex is not a GPT, whatever its id starts with");
        assert_eq!(family(&all, "Grok").model_count(), 2, "nor is a Grok a cursor");
        assert_eq!(family(&all, "GPT").model_count(), 4);

        let unnamed = families(&[model("glm-5.2-high", "5.2 High"), model("glm-5.2-max", "")]);
        assert_eq!(unnamed.len(), 1, "{unnamed:?}");
        assert_eq!(unnamed[0].label, "glm", "the first word of the id, for a name that starts with no letter");
    }

    #[test]
    fn a_version_holds_the_ids_that_differ_only_in_effort_and_speed() {
        let all = families(&listed());
        let claude: Vec<Vec<&str>> = family(&all, "Claude").versions.iter().map(ids).collect();
        assert_eq!(
            claude,
            [
                vec!["claude-opus-5-thinking-high", "claude-opus-5-low"],
                vec!["claude-opus-5-5-low", "claude-opus-5-5-medium", "claude-opus-5-5-max-fast"],
                vec!["claude-4.6-sonnet-medium", "claude-4.6-sonnet-medium-thinking"],
            ]
        );
        let gpt: Vec<Vec<&str>> = family(&all, "GPT").versions.iter().map(ids).collect();
        assert_eq!(gpt, [vec!["gpt-5.2", "gpt-5.2-fast"], vec!["gpt-5.5-none", "gpt-5.5-extra-high-fast"]]);
        assert_eq!(family(&all, "Kimi").versions.len(), 1, "`code` is not an effort");

        for (id, key) in [("auto", "auto"), ("max", "max"), ("high-fast", "high"), ("muse-wonder-1.0-minimal", "muse-wonder-1.0")] {
            assert_eq!(version_key(id), key, "an id is never left without a word");
        }
    }

    #[test]
    fn a_version_is_labelled_by_the_words_its_names_share() {
        let all = families(&listed());
        let labels = |name: &str| family(&all, name).versions.iter().map(|v| v.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels("Claude"), ["Claude Opus 5 1M", "Claude Opus 5.5 1M", "Claude Sonnet 4.6 1M"]);
        assert_eq!(labels("GPT"), ["GPT-5.2", "GPT-5.5"]);
        assert_eq!(labels("Codex"), ["Codex 5.3"]);
        assert_eq!(labels("Gemini"), ["Gemini 3.1 Pro"], "the whole name of a version of one model");
        assert_eq!(labels("Auto"), ["Auto (default)"]);

        let spaced = families(&[model("grok-4.7-low", "Grok 4.7  Low"), model("grok-4.7-high", "Grok 4.7  High")]);
        assert_eq!(spaced[0].versions[0].label, "Grok 4.7");
        let nameless = families(&[model("x-1-low", ""), model("x-1-high", "")]);
        assert_eq!(nameless[0].versions[0].label, "x-1", "its key, when the names share no word");
    }

    #[test]
    fn every_listed_model_is_in_one_version_in_the_order_listed() {
        let list = listed();
        let all = families(&list);
        let mut grouped: Vec<&ListedModel> = all.iter().flat_map(|f| &f.versions).flat_map(|v| &v.models).collect();
        assert_eq!(grouped.len(), list.len(), "none is lost and none is twice");
        for f in &all {
            for v in &f.versions {
                let at: Vec<usize> = v.models.iter().map(|m| list.iter().position(|l| l == m).expect("listed")).collect();
                assert!(at.windows(2).all(|w| w[0] < w[1]), "{}: {at:?}", v.label);
            }
            let first: Vec<usize> =
                f.versions.iter().map(|v| list.iter().position(|l| *l == v.models[0]).expect("listed")).collect();
            assert!(first.windows(2).all(|w| w[0] < w[1]), "{}: {first:?}", f.label);
        }
        grouped.sort_by_key(|m| list.iter().position(|l| l == *m));
        assert!(grouped.iter().zip(&list).all(|(a, b)| *a == b));
        assert_eq!(families(&[]), []);
    }

    const AUTO: usize = 0;
    const CODEX: usize = 1;
    const CLAUDE: usize = 3;
    const KIMI: usize = 6;

    fn picker(in_use: &str) -> Picker {
        Picker::new(&listed(), in_use)
    }

    /// The outcome of the last of `keys`.
    fn press(picker: &mut Picker, keys: &[Key]) -> Outcome {
        keys.iter().map(|key| picker.press(*key)).last().expect("a key")
    }

    /// A picker on the models of the family at `family`, through its version
    /// at `version` when it has more than one.
    fn on_models(family: usize, version: usize) -> Picker {
        let mut picker = picker("not-listed");
        let down = |n: usize| vec![Key::Down; n];
        press(&mut picker, &[down(family), vec![Key::Pick]].concat());
        if matches!(picker.screen, Screen::Versions { .. }) {
            press(&mut picker, &[down(version), vec![Key::Pick]].concat());
        }
        assert_eq!(picker.screen, Screen::Models { family, version });
        picker
    }

    #[test]
    fn the_picker_opens_on_the_family_in_use() {
        let mut picker = picker("claude-opus-5-5-medium");
        assert_eq!((picker.screen, picker.cursor), (Screen::Families, CLAUDE));
        assert_eq!(picker.press(Key::Pick), Outcome::Open);
        assert_eq!((picker.screen, picker.cursor), (Screen::Versions { family: CLAUDE }, 1), "and on its version");
        picker.press(Key::Pick);
        assert_eq!((picker.screen, picker.cursor), (Screen::Models { family: CLAUDE, version: 1 }, 1), "and on the model");
        assert_eq!(picker.marked, None, "the model in use is not marked for the person");

        let mut elsewhere = self::picker("claude-opus-5-5-medium");
        press(&mut elsewhere, &[Key::Pick, Key::Up, Key::Pick]);
        assert_eq!((elsewhere.screen, elsewhere.cursor), (Screen::Models { family: CLAUDE, version: 0 }, 0), "another version opens on its first");

        let unlisted = self::picker("gpt-9-imaginary");
        assert_eq!((unlisted.screen, unlisted.cursor), (Screen::Families, AUTO));
    }

    #[test]
    fn the_cursor_moves_and_wraps() {
        let mut families = picker("not-listed");
        assert_eq!(press(&mut families, &[Key::Down, Key::Down]), Outcome::Open);
        assert_eq!(families.cursor, 2);
        press(&mut families, &[Key::Up, Key::Up, Key::Up]);
        assert_eq!(families.cursor, KIMI, "up from the first is the last");
        families.press(Key::Down);
        assert_eq!(families.cursor, AUTO, "down from the last is the first");

        let mut versions = picker("not-listed");
        press(&mut versions, &[vec![Key::Down; CLAUDE], vec![Key::Pick, Key::Up]].concat());
        assert_eq!((versions.screen, versions.cursor), (Screen::Versions { family: CLAUDE }, 2));

        let mut models = on_models(CLAUDE, 1);
        press(&mut models, &[Key::Down, Key::Down, Key::Down]);
        assert_eq!(models.cursor, 0, "three models, three steps round");

        let mut empty = Picker::new(&[], "auto");
        assert_eq!(press(&mut empty, &[Key::Up, Key::Down, Key::Pick]), Outcome::Open, "nothing to move over");
    }

    #[test]
    fn a_family_opens_its_versions_or_its_only_versions_models() {
        let mut claude = picker("not-listed");
        press(&mut claude, &[vec![Key::Down; CLAUDE], vec![Key::Pick]].concat());
        assert_eq!((claude.screen, claude.cursor), (Screen::Versions { family: CLAUDE }, 0));

        let mut codex = picker("not-listed");
        press(&mut codex, &[Key::Down, Key::Pick]);
        assert_eq!((codex.screen, codex.cursor), (Screen::Models { family: CODEX, version: 0 }, 0), "one version: straight to its models");
    }

    #[test]
    fn a_model_is_marked_first_and_chosen_on_the_second_press() {
        let mut picker = on_models(CODEX, 0);
        picker.press(Key::Down);
        assert_eq!(picker.press(Key::Pick), Outcome::Open, "the first press chooses nothing");
        assert_eq!(picker.marked, Some(1));
        assert_eq!(picker.press(Key::Pick), Outcome::Chosen("gpt-5.3-codex-high".into()));
    }

    #[test]
    fn marking_another_model_moves_the_mark() {
        let mut picker = on_models(CODEX, 0);
        assert_eq!(press(&mut picker, &[Key::Pick, Key::Down, Key::Pick]), Outcome::Open, "a press on another model confirms none");
        assert_eq!(picker.marked, Some(1));
        assert_eq!(picker.press(Key::Pick), Outcome::Chosen("gpt-5.3-codex-high".into()));

        let mut moved = on_models(CODEX, 0);
        press(&mut moved, &[Key::Pick, Key::Down, Key::Down]);
        assert_eq!(moved.marked, Some(0), "moving the cursor leaves the mark where it was");
        assert_eq!(press(&mut moved, &[Key::Down, Key::Pick]), Outcome::Chosen("gpt-5.3-codex-low".into()));
    }

    #[test]
    fn back_leaves_one_step_and_cancels_at_the_families() {
        let mut claude = on_models(CLAUDE, 1);
        assert_eq!(press(&mut claude, &[Key::Pick, Key::Back]), Outcome::Open);
        assert_eq!((claude.screen, claude.cursor, claude.marked), (Screen::Versions { family: CLAUDE }, 1, None));
        assert_eq!(press(&mut claude, &[Key::Pick, Key::Pick]), Outcome::Open, "the mark did not survive the step back");
        assert_eq!(press(&mut claude, &[Key::Back, Key::Back]), Outcome::Open);
        assert_eq!((claude.screen, claude.cursor), (Screen::Families, CLAUDE));
        assert_eq!(claude.press(Key::Back), Outcome::Cancelled);

        let mut codex = on_models(CODEX, 0);
        assert_eq!(codex.press(Key::Back), Outcome::Open);
        assert_eq!((codex.screen, codex.cursor), (Screen::Families, CODEX), "no versions screen to go back to");
    }

    #[test]
    fn quitting_cancels_from_any_screen() {
        assert_eq!(picker("auto").press(Key::Quit), Outcome::Cancelled);
        let mut versions = picker("claude-opus-5-low");
        assert_eq!(press(&mut versions, &[Key::Pick, Key::Quit]), Outcome::Cancelled);
        let mut models = on_models(CODEX, 0);
        assert_eq!(press(&mut models, &[Key::Pick, Key::Quit]), Outcome::Cancelled, "a marked model is not chosen by leaving");
    }
}
