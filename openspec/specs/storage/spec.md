# storage — Especificación

Fuente de verdad del comportamiento actual del almacenamiento: el outbox donde queda un save aceptado hasta que termina de guardarse, el apagado del daemon, la reinyección de lo pendiente al arrancar y la importación del volcado de versiones anteriores, la resolución del directorio de datos, el runner de migraciones y la verificación del esquema, el pooling del modelo y el re-embebido de un store CLS al arrancar, los snapshots, `clawg backup` y `clawg restore`, la escritura de `workspace.md`, y lo que `clawg status`, MCP y el instalador dicen cuando la base no se puede abrir.

**Origen:**
- [data-preservation](../../changes/archive/2026-10-12-data-preservation/proposal.md) (partes A + F), archivado el 2026-10-12.
- [storage-safety-net](../../changes/archive/2026-10-12-storage-safety-net/proposal.md) (partes B + C de data-preservation), archivado el 2026-10-12.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Contrato de embedding y pooling", el motivo `pre-reembed` y la fila de V12).
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (sólo la fila de V13).
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (el outbox, el apagado y el arranque sin volcado, el volcado viejo y la fila de V14). El contrato de `POST /v1/memories` que lo usa está en [`specs/ingest`](../ingest/spec.md).

## Definiciones

- **Save aceptado:** uno que respondió `201` o `202`: su fila está confirmada en el outbox, o ya terminó de guardarse ([`specs/ingest`](../ingest/spec.md), «Aceptar es durable»).
- **Outbox:** la tabla `ingest_outbox` de `memory.db` (migración V14, `crates/clawg-core/migrations/V14__ingest_outbox.sql`). Una fila por save aceptado y no terminado: `operation_id` y `agent_id` (únicos juntos), `content` (el texto ya enmascarado, en su propia columna para que `purge` lo encuentre con `instr`), `event` (el `MemoryEvent` en JSON con `content` vacío), `state` (`pending` o `failed`), `attempts`, `last_error`, `progress`, `created_at` y `updated_at`. El `id` es `AUTOINCREMENT` porque el worker recuerda por id las filas que ya tomó en esta ejecución, y un id reusado se leería como una ya vista.
- **Progreso:** la lista de posiciones de un save ya escritas (`NotedPiece`, `outbox.rs:84`): para cada una, su posición, el hash de su texto, la fila de `memory` que la tiene, si este save la insertó o la reforzó, y la cabecera con la que se embebió.
- **Tope de intentos (`MAX_ATTEMPTS`, `outbox.rs:24`):** 3.
- **Apagado ordenado:** el que empieza con Ctrl-C o con `POST /v1/shutdown`. Un kill del proceso no es un apagado ordenado.
- **Plazo de drenaje (`DRAIN_DEADLINE`, `crates/clawg-core/src/runtime.rs:54`):** el tiempo máximo que el apagado dedica a ingerir lo encolado.
- **Volcado viejo:** el archivo `<data>/ingest-pending.jsonl` (`SPILL_FILE`, `crates/clawg-core/src/memory/spill.rs:11`), un `MemoryEvent` en JSON por línea, que las versiones anteriores al outbox escribían en un apagado ordenado. Ninguna versión actual lo escribe.
- **Directorio de backups:** `<data>/backups/` (`backups_dir`, `crates/clawg-core/src/paths.rs`).
- **Snapshot:** una copia completa y autónoma de `memory.db` hecha con `VACUUM INTO`. Incluye lo commiteado en el WAL y no tiene archivo WAL propio.
- **Motivo** de un snapshot (`Reason`, `crates/clawg-core/src/memory/backup.rs:119`): `pre-migration`, `pre-restore` y `pre-reembed` son **automáticos**; `manual` es el de `clawg backup`.
- **Plan de esquema** (`SchemaPlan`, `backup.rs:63`): la comparación entre el historial de `refinery_schema_history` y las migraciones embebidas en el binario. `Fresh` (sin historial), `UpToDate`, `Pending { from, to }` (un prefijo; faltan de `from + 1` a `to`), `Newer { db, binary }` (una versión aplicada mayor que la última del binario) o `Diverged { version }` (otro nombre o checksum, o un hueco).

