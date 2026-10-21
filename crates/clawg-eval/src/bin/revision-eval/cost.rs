//! What showing a memory's `topic_key` costs: the tokens of the key on every
//! hit of `search_memory` and `recall_context`, the fixed cost of the text
//! that teaches it, and the reference warning line.
//!
//! Production shows the key in `search_memory` only. The `recall_context`
//! prompt is rewritten here, in the tool, so nothing the product returns
//! changes. Everything in this module is pure: the I/O half hands it the text
//! the router produced.

use std::collections::HashMap;

use clawg_cli::client::Found;
use clawg_cli::mcp::tools::format_search;
use clawg_core::eval_api::EvalTokenCounter;
use clawg_core::memory::prompt_builder::DEFAULT_BUDGET;
use serde::Serialize;

use crate::chains::header_id;

/// What `save_memory` would append after its `close to earlier memories`
/// line for a save without a key that is close to an earlier one. It is
/// fixed here, before anything is measured, with a 3-digit id like a real
/// store's.
pub const WARNING_LINE: &str = "this save has no topic_key: if it revises #123, save it again under a topic_key naming their subject, so the next revision replaces it";

/// The MCP contract the model reads, and the protocol compiled into the
/// binary, as the product ships them.
const CONTRACT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../clawg-cli/src/mcp/agent_contract.json"));
const PROTOCOL: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../clawg-cli/src/mcp/protocol.md"));

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CostError {
    #[error("the agent contract is not JSON: {0}")]
    Contract(String),
    #[error("the agent contract has no description for the topic_key argument of save_memory")]
    NoKeyDescription,
    #[error("protocol.md has no paragraph that teaches the topic_key")]
    NoKeyParagraph,
}

/// A prompt after the keys were added to its hit headers.
#[derive(Debug, PartialEq)]
pub struct Rewritten {
    pub prompt: String,
    /// Lines that open a hit.
    pub headers: usize,
    /// Headers that gained a key.
    pub keyed: usize,
    /// Bytes the keys added.
    pub added_bytes: usize,
}

/// Adds ` topic=K` to the header of every hit whose row has a key in `keys`,
/// or `fallback` when it has none, where `format_hit_line` would put it:
/// before ` by ` or the closing `]`. A saved text never opens a line like a
/// header (`indent_continuations`), so nothing else is touched.
pub fn rewrite_prompt(prompt: &str, keys: &HashMap<i64, String>, fallback: Option<&str>) -> Rewritten {
    let mut out = String::with_capacity(prompt.len());
    let (mut headers, mut keyed, mut added_bytes) = (0, 0, 0);
    for line in prompt.split_inclusive('\n') {
        let Some(id) = header_id(line) else {
            out.push_str(line);
            continue;
        };
        headers += 1;
        let key = keys.get(&id).map(String::as_str).or(fallback);
        let close = line.find(']');
        let (Some(key), Some(close)) = (key, close) else {
            out.push_str(line);
            continue;
        };
        let at = line[..close].find(" by ").unwrap_or(close);
        let topic = format!(" topic={key}");
        out.push_str(&line[..at]);
        out.push_str(&topic);
        out.push_str(&line[at..]);
        keyed += 1;
        added_bytes += topic.len();
    }
    Rewritten { prompt: out, headers, keyed, added_bytes }
}

/// The `search_memory` answer for `found` without any key and with them, as
/// a bridge that saves into `project` and reads as `reader` formats it.
/// `fallback` gives a key to the hits that have none. The save project must
/// be the rows' own: a bridge hides the key of a row it could not revise.
pub fn search_pair(found: &Found, reader: &str, project: &str, fallback: Option<&str>) -> (String, String) {
    let mut with = found.clone();
    for hit in &mut with.hits {
        if hit.topic_key.is_none() {
            hit.topic_key = fallback.map(str::to_string);
        }
    }
    let mut without = found.clone();
    for hit in &mut without.hits {
        hit.topic_key = None;
    }
    (format_search(without, reader, Some(project)), format_search(with, reader, Some(project)))
}

