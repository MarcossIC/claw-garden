//! The score formula applied to every KNN hit: a pure, synchronous
//! `score()` combining cosine similarity, tier-floor-bounded recency decay,
//! an importance multiplier, and a live-status gate into one ranking value.

use crate::memory::clock::TimestampMs;
use crate::memory::TableKind;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// One row exactly as a KNN query returns it, before scoring: the columns
/// `SEARCH_SQL_*` selects, plus the table it came from. `knn_one` attaches
/// `kind` from its `KnnTable` descriptor rather than the query returning it —
/// a single SQL string only ever serves one table and never says which.
#[derive(Debug, Clone)]
pub(crate) struct RawHit {
    pub id: i64,
    pub kind: TableKind,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub distance: f64,
    pub created_at: TimestampMs,
    pub last_accessed_at: TimestampMs,
    pub importance: f64,
    /// The row holds every identifier the query names, and that identifier
    /// is rare enough to single it out (`search::identifier_matches`).
    pub lexical_match: bool,
    /// Who saved the row: the reader, or an agent it was granted.
    pub agent_id: Option<String>,
    /// The key a save revises the row under, if it has one.
    pub topic_key: Option<String>,
}

/// Tier-floor decay minimums — `MAX(tier_floor, exp_decay(...))` never lets a
/// hot-tier row's score collapse to near-zero purely from staleness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TierFloors {
    pub tier1: f64,
    pub tier2: f64,
    pub other: f64,
}

/// Per-`memory_type` decay rates, plus the two table-level rates episodic and
/// procedural rows use. Those two are NOT entries in this same string table:
/// episodic/procedural rows carry no `memory_type` of their own (see
/// `lambda_for`), and folding their spellings in here would let any memory
/// row that ever carried them silently re-route through a rate meant for a
/// different table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DecayLambdas {
    pub user_preference: f64,
    pub architectural_decision: f64,
    pub session_summary: f64,
    pub semantic_fact: f64,
    pub chat_message: f64,
    pub tool_error: f64,
    pub fallback: f64,
    pub episode: f64,
    pub procedure: f64,
}

/// `base + clamp(importance, clamp_lo, clamp_hi) / divisor` — kept in this
/// literal form rather than re-expressed as a `min + (imp-1)*(max-min)/9`
/// range: the two are algebraically equal but not bit-equal (0.55+0.2 vs
/// 0.5+0.25 differ at the last ULP), and this is the form the score formula
/// has always computed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImportanceFactor {
    pub base: f64,
    pub divisor: f64,
    pub clamp_lo: f64,
    pub clamp_hi: f64,
}

impl ImportanceFactor {
    /// Sweep-points-only constructor: derives `(base, divisor)` so the
    /// formula's two endpoints land exactly on `min`/`max`, keeping
    /// `clamp_lo`/`clamp_hi` at whatever `ScoringConfig::DEFAULT` declares —
    /// a swept point still clamps raw importance at the same boundary the
    /// corpus was seeded against; only the compression range moves.
    ///
    /// Never the path `ScoringConfig::DEFAULT` takes: solving `divisor =
    /// (clamp_hi-clamp_lo)/(max-min)` and `base = min - clamp_lo/divisor`
    /// introduces floating-point division `DEFAULT`'s literal
    /// `divisor: 60.0` never goes through, so even `spanning(0.85, 1.0)` —
    /// targeting DEFAULT's own endpoints — lands one ULP away from it.
    pub(crate) fn spanning(min: f64, max: f64) -> Self {
        let clamp_lo = ScoringConfig::DEFAULT.importance.clamp_lo;
        let clamp_hi = ScoringConfig::DEFAULT.importance.clamp_hi;
        let divisor = (clamp_hi - clamp_lo) / (max - min);
        let base = min - clamp_lo / divisor;
        Self { base, divisor, clamp_lo, clamp_hi }
    }
}

/// Everything `score()` needs, parameterized. Production always scores
/// through `&ScoringConfig::DEFAULT`; a sweep point is built as
/// `ScoringConfig { importance: X, ..ScoringConfig::DEFAULT }`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScoringConfig {
    pub tier_floors: TierFloors,
    pub decay: DecayLambdas,
    pub importance: ImportanceFactor,
    /// Re-scales cosine similarity as `(cos-floor)/(1-floor)`, clamped to
    /// `[0,1]`, when `Some`. `None` means no arithmetic runs at all — the
    /// no-op is read off the code rather than argued from IEEE equality.
    pub cosine_floor: Option<f64>,
    /// Added to the re-scaled cosine of a row that is a lexical match. The
    /// embedder reads `resolveLedger4182` and `resolveLedger4812` as nearly
    /// the same text; this is the term that tells them apart.
    pub lexical_bonus: f64,
}

