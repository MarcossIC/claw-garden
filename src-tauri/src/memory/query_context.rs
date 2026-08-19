//! Composes the text that gets embedded for retrieval: the current message
//! the user just sent, plus as much recent conversational context as the
//! token budget allows. Pure — no Tauri, no IO, no globals — so every case
//! below is testable with a deterministic counter and never needs the real
//! tokenizer or model.
//!
//! The identity case (`count(current) >= budget`) has no explicit `>=`
//! branch on purpose: under the admission loop below it, `>=` and `>` are
//! indistinguishable — at exact equality the remaining budget is 0, every
//! candidate turn costs at least 1 token, so nothing is ever admitted either
//! way. `saturating_sub` makes that case structural instead of a comparison
//! a mutation test could flip without changing behavior.

use std::collections::HashMap;

use crate::models::chat::{ChatMessage, Role};

/// How many of the most recent prior user turns are ever offered to the
/// composer. Lives on the selector rather than the composer so a change to
/// this bound is observable in a pure unit test over `select_prior_user_turns`
/// alone, without needing a chat-path integration test to notice it.
pub(crate) const MAX_CONTEXT_TURNS: usize = 2;

/// Separator placed between the current message and each admitted prior
/// turn. A plain newline: stored chat chunks already carry them, so this
/// stays in-distribution for the tokenizer instead of introducing a symbol
/// the corpus never contains.
const TURN_SEPARATOR: &str = "\n";

/// Token cost charged for the separator placed before each admitted turn.
/// Under SentencePiece-style tokenizers a bare whitespace separator costs 0
/// or 1 tokens, never more, so charging 1 keeps the budget an upper bound
/// rather than an estimate that could under-charge and let the embedder
/// silently right-truncate more than expected.
const SEPARATOR_TOKEN_COST: usize = 1;

/// Builds the text handed to the embedder: `current` first, then as many of
/// `prior_turns_newest_first` as fit inside `budget_tokens`, newest first.
///
/// `current` always leads and is never clipped: either the output equals
/// `current` exactly, or its token count under `count_tokens` is
/// `<= budget_tokens`. Admission walks the supplied turns in order and stops
/// at the first one that does not fit — a later, smaller turn is never
/// tried, so a rejected turn can never leave a gap for something further
/// down the list to fill.
pub(crate) fn compose_query_context(
    current: &str,
    prior_turns_newest_first: &[String],
    budget_tokens: usize,
    count_tokens: &dyn Fn(&str) -> usize,
) -> String {
    let mut remaining = budget_tokens.saturating_sub(count_tokens(current));
    let mut out = String::from(current);
    for turn in prior_turns_newest_first {
        let t = turn.trim();
        if t.is_empty() {
            continue;
        }
        let cost = SEPARATOR_TOKEN_COST + count_tokens(t);
        if cost > remaining {
            break;
        }
        remaining -= cost;
        out.push_str(TURN_SEPARATOR);
        out.push_str(t);
    }
    out
}

/// The token budget available to [`compose_query_context`]: the embedder's
/// own max window, minus the tokens it reserves for special tokens, minus
/// whatever the query prefix costs once tokenized. Subtracting both keeps
/// the result a true upper bound on what the embedder accepts after
/// tokenizing `prefix + text` together.
pub(crate) fn query_budget_tokens(max_tokens: usize, reserve: usize, prefix_tokens: usize) -> usize {
    max_tokens.saturating_sub(reserve).saturating_sub(prefix_tokens)
}