## Apagado sin pérdida

### Un save aceptado está en el outbox hasta que termina

Un save que termina de guardarse MUST NOT dejar su fila. Cada pieza que el worker inserta o refuerza MUST anotar su posición en el progreso de la fila en la misma transacción que la escribe (`apply_note`, `crates/clawg-core/src/memory/outbox.rs:116`, llamado desde `insert_noting` y `reinforce_noting`). El vínculo de `memory_piece` ([`specs/pieces`](../pieces/spec.md)) y el borrado de la fila MUST ir en una transacción (`outbox_finish`, `outbox.rs:329`, que usa `link_in`, `crates/clawg-core/src/memory/pieces.rs:207`). Las copias repetidas dentro del mismo save se refuerzan después de anotar su posición y sin nota: un kill en ese punto pierde a lo sumo un refuerzo, nunca agrega uno.

#### Escenario
- GIVEN un daemon con el embedder listo
- WHEN un save de 3 piezas responde `201`
- THEN el outbox está vacío y `memory_piece` tiene las 3 piezas

**Tests:** `a_confirmed_save_leaves_no_outbox_row` (`crates/clawg-cli/tests/api.rs`; comprueba el outbox vacío, no el vínculo); `each_piece_is_noted_with_the_row_it_wrote` (`worker.rs`, mod tests); `finishing_links_the_pieces_and_drops_the_row_together` y `a_note_inside_a_transaction_is_the_progress_the_next_claim_reads` (`outbox.rs`, mod tests).

### Un replay retoma donde quedó

Al tomar una fila con progreso, el worker MUST saltar las posiciones anotadas, MUST NOT reforzar sus filas y MUST vincular cada una con la cabecera anotada (`ingest_pieces`, `worker.rs:643-689`). Si el progreso no encaja con el troceo de hoy (nombra una posición que no existe, repite una, o el hash anotado no es el del texto de esa posición), MUST descartarlo con un warn y seguir por el dedup (`resume_positions`, `outbox.rs:96`, pura): sin saber qué fila tiene cada pieza, el dedup es lo único que evita duplicarlas.

#### Escenario
- GIVEN un save de 3 piezas o más cuyo progreso anota las dos primeras, como si el proceso hubiera muerto antes de la tercera
- WHEN el worker toma la fila
- THEN inserta sólo la tercera, el `access_count` de cada pieza sigue en 1 y `memory_piece` tiene todas con la cabecera original en las piezas posteriores a la primera

**Tests:** `a_replayed_save_resumes_after_its_last_noted_piece` y `progress_that_does_not_fit_the_split_is_dropped` (`worker.rs`, mod tests); `resume_positions_places_each_noted_piece_and_refuses_one_that_does_not_fit` (`outbox.rs`, mod tests).

### Una entrega doble se salta

El worker MUST tomar cada fila a lo sumo una vez por ejecución (`take_row`, `worker.rs:415`, con el conjunto `taken` de `memory_worker`, `worker.rs:256`): una fila que el handler escribió antes de la lectura de arranque llega también por el canal. Una fila que ya no existe, o que está `failed`, MUST saltarse sin escribir nada (`Claimed::Skip`, `outbox.rs:138`). Si la segunda entrega trae un canal de respuesta y la primera vino de la lectura de arranque, el que espera recibe el reporte de la primera.

#### Escenario
- GIVEN una fila pendiente que llega por la lectura de arranque y por el canal
- THEN se procesa una vez y la fila del texto tiene `access_count = 1`

**Tests:** `a_row_delivered_twice_is_ingested_once` (`worker.rs`, mod tests); el salto de una fila inexistente o `failed` lo cubre `claiming_spends_an_attempt_and_gives_up_once_none_is_left` (`outbox.rs`, mod tests), a nivel del store y no del worker.

