# revision-aware-delete — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. Las columnas y la causa (R1, R2)

- [ ] Test `a_database_from_before_revivals_opens_with_its_supersede_causes` (`db.rs`), `a_revision_records_why_it_replaced` (`worker.rs`), `a_verdict_records_why_it_replaced` (`dispute.rs`).
- [ ] `V17__revival.sql`, `migrations.rs` tocado, checksum fijado; `SUPERSEDE_SQL` con causa; `REVIVE_SQL` la borra.
- Commit: `feat(core): record why a memory was superseded`

## 2. La regla (R3)

- [ ] Test puro `revival_cases`.
- [ ] `revival.rs` con `plan_revival`.
- Commit: `feat(core): decide which superseded rows a delete brings back`

## 3. Borrar revive (R4)

- [ ] Tests de `admin.rs`.
- [ ] `revive_after_delete` en la transacción del borrado; `delete_memory_core` devuelve `Deleted`.
- Commit: `feat(core): bring back the version a deleted revision replaced`

## 4. Restaurar (R5, R6)

- [ ] Tests de `admin.rs`.
- [ ] Dueña de la clave sin las revividas, volver a reemplazar, revivir al restaurar una reemplazada, `topic_key`, `replaced` y `superseded_by_deleted` en `RestoredRow`.
- Commit: `feat(core): restore a deleted revision over the version it brought back`

## 5. Redetección (R7)

- [ ] Tests de `admin.rs`.
- [ ] `detect_contradiction` antes de la transacción, ventana y pares dentro; `related` en `RestoredRow`.
- Commit: `feat(core): pair a restored memory with what was saved while it was deleted`

## 6. Barrido (R8)

- [ ] Test `maintenance_revives_what_an_earlier_delete_left_hidden`.
- [ ] Paso en `run_maintenance_once_at`.
- Commit: `feat(core): revive versions an earlier delete left hidden`

## 7. HTTP, puente y CLI (R9)

- [ ] Tests en `tests/api.rs`, `tools.rs`, `undelete.rs`.
- [ ] `DeletedBody`, `Deleted`, `Restored`, `format_delete`, `format_restore`, `undelete::report`, descripciones; contrato bendecido.
- Commit: `feat(cli): say what a delete brought back and what a restore paired`

## 8. Docs

- [ ] `.claude/CLAUDE.md`.
- Commit: `docs: describe revision-aware delete and restore`

## 9. Verificación

- [ ] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [ ] Lo mismo en Windows, en `target-test`.
- [ ] Perfil dev con el puente MCP real.
- [ ] `verify.md`.

## 10. Archive

- [ ] Specs `restore`, `disputes`, `storage`, `ingest` y `mcp`; `git mv` al archivo; subir el minor.
