# eval-rigor — Verificación

**Estado:** verificado en Linux, commit por commit. Falta la suite en Windows y `rag-eval --real` sobre el set privado ([pendiente](#pendiente-máquina-de-marcos)).

## Tests

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama, en un worktree aparte.

| Commit | Pasan | Fallan | Warnings |
|---|---|---|---|
| Base (`main`, f8a361e) | 1.902 | 10 | 0 |
| `docs(openspec): propose paired tests, intervals and new case classes for rag-eval` | 1.902 | 10 | 0 |
| `test(eval): diff results from an inline fixture instead of an ignored file` | 1.905 | 7 | 0 |
| `feat(eval): report an exact paired p-value for every metric in --diff` | 1.912 | 7 | 0 |
| `feat(eval): add a paired bootstrap interval to the continuous metrics in --diff` | 1.917 | 7 | 0 |
| `feat(eval): compare what the agent received in --diff` | 1.921 | 7 | 0 |
| `feat(eval): fail a diff on a significant regression of a primary metric` | 1.927 | 7 | 0 |
| `feat(eval): report strict and partial hits with Wilson intervals` | 1.932 | 7 | 0 |
| `feat(eval): split cases into calibration and test by a hash of the query` | 1.936 | 7 | 0 |
| `feat(eval): measure abstention on cases that have no answer` | 1.943 | 7 | 0 |
| `feat(eval): measure whether a newer row outranks the one it replaces` | 1.952 | 7 | 0 |
| `feat(eval): rank by bm25 alone as a floor for the full ranking` | 1.955 | 7 | 0 |
| `docs: describe how rag-eval compares two runs` | 1.955 | 7 | 0 |

Todos los commits compilan. Las 7 fallas que quedan son las de `doctor::tests` de `main`, que modelan un PATH de Windows. Las 3 de `diff::tests` que leían `docs/eval/results/` (ignorado por git) pasan desde el segundo commit (R12).

**Conteo:** 50 tests nuevos, ninguno borrado. En Windows se espera la base más 50, sin fallas.

`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina con 0 en la punta de la rama.

## Lo que muestra el diff nuevo

Dos archivos sintéticos con la forma del caso de project-indexing: 46 casos, R@5 baja en 3 y `search.found` de agent-view baja de 41 a 30, con 100 tokens menos por búsqueda. Antes, `--diff` daba 0/0/0 en agent-view. Ahora:

```
  agent_view (fewer tokens is better; the mean is second run minus first):
    metric          better  worse     net       p
    search.found         0     11     -11  0.0010
    context.found        0      0       0  1.0000
    search.tokens       46      0      46  0.0000  mean -100.0000 [-100.0000, -100.0000]
    context.tokens       0      0       0  1.0000  mean +0.0000 [+0.0000, +0.0000]
  primary:
    retrieval recall@5: not distinguishable with 3 case(s) moving (p = 0.2500); a conclusion needs at least 6 cases moving one way
    agent_view search.found: distinguishable, worse (p = 0.0010, 0 better / 11 worse)
a primary metric got worse with p < 0.05
```

Con `--fail-on-regression` sale con 6.

## Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1 | `stats.rs`: `wilson_*`, `mcnemar_exact_*`, `bootstrap_*`, `six_cases_one_way_is_the_smallest_significant_count`, `splitmix_matches_the_golden_mine_sequence` |
| R2, R3 | `diff.rs`: `six_one_way_improvements_are_significant`, `one_improvement_is_not_distinguishable_and_says_how_many_are_needed`, `the_verdict_never_says_there_is_no_difference`, `a_continuous_metric_carries_a_bootstrap_interval`, `a_found_flip_in_agent_view_shows_up`, `fewer_tokens_count_as_better`, `agent_view_joins_on_query_and_skips_known_red`, `a_file_without_agent_view_diffs_without_the_block`, `abstention_is_diffed_when_both_files_have_it`, `freshness_is_diffed_when_both_files_have_update_cases` |
| R4 | `diff.rs`, `cli.rs` y `main.rs` (`diff_exits_six_on_a_significant_regression_only_with_the_flag`) |
| R5, R6, R7 | `report.rs` |
| R8, R9, R10 | `golden.rs`, `metrics.rs`, `seed.rs`, `report.rs`, `agent_view/mod.rs`, `eval_api.rs` y los de punta a punta de `main.rs` |
| R11 | `lexical_search_ranks_by_bm25_without_vectors` (`eval_api.rs`), `lexical_only_conflicts_with_the_other_arms`, `a_lexical_only_run_writes_a_tagged_file_that_diffs` |
| R12 | los tres de `diff.rs`, en verde en Linux sin `docs/eval/` |

## Pendiente (máquina de Marcos)

1. `cargo test --workspace --target-dir <dir>` en Windows: la base más 50 tests, 0 fallas.
2. `rag-eval --real` sobre `docs/eval/golden.json` y `--diff` contra `2026-10-15-real-project-indexing-tip-anchor.json`: 0 mejores y 0 peores en todas las métricas, p = 1, y el mismo `content_sha256`. El reporte nuevo trae `hits`, `by_split` y la nota «at least 6 cases moving one way (13.0 points of 46)».
3. `rag-eval --diff` entre los `2026-10-15-real-relsweep-*-agent-view.json` de 0,60 y 0,35: `search.found` con 6 mejores, 0 peores y p = 0,031; entre 0,35 y 0,20, 1 y 0, p = 1.
4. Opcional: `rag-eval --real --lexical-only` y su `--diff` contra el ancla, para tener el piso de BM25.