impl ScoringConfig {
    /// The exact numbers this formula has always used, values this crate
    /// once computed via five registered SQLite UDFs before the ranking
    /// computation moved into Rust. A `const`, not a function: a `&'static`
    /// needs no per-call construction, and the numbers live in exactly one
    /// place, which is what makes "provably identical to the original SQL"
    /// readable rather than derived.
    pub(crate) const DEFAULT: Self = Self {
        tier_floors: TierFloors { tier1: 0.5, tier2: 0.2, other: 0.05 },
        decay: DecayLambdas {
            user_preference: 0.0019,
            architectural_decision: 0.0039,
            session_summary: 0.0077,
            semantic_fact: 0.0039,
            chat_message: 0.023,
            tool_error: 0.099,
            fallback: 0.023,
            episode: 0.0077,
            procedure: 0.0039,
        },
        // Compression range narrowed from [0.55, 1.0] to [0.85, 1.0]. Over
        // the evaluation corpus the wider range let raw `importance` decide
        // the top-ranked result far more often than relevance did — the
        // measured dominance of importance over cosine similarity in the
        // final ranking was extreme enough that importance was effectively
        // overriding retrieval rather than breaking ties within it. Pulling
        // the floor of the range up to 0.85 shrinks how much a single point
        // of raw importance can move the product, so it acts as a
        // tie-breaker among otherwise-similar matches instead of the
        // dominant term. Kept in the literal `base + clamp(imp,lo,hi) /
        // divisor` form documented on `ImportanceFactor`, not produced by
        // calling the range-solving constructor: at `imp == clamp_lo ==
        // 1.0` the clamp/divisor term contributes `1.0 / 60.0`, so `base`
        // is written as that amount subtracted from the target floor,
        // `0.85 - 1.0/60.0 = 0.8333333333333333`; at `imp == clamp_hi ==
        // 10.0` the same base plus `10.0/60.0` lands on exactly `1.0`. The
        // range-solving constructor exists only to build sweep points and,
        // by its own contract, cannot reproduce this exact literal — it
        // reaches the same range through a division the compiler doesn't
        // perform for a `const`, which lands one ULP away.
        importance: ImportanceFactor {
            base: 0.8333333333333333, // 0.85 - 1.0 / 60.0, see comment above
            divisor: 60.0,
            clamp_lo: 1.0,
            clamp_hi: 10.0,
        },
        // Re-scales cosine similarity against 0.79, the 5th percentile of
        // the query-to-passage cosines of this repository's documentation
        // under mean pooling (0.7904; the CLS-pooled 0.8793 it replaces sat
        // at the same percentile of that corpus and would now put 91% of the
        // top-10 hits under the floor). Swept from 0.70 to 0.83 over the
        // evaluation corpus with `rag-eval --sweep-cos-floor`, 0.79 keeps the
        // best MRR, nDCG@10 and recall@10. Left
        // unscaled, every hit's cosine term falls inside a narrow band, so it
        // barely moves relative to the importance factor's own range and has
        // almost no say in the final ranking; re-scaling against this floor
        // stretches that band back out so similarity can compete with
        // importance in the product.
        //
        // This number is bound to the embedding prefix configuration in
        // effect when it was measured — the query and passage prefixes
        // production sends to the embedder today. The prefixes are part of
        // what the embedder actually encodes, so changing either one shifts
        // the corpus's whole cosine-similarity distribution and silently
        // invalidates this floor. There is no way to detect that from the
        // code path itself: if either prefix ever changes, this floor must
        // be re-measured against the new distribution, not carried over.
        cosine_floor: Some(0.79),
        // As much as a perfect cosine: with equal decay and importance an
        // exact identifier always ranks above a row that only reads alike,
        // while decay and importance keep multiplying both.
        lexical_bonus: 1.0,
    };
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// For unit vectors, ||a-b||^2 = 2 - 2*cos(a,b) => cos(a,b) = 1 - d^2/2.
/// Clamped to 0.0: cos ranges [-1, 1], so a distance past sqrt(2) would
/// otherwise go negative — `max(0.0)` keeps every KNN hit's similarity
/// factor inside [0, 1] instead of dragging the whole score product negative.
fn cosine_from_l2_unit(distance: f64) -> f64 {
    (1.0 - distance * distance / 2.0).max(0.0)
}

/// Re-scales an already-clamped-nonnegative cosine against a measured
/// corpus floor: `(cos-floor)/(1-floor)`, clamped to `[0,1]`. The measured
/// live-corpus cosine distribution sits in a 0.86-0.95 band (p5-p95 swings
/// only 1.04x), so left unscaled the similarity term has almost no room to
/// compete with `importance_factor`'s 1.8x range inside the product this
/// feeds — re-scaling spreads that same 1.04x band across the full [0,1]
/// interval instead of leaving it crammed into the top few percent.
fn rescale_cosine(cosine: f64, floor: f64) -> f64 {
    ((cosine - floor) / (1.0 - floor)).clamp(0.0, 1.0)
}

/// Resolves the decay rate for one row. Dispatches on `TableKind` FIRST:
/// episodic and procedural rows have no `memory_type` of their own — their
/// SQL selects the literal strings `'episode'`/`'procedure'` in that column —
/// so ownership of their rate stays with the table it came from, not with a
/// shared string match keyed on a spelling those rows never actually carry.
pub(crate) fn lambda_for(cfg: &ScoringConfig, kind: TableKind, memory_type: &str) -> f64 {
    match kind {
        TableKind::Episodic => cfg.decay.episode,
        TableKind::Procedural => cfg.decay.procedure,
        TableKind::Memory => match memory_type {
            "user_preference" => cfg.decay.user_preference,
            "architectural_decision" => cfg.decay.architectural_decision,
            "session_summary" => cfg.decay.session_summary,
            "semantic_fact" => cfg.decay.semantic_fact,
            "chat_message" => cfg.decay.chat_message,
            "tool_error" => cfg.decay.tool_error,
            _ => cfg.decay.fallback,
        },
    }
}

// The stored stamp is milliseconds and this decay reads a whole number of
// seconds, so both sides are lifted to milliseconds and the difference
// divided back down — the two are never rounded apart. `now` is floored to a
// whole second first because the age it produces has always been a
// whole-second value, and both divisions truncate toward zero, so a stamp in
// the future still yields the negative age that says the clock is wrong.
//
// `/` is used rather than `div_euclid` because that is what the original integer
// division did: they agree for every exact multiple and every non-negative
// dividend, and diverge only when the dividend is negative and not evenly
// divisible — exactly the future-stamp case this formula exists to get right.
fn age_secs_from(now: TimestampMs, last_accessed_at: TimestampMs) -> i64 {
    let now_secs = now.as_millis_i64() / 1000;
    (now_secs * 1000 - last_accessed_at.as_millis_i64()) / 1000
}

/// Pure, sync, no SQLite connection — the ranking value for `hit`. `now` is
/// read once by the caller and shared across every row scored in a search,
/// so no two rows in the same result set have their decay read at a
/// different instant.
pub(crate) fn score(cfg: &ScoringConfig, hit: &RawHit, now: TimestampMs) -> f64 {
    let cosine = cosine_from_l2_unit(hit.distance);
    // `None` runs no arithmetic at all — the raw clamped cosine passes
    // through untouched, which is why `Some(0.0)` and `None` stay bit-equal
    // (division by 1.0 and subtraction of 0.0 are both exact in IEEE 754,
    // so `Some(0.0)`'s arithmetic reduces to the identity anyway). `Some(f)`
    // for any other `f` is the re-scale the sweep varies.
    let cosine = match cfg.cosine_floor {
        Some(floor) => rescale_cosine(cosine, floor),
        None => cosine,
    };
    // Added after the re-scale so a match below the floor still scores. A
    // row without a match runs no arithmetic and keeps its exact score.
    let relevance = if hit.lexical_match { cosine + cfg.lexical_bonus } else { cosine };

    let tier_floor = match hit.tier {
        1 => cfg.tier_floors.tier1,
        2 => cfg.tier_floors.tier2,
        _ => cfg.tier_floors.other,
    };
    let lambda = lambda_for(cfg, hit.kind, &hit.memory_type);
    let age_secs = age_secs_from(now, hit.last_accessed_at);
    let age_days = age_secs as f64 / 86_400.0;
    let decay = (-lambda * age_days).exp();

    let importance = cfg.importance.base
        + hit
            .importance
            .clamp(cfg.importance.clamp_lo, cfg.importance.clamp_hi)
            / cfg.importance.divisor;

    // A disputed row ranks like an active one. The contradiction detector
    // flags neighbours by embedding distance, which measures topical
    // similarity, so it also flags same-topic rows holding a different fact;
    // demoting them buried correct answers. The possible conflict stays
    // recorded on the row and visible in the CONFLICTS block of the context.
    let status = match hit.status.as_str() {
        "active" | "disputed" => 1.0,
        _ => 0.0,
    };

    relevance * tier_floor.max(decay) * importance * status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `RawHit` fixture — every field but the ones a test cares
    /// about is filled with an inert placeholder.
    fn make_raw_hit(
        kind: TableKind,
        memory_type: &str,
        tier: i64,
        status: &str,
        distance: f64,
        last_accessed_at: TimestampMs,
        importance: f64,
    ) -> RawHit {
        RawHit {
            id: 1,
            kind,
            content: String::new(),
            memory_type: memory_type.to_string(),
            tier,
            status: status.to_string(),
            distance,
            created_at: TimestampMs::from_unix_millis(0),
            last_accessed_at,
            importance,
            lexical_match: false,
            topic_key: None,
            agent_id: None,
        }
    }

    /// A `RawHit` with every scoring factor neutralized to 1.0 except
    /// `importance`: zero distance (cosine=1.0), zero age (decay=1.0
    /// regardless of lambda, since `exp(0)=1`), and the given status. The
    /// full `score()` pipeline then reduces to exactly the importance
    /// factor, the same isolation a standalone `importance_factor()` UDF
    /// used to give for free.
    fn neutral_hit_at_importance(importance: f64, status: &str) -> RawHit {
        make_raw_hit(
            TableKind::Memory,
            "chat_message",
            1,
            status,
            0.0,
            TimestampMs::from_unix_millis(0),
            importance,
        )
    }

    fn epoch() -> TimestampMs {
        TimestampMs::from_unix_millis(0)
    }

    // ── ScoringConfig::DEFAULT — pins the shipped literals themselves ──
    //
    // Every formula test below re-derives its expectation from
    // `cfg.importance.{base,divisor,clamp_lo,clamp_hi}` / `cfg.cosine_floor`
    // read off whatever config is passed in, never from a bare number. That
    // is deliberate: it lets those tests keep checking that `score()`
    // assembles the formula correctly no matter what the default's numbers
    // are. But precisely because of that, none of them can notice if the
    // numbers themselves change — a test built to survive the default
    // moving cannot also be the test that catches the default moving. This
    // test is that second, narrower check: it pins `ScoringConfig::DEFAULT`'s
    // six ranking literals as bare values, so an edit to any of them fails
    // a test instead of silently changing what config every formula test is
    // a tautology about.
    //
    // `cosine_floor`'s value is the 5th percentile of the cosine-similarity
    // distribution under mean pooling, and it is valid only for the pooling
    // and the embedding prefixes production sends today — changing either the query or passage prefix
    // shifts that whole distribution and invalidates this number; it would
    // need to be re-measured, not carried over.
    #[test]
    fn scoring_config_default_pins_the_shipped_ranking_literals() {
        const REEVAL_NOTE: &str = "the ranking balance between cosine similarity and importance \
            was re-tuned. Every eval number recorded in the results JSONs (recall@k, mrr, ndcg, \
            importance_rank_dominance, modal_rank_agreement, ...) describes the OLD balance, not \
            the one that now ships — those numbers no longer describe the shipped system. A \
            fresh `--real --audit` run is required before this change can be trusted.";

        let importance = ScoringConfig::DEFAULT.importance;
        assert_eq!(
            importance.base, 0.8333333333333333,
            "ScoringConfig::DEFAULT.importance.base changed from its shipped value: {REEVAL_NOTE}"
        );
        assert_eq!(
            importance.divisor, 60.0,
            "ScoringConfig::DEFAULT.importance.divisor changed from its shipped value: {REEVAL_NOTE}"
        );
        assert_eq!(
            importance.clamp_lo, 1.0,
            "ScoringConfig::DEFAULT.importance.clamp_lo changed from its shipped value: {REEVAL_NOTE}"
        );
        assert_eq!(
            importance.clamp_hi, 10.0,
            "ScoringConfig::DEFAULT.importance.clamp_hi changed from its shipped value: {REEVAL_NOTE}"
        );
        assert_eq!(
            ScoringConfig::DEFAULT.cosine_floor, Some(0.79),
            "ScoringConfig::DEFAULT.cosine_floor changed from its shipped value (or the cosine \
             re-scale was turned off): {REEVAL_NOTE} This floor specifically is the 5th \
             percentile of the cosine-similarity distribution measured over the evaluation \
             corpus and is only valid for today's embedding prefixes — if this failed because \
             a prefix changed, the floor must be re-measured against the new distribution \
             before it is re-pinned here."
        );
        assert_eq!(
            ScoringConfig::DEFAULT.lexical_bonus, 1.0,
            "ScoringConfig::DEFAULT.lexical_bonus changed from its shipped value: {REEVAL_NOTE}"
        );
    }

    // importance_factor: base + clamp(imp,lo,hi)/divisor. Every case below
    // drives the full `score()` pipeline with cosine, decay, and status
    // neutralized to 1.0, so the result is exactly this one factor. The
    // expectation is computed from `ScoringConfig::DEFAULT`'s own fields
    // (independently of `score()`'s internals) rather than pinned as a bare
    // literal: a future change to the default's importance range then
    // re-derives the expectation for its new numbers automatically, while
    // this test keeps checking the one thing it actually exists to check —
    // that `score()` applies the formula correctly — instead of quietly
    // going stale the moment the literals move.
    fn expected_importance_factor(cfg: &ScoringConfig, importance: f64) -> f64 {
        cfg.importance.base
            + importance.clamp(cfg.importance.clamp_lo, cfg.importance.clamp_hi) / cfg.importance.divisor
    }

    #[test]
    fn importance_factor_min_value() {
        let cfg = ScoringConfig::DEFAULT;
        let expected = expected_importance_factor(&cfg, cfg.importance.clamp_lo);
        let v = score(&cfg, &neutral_hit_at_importance(cfg.importance.clamp_lo, "active"), epoch());
        assert!((v - expected).abs() < 1e-9, "expected {expected}, got {v}");
    }

    #[test]
    fn importance_factor_max_value() {
        let cfg = ScoringConfig::DEFAULT;
        let expected = expected_importance_factor(&cfg, cfg.importance.clamp_hi);
        let v = score(&cfg, &neutral_hit_at_importance(cfg.importance.clamp_hi, "active"), epoch());
        assert!((v - expected).abs() < 1e-9, "expected {expected}, got {v}");
    }

    #[test]
    fn importance_factor_mid_value() {
        let cfg = ScoringConfig::DEFAULT;
        // 5.0 is an arbitrary point strictly inside [clamp_lo, clamp_hi],
        // not the midpoint of the range — it exercises the unclamped branch
        // of the formula, same as it always has.
        let expected = expected_importance_factor(&cfg, 5.0);
        let v = score(&cfg, &neutral_hit_at_importance(5.0, "active"), epoch());
        assert!((v - expected).abs() < 1e-9, "expected {expected}, got {v}");
    }

    #[test]
    fn importance_factor_clamp_below_floor() {
        let cfg = ScoringConfig::DEFAULT;
        // 0.0 is below clamp_lo; the clamp inside expected_importance_factor
        // pulls it back up to clamp_lo, so this must equal the min-value case.
        let expected = expected_importance_factor(&cfg, 0.0);
        let v = score(&cfg, &neutral_hit_at_importance(0.0, "active"), epoch());
        assert!((v - expected).abs() < 1e-9, "expected {expected} (clamped), got {v}");
    }

    #[test]
    fn importance_factor_clamp_above_ceiling() {
        let cfg = ScoringConfig::DEFAULT;
        // 15.0 is above clamp_hi; the clamp pulls it back down to clamp_hi,
        // so this must equal the max-value case.
        let expected = expected_importance_factor(&cfg, 15.0);
        let v = score(&cfg, &neutral_hit_at_importance(15.0, "active"), epoch());
        assert!((v - expected).abs() < 1e-9, "expected {expected} (clamped), got {v}");
    }

    #[test]
    fn full_score_expression_in_sql_integration() {
        // Reassembles the expected score from the same building blocks
        // score() itself calls (cosine_from_l2_unit, the cosine re-scale
        // gate, lambda_for, age_secs_from, the importance formula, the
        // status gate) rather than pinning a printed-out literal. This
        // still catches an assembly bug in score() — wrong multiplication
        // order, a dropped factor, tier_floor/decay picking the wrong
        // branch, the re-scale gate not actually being wired up — while
        // staying correct across any future change to what
        // ScoringConfig::DEFAULT's fields are.
        let cfg = ScoringConfig::DEFAULT;
        let now = TimestampMs::from_unix_millis(1_700_000_000_000);
        let last_accessed_at = TimestampMs::from_unix_millis(now.as_millis_i64() - 86_400_000);
        let raw = make_raw_hit(
            TableKind::Memory, "chat_message", 2, "active", 0.1, last_accessed_at, 5.0,
        );

        let cosine_raw = cosine_from_l2_unit(raw.distance);
        let cosine = match cfg.cosine_floor {
            Some(floor) => rescale_cosine(cosine_raw, floor),
            None => cosine_raw,
        };
        let tier_floor = cfg.tier_floors.tier2; // raw.tier == 2
        let lambda = lambda_for(&cfg, raw.kind, &raw.memory_type);
        let age_days = age_secs_from(now, raw.last_accessed_at) as f64 / 86_400.0;
        let decay = (-lambda * age_days).exp();
        let importance = expected_importance_factor(&cfg, raw.importance);
        let expected = cosine * tier_floor.max(decay) * importance;

        let v = score(&cfg, &raw, now);
        assert!((v - expected).abs() < 1e-9, "expected {expected}, got {v}");
        assert!(v > 0.0 && v <= 1.0, "must be in (0.0, 1.0], got {v}");
    }

    // A disputed row is a possible conflict, not a wrong answer: it must rank
    // exactly like an active one. Anything else (superseded) never ranks.
    #[test]
    fn disputed_scores_exactly_like_active() {
        let cfg = ScoringConfig::DEFAULT;
        let now = TimestampMs::from_unix_millis(1_700_000_000_000);
        let at = TimestampMs::from_unix_millis(now.as_millis_i64() - 86_400_000);
        let hit = |status| make_raw_hit(TableKind::Memory, "semantic_fact", 2, status, 0.1, at, 5.0);

        let active = score(&cfg, &hit("active"), now);
        let disputed = score(&cfg, &hit("disputed"), now);

        assert!(active > 0.0);
        assert_eq!(disputed, active);
    }

    #[test]
    fn superseded_never_ranks() {
        let cfg = ScoringConfig::DEFAULT;
        let v = score(&cfg, &neutral_hit_at_importance(5.0, "superseded"), epoch());
        assert_eq!(v, 0.0);
    }

    // cosine_from_l2_unit — converts L2 distance between unit vectors into
    // exact cosine similarity, clamped so a KNN hit never scores below zero
    // regardless of how far apart the two vectors are.
    #[test]
    fn cosine_from_l2_unit_matches_rust_cosine() {
        fn normalize(v: [f64; 3]) -> [f64; 3] {
            let norm = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / norm, v[1] / norm, v[2] / norm]
        }
        fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
            a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
        }
        fn l2(a: [f64; 3], b: [f64; 3]) -> f64 {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        }

