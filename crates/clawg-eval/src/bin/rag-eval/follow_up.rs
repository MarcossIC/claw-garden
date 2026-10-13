//! `--follow-up` synthetic query arms: measures whether folding a prior
//! turn into the composed query (production's own
//! `memory::query_context::compose_query_context`, crossed through
//! `eval_api`) helps a content-free follow-up message retrieve the document
//! its original query would have found.
//!
//! Three arms per case, none of them hand-authored multi-turn golden data:
//! - **Ceiling**: the golden query, unmodified — today's normal retrieval.
//! - **Floor**: a content-free template alone — the query-independent
//!   ranking (importance/recency decay), not chance.
//! - **Treatment**: the template composed with the golden query as its one
//!   prior turn — what a real follow-up message would actually embed.

use clawg_core::eval_api::compose_query_context;

/// Content-free follow-up templates, Spanish. Deliberately carry no lexical
/// overlap with any golden-set query or document: a template that restates
/// the answer would let the floor arm cheat, which is exactly the failure
/// mode the resolvability gate exists to catch honestly instead of hiding.
pub(crate) const TEMPLATES_ES: &[&str] = &[
    "¿podés contarme más sobre eso?",
    "¿y qué más?",
    "seguí, por favor",
    "¿tenés algo más al respecto?",
];

/// English counterpart of [`TEMPLATES_ES`] — same content-free property,
/// same reasoning.
pub(crate) const TEMPLATES_EN: &[&str] = &[
    "can you tell me more about that?",
    "and what else?",
    "go on, please",
    "do you have anything more on that?",
];

/// Picks a template deterministically from `case_index`, rotating through
/// the language-appropriate list. `lang` is resolved by the caller via the
/// SAME `case_lang()`-style derivation `main.rs` already uses to attribute a
/// `RetrievalCase` to a language (join over `expected_refs` against the
/// corpus's own `lang`) — this module adds no `lang` field of its own.
/// `None`/unrecognized falls back to English, matching the majority of the
/// current golden set and keeping this function total.
pub(crate) fn template_for(lang: Option<&str>, case_index: usize) -> &'static str {
    let list = match lang {
        Some("es") => TEMPLATES_ES,
        _ => TEMPLATES_EN,
    };
    list[case_index % list.len()]
}

/// The two query texts one arm needs — same `(lexical, embedded)` shape as
/// production's `QueryTexts`, redeclared here rather than reused directly:
/// that type lives behind the private `memory` module tree, and reaching
/// into it would be exactly the widened-module-tree shortcut `eval_api.rs`'s
/// own doc comment explains this crate never takes.
pub(crate) struct ArmQuery {
    pub(crate) lexical: String,
    pub(crate) embedded: String,
}

/// The three arms scored for one retrieval case.
pub(crate) struct FollowUpArms {
    pub(crate) ceiling: ArmQuery,
    pub(crate) floor: ArmQuery,
    pub(crate) treatment: ArmQuery,
}

/// Builds the three arms for one case. `query` is the golden case's own
/// text; `template` is the content-free follow-up selected for it (see
/// [`template_for`]); `budget`/`counter` are the same budget and token
/// counter production's `compute_memory_prefix` would use for this turn.
///
/// The treatment arm's `lexical` field is the TEMPLATE, never `query` —
/// handing the lexical/FTS5 channel the golden query would let BM25 cheat,
/// and the arm would stop describing what production actually does with a
/// real follow-up (whose lexical text is always the raw message the user
/// typed, composition never touches it).
pub(crate) fn build_arms(
    query: &str,
    template: &str,
    budget: usize,
    counter: &dyn Fn(&str) -> usize,
) -> FollowUpArms {
    FollowUpArms {
        ceiling: ArmQuery { lexical: query.to_string(), embedded: query.to_string() },
        floor: ArmQuery { lexical: template.to_string(), embedded: template.to_string() },
        treatment: ArmQuery {
            lexical: template.to_string(),
            embedded: compose_query_context(template, &[query.to_string()], budget, counter),
        },
    }
}

