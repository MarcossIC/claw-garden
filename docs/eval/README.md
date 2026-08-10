# RAG retrieval-quality harness

This directory holds the golden set and results for `rag-eval`, the binary
that measures retrieval quality for claw-garden's three-layer memory system.
It exists so that no claim of "this change improved retrieval" is made
without a reproducible number behind it.

**`docs/eval/golden.json` is the operational definition of "quality" for this
RAG.** Every metric the harness prints — Recall@k, MRR, nDCG@10, the
contradiction-threshold curve — means exactly what the cases in this file say
it means. Change what's in the corpus, the queries, or the expected refs, and
you change what "better retrieval" means for every phase that reads these
numbers. Treat edits to this file with the same seriousness as a change to
the ranking formula itself, because in effect that's what it is.

## Status of this file: provisional template

The current `golden.json` was authored by reading the code — the tier rules,
the memory types, the embedder — not from real usage. It has exactly enough
cases (13 retrieval cases, 9 contradiction pairs) to exercise every schema
axis and prove the harness runs end to end. It is **not** a finished golden
set.

A second pass adds roughly 40 more retrieval cases and contradiction pairs
drawn from the user's actual usage — real queries, real memory content, real
edge cases that only show up in practice. Until that pass lands, treat any
run recorded under `docs/eval/results/` as a baseline over a template, not
over real usage. Do not compare a later "full golden set" run against this
one and read the delta as a retrieval improvement — the two runs are
measuring different things by construction.

## Running the harness

```
cargo run --release --bin rag-eval --features eval -- --fake
cargo run --release --bin rag-eval --features eval -- --real
```

The binary is not part of `cargo test --lib` and never blocks CI. It needs
the real embedding model (~470 MB) for `--real` mode, so it runs as a manual
step or a manual CI workflow only.

| Flag | Effect |
|---|---|
| `--fake` | Seeds and scores using `FakeEmbedder` (deterministic, SipHash-seeded, no model download) |
| `--real` | Seeds and scores using the production `multilingual-e5-small` model; asserts `‖v‖ ≈ 1.0` |
| `--check-only` | With `--real`, verifies the model is present/loadable without downloading or running the harness |
| `--golden <path>` | Overrides the default `docs/eval/golden.json` |
| `--out <dir>` | Overrides the default `docs/eval/results/` |
| `--force` | Allows overwriting an existing results file for the same date+config |

Exit codes: `0` success, `2` golden set failed validation, `3` `--real
--check-only` and the model is absent.

## What each mode measures, and what it cannot measure

The two modes are not interchangeable and answer different questions.

**`--fake` measures the plumbing and the metric code** — that rows get
seeded, that `search_all_memories` returns something, that Recall/MRR/nDCG
are computed correctly over whatever comes back. It validates the *ranking
formula's arithmetic*, nothing about *semantic quality*.

It **cannot** evaluate:
- Whether the e5 `query:`/`passage:` prefixes matter (no real semantics to
  test them against).
- The contradiction threshold curve. `FakeEmbedder` produces unnormalized
  random vectors (‖v‖ ≈ 11.3), so every pair sits at roughly the same large
  L2 distance regardless of content — the curve comes back essentially flat
  (recall ≈ 0, false-positive rate ≈ 0 at every threshold). That is expected,
  not a bug in the harness or the golden set. Every `--fake` run must record
  `fake-embedder-unnormalized-contradiction-sweep-degenerate` in
  `known_bugs_live` so a reader doesn't mistake the flat curve for a working
  contradiction detector.
- Any ranking-order claim that depends on real similarity. `FakeEmbedder`
  vectors carry no semantics, so the similarity term is noise no matter how
  the score is computed — `--fake` measures the *shape* of the ranking
  formula, never whether the right memory came back.

  > Historical note: until the L2-vs-cosine fix, the score multiplied a
  > negative `(1.0 - distance)` term by a positive importance factor, so
  > higher-importance rows sorted *worse* than lower-importance ones. That
  > defect is fixed — the score now goes through `cosine_from_l2_unit`, which
  > is exact cosine for unit vectors and clamped to `[0, 1]`. Runs from before
  > the fix still list `knn-distance-is-l2-not-cosine`; that is correct for
  > those runs and must not be copied into new ones.

**`--real` measures semantic quality** — whether the KNN pool actually
returns query-relevant memories, and whether the contradiction threshold
curve has a usable elbow. Use `--real` for the phases that touch prefixes,
thresholds, hybrid search, or tokenizer choice; `--fake` for phases that only
touch the ranking formula itself.

## Reading the output

A run writes `docs/eval/results/{date}-{fake|real}.json` with:

- `retrieval.aggregate` — Recall@1/@5/@10, MRR, nDCG@10 averaged over
  `status: "active"` cases only.
- `retrieval.blocked` — the same per-case numbers for `status: "known_red"`
  cases, deliberately **excluded** from the aggregate. If a `blocked` case's
  score turns nonzero on a later run, the harness prints
  `KNOWN_RED CASE NOW PASSING: '<query>'` — that is the concrete,
  code-free signal that whatever tier-reachability fix was blocking it has
  landed.
- `retrieval.by_lang` — the same five metrics, split by language, one entry
  per language code, ordered by code. A case's language is the `lang` its
  `expected_refs` corpus rows **agree** on; a case whose expected rows leave
  `lang` unset or disagree about it is absent here while still counting toward
  `aggregate`, so these `n_cases` need not sum to `per_case.len()`. The model
  is multilingual and this golden set is deliberately bilingual, so a single
  headline average can hide one language retrieving markedly worse than the
  other — which is an actionable defect, and invisible until the numbers are
  split.
