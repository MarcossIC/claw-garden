# storage — Especificación

Fuente de verdad del comportamiento actual del almacenamiento: el apagado del daemon, el volcado y la reinyección de eventos sin ingerir, la resolución del directorio de datos, el runner de migraciones y la verificación del esquema, el pooling del modelo y el re-embebido de un store CLS al arrancar, los snapshots, `clawg backup` y `clawg restore`, la escritura de `workspace.md`, y lo que `clawg status`, MCP y el instalador dicen cuando la base no se puede abrir.

**Origen:**
- [data-preservation](../../changes/archive/2026-10-12-data-preservation/proposal.md) (partes A + F), archivado el 2026-10-12.
- [storage-safety-net](../../changes/archive/2026-10-12-storage-safety-net/proposal.md) (partes B + C de data-preservation), archivado el 2026-10-12.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Contrato de embedding y pooling", el motivo `pre-reembed` y la fila de V12).

## Definiciones

- **Evento aceptado:** un `MemoryEvent` que entró al canal de ingesta.
- **Apagado ordenado:** el que empieza con Ctrl-C o con `POST /v1/shutdown`. Un kill del proceso no es un apagado ordenado.
- **Plazo de drenaje (`DRAIN_DEADLINE`, `crates/clawg-core/src/runtime.rs:52`):** el tiempo máximo que el apagado dedica a ingerir lo encolado.
- **Volcado:** el archivo `<data>/ingest-pending.jsonl` (`SPILL_FILE`, `crates/clawg-core/src/memory/spill.rs:14`), un `MemoryEvent` en JSON por línea.
- **Directorio de backups:** `<data>/backups/` (`backups_dir`, `crates/clawg-core/src/paths.rs`).
- **Snapshot:** una copia completa y autónoma de `memory.db` hecha con `VACUUM INTO`. Incluye lo commiteado en el WAL y no tiene archivo WAL propio.
- **Motivo** de un snapshot (`Reason`, `crates/clawg-core/src/memory/backup.rs:119`): `pre-migration`, `pre-restore` y `pre-reembed` son **automáticos**; `manual` es el de `clawg backup`.
- **Plan de esquema** (`SchemaPlan`, `backup.rs:63`): la comparación entre el historial de `refinery_schema_history` y las migraciones embebidas en el binario. `Fresh` (sin historial), `UpToDate`, `Pending { from, to }` (un prefijo; faltan de `from + 1` a `to`), `Newer { db, binary }` (una versión aplicada mayor que la última del binario) o `Diverged { version }` (otro nombre o checksum, o un hueco).

## Apagado sin pérdida

### Ningún evento aceptado se pierde en un apagado ordenado

En un apagado ordenado, cada evento aceptado MUST terminar en uno de dos lugares: insertado en la base (o absorbido por el gate de dedup), o escrito en el volcado. El apagado MUST NOT descartar un evento.

La decisión de seguir drenando o volcar es pura (`drain_step`, `crates/clawg-core/src/memory/worker.rs:160`). `MemoryRuntime::shutdown` (`crates/clawg-core/src/runtime.rs:191`) vuelca lo que el worker devuelve, y el daemon la llama antes de borrar su estado (`crates/clawg-cli/src/daemon/serve.rs:151-155`).

#### Escenario: embedder listo
- GIVEN el embedder cargado y 20 eventos en el canal
- WHEN se hace un apagado ordenado
- THEN la base tiene las filas de los 20 eventos y no existe el volcado

#### Escenario: embedder sin cargar
- GIVEN el embedder sin cargar y 3 eventos en el canal (uno retenido por `wait_for_embedder`)
- WHEN se hace un apagado ordenado
- THEN el volcado tiene 3 líneas, en el orden en que entraron, y el log no dice "dropped"

#### Escenario: vence el plazo
- GIVEN un embedder que tarda más que `DRAIN_DEADLINE` en procesar los eventos encolados
- WHEN se hace un apagado ordenado
- THEN el evento en curso termina su insert, y los que no empezaron quedan en el volcado

**Tests:** `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_spills_events_held_for_an_embedder_that_never_loaded` y `shutdown_spills_what_the_drain_deadline_left_unstarted` (`crates/clawg-core/tests/shutdown.rs`), más los unit de `drain_step` en `worker.rs`.

### Reinyección al arrancar

Si el volcado existe al arrancar, el worker MUST procesar sus eventos antes que cualquier evento nuevo del canal, en el orden del archivo. MUST borrar el volcado sólo cuando procesó todas sus líneas. Si un apagado ordenado llega a mitad de la reinyección, el volcado nuevo MUST contener las líneas no procesadas del anterior, seguidas de lo encolado.