### Intentos y fallo a la vista

Tomar una fila MUST sumar un intento antes de procesarla (`outbox_claim`, `outbox.rs:252`). Si la fila ya tenía `MAX_ATTEMPTS`, MUST quedar `failed` con la causa `the daemon stopped N times while ingesting this save`, sin procesarse, y el worker MUST seguir con la siguiente (`claim_step`, `outbox.rs:63`, pura): tres cortes del proceso seguidos con el mismo save dicen que el save es lo que lo mata.

Un fallo del worker MUST anotar su causa en `last_error` (`outbox_fail`, `outbox.rs:299`). Un fallo transitorio (`IngestFailure::EmbedderUnavailable`) MUST NOT contar como intento. Con el tope alcanzado la fila MUST quedar `failed`; si no, `pending` y se reintenta en el próximo arranque, no en la misma ejecución, porque el mismo texto falla igual (`after_failure`, `outbox.rs:74`, pura). Una fila cuyo `event` ya no parsea MUST darse por perdida con esa causa en vez de bloquear a las demás (`outbox_pending`, `outbox.rs:345`).

Si el handler contesta al que guardó con el fallo (`500` o `503`), MUST borrar la fila ([`specs/ingest`](../ingest/spec.md), «Un fallo del worker llega al que guardó»). Una fila `failed` MUST NOT borrarse sola: queda a la vista en `/health` ([`specs/ingest`](../ingest/spec.md), «Estado del outbox en `/health`»).

#### Escenario: evento que corta el proceso
- GIVEN una fila con `attempts = 3`
- WHEN el worker la toma
- THEN queda `failed`, no se escribe ninguna memoria y la fila siguiente se procesa

#### Escenario: fallo después de un `202`
- GIVEN un save aceptado con el embedder cargando y un embedder cuyo vector rompe el insert (NaN)
- WHEN el worker lo procesa
- THEN la fila queda `pending` con `attempts = 1` y `last_error` con la causa (`store insert failed: …`)

#### Escenario: fallo contestado
- GIVEN un daemon con un embedder cuyo batch falla
- WHEN un save responde `500 ingest_failed`
- THEN el outbox está vacío

**Tests:** `claim_step_processes_until_the_attempts_run_out` y `after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt` (`outbox.rs`, mod tests); `claiming_spends_an_attempt_and_gives_up_once_none_is_left`, `a_failure_keeps_its_cause_and_a_transient_one_its_attempt` y `an_event_that_does_not_parse_is_given_up_with_its_cause` (`outbox.rs`, mod tests); `a_row_that_stopped_the_process_three_times_fails_and_the_next_one_runs` y `a_failure_after_the_answer_stays_in_the_outbox_with_its_cause` (`worker.rs`, mod tests); `a_failure_the_caller_was_told_leaves_no_outbox_row` (`tests/api.rs`). Que un fallo transitorio devuelva su intento está probado en el store y en la función pura, no a través del worker; que no se reintente en la misma ejecución no tiene test.

### Ningún save aceptado se pierde en un apagado ordenado

En un apagado ordenado, cada save aceptado MUST terminar en uno de dos lugares: guardado en la base (o absorbido por el gate de dedup), o en el outbox para el próximo arranque. El apagado MUST seguir drenando hasta `DRAIN_DEADLINE`, MUST NOT escribir `ingest-pending.jsonl` y MUST NOT descartar un save: lo que el worker no empezó queda en su fila.

La decisión de seguir drenando o dejar la cola es pura (`drain_step`, `crates/clawg-core/src/memory/worker.rs:198`; su variante `DrainStep::Spill` conserva el nombre de cuando el apagado volcaba a un archivo). `MemoryRuntime::shutdown` (`crates/clawg-core/src/runtime.rs:267`) espera al worker, y el daemon la llama antes de borrar su estado (`crates/clawg-cli/src/daemon/serve.rs:175`). Sólo un evento encolado sin fila (los caminos en proceso de tests y evaluación) puede quedar sin ingerir, y el apagado lo registra con un warn (`runtime.rs:283`).

