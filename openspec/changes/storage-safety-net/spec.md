# storage-safety-net — Especificación (partes B + C)

**Dominio destino:** `specs/storage/spec.md`. Es un delta: todo lo de abajo se **agrega** a la spec consolidada al archivar. No modifica ni quita ningún requisito existente.

**Alcance:** la verificación del esquema y el snapshot antes de migrar (`MemoryStore::open`), los comandos `clawg backup` y `clawg restore`, `POST /v1/backup`, la retención de `<data>/backups/`, el bloque `store` de `clawg status`, el mensaje de MCP cuando el daemon no arranca y la verificación del stop en `scripts/install.ps1`. No cambia el esquema.

## Definiciones

- **Directorio de backups:** `<data>/backups/`.
- **Snapshot:** una copia completa y autónoma de `memory.db` hecha con `VACUUM INTO`. Incluye lo commiteado en el WAL y no tiene archivo WAL propio.
- **Motivo:** por qué existe un snapshot. `pre-migration` y `pre-restore` son **automáticos**; `manual` es el de `clawg backup`.
- **Plan de esquema:** la comparación entre el historial de `refinery_schema_history` de una base y las migraciones embebidas en el binario. Sus resultados son:
  - `Fresh`: la base no tiene historial;
  - `UpToDate`: tiene exactamente las migraciones del binario;
  - `Pending { from, to }`: tiene un prefijo de las del binario, y faltan de `from + 1` a `to`;
  - `Newer { db, binary }`: tiene una versión aplicada mayor que la última del binario;
  - `Diverged { version }`: una versión aplicada tiene otro nombre o checksum que la del binario, o falta en el binario.

## Verificación del esquema

### R1. El plan de esquema se decide antes de migrar

La decisión MUST ser una función pura de (historial aplicado, migraciones del binario). `MemoryStore::open` MUST calcularla antes de llamar al runner. Con `Newer` o `Diverged`, `open` MUST devolver un error tipado (`MemoryError::SchemaNewer` o `MemoryError::SchemaDiverged`) y MUST NOT escribir nada en `memory.db`.

Los mensajes MUST decir qué versión tiene la base y cuál conoce el binario, que la base no se tocó, y qué hacer: instalar la versión que la escribió, o `clawg restore` de un backup anterior.

#### Escenario: base más nueva
- GIVEN una base migrada a V8 con una fila V9 agregada a mano en `refinery_schema_history`
- WHEN se abre con el binario actual (V8)
- THEN el error es `SchemaNewer { db: 9, binary: 8 }`, el mensaje nombra `clawg restore`, y `memory.db` es byte a byte igual al de antes

#### Escenario: checksum divergente
- GIVEN una base migrada a V8 con el checksum de V3 cambiado en el historial
- WHEN se abre
- THEN el error es `SchemaDiverged { version: 3 }` y la base no cambió

#### Escenario: tabla de decisión
- GIVEN los historiales vacío, V1–V8, V1–V5, V1–V9 y V1–V8 con V3 alterada, contra un binario V1–V8
- THEN los planes son `Fresh`, `UpToDate`, `Pending { from: 5, to: 8 }`, `Newer { db: 9, binary: 8 }` y `Diverged { version: 3 }`

**Tests:** `schema_plan_covers_every_history_shape` (unit), `a_newer_database_is_refused_untouched` y `a_diverged_database_is_refused_untouched` (`db.rs`).

### R2. Snapshot antes de migrar

Con plan `Pending`, `open` MUST hacer un snapshot con motivo `pre-migration` antes de aplicar la primera migración pendiente. Si el snapshot falla, `open` MUST devolver el error (`MemoryError::Backup`) y MUST NOT migrar. Con `Fresh` y `UpToDate` MUST NOT hacer snapshot.

#### Escenario: base vieja con filas
- GIVEN una base migrada sólo hasta V7, con filas en `memory`
- WHEN se abre con el binario V8
- THEN `backups/` tiene un snapshot `pre-migration` cuyo historial llega a V7 y que contiene esas filas, y la base quedó en V8

