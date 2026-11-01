# knn-prefilter — Tareas (la puerta de medición)

Strict TDD: en cada grupo el test va primero y falla (RED), por la aserción o porque no compila; después el código (GREEN). Cada grupo es un commit convencional atómico, que se comprueba con las pruebas de los archivos que cambia: `cargo test -p clawg-core --lib crowding` (en Windows, con `--target-dir target-test`). La suite completa (`cargo test --workspace`) y los dos clippy (`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`) corren una vez, sobre el último commit. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea. Ningún build release, instalación ni acceso a `~/.clawg*`; nada necesita el modelo real.

Requisitos en la [spec](spec.md) (KP1 a KP8); formas, umbrales y corpus en la [propuesta](proposal.md).

## 1. Propuesta, spec y tareas, antes de medir

- [x] `proposal.md` con las formas de base, el modelo del corpus y los umbrales U1 a U3 y K1; `spec.md`; este archivo; `state.yaml`.
- Commit: `docs(openspec): propose measuring the KNN window against the exact neighbours`

## 2. El corpus y lo puro (KP1, KP2, KP4)

- [ ] RED en `crates/clawg-core/src/memory/crowding.rs` (`#[cfg(test)] mod crowding;` en `crates/clawg-core/src/memory/mod.rs`): `the_same_seed_lays_out_the_same_corpus`, `a_shape_holds_its_chains_copies_and_unowned_rows`, `the_layout_puts_topics_claims_and_unrelated_rows_at_their_distances`, `eligibility_follows_the_production_predicates`, `loss_counts_the_reference_rows_the_result_left_out`, `a_quantile_is_the_nearest_rank`. Fallan porque no compilan.
- [ ] GREEN: SplitMix64 con gaussiana de Box-Muller (el mismo SplitMix64 que copian `perf_bench.rs` y `revision-eval`), las direcciones y la mezcla, `Shape` y `lay_out(shape, seed)`, `eligible`, `loss` y `quantile`.
- [ ] Comprobar: `cargo test -p clawg-core --lib crowding`.
- Commit: `test(core): lay out a corpus of agents, projects and revision chains from chosen vectors`

## 3. La medición sobre el store, y que ve lo que dice (KP3, KP5, KP8)

- [ ] RED: `the_exact_query_returns_the_pure_reference`, `without_foreign_rows_nothing_is_lost_in_one_pass`, `rows_past_the_widened_window_are_counted_lost`.
- [ ] GREEN: sembrar el corpus en un store de `test_dir()` por SQL, en una transacción (una fila borrada sin vector); la consulta exacta con `reach_predicate!` y `memory_project_predicate!`; una búsqueda como la del daemon y una sonda de `detect_contradiction` que leen `search.knn_retry` y `search.duration` del recorder; cerrar el store.
- [ ] Comprobar: `cargo test -p clawg-core --lib crowding`.
- Commit: `test(core): measure a search's window against the exact eligible neighbours`

## 4. Las dos corridas ignoradas (KP6, KP7)

- [ ] `what_the_window_loses_against_the_exact_neighbours`: las cinco formas, el informe por tipo de consulta y la lectura de U1 a U3.
- [ ] `what_the_exact_query_costs_against_the_window`: las nueve celdas, la comparación de filas, los cocientes y el plan.
- [ ] Comprobar que compilan y que los tests de la suite siguen pasando: `cargo test -p clawg-core --lib crowding`.
- Commit: `test(core): report what the KNN window loses and what the exact query costs`

## 5. Correr y registrar

- [ ] `cargo test -p clawg-core --lib -- --ignored what_the_window_loses --nocapture` y `… what_the_exact_query_costs …`, en debug.
- [ ] `verify.md`: la máquina y el build, cada medida por forma, cada umbral con su lectura, K1 y la decisión de la puerta. `state.yaml` y la fila 37 de `openspec/README.md`.
- [ ] La suite completa y los dos clippy, sobre el último commit.
- Commit: `docs(openspec): record the KNN window measurement`
