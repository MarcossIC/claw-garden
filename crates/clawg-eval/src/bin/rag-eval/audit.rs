//! Audits the golden set itself rather than the retrieval it measures.
//!
//! Every other module here answers "how well did retrieval do?". This one
//! answers the question that has to be settled first: **is this golden set
//! capable of answering that at all?** A set whose cases are trivially easy
//! reports perfect scores and cannot detect a regression, and nothing about a
//! green run distinguishes it from a set that is genuinely being satisfied.
//! The 13-document template that preceded the domain set sat at recall@10 =
//! 1.0000 for months while measuring nothing.
//!
//! Three of the metrics below were previously computed by hand — twice during
//! the embedding-contract work and again during tier reachability, each time
//! from a fresh ad-hoc script, each time with the numbers quoted into a
//! document nobody could reproduce. Computing them here makes the difficulty
//! of a set a fact in the run log instead of a claim in a commit message.
//!
//! ## The distinction this module refuses to blur
//!
//! `importance_rank_dominance` is REPORTED but never gates. The other metrics
//! judge the golden set; dominance judges the *system* — it measures whether
//! ranking is driven by `importance` instead of relevance. Gating on it would
//! reject a perfectly good golden set because the retrieval it audits has a
//! known defect, which inverts the whole point of having an instrument.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::golden::CorpusRow;
use crate::report::CaseResult;

/// Fraction of cases scoring a perfect recall@10 above which the set can no
/// longer detect a regression: once nearly every case is already perfect,
/// there is no room left for a change to show up as a loss.
const MAX_SATURATION: f64 = 0.90;

/// Agreement with the modal ranking above which the ordering is effectively
/// independent of the query. Measured at 0.69 and 0.62 on earlier sets, both
/// of which were flagged in the plan as evidence the ranking ignored the
/// query; the gate sits above those so it fires on a set that is worse, not on
/// one merely known to be imperfect.
const MAX_MODAL_AGREEMENT: f64 = 0.80;

/// Mean query/answer token overlap above which the set is measuring paraphrase
/// detection rather than retrieval.
const MAX_MEAN_OVERLAP: f64 = 0.35;

/// Per-case overlap worth naming in the report so it can be rewritten.
const MIRROR_CASE_OVERLAP: f64 = 0.50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The set can carry a claim.
    Usable,
    /// At least one gate failed; numbers from this set should not be quoted.
    Unusable,
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorCase {
    pub query: String,
    pub expected_ref: String,
    pub overlap: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub n_cases: usize,
    /// Fraction of cases that already score a perfect recall@1 / recall@10.
    pub saturation_recall_1: f64,
    pub saturation_recall_10: f64,
    /// Mean fraction of rank positions where a case's ordering matches the
    /// most common ordering across all cases. 1.0 means every query returns
    /// the same list.
    pub modal_rank_agreement: f64,
    /// Spearman correlation between a document's `importance` and how well it
    /// ranks on average. `+1.0` means the ranking is exactly the corpus sorted
    /// by importance, regardless of what was asked. REPORTED, NEVER GATED —
    /// see the module docs.
    pub importance_rank_dominance: f64,
    pub mean_lexical_overlap: f64,
    pub mirror_cases: Vec<MirrorCase>,
    pub verdict: Verdict,
    /// One line per failed gate, in the words the report prints.
    pub failures: Vec<String>,
}

/// Spanish and English function words. They carry no topical signal, so
/// leaving them in would let a query and an unrelated document share half
/// their tokens and read as a paraphrase of each other.
const STOPWORDS: &[&str] = &[
    "a", "al", "and", "are", "as", "at", "be", "by", "com", "como", "con", "cual", "cuando",
    "de", "del", "did", "do", "does", "e", "el", "en", "era", "es", "esa", "ese", "eso", "esta",
    "este", "esto", "for", "from", "fue", "get", "ha", "han", "has", "have", "how", "in", "is",
    "it", "its", "la", "las", "lo", "los", "mas", "me", "no", "of", "on", "or", "para", "pero",
    "por", "que", "se", "ser", "si", "sin", "so", "son", "su", "sus", "than", "that", "the",
    "их", "their", "them", "then", "there", "these", "this", "to", "un", "una", "uno", "unos",
    "up", "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with",
    "y", "ya", "yo", "you", "your",
];