#### Escenario: base al día
- GIVEN una base ya en V8
- WHEN se abre
- THEN `backups/` no existe o está vacío

#### Escenario: falla el snapshot
- GIVEN una base en V7 y un archivo común llamado `backups` en el directorio de datos
- WHEN se abre
- THEN el error es `Backup`, y el historial de la base sigue en V7

**Tests:** `opening_an_older_database_snapshots_it_before_migrating`, `opening_a_current_database_takes_no_snapshot` y `a_failed_snapshot_stops_the_migration` (`db.rs`).

## Snapshots

### R3. Un snapshot es completo y no se ve a medias

Un snapshot MUST contener todo lo commiteado, incluido lo que está sólo en el WAL. MUST escribirse a un temporal en el directorio de backups y renombrarse al nombre final recién cuando está completo; un snapshot fallido MUST NOT dejar ni el temporal ni un archivo final.

El nombre MUST ser `memory-<AAAA-MM-DD>T<hh-mm-ss.mmm>Z-<motivo>.db`, en UTC, generado por una función pura, de modo que el orden alfabético sea el cronológico.

#### Escenario: WAL
- GIVEN una base en WAL con un insert commiteado y sin checkpoint
- WHEN se hace un snapshot
- THEN el snapshot abierto solo (sin WAL) tiene esa fila, y también sus tablas vec0 y FTS consultables

#### Escenario: nombre
- GIVEN el instante 1791827476123 ms y el motivo `manual`
- THEN el nombre es `memory-2026-10-12T17-51-16.123Z-manual.db`

**Tests:** `a_snapshot_includes_writes_still_in_the_wal`, `a_snapshot_keeps_vectors_and_full_text_searchable`, `snapshot_names_sort_in_time_order` y `a_failed_snapshot_leaves_no_file_behind` (`backup.rs`).

### R4. Retención

Después de cada snapshot automático, el directorio de backups MUST conservar los 5 automáticos más nuevos (`AUTO_KEEP`) y borrar los demás automáticos. Los `manual` MUST NOT borrarse nunca. Un archivo cuyo nombre no sigue el formato MUST NOT tocarse. La elección de qué borrar MUST ser pura.

#### Escenario
- GIVEN 7 snapshots automáticos, 2 manuales y un `notas.txt`
- WHEN se hace un snapshot automático más
- THEN quedan los 5 automáticos más nuevos, los 2 manuales y `notas.txt`

**Tests:** `pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one` (unit) y `an_automatic_snapshot_prunes_older_automatic_ones` (`backup.rs`).

## Comandos

### R5. `clawg backup`

`clawg backup [--profile P]` MUST producir un snapshot `manual` e imprimir su ruta.

- Con el daemon andando, MUST pedirlo por `POST /v1/backup` (con el token), que hace el snapshot sobre la conexión del daemon y responde `{"path": "..."}`.
- Sin daemon, MUST tomar el lock del store (titular `Maintenance`), copiar sin migrar y soltar el lock.
- Si otro proceso que no es el daemon tiene el lock, MUST fallar con el mensaje de `StoreInUse`.

#### Escenario: con daemon
- GIVEN un daemon en proceso con 3 memorias
- WHEN se llama `POST /v1/backup`
- THEN responde 200 con una ruta bajo `backups/`, y ese archivo tiene las 3 filas

#### Escenario: sin daemon
- GIVEN un directorio de datos con una base en V8 y sin daemon
- WHEN se hace el backup sin daemon
- THEN existe un snapshot `manual` con las mismas filas, la base no cambió y el lock quedó libre

**Tests:** `backup_endpoint_writes_a_manual_snapshot` (`api.rs`), `an_offline_backup_copies_without_migrating` y `an_offline_backup_refuses_a_held_store` (`backup.rs`).

