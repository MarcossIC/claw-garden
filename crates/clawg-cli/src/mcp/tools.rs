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

use clawg_core::memory::dispute::{Resolution, ResolvedRow};
use clawg_core::memory::ProjectIdentity;
use clawg_core::paths::Profile;

use crate::client::{Context, Health, Hit, MemoryRow, SavedText, Synthesis, Updated};
use crate::relevance::{relevant_len, RELEVANCE_RATIO};

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

/// Drops the hits too far below the best one to be worth their tokens.
///
/// Expects `hits` best first, which is the order search returns them in.
pub fn relevant_hits(mut hits: Vec<Hit>) -> Vec<Hit> {
    let scores: Vec<f64> = hits.iter().map(|h| h.score).collect();
    hits.truncate(relevant_len(&scores, RELEVANCE_RATIO));
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
            let piece = h.piece.map(|p| format!(" piece={}/{}", p.piece, p.pieces)).unwrap_or_default();
            format!(
                "{}. #{} [{}/{}/t{}{piece} score={:.2}] {}",
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

/// One line per side of a resolved dispute, the kept row first.
pub fn format_resolution(r: &Resolution) -> String {
    let line = |row: &ResolvedRow| match row.superseded_by {
        Some(by) if row.status == "superseded" => format!("#{} superseded by #{by}", row.id),
        _ => format!("#{} {}", row.id, row.status),
    };
    format!("{}\n{}", line(&r.keep), line(&r.other))
}

/// What `read_memory` tells the model: which piece it asked for and every
/// row of the save, then the whole text. A position with no row it may read
/// is named, so the model knows the text has a gap there.
pub fn format_save(save: &SavedText) -> String {
    if save.pieces <= 1 {
        return format!("#{}\n{}", save.id, save.text);
    }
    let ids = save
        .ids
        .iter()
        .map(|id| id.map_or_else(|| "(missing)".to_string(), |id| format!("#{id}")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("#{} is piece {} of {} of one save: {ids}\n{}", save.id, save.piece, save.pieces, save.text)
}

/// What `update_memory` tells the model about the row it changed. The tier is
/// the one thing the model cannot work out from its own request, and the one
/// that says whether the row is still in the workspace.
pub fn format_update(u: &Updated) -> String {
    let fields = match &u.memory_type {
        Some(t) => format!("{t}, importance {:.1}", u.importance),
        None => format!("importance {:.1}", u.importance),
    };
    format!("updated #{} {}: {fields}, tier {}", u.id, u.kind, u.tier)
}

/// Renders inventory rows as a compact one-line-per-row table.
pub fn format_rows(rows: &[MemoryRow]) -> String {
    if rows.is_empty() {
        return "no memories stored".to_string();
    }
    rows.iter()
        .map(|r| {
            format!(
                "#{} {} {} t{} imp={:.1} acc={} {}{} {}",
                r.id,
                r.kind,
                // Only `memory` rows have a type; a dash keeps every later
                // field in the same column for the other two kinds.
                r.memory_type.as_deref().unwrap_or("-"),
                r.tier,
                r.importance,
                r.access_count,
                r.status,
                sources(r),
                truncate_content(&r.summary, CONTENT_PREVIEW_LIMIT)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// ` from=#3,#7` for a conclusion, nothing for any other row.
fn sources(r: &MemoryRow) -> String {
    match &r.derived_from {
        Some(ids) if !ids.is_empty() => {
            let ids: Vec<String> = ids.iter().map(|id| format!("#{id}")).collect();
            format!(" from={}", ids.join(","))
        }
        _ => String::new(),
    }
}

/// What a conclusion is and how to save one, at the top of every
/// `synthesize` answer that has something to review. It is the only place the
/// model learns what this store means by a conclusion.
const SYNTHESIS_GUIDE: &str = "\
Look for conclusions in the memories below: a preference of the user, a convention or \
guideline of the project, a recurring error and its fix, or a trend in the decisions made. \
A conclusion generalizes two or more memories; do not restate a single one. Save each \
conclusion with `save_memory`, the memory_type that fits (user_preference, \
architectural_decision or semantic_fact), and `derived_from` set to the ids it rests on. \
If an existing conclusion is now wrong or incomplete, save the corrected one and settle the \
conflict it raises with `resolve_conflict`. If nothing generalizes yet, save nothing.";

fn synthesis_line(r: &MemoryRow) -> String {
    format!(
        "#{} {}{} {}",
        r.id,
        r.memory_type.as_deref().unwrap_or("-"),
        sources(r),
        truncate_content(&r.summary, CONTENT_PREVIEW_LIMIT)
    )
}

/// What `synthesize` hands the model: the guide, the conclusions already
/// drawn (first, so a new one updates them instead of repeating them), then
/// the memories no conclusion covers, saying how many more there are.
pub fn format_synthesis(s: &Synthesis) -> String {
    let conclusions = (!s.conclusions.is_empty()).then(|| {
        let mut lines = vec![String::new(), "Conclusions already drawn:".to_string()];
        lines.extend(s.conclusions.iter().map(synthesis_line));
        lines
    });
    // The conclusions still answer "what have you learned" when nothing is
    // left to conclude; only the guide to saving one would be noise.
    if s.pending.is_empty() {
        let mut out = vec!["no memories are waiting for a conclusion".to_string()];
        out.extend(conclusions.into_iter().flatten());
        return out.join("\n");
    }
    let mut out = vec![SYNTHESIS_GUIDE.to_string()];
    out.extend(conclusions.into_iter().flatten());
    out.push(String::new());
    out.push(format!(
        "Memories without a conclusion ({} of {}, newest first):",
        s.pending.len(),
        s.pending_total
    ));
    out.extend(s.pending.iter().map(synthesis_line));
    out.join("\n")
}

/// Unreviewed memories from which `recall_context` reminds the model to look
/// for conclusions. Below it a reminder would come with nearly every recall
/// and stop being read; far above it, the review is a long read.
pub const SYNTHESIS_NUDGE_THRESHOLD: usize = 20;

fn synthesis_nudge(unreviewed: usize) -> Option<String> {
    (unreviewed >= SYNTHESIS_NUDGE_THRESHOLD).then(|| {
        format!(
            "{unreviewed} memories here have not been reviewed for conclusions yet. When the \
             current task is done, call `synthesize`."
        )
    })
}

/// What `recall_context` hands the model: the context, then the reminder to
/// synthesize when enough memories wait for it.
pub fn format_recall(context: &Context) -> String {
    let text = format_context(&context.prompt);
    match synthesis_nudge(context.unreviewed) {
        Some(nudge) => format!("{text}\n\n{nudge}"),
        None => text,
    }
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
/// directory. `resolve` turns a path into its project; it is passed in so this
/// rule stays testable without a filesystem.
pub fn resolve_project(
    default_project: Option<&ProjectIdentity>,
    project_path: Option<&str>,
    all_projects: bool,
    resolve: impl FnOnce(&Path) -> ProjectIdentity,
) -> Option<ProjectIdentity> {
    if all_projects {
        return None;
    }
    match project_path {
        Some(p) if !p.is_empty() => Some(resolve(Path::new(p))),
        _ => default_project.cloned(),
    }
}

/// Renders `memory_status`: the daemon's health as pretty JSON, plus the two
/// facts health cannot carry — which data directory and which profile this
/// bridge is attached to.
pub fn format_status(health: &Health, profile: Profile, base_dir: &Path, agent: &str, origin: &str) -> String {
    // `to_string_pretty` on a struct that always serializes cannot fail; the
    // fallback exists so this function has no panic in it rather than because
    // the branch is reachable.
    let json = serde_json::to_string_pretty(health)
        .unwrap_or_else(|_| format!("{{\"status\":\"{}\"}}", health.status));
    format!(
        "{json}\n\nprofile: {}\ndata dir: {}\nagent: {agent} ({origin})",
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
            piece: None,
        }
    }

    fn scored(scores: &[f64]) -> Vec<Hit> {
        scores.iter().enumerate().map(|(i, &s)| hit(i as i64 + 1, "Memory", "semantic_fact", 3, s, "x")).collect()
    }

    fn ids(hits: &[Hit]) -> Vec<i64> {
        hits.iter().map(|h| h.id).collect()
    }

    #[test]
    fn relevant_hits_keeps_the_ones_the_cut_lets_through_in_order() {
        let kept = relevant_hits(scored(&[0.50, 0.40, 0.31, 0.29, 0.10]));
        assert_eq!(ids(&kept), [1, 2, 3]);
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

    #[test]
    fn format_hits_names_the_piece_of_a_long_save() {
        let mut piece = hit(58, "Memory", "semantic_fact", 3, 0.81, "the retry budget was set");
        piece.piece = Some(crate::client::PiecePosition { piece: 2, pieces: 4 });
        let out = format_hits(&[piece, hit(7, "Memory", "semantic_fact", 3, 0.5, "a short one")]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "1. #58 [Memory/semantic_fact/t3 piece=2/4 score=0.81] the retry budget was set");
        assert_eq!(lines[1], "2. #7 [Memory/semantic_fact/t3 score=0.50] a short one");
    }

    /// A newline inside a memory would split one hit across two lines and make
    /// the numbering meaningless to whoever is reading it.
    #[test]
    fn format_hits_keeps_each_hit_on_one_line() {
        let out = format_hits(&[hit(1, "Memory", "chat_message", 3, 0.1, "two\nlines")]);
        assert_eq!(out.lines().count(), 1, "out was: {out:?}");
    }

    // ── format_save ─────────────────────────────────────────────────────────

    #[test]
    fn format_save_names_the_piece_and_the_missing_ones() {
        let save = SavedText {
            kind: "memory".into(),
            id: 58,
            piece: 2,
            pieces: 4,
            ids: vec![Some(57), Some(58), None, Some(60)],
            missing: vec![3],
            text: "first and second\nfourth".into(),
        };
        assert_eq!(
            format_save(&save),
            "#58 is piece 2 of 4 of one save: #57, #58, (missing), #60\nfirst and second\nfourth"
        );
        let single = SavedText { piece: 1, pieces: 1, ids: vec![Some(58)], missing: vec![], text: "short".into(), ..save };
        assert_eq!(format_save(&single), "#58\nshort");
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

    #[test]
    fn format_update_names_the_fields_and_the_tier() {
        let memory = Updated {
            kind: "memory".into(),
            id: 12,
            importance: 8.0,
            memory_type: Some("user_preference".into()),
            tier: 1,
        };
        assert_eq!(format_update(&memory), "updated #12 memory: user_preference, importance 8.0, tier 1");
        let skill = Updated { kind: "procedural".into(), id: 3, importance: 2.0, memory_type: None, tier: 1 };
        assert_eq!(format_update(&skill), "updated #3 procedural: importance 2.0, tier 1");
    }

    #[test]
    fn rows_show_where_a_conclusion_came_from() {
        let conclusion = MemoryRow { derived_from: Some(vec![3, 7]), ..row(9, "memory", Some("user_preference"), "tabs") };
        let out = format_rows(&[conclusion, row(3, "memory", Some("semantic_fact"), "a fact")]);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains(" from=#3,#7 "), "line was: {}", lines[0]);
        assert!(!lines[1].contains("from="), "line was: {}", lines[1]);
    }

    // ── synthesis ───────────────────────────────────────────────────────────

    fn synthesis(pending: Vec<MemoryRow>, pending_total: usize, conclusions: Vec<MemoryRow>) -> Synthesis {
        Synthesis { pending, pending_total, conclusions }
    }

    #[test]
    fn the_synthesis_text_says_what_to_save() {
        let out = format_synthesis(&synthesis(
            vec![row(12, "memory", Some("semantic_fact"), "the go service uses tabs")],
            40,
            vec![MemoryRow { derived_from: Some(vec![3, 7]), ..row(9, "memory", Some("user_preference"), "tabs everywhere") }],
        ));

        for needle in ["save_memory", "derived_from", "two or more", "save nothing", "resolve_conflict"] {
            assert!(out.contains(needle), "the guide must mention {needle}: {out}");
        }
        assert!(out.contains("#9 user_preference from=#3,#7 tabs everywhere"), "{out}");
        assert!(out.contains("1 of 40"), "the agent must know the list is partial: {out}");
        assert!(out.contains("#12 semantic_fact the go service uses tabs"), "{out}");
        assert!(out.find("#9 ").unwrap() < out.find("#12 ").unwrap(), "conclusions come first: {out}");
    }

    #[test]
    fn a_synthesis_with_nothing_pending_asks_for_nothing() {
        let out = format_synthesis(&synthesis(vec![], 0, vec![]));
        assert!(!out.contains("save_memory"), "{out}");
        assert!(out.contains("no memories"), "{out}");
    }

    #[test]
    fn the_conclusions_show_even_with_nothing_pending() {
        let out = format_synthesis(&synthesis(
            vec![],
            0,
            vec![MemoryRow { derived_from: Some(vec![3, 7]), ..row(9, "memory", Some("user_preference"), "tabs everywhere") }],
        ));
        assert!(out.contains("no memories"), "{out}");
        assert!(out.contains("#9 user_preference from=#3,#7 tabs everywhere"), "what was learned stays visible: {out}");
        assert!(!out.contains("save_memory"), "with nothing pending there is nothing to save: {out}");
    }

    #[test]
    fn the_nudge_appears_from_the_threshold() {
        let context = |unreviewed| Context { prompt: "## WORKSPACE\nstuff".into(), unreviewed };

        assert_eq!(format_recall(&context(SYNTHESIS_NUDGE_THRESHOLD - 1)), "## WORKSPACE\nstuff");
        let out = format_recall(&context(SYNTHESIS_NUDGE_THRESHOLD));
        assert!(out.starts_with("## WORKSPACE\nstuff\n\n"), "the context comes first, untouched: {out}");
        assert!(out.contains("synthesize"), "{out}");
        assert!(out.contains(&SYNTHESIS_NUDGE_THRESHOLD.to_string()), "{out}");
    }

    #[test]
    fn the_nudge_follows_an_empty_context_too() {
        let out = format_recall(&Context { prompt: String::new(), unreviewed: 50 });
        assert!(out.starts_with("no context yet\n\n"), "{out}");
    }

    #[test]
    fn format_resolution_names_both_rows_and_what_replaced_one() {
        let row = |id, status: &str, superseded_by| ResolvedRow { id, status: status.into(), superseded_by };
        let out = format_resolution(&Resolution { keep: row(9, "active", None), other: row(4, "superseded", Some(9)) });
        assert_eq!(out, "#9 active\n#4 superseded by #9");
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

    // ── resolve_project ─────────────────────────────────────────────────────

    #[test]
    fn resolve_project_resolves_an_explicit_path() {
        let default = identity("proj-default");
        let resolved = resolve_project(Some(&default), Some("/some/other/project"), false, |p| {
            assert_eq!(p, Path::new("/some/other/project"));
            identity("other-1a2b")
        })
        .expect("an explicit path must resolve");
        assert_eq!(resolved.id, "other-1a2b", "an explicit path must override the default");
    }

    #[test]
    fn without_a_path_the_bridges_own_project_is_used() {
        let default = identity("proj-default");
        let resolved = resolve_project(Some(&default), Some(""), false, |_| unreachable!());
        assert_eq!(resolved, Some(default));
    }

    /// The only way to ask for the unscoped view. A caller that passed both
    /// meant the broader one — narrowing it would silently ignore the request.
    #[test]
    fn all_projects_outranks_everything() {
        let d = identity("d");
        assert_eq!(resolve_project(Some(&d), Some("/p"), true, |_| unreachable!()), None);
        assert_eq!(resolve_project(None, None, true, |_| unreachable!()), None);
    }

    #[test]
    fn resolve_project_is_none_when_there_is_nothing_to_scope_to() {
        assert_eq!(resolve_project(None, None, false, |_| unreachable!()), None);
    }

    fn identity(id: &str) -> ProjectIdentity {
        ProjectIdentity { id: id.into(), root: format!("/{id}").into(), legacy_id: id.into() }
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
        let out = format_status(&health(), Profile::Dev, Path::new("/home/u/.clawg-dev"), "a", "from --agent");
        assert!(out.contains("\"pid\": 4242"), "health must be pretty JSON: {out}");
        assert!(out.contains("loading"), "out was: {out}");
        assert!(out.contains(".clawg-dev"), "the data dir must be named: {out}");
        assert!(out.contains("dev"), "out was: {out}");
    }

    #[test]
    fn format_status_names_the_agent() {
        let out = format_status(&health(), Profile::Dev, Path::new("/d"), "claude-code", "from --agent");
        assert!(out.ends_with("agent: claude-code (from --agent)"), "out was: {out}");
    }
}