#### Escenario: embedder listo
- GIVEN el embedder cargado y 20 saves aceptados
- WHEN se hace un apagado ordenado
- THEN la base tiene las filas de los 20, el outbox está vacío y no existe `ingest-pending.jsonl`

#### Escenario: embedder sin cargar
- GIVEN el embedder sin cargar y 3 saves aceptados (uno retenido por `hold_for_embedder`)
- WHEN se hace un apagado ordenado
- THEN no existe `ingest-pending.jsonl` y el outbox tiene las 3 filas; al arrancar con el embedder listo se ingieren en orden

#### Escenario: vence el plazo
- GIVEN un embedder que tarda más que `DRAIN_DEADLINE` en procesar los saves encolados
- WHEN se hace un apagado ordenado
- THEN el save en curso termina su insert, y los que no empezaron quedan en el outbox

**Tests:** `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_leaves_saves_held_for_the_embedder_in_the_outbox` y `shutdown_leaves_what_the_drain_deadline_left_unstarted_in_the_outbox` (`crates/clawg-core/tests/shutdown.rs`), más los unit de `drain_step` en `worker.rs` (`a_drain_keeps_ingesting_until_its_deadline` y `a_drain_without_an_embedder_spills_at_once`).

### Arranque: se ingiere lo pendiente antes que lo nuevo

Al arrancar, el worker (`ingest_worker_future`, `crates/clawg-core/src/runtime.rs:401`) MUST, en este orden:

1. importar `ingest-pending.jsonl` si existe (`import_spill`, `runtime.rs:431`): cada evento válido pasa a una fila, enmascarado, todos en una transacción con `synchronous = FULL` (`outbox_import`, `outbox.rs:234`); el archivo MUST borrarse sólo después del commit. Si no se pudo borrar, se avisa: una segunda importación la absorbe el dedup;
2. procesar las filas `pending` en orden de id antes que cualquier evento del canal (`outbox_pending`, `outbox.rs:345`; `Replay`, `worker.rs:159`).

Si no se puede leer el outbox, el worker lo registra y sigue con el canal: esas filas esperan al próximo arranque.

#### Escenario: reinyección
- GIVEN dos saves aceptados que el apagado dejó en el outbox y un save nuevo, con el embedder listo
- WHEN arranca el worker
- THEN los 3 quedan en la base en ese orden y el outbox queda vacío

#### Escenario: proceso que murió
- GIVEN un save aceptado mientras el modelo cargaba y un proceso que terminó sin apagado ordenado
- WHEN se abre otro runtime sobre el mismo directorio con el embedder listo
- THEN la memoria queda en la base y el outbox vacío

**Tests:** `pending_rows_are_ingested_first_on_startup` y `a_save_accepted_while_the_model_loads_survives_a_process_that_dies` (`crates/clawg-core/tests/shutdown.rs`); `pending_rows_come_back_oldest_first_and_failed_ones_do_not` (`outbox.rs`, mod tests); con el binario y un kill duro, `a_save_accepted_before_a_hard_kill_is_stored_after_the_restart` (`crates/clawg-cli/tests/binary.rs`, ver [`specs/install`](../install/spec.md)).

### El volcado de una versión anterior