### R6. `clawg restore`

`clawg restore <ARCHIVO> [--profile P]` MUST:

1. tomar el lock del store; si lo tiene el daemon, MUST fallar diciendo que se pare con `clawg stop`;
2. validar el archivo: SQLite íntegro (`PRAGMA quick_check` = `ok`), con historial de refinery, y con un plan contra el binario que no sea `Newer`, `Diverged` ni `Fresh`. Si no pasa, MUST fallar sin tocar nada;
3. si existe `memory.db`, hacer un snapshot `pre-restore` y un `wal_checkpoint(TRUNCATE)`. Si falla, MUST fallar sin tocar nada;
4. copiar el archivo a un temporal en el directorio de datos, con `fsync`; borrar `memory.db-wal` y `memory.db-shm`; y hacer `rename` sobre `memory.db`;
5. imprimir el archivo restaurado y la ruta del snapshot `pre-restore`.

Una base restaurada con plan `Pending` MUST abrir con el próximo arranque, que la migra con su propio snapshot (R2).

#### Escenario: restore
- GIVEN una base con las filas A y B, y un snapshot que sólo tiene A
- WHEN se restaura el snapshot
- THEN la base tiene sólo A, `backups/` tiene un `pre-restore` con A y B, y no quedan temporales

#### Escenario: archivo inválido
- GIVEN un archivo de texto, una base SQLite sin historial de refinery, y una base con V9
- WHEN se intenta restaurar cada uno
- THEN cada uno falla con un error que lo nombra, y `memory.db` es byte a byte igual

#### Escenario: daemon andando
- GIVEN el lock del store tomado por el daemon
- WHEN se intenta restaurar
- THEN falla con un mensaje que nombra `clawg stop`, y no se creó ningún snapshot

**Tests:** `restore_replaces_the_store_and_keeps_the_previous_one`, `restore_refuses_files_it_cannot_open_safely`, `restore_refuses_while_the_daemon_holds_the_store` y `a_restored_older_snapshot_migrates_on_the_next_open` (`backup.rs`).

## Mensajes y actualización

### R7. `clawg status` muestra el esquema sin daemon

Sin daemon, el JSON de `clawg status` MUST tener un bloque `store` con `state` (`missing`, `fresh`, `up_to_date`, `pending`, `newer`, `diverged` o `unreadable`), `db_version`, `binary_version` y un `message` legible cuando el estado impide arrancar. Con daemon, `store` MUST ser `null`: el daemon ya abrió la base. La lectura MUST hacerse en sólo lectura, sin lock y sin migrar. El armado del bloque MUST ser puro.

#### Escenario
- GIVEN un directorio de datos sin daemon y con una base que tiene V9
- WHEN se arma el reporte
- THEN `running` es `false`, `store.state` es `newer`, `store.db_version` es 9 y `store.message` nombra `clawg restore`

**Tests:** `a_stopped_daemon_reports_the_store_schema` (`status.rs`) y `probing_a_newer_store_reads_without_writing` (`backup.rs`).

### R8. MCP nombra `clawg status` cuando el daemon no arrancó

Cuando el autoarranque no deja un daemon sirviendo (`StartTimeout`), el texto de la herramienta MUST decir que clawg no pudo arrancar su daemon y que `clawg status` muestra la causa. Cuando no hay daemon y el autoarranque está apagado (`NotRunning`), MUST seguir pidiendo `clawg serve`.

**Test:** `a_daemon_that_did_not_start_points_at_status` (`server.rs`).

### R9. El instalador aborta si el daemon sigue vivo

Después de `clawg stop --profile prod`, `scripts/install.ps1` MUST ejecutar `clawg status --profile prod` con el binario anterior. Si sale con 0, MUST abortar antes de mover el binario, con un mensaje que diga que el daemon sigue corriendo y que no cambió nada.

**Verificación:** manual en Windows (no hay PowerShell en CI ni en Linux). Ver `verify.md`.
