# storage-safety-net — Tareas (partes B + C)

Cada tarea es un commit atómico (conventional commits) que compila y deja `cargo test --workspace` en verde. Dentro de cada una, primero el test que falla (RED), después el código (GREEN). En Windows: `cargo test --workspace --target-dir <otro-dir>`.

Los mensajes de commit son sugeridos. No mencionan planes, fases ni specs.

## Verificación del esquema

### 1. Plan de esquema puro — R1
- [x] RED: `schema_plan_covers_every_history_shape` en un módulo nuevo `crates/clawg-core/src/memory/backup.rs`: historial vacío, igual, prefijo, con una versión mayor, con un checksum distinto y con un hueco.
- [x] GREEN: `enum SchemaPlan { Fresh, UpToDate, Pending { from, to }, Newer { db, binary }, Diverged { version } }` y `fn schema_plan(applied, known) -> SchemaPlan`, sin I/O.
- [x] Commit: `feat(core): decide the schema plan of a database before migrating it`

### 2. Rechazo de bases más nuevas o divergentes — R1
- [x] RED: `a_newer_database_is_refused_untouched` y `a_diverged_database_is_refused_untouched` en `db.rs`: base en disco en un tempdir, historial alterado a mano, y comparación byte a byte del archivo después de cerrar.
- [x] GREEN: `read_history(&rusqlite::Connection)`; `MemoryStore::open` calcula el plan antes de `run_migrations` y devuelve `MemoryError::SchemaNewer { db, binary }` o `MemoryError::SchemaDiverged { version }`, con mensajes que nombran `clawg restore`.
- [x] Commit: `feat(core): refuse a newer or diverged database with a message that says what to do`

## Snapshots

### 3. Nombre y escritura de un snapshot — R3
- [x] RED: `snapshot_names_sort_in_time_order` (con el ejemplo de la spec), `a_snapshot_includes_writes_still_in_the_wal`, `a_snapshot_keeps_vectors_and_full_text_searchable` y `a_failed_snapshot_leaves_no_file_behind`.
- [x] GREEN: `enum Reason { PreMigration, PreRestore, Manual }`, `fn snapshot_name(ms, reason)` y su parser, y `fn snapshot_into(conn, dir, reason) -> Result<PathBuf, BackupError>` con `VACUUM INTO` a un `.tmp` y `rename`. `BackupError` con `thiserror`.
- [x] Commit: `feat(core): write consistent snapshots of the memory store`

### 4. Retención — R4
- [x] RED: `pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one` (unit) y `an_automatic_snapshot_prunes_older_automatic_ones`.
- [x] GREEN: `const AUTO_KEEP: usize = 5`, `fn to_prune(names) -> Vec<String>` pura, y poda best-effort dentro de `snapshot_into` después de cada snapshot automático.
- [x] Commit: `feat(core): keep only the newest automatic snapshots`

### 5. Snapshot antes de migrar — R2
- [x] RED: `opening_an_older_database_snapshots_it_before_migrating` (base sembrada hasta V7 con el runner con `set_target`, con filas), `opening_a_current_database_takes_no_snapshot` y `a_failed_snapshot_stops_the_migration`.
- [x] GREEN: con plan `Pending`, `open` llama a `snapshot_into(.., PreMigration)` antes de `run_migrations`; el error es `MemoryError::Backup`.
- [x] Commit: `feat(core): snapshot the store before applying pending migrations`

## Comandos

### 6. Backup sin daemon — R5
- [x] RED: `an_offline_backup_copies_without_migrating` y `an_offline_backup_refuses_a_held_store`.
- [x] GREEN: `StoreHolder::Maintenance`; `fn backup_offline(base_dir) -> Result<PathBuf, MemoryError>` (lock, conexión directa sin migrar, `snapshot_into(.., Manual)`). Devuelve `MemoryError` para que `StoreInUse` llegue tal cual.
- [x] Commit: `feat(core): back up a stopped store without migrating it`

