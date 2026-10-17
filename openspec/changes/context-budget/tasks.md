# context-budget — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El resultado de la búsqueda sale de `build_memory_prompt` (R7, D5)

- [x] Test primero: `the_retrieval_outcome_names_each_silence` (sin embedder, `embed` que falla, búsqueda que funciona): no compila.
- [x] `MemoryPrompt { text, retrieval }` y `Retrieval`; los llamadores leen `.text`.
- Commit: `feat(core): say whether the memory prompt searched memories`

## 2. El daemon y el puente avisan (R7)

- [x] Tests primero: `context_says_whether_memories_were_searched` (`tests/api.rs`), `a_context_without_retrieval_still_parses` (`client.rs`), `recall_says_when_memories_were_not_searched` y `an_old_daemon_gets_no_invented_notice` (`tools.rs`).
- [x] `retrieval` en `ContextBody` desde `embedder_status`; `Context::retrieval`; la frase en `format_recall`.
- Commit: `feat(cli): tell the agent when recall_context could not search memories`

## 3. El presupuesto (R1 a R5, D1 a D4)

- [x] Tests primero en `prompt_builder.rs`: los de R1 a R5.
- [x] El ajuste puro, el bloque `BUDGET`, las posiciones de conflictos sobre lo impreso y los accesos de lo impreso; `build_memory_prompt` recibe el presupuesto.
- Commit: `feat(core): fit the memory prompt to a byte budget, cutting by priority`

## 4. `budget` en la API y la herramienta (R6)

- [x] Tests primero: `context_takes_a_budget_and_clamps_it`, `context_refuses_a_zero_budget` (`tests/api.rs`), `recall_context_passes_its_budget` y `a_hit_cut_by_the_budget_is_readable_whole` (`server.rs`).
- [x] `ContextRequest::budget`, `DaemonClient::context` con `budget`, `RecallContextArgs::budget`, la descripción de `recall_context`; `CLAWG_BLESS=1` del contrato del agente y revisión del diff.
- Commit: `feat(cli): let recall_context take a byte budget`

## 5. Docs

- [x] `.claude/CLAUDE.md` (API HTTP y herramienta), el mapa de `openspec/README.md` y este cambio.
- Commit: `docs: describe the recall_context budget and its retrieval notice`

## 6. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.
- [ ] `rag-eval --real --agent-view` antes y después (necesita el modelo real y `docs/eval/golden.json`): `found` de `recall_context` 39/46 y tokens sin aumento.