/// Lowercases, strips diacritics, and splits on anything that is not
/// alphanumeric.
///
/// Diacritics are folded so that a query asking about a `función` and a
/// document describing a `funcion` are recognised as the same word — the
/// extractor's own sources are inconsistent about accents, and treating the
/// two spellings as distinct tokens would under-report overlap on exactly the
/// Spanish cases where mirroring is most likely.
fn tokenize(text: &str) -> HashSet<String> {
    let folded: String = text
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect();

    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2 && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect()
}

/// Jaccard overlap between a query's tokens and its expected answer's tokens.
///
/// This is the guard against the failure mode that makes generated golden sets
/// worthless: a query written as a restatement of its own answer is found by
/// any embedder, so the case measures paraphrase detection and reports it as
/// retrieval quality. Two empty token sets score 0.0 — an empty query cannot
/// mirror anything, and 1.0 would flag it as the worst case in the set.
pub fn lexical_overlap(query: &str, answer: &str) -> f64 {
    let q = tokenize(query);
    let a = tokenize(answer);
    if q.is_empty() || a.is_empty() {
        return 0.0;
    }
    let intersection = q.intersection(&a).count() as f64;
    let union = q.union(&a).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Fraction of cases whose score under `pick` is a perfect 1.0.
fn saturation(cases: &[CaseResult], pick: impl Fn(&CaseResult) -> f64) -> f64 {
    if cases.is_empty() {
        return 0.0;
    }
    let perfect = cases.iter().filter(|c| pick(c) >= 1.0).count();
    perfect as f64 / cases.len() as f64
}

/// Mean agreement between each case's ranking and the modal ranking.
///
/// For every rank position, the modal document is whichever one appears there
/// most often across cases; a case's agreement is the fraction of positions
/// where it matches. High agreement means the ordering barely responds to the
/// query — the finding that opened the score-weighting phase, computed by hand
/// there and computed here instead.
pub fn modal_rank_agreement(cases: &[CaseResult]) -> f64 {
    let scored: Vec<&CaseResult> = cases.iter().filter(|c| !c.ranked_refs.is_empty()).collect();
    if scored.is_empty() {
        return 0.0;
    }
    let width = scored.iter().map(|c| c.ranked_refs.len()).max().unwrap_or(0);

    let modal: Vec<Option<&str>> = (0..width)
        .map(|position| {
            let mut tally: HashMap<&str, usize> = HashMap::new();
            for case in &scored {
                if let Some(r) = case.ranked_refs.get(position) {
                    *tally.entry(r.as_str()).or_insert(0) += 1;
                }
            }
            // Ties resolve to the lexicographically smaller ref so the modal
            // ranking is the same on every run over the same data.
            tally
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
                .map(|(r, _)| r)
        })
        .collect();

    let total: f64 = scored
        .iter()
        .map(|case| {
            let matches = case
                .ranked_refs
                .iter()
                .enumerate()
                .filter(|(position, r)| modal.get(*position).copied().flatten() == Some(r.as_str()))
                .count();
            matches as f64 / case.ranked_refs.len() as f64
        })
        .sum();

    total / scored.len() as f64
}

/// Ranks values, averaging the positions of tied entries — the standard
/// fractional ranking Spearman is defined against. Without tie-averaging, a
/// corpus with repeated importance values (this one has many) would get an
/// arbitrary order imposed on the ties and a correlation that changes with
/// input order.
fn fractional_ranks(values: &[f64], descending: bool) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| {
        let ord = values[a].partial_cmp(&values[b]).unwrap_or(std::cmp::Ordering::Equal);
        if descending {
            ord.reverse()
        } else {
            ord
        }
    });

    let mut ranks = vec![0.0; values.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        // Positions i..=j are tied; every one of them gets their mean rank.
        let mean_rank = ((i + j) as f64) / 2.0 + 1.0;
        for &index in &order[i..=j] {
            ranks[index] = mean_rank;
        }
        i = j + 1;
    }
    ranks
}

