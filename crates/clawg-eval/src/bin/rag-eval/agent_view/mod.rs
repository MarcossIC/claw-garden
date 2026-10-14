//! `--agent-view`: measures retrieval as an agent receives it, not as the
//! store ranks it.
//!
//! The corpus goes in through the real ingest pipeline, and every query is
//! answered by the daemon's own router and rendered by the bridge's own
//! formatters. What is scored is the text that would reach the model: whether
//! a golden row is visible in it, how many tokens it costs and how long it
//! takes.

mod matcher;
mod parse;
pub mod runner;
mod stats;

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::golden::CaseStatus;
use matcher::{visible_segment, Corpus};
use parse::{context_sections, context_segments, parse_search_output, ParseError};
use stats::{summarize, Sample, ToolStats};

/// What the search tool showed for one case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchOutcome {
    /// An expected row was visible in the results.
    pub found: bool,
    pub found_refs: Vec<String>,
    /// Printed position of the first line carrying an expected row.
    pub rank: Option<usize>,
    pub tokens: usize,
    pub latency_ms: f64,
    /// A line carrying an expected row was cut at the preview limit, so the
    /// agent saw only the start of it.
    pub truncated: bool,
    /// Every printed line, in order. Results written before lines were
    /// recorded have none.
    #[serde(default)]
    pub lines: Vec<LineView>,
}

/// One printed search line, reduced to what a cut-off rule needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineView {
    pub score: f64,
    pub tokens: usize,
    /// The line carries a row the case expects.
    pub expected: bool,
}

/// What `recall_context` showed for one case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextOutcome {
    pub found: bool,
    pub found_refs: Vec<String>,
    pub tokens: usize,
    pub latency_ms: f64,
    /// Tokens of each titled section. Results written before sections were
    /// recorded have none.
    #[serde(default)]
    pub sections: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseView {
    pub query: String,
    pub expected_refs: Vec<String>,
    pub status: CaseStatus,
    /// Expected rows whose content another corpus row repeats verbatim, so a
    /// match on them cannot be attributed to them alone.
    pub ambiguous_refs: Vec<String>,
    pub search: SearchOutcome,
    pub context: ContextOutcome,
}

/// One tool's summary, split the way `retrieval` and `blocked` are: `known_red`
/// cases expect tier-1 rows the search index structurally cannot return, so
/// folding them in would drown the number that can move.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ToolSummary {
    pub active: ToolStats,
    /// Left out of the JSON when the golden set has no such cases, so a file
    /// never carries a block of zeros that reads like a measurement.
    #[serde(default, skip_serializing_if = "ToolStats::is_empty")]
    pub known_red: ToolStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentViewResults {
    pub corpus_size: usize,
    /// Rows in the store after ingest. Differs from `corpus_size` when the
    /// chunker splits a long row or the dedup gate merges two identical ones.
    pub stored_rows: u64,
    pub min_segment_chars: usize,
    pub ambiguous_refs: Vec<String>,
    /// Corpus rows too short for any fragment of them to qualify as a match;
    /// expecting one of them can only ever read as a miss.
    pub unmatchable_refs: Vec<String>,
    pub search: ToolSummary,
    pub context: ToolSummary,
    pub per_case: Vec<CaseView>,
}

/// Which expected rows a search output shows, where, and whether the agent got
/// all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRead {
    pub found_refs: Vec<String>,
    pub rank: Option<usize>,
    pub truncated: bool,
    pub lines: Vec<LineView>,
}

/// `count_tokens` prices one printed line.
pub fn read_search(
    text: &str,
    count_tokens: &dyn Fn(&str) -> usize,
    expected: &[String],
    corpus: &Corpus,
) -> Result<SearchRead, ParseError> {
    let mut found: HashSet<&str> = HashSet::new();
    let mut rank = None;
    let mut truncated = false;
    let mut lines = Vec::new();
    for line in parse_search_output(text)? {
        let shown = visible_segment(&line.content);
        let carried: Vec<&str> = corpus
            .refs_containing(&shown.text)
            .into_iter()
            .filter(|r| expected.iter().any(|e| e == r))
            .collect();
        lines.push(LineView {
            score: line.score,
            tokens: count_tokens(&line.printed),
            expected: !carried.is_empty(),
        });
        if carried.is_empty() {
            continue;
        }
        rank.get_or_insert(line.rank);
        truncated |= shown.truncated;
        found.extend(carried);
    }
    Ok(SearchRead { found_refs: in_expected_order(expected, &found), rank, truncated, lines })
}

/// Expected rows a `recall_context` output shows.
pub fn read_context(text: &str, expected: &[String], corpus: &Corpus) -> Vec<String> {
    let found: HashSet<&str> = context_segments(text)
        .iter()
        .flat_map(|segment| corpus.refs_containing(&visible_segment(segment).text))
        .collect();
    in_expected_order(expected, &found)
}