        let pairs = [
            (normalize([1.0, 0.0, 0.0]), normalize([1.0, 0.0, 0.0])), // identical: cos = 1.0
            (normalize([1.0, 0.0, 0.0]), normalize([0.0, 1.0, 0.0])), // orthogonal: cos = 0.0
            (normalize([1.0, 0.0, 0.0]), normalize([1.0, 1.0, 0.0])), // acute: cos ≈ 0.707
            (normalize([1.0, 0.0, 0.0]), normalize([-1.0, 0.0, 0.0])), // opposite: cos = -1.0 → clamp
            (normalize([1.0, 2.0, 3.0]), normalize([-3.0, -2.0, -1.0])), // obtuse: cos ≈ -0.714 → clamp
        ];

        for (a, b) in pairs {
            let rust_cosine = dot(a, b);
            let expected = rust_cosine.max(0.0);
            let distance = l2(a, b);
            let value = cosine_from_l2_unit(distance);
            assert!(
                (value - expected).abs() < 1e-9,
                "a={a:?} b={b:?}: dot product cosine={rust_cosine}, expected (clamped)={expected}, \
                 cosine_from_l2_unit(distance={distance})={value}"
            );
        }
    }

    #[test]
    fn cosine_from_l2_unit_never_negative() {
        // d=1.5 clamps to 0.0, not the unclamped magnitude: 1 - 1.5²/2 = -0.125,
        // and d=1.5 exceeds sqrt(2) ≈ 1.414 where cosine similarity for unit
        // vectors turns negative — `max(0.0)` is exactly what floors it.
        for (d, expected) in [(1.0_f64, 0.5_f64), (1.5, 0.0), (2.0, 0.0)] {
            let v = cosine_from_l2_unit(d);
            assert!((v - expected).abs() < 1e-9, "d={d}: expected {expected}, got {v}");
            assert!(v >= 0.0, "d={d}: score must never be negative, got {v}");
        }
    }

    // ── age_secs truncation — behaves like integer division ──

    #[test]
    fn age_secs_truncates_toward_zero() {
        let now_ms = 1_700_000_000_000_i64; // second-aligned baseline
        let now = TimestampMs::from_unix_millis(now_ms);
        // (offset_ms subtracted from now to get last_accessed_at, expected age_secs)
        let cases = [
            (0_i64, 0_i64),
            (999, 0),   // just under a second: truncates to 0, not rounds to 1
            (1000, 1),
            (1001, 1),
            (-1, 0),    // 1ms in the future: trunc-toward-zero gives 0, div_euclid would give -1
            (-1000, -1),
        ];
        for (offset_ms, expected_age_secs) in cases {
            let last_accessed_at = TimestampMs::from_unix_millis(now_ms - offset_ms);
            assert_eq!(
                age_secs_from(now, last_accessed_at),
                expected_age_secs,
                "offset_ms={offset_ms}"
            );
        }
    }

    #[test]
    fn age_secs_floors_now_to_a_whole_second_before_subtracting() {
        // now is 999ms into its second; last_accessed_at is 1ms before the
        // *previous* whole second boundary. Flooring `now` to 1_700_000_000_000
        // first gives age_secs=0 (a whole-second read always has, by
        // construction, no fractional part). Subtracting the raw, unfloored
        // `now` instead would give age_secs=1 — a different answer for the
        // identical instant.
        let now = TimestampMs::from_unix_millis(1_700_000_000_999);
        let last_accessed_at = TimestampMs::from_unix_millis(1_699_999_999_999);
        assert_eq!(age_secs_from(now, last_accessed_at), 0);
    }

    // ── cosine_floor: the sweep's zero point must coincide with the baseline ──

    #[test]
    fn cosine_floor_some_zero_is_bit_identical_to_none() {
        // A floor of 0.0 reduces to the identity: `(cos-0.0)/(1.0-0.0)` is
        // exact IEEE 754 subtraction and division, so `Some(0.0)` lands on
        // the identical bit pattern `None` (no arithmetic at all) produces,
        // regardless of what `cosine_floor` the DEFAULT itself carries.
        // Explicit `None`/`Some(0.0)` overrides are compared here rather
        // than reading `DEFAULT`'s own field, since the production default
        // is no longer `None` — this test's subject is the gate's zero
        // point, not which value happens to ship today.
        let now = TimestampMs::from_unix_millis(1_700_000_000_000);
        let last_accessed_at = TimestampMs::from_unix_millis(now.as_millis_i64() - 86_400_000);
        let raw = make_raw_hit(TableKind::Memory, "chat_message", 2, "active", 0.1, last_accessed_at, 5.0);

        let without_floor = ScoringConfig { cosine_floor: None, ..ScoringConfig::DEFAULT };
        let some_zero_floor = ScoringConfig { cosine_floor: Some(0.0), ..ScoringConfig::DEFAULT };

        let a = score(&without_floor, &raw, now);
        let b = score(&some_zero_floor, &raw, now);
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "Some(0.0) must produce the identical bit pattern None does: a={a} b={b}"
        );
    }

    // ── ImportanceFactor::spanning — sweep-points-only range constructor ──

    /// Evaluates the importance factor formula directly against a built
    /// `ImportanceFactor`, mirroring what `score()` computes internally —
    /// used here to test the constructor's algebra in isolation, without
    /// building a `RawHit` and neutralizing every other factor to reach it.
    fn factor_at(f: &ImportanceFactor, importance: f64) -> f64 {
        f.base + importance.clamp(f.clamp_lo, f.clamp_hi) / f.divisor
    }

    #[test]
    fn spanning_derives_base_and_divisor_so_the_endpoints_land_exactly_on_min_and_max() {
        for (min, max) in [(0.55, 1.0), (0.7, 1.0), (0.85, 1.0), (0.5, 0.9), (0.0, 1.0)] {
            let f = ImportanceFactor::spanning(min, max);
            let at_lo = factor_at(&f, f.clamp_lo);
            let at_hi = factor_at(&f, f.clamp_hi);
            assert!((at_lo - min).abs() < 1e-9, "min={min} max={max}: f(clamp_lo)={at_lo}, expected {min}");
            assert!((at_hi - max).abs() < 1e-9, "min={min} max={max}: f(clamp_hi)={at_hi}, expected {max}");
        }
    }

    #[test]
    fn spanning_keeps_the_defaults_clamp_bounds() {
        // spanning varies the COMPRESSION RANGE (base/divisor), never the
        // clamp bounds — those stay whatever ScoringConfig::DEFAULT declares,
        // so a swept importance point still clamps raw importance at the same
        // [1, 10] boundary the corpus was seeded against.
        let f = ImportanceFactor::spanning(0.6, 0.95);
        assert_eq!(f.clamp_lo, ScoringConfig::DEFAULT.importance.clamp_lo);
        assert_eq!(f.clamp_hi, ScoringConfig::DEFAULT.importance.clamp_hi);
    }

    #[test]
    fn spanning_targeting_the_defaults_own_endpoints_is_never_bit_equal_to_default() {
        // 0.85/1.0 are DEFAULT's own (base=0.8333333333333333, divisor=60.0)
        // endpoints, computed here directly from DEFAULT's fields rather
        // than hardcoded, so this test keeps targeting "the default's own
        // endpoints" even if the default's range moves again later. Solving
        // for them through division reintroduces floating-point rounding the
        // literal form never had — the two are algebraically equal, not
        // bit-equal. This is the guarantee that makes "the sweep's grid can
        // never accidentally reproduce the baseline" checkable rather than
        // assumed.
        let default_importance = ScoringConfig::DEFAULT.importance;
        let default_min = default_importance.base + default_importance.clamp_lo / default_importance.divisor;
        let default_max = default_importance.base + default_importance.clamp_hi / default_importance.divisor;

        let spanned = ImportanceFactor::spanning(default_min, default_max);
        assert_ne!(
            spanned, default_importance,
            "a spanning-derived config must never be bit-identical to the literal-form DEFAULT"
        );
    }

    // ── cosine re-scale — sweep-points-only, gated by cosine_floor: Some ──

    #[test]
    fn rescale_cosine_maps_the_floor_to_zero() {
        for floor in [0.0, 0.5, 0.8625, 0.95] {
            let v = rescale_cosine(floor, floor);
            assert!(v.abs() < 1e-12, "floor={floor}: expected 0.0, got {v}");
        }
    }

    #[test]
    fn rescale_cosine_maps_one_to_one_regardless_of_floor() {
        for floor in [0.0, 0.5, 0.8625, 0.95] {
            let v = rescale_cosine(1.0, floor);
            assert!((v - 1.0).abs() < 1e-12, "floor={floor}: expected 1.0, got {v}");
        }
    }

    #[test]
    fn rescale_cosine_clamps_a_cosine_below_the_floor_to_zero() {
        let v = rescale_cosine(0.5, 0.8625);
        assert_eq!(v, 0.0, "a cosine below the floor must clamp to 0.0, not go negative");
    }

    #[test]
    fn rescale_cosine_is_monotone_non_decreasing_for_any_floor() {
        for floor in [0.0, 0.5, 0.8625, 0.95] {
            let mut prev = rescale_cosine(floor, floor);
            for step in 1..=20 {
                let cosine = floor + (1.0 - floor) * (step as f64 / 20.0);
                let v = rescale_cosine(cosine, floor);
                assert!(v + 1e-12 >= prev, "floor={floor}: not monotone at cosine={cosine}: prev={prev} v={v}");
                prev = v;
            }
        }
    }

    #[test]
    fn cosine_floor_some_nonzero_changes_the_score_from_none() {
        // The activation proof: unlike `Some(0.0)` (a true no-op, tested
        // above), a real floor must move the score — otherwise the gate in
        // `score()` was never wired to `rescale_cosine` at all. Compared
        // against an explicit `None` override rather than `DEFAULT` itself,
        // since the production default now carries its own non-zero floor.
        let now = TimestampMs::from_unix_millis(1_700_000_000_000);
        let last_accessed_at = TimestampMs::from_unix_millis(now.as_millis_i64() - 86_400_000);
        let raw = make_raw_hit(TableKind::Memory, "chat_message", 2, "active", 0.1, last_accessed_at, 5.0);

        let without_floor = ScoringConfig { cosine_floor: None, ..ScoringConfig::DEFAULT };
        let with_floor = ScoringConfig { cosine_floor: Some(0.8625), ..ScoringConfig::DEFAULT };

        let a = score(&without_floor, &raw, now);
        let b = score(&with_floor, &raw, now);
        assert_ne!(a.to_bits(), b.to_bits(), "a measured floor must change the score, got a={a} b={b}");
    }

    // ── TableKind dispatch in lambda_for ──

    #[test]
    fn lambda_for_episodic_and_procedural_use_their_own_table_rate() {
        let cfg = ScoringConfig::DEFAULT;
        assert_eq!(lambda_for(&cfg, TableKind::Episodic, "episode"), cfg.decay.episode);
        assert_eq!(lambda_for(&cfg, TableKind::Procedural, "procedure"), cfg.decay.procedure);
        // Neither spelling is in the memory_type table, so if dispatch ever
        // fell through to it by mistake these would silently read the
        // fallback rate (0.023) instead.
        assert_ne!(cfg.decay.episode, cfg.decay.fallback);
        assert_ne!(cfg.decay.procedure, cfg.decay.fallback);
    }

    #[test]
    fn lambda_for_memory_dispatches_on_memory_type() {
        let cfg = ScoringConfig::DEFAULT;
        assert_eq!(lambda_for(&cfg, TableKind::Memory, "user_preference"), cfg.decay.user_preference);
        assert_eq!(lambda_for(&cfg, TableKind::Memory, "tool_error"), cfg.decay.tool_error);
        assert_eq!(lambda_for(&cfg, TableKind::Memory, "unrecognized"), cfg.decay.fallback);
    }

    // ── lexical bonus ──

    #[test]
    fn a_lexical_match_adds_the_bonus_after_the_floor() {
        // A distance whose cosine sits below the floor: cos' is 0, so before
        // the bonus this row scored exactly 0.
        let now = epoch();
        let mut raw = make_raw_hit(TableKind::Memory, "chat_message", 1, "active", 1.0, now, 1.0);
        raw.lexical_match = true;
        let cfg = ScoringConfig::DEFAULT;

        let importance = cfg.importance.base + 1.0 / cfg.importance.divisor;
        let s = score(&cfg, &raw, now);
        assert!(s > 0.0, "a match below the floor must score above zero, got {s}");
        assert_eq!(s, cfg.lexical_bonus * importance);
    }

    #[test]
    fn without_a_match_the_score_is_unchanged() {
        let now = TimestampMs::from_unix_millis(1_700_000_000_000);
        let last_accessed_at = TimestampMs::from_unix_millis(now.as_millis_i64() - 86_400_000);
        let raw = make_raw_hit(TableKind::Memory, "chat_message", 2, "active", 0.3, last_accessed_at, 5.0);
        let without_bonus = ScoringConfig { lexical_bonus: 0.0, ..ScoringConfig::DEFAULT };

        let a = score(&ScoringConfig::DEFAULT, &raw, now);
        let b = score(&without_bonus, &raw, now);
        assert_eq!(a.to_bits(), b.to_bits());

        let below_floor = make_raw_hit(TableKind::Memory, "chat_message", 1, "active", 1.0, epoch(), 1.0);
        assert_eq!(score(&ScoringConfig::DEFAULT, &below_floor, epoch()), 0.0);
    }

    #[test]
    fn a_match_outranks_a_closer_row_with_the_same_decay_and_importance() {
        let now = epoch();
        let closer = make_raw_hit(TableKind::Memory, "code_block", 1, "active", 0.0, now, 5.0);
        let mut matched = make_raw_hit(TableKind::Memory, "code_block", 1, "active", 0.3, now, 5.0);
        matched.lexical_match = true;
        let cfg = ScoringConfig::DEFAULT;
        assert!(score(&cfg, &matched, now) > score(&cfg, &closer, now));
    }
}
