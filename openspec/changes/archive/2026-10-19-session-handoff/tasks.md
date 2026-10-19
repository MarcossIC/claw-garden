# session-handoff — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. La sesión en el evento y en la fila (R2, D2)

- [x] Tests primero: `an_event_without_a_session_reads_as_none` (`events.rs`), `a_save_writes_its_session_on_every_row` (`worker.rs`, que trocea el save): no compilan.
- [x] `MemoryEvent::session_id`, `NewMemoryRecord::source_session_id` y el `INSERT`.
- Commit: `feat(core): write the session a save came from on its rows`

## 2. El bloque LAST SESSION (R3, R4, D3, D4)

- [x] Tests primero en `prompt_builder.rs`: `the_last_session_block_shows_the_newest_summary`, `the_last_session_block_respects_sharing`, `the_last_session_block_is_not_repeated_as_a_hit`, `a_context_without_a_query_opens_with_the_last_session`, `the_last_session_block_gives_way_after_the_hits`; `a_reach_admits_the_reader_and_whoever_is_not_hidden` (`sharing.rs`).
- [x] La consulta del último resumen, el bloque en `render` y en `fit_to_budget`; `build_session_prompt` recibe el `Reach`.
- Commit: `feat(core): open the memory prompt with the last session's summary`

## 3. La evaluación lee el bloque (R6, D5)

- [x] Test primero: `the_last_session_block_counts_as_shown` (`agent_view/parse.rs`).
- [x] `context_segments` lee LAST SESSION.
- Commit: `feat(eval): count the last session block as shown in the agent view`

## 4. El registro de sesiones y el sello en el save (R1, R2, D1)

- [x] Tests primero: `a_session_registry_keeps_the_latest_per_agent_and_project`, `a_session_id_is_checked` (`daemon/sessions.rs`), `a_save_carries_the_registered_session`, `a_queued_save_keeps_its_session_in_the_outbox`, `rows_without_a_session_stay_reachable` (`tests/api.rs`).
- [x] `daemon/sessions.rs`, `POST /v1/sessions`, el sello en `save_memory`.
- Commit: `feat(cli): register the host's session and stamp it on saves`

## 5. Los hooks registran la sesión (R5)

- [x] Tests primero: `the_hook_input_tolerates_unknown_and_missing_fields` con `session_id` (`hook.rs`), `a_hook_registers_the_session_of_the_saves` (`tests/binary.rs`).
- [x] `HookInput::session_id`, `DaemonClient::register_session`, el registro en los dos eventos.
- Commit: `feat(cli): register the session from the client's hooks`

## 6. Docs

- [x] `protocol.md` (fuera de la apertura), `.claude/CLAUDE.md`, el mapa de `openspec/README.md` y `state.yaml`.
- Commit: `docs: describe the session registry and the last session block`

## 7. Verificación

- [x] Suite y clippy en Linux sobre el último commit.
- [x] El CI en verde en Linux y Windows.
- [x] `rag-eval --real --agent-view` contra la línea base, con el umbral de la propuesta (pide el modelo real y el golden set de `docs/eval/`).
- [x] Con Claude Code sobre el perfil dev, «Probar a mano» del diseño.
