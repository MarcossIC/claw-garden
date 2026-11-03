# search-arm-timing — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción. Una tarea, un commit convencional, comprobado con los tests de lo que toca; la suite y los dos clippy corren una vez, en el último commit.

## 0. Antes del código

- [x] `rag-eval --fake` y `rag-eval --fake --agent-view` sobre un golden, con el código de antes. Sin `docs/eval/golden.json` (fuera de git), uno sintético: `golden-mine synth` con la base de `crates/clawg-eval/tests/synth_roundtrip.rs`, `--seed 7 --distractors 40`. Los resultados quedan fuera del repositorio.

## 1. Cada brazo mide su ejecución en el hilo de la conexión (A1)

- [x] Test primero: `an_arm_measures_its_run_on_the_connection_not_its_wait` (`crates/clawg-core/src/memory/search.rs`), en rojo porque los brazos incluyen la retención.
- [x] `KnnOutcome::ran`, tomado dentro de la clausura de `knn_one`. `identifier_matches`, `lexical_candidates` y `score_lexical_candidates` devuelven su duración, por una función que corre su consulta con `call_raw` y la cronometra en el hilo. `record_arm` toma una duración.
- [x] `search_all_memories_inner_fails_soft_when_memory_fts_is_missing` comprueba además que el canal léxico que falló registra su brazo (pasa antes y después: fija lo que no cambia).
- Commit: `fix(search): time each arm on the connection's thread, without its wait in the queue`

## 2. Los brazos esperan su turno uno detrás de otro (A2)

- [x] Test primero: `a_call_that_arrives_during_a_search_runs_between_its_arms` (`search.rs`), en rojo porque `try_join!` encola los brazos juntos.
- [x] Esperas en orden en lugar de `try_join!`, con el mismo orden de brazos; los comentarios que hablaban del `try_join!`.
- Commit: `fix(search): send the arms to the connection one after another`

## 3. Docs (A3)

- [x] `docs/reference/data-profiles.md` y R14 de `openspec/specs/metrics/spec.md`: qué mide un brazo, que `total` sigue de punta a punta, y que hasta 0.72.2 cada brazo incluía la espera en la cola, que `--by version` separa.
- Commit: `docs: say what each search arm measures and since which version`

## 4. Cierre

- [x] Suite y los dos clippy sobre el último commit, en Linux; Windows en el CI del pull request.
- [x] `rag-eval --fake` y `--agent-view` con el código nuevo sobre el golden de la tarea 0, y `rag-eval --diff` contra los de antes: ningún caso se mueve.
- [x] `verify.md`, `state.yaml` y la fila 101 de `openspec/README.md`.
- Commit: `docs(openspec): record the search arm timing as applied and verified`

## Al archivar (fuera de este cambio)

- `rag-eval --fake --diff` sobre el golden de 46 casos (`docs/eval/golden.json`, fuera de git), en una máquina que lo tenga.

- Si la versión que lo publica no es la siguiente a 0.73.0, corregir «hasta 0.73.0» en `docs/reference/data-profiles.md` y en R14 de `specs/metrics`.
- La línea de origen de `specs/metrics` y la fila 101 de `openspec/README.md` con la versión.