Una versión anterior al outbox escribía en un apagado ordenado los eventos sin ingerir en `ingest-pending.jsonl`. El volcado MUST seguir importándose al arrancar (arriba), con los mismos criterios de siempre: una línea que no se puede parsear MUST NOT bloquear las demás; se registra con un warn y se mueve a `<data>/ingest-pending.rejected.jsonl` (`REJECTED_FILE`, `crates/clawg-core/src/memory/spill.rs:15`), que nada borra. Si no se puede guardar en ese archivo, el volcado MUST NOT borrarse (`Spilled::file`, `spill.rs:23`), porque es la única copia de esas líneas; las líneas válidas ya están en el outbox, así que el próximo arranque las importa otra vez y el dedup las absorbe. El contenido de cada evento importado MUST enmascararse antes de entrar al outbox. `parse_lines` (`spill.rs:96`) es pura y `load` (`spill.rs:49`) lee el archivo y aparta lo ilegible; un evento escrito antes de `derived_from` MUST leerse con `derived_from` vacío ([`specs/synthesis`](../synthesis/spec.md)).

#### Escenario: volcado viejo
- GIVEN un `ingest-pending.jsonl` con dos eventos válidos (uno con un secreto) y una línea truncada
- WHEN arranca el worker
- THEN las dos memorias quedan en la base, la del secreto enmascarada, el archivo ya no existe y la línea truncada está en `ingest-pending.rejected.jsonl`

**Tests:** `an_old_spill_is_imported_masked_and_removed` (`crates/clawg-core/tests/shutdown.rs`), `imported_events_get_rows_of_their_own` (`outbox.rs`, mod tests), `spill_lines_round_trip_events_with_newlines_and_overrides` e `invalid_spill_lines_are_set_apart_and_the_valid_ones_keep_their_order` (`spill.rs`, mod tests). Que el archivo no se borre cuando lo ilegible no se pudo guardar no tiene test.

### Flush final del `AccessTracker`

En un apagado ordenado, el tracker MUST escribir el buffer pendiente antes de terminar, y el apagado MUST esperar a que termine (`AccessTracker::shutdown`, `crates/clawg-core/src/memory/access.rs:134`). Lo mismo vale cuando se sueltan todos los `Sender`.

#### Escenario
- GIVEN un hit registrado con `record` hace menos de 30 s
- WHEN se hace un apagado ordenado
- THEN el `access_count` de esa fila subió en 1

**Tests:** `access_tracker_flushes_its_buffer_on_shutdown`, `access_tracker_flushes_its_buffer_when_every_sender_is_dropped` y `shutting_down_twice_is_harmless` (`access.rs`).

### Checkpoint al terminar

Después de drenar y del flush del tracker, el apagado SHOULD hacer `PRAGMA wal_checkpoint(TRUNCATE)` (`checkpoint`, `runtime.rs:378`). Un fallo del checkpoint MUST registrarse y MUST NOT impedir el resto del apagado.

#### Escenario
- GIVEN una base en WAL con escrituras recientes
- WHEN se hace un apagado ordenado
- THEN `memory.db-wal` no existe o mide 0 bytes, y una copia de `memory.db` sola contiene las escrituras

**Test:** `shutdown_checkpoints_the_wal_into_memory_db` (`crates/clawg-core/tests/shutdown.rs`).

### El plazo de drenaje cabe en `STOP_TIMEOUT`

`DRAIN_DEADLINE` más un margen para el save en curso, el flush y el checkpoint MUST NOT superar `STOP_TIMEOUT` (`crates/clawg-cli/src/stop.rs:19`). Hoy son 10 s + 5 s = 15 s. El doc de `STOP_TIMEOUT` MUST describir lo que el apagado hace.

**Test:** `the_drain_deadline_leaves_room_inside_the_stop_timeout` (`stop.rs`).

**Límite conocido:** el plazo acota el apagado ordenado, pero no la vida del proceso. Si el apagado llega mientras carga el modelo, el proceso sigue vivo hasta que esa carga vuelve: corre en `spawn_blocking` (`crates/clawg-core/src/memory/bootstrap.rs:492`), y el runtime de `#[tokio::main]` (`crates/clawg-cli/src/main.rs:37`) espera a esas tareas al soltarse. Los datos ya están a salvo para entonces: lo no ingerido está en el outbox. Está en el backlog de `openspec/README.md`.

## Directorio de datos

