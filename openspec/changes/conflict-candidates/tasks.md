# conflict-candidates — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. La tabla de pares (C9)

- [x] Test `a_database_with_disputes_opens_with_escalated_pairs` (`db.rs`): siembra V14 y falla porque la tabla no existe.
- [x] `V15__memory_conflict.sql` con la conversión, `migrations.rs` tocado y el checksum fijado.
- Commit: `feat(core): record dispute pairs in memory_conflict`

## 2. Candidatas y escalamiento (C1–C3)

- [x] Test puro `needs_user_cases` y tests del insert y del worker.
- [x] `detect_contradiction` devuelve `Candidate` (id, distancia, save, escalado, extracto). `NewMemoryRecord.candidates` reemplaza a `disputed_against`. Se borra la tabla CLS del comentario del umbral. El worker colapsa por save.
- Commit: `feat(core): note close rows as conflict candidates and escalate only the risky ones`

## 3. Veredictos tipados (C4)

- [x] Tests de `dispute.rs`.
- [x] `Verdict`, `judge_pair`, `open_pairs`; `resolve_dispute_core` sobre `judge_pair`.
- Commit: `feat(core): judge conflict pairs with typed verdicts`

## 4. Borrar, restaurar, purgar, mantener (C8)

- [x] Tests en `admin.rs`, `secrets.rs` y `maintenance.rs`.
- [x] `settle_challengers` al restaurar, el borrado en purge y el de huérfanos en mantenimiento.
- Commit: `feat(core): keep conflict pairs through restore, purge and maintenance`

## 5. HTTP, puente y textos (C5–C7)

- [x] Tests en `tests/api.rs`, `server.rs` y `prompt_builder.rs`.
- [x] Las rutas `/v1/disputes/judge` y `GET /v1/disputes`, `related` en el save, `resolve_conflict` con `verdict` y `memory_status`. `CONFLICT_NOTE` y `protocol.md`. El contrato se bendice y el diff se revisa.
- Commit: `feat(cli): judge conflicts with verdicts and show candidates on save`

## 6. Medición (C10)

- [x] Tests de `conflict-eval` y `conflict_pairs_reads_the_table`.
- [x] Bin `conflict-eval` y set etiquetado `crates/clawg-eval/data/conflict-pairs.json`. El flujo son las oraciones con MUST de `openspec/specs` (más de 200), leídas al correr.
- Commit: `feat(eval): measure conflict candidates per class and per save`

## 7. Docs y limpieza

- [x] Se actualiza `.claude/CLAUDE.md`. La tabla CLS de `contradiction.rs` salió en la tarea 2, que reescribió ese comentario.
- Commit: `docs: describe conflict candidates and verdicts`

## 8. Verificación

- [ ] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [ ] Lo mismo en Windows, en `target-test`.
- [ ] `conflict-eval --real` y `rag-eval --real --diff`, con el modelo real y `docs/eval/`.
- [ ] `verify.md`.

## 9. Archive

- [ ] Specs `disputes`, `mcp`, `retrieval` y `storage`; `git mv` al archivo; subir el minor.
