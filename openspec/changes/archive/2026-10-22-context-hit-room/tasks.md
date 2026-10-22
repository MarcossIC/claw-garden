# context-hit-room — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Los hits de una consulta tienen lugar en el contexto (R1, R2, R3; D1-D3)

- [x] Tests primero: `the_hits_over_their_room_go_before_the_last_session`, `the_hits_in_their_room_outlast_the_last_session_and_the_conflicts`, `a_lone_hit_over_its_room_is_shortened_to_it`, `a_room_never_takes_the_prompt_over_the_budget`, `a_mature_store_still_shows_the_hit_of_the_query` y `a_context_without_a_query_keeps_the_last_session_over_its_saves` (`prompt_builder.rs`).
- [x] `PromptParts::hit_room` y `hit_bytes`; `fit_to_budget` corta lo que pasa de la reserva, después LAST SESSION, los conflictos y el resto de los hits; `build_memory_prompt` reserva `budget / 4` y `build_session_prompt` nada.
- Commit: `fix(context): keep a quarter of the budget for the hits of the query`

## 2. Docs

- [x] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say a context keeps a quarter of its budget for its hits`

## 3. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.
- [ ] **[modelo real, export del store]** El `recall_context` real que cortaba los 10 hits, y `revision-eval chains --real --background`: la última versión de cada cadena tiene que aparecer en `recall_context`.

## 4. Archive

- [x] `specs/retrieval` con R1, R2 y R3 y las citas de `prompt_builder.rs` realineadas en todas las specs, y la nota de la corrida con fondo en `specs/eval`; `git mv` a `changes/archive/2026-10-22-context-hit-room`; `state.yaml` archivado; fila 75 y el orden de ejecución del `README.md` de openspec; subir el minor.
