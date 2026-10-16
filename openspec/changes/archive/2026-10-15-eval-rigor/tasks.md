# eval-rigor — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila, pasa la suite y no deja warnings. Una función de `stats.rs` entra en el mismo commit que su primer llamador, para no dejar código muerto ni un `allow`.

## 1. Los tests de `--diff` sin archivos ignorados (R12, D8)

- [x] Los tres tests de `diff.rs` que leen `docs/eval/results/2026-08-16-real-symmetric.json` pasan a un fixture en el código. Hoy fallan en Linux.
- Commit: `test(eval): diff results from an inline fixture instead of an ignored file`

## 2. p exacto en `--diff` (R1, R2, D1, D2)

- [x] Tests de `mcnemar_exact` y `min_one_way_significant` (`stats.rs`).
- [x] Tests `six_one_way_improvements_are_significant`, `one_improvement_is_not_distinguishable_and_says_how_many_are_needed` y `the_verdict_never_says_there_is_no_difference` (`diff.rs`).
- [x] `stats.rs` con las dos funciones; `MetricDiff::p_value`; columna `p` y línea de veredicto para R@5.
- Commit: `feat(eval): report an exact paired p-value for every metric in --diff`

## 3. Intervalo bootstrap en las métricas continuas (R1, R2, D1, D3)

- [x] Tests de SplitMix64 y de `paired_bootstrap_ci` (`stats.rs`); `a_continuous_metric_carries_a_bootstrap_interval` (`diff.rs`).
- [x] `MetricDiff::ci` para MRR y nDCG@10, impreso al lado del p.
- Commit: `feat(eval): add a paired bootstrap interval to the continuous metrics in --diff`

## 4. Agent-view en `--diff` (R3, D2)

- [x] Tests `a_found_flip_in_agent_view_shows_up`, `fewer_tokens_count_as_better`, `agent_view_joins_on_query_and_skips_known_red` y `a_file_without_agent_view_diffs_without_the_block` (`diff.rs`).
- [x] Bloque `agent_view` en `RunDiff` y su veredicto primario de `search_found`; texto de ayuda.
- Commit: `feat(eval): compare what the agent received in --diff`

## 5. `--fail-on-regression` (R4, D2)

- [x] Tests de `diff.rs`, `cli.rs` y `main.rs` de R4.
- [x] El flag, el código de salida 6 y la ayuda.
- Commit: `feat(eval): fail a diff on a significant regression of a primary metric`

## 6. Aciertos estrictos y parciales, y la nota de procedencia (R5, R6, D5)

- [x] Tests de `wilson` (`stats.rs`) y de R5 y R6 (`report.rs`).
- [x] `Rate`, `Hits`, `retrieval.hits` y `hits` en `by_lang` y `by_subset`; `min_one_way_cases` y la nota; R@5 estricto y parcial en la consola.
- Commit: `feat(eval): report strict and partial hits with Wilson intervals`

## 7. Partición por hash (R7, D4)

- [x] Tests de R7 (`report.rs`).
- [x] `split_of`, `retrieval.by_split` y su tabla en la consola.
- Commit: `feat(eval): split cases into calibration and test by a hash of the query`

## 8. Casos `unanswerable` (R8, R10, D6)

- [x] Tests de validación (`golden.rs`), del reporte, de `main.rs`, de agent-view y `a_golden_without_the_new_classes_measures_exactly_as_before`.
- [x] `CaseSubset::Unanswerable`, la regla de esperadas vacías, `retrieval.unanswerable` y `abstention`, el bloque de agent-view, y la abstención en `--diff`.
- Commit: `feat(eval): measure abstention on cases that have no answer`

## 9. Casos `update` (R9, D6)

- [x] Test `eval_insert_backdates_created_at` (`eval_api.rs`); `EvalStore::insert` con `age_ms`.
- [x] Tests de validación, de `fresh_first`, de `seed.rs` (la conversión de `age_days` a milisegundos: `SeedResult` no expone la fecha de una fila, así que el retraso real lo prueba `eval_insert_backdates_created_at`) y de la frescura en `--diff`.
- [x] `CaseSubset::Update`, `stale_refs`, `age_days`, `fresh_first` y `retrieval.freshness`.
- Commit: `feat(eval): measure whether a newer row outranks the one it replaces`

## 10. Brazo sólo léxico (R11, D7)

- [x] Test `lexical_search_ranks_by_bm25_without_vectors` (`eval_api.rs`) y `EvalStore::lexical_search`.
- [x] Tests de `cli.rs` y `main.rs` de R11; el flag y la ayuda.
- Commit: `feat(eval): rank by bm25 alone as a floor for the full ranking`

## 11. Docs

- [x] `.claude/CLAUDE.md`: cómo comparar dos corridas con `--diff` y qué es una métrica primaria.
- [x] `openspec/README.md`: estado de 33 en el mapa; sacar del backlog el punto de `--diff` y agent-view; sumar al backlog la tabla de CLS de `contradiction.rs:35-45`.
- Commit: `docs: describe how rag-eval compares two runs`

## 12. Verificación

- [x] `cargo test --workspace` en Linux en cada commit; el comando de `clippy` de `.claude/CLAUDE.md`.
- [x] `verify.md` con el conteo y lo pendiente en Windows con el golden set local: la suite en Windows, `rag-eval --real` contra el ancla (diff vacío) y `--diff` de los `relsweep` de agent-view (0,60 contra 0,35: 6 y 0, p = 0,031).
- [x] Correr esos pasos sobre `main` en `210c692`: pasan 1.983 y falla 1, un test de project-root-guard que no es de este cambio. `clippy` sale con código 0. El ancla da un diff vacío, y los `relsweep` y el piso léxico dan lo anticipado.

## 13. Archive

- [x] [`specs/eval`](../../../specs/eval/spec.md) con R1 a R12, la línea **Origen**, los `ruta:línea` revisados contra `main` y el piso léxico en la baseline medida.
- [x] `git mv` a `changes/archive/2026-10-15-eval-rigor`; `state.yaml` archivado; fila 33 y orden de ejecución del `README.md` de openspec; base de tests de `.claude/CLAUDE.md` y `openspec/config.yaml`; subir el minor.