Una línea que no se puede parsear MUST NOT bloquear las demás: se registra con un warn y se mueve a `<data>/ingest-pending.rejected.jsonl` (`REJECTED_FILE`, `spill.rs:18`), que nada borra. Si no se puede guardar en ese archivo, el volcado MUST NOT borrarse, porque es la única copia de esas líneas.

#### Escenario: reinyección
- GIVEN un volcado con 2 eventos y un save nuevo en el canal
- WHEN arranca el worker con el embedder listo
- THEN los 3 quedan en la base, los 2 del volcado con ids menores que el nuevo, y el volcado ya no existe

#### Escenario: línea corrupta
- GIVEN un volcado con una línea válida, una truncada y otra válida
- THEN se ingieren las dos válidas, y la truncada queda en `ingest-pending.rejected.jsonl`

#### Escenario: apagado durante la reinyección
- GIVEN un volcado de 3 eventos, el embedder sin cargar y 1 save nuevo
- WHEN se hace un apagado ordenado
- THEN el volcado tiene 4 líneas: las 3 anteriores y luego la nueva

**Tests:** `spilled_events_are_ingested_first_on_startup`, `a_corrupt_spill_line_is_set_aside_and_the_rest_ingested` y `a_shutdown_during_replay_keeps_the_unreplayed_lines_first` (`crates/clawg-core/tests/shutdown.rs`).

### Formato del volcado

Las funciones que serializan y parsean el volcado MUST ser puras, sin I/O (`to_lines` y `parse_lines`, `spill.rs:109` y `spill.rs:122`). El volcado MUST escribirse de forma atómica: archivo temporal en el mismo directorio y `rename` (`write`, `spill.rs:135`). Un evento MUST sobrevivir ida y vuelta sin cambios, incluidos `memory_type` e `importance`.

#### Escenario
- GIVEN un `MemoryEvent` explícito con `memory_type = "decision"`, `importance = 8.0` y texto con saltos de línea
- WHEN se serializa y se parsea
- THEN el resultado es igual al original y ocupa una sola línea

**Tests:** `spill_lines_round_trip_events_with_newlines_and_overrides`, `invalid_spill_lines_are_set_apart_and_the_valid_ones_keep_their_order` y `write_replaces_the_spill_file_with_exactly_the_given_events` (`spill.rs`).

### Flush final del `AccessTracker`

En un apagado ordenado, el tracker MUST escribir el buffer pendiente antes de terminar, y el apagado MUST esperar a que termine (`AccessTracker::shutdown`, `crates/clawg-core/src/memory/access.rs:134`). Lo mismo vale cuando se sueltan todos los `Sender`.

#### Escenario
- GIVEN un hit registrado con `record` hace menos de 30 s
- WHEN se hace un apagado ordenado
- THEN el `access_count` de esa fila subió en 1

**Tests:** `access_tracker_flushes_its_buffer_on_shutdown`, `access_tracker_flushes_its_buffer_when_every_sender_is_dropped` y `shutting_down_twice_is_harmless` (`access.rs`).

### Checkpoint al terminar

Después de drenar y del flush del tracker, el apagado SHOULD hacer `PRAGMA wal_checkpoint(TRUNCATE)` (`checkpoint`, `runtime.rs:311`). Un fallo del checkpoint MUST registrarse y MUST NOT impedir el resto del apagado.

#### Escenario
- GIVEN una base en WAL con escrituras recientes
- WHEN se hace un apagado ordenado
- THEN `memory.db-wal` no existe o mide 0 bytes, y una copia de `memory.db` sola contiene las escrituras

**Test:** `shutdown_checkpoints_the_wal_into_memory_db` (`crates/clawg-core/tests/shutdown.rs`).

### El plazo de drenaje cabe en `STOP_TIMEOUT`

`DRAIN_DEADLINE` más un margen para el volcado, el flush y el checkpoint MUST NOT superar `STOP_TIMEOUT` (`crates/clawg-cli/src/stop.rs:19`). Hoy son 10 s + 5 s = 15 s. El doc de `STOP_TIMEOUT` MUST describir lo que el apagado hace.

**Test:** `the_drain_deadline_leaves_room_inside_the_stop_timeout` (`stop.rs`).

