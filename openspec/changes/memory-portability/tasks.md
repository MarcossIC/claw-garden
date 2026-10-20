# memory-portability — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El formato (R1, R6)

- [ ] Tests primero: `the_pinned_example_reads_and_writes_back_unchanged`, `a_header_of_another_format_or_version_is_refused`, `invalid_lines_are_reported_and_the_rest_imports` (`portability/format.rs`).
- [ ] Tipos de línea, `parse` y `render` puros.
- Commit: `feat(core): define the clawg-memories export format`

## 2. El export (R2)

- [ ] Tests primero: `export_filters_by_project_agent_and_deleted`, `links_leave_only_with_the_rows_that_hold_them`, `a_store_from_v16_exports_with_null_for_what_it_lacks`, `a_newer_store_is_not_exported` (`portability/export.rs`); `markdown_groups_rows_by_project_and_type` (`portability/markdown.rs`).
- [ ] `export` sobre una conexión de sólo lectura y `markdown::render`.
- Commit: `feat(core): export memories by row`

## 3. El import en el núcleo (R3–R7)

- [ ] Tests primero: los de `portability/import.rs` que nombra la spec.
- [ ] `Reason::PreImport`; `import` en tres pasadas (D2).
- Commit: `feat(core): import exported memories with their states and links`

## 4. La ruta y los comandos (R2, R3, R7)

- [ ] Tests primero: `import_needs_a_running_daemon`, `import_answers_400_for_an_unknown_version` (`tests/api.rs`); `export_refuses_an_existing_file_and_writes_owner_only` y el reporte (`export.rs`, `import.rs` del CLI); parseo de los argumentos (`cli.rs`); `export_and_import_round_trip_through_the_binary` (`tests/binary.rs`).
- [ ] `POST /v1/import`, `DaemonClient::import`, `clawg export` y `clawg import`.
- Commit: `feat(cli): add clawg export and clawg import`

## 5. Docs

- [ ] `.claude/CLAUDE.md` (comandos y API) y estas tareas.
- Commit: `docs: describe clawg export and clawg import`

## 6. Verificación

- [ ] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.
