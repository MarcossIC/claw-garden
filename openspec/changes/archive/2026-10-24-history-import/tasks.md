# history-import — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. V23 y el origen en el formato (HI1, HI2)

- [x] Tests `a_database_from_the_previous_version_opens_with_no_origin_project` (`db.rs`), `an_export_carries_the_origin_project` (`export.rs`), `an_import_keeps_the_origin_project` y `an_export_without_an_origin_project_imports` (`import.rs`).
- [x] `V23__origin_project.sql`, el checksum fijado, los conteos de migraciones; `MemoryLine.origin_project`, el export y `INSERT_SQL`.
- Commit: `feat(core): keep the project a memory had in the tool it came from` (`5f06bf4`)

## 2. El filtro (HI3)

- [x] Tests de `relocate.rs`, `admin.rs`, `cli.rs` y `tests/api.rs`.
- [x] `MoveFilter.origin_project`, `ListParams.origin_project`; `origin_project` en `POST /v1/memories/move` y `GET /v1/memories`; `--origin-project` en `projects move` y `memories list`.
- Commit: `feat: select memories by the project they had in another tool` (`25a6961`)

## 3. El modelo común y el adaptador (HI4)

- [x] Tests de `engram.rs` sobre exports escritos a mano, `0.1.0` y `0.2.0` (listas en `null`, un campo que no conoce, notas reescritas en la fuente), y los de `parse_utc` (`foreign.rs`), con y sin fracción de segundo.
- [x] `foreign.rs`, `engram.rs` (listas `Option<Vec<_>>`, `extra` con `#[serde(flatten)]` contado por nombre); `days_from_civil` pasa a `pub(crate)`.
- Commit: `feat(core): read an engram export into notes of a common model` (`7cec798`)

## 4. La traducción y el import de notas nuevas (HI5 a HI10, HI15)

- [x] Tests de `translate.rs` y `an_imported_note_has_the_rows_of_a_save_of_its_text`, `a_key_held_here_supersedes_the_imported_note`, `a_mapped_root_whose_id_is_taken_gets_its_own`, `notes_mapped_to_clones_of_one_remote_share_its_project`, `a_project_taken_while_importing_is_told_by_its_notes` (`import.rs`).
- [x] `translate.rs` (`translate`, `rewrite_key`, grupos de clave y de texto); `import_foreign` con filas ya enmascaradas, `by_lang` y `ForeignReport`.
- Commit: `feat(core): import notes from another tool split and masked as saves` (`2a5dc7c`)

## 5. Reconocimiento por origen y revisiones (HI11 a HI13)

- [x] Tests `a_second_import_adds_nothing_even_with_another_map`, `a_moved_note_is_recognized_where_it_is`, `a_note_deleted_here_is_not_brought_back`, `a_note_rewritten_at_the_source_revises_the_stored_one_where_it_is`, `an_older_version_enters_superseded`, `a_note_deleted_at_the_source_changes_nothing`, `write_batches_never_split_a_note` y el de vínculos entre transacciones (`import.rs`).
- [x] La decisión por origen en `place`, `note_batches`, la regla de piezas por origen.
- Commit: `feat(core): recognize imported notes by their origin and take in their rewrites` (`e4d06e2`)

## 6. Borradas y relaciones (HI14)

- [x] Tests `relations_map_to_pair_states` (`translate.rs`) y los de `import.rs`.
- [x] `pair_for`, las líneas `conflict` y los estados de las filas; `write_links` escribe cada par como `(mayor, menor)`.
- Commit: `feat(core): import the relations between notes as conflict pairs` (`0036b66`)

## 7. La ruta y el comando (HI16, HI17, HI18)

- [x] Tests de `tests/api.rs`, `cli.rs` e `import.rs` del CLI.
- [x] `ImportRequest.from` y `project_map`, el `503` sin tokenizer; `--from` y `--project-map`; `render` en los dos idiomas.
- Commit: `feat(cli): import a memory export from another tool` (`ac6866c`)

## 8. El binario real (HI16)

- [x] `an_engram_export_imports_through_the_binary` (`tests/binary.rs`); el stand-in `fake` le da al proceso un tokenizer de una palabra por token, como una carga del modelo le da el suyo.
- Commit: `test(cli): import an engram export through a real daemon` (`4705984`)

## 9. Docs

- [x] `.claude/CLAUDE.md`: el comando, la API, la columna, el filtro y cómo exportar todo (`engram export --all`) fuera del repo; `README.md`: traer memorias de otra herramienta, el mapeo y que `migrated` sólo nombra al autor.
- Commit: `docs: describe importing memories from another tool` (`7507a13`)

## 10. Verificación

- [x] `cargo test --workspace --target-dir target-test` y el clippy de `.claude/CLAUDE.md` en Windows.
- [x] Linux, sobre `develop` después del merge (2026-10-24): la suite y los tres clippy del CI ([verify](verify.md#linux-2026-10-24)).
- [x] El binario real en un home temporal (2026-10-23): el binario de la suite con `CLAWG_TEST_HOME` y el modelo real descargado. Sin el modelo cargado, `503 tokenizer_not_ready` también en `--dry-run`; con él, un export sintético de 9 notas da el mismo reporte en el ensayo y en el import, y un segundo import las encuentra a las 9. Un export más nuevo con una nota de 1.740 palabras la parte en 33 piezas con el tokenizer real, enlazadas, y `/save` de la pieza 17 la devuelve entera; la nota en español reescrita entra como revisión (la vieja `superseded`, causa `revision`). La decisión con un `supersedes` marcado por un agente queda viva y el par `escalated`, con motivo y evidencia; secreto enmascarado, clave reescrita, borrada sin vector, `origin_project` guardado. Un segundo import sin nada que insertar igual escribe su snapshot `pre-import`, como el de un archivo de clawg.
- [ ] No se corrió ([verify](verify.md#lo-que-no-se-midió)). Perfil dev, con autorización: el export real (`engram export --all`) con `--dry-run`, el import, una docena de búsquedas, cuánto tarda, si el daemon contesta mientras tanto, y `projects move --origin-project`.

## 11. Archive

- [x] Las specs de la propuesta (HI2 y HI4 a HI18 en `specs/portability`, HI1 en `specs/storage`, HI3 en `specs/project`); `git mv` al archivo; subir el minor.
