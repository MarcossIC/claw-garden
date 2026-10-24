# import-undo — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. El esquema (IU6)

- [ ] Tests primero: `a_store_from_before_the_import_journal_keeps_its_rows` (`db.rs`), la fila 26 en `published_migration_checksums_are_pinned`.
- [ ] V26 con `memory_import`, `memory_import_row`, `memory_import_supersede` y `memory_import_extra`.
- [ ] `Reason::PreUndo` (`pre-undo`), cubierto por `snapshot_names_parse_back_to_their_reason`.

## 2. El diario (IU1)

- [ ] Tests primero (`import.rs`): `an_import_is_numbered_and_journaled`, `a_dry_run_and_an_import_that_adds_nothing_have_no_number`.
- [ ] `Journal` en `SqlSeat`, `record_projects` y la transacción de los vínculos; `import_id` en `ImportReport`; el registro de imports en curso en `MemoryStore`.
- [ ] El oráculo de import (`import.rs`, `tests/api.rs`, `tests/binary.rs`) pasa; sólo cambian las comparaciones entre un dry run y el real, que suman `import_id` a lo que difiere.

## 3. Deshacer (IU2, IU3, IU4)

- [ ] Tests primero (`undo.rs`): `decide_keeps_what_changed_and_what_is_named` y los de IU2 a IU4.
- [ ] `decide` pura, la carga en lotes y la escritura en transacciones de `WRITE_BATCH` filas con la nueva lectura del criterio.
- [ ] `undo_import` con `UndoReport`, `UndoError` y el snapshot `pre-undo`.

## 4. Las puertas (IU5)

- [ ] Tests primero: `import_takes_a_file_or_an_undo` (`cli.rs`), el render en los dos idiomas, el flujo en `tests/api.rs`.
- [ ] `POST /v1/import/undo`, `DaemonClient::undo_import` y `clawg import --undo N [--dry-run]`; la línea del número en la salida del import.

## 5. Docs

- [ ] `.claude/CLAUDE.md` (`clawg import`, la API y el diario), la fila 84 de `openspec/README.md`. El delta de `specs/portability` y `specs/storage` va con el archivo.

## 6. Verificación

- [ ] Suite y clippy de `.claude/CLAUDE.md` en Linux.
- [ ] El banco de 81 con el diario y el tiempo de `--undo` de 20.000 notas, en `verify.md`.
- [ ] Suite y clippy en Windows (`target-test`).
- [ ] El CI en verde en Linux y Windows.

## 7. Archive

- [ ] `specs/portability` y `specs/storage` con IU1 a IU6; `git mv` a `changes/archive/AAAA-MM-DD-import-undo`; `state.yaml` archivado; la fila 84 y el orden de ejecución del `README.md` de openspec; subir el minor.
