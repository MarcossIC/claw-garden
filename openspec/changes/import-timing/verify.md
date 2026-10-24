# import-timing — Verificación

## Linux (2026-10-24)

La suite (`cargo test --workspace --locked`) y los dos clippy de `.claude/CLAUDE.md` pasan: el del workspace con `--all-targets` y el de `-p clawg-cli` sin `testing`. Se enlazó ONNX Runtime 1.29 en dinámico (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque la descarga de `ort-sys` no estaba disponible. Ningún test carga el modelo real.

- **IT1, las fases:** `an_import_times_each_step_once_in_order` (las notas de muestra de otra herramienta, con una nota borrada en la fuente: `embed` tiene las insertadas no borradas), `a_clawg_file_is_prepared_rather_than_translated` y `a_dry_run_times_only_what_it_runs`.
- **IT2, la conexión:** `time_in_sqlite_never_exceeds_the_call`. Con un export de clawg de 600 filas en un store vacío, `load` hace 4 llamadas (3 lotes de textos y las raíces) y `write` los 3 lotes de `note_batches`. En las dos fases, el tiempo en SQLite más la espera no supera la duración, y la llamada más lenta no supera el tiempo en SQLite.
- **IT3, el texto:** `a_step_reads_as_its_numbers` y `the_summary_names_every_step` (`timing.rs`), que no compilaban antes de las estructuras.
- **IT4, sin cambios:** las pruebas de import de `import.rs` y `undo.rs`, `tests/api.rs` y `tests/binary.rs` pasan. La única edición fue el argumento `&mut ImportTimings` en las llamadas a `import`, `import_foreign`, `prepare_file`, `prepare_export` y `load`. Ninguna aserción cambió.

## Pendiente

- El CI del pull request en Linux y Windows.
- **[perfil dev, modelo real, con autorización]** La importación real de la propuesta: la suma de las fases contra el tiempo del cliente, con un resto menor al 1 %. No se corrió.