/// What a set of outputs weighs without the keys and with them.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Cost {
    pub outputs: usize,
    pub tokens_without: usize,
    pub tokens_with: usize,
    pub total_token_delta: usize,
    pub mean_token_delta: f64,
    pub bytes_without: usize,
    pub bytes_with: usize,
    pub total_byte_delta: usize,
    /// Outputs over the byte budget before the keys, and after them. Both
    /// are 0 for an output with no budget.
    pub over_budget_without: usize,
    pub over_budget_with: usize,
}

impl Cost {
    /// Sums `(without, with)` pairs. Nothing is cut to fit `budget`: cutting
    /// is the product's job, so this only counts what would not fit.
    pub fn of<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>, counter: &EvalTokenCounter, budget: Option<usize>) -> Self {
        let mut cost = Cost::default();
        for (without, with) in pairs {
            cost.outputs += 1;
            cost.tokens_without += counter.count(without);
            cost.tokens_with += counter.count(with);
            cost.bytes_without += without.len();
            cost.bytes_with += with.len();
            if let Some(budget) = budget {
                cost.over_budget_without += usize::from(without.len() > budget);
                cost.over_budget_with += usize::from(with.len() > budget);
            }
        }
        cost.total_token_delta = cost.tokens_with.saturating_sub(cost.tokens_without);
        cost.total_byte_delta = cost.bytes_with.saturating_sub(cost.bytes_without);
        if cost.outputs > 0 {
            cost.mean_token_delta = cost.total_token_delta as f64 / cost.outputs as f64;
        }
        cost
    }
}

/// The tokens of the text that teaches the key.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FixedCost {
    /// The description of the `topic_key` argument of `save_memory`.
    pub schema_description_tokens: usize,
    /// The paragraph of `protocol.md` that opens with the key.
    pub protocol_paragraph_tokens: usize,
}

fn key_description(contract: &str) -> Result<String, CostError> {
    let contract: serde_json::Value = serde_json::from_str(contract).map_err(|e| CostError::Contract(e.to_string()))?;
    contract["tools"]["save_memory"]["inputSchema"]["properties"]["topic_key"]["description"]
        .as_str()
        .map(str::to_string)
        .ok_or(CostError::NoKeyDescription)
}

/// The bullet of `protocol.md` that names the `topic_key`, with its
/// continuation lines.
fn key_paragraph(protocol: &str) -> Result<String, CostError> {
    let mut lines = protocol.lines().skip_while(|l| !(l.starts_with("- ") && l.contains("`topic_key`")));
    let first = lines.next().ok_or(CostError::NoKeyParagraph)?;
    let rest = lines.take_while(|l| l.starts_with("  "));
    Ok(std::iter::once(first).chain(rest).collect::<Vec<_>>().join("\n"))
}

pub fn fixed_cost(contract: &str, protocol: &str, counter: &EvalTokenCounter) -> Result<FixedCost, CostError> {
    Ok(FixedCost {
        schema_description_tokens: counter.count(&key_description(contract)?),
        protocol_paragraph_tokens: counter.count(&key_paragraph(protocol)?),
    })
}