/// What each section of a `recall_context` prompt costs.
pub fn context_section_tokens(
    text: &str,
    count_tokens: &dyn Fn(&str) -> usize,
) -> BTreeMap<String, usize> {
    context_sections(text).into_iter().map(|(title, section)| (title, count_tokens(&section))).collect()
}

fn in_expected_order(expected: &[String], found: &HashSet<&str>) -> Vec<String> {
    expected.iter().filter(|e| found.contains(e.as_str())).cloned().collect()
}

/// Human-readable run summary, one block per tool.
pub fn render_summary(results: &AgentViewResults) -> String {
    let mut out = format!(
        "agent view: {} rows stored for a corpus of {}
",
        results.stored_rows, results.corpus_size
    );
    if !results.unmatchable_refs.is_empty() {
        out.push_str(&format!(
            "  {} corpus row(s) shorter than {} chars can never be found: {}
",
            results.unmatchable_refs.len(),
            results.min_segment_chars,
            results.unmatchable_refs.join(", ")
        ));
    }
    if !results.ambiguous_refs.is_empty() {
        out.push_str(&format!(
            "  {} corpus row(s) repeat another row's content, so a match on them is ambiguous: {}
",
            results.ambiguous_refs.len(),
            results.ambiguous_refs.join(", ")
        ));
    }
    for (tool, summary) in [("search", &results.search), ("context", &results.context)] {
        out.push_str(&tool_line(tool, "active", &summary.active));
        if summary.known_red.n_cases > 0 {
            out.push_str(&tool_line(tool, "known_red", &summary.known_red));
        }
    }
    out
}

fn tool_line(tool: &str, group: &str, stats: &ToolStats) -> String {
    format!(
        "  {tool} ({group}, n={}): hit rate {:.4}, tokens mean {:.1} max {}, latency p50 {:.2} ms p99 {:.2} ms
",
        stats.n_cases,
        stats.hit_rate,
        stats.mean_tokens,
        stats.max_tokens,
        stats.latency_p50_ms,
        stats.latency_p99_ms
    )
}