- `contradiction.by_lang` — the same threshold sweep, restricted to the pairs
  of each language. Production applies **one** threshold to both languages, so
  "does one threshold serve both" is a question the combined curve cannot
  answer. Note the small-n caveat below applies per language even harder: a
  language with three pairs produces a curve of thirds.
- `contradiction.curve` — one row per swept threshold
  (`0.10, 0.15, ..., 0.40`), each with `recall`, `fpr_no_contradiction`, and
  `fpr_identical` reported separately. A pair labeled `identical` scoring as
  "contradictory" at a given threshold is a different kind of false positive
  than a genuinely distinct, non-contradictory pair being flagged — don't
  merge the two rates.
- `known_bugs_live` — short slugs for defects known to be live in the
  codebase **at the moment of that run** (e.g.
  `fake-embedder-unnormalized-contradiction-sweep-degenerate`). A baseline
  taken on broken code is what makes the later fix measurable; this field is
  what stops that baseline from being misread as "this is how well retrieval
  works."

  The list is hand-maintained in `known_bugs_live()` — the harness measures
  around these bugs, it does not detect them. So a slug outlives its bug
  unless someone removes it, and a stale slug is worse than none: it tells
  every future reader that a fixed defect is still corrupting the numbers.
  **Fixing a listed bug includes deleting its slug in the same change.**
  Older result files keep the slugs that were true when they were written —
  they are a ledger, not a status board, and are never edited retroactively.

## Adding cases

### The tier trap — read this before adding a retrieval case

Not every memory the system stores is reachable by `search_all_memories`.
The KNN queries in `search.rs` filter `tier > 1` — tier-1 rows exist, but
they reach the prompt through a completely different, queryless channel
(`workspace.rs`), not through search. If you write a retrieval case whose
`expected_refs` points at a corpus row that seeds at tier 1, that case will
score zero **by construction**, no matter how good the ranking formula is.
Read as a plain recall failure, it looks like a ranking bug. It is not — it's
a case that was never retrievable to begin with.

A corpus row seeds at tier 1, and is therefore invisible to this harness,
when **either**:
- `importance >= 9.0` (any `memory_type`), or
- `memory_type == "user_preference"` (regardless of importance).

Everything else — `architectural_decision` / `session_summary` (tier 2),
`chat_message` / `tool_error` / `code_block` / `semantic_fact` (tier 3) — is
reachable, at any importance below 9.0.

The loader enforces this automatically: it computes the same tier every
corpus row would get at insert time and aborts with `E_TIER1_UNMARKED` if an
`expected_ref` resolves to tier 1 and the case isn't marked. So the failure
mode isn't silent — but it's still worth knowing *why* before you hit the
error, instead of guessing at it from the message. Two ways to write a
tier-1 case correctly:

1. **Don't** — pick a corpus row with a reachable `memory_type` and
   `importance < 9.0` for anything you expect the harness to actually find.
2. **Do, deliberately** — if you want to document that a `user_preference`
   or `importance >= 9.0` memory is *currently* unreachable (as this file's
   `c10`/`c11`/`c12` cases do), mark the case explicitly:
   ```json
   {
     "query": "...",
     "corpus_refs": ["c10"],
     "expected_refs": ["c10"],
     "status": "known_red",
     "blocked_by": "rag-tier-reachability"
   }
   ```
   `blocked_by` is required whenever `status` is `known_red` — the loader
   rejects a `known_red` case with no `blocked_by`, and separately warns
   (without aborting) if a `known_red` case turns out not to need the marker
   (none of its `expected_refs` actually seed at tier 1 — a stale marker).

### Schema reference

```json
{
  "corpus": [
    {"ref": "c01", "content": "...", "memory_type": "architectural_decision", "importance": 6.5, "lang": "es"}
  ],
  "retrieval_cases": [
    {"query": "...", "corpus_refs": ["c01"], "expected_refs": ["c01"]}
  ],
  "contradiction_pairs": [
    {"a": "...", "b": "...", "label": "contradiction", "lang": "es"}
  ]
}
```

- `memory_type` must be one of `chat_message`, `tool_error`, `code_block`,
  `session_summary`, `user_preference`, `architectural_decision`,
  `semantic_fact`. `procedural_skill` is not valid here — it lives in a
  separate table not reachable through the insert path this harness uses.
- `corpus_refs` documents the pool a case is meant to be evaluated against
  for readability; it is not an isolation mechanism — the whole corpus is
  retrievable to every query. `expected_refs` must be a subset of
  `corpus_refs`, which must itself be a subset of `corpus`.
- `status` defaults to `"active"` if omitted; the only other value is
  `"known_red"`.
- `label` on a contradiction pair is one of `contradiction`,
  `no_contradiction`, `identical`. The `identical` label is not decorative —
  it's the case a naive near-duplicate threshold gets wrong. This file
  includes both a verbatim-duplicate `identical` pair and a paraphrase
  `identical` pair (same meaning, different wording) — the paraphrase is the
  harder, more realistic case, and the one worth adding more of in the
  40-case follow-up.
- All three top-level structures reject unknown fields outright — a typo'd
  key is a load-time error, not a silently ignored expectation.
- Every language in this system's real use — Rioplatense Spanish and
  English — should be represented in new cases, since the production model
  is `multilingual-e5-small`.

### Realistic content, not lorem ipsum

Every case in this template uses domain content an assistant memory system
would actually store: architectural decisions, tool failures, session
summaries, conventions, user preferences. Keep that standard when extending
the file — a case whose intent isn't obvious to a human reviewer at a glance
is a case that will be silently misread the next time someone edits nearby.
