# protected-verdicts — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. La guarda de los veredictos (R1, R2, D1, D2)

- [x] Tests primero: `a_protected_row_is_not_superseded_after_conflicts` (reemplaza a `a_protected_pending_pair_is_not_superseded_without_the_user`), `a_pair_escalated_at_save_does_not_open_the_guard`, `a_judged_pair_does_not_open_the_guard`, `a_pair_without_a_row_does_not_open_the_guard`, `a_protected_keep_needs_no_answer`, `a_declared_answer_supersedes_and_is_recorded`, `a_declared_answer_needs_a_reason`.
- [x] `judge_pair` con `user_answered`; `NeedsUser` sobre `other` en todo estado; `AnswerRequired`; el `reason` marcado. `MemoryType::is_protected`. El daemon toma el campo y responde `400` a `AnswerRequired`; `a_protected_verdict_without_the_answer_is_a_409` en `tests/api.rs`.
- Commit: `feat(core): supersede a preference or a decision only on the user's declared answer`

## 2. El descenso de tipo (R3, D3)

- [x] Tests primero: `demoting_a_protected_row_needs_the_answer`, `promoting_or_changing_importance_needs_no_answer`, `a_demotion_without_the_answer_is_a_409`.
- [x] `MemoryPatch::answered`, `UpdateError`, el chequeo en `update_memory_core`; `user_answered` en `UpdateRequest` y el `409`.
- Commit: `feat(core): demote a preference or a decision only on the user's declared answer`

## 3. El puente (R4, D4)

- [x] Tests primero: `resolve_conflict_sends_the_users_answer`, `update_memory_sends_the_users_answer`.
- [x] `user_answered` en `ResolveConflictArgs`, `UpdateMemoryArgs`, `client.judge` y `client.update`; descripciones; `protocol.md`; contrato bendecido.
- Commit: `feat(mcp): let the agent declare the user's answer to a protected verdict`

## 4. Docs

- [x] `.claude/CLAUDE.md`, el mapa de openspec y estas tareas.
- Commit: `docs: say a preference or a decision is replaced only on a declared answer`

## 5. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [x] El CI en verde en Linux y Windows (run 39408972625 sobre `b4ea6af`).

## 6. Archive

- [x] `specs/disputes` (R3 y `conflicts`), `specs/mcp` (`resolve_conflict`), `specs/update` (R1, R4, R5); `git mv` a `changes/archive/2026-10-20-protected-verdicts`; `state.yaml`; fila 63 y el orden de ejecución del `README.md` de openspec; subir el minor.