### Sin home no hay directorio de datos

Si no se puede resolver el home, la resolución MUST devolver un error tipado que lo diga (`PathsError`, `crates/clawg-core/src/paths.rs:149`), y MUST NOT usar el directorio actual. `clawg serve`, `clawg mcp`, `clawg status` y `clawg stop` MUST terminar con ese mensaje y sin crear ningún archivo.

La resolución es una función pura que recibe el home (`data_dir_in`, `paths.rs:159`), para no depender del entorno. En un build de test, el home puede venir de `CLAWG_TEST_HOME` ([`specs/install`](../install/spec.md)); el binario que se entrega usa siempre el del sistema.

#### Escenario
- GIVEN un resolvedor de home que devuelve `None`
- WHEN se calcula el directorio de datos
- THEN el resultado es `Err` y el mensaje nombra el home

**Tests:** `data_dir_without_a_home_is_an_error` y `data_dir_in_a_home_is_the_profile_directory_under_it` (`paths.rs`).

## Migraciones

### Migraciones atómicas

Las migraciones pendientes de un arranque MUST aplicarse en una sola transacción junto con sus filas de historial (`configured`, `crates/clawg-core/src/memory/db.rs:382`, con `set_grouped(true)`). Si una falla, la base MUST quedar como estaba antes del arranque.

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

Los checksums de las migraciones publicadas (hoy V1–V14) MUST coincidir con una lista fija en el código (`PINNED`, `crates/clawg-core/src/memory/migrations.rs:44`). Cambiar una migración publicada MUST romper un test. Una migración nueva agrega su fila a la lista en el mismo commit.

**Test:** `published_migration_checksums_are_pinned` (`migrations.rs`).

### Una base de la versión anterior abre con el outbox vacío

Una base en V13 MUST abrir con V14 con cada fila idéntica (`memory`, `memory_piece` y `project` incluidas) y el outbox vacío: los saves aceptados vivían sólo en memoria antes de esta tabla, y no hay nada que traspasar.

#### Escenario
- GIVEN una base migrada hasta V13, con memorias, un vínculo de piezas y un proyecto
- WHEN se aplica V14
- THEN cada fila es igual a la de antes y `ingest_outbox` tiene 0 filas

**Test:** `a_database_from_the_previous_version_opens_with_an_empty_outbox` (`db.rs`). Llama a `run_migrations` directo, así que el snapshot `pre-migration` de la apertura lo cubre la regla general de «Snapshot antes de migrar».

### Verificación del esquema antes de migrar

La decisión es pura (`schema_plan`, `backup.rs:83`). `MemoryStore::open` MUST calcularla antes de cualquier pragma y antes del runner (`check_schema`, `crates/clawg-core/src/memory/db.rs:388`, llamada en `db.rs:185`). Con `Newer` o `Diverged`, `open` MUST devolver `MemoryError::SchemaNewer` o `MemoryError::SchemaDiverged` (`crates/clawg-core/src/memory/error.rs:58` y `:64`) y MUST NOT escribir nada en `memory.db`. Los mensajes MUST decir las versiones, que nada cambió, y qué hacer: instalar la versión que la escribió o `clawg restore` de un backup anterior.

#### Escenario: base más nueva
- GIVEN una base en la última versión del binario, N (hoy V14), con una fila N + 1 agregada en `refinery_schema_history`
- WHEN se abre
- THEN el error es `SchemaNewer { db: N + 1, binary: N }`, nombra `clawg restore`, y `memory.db` es byte a byte igual

#### Escenario: checksum divergente
- GIVEN una base al día con el checksum de V3 cambiado
- THEN el error es `SchemaDiverged { version: 3 }` y la base no cambió

**Tests:** `schema_plan_covers_every_history_shape`, `a_divergence_the_binary_knows_about_wins_over_newer` y `history_order_does_not_matter` (`backup.rs`); `a_newer_database_is_refused_untouched` y `a_diverged_database_is_refused_untouched` (`db.rs`).

