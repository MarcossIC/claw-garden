//! Reads the text the MCP tools hand an agent back into content fragments.
//!
//! The formats are `clawg_cli::mcp::tools::format_search` for search and the
//! prompt builder's blocks for recall. Parsing them (rather than reusing the
//! structured hits) is what ties the measurement to the text itself: if either
//! format drifts, these parsers fail loudly instead of the metric silently
//! describing something the agent no longer receives.

/// One numbered line of a search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchLine {
    /// The 1-based position the tool printed.
    pub rank: usize,
    /// The row id the line leads with.
    pub id: i64,
    pub score: f64,
    /// The whole line, which is what it costs the agent.
    pub printed: String,
    /// Content exactly as shown, truncation marker included.
    pub content: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("search output line does not follow the hit format: {0:?}")]
    SearchLine(String),
}

/// Text the search tool prints when nothing matched.
const NO_HITS: &str = "no memories found";

/// Splits `format_search` output into its lines.
pub fn parse_search_output(text: &str) -> Result<Vec<SearchLine>, ParseError> {
    if text.trim() == NO_HITS {
        return Ok(Vec::new());
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_search_line(line).ok_or_else(|| ParseError::SearchLine(line.to_string())))
        .collect()
}

/// `{rank}. #{id} [{kind}/{type}/t{tier} score={score}] {content}`, with
/// `piece={k}/{n}` before the score for a piece of a long save. The first `]`
/// closes the metadata because nothing inside it can contain one; content is
/// free to.
fn parse_search_line(line: &str) -> Option<SearchLine> {
    let (rank, rest) = line.split_once(". #")?;
    let rank = rank.parse().ok()?;
    let (id, rest) = rest.split_once(" [")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id = id.parse().ok()?;
    let (meta, after) = rest.split_once(']')?;
    let score = meta.split_once(" score=")?.1.parse().ok()?;
    let content = match after.strip_prefix(' ') {
        Some(content) => content,
        None if after.is_empty() => after,
        None => return None,
    };
    Some(SearchLine { rank, id, score, printed: line.to_string(), content: content.to_string() })
}

/// A `recall_context` prompt cut at its `=== TITLE ===` lines, as
/// `(title, section text with its header)`.
pub fn context_sections(text: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        let title = line.strip_prefix("=== ").and_then(|l| l.strip_suffix(" ==="));
        match (title, sections.last_mut()) {
            (Some(title), _) => sections.push((title.to_string(), vec![line])),
            (None, Some((_, lines))) => lines.push(line),
            (None, None) => {}
        }
    }
    sections.into_iter().map(|(title, lines)| (title, lines.join("\n").trim_end().to_string())).collect()
}

/// Content fragments of a `recall_context` prompt, in the order they appear:
/// each hit of RECENT CONTEXT and LONG-TERM KNOWLEDGE and each workspace
/// bullet. ACTIVE CONFLICTS yields nothing, so the text a conflict carries
/// never counts as a row the agent was shown.
pub fn context_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    for (title, section) in context_sections(text) {
        if title == "WORKSPACE" {
            segments.extend(bullet_segments(&section));
        } else if title.starts_with("RECENT CONTEXT") || title == "LONG-TERM KNOWLEDGE" {
            segments.extend(hit_segments(&section));
        }
    }
    segments
}

/// A hit runs from its header to the next one: its text may hold blank
/// lines, dashes and code.
fn hit_segments(section: &str) -> Vec<String> {
    let mut segments: Vec<Vec<&str>> = Vec::new();
    for line in section.lines().skip(1) {
        match (hit_text(line), segments.last_mut()) {
            (Some(text), _) => segments.push(vec![text]),
            (None, Some(lines)) => lines.push(line),
            (None, None) => {}
        }
    }
    joined(segments)
}

/// A bullet runs until the next bullet or workspace markup.
fn bullet_segments(section: &str) -> Vec<String> {
    let mut segments: Vec<Vec<&str>> = Vec::new();
    let mut open = false;
    for line in section.lines().skip(1) {
        if let Some(bullet) = line.strip_prefix("- ") {
            segments.push(vec![bullet]);
            open = true;
        } else if line.starts_with("## ") || line.starts_with("<!-- ") {
            open = false;
        } else if let (true, Some(lines)) = (open, segments.last_mut()) {
            lines.push(line);
        }
    }
    joined(segments)
}

