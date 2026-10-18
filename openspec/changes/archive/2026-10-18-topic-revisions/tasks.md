# topic-revisions — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. La columna (T1)

- [x] Test `a_database_from_before_topic_keys_opens_unchanged` (`db.rs`): siembra V15 y falla porque la columna no existe.
- [x] `V16__topic_key.sql`, `migrations.rs` tocado y el checksum fijado.
- Commit: `feat(core): add a topic key column to memory`

## 2. Formato (T2)

- [x] Test puro `topic_key_cases`.
- [x] `topic.rs` con `validate_topic_key` y `TopicKeyError`.
- Commit: `feat(core): validate topic keys`

## 3. Revisión en el núcleo (T3–T8)

- [x] Tests de `worker.rs` y `events.rs`.
- [x] `MemoryEvent.topic_key`, `NewMemoryRecord.topic_key`, la lectura del alcance, la herencia, los `siblings`, el reemplazo al terminar el save y `IngestReport.replaced`.
- Commit: `feat(core): replace the live memory of a topic key on save`

## 4. Restaurar (T9)

- [x] Test `a_row_whose_key_is_live_again_is_not_restored`.
- [x] `RestoreError::TopicTaken` y su comprobación.
- Commit: `feat(core): refuse to restore a row whose topic key is live again`

## 5. HTTP, puente y textos (T3, T9, T10)

- [x] Tests en `tests/api.rs`, `server.rs` y `tools.rs`.
- [x] `topic_key` en `SaveRequest`, `SaveBody`, `SaveArgs` y `SaveMemoryArgs`; `replaced` en la respuesta y en el texto; el aviso de daemon anterior; `409` al restaurar; `topic=` y `by=` en el listado; `protocol.md`. El contrato se bendice y el diff se revisa.
- Commit: `feat(cli): revise a memory by topic key from save_memory`

## 6. Docs

- [x] `.claude/CLAUDE.md`.
- Commit: `docs: describe topic key revisions`

## 7. Verificación

- [x] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [ ] Lo mismo en Windows, en `target-test`.
- [ ] Compuerta previa y `rag-eval --real --diff` contra el ancla, con el modelo real y `docs/eval/`.
- [x] `verify.md` (con lo pendiente).
- [ ] Perfil dev con Claude Code.

## 8. Archive

- [ ] Specs `ingest`, `pieces`, `update`, `disputes`, `storage` y `mcp`; `git mv` al archivo; subir el minor.