**Límite conocido:** el plazo acota el apagado ordenado, pero no la vida del proceso. Si el apagado llega mientras carga el modelo, el proceso sigue vivo hasta que esa carga vuelve: corre en `spawn_blocking` (`crates/clawg-core/src/memory/bootstrap.rs:318`), y el runtime de `#[tokio::main]` (`crates/clawg-cli/src/main.rs:30`) espera a esas tareas al soltarse. Los datos ya están a salvo para entonces. Está en el backlog de `openspec/README.md`.

## Directorio de datos

### Sin home no hay directorio de datos

Si no se puede resolver el home, la resolución MUST devolver un error tipado que lo diga (`PathsError`, `crates/clawg-core/src/paths.rs:149`), y MUST NOT usar el directorio actual. `clawg serve`, `clawg mcp`, `clawg status` y `clawg stop` MUST terminar con ese mensaje y sin crear ningún archivo.

La resolución es una función pura que recibe el home (`data_dir_in`, `paths.rs:159`), para no depender del entorno.

#### Escenario
- GIVEN un resolvedor de home que devuelve `None`
- WHEN se calcula el directorio de datos
- THEN el resultado es `Err` y el mensaje nombra el home

**Tests:** `data_dir_without_a_home_is_an_error` y `data_dir_in_a_home_is_the_profile_directory_under_it` (`paths.rs`).

## Migraciones

### Migraciones atómicas

Las migraciones pendientes de un arranque MUST aplicarse en una sola transacción junto con sus filas de historial (`configured`, `crates/clawg-core/src/memory/db.rs:367`, con `set_grouped(true)`). Si una falla, la base MUST quedar como estaba antes del arranque.

#### Escenario: fallo a mitad
- GIVEN una base vacía y la lista V1–V8 más una V9 cuyo SQL falla
- WHEN se migra con la configuración de producción
- THEN no existe la tabla `memory` ni ninguna fila en `refinery_schema_history`

#### Escenario: base existente
- GIVEN una base migrada a V8 por el runner sin agrupar, con filas
- WHEN se abre con el runner agrupado
- THEN no se aplica nada y las filas siguen igual

**Tests:** `a_failing_migration_rolls_back_every_migration_of_its_run` y `a_v8_database_opens_unchanged_under_grouped_migrations` (`db.rs`).

### Checksums fijados

Los checksums de las migraciones publicadas (hoy V1–V12) MUST coincidir con una lista fija en el código (`PINNED`, `crates/clawg-core/src/memory/migrations.rs:44`). Cambiar una migración publicada MUST romper un test. Una migración nueva agrega su fila a la lista en el mismo commit.

**Test:** `published_migration_checksums_are_pinned` (`migrations.rs`).

### Verificación del esquema antes de migrar

La decisión es pura (`schema_plan`, `backup.rs:83`). `MemoryStore::open` MUST calcularla antes de cualquier pragma y antes del runner (`check_schema`, `crates/clawg-core/src/memory/db.rs:388`, llamada en `db.rs:185`). Con `Newer` o `Diverged`, `open` MUST devolver `MemoryError::SchemaNewer` o `MemoryError::SchemaDiverged` (`crates/clawg-core/src/memory/error.rs:58` y `:64`) y MUST NOT escribir nada en `memory.db`. Los mensajes MUST decir las versiones, que nada cambió, y qué hacer: instalar la versión que la escribió o `clawg restore` de un backup anterior.

#### Escenario: base más nueva
- GIVEN una base en la última versión del binario, N (hoy V12), con una fila N + 1 agregada en `refinery_schema_history`
- WHEN se abre
- THEN el error es `SchemaNewer { db: N + 1, binary: N }`, nombra `clawg restore`, y `memory.db` es byte a byte igual

#### Escenario: checksum divergente
- GIVEN una base al día con el checksum de V3 cambiado
- THEN el error es `SchemaDiverged { version: 3 }` y la base no cambió

**Tests:** `schema_plan_covers_every_history_shape`, `a_divergence_the_binary_knows_about_wins_over_newer` y `history_order_does_not_matter` (`backup.rs`); `a_newer_database_is_refused_untouched` y `a_diverged_database_is_refused_untouched` (`db.rs`).

### Snapshot antes de migrar

Con plan `Pending`, `open` MUST hacer un snapshot `pre-migration` antes de aplicar la primera migración pendiente (`db.rs:207`). Si falla, MUST devolver `MemoryError::Backup` y MUST NOT migrar. Con `Fresh` y `UpToDate` MUST NOT hacer snapshot.