/// Walks `agent_id`'s conversation newest-first, keeping up to `max_turns`
/// prior `Role::User` messages and returning them newest-first. Skips
/// whitespace-only turns and any turn whose trimmed content equals
/// `current.trim()` — checked at every position, not just the most recent
/// one, so this catches the just-pushed message wherever it landed (including
/// the `is_retry` path, which re-sends the same content without appending a
/// new row).
pub(crate) fn select_prior_user_turns(
    conversations: &HashMap<String, Vec<ChatMessage>>,
    agent_id: &str,
    current: &str,
    max_turns: usize,
) -> Vec<String> {
    let current_trimmed = current.trim();
    let Some(history) = conversations.get(agent_id) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(max_turns);
    for msg in history.iter().rev() {
        if out.len() >= max_turns {
            break;
        }
        if msg.role != Role::User {
            continue;
        }
        let trimmed = msg.content.trim();
        if trimmed.is_empty() || trimmed == current_trimmed {
            continue;
        }
        out.push(msg.content.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: Role, content: &str) -> ChatMessage {
        ChatMessage {
            id: "id".to_string(),
            role,
            content: content.to_string(),
            tool_calls: None,
            timestamp_ms: 0,
            is_error: None,
        }
    }

    // A word-count counter: genuinely varies per string, so ordering and cost
    // mutations are independently observable (a constant counter would make
    // the separator charge and the turn charge indistinguishable).
    fn word_count(s: &str) -> usize {
        s.split_whitespace().count()
    }

    #[test]
    fn compose_with_no_prior_turns_returns_the_current_message_byte_identical() {
        let counter = word_count;
        assert_eq!(compose_query_context("hello world", &[], 100, &counter), "hello world");
        assert_eq!(
            compose_query_context(
                "hello world",
                &["   ".to_string(), "\t\n".to_string()],
                100,
                &counter
            ),
            "hello world",
            "whitespace-only prior turns must be skipped, same as an empty list"
        );
    }

    #[test]
    fn compose_puts_the_current_message_first_then_newest_to_oldest() {
        let counter = word_count;
        let newest_first = vec!["newer turn short".to_string(), "older turn text here".to_string()];
        let out = compose_query_context("current message", &newest_first, 100, &counter);
        assert_eq!(out, "current message\nnewer turn short\nolder turn text here");
    }

    #[test]
    fn a_turn_that_fits_only_when_the_separator_is_free_is_rejected() {
        let counter = word_count;
        // current "a" -> count 1; budget 3 -> remaining 2.
        // turn "b c" -> cost = 1(sep) + 2(words) = 3 > remaining(2) -> rejected.
        // A mutant that drops the separator charge would compute cost=2<=2 and admit it.
        let out = compose_query_context("a", &["b c".to_string()], 3, &counter);
        assert_eq!(out, "a", "the separator must be charged; without it this turn would wrongly fit");
    }

    #[test]
    fn admits_a_turn_costing_exactly_the_remaining_budget_and_rejects_one_costing_one_more() {
        let counter = word_count;
        // current "a" -> count 1; budget 4 -> remaining 3.
        let out_admit = compose_query_context("a", &["b c".to_string()], 4, &counter);
        assert_eq!(
            out_admit, "a\nb c",
            "a turn costing exactly the remaining budget (1 sep + 2 words = 3) must be admitted"
        );

        let out_reject = compose_query_context("a", &["b c d".to_string()], 4, &counter);
        assert_eq!(
            out_reject, "a",
            "a turn costing one more than the remaining budget (1 sep + 3 words = 4 > 3) must be rejected"
        );
    }

    #[test]
    fn admission_stops_at_the_first_turn_that_does_not_fit() {
        let counter = word_count;
        let big = "one two three four five six seven eight nine ten".to_string();
        let small = "hi".to_string();
        // current "cur" -> count 1; budget 5 -> remaining 4. big costs 1+10=11 > 4.
        let out = compose_query_context("cur", &[big, small], 5, &counter);
        assert_eq!(
            out, "cur",
            "must stop at the first non-fitting turn — never skip ahead to a smaller one that would fit"
        );
        assert!(!out.contains("one"), "no fragment of the rejected turn may leak into the output");
    }

    #[test]
    fn current_alone_at_or_over_budget_returns_current_unchanged() {
        let counter = word_count;
        // count("one two three") == 3 == budget: remaining is 0, nothing is admitted.
        let out_at = compose_query_context("one two three", &["extra turn text".to_string()], 3, &counter);
        assert_eq!(out_at, "one two three", "current exactly at budget must not admit any prior turn");

        // count("one two three four five") == 5 > budget(3).
        let out_over =
            compose_query_context("one two three four five", &["extra".to_string()], 3, &counter);
        assert_eq!(
            out_over, "one two three four five",
            "current already over budget must be returned unchanged, never truncated by the composer itself"
        );
    }

    #[test]
    fn query_budget_tokens_subtracts_both_the_reserve_and_the_prefix() {
        assert_eq!(query_budget_tokens(512, 4, 2), 506);
        assert_eq!(query_budget_tokens(10, 4, 20), 0, "must saturate, never underflow");
    }

    #[test]
    fn the_just_pushed_message_is_never_repeated_as_context() {
        let mut conversations = HashMap::new();
        conversations.insert(
            "agent-1".to_string(),
            vec![
                make_msg(Role::User, "older distinct turn"),
                make_msg(Role::Assistant, "assistant reply"),
                make_msg(Role::User, "the current message"),
            ],
        );
        let turns = select_prior_user_turns(&conversations, "agent-1", "the current message", 5);
        assert!(
            !turns.iter().any(|t| t.trim() == "the current message"),
            "the just-pushed message must never appear as its own context, regardless of position"
        );
        assert_eq!(turns, vec!["older distinct turn".to_string()]);
    }

    /// The `is_retry` shape: `stream_chat` does NOT re-push the message on a
    /// retry, so the current message is already in history from the first
    /// attempt — with the failed attempt's assistant turn sitting AFTER it.
    /// The current message is therefore not the newest row, and a skip check
    /// that only looked at the most recent position would feed the message
    /// back to itself as its own context. The test above cannot see that: its
    /// fixture puts the current message last, which is the one position an
    /// index-0-only check still covers.
    #[test]
    fn the_current_message_is_skipped_even_when_it_is_not_the_newest_turn() {
        let mut conversations = HashMap::new();
        conversations.insert(
            "agent-1".to_string(),
            vec![
                make_msg(Role::User, "older distinct turn"),
                make_msg(Role::User, "the current message"),
                make_msg(Role::Assistant, "failed attempt reply"),
            ],
        );
        let turns = select_prior_user_turns(&conversations, "agent-1", "the current message", 5);
        assert_eq!(
            turns,
            vec!["older distinct turn".to_string()],
            "the current message must be skipped at ANY position, not only the newest one"
        );
    }

    #[test]
    fn select_prior_user_turns_ignores_assistant_turns() {
        let mut conversations = HashMap::new();
        conversations.insert(
            "agent-1".to_string(),
            vec![
                make_msg(Role::User, "user turn one"),
                make_msg(Role::Assistant, "assistant turn"),
                make_msg(Role::Tool, "tool turn"),
            ],
        );
        let turns = select_prior_user_turns(&conversations, "agent-1", "current", 5);
        assert_eq!(turns, vec!["user turn one".to_string()]);
    }

    #[test]
    fn turns_from_another_agent_never_enter_the_selection() {
        let mut conversations = HashMap::new();
        conversations.insert("agent-a".to_string(), vec![make_msg(Role::User, "agent a turn")]);
        conversations.insert("agent-b".to_string(), vec![make_msg(Role::User, "agent b turn")]);
        let turns = select_prior_user_turns(&conversations, "agent-a", "current", 5);
        assert_eq!(turns, vec!["agent a turn".to_string()]);
    }

    #[test]
    fn select_prior_user_turns_takes_at_most_max_context_turns_of_more_eligible() {
        let mut conversations = HashMap::new();
        conversations.insert(
            "agent-1".to_string(),
            vec![
                make_msg(Role::User, "turn one"),
                make_msg(Role::User, "turn two"),
                make_msg(Role::User, "turn three"),
                make_msg(Role::User, "turn four"),
            ],
        );
        let turns = select_prior_user_turns(&conversations, "agent-1", "current", MAX_CONTEXT_TURNS);
        assert_eq!(
            turns,
            vec!["turn four".to_string(), "turn three".to_string()],
            "newest-first, capped at MAX_CONTEXT_TURNS ({MAX_CONTEXT_TURNS})"
        );
    }
}
