# import-undo — Verificación

## Linux (2026-10-24)

La suite (`cargo test --workspace --locked`) y los dos clippy de `.claude/CLAUDE.md` pasan: el del workspace con `--all-targets` y el de `-p clawg-cli` sin `testing`. Se enlazó ONNX Runtime 1.29 en dinámico (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque la descarga de `ort-sys` no estaba disponible. Ningún test carga el modelo real.

- **IU1, el diario:** `an_import_is_numbered_and_journaled` y `a_dry_run_and_an_import_that_adds_nothing_have_no_number` (`import.rs`). El oráculo de import de `import.rs`, `tests/api.rs` y `tests/binary.rs` pasa; las únicas aserciones que cambiaron son las que comparan un dry run con el real, y el banco, que quita del reporte lo que cambia entre corridas: suman `import_id` a lo que difiere.
- **IU2, la vuelta:** `undoing_a_clawg_import_gives_the_store_back` y `undoing_a_foreign_import_gives_the_store_back` comparan, fila por fila, `memory`, `memory_vectors` (con el vector), las filas del índice de texto (`memory_fts_docsize`), `memory_piece`, `memory_derivation`, `memory_conflict`, `project` y `project_remote` antes de importar y después de deshacer: son iguales, y el archivo de clawg vuelve a importarse entero con el número 2. `undoing_a_rewrite_brings_the_stored_version_back`, `undoing_fills_empties_the_links_it_filled` y `what_was_saved_after_stays` (`undo.rs`).
- **IU3, el criterio:** `decide_keeps_what_changed_and_what_is_named` (cada motivo, la iteración, un vínculo propio que no retiene, lo que se restaura y lo que no, y una raíz con y sin uso), `the_rows_someone_changed_stay` (edición, borrado, restauración, par juzgado, fuente de una conclusión y pieza de un save que queda se quedan; movida, accedida, bajada de `tier` y reemplazada por clave se van) y `a_purged_row_does_not_stop_the_undo`.
- **IU4:** `an_undo_twice_changes_nothing_the_second_time`, `an_unknown_or_running_import_is_refused` y `a_dry_run_undo_reports_without_writing`.
- **IU5:** `import_takes_a_file_or_an_undo` (`cli.rs`), `render_undo_names_what_it_took_back_and_what_stayed` y `the_undo_report_speaks_spanish`, `import_report_lists_counts_and_invalid_lines` con el deshacer contra un daemon de prueba (`crates/clawg-cli/src/import.rs`) y `an_import_is_undone_by_its_number` (`tests/api.rs`, también sin embedder).
- **IU6:** `a_store_from_before_the_import_journal_keeps_its_rows` (`db.rs`), la fila 26 de `published_migration_checksums_are_pinned` y `snapshot_names_parse_back_to_their_reason`.
- **Índices:** `the_undo_lookups_seek_an_index` exige que cada lectura y cada borrado por fila del deshacer busque por un índice.

### El banco de 81, con y sin el diario

`twice_the_notes_import_in_at_most_three_times_the_time`, en debug, solo (`--test-threads=1`), dos corridas de cada lado:

| | 10.000 notas | 20.000 notas | Razón |
|---|---|---|---|
| Antes (`be1a1a8`, sin el diario) | 3,31 s y 3,64 s | 6,96 s y 7,19 s | 2,1 y 2,0 |
| Después | 4,13 s y 3,91 s | 7,57 s y 7,62 s | 1,8 y 1,9 |

El diario agrega una fila por fila insertada en la misma transacción: cuesta entre un 5 % y un 20 % y no cambia la forma de la curva.

### El deshacer

Con un test temporal que no se commiteó, en debug: importar notas de una fila de otra herramienta en un store vacío y deshacer el import.

| | Corrida en seco | Deshacer |
|---|---|---|
| 10.000 notas | 0,40 s | 2,14 s |
| 20.000 notas | 0,60 s | 4,93 s |

Lineal. Ninguna llamada a la conexión dura con el import: la carga va en llamadas de 256 búsquedas y el borrado en transacciones de 256 filas.

## Pendiente

- La suite y clippy en Windows (`target-test`).
- El CI del pull request en Linux y Windows.