/// Whether the ceiling/floor gap resolves the claimed recovery effect at
/// all — a precondition on the INSTRUMENT, not a claim about the treatment
/// number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Measurable,
    Void,
}

/// `gap = ceiling_r10 - floor_r10` must exceed `5.0 / n_active_cases`.
/// Derived, not invented: the 80%-recovery claim's 20% shortfall must
/// exceed one case's worth of recall (`0.2 * gap > 1 / n`), which rearranges
/// to exactly this inequality. A floor that carries real signal collapses
/// the gap and voids through the same check; `floor > ceiling` gives a
/// negative gap and voids too — both are the SAME comparison, not special
/// cases.
///
/// `n_active_cases == 0` is guarded explicitly rather than left to
/// `5.0 / 0.0` producing `+inf` (under which `gap > min_gap` would always be
/// `false` anyway, arriving at `Void` by accident of float semantics rather
/// than by a reason visible in the code).
pub(crate) fn verdict(ceiling_r10: f64, floor_r10: f64, n_active_cases: usize) -> Verdict {
    if n_active_cases == 0 {
        return Verdict::Void;
    }
    let gap = ceiling_r10 - floor_r10;
    let min_gap = 5.0 / n_active_cases as f64;
    if gap > min_gap {
        Verdict::Measurable
    } else {
        Verdict::Void
    }
}

