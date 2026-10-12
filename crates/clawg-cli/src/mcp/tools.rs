//! Pure rendering for the MCP tools' text output.
//!
//! Every function here takes data and returns a `String`. That is deliberate:
//! an MCP tool's return value is read by a language model, so its shape is part
//! of the product — and the only way to keep it under test without a protocol
//! round-trip is to keep the formatting free of I/O.
//!
//! The house style for every list: one line per row, the id first (because the
//! follow-up tool call needs it), then the metadata that decides whether the
//! row is worth reading, then the content.

use std::path::Path;

use clawg_core::paths::Profile;

use crate::client::{Health, Hit, MemoryRow};

/// How much of a hit's content a search result shows.
///
/// A retrieval answer is a menu, not the meal: the model picks an id and can
/// ask for more. Ten full memories would crowd out the conversation that
/// prompted the search.
pub const CONTENT_PREVIEW_LIMIT: usize = 500;

/// The `memory_type` values the store understands, in the order a caller is
/// most likely to want them. Rendered into the tool schema so the model picks
/// from the set instead of inventing a string the query side will never match.
pub const MEMORY_TYPE_VALUES: &str = "semantic_fact | user_preference | \
     architectural_decision | session_summary | code_block | chat_message | tool_error";

/// Shortens `s` to at most `max` characters, marking that it was cut.
///
/// Counts characters, not bytes: slicing a UTF-8 string at a byte offset panics
/// mid-codepoint, and the content here is arbitrary user text.
pub fn truncate_content(s: &str, max: usize) -> String {
    // Newlines are flattened here rather than at each call site: every renderer
    // in this module is line-oriented, and one embedded newline would break the
    // numbering it took the caller a line to establish.
    let flat: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();

    if flat.chars().count() <= max {
        return flat;
    }
    let kept: String = flat.chars().take(max).collect();
    format!("{kept}…")
}

/// Share of the best hit's score another hit needs to be shown.
///
/// Relative, not absolute: measured on the golden set, the score of the
/// expected memory and of the rest overlap (0.15–0.51 against up to 0.47), so
/// no fixed floor separates them, while a clear winner can still shorten the
/// list for free.
pub const RELEVANCE_RATIO: f64 = 0.6;

/// Hits shown regardless of score.
pub const MIN_SHOWN_HITS: usize = 2;

/// Drops the hits too far below the best one to be worth their tokens.
///
/// Expects `hits` best first, which is the order search returns them in.
pub fn relevant_hits(mut hits: Vec<Hit>) -> Vec<Hit> {
    let Some(best) = hits.first().map(|h| h.score) else {
        return hits;
    };
    let shown = hits
        .iter()
        .enumerate()
        .take_while(|(i, h)| *i < MIN_SHOWN_HITS || h.score >= RELEVANCE_RATIO * best)
        .count();
    hits.truncate(shown);
    hits
}

/// What `search_memory` hands the model for a search's hits.
pub fn format_search(hits: Vec<Hit>) -> String {
    format_hits(&relevant_hits(hits))
}

