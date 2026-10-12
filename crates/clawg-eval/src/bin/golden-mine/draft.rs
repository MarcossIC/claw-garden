//! Input and output shapes for the mining pass: the candidate pool produced by
//! the engram extractor on the way in, and the golden-set draft on the way out.
//!
//! The draft is deliberately NOT a valid `golden.json`. Its cases carry no
//! queries — those need semantic judgement the mining pass cannot supply — and
//! it carries the expected document's text alongside each case so whoever
//! writes those queries can see what they are writing about. Emitting
//! something the harness would happily load, with empty strings where the
//! queries belong, would let a half-finished set be measured and quoted as if
//! it were real.

use serde::{Deserialize, Serialize};

/// One row as the extractor emits it. `src_id` and `src_project` are carried
/// through the whole pipeline so any document in a committed golden set can be
/// traced back to the observation it came from.
#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub src_id: serde_json::Value,
    pub src_project: Option<String>,
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    pub lang: String,
}

/// A corpus row in the emitted draft — the same field set `golden.json` uses,
/// plus provenance.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DraftRow {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    pub lang: String,
    pub src_id: serde_json::Value,
    pub src_project: Option<String>,
}

/// A retrieval case with everything except the query.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DraftCase {
    /// Always empty on emit. The field exists so the shape of the finished
    /// case is visible in the draft rather than something a later step has to
    /// remember to add.
    pub query: String,
    pub corpus_refs: Vec<String>,
    pub expected_refs: Vec<String>,
    /// The expected document's text, for whoever writes the query. Stripped
    /// when the draft is finalised into `golden.json`, which rejects unknown
    /// fields.
    pub expected_content: String,
    /// Cosine similarity between the expected document and its strongest
    /// distractor. Close to 1.0 means this case is genuinely hard.
    pub hardest_distractor_cosine: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Draft {
    pub corpus: Vec<DraftRow>,
    pub draft_cases: Vec<DraftCase>,
}

/// Stable, zero-padded ref for a corpus position: `c01`, `c02`, … Padding
/// keeps refs sorting lexicographically in the same order they were emitted,
/// which is what makes a diff between two generated drafts readable.
pub fn ref_for(index: usize) -> String {
    format!("c{:02}", index + 1)
}

/// Assembles the draft from the selected corpus and the mined cases.
///
/// `selected` holds indices into `pool`; a case's `expected` and `distractors`
/// are positions within `selected`, not within `pool`. Keeping those two index
/// spaces distinct is the one thing that can go quietly wrong here, so the
/// translation happens exactly once, in this function.
pub fn build(
    pool: &[Candidate],
    selected: &[usize],
    cases: &[crate::mine::MinedCase],
    hardest: &[f64],
) -> Draft {
    let corpus: Vec<DraftRow> = selected
        .iter()
        .enumerate()
        .map(|(position, &pool_index)| {
            let c = &pool[pool_index];
            DraftRow {
                ref_: ref_for(position),
                content: c.content.clone(),
                memory_type: c.memory_type.clone(),
                importance: c.importance,
                lang: c.lang.clone(),
                src_id: c.src_id.clone(),
                src_project: c.src_project.clone(),
            }
        })
        .collect();

    let draft_cases = cases
        .iter()
        .zip(hardest.iter().chain(std::iter::repeat(&0.0)))
        .map(|(case, &hardest_cos)| {
            // The expected document leads its own pool: `expected_refs` must be
            // a subset of `corpus_refs` or the harness rejects the file.
            let mut corpus_refs = vec![ref_for(case.expected)];
            corpus_refs.extend(case.distractors.iter().map(|&d| ref_for(d)));
            DraftCase {
                query: String::new(),
                corpus_refs,
                expected_refs: vec![ref_for(case.expected)],
                expected_content: corpus[case.expected].content.clone(),
                hardest_distractor_cosine: hardest_cos,
            }
        })
        .collect();

    Draft { corpus, draft_cases }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mine::MinedCase;

    fn candidate(content: &str, memory_type: &str) -> Candidate {
        Candidate {
            src_id: serde_json::json!(1),
            src_project: Some("p".into()),
            content: content.into(),
            memory_type: memory_type.into(),
            importance: 5.0,
            lang: "en".into(),
        }
    }

    #[test]
    fn refs_are_zero_padded_from_one() {
        assert_eq!(ref_for(0), "c01");
        assert_eq!(ref_for(9), "c10");
    }

    #[test]
    fn build_maps_pool_indices_into_corpus_positions() {
        // Selecting out of order is the case that catches an index-space mixup:
        // pool index 2 becomes corpus position 0, so a case expecting position
        // 0 must render as c01 carrying "third".
        let pool = vec![
            candidate("first", "a"),
            candidate("second", "a"),
            candidate("third", "a"),
        ];
        let draft = build(
            &pool,
            &[2, 0],
            &[MinedCase { expected: 0, distractors: vec![1] }],
            &[0.9],
        );
        assert_eq!(draft.corpus[0].ref_, "c01");
        assert_eq!(draft.corpus[0].content, "third");
        assert_eq!(draft.corpus[1].content, "first");
        assert_eq!(draft.draft_cases[0].expected_refs, vec!["c01"]);
        assert_eq!(draft.draft_cases[0].expected_content, "third");
    }

    #[test]
    fn expected_ref_is_always_inside_corpus_refs() {
        // The harness aborts the load with E_EXPECTED_NOT_IN_POOL otherwise, so
        // a draft that violated this would be unusable at the last step.
        let pool: Vec<Candidate> = (0..5).map(|i| candidate(&format!("d{i}"), "a")).collect();
        let draft = build(
            &pool,
            &[0, 1, 2, 3, 4],
            &[MinedCase { expected: 3, distractors: vec![1, 4] }],
            &[0.8],
        );
        let case = &draft.draft_cases[0];
        for expected in &case.expected_refs {
            assert!(case.corpus_refs.contains(expected));
        }
    }

    #[test]
    fn queries_are_emitted_empty_for_a_later_pass_to_fill() {
        let pool = vec![candidate("a", "a"), candidate("b", "a")];
        let draft = build(&pool, &[0, 1], &[MinedCase { expected: 0, distractors: vec![1] }], &[0.5]);
        assert_eq!(draft.draft_cases[0].query, "");
    }

    #[test]
    fn provenance_survives_into_the_draft() {
        let pool = vec![candidate("a", "a")];
        let draft = build(&pool, &[0], &[], &[]);
        assert_eq!(draft.corpus[0].src_project.as_deref(), Some("p"));
    }

    #[test]
    fn a_missing_hardness_value_does_not_truncate_the_cases() {
        // The zip against a repeating tail exists so a short `hardest` slice
        // cannot silently drop cases — it would look like the miner found
        // fewer than it did.
        let pool: Vec<Candidate> = (0..3).map(|i| candidate(&format!("d{i}"), "a")).collect();
        let cases = vec![
            MinedCase { expected: 0, distractors: vec![1] },
            MinedCase { expected: 1, distractors: vec![2] },
        ];
        let draft = build(&pool, &[0, 1, 2], &cases, &[0.9]);
        assert_eq!(draft.draft_cases.len(), 2);
        assert_eq!(draft.draft_cases[1].hardest_distractor_cosine, 0.0);
    }
}