### 7. `clawg backup` y `POST /v1/backup` — R5
- [x] RED: `backup_endpoint_writes_a_manual_snapshot` en `api.rs`, y el parseo de `clawg backup [--profile]` en `cli.rs`.
- [x] GREEN: `MemoryRuntime::snapshot(reason)` (el runtime conoce el directorio de datos; el store no), la ruta en el router detrás del token, `DaemonClient::backup()`, y el comando en `main.rs`: con daemon por la API, sin daemon con `backup_offline`.
- [x] Commit: `feat(cli): add clawg backup`

### 8. Restore — R6
- [x] RED: `restore_replaces_the_store_and_keeps_the_previous_one`, `restore_refuses_files_it_cannot_open_safely`, `restore_refuses_while_the_daemon_holds_the_store`, `a_restored_older_snapshot_migrates_on_the_next_open` y `restoring_the_oldest_kept_automatic_snapshot_survives_the_pruning`.
- [x] GREEN: `fn restore(base_dir, from) -> Result<Restored, MemoryError>` con el orden de D3 (lock, validar, temporal con `fsync`, snapshot `pre-restore` y checkpoint, borrar WAL y SHM, `rename`). El temporal va antes del snapshot porque la poda puede borrar el archivo que se restaura.
- [x] Commit: `feat(core): restore the store from a snapshot, keeping the one it replaces`

### 9. `clawg restore` — R6
- [x] RED: `restore_takes_a_file_and_a_profile` en `cli.rs` y los tests de redacción de un módulo nuevo, `crates/clawg-cli/src/restore.rs`.
- [x] GREEN: el comando en `main.rs`; con `StoreInUse` del daemon agrega "run `clawg stop` first".
- [x] Commit: `feat(cli): add clawg restore`

### 9b. Base inexistente y máquina nueva — R5, R6
Lo encontró la prueba de punta a punta con el binario: sin directorio de datos, `clawg backup` fallaba al crear `memory.lock` con un error de lock poco claro, y `clawg restore` no podía mudar una base a una máquina nueva.
- [x] RED: `an_offline_backup_of_nothing_says_so` con un directorio que no existe, y `restore_onto_a_machine_without_a_data_directory`.
- [x] GREEN: `backup_offline` mira `memory.db` antes de tomar el lock; `restore` crea el directorio de datos.
- [x] Commit: `fix(core): name a missing store and restore onto a new data directory`

## Mensajes y actualización

### 10. `clawg status` muestra el esquema — R7
- [x] RED: `a_stopped_daemon_reports_the_store_schema` en `status.rs` y `probing_a_newer_store_reads_without_writing` en `backup.rs`. La primera versión, con `SQLITE_OPEN_READ_ONLY`, dejaba `memory.db-wal` y `memory.db-shm`: el test lo detectó.
- [x] GREEN: `fn probe(base_dir) -> StoreProbe` con la URI `immutable=1` cuando no hay `-wal`; `status_report` recibe el probe; `main.rs` lo calcula sólo sin daemon.
- [x] Commit: `feat(cli): show the store's schema state in clawg status when the daemon is down`

### 11. Mensaje de MCP — R8
- [x] RED: `a_daemon_that_did_not_start_points_at_status` en `server.rs`.
- [x] GREEN: `explain` separa `StartTimeout` de `NotRunning`. `a_tool_call_when_the_daemon_never_comes_up_gives_the_readable_error` (`mcp/mod.rs`) esperaba `clawg serve` y pasa a esperar `clawg status`.
- [x] Commit: `fix(cli): point at clawg status when the daemon could not start`

### 12. El instalador verifica el stop — R9
- [x] `scripts/install.ps1`: después de `stop`, `status --profile prod`; si sale con 0, abortar antes de mover el binario.
- [x] Commit: `fix(install): abort when the daemon is still running after clawg stop`

## Verificación

- [x] `cargo test --workspace`: reportar el conteo contra la base. Ver `verify.md`.
- [ ] (pendiente, ver `verify.md`) Prueba manual en Windows, perfil dev, **sólo con autorización**: `clawg backup` con el daemon andando, `clawg stop`, `clawg restore`, arrancar y comparar el conteo de filas. Instalador con un daemon que no para.
