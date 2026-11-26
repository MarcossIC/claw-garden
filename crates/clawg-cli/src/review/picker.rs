//! The screen the reviewer's model is chosen on: the models of the Cursor CLI
//! by family, then by version, then one by one.
//!
//! Everything here is pure: how a list is grouped, what a key does and what
//! is drawn. The terminal it runs over is a trait, so a test can stand in for it.

use clawg_core::preferences::Lang;

use super::prefs::{says, ListedModel, Said};

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
    let mut families: Vec<Family> = Vec::new();
    for model in list {
        let (label, key) = (family_label(model), version_key(&model.id));
        let at = families.iter().position(|family| family.label == label).unwrap_or_else(|| {
            families.push(Family { label, versions: Vec::new() });
            families.len() - 1
        });
        // Until every model is in its version, a version goes by its key.
        let versions = &mut families[at].versions;
        match versions.iter_mut().find(|version| version.label == key) {
            Some(version) => version.models.push(model.clone()),
            None => versions.push(Version { label: key, models: vec![model.clone()] }),
        }
    }
    for version in families.iter_mut().flat_map(|family| &mut family.versions) {
        let shared = shared_words(&version.models);
        if !shared.is_empty() {
            version.label = shared;
        }
    }
    families
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

    /// The rows of the screen shown, all of them, each as wide as the widest
    /// so that the cursor's bar is even.
    fn list(&self, lang: Lang) -> Vec<Line> {
        let in_use = |place: Place| self.in_use == Some(place);
        // Each row's text, whether it is what `clawg review` uses now, and
        // whether it is the marked model.
        let rows: Vec<(String, bool, bool)> = match self.screen {
            Screen::Families => {
                let width = self.families.iter().map(|f| f.label.chars().count()).max().unwrap_or(0);
                let used = self.in_use.map(|(family, _, _)| family);
                self.families
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (format!("{:<width$}  {:>3}", f.label, f.model_count()), used == Some(i), false))
                    .collect()
            }
            Screen::Versions { family } => {
                let versions = &self.families[family].versions;
                let width = versions.iter().map(|v| v.label.chars().count()).max().unwrap_or(0);
                let used = self.in_use.filter(|(f, _, _)| *f == family).map(|(_, version, _)| version);
                versions
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("{:<width$}  {:>3}", v.label, v.models.len()), used == Some(i), false))
                    .collect()
            }
            Screen::Models { family, version } => {
                let models = &self.families[family].versions[version].models;
                let names: Vec<String> = models.iter().map(|m| m.name.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
                let name_width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
                let id_width = models.iter().map(|m| m.id.len()).max().unwrap_or(0);
                models
                    .iter()
                    .zip(&names)
                    .enumerate()
                    .map(|(i, (m, name))| {
                        let marked = self.marked == Some(i);
                        let circle = if marked { '●' } else { '○' };
                        (format!("{circle} {name:<name_width$}  {:<id_width$}", m.id), in_use((family, version, i)), marked)
                    })
                    .collect()
            }
        };
        let in_use_word = says(lang, &Said::InUse);
        let mut lines: Vec<Line> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (text, used, marked))| {
                let pointer = if i == self.cursor { "› " } else { "  " };
                let tag = if used { format!("   {in_use_word}") } else { String::new() };
                let tone = match (i == self.cursor, used || marked) {
                    (true, _) => Tone::Bright,
                    (false, true) => Tone::Plain,
                    (false, false) => Tone::Dim,
                };
                Line { text: format!("{pointer}{text}{tag}"), tone }
            })
            .collect();
        let width = lines.iter().map(|l| l.text.chars().count()).max().unwrap_or(0);
        for line in &mut lines {
            line.text = format!("{:<width$}", line.text);
        }
        lines
    }

    /// Where the screen shown is, for the line above its rows.
    fn crumb(&self, lang: Lang) -> String {
        match self.screen {
            Screen::Families => says(lang, &Said::PickFamily),
            Screen::Versions { family } => format!("{} › {}", self.families[family].label, says(lang, &Said::PickVersion)),
            Screen::Models { family, version } => {
                format!("{} › {}", self.families[family].label, self.families[family].versions[version].label)
            }
        }
    }

    /// The keys of the screen shown; on the models, also that one takes two
    /// presses.
    fn help(&self, lang: Lang) -> Vec<String> {
        match self.screen {
            Screen::Families => vec![says(lang, &Said::KeysToOpen { top: true })],
            Screen::Versions { .. } => vec![says(lang, &Said::KeysToOpen { top: false })],
            Screen::Models { .. } => vec![says(lang, &Said::KeysToMark), says(lang, &Said::TwoPresses)],
        }
    }

    /// What a terminal of `rows` by `cols` shows: never more lines than
    /// `rows`, none wider than `cols`, and the cursor's row always among them.
    pub fn frame(&self, lang: Lang, rows: usize, cols: usize) -> Vec<Line> {
        let rows = rows.max(1);
        let list = self.list(lang);
        let head = [
            Line::new(Tone::Plain, says(lang, &Said::PickerTitle)),
            Line::blank(),
            Line::new(Tone::Plain, self.crumb(lang)),
            Line::blank(),
        ];
        let foot: Vec<Line> =
            std::iter::once(Line::blank()).chain(self.help(lang).into_iter().map(|help| Line::new(Tone::Dim, format!("  {help}")))).collect();
        let space = rows.saturating_sub(head.len() + foot.len());
        let mut lines = Vec::new();
        if head.len() + list.len() + foot.len() <= rows {
            lines.extend(head);
            lines.extend(list);
            lines.extend(foot);
        } else if space >= 3 {
            // A line above the rows and one below say how many do not fit.
            let shown = window(list.len(), self.cursor, space - 2);
            let more = |above: bool, hidden: usize| match hidden {
                0 => Line::blank(),
                _ => Line::new(Tone::Dim, format!("  {}", says(lang, &Said::More { above, rows: hidden }))),
            };
            lines.extend(head);
            lines.push(more(true, shown.start));
            lines.push(more(false, list.len() - shown.end));
            lines.splice(lines.len() - 1..lines.len() - 1, list[shown].iter().cloned());
            lines.extend(foot);
        } else {
            // Too short a terminal for anything but rows.
            lines.extend(list[window(list.len(), self.cursor, rows)].iter().cloned());
        }
        // One column short of the width: a line that reaches the last column
        // makes some terminals start another.
        let width = cols.saturating_sub(1).max(1);
        for line in &mut lines {
            if line.text.chars().count() > width {
                line.text = line.text.chars().take(width).collect();
            }
        }
        lines
    }
}