/// Maps a verdict to an exit-code override: `Some(EXIT_GOLDEN_UNFIT)` for
/// `Void` — reusing `--audit`'s exact "well-formed and useless for this
/// claim" semantics — and `None` for `Measurable`, meaning "no override,
/// let the run's normal exit code stand."
pub(crate) fn follow_up_exit_code(verdict: Verdict) -> Option<i32> {
    match verdict {
        Verdict::Void => Some(crate::EXIT_GOLDEN_UNFIT),
        Verdict::Measurable => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_count(s: &str) -> usize {
        s.split_whitespace().count()
    }

    // --- template_for ---------------------------------------------------

    #[test]
    fn template_for_spanish_selects_from_the_spanish_list() {
        let t = template_for(Some("es"), 0);
        assert!(TEMPLATES_ES.contains(&t));
    }

    #[test]
    fn template_for_english_and_unrecognized_and_none_select_from_the_english_list() {
        assert!(TEMPLATES_EN.contains(&template_for(Some("en"), 0)));
        assert!(TEMPLATES_EN.contains(&template_for(Some("fr"), 0)));
        assert!(TEMPLATES_EN.contains(&template_for(None, 0)));
    }

    #[test]
    fn template_for_rotates_deterministically_by_case_index() {
        let a = template_for(Some("es"), 0);
        let b = template_for(Some("es"), 1);
        assert_ne!(a, b, "adjacent indices must rotate to different templates");
        // Same index, same language -> same template every time (reproducible).
        assert_eq!(template_for(Some("es"), 5), template_for(Some("es"), 5));
        // Wraps around.
        assert_eq!(
            template_for(Some("es"), 0),
            template_for(Some("es"), TEMPLATES_ES.len())
        );
    }

    // --- build_arms -------------------------------------------------------

    #[test]
    fn ceiling_arm_is_the_golden_query_on_both_channels() {
        let counter = word_count;
        let arms = build_arms("what did we decide about the auth flow", "template text", 100, &counter);
        assert_eq!(arms.ceiling.lexical, "what did we decide about the auth flow");
        assert_eq!(arms.ceiling.embedded, "what did we decide about the auth flow");
    }

    #[test]
    fn floor_arm_is_the_template_alone_on_both_channels() {
        let counter = word_count;
        let arms = build_arms("golden query", "can you tell me more about that?", 100, &counter);
        assert_eq!(arms.floor.lexical, "can you tell me more about that?");
        assert_eq!(arms.floor.embedded, "can you tell me more about that?");
    }

    /// The mutation this kills: feeding the treatment arm's lexical channel
    /// the golden query instead of the template — which would let BM25
    /// cheat and stop the arm from describing production.
    #[test]
    fn treatment_arm_sends_the_template_to_the_lexical_channel() {
        let counter = word_count;
        let query = "what did we decide about the auth flow";
        let template = "can you tell me more about that?";
        let arms = build_arms(query, template, 100, &counter);

        assert_eq!(
            arms.treatment.lexical, template,
            "the treatment arm's lexical text must be the content-free template, never the golden query"
        );
        assert_ne!(arms.treatment.lexical, query);
    }

    #[test]
    fn treatment_arm_embeds_via_the_same_compose_query_context_the_lib_exports() {
        let counter = word_count;
        let query = "what did we decide about the auth flow";
        let template = "can you tell me more about that?";
        let arms = build_arms(query, template, 100, &counter);

        let expected = compose_query_context(template, &[query.to_string()], 100, &counter);
        assert_eq!(
            arms.treatment.embedded, expected,
            "the treatment arm must build its embedded text through eval_api's compose_query_context — \
             the same function production uses — not a local reimplementation"
        );
        // And it must genuinely differ from the template alone: the query
        // was folded in as a prior turn, which the floor arm does not have.
        assert_ne!(arms.treatment.embedded, template);
    }

    // --- verdict ------------------------------------------------------------

    #[test]
    fn verdict_measurable_when_gap_exceeds_5_over_n() {
        let n = 46usize;
        let min_gap = 5.0 / n as f64;
        let ceiling = 0.6;
        let floor = ceiling - min_gap - 0.0001; // strictly more than min_gap
        assert_eq!(verdict(ceiling, floor, n), Verdict::Measurable);
    }

    #[test]
    fn verdict_void_when_gap_at_or_below_5_over_n() {
        let n = 46usize;
        let min_gap = 5.0 / n as f64;

        // EXACTLY at the boundary: gap == min_gap must be Void (the check is
        // strictly `>`, not `>=`). Subtracting `min_gap` from an arbitrary
        // ceiling and taking the difference back does NOT land on equality —
        // `0.6 - (0.6 - min_gap)` comes back one ULP BELOW `min_gap`, so a
        // fixture built that way passes under `>` and under `>=` alike and
        // pins nothing. Anchoring the floor at 0.0 makes the subtraction
        // exact (`x - 0.0 == x` for every finite x), which is the only shape
        // that actually reaches the boundary this assertion claims to test.
        assert_eq!(
            verdict(min_gap, 0.0, n),
            Verdict::Void,
            "gap exactly equal to min_gap must be Void — the comparison is strictly `>`"
        );

        // One ULP above the boundary is the other side of that same edge: it
        // must flip to Measurable, so the pair brackets the comparison from
        // both directions rather than only asserting one of them.
        assert_eq!(
            verdict(f64::from_bits(min_gap.to_bits() + 1), 0.0, n),
            Verdict::Measurable,
            "the smallest representable gap above min_gap must be Measurable"
        );

        // Floor > ceiling: negative gap.
        assert_eq!(verdict(0.3, 0.5, n), Verdict::Void, "floor > ceiling must be Void");

        // n_active_cases == 0.
        assert_eq!(verdict(1.0, 0.0, 0), Verdict::Void, "n_active_cases == 0 must be Void");
    }

    // --- follow_up_exit_code -------------------------------------------------

    #[test]
    fn follow_up_exit_code_void_is_exit_golden_unfit() {
        assert_eq!(follow_up_exit_code(Verdict::Void), Some(crate::EXIT_GOLDEN_UNFIT));
    }

    #[test]
    fn follow_up_exit_code_measurable_does_not_override() {
        assert_eq!(follow_up_exit_code(Verdict::Measurable), None);
    }
}