fn joined(segments: Vec<Vec<&str>>) -> Vec<String> {
    segments
        .into_iter()
        .map(|lines| lines.join("\n").trim_end().to_string())
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// The text after a hit header, `[YYYY-MM-DD #id type] `, where the type is
/// one word and may be absent, and a piece of a long save adds `piece={k}/{n}`.
fn hit_text(line: &str) -> Option<&str> {
    let (header, text) = line.strip_prefix('[')?.split_once("] ")?;
    let header = without_piece(header);
    let (date, rest) = header.split_once(" #")?;
    let id = rest.split_once(' ').map_or(rest, |(id, kind)| {
        if kind.is_empty() || kind.contains(' ') { "" } else { id }
    });
    let b = date.as_bytes();
    let is_date = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().chain(&b[5..7]).chain(&b[8..]).all(u8::is_ascii_digit);
    let is_id = !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit());
    (is_date && is_id).then_some(text)
}

/// `header` without a trailing ` piece={k}/{n}`, or as it came.
fn without_piece(header: &str) -> &str {
    let Some((before, piece)) = header.rsplit_once(" piece=") else {
        return header;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match piece.split_once('/') {
        Some((k, n)) if digits(k) && digits(n) => before,
        _ => header,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_result_sentence_parses_to_no_lines() {
        assert!(parse_search_output("no memories found").unwrap().is_empty());
    }

    #[test]
    fn a_hit_line_yields_its_score_and_the_line_as_printed() {
        let text = "2. #42 [Memory/chat_message/t3 score=0.87] some content";
        let line = &parse_search_output(text).unwrap()[0];
        assert_eq!(line.score, 0.87);
        assert_eq!(line.printed, text);
    }

    #[test]
    fn a_hit_line_with_a_piece_yields_its_id_score_and_content() {
        let text = "1. #58 [memory/semantic_fact/t3 piece=2/4 score=0.81] the retry budget was set";
        let line = &parse_search_output(text).unwrap()[0];
        assert_eq!((line.rank, line.id, line.score), (1, 58, 0.81));
        assert_eq!(line.content, "the retry budget was set");
    }

    #[test]
    fn a_line_without_a_score_is_an_error() {
        assert!(parse_search_output("1. #5 [Memory/chat_message/t3] content").is_err());
    }

    #[test]
    fn a_hit_line_yields_its_rank_and_content() {
        let text = "1. #42 [Memory/chat_message/t3 score=0.87] the retrieval decay term compares milliseconds";
        let line = &parse_search_output(text).unwrap()[0];
        assert_eq!(line.rank, 1);
        assert_eq!(line.content, "the retrieval decay term compares milliseconds");
    }

    #[test]
    fn several_lines_keep_their_printed_order() {
        let text = "1. #7 [Memory/code_block/t2 score=0.91] first\n2. #9 [Episodic/-/t1 score=0.50] second";
        let lines = parse_search_output(text).unwrap();
        assert_eq!(lines.iter().map(|l| (l.rank, l.content.as_str())).collect::<Vec<_>>(), vec![(1, "first"), (2, "second")]);
    }

    #[test]
    fn content_containing_brackets_is_kept_whole() {
        let text = "3. #5 [Memory/chat_message/t3 score=0.40] array[0] was [wrong] here";
        assert_eq!(parse_search_output(text).unwrap()[0].content, "array[0] was [wrong] here");
    }

    #[test]
    fn a_truncated_line_keeps_its_ellipsis_for_the_matcher() {
        let text = "1. #5 [Memory/chat_message/t3 score=0.40] cut off here…";
        assert_eq!(parse_search_output(text).unwrap()[0].content, "cut off here…");
    }

    #[test]
    fn a_line_in_another_shape_is_an_error() {
        let err = parse_search_output("1. #5 no brackets at all").unwrap_err();
        assert_eq!(err, ParseError::SearchLine("1. #5 no brackets at all".to_string()));
    }

    #[test]
    fn blank_lines_are_ignored() {
        let text = "1. #1 [Memory/chat_message/t3 score=0.40] only\n\n";
        assert_eq!(parse_search_output(text).unwrap().len(), 1);
    }

    #[test]
    fn a_prompt_splits_into_its_titled_sections_header_included() {
        let text = "=== WORKSPACE ===\n- a rule\n\n=== ACTIVE CONFLICTS (review needed) ===\nOLD: x\nNEW: y\n";
        assert_eq!(
            context_sections(text),
            vec![
                ("WORKSPACE".to_string(), "=== WORKSPACE ===\n- a rule".to_string()),
                (
                    "ACTIVE CONFLICTS (review needed)".to_string(),
                    "=== ACTIVE CONFLICTS (review needed) ===\nOLD: x\nNEW: y".to_string()
                ),
            ]
        );
    }

    #[test]
    fn the_no_context_sentence_has_no_sections() {
        assert!(context_sections("no context yet").is_empty());
    }

    #[test]
    fn a_hit_line_is_one_segment() {
        let text = "=== RECENT CONTEXT (Last 7 Days) ===\n[2026-08-10 #4 chat_message] the retrieval decay term compares milliseconds\n";
        assert_eq!(context_segments(text), vec!["the retrieval decay term compares milliseconds".to_string()]);
    }

    #[test]
    fn a_context_hit_with_a_piece_is_one_segment() {
        let text = "=== LONG-TERM KNOWLEDGE ===\n[2026-10-15 #58 semantic_fact piece=2/4] the retry budget was set\n";
        assert_eq!(context_segments(text), vec!["the retry budget was set".to_string()]);
    }

    #[test]
    fn a_hit_of_another_kind_is_one_segment() {
        let text = "=== LONG-TERM KNOWLEDGE ===\n[2026-08-10 #3 procedural/procedure] run the release script\n";
        assert_eq!(context_segments(text), vec!["run the release script".to_string()]);
    }

    #[test]
    fn a_multi_line_hit_runs_until_the_next_header() {
        let text = "=== LONG-TERM KNOWLEDGE ===\n[2026-08-10 #12 code_block] fn main() {\n    run();\n\n    stop();\n}\n[2026-08-09 #7 chat_message] second\n";
        let segments = context_segments(text);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], "fn main() {\n    run();\n\n    stop();\n}", "a blank line inside content must not end the hit");
        assert_eq!(segments[1], "second");
    }

    #[test]
    fn a_hit_ends_at_the_next_section_separator() {
        let text = "=== RECENT CONTEXT (Last 7 Days) ===\n[2026-08-10 #2 chat_message] recent row\n\n=== LONG-TERM KNOWLEDGE ===\n[2026-01-01 #1 chat_message] old row\n";
        assert_eq!(context_segments(text), vec!["recent row".to_string(), "old row".to_string()]);
    }

    #[test]
    fn a_hit_may_open_with_a_dash() {
        let text = "=== RECENT CONTEXT (Last 7 Days) ===\n[2026-08-10 #2 chat_message] notes:\n- first point\n- second point\n";
        assert_eq!(context_segments(text), vec!["notes:\n- first point\n- second point".to_string()]);
    }

    #[test]
    fn workspace_bullets_are_segments_without_their_dash() {
        let text = "=== WORKSPACE ===\n## User Preferences (always active)\n- always answer in rioplatense spanish\n## Architectural Decisions\n- use sqlite for the store\n";
        assert_eq!(
            context_segments(text),
            vec!["always answer in rioplatense spanish".to_string(), "use sqlite for the store".to_string()]
        );
    }

    #[test]
    fn workspace_markers_end_a_bullet() {
        let text = "=== WORKSPACE ===\n<!-- AUTO-START -->\n## User Preferences (always active)\n- always answer in rioplatense spanish\n<!-- AUTO-END -->\n";
        assert_eq!(context_segments(text), vec!["always answer in rioplatense spanish".to_string()]);
    }

    #[test]
    fn a_workspace_bullet_continues_over_lines_that_are_not_markup() {
        let text = "=== WORKSPACE ===\n## Architectural Decisions\n- first line of a decision\nand its second line\n- next decision\n";
        let segments = context_segments(text);
        assert_eq!(segments[0], "first line of a decision\nand its second line");
        assert_eq!(segments[1], "next decision");
    }

    #[test]
    fn conflict_text_yields_no_segments() {
        let text = "=== ACTIVE CONFLICTS (review needed) ===\nNOTE: these memories say close things and may conflict; the user has not said which holds.\nNEW [2026-08-11 #2 disputed] x\n- looks like a bullet\nOLD [2026-08-10 #1] y\n[2026-08-09 #5 chat_message] looks like a hit\n";
        assert!(context_segments(text).is_empty());
    }

    #[test]
    fn the_no_context_sentence_yields_nothing() {
        assert!(context_segments("no context yet").is_empty());
    }
}
