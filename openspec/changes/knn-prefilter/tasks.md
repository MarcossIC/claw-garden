# knn-prefilter — Tareas (la puerta de medición)

Strict TDD: en cada grupo el test va primero y falla (RED), por la aserción o porque no compila; después el código (GREEN). Cada grupo es un commit convencional atómico, que se comprueba con las pruebas de los archivos que cambia: `cargo test -p clawg-core --lib crowding` (en Windows, con `--target-dir target-test`). La suite completa (`cargo test --workspace`) y los dos clippy (`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`) corren una vez, sobre el último commit. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea. Ningún build release, instalación ni acceso a `~/.clawg*`; nada necesita el modelo real.

Requisitos en la [spec](spec.md) (KP1 a KP8); formas, umbrales y corpus en la [propuesta](proposal.md).

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| Tests de más | los de la spec | `a_closed_reading_hides_every_other_agent` (KP2), `the_detector_reference_keeps_the_pool_of_the_detector` (KP3), `the_live_version_behind_its_revisions_is_found_first` (KP5), `the_thresholds_read_only_what_decides` (KP6) y `preferences_and_deletions_fall_apart_from_agents_and_topics` (KP1) | El lector, la lectura de los umbrales y el pool del detector son lógica que el informe usa; sin un test que los lea, sus campos quedaban sin leer hasta la tarea 4, y una advertencia es un error. El pool del detector es privado y la medición lo repite (spec, KP3) |
| `decides` de la forma | tarea 2 | tarea 4 | Lo lee el informe |
| Una de cada 20 | un contador | un sorteo por fila, en un commit propio después de la tarea 4 | La primera corrida mostró, en E1, temas en que todas las filas del lector estaban borradas: con 24 temas y 5 agentes, el contador volvía al mismo agente y tema cada 120 filas, múltiplo de 20 ([verify](verify.md), «Primera corrida») |

## 1. Propuesta, spec y tareas, antes de medir

- [x] `proposal.md` con las formas de base, el modelo del corpus y los umbrales U1 a U3 y K1; `spec.md`; este archivo; `state.yaml`.
- Commit: `docs(openspec): propose measuring the KNN window against the exact neighbours`

## 2. El corpus y lo puro (KP1, KP2, KP4)

- [x] RED en `crates/clawg-core/src/memory/crowding.rs` (`#[cfg(test)] mod crowding;` en `crates/clawg-core/src/memory/mod.rs`): `the_same_seed_lays_out_the_same_corpus`, `a_shape_holds_its_chains_copies_and_unowned_rows`, `the_layout_puts_topics_claims_and_unrelated_rows_at_their_distances`, `eligibility_follows_the_production_predicates`, `loss_counts_the_reference_rows_the_result_left_out`, `a_quantile_is_the_nearest_rank`. Fallan porque no compilan.
- [x] GREEN: SplitMix64 con gaussiana de Box-Muller (el mismo SplitMix64 que copian `perf_bench.rs` y `revision-eval`), las direcciones y la mezcla, `Shape` y `lay_out(shape, seed)`, `eligible`, `loss` y `quantile`.
- [x] Comprobar: `cargo test -p clawg-core --lib crowding`.
- Commit: `test(core): lay out a corpus of agents, projects and revision chains from chosen vectors`

## 3. La medición sobre el store, y que ve lo que dice (KP3, KP5, KP8)

- [x] RED: `the_exact_query_returns_the_pure_reference`, `without_foreign_rows_nothing_is_lost_in_one_pass`, `rows_past_the_widened_window_are_counted_lost`.
- [x] GREEN: sembrar el corpus en un store de `test_dir()` por SQL, en una transacción (una fila borrada sin vector); la consulta exacta con `reach_predicate!` y `memory_project_predicate!`; una búsqueda como la del daemon y una sonda de `detect_contradiction` que leen `search.knn_retry` y `search.duration` del recorder; cerrar el store.
- [x] Comprobar: `cargo test -p clawg-core --lib crowding`.
- Commit: `test(core): measure a search's window against the exact eligible neighbours`

## 4. Las dos corridas ignoradas (KP6, KP7)

- [x] `what_the_window_loses_against_the_exact_neighbours`: las cinco formas, el informe por tipo de consulta y la lectura de U1 a U3.
- [x] `what_the_exact_query_costs_against_the_window`: las nueve celdas, la comparación de filas, los cocientes y el plan.
- [x] Comprobar que compilan y que los tests de la suite siguen pasando: `cargo test -p clawg-core --lib crowding`.
- Commit: `test(core): report what the KNN window loses and what the exact query costs`
- [x] Después de la primera corrida, el sorteo por fila con `preferences_and_deletions_fall_apart_from_agents_and_topics`, que falló con el contador.
- Commit: `test(core): draw a corpus row's preference and deletion apart from its agent and topic`

## 5. Correr y registrar

- [x] `cargo test -p clawg-core --lib -- --ignored what_the_window_loses --nocapture` y `… what_the_exact_query_costs …`, en debug.
- [x] `verify.md`: la máquina y el build, cada medida por forma, cada umbral con su lectura, K1 y la decisión de la puerta. `state.yaml` y la fila 37 de `openspec/README.md`.
- [x] La suite completa y los dos clippy, sobre el último commit.
- Commit: `docs(openspec): record the KNN window measurement`
