# memory-portability — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El formato (R1, R6)

- [x] Tests: `the_pinned_example_reads_and_writes_back_unchanged`, `a_header_of_another_format_or_version_is_refused`, `invalid_lines_are_reported_and_the_rest_imports` (`portability/format.rs`).
- [x] Tipos de línea, `parse` y `render` puros.
- Commit: `feat(core): define the clawg-memories export format`

## 2. El export (R2)

- [x] Tests: `export_filters_by_project_agent_and_deleted`, `links_leave_only_with_the_rows_that_hold_them`, `every_column_travels`, `a_store_from_v12_or_v16_exports_with_null_for_what_it_lacks`, `a_newer_store_is_not_exported` (`portability/export.rs`); `markdown_groups_rows_by_project_and_type` (`portability/markdown.rs`).
- [x] `export` sobre una conexión de sólo lectura y `markdown::render`.
- Commit: `feat(core): export memories by row, as JSON Lines or Markdown`

## 3. El import en el núcleo (R3–R7)

- [x] Tests: los de `portability/import.rs` que nombra la spec.
- [x] `Reason::PreImport`; `import` en tres pasadas (D2).
- Commit: `feat(core): import exported memories with their states and links`
- [x] Lo que encontró el repaso del diff: el `context` de una pieza se enmascara como el texto. Test: `imported_text_is_masked_and_indexed_once`.
- Commit: `fix(core): mask the head of an imported save as its text`

## 4. La ruta y los comandos (R2, R3, R7)

- [x] Tests: `an_import_stores_rows_with_their_state_and_a_dry_run_stores_none`, `import_answers_400_for_an_unknown_version_before_the_embedder` (`tests/api.rs`); `export_refuses_an_existing_file_and_writes_owner_only` y el reporte (`export.rs`, `import.rs` del CLI); parseo de los argumentos (`cli.rs`); `export_and_import_round_trip_through_the_binary` (`tests/binary.rs`).
- [x] `POST /v1/import`, `DaemonClient::import`, `clawg export` y `clawg import`.
- Commit: `feat(cli): add clawg export and clawg import`

## 5. Docs

- [x] `.claude/CLAUDE.md` (comandos y API), el `README.md` y estas tareas.
- Commit: `docs: describe clawg export and clawg import`

## 6. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 7. Archive

- [x] `specs/portability` con R1 a R7 y dónde está cada parte en el código; `git mv` a `changes/archive/2026-10-20-memory-portability`; `state.yaml` archivado; fila 43, su lugar en el orden de ejecución y la idea evaluada del `README.md` de openspec; subir el minor.