/// What one query of the keyed arm returned.
pub struct Query {
    pub found: Found,
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct M4 {
    /// `search_memory` with the key the rows have, against without any.
    pub search: Cost,
    /// The same with the longest key of the set on every hit: the upper bound
    /// for a store where every row has a key.
    pub search_all_keyed: Cost,
    /// `recall_context` rewritten with the key the rows have, against the
    /// prompt as the product returns it.
    pub context: Cost,
    pub context_all_keyed: Cost,
    pub context_budget: usize,
    pub longest_key_bytes: usize,
    pub fixed: FixedCost,
    pub warning_line: &'static str,
    pub warning_line_tokens: usize,
}

/// M4 over the keyed arm's queries. `keys` is each listed row's key, and
/// `longest_key` the one the upper bound puts on a row without one.
pub fn m4(
    queries: &[Query],
    keys: &HashMap<i64, String>,
    longest_key: &str,
    reader: &str,
    project: &str,
    counter: &EvalTokenCounter,
) -> Result<M4, CostError> {
    let search_cost = |fallback: Option<&str>| {
        let pairs: Vec<(String, String)> = queries.iter().map(|q| search_pair(&q.found, reader, project, fallback)).collect();
        Cost::of(pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())), counter, None)
    };
    let context_cost = |fallback: Option<&str>| {
        let rewritten: Vec<Rewritten> = queries.iter().map(|q| rewrite_prompt(&q.prompt, keys, fallback)).collect();
        Cost::of(queries.iter().zip(&rewritten).map(|(q, r)| (q.prompt.as_str(), r.prompt.as_str())), counter, Some(DEFAULT_BUDGET))
    };
    Ok(M4 {
        search: search_cost(None),
        search_all_keyed: search_cost(Some(longest_key)),
        context: context_cost(None),
        context_all_keyed: context_cost(Some(longest_key)),
        context_budget: DEFAULT_BUDGET,
        longest_key_bytes: longest_key.len(),
        fixed: fixed_cost(CONTRACT, PROTOCOL, counter)?,
        warning_line: WARNING_LINE,
        warning_line_tokens: counter.count(WARNING_LINE),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn keys() -> HashMap<i64, String> {
        [(7, "storage/engine".to_string()), (9, "style/indent".to_string())].into_iter().collect()
    }

    const PROMPT: &str = "=== RECENT ===\n\
        [2026-10-20 #7 architectural_decision] The engine is PostgreSQL 16.\n\
        [2026-10-20 #8 semantic_fact] A fact with no key.\n\
        [2026-10-19 #9 user_preference piece=1/2 by other] Two spaces.\n\
        [2026-10-19 #9 user_preference piece=2/2] more\n";

    #[test]
    fn the_rewrite_adds_the_key_only_to_keyed_hits() {
        let r = rewrite_prompt(PROMPT, &keys(), None);
        let lines: Vec<&str> = r.prompt.lines().collect();
        assert_eq!(lines[0], "=== RECENT ===");
        assert_eq!(lines[1], "[2026-10-20 #7 architectural_decision topic=storage/engine] The engine is PostgreSQL 16.");
        assert_eq!(lines[2], "[2026-10-20 #8 semantic_fact] A fact with no key.");
        assert_eq!(lines[3], "[2026-10-19 #9 user_preference piece=1/2 topic=style/indent by other] Two spaces.");
        assert_eq!(lines[4], "[2026-10-19 #9 user_preference piece=2/2 topic=style/indent] more");
        assert_eq!((r.headers, r.keyed), (4, 3));
    }

    #[test]
    fn the_byte_delta_is_the_sum_of_the_added_keys() {
        let r = rewrite_prompt(PROMPT, &keys(), None);
        let expected = " topic=storage/engine".len() + 2 * " topic=style/indent".len();
        assert_eq!(r.added_bytes, expected);
        assert_eq!(r.prompt.len(), PROMPT.len() + expected);

        let bound = rewrite_prompt(PROMPT, &keys(), Some("the/longest-key"));
        assert_eq!(bound.keyed, 4, "the upper bound keys the unkeyed hit too");
        assert!(bound.prompt.contains("#8 semantic_fact topic=the/longest-key]"));
        assert_eq!(bound.prompt.len(), PROMPT.len() + expected + " topic=the/longest-key".len());
    }

    #[test]
    fn a_prompt_over_budget_after_the_rewrite_is_counted() {
        let filler = "x".repeat(DEFAULT_BUDGET - 40);
        let prompt = format!("[2026-10-20 #7 semantic_fact] {filler}\n");
        assert!(prompt.len() <= DEFAULT_BUDGET);
        let r = rewrite_prompt(&prompt, &keys(), None);
        assert!(r.prompt.len() > DEFAULT_BUDGET);

        let counter = EvalTokenCounter::fake();
        let cost = Cost::of([(prompt.as_str(), r.prompt.as_str())], &counter, Some(DEFAULT_BUDGET));
        assert_eq!((cost.over_budget_without, cost.over_budget_with), (0, 1));
        assert_eq!(cost.total_byte_delta, r.added_bytes);
        let none = Cost::of([(prompt.as_str(), r.prompt.as_str())], &counter, None);
        assert_eq!((none.over_budget_without, none.over_budget_with), (0, 0));
    }

    #[test]
    fn m4_reports_the_fixed_cost_of_the_key_from_the_contract_and_the_protocol() {
        let counter = EvalTokenCounter::fake();
        let fixed = fixed_cost(CONTRACT, PROTOCOL, &counter).unwrap();
        assert!(fixed.schema_description_tokens > 10 && fixed.protocol_paragraph_tokens > 30, "{fixed:?}");

        let description = key_description(CONTRACT).unwrap();
        assert!(description.contains("A stable name"), "{description}");
        let paragraph = key_paragraph(PROTOCOL).unwrap();
        assert!(paragraph.contains("`topic_key`") && paragraph.contains("replaces"), "{paragraph}");
        assert!(!paragraph.contains("Write it so it reads on its own"), "the paragraph ends with its bullet");

        let no_description = CONTRACT.replace("\"topic_key\": {", "\"topic_key_gone\": {");
        assert_eq!(fixed_cost(&no_description, PROTOCOL, &counter), Err(CostError::NoKeyDescription));
        let no_paragraph = PROTOCOL.replace("`topic_key`", "`key`");
        assert_eq!(fixed_cost(CONTRACT, &no_paragraph, &counter), Err(CostError::NoKeyParagraph));
        assert!(matches!(fixed_cost("not json", PROTOCOL, &counter), Err(CostError::Contract(_))));
    }

    fn hit(id: i64, topic_key: Option<&str>) -> serde_json::Value {
        json!({"id": id, "kind": "Memory", "content": "The engine is PostgreSQL 16.", "memory_type": "semantic_fact",
               "tier": 1, "score": 0.9, "agent_id": "a", "topic_key": topic_key, "project_id": "p"})
    }

    #[test]
    fn the_key_of_a_search_hit_shows_for_the_save_project_of_its_row() {
        let found: Found = serde_json::from_value(json!({"hits": [hit(1, Some("storage/engine")), hit(2, None)]})).unwrap();
        let (without, with) = search_pair(&found, "a", "p", None);
        assert!(!without.contains("topic="));
        assert!(with.contains("topic=storage/engine") && with.matches("topic=").count() == 1, "{with}");
        let (_, bound) = search_pair(&found, "a", "p", Some("the/longest-key"));
        assert_eq!(bound.matches("topic=").count(), 2, "{bound}");
    }

    #[test]
    fn m4_sums_the_queries_and_prices_the_warning_line() {
        let found: Found = serde_json::from_value(json!({"hits": [hit(7, Some("storage/engine"))]})).unwrap();
        let queries = [Query { found, prompt: PROMPT.to_string() }];
        let counter = EvalTokenCounter::fake();
        let m4 = m4(&queries, &keys(), "the/longest-key", "a", "p", &counter).unwrap();
        assert_eq!(m4.search.outputs, 1);
        assert!(m4.search.total_token_delta == 1 && m4.search.total_byte_delta == " topic=storage/engine".len());
        assert!(m4.context.total_byte_delta > 0 && m4.context_all_keyed.total_byte_delta > m4.context.total_byte_delta);
        assert!(m4.search_all_keyed.total_byte_delta == m4.search.total_byte_delta, "one hit, and it has a key");
        assert_eq!(m4.warning_line_tokens, counter.count(WARNING_LINE));
        assert!(m4.warning_line_tokens > 10);
    }
}
