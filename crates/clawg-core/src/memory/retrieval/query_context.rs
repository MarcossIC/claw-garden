//! Composes the text that gets embedded for retrieval: the current message
//! the user just sent, plus as much recent conversational context as the
//! token budget allows. Pure — no app runtime, no IO, no globals — so every case
//! below is testable with a deterministic counter and never needs the real
//! tokenizer or model.
//!
//! The identity case (`count(current) >= budget`) has no explicit `>=`
//! branch on purpose: under the admission loop below it, `>=` and `>` are
//! indistinguishable — at exact equality the remaining budget is 0, every
//! candidate turn costs at least 1 token, so nothing is ever admitted either
//! way. `saturating_sub` makes that case structural instead of a comparison
//! a mutation test could flip without changing behavior.

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
pub fn compose_query_context(
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
pub fn query_budget_tokens(max_tokens: usize, reserve: usize, prefix_tokens: usize) -> usize {
    max_tokens.saturating_sub(reserve).saturating_sub(prefix_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
