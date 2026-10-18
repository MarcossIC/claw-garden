# agent-lifecycle-hooks — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El contexto sin consulta en el core (R1, D2)

- [x] Tests primero en `prompt_builder.rs`: `a_context_without_a_query_lists_the_recent_saves_by_date`, `a_context_without_a_query_shows_a_long_save_once_by_its_opening`, `a_context_without_a_query_reads_only_its_own_agent`: no compilan.
- [x] `build_session_prompt` y `Retrieval::Recent`; el armado de conflictos, presupuesto y render compartido con `build_memory_prompt`.
- Commit: `feat(core): build a memory prompt without a query from the recent saves`

## 2. `query` opcional en `POST /v1/context` (R1)

- [x] Test primero: `context_without_a_query_does_not_need_the_embedder` (`tests/api.rs`).
- [x] `ContextRequest::query` opcional, en blanco como ausente; `RetrievalState::Recent`.
- Commit: `feat(cli): answer a context request without a query from the recent saves`

## 3. `clawg hook` (R2 a R5, D1, D3 a D6)

- [x] Tests primero: los de `cli.rs` y `hook.rs` de R2 a R5.
- [x] `Command::Hook`, el módulo `hook`, `DaemonClient::recent_context`, la salida de `main` sin 2.
- Commit: `feat(cli): add clawg hook for the client's session-start and user-prompt-submit`

## 4. El binario real (R3, R4)

- [x] Tests en `tests/binary.rs`: `a_hook_without_a_daemon_prints_nothing`, `a_hook_with_input_that_is_not_json_prints_nothing`, `a_hook_usage_error_exits_1`, `session_start_injects_the_saved_memories`.
- Commit: `test(cli): run clawg hook against a real daemon`

## 5. Docs

- [x] `.claude/CLAUDE.md` (comando y API), el mapa de `openspec/README.md`, `state.yaml` y la configuración manual para probar.
- Commit: `docs: describe clawg hook and the context without a query`

## 6. Verificación

- [x] Suite y clippy en Linux sobre el último commit.
- [ ] El CI en verde en Linux y Windows.
- [ ] Con Claude Code sobre el perfil dev, con la configuración manual: el inicio de sesión trae el contexto, `/compact` lo vuelve a traer con la línea de compactación, y el recordatorio sale después de 30 minutos sin guardar.