#### Escenario
- GIVEN una base migrada hasta V7, con filas
- WHEN se abre con el binario actual
- THEN `backups/` tiene un snapshot `pre-migration` en V7 con esas filas, y la base quedó en la última versión

**Tests:** `opening_an_older_database_snapshots_it_before_migrating`, `opening_a_current_database_takes_no_snapshot`, `a_new_database_takes_no_snapshot` y `a_failed_snapshot_stops_the_migration` (`db.rs`).

## Contrato de embedding y pooling

### El modelo se carga con pooling mean

`FastEmbedLocal` MUST cargar multilingual-e5-small con pooling mean, el de su entrenamiento (`e5_model`, `crates/clawg-core/src/memory/embedder.rs:125`). Sin indicarlo, fastembed toma el token CLS de un modelo definido por el usuario, y entonces todos los pasajes de un corpus quedan a 0,90–0,91 de coseno de cualquier pregunta.

El id del modelo en el contrato de embedding MUST ser `multilingual-e5-small/mean` (`E5_MODEL_ID`, `embedder.rs:115`). El id anterior, `multilingual-e5-small` (`LEGACY_CLS_MODEL_ID`, `embedder.rs:120`), nombra los vectores CLS: el archivo del modelo es el mismo, el espacio no. El contrato no tiene una columna de pooling; el id es lo que distingue espacios, y el índice de documentación de los proyectos, que guarda el `model_id` de cada doc, se reindexa solo cuando cambia ([`specs/project-docs`](../project-docs/spec.md)).

**Test:** `the_e5_model_is_pooled_by_mean` y `the_model_id_names_its_pooling_apart_from_the_cls_vectors_of_before` en `embedder.rs`.

### Re-embebido de un store CLS al arrancar

Al arrancar, después de cargar el modelo y dentro del bootstrap del embedder, el daemon verifica el contrato (`verify_or_reembed`, `crates/clawg-core/src/memory/bootstrap.rs:269`, cableado en `production_deps`, `bootstrap.rs:231`, y `crates/clawg-cli/src/daemon/serve.rs:88`). Si el contrato guardado tiene `LEGACY_CLS_MODEL_ID` y el embedder cargado tiene `E5_MODEL_ID`, con la misma dimensión y los mismos prefijos (`is_pooling_upgrade`, `bootstrap.rs:255`, pura), MUST:

1. escribir un snapshot `pre-reembed` en `<data>/backups`; si falla, no re-embebe, y el intento termina como un fallo de verificación, que se reintenta con la espera de siempre ([`specs/ingest`](../ingest/spec.md), «Reintento del bootstrap»);
2. re-embeber `memory`, `episodic_trajectory` y `procedural_skill` con el embedder cargado (`reembed_all` en modo `ReInfer`, `crates/clawg-core/src/memory/maintenance.rs:467`);
3. habilitar la memoria sólo si las tres tablas terminaron sin fallas, que es cuando `reembed_all` escribe el contrato nuevo. Si no, el contrato queda como estaba, la memoria sigue deshabilitada y el estado es el de un contrato que no coincide: ese bootstrap no se reintenta, pero el próximo arranque vuelve a empezar, y las filas ya hechas dan los mismos bytes.

Mientras el snapshot y el re-embebido corren, el estado del embedder MUST ser `loading`: el bootstrap publica `loading` antes de empezar y `ready` sólo cuando el embedder entró a su celda. Un save de ese lapso espera en la cola como con cualquier carga ([`specs/ingest`](../ingest/spec.md), «Embedder cargando»).

Cualquier otro contrato que no coincide (otro modelo, otra dimensión, otros prefijos, o el mismo CLS con un embedder que no es el mean) MUST seguir siendo la parada de siempre: memoria deshabilitada, sin snapshot y sin re-embeber.

#### Escenario: un store de vectores CLS
- GIVEN un store con contrato `multilingual-e5-small`, dim 384, y una memoria
- WHEN arranca con el embedder mean
- THEN queda un snapshot `…-pre-reembed.db`, el contrato pasa a `multilingual-e5-small/mean` y la verificación da `Match`

#### Escenario: otro modelo
- GIVEN un store con contrato de otro modelo
- WHEN arranca con el embedder mean
- THEN no hay snapshot, el contrato no cambia y la verificación da `Mismatch`

#### Escenario: un store que ya coincide
- GIVEN un store con contrato `multilingual-e5-small/mean`
- WHEN arranca con el embedder mean
- THEN no hay snapshot ni re-embebido

