# search-arm-timing — Verificación

## Suite y clippy

En Linux x86_64, build debug, con el código de `cf39d3c`, el último commit que toca código; los que siguen sólo tocan documentos.

- `cargo test --locked --workspace` pasa.
- `cargo clippy --locked --workspace --all-targets -- -A clippy::all -D warnings` pasa.
- `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` pasa.
- Windows queda para el CI del pull request.

## Los tests en rojo

| Test | Antes | Después |
|---|---|---|
| `an_arm_measures_its_run_on_the_connection_not_its_wait` | falla con el código de `develop`: `knn_memory took 203.134ms of a search that waited 203.857ms` | pasa |
| `a_call_that_arrives_during_a_search_runs_between_its_arms` | falla sobre `99248f0`, que ya mide en el hilo pero todavía usa `try_join!`: `total` de 104,167 ms, por debajo de 250 | pasa |

Los dos corrieron seis veces seguidas sin fallar, y también dentro de la suite, en paralelo con el resto de los tests.

## El spec contra el código

- **A1.** El reloj de un KNN arranca dentro de la clausura de `knn_one` (`crates/clawg-core/src/memory/search.rs:573`) y vuelve en `KnnOutcome::ran` (`:439`, `:687`). `run_fail_soft` (`:852`) cronometra dentro de `call_raw` (`:860`) la consulta de `identifier_matches`, `lexical_candidates` y `score_lexical_candidates`. Una consulta que falla devuelve lo que corrió, y una llamada que no llegó al hilo devuelve 0. `record_arm` (`:1472`) registra la duración que recibe. `total` se sigue tomando desde `t0` (`:1299`) hasta el final (`:1450`). `search_all_memories_inner_fails_soft_when_memory_fts_is_missing` comprueba que el canal léxico que falló registra su brazo.
- **A2.** Los brazos se esperan en orden: `identifiers` (`:1315`), los tres KNN, cada uno con `?`, y los candidatos léxicos (`:1361`). En el módulo no queda ningún `try_join!`.
- **A3.** `a_search_records_how_long_each_arm_took` y `a_widened_window_is_counted_by_its_table` pasan sin cambios. Los tests de `crowding.rs`, que leen `knn_memory` y `total`, pasan (`cargo test -p clawg-core --lib --features eval memory::crowding`).
- No hay consultas nuevas. El SQL de cada brazo es el mismo. Sobre `develop` cambian `search.rs` y documentos: ni migraciones, ni `metrics.db`, ni `agent_contract.json` o `agent_contract.es.json`.

## El ranking

Se corrieron `rag-eval --fake` y `rag-eval --fake --agent-view` dos veces sobre el mismo golden sintético: antes, sobre `d34366c` (el código de `develop`), y después, sobre `b8f2331`. El golden sale de `golden-mine synth` con la base de `crates/clawg-eval/tests/synth_roundtrip.rs`, `--seed 7 --distractors 40`: 776 filas y 110 casos, de los pares `en>en` y `es>es`.

- `rag-eval --diff` no mueve ningún caso en ninguna métrica: retrieval, similarity, la vista del agente y cada par.
- Los JSON de antes y de después son iguales salvo en `git_sha`, `tag` y, en la vista del agente, las latencias de reloj. Los hits de cada caso son idénticos.

### El golden de 46 casos

En Windows x86_64, build debug (`cargo build --locked -p clawg-eval --bin rag-eval`), sobre `docs/eval/golden.json`, que está fuera de git: 46 casos y 48 filas, `content_sha256` `363d3e08…` en las cuatro corridas. `rag-eval --fake` y `rag-eval --fake --agent-view` corrieron antes, sobre `d34366c`, en un worktree temporal, y después, sobre `b100e16`, con el árbol limpio.

- `rag-eval --diff` no mueve ningún caso en ninguna métrica: retrieval, similarity, la vista del agente y los pares `en>en` (7 casos), `es>en` (19) y `es>es` (20). Su veredicto dice «not distinguishable with 0 case(s) moving», porque para concluir pide 6 casos moviéndose en una dirección. Lo que cierra la pregunta es la línea siguiente.
- Los JSON de antes y de después son iguales campo por campo salvo `git_sha`, `tag` y, en la vista del agente, las latencias de reloj (`latency_ms` de cada caso y `latency_p50_ms` y `latency_p99_ms` de cada corte). Los hits de cada caso son idénticos.
- Entre `d34366c` y `b100e16` hay 41 commits que tocan `crates/`, y los de A1 y A2 son dos de ellos. Ninguno mueve un caso.

## Lo que no se midió

- **Una base real.** Falta medir cuánto se desviaban los brazos y cuánto crece `total` con la cola ocupada, ahora que cada brazo vuelve a hacer fila. Lo mide read-connections en su puerta, con el modelo real y un perfil de datos, y eso pide autorización.