fn pearson(xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len();
    if n < 2 || n != ys.len() {
        return 0.0;
    }
    let mean_x = xs.iter().sum::<f64>() / n as f64;
    let mean_y = ys.iter().sum::<f64>() / n as f64;
    let mut cov = 0.0;
    let mut var_x = 0.0;
    let mut var_y = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        cov += (x - mean_x) * (y - mean_y);
        var_x += (x - mean_x).powi(2);
        var_y += (y - mean_y).powi(2);
    }
    if var_x == 0.0 || var_y == 0.0 {
        // One of the series is constant, so no monotone relationship exists to
        // measure. Zero is the honest answer; NaN would poison the report.
        return 0.0;
    }
    cov / (var_x.sqrt() * var_y.sqrt())
}

/// Spearman correlation between corpus `importance` and mean rank position.
///
/// `+1.0` means the two orderings coincide exactly: the most important
/// document is first for every query, whatever was asked. That is the
/// score-weighting finding, and this is the number that phase needs in order
/// to claim it improved anything.
///
/// Only documents that appear in at least one ranking are included — a
/// document that never surfaces has no mean rank, and treating its absence as
/// a rank would invent data.
pub fn importance_rank_dominance(cases: &[CaseResult], corpus: &[CorpusRow]) -> f64 {
    let mut positions: HashMap<&str, Vec<f64>> = HashMap::new();
    for case in cases {
        for (position, r) in case.ranked_refs.iter().enumerate() {
            positions.entry(r.as_str()).or_default().push(position as f64 + 1.0);
        }
    }

    let mut importances = Vec::new();
    let mut mean_ranks = Vec::new();
    for row in corpus {
        if let Some(seen) = positions.get(row.ref_.as_str()) {
            importances.push(row.importance);
            mean_ranks.push(seen.iter().sum::<f64>() / seen.len() as f64);
        }
    }
    if importances.len() < 2 {
        return 0.0;
    }

    // Importance descending and mean rank ascending both put "best" at rank 1,
    // so the two agree (+1) exactly when importance dictates the ordering.
    let importance_ranks = fractional_ranks(&importances, true);
    let position_ranks = fractional_ranks(&mean_ranks, false);
    pearson(&importance_ranks, &position_ranks)
}

/// Runs every check and decides whether the set can carry a claim.
pub fn audit(cases: &[CaseResult], corpus: &[CorpusRow]) -> AuditReport {
    let content_by_ref: HashMap<&str, &str> =
        corpus.iter().map(|r| (r.ref_.as_str(), r.content.as_str())).collect();

    let mut overlaps = Vec::new();
    let mut mirror_cases = Vec::new();
    for case in cases {
        for expected in &case.expected_refs {
            let Some(answer) = content_by_ref.get(expected.as_str()) else {
                continue;
            };
            let overlap = lexical_overlap(&case.query, answer);
            overlaps.push(overlap);
            if overlap >= MIRROR_CASE_OVERLAP {
                mirror_cases.push(MirrorCase {
                    query: case.query.clone(),
                    expected_ref: expected.clone(),
                    overlap,
                });
            }
        }
    }
    mirror_cases.sort_by(|a, b| b.overlap.partial_cmp(&a.overlap).unwrap_or(std::cmp::Ordering::Equal));

    let mean_lexical_overlap = if overlaps.is_empty() {
        0.0
    } else {
        overlaps.iter().sum::<f64>() / overlaps.len() as f64
    };

    let saturation_recall_1 = saturation(cases, |c| c.recall_1);
    let saturation_recall_10 = saturation(cases, |c| c.recall_10);
    let modal = modal_rank_agreement(cases);

    let mut failures = Vec::new();
    if saturation_recall_10 > MAX_SATURATION {
        failures.push(format!(
            "saturated: {:.1}% of cases already score a perfect recall@10 (limit {:.0}%) — \
             a regression has no room to show up as a loss",
            saturation_recall_10 * 100.0,
            MAX_SATURATION * 100.0
        ));
    }
    if modal > MAX_MODAL_AGREEMENT {
        failures.push(format!(
            "ranking barely responds to the query: modal agreement {modal:.4} \
             (limit {MAX_MODAL_AGREEMENT:.2})"
        ));
    }
    if mean_lexical_overlap > MAX_MEAN_OVERLAP {
        failures.push(format!(
            "queries mirror their answers: mean token overlap {mean_lexical_overlap:.4} \
             (limit {MAX_MEAN_OVERLAP:.2}) — this measures paraphrase detection, not retrieval"
        ));
    }

    let verdict = if failures.is_empty() { Verdict::Usable } else { Verdict::Unusable };

    AuditReport {
        n_cases: cases.len(),
        saturation_recall_1,
        saturation_recall_10,
        modal_rank_agreement: modal,
        importance_rank_dominance: importance_rank_dominance(cases, corpus),
        mean_lexical_overlap,
        mirror_cases,
        verdict,
        failures,
    }
}