### Snapshot antes de migrar

Con plan `Pending`, `open` MUST hacer un snapshot `pre-migration` antes de aplicar la primera migración pendiente (`db.rs:213`). Si falla, MUST devolver `MemoryError::Backup` y MUST NOT migrar. Con `Fresh` y `UpToDate` MUST NOT hacer snapshot.

#### Escenario
- GIVEN una base migrada hasta V7, con filas
- WHEN se abre con el binario actual
- THEN `backups/` tiene un snapshot `pre-migration` en V7 con esas filas, y la base quedó en la última versión

**Tests:** `opening_an_older_database_snapshots_it_before_migrating`, `opening_a_current_database_takes_no_snapshot`, `a_new_database_takes_no_snapshot` y `a_failed_snapshot_stops_the_migration` (`db.rs`).

## Contrato de embedding y pooling

### El modelo se carga con pooling mean

`FastEmbedLocal` MUST cargar multilingual-e5-small con pooling mean, el de su entrenamiento (`e5_model`, `crates/clawg-core/src/memory/embedder.rs:138`). Sin indicarlo, fastembed toma el token CLS de un modelo definido por el usuario, y entonces todos los pasajes de un corpus quedan a 0,90–0,91 de coseno de cualquier pregunta.

El id del modelo en el contrato de embedding MUST ser `multilingual-e5-small/mean` (`E5_MODEL_ID`, `embedder.rs:128`). El id anterior, `multilingual-e5-small` (`LEGACY_CLS_MODEL_ID`, `embedder.rs:133`), nombra los vectores CLS: el archivo del modelo es el mismo, el espacio no. El contrato no tiene una columna de pooling; el id es lo que distingue espacios, y el índice de documentación de los proyectos, que guarda el `model_id` de cada doc, se reindexa solo cuando cambia ([`specs/project-docs`](../project-docs/spec.md)).

**Test:** `the_e5_model_is_pooled_by_mean` y `the_model_id_names_its_pooling_apart_from_the_cls_vectors_of_before` en `embedder.rs`.

### Re-embebido de un store CLS al arrancar

Al arrancar, después de cargar el modelo y dentro del bootstrap del embedder, el daemon verifica el contrato (`verify_or_reembed`, `crates/clawg-core/src/memory/bootstrap.rs:394`, cableado en `production_deps`, `bootstrap.rs:232`, que `serve_deps`, `bootstrap.rs:319`, entrega al daemon en `crates/clawg-cli/src/daemon/serve.rs:94`). Si el contrato guardado tiene `LEGACY_CLS_MODEL_ID` y el embedder cargado tiene `E5_MODEL_ID`, con la misma dimensión y los mismos prefijos (`is_pooling_upgrade`, `bootstrap.rs:380`, pura), MUST:

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

Un snapshot MUST contener todo lo commiteado, incluido lo que está sólo en el WAL, con las tablas vec0 y FTS consultables. MUST escribirse a un `.tmp` en el directorio de backups y renombrarse al terminar; uno fallido MUST NOT dejar archivos (`snapshot_into`, `backup.rs:280`).

El nombre MUST ser `memory-<AAAA-MM-DD>T<hh-mm-ss.mmm>Z-<motivo>.db` en UTC, de una función pura (`snapshot_name`, `backup.rs:180`), de modo que el orden alfabético sea el cronológico. Por ejemplo, 1791827476123 ms y `manual` dan `memory-2026-10-12T17-51-16.123Z-manual.db`.

Un snapshot MUST incluir el outbox, y con él el texto enmascarado de los saves que esperaban o habían fallado. Un `restore` MUST reemplazar las filas pendientes por las del snapshot: las de la base reemplazada quedan sólo en el `pre-restore` que `restore` ya escribe, y no se traspasan, porque mezclar filas de dos bases rompería el orden y los ids del progreso. `clawg purge` nombra los snapshots con una fila del outbox que contiene el texto purgado ([`specs/secrets`](../secrets/spec.md), R6).