/// How bright a line of the picker is drawn: the cursor's row brightest, what
/// is in use or marked next, the rest behind them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Dim,
    Plain,
    Bright,
}

/// One line of the picker's screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub tone: Tone,
}

impl Line {
    fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self { text: text.into(), tone }
    }

    fn blank() -> Self {
        Self::new(Tone::Plain, "")
    }
}

/// The rows of a list of `len` that fit in `height` lines with `cursor`
/// among them.
fn window(len: usize, cursor: usize, height: usize) -> std::ops::Range<usize> {
    let height = height.min(len);
    let start = cursor.saturating_sub(height / 2).min(len - height);
    start..start + height
}

/// The key an arrow's last byte names; the right arrow is none of the picker's.
fn arrow(last: u8) -> Option<Key> {
    match last {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'D' => Some(Key::Back),
        _ => None,
    }
}

/// The keys in `bytes`, what a terminal sent in one read. Anything that is
/// not one of the picker's keys is skipped whole, so the tail of a sequence
/// is never read as a key.
pub fn decode(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        at += 1;
        match byte {
            0x1b => match bytes.get(at) {
                // A sequence arrives whole, so an escape with nothing after
                // it is the Esc key.
                None => keys.push(Key::Back),
                Some(b'[') => {
                    let parameters = at + 1;
                    at = parameters;
                    while bytes.get(at).is_some_and(|b| (0x20..=0x3f).contains(b)) {
                        at += 1;
                    }
                    // With parameters it is another key: a page, or an arrow
                    // with Ctrl held.
                    let bare = at == parameters;
                    if let Some(&last) = bytes.get(at) {
                        at += 1;
                        keys.extend(arrow(last).filter(|_| bare));
                    }
                }
                Some(b'O') => {
                    if let Some(&last) = bytes.get(at + 1) {
                        keys.extend(arrow(last));
                    }
                    at += 2;
                }
                // Alt with a key.
                Some(_) => at += 1,
            },
            b'\r' => {
                // One Enter is one press, however the terminal ends a line:
                // read as two, it would mark a model and confirm it.
                if bytes.get(at) == Some(&b'\n') {
                    at += 1;
                }
                keys.push(Key::Pick);
            }
            b'\n' | b' ' => keys.push(Key::Pick),
            0x7f | 0x08 => keys.push(Key::Back),
            0x03 | b'q' | b'Q' => keys.push(Key::Quit),
            _ => {}
        }
    }
    keys
}