**Tests:** `only_the_cls_vectors_of_the_same_model_are_a_pooling_upgrade`, `cls_vectors_are_snapshotted_then_reembedded_by_mean`, `any_other_mismatch_is_left_as_it_is` y `a_store_that_matches_is_not_touched` en `crates/clawg-core/src/memory/bootstrap.rs`; el nombre `pre-reembed` en `snapshot_names_sort_in_time_order` (`backup.rs`). La regla de que el contrato se escribe al final está en los tests de `reembed_all` de `maintenance.rs`.

## Snapshots

### Un snapshot es completo y no se ve a medias

Un snapshot MUST contener todo lo commiteado, incluido lo que está sólo en el WAL, con las tablas vec0 y FTS consultables. MUST escribirse a un `.tmp` en el directorio de backups y renombrarse al terminar; uno fallido MUST NOT dejar archivos (`snapshot_into`, `backup.rs:275`).

El nombre MUST ser `memory-<AAAA-MM-DD>T<hh-mm-ss.mmm>Z-<motivo>.db` en UTC, de una función pura (`snapshot_name`, `backup.rs:176`), de modo que el orden alfabético sea el cronológico. Por ejemplo, 1791827476123 ms y `manual` dan `memory-2026-10-12T17-51-16.123Z-manual.db`.

**Tests:** `a_snapshot_includes_writes_still_in_the_wal`, `a_snapshot_keeps_vectors_and_full_text_searchable`, `snapshot_names_sort_in_time_order`, `snapshot_names_parse_back_to_their_reason` y `a_failed_snapshot_leaves_no_file_behind` (`backup.rs`).

### Retención

Después de cada snapshot automático, el directorio de backups MUST conservar los 5 automáticos más nuevos (`AUTO_KEEP`, `backup.rs:231`) y borrar los demás automáticos. Los `manual` MUST NOT borrarse nunca, y un archivo con otro nombre MUST NOT tocarse. La elección es pura (`to_prune`, `backup.rs:236`); el borrado es best-effort.

#### Escenario
- GIVEN 7 snapshots automáticos, 2 manuales y un `notas.txt`
- WHEN se hace un snapshot automático más
- THEN quedan los 5 automáticos más nuevos, los 2 manuales y `notas.txt`

**Tests:** `pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one` y `an_automatic_snapshot_prunes_older_automatic_ones` (`backup.rs`).

## Backup y restore

### `clawg backup`

`clawg backup [--profile P]` (`crates/clawg-cli/src/main.rs:122`) MUST producir un snapshot `manual` e imprimir su ruta.

- Con el daemon andando, MUST pedirlo por `POST /v1/backup` (con el token; `crates/clawg-cli/src/daemon/api.rs:837`, ruta en `:472`), que lo hace sobre la conexión del daemon (`MemoryRuntime::snapshot`, `crates/clawg-core/src/runtime.rs:191`) y responde `{"path": "..."}`.
- Sin daemon, MUST tomar el lock del store con el titular `Maintenance` (`crates/clawg-core/src/memory/store_lock.rs:59`), copiar sin migrar y soltar el lock (`backup_offline`, `backup.rs:306`).
- Sin `memory.db`, MUST decir que no hay base, sin crear el directorio de datos.

**Tests:** `backup_endpoint_writes_a_manual_snapshot` (`crates/clawg-cli/tests/api.rs`), `backup_takes_only_a_profile` (`cli.rs`), `an_offline_backup_copies_without_migrating`, `an_offline_backup_refuses_a_held_store` y `an_offline_backup_of_nothing_says_so` (`backup.rs`).

### `clawg restore`

`clawg restore <ARCHIVO> [--profile P]` (`main.rs:139`, `restore`, `backup.rs:329`) MUST, en este orden:

1. crear el directorio de datos si no existe, y tomar el lock; si lo tiene el daemon, MUST fallar diciendo que se pare con `clawg stop` (`crates/clawg-cli/src/restore.rs:23`);
2. validar el archivo en sólo lectura: `PRAGMA quick_check` = `ok`, con historial de refinery, y un plan que no sea `Newer`, `Diverged` ni `Fresh` (`check_restorable`, `backup.rs:392`). Si no pasa, MUST fallar sin tocar nada;
3. copiar el archivo a `memory.db.restore-tmp` con `fsync` por un handle abierto para escritura, porque en Windows uno de sólo lectura falla con "Access is denied" (`backup.rs:350`), antes del paso 4, cuya poda puede borrar el archivo que se restaura;
4. si existe `memory.db`, hacer un snapshot `pre-restore` y un `wal_checkpoint(TRUNCATE)` (`backup.rs:357`). Si falla, MUST fallar sin reemplazar nada y sin dejar el temporal;
5. borrar `memory.db-wal` y `memory.db-shm` **antes** del `rename` del temporal sobre `memory.db` (`backup.rs:378`): al revés, un corte dejaría el WAL viejo junto a la base nueva;
6. imprimir el archivo restaurado y la ruta del `pre-restore`.