pub fn print_report(report: &AuditReport) {
    println!("  golden-set audit (n={}):", report.n_cases);
    println!("    saturation recall@1     = {:.4}", report.saturation_recall_1);
    println!("    saturation recall@10    = {:.4}   (limit {MAX_SATURATION:.2})", report.saturation_recall_10);
    println!("    modal rank agreement    = {:.4}   (limit {MAX_MODAL_AGREEMENT:.2})", report.modal_rank_agreement);
    println!("    mean lexical overlap    = {:.4}   (limit {MAX_MEAN_OVERLAP:.2})", report.mean_lexical_overlap);
    println!(
        "    importance dominance    = {:+.4}  (reported, never gated: +1 means the ranking is \
         the corpus sorted by importance)",
        report.importance_rank_dominance
    );

    if !report.mirror_cases.is_empty() {
        println!("    queries that restate their own answer (rewrite these):");
        for m in report.mirror_cases.iter().take(10) {
            let shown: String = m.query.chars().take(70).collect();
            println!("      {:.2}  [{}]  {}", m.overlap, m.expected_ref, shown);
        }
    }

    match report.verdict {
        Verdict::Usable => println!("    verdict: USABLE — this set can carry a claim"),
        Verdict::Unusable => {
            println!("    verdict: UNUSABLE — do not quote numbers from this set");
            for f in &report.failures {
                println!("      - {f}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden::CaseStatus;

    fn case(query: &str, ranked: &[&str], recall_1: f64, recall_10: f64) -> CaseResult {
        CaseResult {
            query: query.to_string(),
            expected_refs: vec![ranked.first().copied().unwrap_or("c01").to_string()],
            ranked_refs: ranked.iter().map(|s| s.to_string()).collect(),
            recall_1,
            recall_5: recall_10,
            recall_10,
            mrr: recall_1,
            ndcg_10: recall_10,
            status: CaseStatus::Active,
            lang: None,
        }
    }

    fn row(ref_: &str, importance: f64, content: &str) -> CorpusRow {
        CorpusRow {
            ref_: ref_.to_string(),
            content: content.to_string(),
            memory_type: "semantic_fact".to_string(),
            importance,
            lang: Some("en".to_string()),
        }
    }

    // --- lexical overlap ---------------------------------------------------

    #[test]
    fn a_query_copied_from_its_answer_scores_maximum_overlap() {
        let text = "the maintenance worker demotes inactive memories after ninety days";
        assert!((lexical_overlap(text, text) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_query_asking_by_intent_scores_low_overlap() {
        let answer = "Removed countryId from the patchValue inside AddressLookupController.doLookup.";
        let query = "¿Por qué el campo de país dejó de sobrescribirse al autocompletar un domicilio?";
        assert!(
            lexical_overlap(query, answer) < 0.15,
            "intent-phrased query should not mirror its answer, got {}",
            lexical_overlap(query, answer)
        );
    }

    #[test]
    fn stopwords_alone_do_not_create_overlap() {
        // Without stopword removal these two share "the/of/and" and would look
        // related, letting a filler-heavy query pass as a genuine case.
        assert_eq!(lexical_overlap("the and of that with", "the and of that with"), 0.0);
    }

    #[test]
    fn diacritics_are_folded_so_accent_spelling_does_not_hide_a_mirror() {
        assert!(lexical_overlap("la funcion migrada", "la función migrada") > 0.9);
    }

    #[test]
    fn an_empty_query_scores_zero_not_one() {
        // Two empty sets are trivially "equal"; reporting 1.0 would rank an
        // empty query as the single worst case in the set.
        assert_eq!(lexical_overlap("", "anything at all here"), 0.0);
    }

    // --- saturation --------------------------------------------------------

    #[test]
    fn saturation_counts_only_perfect_cases() {
        let cases = vec![
            case("q1", &["c01"], 1.0, 1.0),
            case("q2", &["c02"], 0.0, 1.0),
            case("q3", &["c03"], 0.0, 0.0),
            case("q4", &["c04"], 0.0, 0.0),
        ];
        assert_eq!(saturation(&cases, |c| c.recall_1), 0.25);
        assert_eq!(saturation(&cases, |c| c.recall_10), 0.50);
    }

    #[test]
    fn saturation_of_no_cases_is_zero_not_nan() {
        assert_eq!(saturation(&[], |c| c.recall_1), 0.0);
    }

    // --- modal rank agreement ----------------------------------------------

    #[test]
    fn identical_rankings_agree_completely() {
        let cases = vec![
            case("q1", &["c01", "c02", "c03"], 0.0, 0.0),
            case("q2", &["c01", "c02", "c03"], 0.0, 0.0),
        ];
        assert!((modal_rank_agreement(&cases) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn fully_disjoint_rankings_barely_agree() {
        let cases = vec![
            case("q1", &["c01", "c02", "c03"], 0.0, 0.0),
            case("q2", &["c04", "c05", "c06"], 0.0, 0.0),
            case("q3", &["c07", "c08", "c09"], 0.0, 0.0),
        ];
        // Every position is a three-way tie, so exactly one case matches the
        // modal pick at each position: 1/3 of positions, for 1 of 3 cases.
        assert!(modal_rank_agreement(&cases) < 0.4);
    }

    #[test]
    fn agreement_of_no_cases_is_zero_not_nan() {
        assert_eq!(modal_rank_agreement(&[]), 0.0);
    }

    #[test]
    fn cases_that_returned_nothing_do_not_drag_agreement_to_zero() {
        // An empty ranking is a retrieval failure, not evidence about how
        // query-dependent the ordering is. Counting it as 0% agreement would
        // let a broken run look like a well-differentiated golden set.
        let cases = vec![
            case("q1", &["c01", "c02"], 0.0, 0.0),
            case("q2", &["c01", "c02"], 0.0, 0.0),
            case("q3", &[], 0.0, 0.0),
        ];
        assert!((modal_rank_agreement(&cases) - 1.0).abs() < 1e-12);
    }

    // --- importance dominance ----------------------------------------------

    #[test]
    fn a_ranking_sorted_purely_by_importance_scores_plus_one() {
        // The exact pathology found during tier reachability: the most
        // important document is first for every query, whatever was asked.
        let cases = vec![
            case("q1", &["c01", "c02", "c03"], 0.0, 0.0),
            case("q2", &["c01", "c02", "c03"], 0.0, 0.0),
        ];
        let corpus = vec![row("c01", 9.5, "a"), row("c02", 7.0, "b"), row("c03", 4.0, "c")];
        assert!(
            (importance_rank_dominance(&cases, &corpus) - 1.0).abs() < 1e-9,
            "got {}",
            importance_rank_dominance(&cases, &corpus)
        );
    }

    #[test]
    fn a_ranking_inverse_to_importance_scores_minus_one() {
        let cases = vec![case("q1", &["c03", "c02", "c01"], 0.0, 0.0)];
        let corpus = vec![row("c01", 9.5, "a"), row("c02", 7.0, "b"), row("c03", 4.0, "c")];
        assert!((importance_rank_dominance(&cases, &corpus) + 1.0).abs() < 1e-9);
    }

    #[test]
    fn dominance_ignores_documents_that_never_surfaced() {
        // c99 is in the corpus but in no ranking. Giving it an invented rank
        // would let unretrieved documents steer the correlation.
        let cases = vec![case("q1", &["c01", "c02"], 0.0, 0.0)];
        let corpus = vec![row("c01", 9.0, "a"), row("c02", 5.0, "b"), row("c99", 1.0, "z")];
        assert!((importance_rank_dominance(&cases, &corpus) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn uniform_importance_yields_zero_not_nan() {
        // A constant series has no variance, so no monotone relationship can
        // exist. NaN here would serialize into the results JSON as `null`.
        let cases = vec![case("q1", &["c01", "c02", "c03"], 0.0, 0.0)];
        let corpus = vec![row("c01", 5.0, "a"), row("c02", 5.0, "b"), row("c03", 5.0, "c")];
        assert_eq!(importance_rank_dominance(&cases, &corpus), 0.0);
    }

    #[test]
    fn ties_in_importance_are_ranked_fractionally() {
        // Two documents share importance 7.0. Fractional ranking gives both
        // rank 2.5 rather than imposing an arbitrary 2 and 3.
        let ranks = fractional_ranks(&[9.0, 7.0, 7.0, 4.0], true);
        assert_eq!(ranks, vec![1.0, 2.5, 2.5, 4.0]);
    }

    // --- the gate ----------------------------------------------------------

    #[test]
    fn a_saturated_set_is_unusable() {
        let cases: Vec<CaseResult> =
            (0..10).map(|i| case(&format!("q{i}"), &["c01"], 1.0, 1.0)).collect();
        let corpus = vec![row("c01", 5.0, "unrelated wording entirely")];
        let report = audit(&cases, &corpus);
        assert_eq!(report.verdict, Verdict::Unusable);
        assert!(report.failures.iter().any(|f| f.contains("saturated")), "{:?}", report.failures);
    }

    #[test]
    fn a_set_of_mirrored_queries_is_unusable_and_names_them() {
        let answer = "the maintenance worker demotes inactive tier-one memories after ninety days";
        let cases = vec![case(answer, &["c01"], 0.0, 0.0)];
        let corpus = vec![row("c01", 5.0, answer)];
        let report = audit(&cases, &corpus);
        assert_eq!(report.verdict, Verdict::Unusable);
        assert_eq!(report.mirror_cases.len(), 1);
        assert!(report.failures.iter().any(|f| f.contains("mirror")), "{:?}", report.failures);
    }

    #[test]
    fn a_discriminating_set_is_usable() {
        let corpus = vec![
            row("c01", 9.0, "Removed countryId from the patchValue inside the lookup controller"),
            row("c02", 5.0, "Cached suggestion items during the pending selection window"),
            row("c03", 3.0, "Consolidated two duplicate reference folders into one"),
        ];
        let cases = vec![
            case("¿Por qué dejó de pisarse el campo de país?", &["c01", "c02"], 1.0, 1.0),
            case("What happens while a selection is still pending?", &["c02", "c03"], 0.0, 1.0),
            case("Había dos carpetas repetidas. ¿Qué se hizo?", &["c03", "c01"], 0.0, 0.0),
        ];
        let report = audit(&cases, &corpus);
        assert_eq!(report.verdict, Verdict::Usable, "failures: {:?}", report.failures);
        assert!(report.mirror_cases.is_empty());
    }

    #[test]
    fn importance_dominance_alone_never_makes_a_set_unusable() {
        // Dominance is a property of the ranking system, not of the golden
        // set. A set that exposes the defect is doing its job; failing it here
        // would reject the instrument for successfully detecting a bug.
        // The two rankings differ in their tail, so modal agreement stays under
        // its gate, but mean position still follows importance almost exactly.
        // That combination is the whole point: a set can be perfectly healthy
        // and still expose importance-driven ranking.
        let corpus = vec![
            row("c01", 9.5, "alpha subject matter entirely distinct wording"),
            row("c02", 7.0, "beta topic sharing nothing lexically"),
            row("c03", 5.0, "gamma unrelated vocabulary altogether"),
            row("c04", 3.0, "delta separate concern with own terms"),
        ];
        let cases = vec![
            case("¿Qué pasó con el asunto principal?", &["c01", "c02", "c03", "c04"], 1.0, 1.0),
            case("What became of the secondary matter?", &["c01", "c02", "c04", "c03"], 0.0, 0.0),
        ];
        let report = audit(&cases, &corpus);
        assert!(
            report.importance_rank_dominance > 0.9,
            "dominance should be near-total here, got {}",
            report.importance_rank_dominance
        );
        assert_eq!(report.verdict, Verdict::Usable, "failures: {:?}", report.failures);
    }

    #[test]
    fn an_expected_ref_missing_from_the_corpus_does_not_panic() {
        let cases = vec![case("q1", &["c99"], 0.0, 0.0)];
        let report = audit(&cases, &[]);
        assert_eq!(report.mean_lexical_overlap, 0.0);
    }
}
