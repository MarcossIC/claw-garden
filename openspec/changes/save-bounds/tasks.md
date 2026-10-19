# save-bounds — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El tope del contenido (R1, D1)

- [ ] Tests primero: `content_is_checked_in_bytes` (`api.rs`); `a_save_over_the_content_limit_is_refused_before_the_outbox` y `a_save_at_the_content_limit_is_stored` (`tests/api.rs`).
- [ ] `MAX_CONTENT_BYTES` y `check_content` en `save_memory`.
- Commit: `feat(daemon): refuse a save whose content is over 64 KiB`

## 2. El agente conoce el tope (R2)

- [ ] Test primero: `save_memory_says_a_save_over_the_limit_was_not_saved` (`server.rs`).
- [ ] La descripción de `content` en `SaveMemoryArgs`, la línea de `protocol.md` y `agent_contract.json` bendecido.
- Commit: `feat(mcp): tell the agent how long a save may be`

## 3. Docs

- [ ] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say how long a save may be`

## 4. Verificación

- [ ] Suite y clippy en Linux sobre el último commit, y cada commit de código por separado ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 5. Archive

- [ ] `specs/ingest` con R1 y R2, el `README.md` de openspec (fila 54, hallazgos, backlog y orden), `git mv` a `changes/archive/`, `state.yaml` archivado, y subir el minor.
