//! The chain set `revision-eval` measures, `data/revision-chains.json`: each
//! chain is one topic saved in two or three versions, and the set must reach
//! its minimums before anything is measured.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// The types a chain may be saved as: the ones the detector pairs, so a
/// transition without a key has a chance to be marked.
const CHAIN_TYPES: [&str; 3] = ["architectural_decision", "user_preference", "semantic_fact"];

/// A version of more words than this holds more than the 480 tokens a save
/// keeps in one piece (`CHUNK_MAX_TOKENS` in clawg-core), whatever the
/// tokenizer: every word is at least one token.
pub const LONG_VERSION_WORDS: usize = 480;

/// How a version differs from the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// Another value in the same wording.
    Value,
    /// Another value in other words.
    Rewrite,
    /// The same value with more scope or a reason added.
    Extend,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub content: String,
    /// The text that tells this version from the others of its chain, as it
    /// appears in `content`.
    pub value: String,
    /// None only for the first version.
    pub change: Option<Change>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chain {
    pub id: String,
    pub lang: String,
    /// Kept as the string the save is made with, as the golden set does.
    pub memory_type: String,
    pub topic_key: String,
    pub query: String,
    pub versions: Vec<Version>,
}

impl Chain {
    fn is_long(&self) -> bool {
        self.versions.iter().any(|v| v.content.split_whitespace().count() > LONG_VERSION_WORDS)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetFile {
    version: u32,
    chains: Vec<Chain>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChainSet {
    pub version: u32,
    /// Of the file with LF line endings, so a result names the exact set it
    /// measured whether the checkout turned them into CRLF or not.
    pub sha256: String,
    pub chains: Vec<Chain>,
}

impl ChainSet {
    /// Steps from one version to the next, over every chain.
    pub fn transitions(&self) -> usize {
        self.chains.iter().map(|c| c.versions.len() - 1).sum()
    }
}

/// Parses a chain set, checks each chain's shape and then the set's minimums.
pub fn parse_set(text: &str) -> Result<ChainSet, String> {
    let file: SetFile = serde_json::from_str(text).map_err(|e| format!("not a chain set: {e}"))?;
    if file.version != 1 {
        return Err(format!("chain set version {} is not 1", file.version));
    }
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for chain in &file.chains {
        check_chain(chain).map_err(|e| format!("chain {}: {e}", chain.id))?;
        if !ids.insert(chain.id.as_str()) {
            return Err(format!("id {} is used twice", chain.id));
        }
        // One agent and project save the whole set, so a shared key would
        // make one chain revise another.
        if !keys.insert(chain.topic_key.as_str()) {
            return Err(format!("topic_key {} is used twice", chain.topic_key));
        }
    }
    check_minimums(&file.chains)?;
    // A JSON string holds its line breaks escaped, so a CRLF can only sit
    // between tokens and changes nothing the set says.
    let sha256 = clawg_core::eval_api::sha256_hex(text.replace("\r\n", "\n").as_bytes());
    Ok(ChainSet { version: file.version, sha256, chains: file.chains })
}

fn check_chain(chain: &Chain) -> Result<(), String> {
    if chain.id.trim().is_empty() {
        return Err("id is empty".into());
    }
    if !["es", "en"].contains(&chain.lang.as_str()) {
        return Err(format!("lang {} is neither es nor en", chain.lang));
    }
    if !CHAIN_TYPES.contains(&chain.memory_type.as_str()) {
        return Err(format!("memory_type {} is not one of {}", chain.memory_type, CHAIN_TYPES.join(", ")));
    }
    clawg_core::memory::topic::validate_topic_key(&chain.topic_key)
        .map_err(|e| format!("topic_key {}: {e}", chain.topic_key))?;
    if chain.query.trim().is_empty() {
        return Err("query is empty".into());
    }
    if !(2..=3).contains(&chain.versions.len()) {
        return Err(format!("{} versions, a chain has 2 or 3 versions", chain.versions.len()));
    }
    let query = chain.query.to_lowercase();
    for (i, version) in chain.versions.iter().enumerate() {
        let n = i + 1;
        match (i, version.change) {
            (0, Some(_)) => return Err("the first version names a change".into()),
            (1.., None) => return Err(format!("version {n} names no change")),
            _ => {}
        }
        if version.content.trim().is_empty() || version.value.trim().is_empty() {
            return Err(format!("version {n} has an empty content or value"));
        }
        if !version.content.contains(&version.value) {
            return Err(format!("the value of version {n} is not in its content"));
        }
        let value = version.value.to_lowercase();
        if chain.versions[..i].iter().any(|earlier| earlier.content.to_lowercase().contains(&value)) {
            return Err(format!("the value of version {n} is already in an earlier version"));
        }
        // A query that holds a value would rank its version first by that
        // word alone.
        if query.contains(&value) {
            return Err(format!("the query holds the value of version {n}"));
        }
    }
    Ok(())
}

/// Each count the set must reach for its rates to mean something: 56
/// transitions give a rate of 0.5 a Wilson interval of about ±0.13.
fn check_minimums(chains: &[Chain]) -> Result<(), String> {
    let count = |keep: &dyn Fn(&Chain) -> bool| chains.iter().filter(|c| keep(c)).count();
    let minimums = [
        ("chains", chains.len(), 40),
        ("es chains", count(&|c| c.lang == "es"), 20),
        ("en chains", count(&|c| c.lang == "en"), 20),
        ("two-version chains", count(&|c| c.versions.len() == 2), 24),
        ("three-version chains", count(&|c| c.versions.len() == 3), 16),
        ("architectural_decision chains", count(&|c| c.memory_type == "architectural_decision"), 20),
        ("user_preference chains", count(&|c| c.memory_type == "user_preference"), 10),
        ("semantic_fact chains", count(&|c| c.memory_type == "semantic_fact"), 10),
        ("long chains", count(&Chain::is_long), 8),
    ];
    let missing: Vec<String> = minimums
        .iter()
        .filter(|(_, n, minimum)| n < minimum)
        .map(|(name, n, minimum)| format!("{n} {name}, fewer than {minimum}"))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::{json, Value};

    /// A set with exactly the minimums: 40 chains, the first 20 in Spanish,
    /// 24 of two versions and 16 of three, 20 decisions, 10 preferences and 10
    /// facts, and the first 8 long.
    fn minimal_set() -> Value {
        let chains: Vec<Value> = (0..40)
            .map(|i| {
                let memory_type = match i % 4 {
                    0 | 1 => "architectural_decision",
                    2 => "user_preference",
                    _ => "semantic_fact",
                };
                let n = if i % 5 < 3 { 2 } else { 3 };
                let versions: Vec<Value> = (0..n)
                    .map(|v| {
                        let value = format!("value{i}{}", ["a", "b", "c"][v]);
                        let padding = if i < 8 && v == 0 { " word".repeat(LONG_VERSION_WORDS) } else { String::new() };
                        let mut version = json!({"content": format!("topic {i} is {value}{padding}"), "value": value});
                        if v > 0 {
                            version["change"] = json!(["value", "rewrite", "extend"][(i + v) % 3]);
                        }
                        version
                    })
                    .collect();
                json!({
                    "id": format!("c{i}"),
                    "lang": if i < 20 { "es" } else { "en" },
                    "memory_type": memory_type,
                    "topic_key": format!("topic/c{i}"),
                    "query": format!("what about topic {i}"),
                    "versions": versions,
                })
            })
            .collect();
        json!({"version": 1, "chains": chains})
    }

    fn parse(set: &Value) -> Result<ChainSet, String> {
        parse_set(&set.to_string())
    }

    #[test]
    fn the_shipped_chain_set_meets_every_minimum() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/revision-chains.json")).unwrap();
        let set = parse_set(&raw).unwrap();
        assert_eq!(set.version, 1);
        assert_eq!(set.sha256, clawg_core::eval_api::sha256_hex(raw.replace("\r\n", "\n").as_bytes()));
        assert_eq!(set.transitions(), 56);
    }

    #[test]
    fn the_set_hash_is_the_same_with_either_line_ending() {
        let lf = serde_json::to_string_pretty(&minimal_set()).unwrap();
        let crlf = lf.replace('\n', "\r\n");
        let hash = parse_set(&lf).unwrap().sha256;
        assert_eq!(parse_set(&crlf).unwrap().sha256, hash);
        assert_eq!(hash, clawg_core::eval_api::sha256_hex(lf.as_bytes()));
    }

    #[test]
    fn a_set_at_its_minimums_parses() {
        let set = parse(&minimal_set()).unwrap();
        assert_eq!(set.chains.len(), 40);
        assert_eq!(set.transitions(), 56);
    }

    #[test]
    fn a_set_below_a_minimum_names_what_is_missing() {
        let mut short = minimal_set();
        short["chains"].as_array_mut().unwrap().pop();
        let err = parse(&short).unwrap_err();
        assert!(err.contains("39 chains, fewer than 40"), "{err}");

        let mut few_long = minimal_set();
        few_long["chains"][0]["versions"][0]["content"] = json!("topic 0 is value0a");
        let err = parse(&few_long).unwrap_err();
        assert!(err.contains("7 long chains, fewer than 8"), "{err}");

        let mut no_preference = minimal_set();
        for chain in no_preference["chains"].as_array_mut().unwrap() {
            if chain["memory_type"] == "user_preference" {
                chain["memory_type"] = json!("semantic_fact");
            }
        }
        let err = parse(&no_preference).unwrap_err();
        assert!(err.contains("0 user_preference chains, fewer than 10"), "{err}");
    }

    #[test]
    fn a_query_that_holds_a_versions_value_is_refused() {
        let mut set = minimal_set();
        set["chains"][5]["query"] = json!("what is value5b");
        let err = parse(&set).unwrap_err();
        assert!(err.contains("c5"), "{err}");

        set["chains"][5]["query"] = json!("What is VALUE5B");
        let err = parse(&set).unwrap_err();
        assert!(err.contains("c5"), "{err}");
    }

    #[test]
    fn a_chain_with_an_invalid_key_or_unknown_field_fails() {
        let mut bad_key = minimal_set();
        bad_key["chains"][3]["topic_key"] = json!("Topic Three");
        let err = parse(&bad_key).unwrap_err();
        assert!(err.contains("c3"), "{err}");

        let mut chain_field = minimal_set();
        chain_field["chains"][3]["note"] = json!("not a field");
        assert!(parse(&chain_field).is_err());

        let mut version_field = minimal_set();
        version_field["chains"][3]["versions"][1]["note"] = json!("not a field");
        assert!(parse(&version_field).is_err());

        let mut set_field = minimal_set();
        set_field["description"] = json!("not a field");
        assert!(parse(&set_field).is_err());
    }

    #[test]
    fn a_chain_out_of_shape_is_refused() {
        let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
            ("2 or 3 versions", Box::new(|s: &mut Value| {
                s["chains"][0]["versions"].as_array_mut().unwrap().truncate(1);
            })),
            ("2 or 3 versions", Box::new(|s: &mut Value| {
                let fourth = json!({"content": "topic 3 is value3d", "value": "value3d", "change": "value"});
                s["chains"][3]["versions"].as_array_mut().unwrap().push(fourth);
            })),
            ("first version", Box::new(|s: &mut Value| s["chains"][0]["versions"][0]["change"] = json!("value"))),
            ("no change", Box::new(|s: &mut Value| {
                s["chains"][0]["versions"][1].as_object_mut().unwrap().remove("change");
            })),
            ("not in its content", Box::new(|s: &mut Value| s["chains"][0]["versions"][1]["value"] = json!("elsewhere"))),
            ("an earlier version", Box::new(|s: &mut Value| {
                s["chains"][0]["versions"][0]["content"] = json!("topic 0 is value0a or value0b");
            })),
            ("an earlier version", Box::new(|s: &mut Value| s["chains"][0]["versions"][1]["value"] = json!("topic 0 is"))),
            ("empty", Box::new(|s: &mut Value| s["chains"][0]["query"] = json!("  "))),
            ("lang", Box::new(|s: &mut Value| s["chains"][0]["lang"] = json!("pt"))),
            ("memory_type", Box::new(|s: &mut Value| s["chains"][0]["memory_type"] = json!("session_summary"))),
            ("id c1", Box::new(|s: &mut Value| s["chains"][0]["id"] = json!("c1"))),
            ("topic_key topic/c1", Box::new(|s: &mut Value| s["chains"][0]["topic_key"] = json!("topic/c1"))),
        ];
        for (expected, mutate) in cases {
            let mut set = minimal_set();
            mutate(&mut set);
            let err = parse(&set).unwrap_err();
            assert!(err.contains(expected), "{expected}: {err}");
        }
        assert!(parse(&json!({"version": 2, "chains": []})).unwrap_err().contains("version 2"));
    }
}