**Tests:** `a_snapshot_includes_writes_still_in_the_wal`, `a_snapshot_keeps_vectors_and_full_text_searchable`, `snapshot_names_sort_in_time_order`, `snapshot_names_parse_back_to_their_reason` y `a_failed_snapshot_leaves_no_file_behind` (`backup.rs`). Que el snapshot lleve el outbox y que un `restore` lo reemplace no tienen test propio: es lo que hace copiar la base entera.

### Retención

Después de cada snapshot automático, el directorio de backups MUST conservar los 5 automáticos más nuevos (`AUTO_KEEP`, `backup.rs:231`) y borrar los demás automáticos. Los `manual` MUST NOT borrarse nunca, y un archivo con otro nombre MUST NOT tocarse. La elección es pura (`to_prune`, `backup.rs:236`); el borrado es best-effort.

#### Escenario
- GIVEN 7 snapshots automáticos, 2 manuales y un `notas.txt`
- WHEN se hace un snapshot automático más
- THEN quedan los 5 automáticos más nuevos, los 2 manuales y `notas.txt`

**Tests:** `pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one` y `an_automatic_snapshot_prunes_older_automatic_ones` (`backup.rs`).

## Backup y restore

### `clawg backup`

`clawg backup [--profile P]` (`crates/clawg-cli/src/main.rs:127`) MUST producir un snapshot `manual` e imprimir su ruta.

- Con el daemon andando, MUST pedirlo por `POST /v1/backup` (con el token; `crates/clawg-cli/src/daemon/api.rs:1130`, ruta en `:600`), que lo hace sobre la conexión del daemon (`MemoryRuntime::snapshot`, `crates/clawg-core/src/runtime.rs:253`) y responde `{"path": "..."}`.
- Sin daemon, MUST tomar el lock del store con el titular `Maintenance` (`crates/clawg-core/src/memory/store_lock.rs:59`), copiar sin migrar y soltar el lock (`backup_offline`, `backup.rs:311`).
- Sin `memory.db`, MUST decir que no hay base, sin crear el directorio de datos.

**Tests:** `backup_endpoint_writes_a_manual_snapshot` (`crates/clawg-cli/tests/api.rs`), `backup_takes_only_a_profile` (`cli.rs`), `an_offline_backup_copies_without_migrating`, `an_offline_backup_refuses_a_held_store` y `an_offline_backup_of_nothing_says_so` (`backup.rs`).

### `clawg restore`

`clawg restore <ARCHIVO> [--profile P]` (`main.rs:144`, `restore`, `backup.rs:334`) MUST, en este orden:

1. crear el directorio de datos si no existe, y tomar el lock; si lo tiene el daemon, MUST fallar diciendo que se pare con `clawg stop` (`crates/clawg-cli/src/restore.rs:23`);
2. validar el archivo en sólo lectura: `PRAGMA quick_check` = `ok`, con historial de refinery, y un plan que no sea `Newer`, `Diverged` ni `Fresh` (`check_restorable`, `backup.rs:397`). Si no pasa, MUST fallar sin tocar nada;
3. copiar el archivo a `memory.db.restore-tmp` con `fsync` por un handle abierto para escritura, porque en Windows uno de sólo lectura falla con "Access is denied" (`stage`, `backup.rs:355`), antes del paso 4, cuya poda puede borrar el archivo que se restaura;
4. si existe `memory.db`, hacer un snapshot `pre-restore` y un `wal_checkpoint(TRUNCATE)` (`keep_and_replace`, `backup.rs:362`). Si falla, MUST fallar sin reemplazar nada y sin dejar el temporal;
5. borrar `memory.db-wal` y `memory.db-shm` **antes** del `rename` del temporal sobre `memory.db` (`swap_in`, `backup.rs:383`): al revés, un corte dejaría el WAL viejo junto a la base nueva;
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
