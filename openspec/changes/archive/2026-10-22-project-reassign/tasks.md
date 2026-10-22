# project-reassign — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. V21 y `Reason::PreMove` (PR12)

- [x] Test `a_database_from_the_previous_version_opens_with_an_empty_move_log` (`db.rs`): siembra V20 y falla porque `memory_move` no existe.
- [x] `V21__memory_move.sql`, el checksum fijado, los conteos de migraciones. `Reason::PreMove` pasó a la tarea 2, su primer uso.
- Commit: `feat(core): log every memory moved between projects`

## 2. El motor (PR1–PR4, PR6, PR7)

- [x] Tests puros de `decide` y `scope_after`, y los de `relocate.rs` sobre `MemoryStore::new_for_test()`.
- [x] `relocate.rs`: leer, decidir, aplicar; `move_memories_core` y `undo_move_core` con el snapshot.
- Commit: junto con la tarea 3, porque `plan_one_in` sólo tiene uso desde `update_memory_core` y un commit sin él dejaba código muerto.

## 3. `PATCH` y el listado sin proyecto (PR5, PR8, PR10)

- [x] Tests de `admin.rs`, `api.rs` y `tests/api.rs`.
- [x] `MemoryPatch` con destino, `update_memory_core` con el movimiento, `UpdatedRow.project_id`; `UpdateRequest` con `project_id` y `no_project`; `ListParams.no_project` y `ListQuery.no_project`.
- Commit: `feat: move a memory with its save to another project through PATCH`

## 4. El lote (PR9)

- [x] Tests de `tests/api.rs` y `cli.rs`.
- [x] `POST /v1/memories/move`; `clawg projects move`; `--no-project` en `memories list|search`.
- Commit: `feat(cli): move memories between projects in bulk and undo it`

## 5. La herramienta (PR11)

- [x] Tests de `server.rs` y `tools.rs`.
- [x] `to_project_path` y `clear_project`, `format_update`, `protocol.md`; el contrato se bendice y el diff se revisa.
- Commit: `feat(mcp): let update_memory move a memory to another project`

## 6. El binario real (PR9)

- [x] `projects_move_moves_and_undoes_a_seeded_store` (`tests/binary.rs`).
- Commit: `test(cli): move and undo through a real daemon`

## 7. Docs

- [x] `.claude/CLAUDE.md`.
- Commit: `docs: describe moving memories between projects`

## 8. Verificación

- [x] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [ ] Windows: queda al CI del pull request ([verify](verify.md)).
- [x] El binario real en un home temporal ([verify](verify.md)).
- [ ] Perfil dev, con autorización (no se corrió): después de history-import (44), `projects move --dry-run`, mover, buscar desde el destino y desde otro proyecto, medir cuánto tarda el lote y `--undo`.

## 9. Archive

- [x] Las specs de la propuesta; `git mv` al archivo; subir el minor.
