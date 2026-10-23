# history-import — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. V23 y el origen en el formato (HI1, HI2)

- [x] Tests `a_database_from_the_previous_version_opens_with_no_origin_project` (`db.rs`), `an_export_carries_the_origin_project` (`export.rs`), `an_import_keeps_the_origin_project` y `an_export_without_an_origin_project_imports` (`import.rs`).
- [x] `V23__origin_project.sql`, el checksum fijado, los conteos de migraciones; `MemoryLine.origin_project`, el export y `INSERT_SQL`.
- Commit: `feat(core): keep the project a memory had in the tool it came from`

## 2. El filtro (HI3)

- [x] Tests de `relocate.rs`, `admin.rs`, `cli.rs` y `tests/api.rs`.
- [x] `MoveFilter.origin_project`, `ListParams.origin_project`; `origin_project` en `POST /v1/memories/move` y `GET /v1/memories`; `--origin-project` en `projects move` y `memories list`.
- Commit: `feat: select memories by the project they had in another tool`

## 3. El modelo común y el adaptador (HI4)

- [ ] Tests de `engram.rs` sobre exports escritos a mano, `0.1.0` y `0.2.0` (listas en `null`, un campo que no conoce, notas reescritas en la fuente), y los de `parse_utc` (`foreign.rs`), con y sin fracción de segundo.
- [ ] `foreign.rs`, `engram.rs` (listas `Option<Vec<_>>`, `extra` con `#[serde(flatten)]` contado por nombre); `days_from_civil` pasa a `pub(crate)`.
- Commit: `feat(core): read an engram export into notes of a common model`

## 4. La traducción y el import de notas nuevas (HI5 a HI10, HI15)

- [ ] Tests de `translate.rs` y `an_imported_note_has_the_rows_of_a_save_of_its_text`, `a_key_held_here_supersedes_the_imported_note`, `a_mapped_root_of_another_project_is_reported_and_skipped` (`import.rs`).
- [ ] `translate.rs` (`translate`, `rewrite_key`, grupos de clave y de texto); `import_foreign` con filas ya enmascaradas, `by_lang` y `ForeignReport`.
- Commit: `feat(core): import notes from another tool split and masked as saves`

## 5. Reconocimiento por origen y revisiones (HI11 a HI13)

- [ ] Tests `a_second_import_adds_nothing_even_with_another_map`, `a_moved_note_is_recognized_where_it_is`, `a_note_deleted_here_is_not_brought_back`, `a_note_rewritten_at_the_source_revises_the_stored_one_where_it_is`, `an_older_version_enters_superseded`, `a_note_deleted_at_the_source_changes_nothing`, `write_batches_never_split_a_note` y el de vínculos entre transacciones (`import.rs`).
- [ ] La decisión por origen en `place`, `note_batches`, la regla de piezas por origen.
- Commit: `feat(core): recognize imported notes by their origin and take in their rewrites`

## 6. Borradas y relaciones (HI14)

- [ ] Tests `relations_map_to_pair_states` (`translate.rs`) y los de `import.rs`.
- [ ] `pair_for`, las líneas `conflict` y los estados de las filas; `write_links` escribe cada par como `(mayor, menor)`.
- Commit: `feat(core): import the relations between notes as conflict pairs`

## 7. La ruta y el comando (HI16, HI17, HI18)

- [ ] Tests de `tests/api.rs`, `cli.rs` e `import.rs` del CLI.
- [ ] `ImportRequest.from` y `project_map`, el `503` sin tokenizer; `--from` y `--project-map`; `render` en los dos idiomas.
- Commit: `feat(cli): import a memory export from another tool`

## 8. El binario real (HI16)

- [ ] `an_engram_export_imports_through_the_binary` (`tests/binary.rs`).
- Commit: `test(cli): import an engram export through a real daemon`

## 9. Docs

- [ ] `.claude/CLAUDE.md`: el comando, la API, la columna, el filtro y cómo exportar todo (`engram export --all`) fuera del repo.
- Commit: `docs: describe importing memories from another tool`

## 10. Verificación

- [ ] `cargo test --workspace --target-dir target-test` y el clippy de `.claude/CLAUDE.md` en Windows.
- [ ] Linux: queda al CI del pull request.
- [ ] El binario real en un home temporal.
- [ ] Perfil dev, con autorización: el export real (`engram export --all`) con `--dry-run`, el import, una docena de búsquedas, cuánto tarda, si el daemon contesta mientras tanto, y `projects move --origin-project`.

## 11. Archive

- [ ] Las specs de la propuesta; `git mv` al archivo; subir el minor.