/// Builds the run summary from the finished per-case readings.
pub fn build_results(
    corpus_size: usize,
    stored_rows: u64,
    corpus: &Corpus,
    per_case: Vec<CaseView>,
) -> AgentViewResults {
    let summary = |status: CaseStatus, sample: fn(&CaseView) -> Sample| {
        let samples: Vec<Sample> =
            per_case.iter().filter(|c| c.status == status).map(sample).collect();
        summarize(&samples)
    };
    let search_sample = |c: &CaseView| Sample {
        found: c.search.found,
        tokens: c.search.tokens,
        latency_ms: c.search.latency_ms,
    };
    let context_sample = |c: &CaseView| Sample {
        found: c.context.found,
        tokens: c.context.tokens,
        latency_ms: c.context.latency_ms,
    };
    AgentViewResults {
        corpus_size,
        stored_rows,
        min_segment_chars: matcher::MIN_SEGMENT_CHARS,
        ambiguous_refs: corpus.ambiguous_refs().into_iter().collect(),
        unmatchable_refs: corpus.unmatchable_refs().into_iter().map(str::to_string).collect(),
        search: ToolSummary {
            active: summary(CaseStatus::Active, search_sample),
            known_red: summary(CaseStatus::KnownRed, search_sample),
        },
        context: ToolSummary {
            active: summary(CaseStatus::Active, context_sample),
            known_red: summary(CaseStatus::KnownRed, context_sample),
        },
        per_case,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "the retrieval decay term compares milliseconds against seconds";
    const B: &str = "el indice knn mide distancia l2 cruda y no similitud coseno";

    fn corpus() -> Corpus {
        Corpus::new([("c1", A), ("c2", B), ("c3", "short row")])
    }

    fn refs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn hit(rank: usize, content: &str) -> String {
        format!("{rank}. #{rank} [Memory/chat_message/t3 score=0.50] {content}")
    }

    fn no_tokens(_: &str) -> usize {
        0
    }

    /// Enough to replay any cut-off rule over a finished run: what each line
    /// scored, what it cost, and whether the case needed it.
    #[test]
    fn read_search_records_each_line_score_tokens_and_whether_it_was_expected() {
        let text = [
            format!("1. #1 [Memory/chat_message/t3 score=0.91] {B}"),
            format!("2. #2 [Memory/chat_message/t3 score=0.40] {A}"),
        ]
        .join("\n");
        let read = read_search(&text, &|line: &str| line.len(), &refs(&["c1"]), &corpus()).unwrap();
        assert_eq!(
            read.lines,
            vec![
                LineView { score: 0.91, tokens: text.lines().next().unwrap().len(), expected: false },
                LineView { score: 0.40, tokens: text.lines().nth(1).unwrap().len(), expected: true },
            ]
        );
    }

    #[test]
    fn context_section_tokens_are_counted_per_section_title() {
        let text = "=== WORKSPACE ===\n- a rule\n\n=== LONG-TERM KNOWLEDGE ===\n[2026-01-01 #1 chat_message] x\n";
        let tokens = context_section_tokens(text, &|s: &str| s.len());
        assert_eq!(tokens.get("WORKSPACE"), Some(&"=== WORKSPACE ===\n- a rule".len()));
        assert_eq!(tokens.get("LONG-TERM KNOWLEDGE"), Some(&"=== LONG-TERM KNOWLEDGE ===\n[2026-01-01 #1 chat_message] x".len()));
    }

    #[test]
    fn a_context_written_before_sections_were_recorded_still_parses() {
        let json = r#"{"found":true,"found_refs":["c1"],"tokens":10,"latency_ms":1.0}"#;
        let outcome: ContextOutcome = serde_json::from_str(json).unwrap();
        assert!(outcome.sections.is_empty());
    }

    #[test]
    fn a_result_written_before_lines_were_recorded_still_parses() {
        let json = r#"{"found":true,"found_refs":["c1"],"rank":1,"tokens":10,"latency_ms":1.0,"truncated":false}"#;
        let outcome: SearchOutcome = serde_json::from_str(json).unwrap();
        assert!(outcome.lines.is_empty());
    }

    #[test]
    fn read_search_reports_the_rank_of_the_first_expected_row() {
        let text = [hit(1, B), hit(2, A)].join("\n");
        let read = read_search(&text, &no_tokens, &refs(&["c1"]), &corpus()).unwrap();
        assert_eq!((read.found_refs, read.rank, read.truncated), (refs(&["c1"]), Some(2), false));
    }

    #[test]
    fn read_search_ignores_rows_the_case_does_not_expect() {
        let text = hit(1, B);
        let read = read_search(&text, &no_tokens, &refs(&["c1"]), &corpus()).unwrap();
        assert_eq!((read.found_refs, read.rank, read.truncated), (vec![], None, false));
    }

    #[test]
    fn read_search_lists_every_expected_row_it_finds_in_expected_order() {
        let text = [hit(1, B), hit(2, A)].join("\n");
        let read = read_search(&text, &no_tokens, &refs(&["c1", "c2"]), &corpus()).unwrap();
        assert_eq!(read.found_refs, refs(&["c1", "c2"]));
        assert_eq!(read.rank, Some(1));
    }

    #[test]
    fn read_search_flags_a_matching_line_cut_at_the_preview_limit() {
        let long_row = format!("{A} {}", "tail ".repeat(200));
        let corpus = Corpus::new([("c1", long_row.as_str())]);
        let preview: String = long_row.chars().take(500).collect();
        let text = hit(1, &format!("{preview}…"));
        let read = read_search(&text, &no_tokens, &refs(&["c1"]), &corpus).unwrap();
        assert_eq!(read.found_refs, refs(&["c1"]));
        assert!(read.truncated);
    }

    #[test]
    fn read_search_does_not_flag_a_cut_line_that_matched_nothing_expected() {
        let text = hit(1, &format!("{B} and then a long tail that got cut off by the preview…"));
        let read = read_search(&text, &no_tokens, &refs(&["c1"]), &corpus()).unwrap();
        assert!(!read.truncated);
    }

    #[test]
    fn read_search_of_the_empty_sentence_finds_nothing() {
        let read = read_search("no memories found", &no_tokens, &refs(&["c1"]), &corpus()).unwrap();
        assert_eq!((read.found_refs, read.rank, read.truncated), (vec![], None, false));
    }

    #[test]
    fn read_search_surfaces_a_format_it_cannot_parse() {
        assert!(read_search("garbage", &no_tokens, &refs(&["c1"]), &corpus()).is_err());
    }

    #[test]
    fn read_context_finds_rows_in_hit_lines_and_workspace_bullets() {
        let text = format!(
            "=== WORKSPACE ===\n## User Preferences (always active)\n- {A}\n\n=== LONG-TERM KNOWLEDGE ===\n[2026-01-01 #8 chat_message] {B}\n"
        );
        assert_eq!(read_context(&text, &refs(&["c1", "c2"]), &corpus()), refs(&["c1", "c2"]));
        assert_eq!(read_context(&text, &refs(&["c2"]), &corpus()), refs(&["c2"]));
    }

    #[test]
    fn read_context_finds_nothing_in_the_no_context_sentence() {
        assert!(read_context("no context yet", &refs(&["c1"]), &corpus()).is_empty());
    }

    fn case_view(status: CaseStatus, search_found: bool, context_found: bool, tokens: usize) -> CaseView {
        CaseView {
            query: "q".to_string(),
            expected_refs: refs(&["c1"]),
            status,
            ambiguous_refs: vec![],
            search: SearchOutcome {
                found: search_found,
                found_refs: vec![],
                rank: None,
                tokens,
                latency_ms: 1.0,
                truncated: false,
                lines: vec![],
            },
            context: ContextOutcome {
                found: context_found,
                found_refs: vec![],
                tokens: tokens * 2,
                latency_ms: 2.0,
                sections: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn build_results_keeps_known_red_cases_out_of_the_active_summary() {
        let results = build_results(
            3,
            4,
            &corpus(),
            vec![
                case_view(CaseStatus::Active, true, true, 100),
                case_view(CaseStatus::Active, false, true, 200),
                case_view(CaseStatus::KnownRed, false, true, 50),
            ],
        );
        assert_eq!(results.search.active.n_cases, 2);
        assert_eq!(results.search.active.hit_rate, 0.5);
        assert_eq!(results.search.known_red.n_cases, 1);
        assert_eq!(results.search.known_red.hit_rate, 0.0);
        assert_eq!(results.context.active.hit_rate, 1.0);
        assert_eq!(results.context.active.max_tokens, 400);
        assert_eq!(results.context.known_red.hit_rate, 1.0);
    }

    #[test]
    fn build_results_records_store_size_against_corpus_size_and_matcher_limits() {
        let results = build_results(3, 5, &corpus(), vec![]);
        assert_eq!((results.corpus_size, results.stored_rows), (3, 5));
        assert_eq!(results.min_segment_chars, matcher::MIN_SEGMENT_CHARS);
        assert_eq!(results.unmatchable_refs, refs(&["c3"]));
        assert!(results.ambiguous_refs.is_empty());
        assert_eq!(results.search, ToolSummary::default());
    }

    #[test]
    fn build_results_lists_corpus_rows_with_identical_content() {
        let twin = Corpus::new([("c1", A), ("c2", A)]);
        assert_eq!(build_results(2, 1, &twin, vec![]).ambiguous_refs, refs(&["c1", "c2"]));
    }

    #[test]
    fn the_summary_names_both_tools_and_both_case_groups() {
        let results = build_results(
            3,
            4,
            &corpus(),
            vec![
                case_view(CaseStatus::Active, true, true, 100),
                case_view(CaseStatus::Active, false, true, 200),
                case_view(CaseStatus::KnownRed, false, true, 50),
            ],
        );
        let text = render_summary(&results);
        assert!(text.contains("4 rows stored for a corpus of 3"), "{text}");
        assert!(text.contains("search (active, n=2): hit rate 0.5000"), "{text}");
        assert!(text.contains("tokens mean 150.0 max 200"), "{text}");
        assert!(text.contains("context (active, n=2): hit rate 1.0000"), "{text}");
        assert!(text.contains("search (known_red, n=1): hit rate 0.0000"), "{text}");
        assert!(text.contains("latency p50 1.00 ms p99 1.00 ms"), "{text}");
    }

    #[test]
    fn the_summary_names_the_rows_that_can_never_be_found() {
        let text = render_summary(&build_results(3, 3, &corpus(), vec![]));
        assert!(text.contains("1 corpus row(s) shorter than 40 chars"), "{text}");
    }

    #[test]
    fn a_group_with_no_cases_is_left_out_of_the_json() {
        let only_active =
            build_results(3, 3, &corpus(), vec![case_view(CaseStatus::Active, true, true, 10)]);
        let json = serde_json::to_value(&only_active).unwrap();
        assert!(json["search"].get("active").is_some());
        assert!(json["search"].get("known_red").is_none(), "{}", json["search"]);
        assert!(json["context"].get("known_red").is_none(), "{}", json["context"]);
        let back: AgentViewResults = serde_json::from_value(json).unwrap();
        assert_eq!(back, only_active, "a file without the group must read back as an empty one");

        let with_blocked = build_results(
            3,
            3,
            &corpus(),
            vec![case_view(CaseStatus::KnownRed, false, true, 10)],
        );
        let json = serde_json::to_value(&with_blocked).unwrap();
        assert!(json["search"].get("known_red").is_some());
    }

    #[test]
    fn results_survive_a_json_round_trip() {
        let results = build_results(3, 3, &corpus(), vec![case_view(CaseStatus::Active, true, false, 10)]);
        let json = serde_json::to_string(&results).unwrap();
        assert_eq!(serde_json::from_str::<AgentViewResults>(&json).unwrap(), results);
    }
}