/// The terminal a picker runs over.
pub trait Terminal {
    /// Its rows and its columns.
    fn size(&self) -> (usize, usize);
    /// The keys pressed since the last read, after waiting for one.
    fn read(&mut self) -> std::io::Result<Vec<Key>>;
    /// Shows `frame` in place of what it showed.
    fn draw(&mut self, frame: &[Line]) -> std::io::Result<()>;
}

/// Shows `picker` on `terminal` and follows its keys until a model is chosen
/// or the choice is cancelled.
pub fn run(picker: &mut Picker, lang: Lang, terminal: &mut impl Terminal) -> std::io::Result<Outcome> {
    loop {
        let (rows, cols) = terminal.size();
        terminal.draw(&picker.frame(lang, rows, cols))?;
        for key in terminal.read()? {
            match picker.press(key) {
                Outcome::Open => {}
                done => return Ok(done),
            }
        }
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

    /// The one line of `frame` with `word` in it.
    fn line<'a>(frame: &'a [Line], word: &str) -> &'a Line {
        let with: Vec<&Line> = frame.iter().filter(|l| l.text.contains(word)).collect();
        assert_eq!(with.len(), 1, "{word:?} in {frame:#?}");
        with[0]
    }

    #[test]
    fn the_family_in_use_is_brighter_and_the_cursor_brightest() {
        let mut picker = picker("claude-opus-5-5-medium");
        picker.press(Key::Down);
        let frame = picker.frame(Lang::Es, 40, 100);
        let (cursor, used) = (line(&frame, "Grok"), line(&frame, "Claude"));
        assert_eq!(cursor.tone, Tone::Bright);
        assert!(cursor.text.starts_with("› Grok"), "{cursor:?}");
        assert_eq!(used.tone, Tone::Plain);
        assert!(used.text.starts_with("  Claude") && used.text.contains("en uso"), "{used:?}");
        for other in ["Auto", "Codex", "GPT", "Gemini", "Kimi"] {
            let other = line(&frame, other);
            assert_eq!(other.tone, Tone::Dim, "{other:?}");
            assert!(other.text.starts_with("  ") && !other.text.contains("en uso"), "{other:?}");
        }
        assert!(line(&frame, "Claude").text.contains("  7"), "with how many models it has: {frame:#?}");

        picker.press(Key::Up);
        let on_it = picker.frame(Lang::En, 40, 100);
        let used = line(&on_it, "Claude");
        assert_eq!(used.tone, Tone::Bright, "the cursor on the family in use");
        assert!(used.text.starts_with("› Claude") && used.text.contains("in use"), "{used:?}");

        picker.press(Key::Pick);
        let versions = picker.frame(Lang::Es, 40, 100);
        assert_eq!(line(&versions, "Claude Opus 5.5 1M").tone, Tone::Bright);
        assert!(line(&versions, "Claude Opus 5.5 1M").text.contains("en uso"));
        assert_eq!(line(&versions, "Claude Sonnet 4.6 1M").tone, Tone::Dim);
        assert!(line(&versions, "elegí una versión").text.starts_with("Claude ›"), "{versions:#?}");
    }

    #[test]
    fn each_model_has_a_circle_filled_when_marked() {
        let mut picker = on_models(CODEX, 0);
        let frame = picker.frame(Lang::Es, 40, 100);
        for (id, name) in [("gpt-5.3-codex-low", "Codex 5.3 Low"), ("gpt-5.3-codex-high", "Codex 5.3 High")] {
            let row = line(&frame, id);
            assert!(row.text.contains("○") && row.text.contains(name), "{row:?}");
        }
        assert!(line(&frame, "Codex ›").text.ends_with("Codex 5.3"), "{frame:#?}");

        press(&mut picker, &[Key::Down, Key::Pick, Key::Down]);
        let frame = picker.frame(Lang::Es, 40, 100);
        let marked = line(&frame, "gpt-5.3-codex-high");
        assert!(marked.text.starts_with("  ● Codex 5.3 High"), "{marked:?}");
        assert_eq!(marked.tone, Tone::Plain, "marked, and brighter than the rest for it");
        assert!(line(&frame, "gpt-5.3-codex-low").text.starts_with("  ○"));
        assert!(line(&frame, "gpt-5.3-codex-xhigh-fast").text.starts_with("› ○"), "the cursor is not the mark");

        let mut used = self::picker("gpt-5.3-codex-low");
        press(&mut used, &[Key::Pick, Key::Down]);
        let row = used.frame(Lang::En, 40, 100);
        let row = line(&row, "gpt-5.3-codex-low");
        assert!(row.text.starts_with("  ○") && row.text.contains("in use"), "in use is not marked: {row:?}");
        assert_eq!(row.tone, Tone::Plain);

        let mut spaced = Picker::new(&[model("grok-4.7-low", "Grok 4.7  Low"), model("grok-4.7-high", "Grok 4.7  High")], "auto");
        spaced.press(Key::Pick);
        assert!(line(&spaced.frame(Lang::En, 40, 100), "grok-4.7-high").text.contains("Grok 4.7 High"));
    }

    #[test]
    fn the_help_says_the_second_press_confirms() {
        let models = on_models(CODEX, 0);
        let es = models.frame(Lang::Es, 40, 100);
        let two = line(&es, "confirma");
        assert!(two.text.contains("marca") && two.text.contains("●") && two.text.contains("otra"), "{two:?}");
        assert!(line(&es, "Espacio").text.contains("marcar"), "{es:#?}");
        let en = models.frame(Lang::En, 40, 100);
        let two = line(&en, "confirms");
        assert!(two.text.contains("marks") && two.text.contains("●") && two.text.contains("another"), "{two:?}");
        for lang in [Lang::Es, Lang::En] {
            for help in models.help(lang).iter().chain(&picker("auto").help(lang)) {
                assert!(help.chars().count() + 2 < 80, "it fits a terminal of 80 columns: {help}");
            }
        }
        assert!(line(&picker("auto").frame(Lang::Es, 40, 100), "Espacio").text.contains("abrir"));
    }

    #[test]
    fn a_frame_fits_the_terminal_and_keeps_the_cursor_in_view() {
        let many: Vec<ListedModel> = (0..30).map(|i| model(&format!("big-{i}"), &format!("Big {i}"))).collect();
        let mut versions = Picker::new(&many, "big-0");
        press(&mut versions, &[vec![Key::Pick], vec![Key::Down; 12]].concat());
        let mut screens = vec![versions.clone(), picker("claude-opus-5-low"), on_models(CLAUDE, 1)];
        press(&mut versions, &[Key::Up; 13]);
        screens.push(versions.clone());
        for picker in &screens {
            for rows in 1..=40 {
                for cols in [1, 2, 9, 40, 80, 200] {
                    let frame = picker.frame(Lang::Es, rows, cols);
                    assert!(frame.len() <= rows, "{rows}x{cols}: {} lines", frame.len());
                    assert!(frame.iter().all(|l| l.text.chars().count() <= cols), "{rows}x{cols}: {frame:#?}");
                    let cursors = frame.iter().filter(|l| l.text.starts_with('›')).count();
                    assert_eq!(cursors, 1, "{rows}x{cols}: {frame:#?}");
                }
            }
        }

        press(&mut versions, &[Key::Down; 13]);
        let frame = versions.frame(Lang::Es, 14, 80);
        assert_eq!(frame.len(), 14);
        let says = |frame: &[Line], said: &str| frame.iter().any(|l| l.text.trim() == said);
        assert!(says(&frame, "↑ 9 más") && says(&frame, "↓ 15 más"), "{frame:#?}");
        assert!(line(&frame, "Big 12").text.starts_with("› "), "{frame:#?}");
        assert!(frame.iter().any(|l| l.text.contains("Big 9 ")) && !frame.iter().any(|l| l.text.contains("Big 8 ")), "{frame:#?}");
        assert!(says(&versions.frame(Lang::En, 14, 80), "↓ 15 more"));

        press(&mut versions, &[Key::Up; 12]);
        let top = versions.frame(Lang::Es, 14, 80);
        assert!(!top.iter().any(|l| l.text.trim().starts_with("↑ ")), "nothing is above the first: {top:#?}");
        assert!(says(&top, "↓ 24 más"), "{top:#?}");

        assert_eq!(window(30, 29, 6), 24..30);
        assert_eq!(window(3, 2, 10), 0..3);
        assert_eq!(window(0, 0, 5), 0..0);
        assert_eq!(Picker::new(&[], "auto").frame(Lang::Es, 0, 0), [], "no list and no terminal: nothing, and no panic");
    }

    #[test]
    fn keys_are_read_from_the_bytes_a_terminal_sends() {
        let one = |bytes: &[u8], key: Key| assert_eq!(decode(bytes), [key], "{bytes:?}");
        for up in [&b"\x1b[A"[..], b"\x1bOA"] {
            one(up, Key::Up);
        }
        for down in [&b"\x1b[B"[..], b"\x1bOB"] {
            one(down, Key::Down);
        }
        for pick in [&b"\r"[..], b"\n", b" "] {
            one(pick, Key::Pick);
        }
        one(b"\r\n", Key::Pick);
        for back in [&b"\x1b"[..], b"\x1b[D", b"\x1bOD", b"\x7f", b"\x08"] {
            one(back, Key::Back);
        }
        for quit in [&b"\x03"[..], b"q", b"Q"] {
            one(quit, Key::Quit);
        }

        assert_eq!(decode(b"\x1b[B\x1b[B\r"), [Key::Down, Key::Down, Key::Pick], "the keys of one read, in order");
        assert_eq!(decode(b"\r\r"), [Key::Pick, Key::Pick]);
        assert_eq!(decode(b"\x1b[5~\x1b[A"), [Key::Up], "a page key is skipped whole");

        let none: [&[u8]; 10] = [b"", b"x", "ñ".as_bytes(), b"\x1b[C", b"\x1b[5~", b"\x1b[1;5A", b"\x1bq", b"\x1b[", b"\x1bO", b"\x1bOP"];
        for other in none {
            assert_eq!(decode(other), [], "{other:?}");
        }
    }

    /// A terminal that answers each read with the next of `reads` and keeps
    /// what it was told to draw.
    struct Scripted {
        reads: std::collections::VecDeque<Vec<Key>>,
        drawn: Vec<Vec<Line>>,
    }

    impl Scripted {
        fn new(reads: &[&[Key]]) -> Self {
            Self { reads: reads.iter().map(|keys| keys.to_vec()).collect(), drawn: Vec::new() }
        }
    }

    impl Terminal for Scripted {
        fn size(&self) -> (usize, usize) {
            (10, 60)
        }

        fn read(&mut self) -> std::io::Result<Vec<Key>> {
            self.reads.pop_front().ok_or_else(|| std::io::ErrorKind::UnexpectedEof.into())
        }

        fn draw(&mut self, frame: &[Line]) -> std::io::Result<()> {
            self.drawn.push(frame.to_vec());
            Ok(())
        }
    }

    #[test]
    fn the_picker_runs_over_a_terminal_until_a_choice_or_a_cancel() {
        let mut terminal = Scripted::new(&[&[Key::Down], &[Key::Pick], &[Key::Down, Key::Pick], &[], &[Key::Pick], &[Key::Quit]]);
        let chosen = run(&mut picker("not-listed"), Lang::Es, &mut terminal).unwrap();
        assert_eq!(chosen, Outcome::Chosen("gpt-5.3-codex-high".into()));
        assert_eq!(terminal.drawn.len(), 5, "one frame before each read");
        assert_eq!(terminal.reads.len(), 1, "nothing is read after the choice");
        assert!(line(&terminal.drawn[0], "Auto").text.starts_with("› "), "{:#?}", terminal.drawn[0]);
        assert!(line(&terminal.drawn[1], "Codex").text.starts_with("› "));
        assert!(line(&terminal.drawn[3], "gpt-5.3-codex-high").text.starts_with("› ● "), "{:#?}", terminal.drawn[3]);
        assert_eq!(terminal.drawn[3], terminal.drawn[4], "a read of no key draws the same");
        assert!(terminal.drawn.iter().all(|frame| frame.len() <= 10 && frame.iter().all(|l| l.text.chars().count() < 60)));

        let mut left = Scripted::new(&[&[Key::Down, Key::Pick, Key::Pick], &[Key::Back, Key::Back, Key::Pick]]);
        assert_eq!(run(&mut picker("not-listed"), Lang::En, &mut left).unwrap(), Outcome::Cancelled, "back from the families");
        assert_eq!(left.drawn.len(), 2);

        let mut closed = Scripted::new(&[&[Key::Down]]);
        let error = run(&mut picker("not-listed"), Lang::En, &mut closed).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof, "a terminal that fails ends the picker");
    }
}
