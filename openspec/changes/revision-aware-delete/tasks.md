# revision-aware-delete — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. Las columnas y la causa (R1, R2)

- [x] Test `a_database_from_before_revivals_opens_with_its_supersede_causes` (`db.rs`), `a_revision_records_why_it_replaced` (`worker.rs`), `a_verdict_records_why_it_replaced` (`dispute.rs`).
- [x] `V17__revival.sql`, `migrations.rs` tocado, checksum fijado; `SUPERSEDE_SQL` con causa; `REVIVE_SQL` la borra.
- Commit: `feat(core): record why a memory was superseded`

## 2–3. La regla y el borrado (R3, R4)

Un solo commit: la regla sola sería código muerto, y las advertencias son errores.

- [x] Test puro `revival_cases` y los de `admin.rs`.
- [x] `revival.rs` con `plan_revival` y `revive_after_delete` en la transacción del borrado; `delete_memory_core` devuelve `Deleted`.
- Commit: `feat(core): bring back the version a deleted revision replaced`
- [x] Hallazgo de la verificación: borrar la última pieza de un save largo aplica la regla a su primera pieza. Commit: `fix(core): revive the version a long revision replaced when its last piece is deleted`

## 4. Restaurar (R5, R6)

- [x] Tests de `admin.rs`.
- [x] Dueña de la clave sin las revividas, volver a reemplazar, revivir al restaurar una reemplazada, `topic_key`, `replaced` y `superseded_by_deleted` en `RestoredRow`.
- Commit: `feat(core): restore a deleted revision over the version it brought back`

## 5. Redetección (R7)

- [x] Tests de `admin.rs`.
- [x] `detect_contradiction` antes de la transacción, ventana y pares dentro; `related` en `RestoredRow`.
- Commit: `feat(core): pair a restored memory with what was saved while it was deleted`

## 6. Barrido (R8)

- [x] Test `maintenance_revives_what_an_earlier_delete_left_hidden`.
- [x] Paso en `run_maintenance_once_at`.
- Commit: `feat(core): revive versions an earlier delete left hidden`

## 7. HTTP, puente y CLI (R9)

- [x] Tests en `tests/api.rs`, `tools.rs`, `undelete.rs`.
- [x] `DeletedBody`, `Deleted`, `Restored`, `format_delete`, `format_restore`, `undelete::report`, descripciones; contrato bendecido.
- Commit: `feat(cli): say what a delete brought back and what a restore paired`

## 8. Docs

- [x] `.claude/CLAUDE.md`.
- Commit: `docs: describe revision-aware delete and restore`

## 9. Verificación

- [x] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [x] Lo mismo en Windows, en `target-test`.
- [x] Perfil dev con el puente MCP real. (El binario real ya se probó en un home temporal: [verify](verify.md#binario-real-home-temporal).)
- [x] V17 a través de `clawg update`: test `an_update_over_a_store_without_supersede_causes_keeps_its_rows_and_tells_revisions_from_verdicts` (`binary.rs`) y una base real de la release anterior. Commit: `test(cli): update over a store without supersede causes`
- [x] `verify.md`.

## 10. Archive

- [ ] Specs `restore`, `disputes`, `storage`, `ingest` y `mcp`; `git mv` al archivo; subir el minor.
