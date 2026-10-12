# data-preservation — Tareas (partes A + F)

Cada tarea es un commit atómico (conventional commits) que compila y deja `cargo test --workspace` en verde. Dentro de cada una, primero el test que falla (RED), después el código (GREEN). En Windows: `cargo test --workspace --target-dir <otro-dir>`.

Los mensajes de commit son sugeridos. No mencionan planes, fases ni specs.

## F. Endurecimientos (independientes entre sí; van primero porque son chicos)

### 1. Checksums fijados — R9
- [ ] RED: `published_migration_checksums_are_pinned` en `crates/clawg-core/src/memory/migrations.rs`, con la tabla (versión, nombre, checksum) de V1–V8 sacada de `runner().get_migrations()`. Falla a propósito con una tabla vacía; se completa con los valores reales y queda en verde.
- [ ] Commit: `test(core): pin the checksums of published migrations`

### 2. Migraciones agrupadas — R8
- [ ] RED: `a_failing_migration_rolls_back_every_migration_of_its_run` en `db.rs`: V1–V8 reales más una V9 con SQL inválido, sobre una base vacía. Espera que no exista `memory` ni filas de historial.
- [ ] RED (protección): `a_v8_database_opens_unchanged_under_grouped_migrations`: siembra filas con el runner actual, reabre con el agrupado y compara.
- [ ] GREEN: `fn configured(runner) -> Runner` con `set_grouped(true)`, usada por `run_migrations` (`db.rs:361-364`). Verificar en el test que vec0 (V2) crea su tabla dentro de la transacción.
- [ ] Commit: `fix(core): apply pending migrations and their history atomically`

### 3. Directorio de datos sin home — R7
- [ ] RED: `data_dir_without_a_home_is_an_error` en `paths.rs`, sobre una función pura `data_dir_in(home: Option<PathBuf>, profile)`.
- [ ] GREEN: `data_dir` devuelve `Result`; error tipado con `thiserror`. Ajustar `data_dir_from_env` (`paths.rs:156`), `serve.rs:66` y `client.rs:624`, y que cada comando imprima el mensaje.
- [ ] Commit: `fix(core): refuse to run without a home directory instead of using ./.clawg`

### 4. Escritura atómica de `workspace.md` — R10
- [ ] RED: `a_failed_workspace_write_leaves_the_previous_file_intact` en `workspace.rs`: crea `workspace.md` con sección USER y un directorio llamado como el temporal; la regeneración devuelve `WorkspaceIo` y el archivo queda igual.
- [ ] GREEN: escribir `workspace.md.tmp` y `rename`; borrar el temporal si falla. Revisar que `write_failure_returns_workspace_io_error` (`workspace.rs:921`) siga valiendo.
- [ ] Commit: `fix(core): write workspace.md through a temporary file and rename`

## A. Apagado sin pérdida

### 5. Volcado: formato puro — R3
- [ ] RED: `spill_lines_round_trip_events_with_newlines_and_overrides` y un test de líneas inválidas (separa válidas de rechazadas, conserva el orden), en un módulo nuevo `crates/clawg-core/src/memory/spill.rs`.
- [ ] GREEN: `fn to_lines(&[MemoryEvent]) -> String` y `fn parse_lines(&str) -> (Vec<MemoryEvent>, Vec<String>)`, sin I/O. Escritura atómica (tmp + rename) en una función aparte.
- [ ] Commit: `feat(core): serialize pending ingest events to a JSON lines file`

### 6. Flush final del `AccessTracker` — R4
- [ ] RED: `access_tracker_flushes_its_buffer_on_shutdown` en `access.rs` (también el caso de soltar todos los `Sender`).
- [ ] GREEN: canal con enum privado (`Hit` / `Close`), `JoinHandle` guardado, `async fn shutdown(&self)`; flush antes de salir en las dos rutas. `inert()` no cambia de comportamiento.
- [ ] Commit: `fix(core): flush buffered access counts when the tracker stops`

### 7. Drenaje con plazo y volcado del resto — R1
- [ ] RED (unit): la decisión pura "seguir / volcar" según instante, plazo y embedder listo.
- [ ] RED (integración, `crates/clawg-core/tests/`): `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_spills_events_held_for_an_embedder_that_never_loaded`, `shutdown_spills_what_the_drain_deadline_left_unstarted`. Usan `MemoryRuntime::new_for_test` y un embedder de prueba que se puede retrasar o dejar sin cargar.
- [ ] GREEN: el worker recibe la señal de apagado (`watch::Receiver<bool>`), hace `rx.close()`, drena hasta `DRAIN_DEADLINE` y devuelve lo no procesado; el evento en curso termina. `MemoryRuntime::shutdown` vuelca lo devuelto con la tarea 5. Corregir el doc del worker (`worker.rs:110-118`) y el warn de `worker.rs:152-159`, que deja de existir.
- [ ] Commit: `fix(core): drain the ingest queue on shutdown and spill what is left`

### 8. Reinyección al arrancar — R2
- [ ] RED: `spilled_events_are_ingested_first_on_startup`, `a_corrupt_spill_line_is_set_aside_and_the_rest_ingested`, `a_shutdown_during_replay_keeps_the_unreplayed_lines_first`.
- [ ] GREEN: el worker lee el volcado antes del primer `recv`, mueve las líneas inválidas a `ingest-pending.rejected.jsonl` y borra el volcado al terminar.
- [ ] Commit: `feat(core): replay spilled ingest events before new ones on startup`

### 9. Checkpoint — R5
- [ ] RED: `shutdown_checkpoints_the_wal_into_memory_db` (base en disco en un tempdir).
- [ ] GREEN: `PRAGMA wal_checkpoint(TRUNCATE)` al final de `MemoryRuntime::shutdown`, best-effort con log.
- [ ] Commit: `fix(core): checkpoint the WAL when the runtime shuts down`

### 10. El daemon usa el apagado ordenado — R6
- [ ] RED: `the_drain_deadline_leaves_room_inside_the_stop_timeout` en `stop.rs`.
- [ ] GREEN: `serve.rs` guarda los `JoinHandle` del worker de ingesta y de mantenimiento, y después de `axum::serve` llama `MemoryRuntime::shutdown` antes de `clear_state`. Corregir el doc de `STOP_TIMEOUT` (`stop.rs:12-14`).
- [ ] Commit: `fix(cli): shut the daemon down through the runtime's drain`

## Verificación

- [ ] `cargo test --workspace`: reportar el conteo contra la base de 1304.
- [ ] Prueba manual en el perfil dev, **sólo con autorización**: arrancar el daemon, guardar mientras el modelo carga, `clawg stop`, comprobar que existe `ingest-pending.jsonl`, arrancar de nuevo y comprobar que las filas aparecen y el archivo ya no está.