/// Renders search hits as a numbered list, or says plainly that there were none.
fn format_hits(hits: &[Hit]) -> String {
    if hits.is_empty() {
        // A sentence, not an empty string: a model reading `""` cannot tell a
        // successful empty search from a broken tool.
        return "no memories found".to_string();
    }
    hits.iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "{}. #{} [{}/{}/t{} score={:.2}] {}",
                i + 1,
                h.id,
                h.kind,
                h.memory_type,
                h.tier,
                h.score,
                truncate_content(&h.content, CONTENT_PREVIEW_LIMIT)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Renders inventory rows as a compact one-line-per-row table.
pub fn format_rows(rows: &[MemoryRow]) -> String {
    if rows.is_empty() {
        return "no memories stored".to_string();
    }
    rows.iter()
        .map(|r| {
            format!(
                "#{} {} {} t{} imp={:.1} acc={} {} {}",
                r.id,
                r.kind,
                // Only `memory` rows have a type; a dash keeps every later
                // field in the same column for the other two kinds.
                r.memory_type.as_deref().unwrap_or("-"),
                r.tier,
                r.importance,
                r.access_count,
                r.status,
                truncate_content(&r.summary, CONTENT_PREVIEW_LIMIT)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Passes a non-empty memory prompt through; substitutes a plain sentence when
/// there is nothing yet.
///
/// An empty string is a legitimate answer from the daemon (a fresh install has
/// no memories), but handing `""` to a model reads as a malfunction.
pub fn format_context(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        "no context yet".to_string()
    } else {
        prompt.to_string()
    }
}

/// Decides which project a tool call is scoped to.
///
/// Precedence, and why: `all_projects` wins because it is the only way to ask
/// for the unscoped view and a caller that passed both meant the broader one;
/// then an explicit `project_path`, which is the caller naming a project other
/// than where the bridge happens to be running; then the bridge's own working
/// directory.
pub fn resolve_project_id(
    default_project_id: Option<&str>,
    project_path: Option<&str>,
    all_projects: bool,
) -> Option<String> {
    if all_projects {
        return None;
    }
    match project_path {
        // `derive_project_id` and nothing else: the store scoped every row with
        // it, so any other spelling of "which project" simply never matches.
        Some(p) if !p.is_empty() => Some(clawg_core::memory::derive_project_id(Path::new(p))),
        _ => default_project_id.map(str::to_owned),
    }
}

/// Renders `memory_status`: the daemon's health as pretty JSON, plus the two
/// facts health cannot carry — which data directory and which profile this
/// bridge is attached to.
pub fn format_status(health: &Health, profile: Profile, base_dir: &Path) -> String {
    // `to_string_pretty` on a struct that always serializes cannot fail; the
    // fallback exists so this function has no panic in it rather than because
    // the branch is reachable.
    let json = serde_json::to_string_pretty(health)
        .unwrap_or_else(|_| format!("{{\"status\":\"{}\"}}", health.status));
    format!(
        "{json}\n\nprofile: {}\ndata dir: {}",
        profile.as_str(),
        base_dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: i64, kind: &str, memory_type: &str, tier: i64, score: f64, content: &str) -> Hit {
        Hit {
            id,
            kind: kind.into(),
            content: content.into(),
            memory_type: memory_type.into(),
            tier,
            score,
        }
    }

    fn scored(scores: &[f64]) -> Vec<Hit> {
        scores.iter().enumerate().map(|(i, &s)| hit(i as i64 + 1, "Memory", "semantic_fact", 3, s, "x")).collect()
    }

    fn ids(hits: &[Hit]) -> Vec<i64> {
        hits.iter().map(|h| h.id).collect()
    }

    #[test]
    fn hits_far_below_the_best_match_are_not_shown() {
        let kept = relevant_hits(scored(&[0.50, 0.40, 0.31, 0.29, 0.10]));
        assert_eq!(ids(&kept), [1, 2, 3]);
    }

    #[test]
    fn a_hit_exactly_at_the_ratio_is_shown() {
        let kept = relevant_hits(scored(&[0.50, 0.40, 0.30]));
        assert_eq!(ids(&kept), [1, 2, 3]);
    }

    /// When every score is low the best one is no clear winner, and the runner
    /// up is as likely to be the answer: measured on the golden set, a query
    /// whose answer scored 0.04 under a best of 0.10.
    #[test]
    fn the_two_best_are_shown_whatever_they_score() {
        let kept = relevant_hits(scored(&[0.10, 0.04, 0.01]));
        assert_eq!(ids(&kept), [1, 2]);
    }

    #[test]
    fn the_search_output_shows_only_the_relevant_hits() {
        let text = format_search(scored(&[0.50, 0.10, 0.05]));
        assert!(text.contains("#1 ") && text.contains("#2 "), "{text}");
        assert!(!text.contains("#3 "), "{text}");
    }

    #[test]
    fn no_hits_stay_no_hits() {
        assert!(relevant_hits(Vec::new()).is_empty());
    }

    fn row(id: i64, kind: &str, memory_type: Option<&str>, summary: &str) -> MemoryRow {
        MemoryRow {
            id,
            kind: kind.into(),
            memory_type: memory_type.map(str::to_owned),
            summary: summary.into(),
            tier: 2,
            importance: 5.0,
            status: "active".into(),
            access_count: 3,
            ..Default::default()
        }
    }

    // ── truncate_content ────────────────────────────────────────────────────

    #[test]
    fn truncate_leaves_short_text_alone() {
        assert_eq!(truncate_content("hello", 10), "hello");
        assert_eq!(truncate_content("hello", 5), "hello", "exactly at the limit is not cut");
    }

    #[test]
    fn truncate_marks_text_it_cut() {
        let out = truncate_content("abcdefghij", 4);
        assert!(out.starts_with("abcd"), "out was: {out}");
        assert!(out.len() > 4, "a cut must be visible, not silent: {out}");
        assert!(out.ends_with('…'), "out was: {out}");
    }

    /// Slicing arbitrary user text at a byte offset panics mid-codepoint. The
    /// content this renders is arbitrary user text.
    #[test]
    fn truncate_does_not_split_a_multibyte_character() {
        let out = truncate_content("ñañañaña", 3);
        assert!(out.starts_with("ñañ"), "out was: {out}");
    }

    #[test]
    fn truncate_to_zero_is_not_a_panic() {
        assert_eq!(truncate_content("abc", 0), "…");
    }

    // ── format_hits ─────────────────────────────────────────────────────────

    #[test]
    fn format_hits_says_so_when_there_are_none() {
        assert_eq!(format_hits(&[]), "no memories found");
    }

    #[test]
    fn format_hits_numbers_rows_and_leads_with_the_id() {
        let out = format_hits(&[
            hit(42, "Memory", "semantic_fact", 2, 0.876, "first"),
            hit(7, "Procedural", "procedural_skill", 1, 0.5, "second"),
        ]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("1. #42 "), "line was: {}", lines[0]);
        assert!(lines[1].starts_with("2. #7 "), "line was: {}", lines[1]);
    }

    #[test]
    fn format_hits_carries_kind_type_tier_and_score() {
        let out = format_hits(&[hit(42, "Memory", "semantic_fact", 2, 0.876, "content here")]);
        assert!(out.contains("Memory/semantic_fact/t2"), "out was: {out}");
        assert!(out.contains("score=0.88"), "score must be readable, not raw f64: {out}");
        assert!(out.contains("content here"), "out was: {out}");
    }

    #[test]
    fn format_hits_truncates_long_content() {
        let long = "x".repeat(CONTENT_PREVIEW_LIMIT + 100);
        let out = format_hits(&[hit(1, "Memory", "chat_message", 3, 0.1, &long)]);
        assert!(out.ends_with('…'), "out ended: {:?}", &out[out.len().saturating_sub(20)..]);
        assert!(out.len() < long.len(), "the preview must be shorter than the content");
    }

    /// A newline inside a memory would split one hit across two lines and make
    /// the numbering meaningless to whoever is reading it.
    #[test]
    fn format_hits_keeps_each_hit_on_one_line() {
        let out = format_hits(&[hit(1, "Memory", "chat_message", 3, 0.1, "two\nlines")]);
        assert_eq!(out.lines().count(), 1, "out was: {out:?}");
    }

    // ── format_rows ─────────────────────────────────────────────────────────

    #[test]
    fn format_rows_says_so_when_there_are_none() {
        assert_eq!(format_rows(&[]), "no memories stored");
    }

    #[test]
    fn format_rows_renders_one_line_per_row_with_the_id_first() {
        let out = format_rows(&[
            row(12, "memory", Some("semantic_fact"), "a fact"),
            row(3, "episodic", None, "a trajectory"),
        ]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("#12 "), "line was: {}", lines[0]);
        assert!(lines[0].contains("memory"), "line was: {}", lines[0]);
        assert!(lines[0].contains("semantic_fact"), "line was: {}", lines[0]);
        assert!(lines[0].contains("a fact"), "line was: {}", lines[0]);
        assert!(lines[1].starts_with("#3 "), "line was: {}", lines[1]);
    }

    /// Only `memory` rows have a type; the column still has to line up for the
    /// other two kinds rather than shifting every field after it.
    #[test]
    fn format_rows_renders_a_missing_memory_type_as_a_dash() {
        let out = format_rows(&[row(3, "episodic", None, "a trajectory")]);
        assert!(out.contains(" - "), "out was: {out}");
    }

    #[test]
    fn format_rows_truncates_and_keeps_one_line_per_row() {
        let long = "y".repeat(CONTENT_PREVIEW_LIMIT + 50);
        let out = format_rows(&[row(1, "memory", Some("chat_message"), &format!("{long}\nmore"))]);
        assert_eq!(out.lines().count(), 1);
        assert!(out.ends_with('…'));
    }

    // ── format_context ──────────────────────────────────────────────────────

    #[test]
    fn format_context_passes_a_real_prompt_through_untouched() {
        assert_eq!(format_context("## WORKSPACE\nstuff"), "## WORKSPACE\nstuff");
    }

    #[test]
    fn format_context_substitutes_a_sentence_for_an_empty_prompt() {
        assert_eq!(format_context(""), "no context yet");
        assert_eq!(format_context("   \n "), "no context yet");
    }

    // ── resolve_project_id ──────────────────────────────────────────────────

    #[test]
    fn resolve_project_id_falls_back_to_the_bridges_own_project() {
        assert_eq!(
            resolve_project_id(Some("proj-default"), None, false),
            Some("proj-default".to_string())
        );
    }

    #[test]
    fn resolve_project_id_derives_an_explicit_path() {
        let derived = resolve_project_id(Some("proj-default"), Some("/some/other/project"), false)
            .expect("an explicit path must resolve");
        assert_ne!(derived, "proj-default", "an explicit path must override the default");
        assert_eq!(
            derived,
            clawg_core::memory::derive_project_id(Path::new("/some/other/project")),
            "the bridge must derive ids exactly as the store does, or nothing matches"
        );
    }

    /// The only way to ask for the unscoped view. A caller that passed both
    /// meant the broader one — narrowing it would silently ignore the request.
    #[test]
    fn all_projects_outranks_everything() {
        assert_eq!(resolve_project_id(Some("d"), Some("/p"), true), None);
        assert_eq!(resolve_project_id(None, None, true), None);
    }

    #[test]
    fn resolve_project_id_is_none_when_there_is_nothing_to_scope_to() {
        assert_eq!(resolve_project_id(None, None, false), None);
    }

    // ── format_status ───────────────────────────────────────────────────────

    fn health() -> Health {
        Health {
            status: "ok".into(),
            version: "0.1.0".into(),
            profile: "dev".into(),
            pid: 4242,
            embedder: "loading".into(),
            embedder_error: None,
            embedder_retryable: None,
            uptime_s: 12,
        }
    }

    #[test]
    fn format_status_includes_the_health_json_the_data_dir_and_the_profile() {
        let out = format_status(&health(), Profile::Dev, Path::new("/home/u/.clawg-dev"));
        assert!(out.contains("\"pid\": 4242"), "health must be pretty JSON: {out}");
        assert!(out.contains("loading"), "out was: {out}");
        assert!(out.contains(".clawg-dev"), "the data dir must be named: {out}");
        assert!(out.contains("dev"), "out was: {out}");
    }
}
