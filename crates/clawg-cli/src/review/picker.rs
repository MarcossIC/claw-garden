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
}