Una base restaurada con plan `Pending` MUST abrir en el próximo arranque, que la migra con su propio snapshot.

#### Escenario: restore
- GIVEN una base con las filas A y B, y un snapshot que sólo tiene A
- WHEN se restaura el snapshot
- THEN la base tiene sólo A, `backups/` tiene un `pre-restore` con A y B, y no quedan temporales

#### Escenario: archivo inválido
- GIVEN un archivo de texto, una base SQLite sin historial, una base más nueva que el binario y una ruta que no existe
- THEN cada uno falla nombrando el archivo, y `memory.db` es byte a byte igual

**Tests:** `restore_replaces_the_store_and_keeps_the_previous_one`, `restore_refuses_files_it_cannot_open_safely`, `restore_refuses_while_the_daemon_holds_the_store`, `a_restored_older_snapshot_migrates_on_the_next_open`, `restoring_the_oldest_kept_automatic_snapshot_survives_the_pruning` y `restore_onto_a_machine_without_a_data_directory` (`backup.rs`); `a_store_held_by_the_daemon_says_to_stop_it` y `restore_takes_a_file_and_a_profile` (`clawg-cli`).

## Mensajes y actualización

### `clawg status` muestra el esquema sin daemon

Sin daemon, el JSON de `clawg status` MUST tener un bloque `store` (`store_block`, `crates/clawg-cli/src/status.rs:48`) con `state` (`missing`, `fresh`, `up_to_date`, `pending`, `newer`, `diverged` o `unreadable`), `db_version`, `binary_version` y un `message` cuando el estado impide arrancar. Con daemon, `store` MUST ser `null` (`main.rs:91`).

La lectura (`probe`, `backup.rs:442`) MUST hacerse sin lock, sin migrar y sin escribir, lo que incluye no dejar `memory.db-wal` ni `memory.db-shm`: sin `-wal`, la base se abre con la URI `immutable=1` (`open_for_probe` e `immutable_uri`, `backup.rs:464` y `:479`).

**Tests:** `a_stopped_daemon_reports_the_store_schema` y `a_store_that_would_open_carries_no_message` (`status.rs`); `probing_a_newer_store_reads_without_writing`, `probing_without_a_store_creates_none`, `probing_a_file_that_is_not_a_store_says_unreadable` e `immutable_uris_escape_what_a_uri_cannot_hold` (`backup.rs`).

### MCP nombra `clawg status` cuando el daemon no arrancó

Con `StartTimeout`, el texto de la herramienta MUST decir que clawg intentó arrancar su daemon y que `clawg status` muestra la causa (`crates/clawg-cli/src/mcp/server.rs:263`). Con `NotRunning` (autoarranque apagado) MUST seguir pidiendo `clawg serve`.

**Tests:** `a_daemon_that_did_not_start_points_at_status` (`server.rs`) y `a_tool_call_when_the_daemon_never_comes_up_gives_the_readable_error` (`mcp/mod.rs`).

### El instalador aborta si el daemon sigue vivo

Después de `clawg stop --profile prod`, `scripts/install.ps1` MUST ejecutar `clawg status --profile prod` con el binario anterior (`install.ps1:177`). Si sale con 0, MUST abortar antes de mover el binario, diciendo que el daemon sigue corriendo y que el binario no se reemplazó.

**Verificación:** manual en Windows; no hay PowerShell en Linux ni en CI.

## Archivos del workspace

### Escritura atómica de `workspace.md`

`regenerate_auto_section` (`crates/clawg-core/src/memory/workspace.rs:57`) MUST escribir a un archivo temporal en el mismo directorio y hacer `rename` sobre `workspace.md` (`write_atomically`, `workspace.rs:413`). Si la escritura falla, `workspace.md` MUST quedar con su contenido anterior, sección USER incluida, y MUST NOT quedar ningún temporal.

#### Escenario
- GIVEN un `workspace.md` con una sección USER escrita a mano
- WHEN la escritura del temporal falla
- THEN `workspace.md` es idéntico al anterior y no queda ningún temporal

**Test:** `a_failed_workspace_write_leaves_the_previous_file_intact` (`workspace.rs`).
