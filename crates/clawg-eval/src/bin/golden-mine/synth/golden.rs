//! The golden-set shape `golden-mine synth` reads and writes. It mirrors
//! `rag-eval`'s schema on purpose instead of sharing it (the two are separate
//! binaries) and rejects unknown keys, so a field `rag-eval` gains later makes
//! reading the base fail loudly rather than being dropped from the output.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Golden {
    pub corpus: Vec<Row>,
    pub retrieval_cases: Vec<Case>,
    pub contradiction_pairs: Vec<Pair>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub query: String,
    pub corpus_refs: Vec<String>,
    pub expected_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub subset: Subset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Active,
    KnownRed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subset {
    #[default]
    Natural,
    Lexical,
    MultiRef,
    HighImportance,
    LowImportance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub a: String,
    pub b: String,
    pub label: PairLabel,
    pub lang: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairLabel {
    Contradiction,
    NoContradiction,
    Identical,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "corpus": [{"ref": "c1", "content": "x", "memory_type": "chat_message", "importance": 5.0}],
        "retrieval_cases": [{"query": "q", "corpus_refs": ["c1"], "expected_refs": ["c1"]}],
        "contradiction_pairs": []
    }"#;

    #[test]
    fn defaults_are_omitted_when_serializing() {
        let golden: Golden = serde_json::from_str(MINIMAL).unwrap();
        let json = serde_json::to_value(&golden).unwrap();
        let case = json["retrieval_cases"][0].as_object().unwrap();
        for key in ["status", "blocked_by", "history", "subset"] {
            assert!(!case.contains_key(key), "{key} must be omitted at its default");
        }
        assert!(!json["corpus"][0].as_object().unwrap().contains_key("lang"));
    }

    #[test]
    fn a_non_default_subset_is_written() {
        let mut golden: Golden = serde_json::from_str(MINIMAL).unwrap();
        golden.retrieval_cases[0].subset = Subset::MultiRef;
        let json = serde_json::to_value(&golden).unwrap();
        assert_eq!(json["retrieval_cases"][0]["subset"], "multi_ref");
    }

    #[test]
    fn the_low_importance_subset_round_trips_by_its_snake_case_name() {
        let mut golden: Golden = serde_json::from_str(MINIMAL).unwrap();
        golden.retrieval_cases[0].subset = Subset::LowImportance;
        let text = serde_json::to_string(&golden).unwrap();
        assert!(text.contains("\"subset\":\"low_importance\""));
        let back: Golden = serde_json::from_str(&text).unwrap();
        assert_eq!(back.retrieval_cases[0].subset, Subset::LowImportance);
    }

    #[test]
    fn an_unknown_key_is_an_error_instead_of_being_dropped() {
        let raw = MINIMAL.replace("\"importance\": 5.0", "\"importance\": 5.0, \"tier\": 1");
        assert!(serde_json::from_str::<Golden>(&raw).is_err());
    }
}
