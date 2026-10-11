//! Reads the text the MCP tools hand an agent back into content fragments.
//!
//! The formats are `clawg_cli::mcp::tools::format_hits` for search and the
//! prompt builder's blocks for recall. Parsing them (rather than reusing the
//! structured hits) is what ties the measurement to the text itself: if either
//! format drifts, these parsers fail loudly instead of the metric silently
//! describing something the agent no longer receives.

/// One numbered line of a search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine {
    /// The 1-based position the tool printed.
    pub rank: usize,
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

/// Splits `format_hits` output into its lines.
pub fn parse_search_output(text: &str) -> Result<Vec<SearchLine>, ParseError> {
    if text.trim() == NO_HITS {
        return Ok(Vec::new());
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| parse_search_line(line).ok_or_else(|| ParseError::SearchLine(line.to_string())))
        .collect()
}

/// `{rank}. #{id} [{kind}/{type}/t{tier} score={score}] {content}`. The first
/// `]` closes the metadata because nothing inside it can contain one; content
/// is free to.
fn parse_search_line(line: &str) -> Option<SearchLine> {
    let (rank, rest) = line.split_once(". #")?;
    let rank = rank.parse().ok()?;
    let (id, rest) = rest.split_once(" [")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (_, after) = rest.split_once(']')?;
    let content = match after.strip_prefix(' ') {
        Some(content) => content,
        None if after.is_empty() => after,
        None => return None,
    };
    Some(SearchLine { rank, content: content.to_string() })
}

/// Content fragments of a `recall_context` prompt: each hit's `CONTENT:`
/// block and each workspace bullet, in the order they appear.
pub fn context_segments(text: &str) -> Vec<String> {
    enum Open {
        Nothing,
        Content,
        Bullet,
    }

    let mut segments = Vec::new();
    let mut open = Open::Nothing;
    let mut lines: Vec<&str> = Vec::new();
    let mut close = |lines: &mut Vec<&str>| {
        let joined = lines.join("
");
        lines.clear();
        let joined = joined.trim_end();
        if !joined.is_empty() {
            segments.push(joined.to_string());
        }
    };

    for line in text.lines() {
        if let Some(content) = line.strip_prefix("CONTENT: ") {
            close(&mut lines);
            open = Open::Content;
            lines.push(content);
        } else if let (Some(bullet), false) = (line.strip_prefix("- "), matches!(open, Open::Content)) {
            close(&mut lines);
            open = Open::Bullet;
            lines.push(bullet);
        } else if is_markup(line) {
            close(&mut lines);
            open = Open::Nothing;
        } else if !matches!(open, Open::Nothing) {
            lines.push(line);
        }
    }
    close(&mut lines);
    segments
}

/// Lines that end whatever block was open: section separators, workspace
/// markers and headings, and the dated header that precedes every hit and
/// conflict line.
fn is_markup(line: &str) -> bool {
    line.starts_with("=== ")
        || line.starts_with("## ")
        || line.starts_with("<!-- ")
        || starts_with_date_header(line)
}

/// `[YYYY-MM-DD] `.
fn starts_with_date_header(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() > 12
        && b[0] == b'['
        && b[5] == b'-'
        && b[8] == b'-'
        && b[11] == b']'
        && b[1..5].iter().chain(&b[6..8]).chain(&b[9..11]).all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_result_sentence_parses_to_no_lines() {
        assert_eq!(parse_search_output("no memories found"), Ok(vec![]));
    }

    #[test]
    fn a_hit_line_yields_its_rank_and_content() {
        let text = "1. #42 [Memory/chat_message/t3 score=0.87] the retrieval decay term compares milliseconds";
        assert_eq!(
            parse_search_output(text),
            Ok(vec![SearchLine {
                rank: 1,
                content: "the retrieval decay term compares milliseconds".to_string()
            }])
        );
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
    fn a_content_block_is_one_segment() {
        let text = "=== RECENT CONTEXT (Last 7 Days) ===\n[2026-08-10] SOURCE: Memory | TYPE: chat_message\nCONTENT: the retrieval decay term compares milliseconds\n";
        assert_eq!(context_segments(text), vec!["the retrieval decay term compares milliseconds".to_string()]);
    }

    #[test]
    fn a_multi_line_content_block_runs_until_the_next_header() {
        let text = "=== LONG-TERM KNOWLEDGE ===\n[2026-08-10] SOURCE: Memory | TYPE: code_block\nCONTENT: fn main() {\n    run();\n\n    stop();\n}\n[2026-08-09] SOURCE: Memory | TYPE: chat_message\nCONTENT: second\n";
        let segments = context_segments(text);
        assert_eq!(segments.len(), 2);
        assert!(segments[0].starts_with("fn main() {"));
        assert!(segments[0].contains("stop();"), "a blank line inside content must not end the block");
        assert_eq!(segments[1], "second");
    }

    #[test]
    fn a_content_block_ends_at_the_next_section_separator() {
        let text = "=== RECENT CONTEXT (Last 7 Days) ===\n[2026-08-10] SOURCE: Memory | TYPE: chat_message\nCONTENT: recent row\n\n=== LONG-TERM KNOWLEDGE ===\n[2026-01-01] SOURCE: Memory | TYPE: chat_message\nCONTENT: old row\n";
        assert_eq!(context_segments(text), vec!["recent row".to_string(), "old row".to_string()]);
    }

    #[test]
    fn workspace_bullets_are_segments_without_their_dash() {
        let text = "=== WORKSPACE ===\n<!-- AUTO-START -->\n## User Preferences (always active)\n- always answer in rioplatense spanish\n## Architectural Decisions\n- use sqlite for the store\n## Procedural Skills\n<!-- AUTO-END -->\n";
        assert_eq!(
            context_segments(text),
            vec!["always answer in rioplatense spanish".to_string(), "use sqlite for the store".to_string()]
        );
    }

    #[test]
    fn a_workspace_bullet_continues_over_lines_that_are_not_markup() {
        let text = "<!-- AUTO-START -->\n## Architectural Decisions\n- first line of a decision\nand its second line\n- next decision\n<!-- AUTO-END -->\n";
        let segments = context_segments(text);
        assert_eq!(segments[0], "first line of a decision\nand its second line");
        assert_eq!(segments[1], "next decision");
    }

    #[test]
    fn conflict_blocks_and_the_no_context_sentence_yield_nothing() {
        assert!(context_segments("no context yet").is_empty());
        let text = "=== ACTIVE CONFLICTS (review needed) ===\n[2026-08-10] OLD: \"a\" [memory:1, importance:5, tier:3]\n[2026-08-11] NEW: \"b\" [memory:2, importance:5, status:disputed]\nNOTE: estas memorias se contradicen y aún no resolviste cuál vale.\n";
        assert!(context_segments(text).is_empty());
    }
}
