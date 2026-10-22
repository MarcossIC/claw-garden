# storage — Especificación

Fuente de verdad del comportamiento actual del almacenamiento: el outbox donde queda un save aceptado hasta que termina de guardarse, el apagado del daemon, la reinyección de lo pendiente al arrancar y la importación del volcado de versiones anteriores, la resolución del directorio de datos, el runner de migraciones y la verificación del esquema, el pooling del modelo y el re-embebido de un store CLS al arrancar, la descarga del modelo (la caché, los plazos de red, un hash por archivo fuera de los hilos del runtime y el origen `CLAWG_HF_BASE_URL`), el ciclo de vida del embedder (soltar el modelo ocioso, recargarlo desde la caché y repararla, recuperar un hilo de inferencia muerto, un solo tokenizer por proceso y la memoria que vuelve al sistema), los snapshots, `clawg backup` y `clawg restore`, la escritura de `workspace.md`, el log del daemon (su rotación y la versión en su primera línea), y lo que `clawg status`, MCP y el instalador dicen cuando la base no se puede abrir.

**Origen:**
- [data-preservation](../../changes/archive/2026-10-12-data-preservation/proposal.md) (partes A + F), archivado el 2026-10-12.
- [storage-safety-net](../../changes/archive/2026-10-12-storage-safety-net/proposal.md) (partes B + C de data-preservation), archivado el 2026-10-12.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Contrato de embedding y pooling", el motivo `pre-reembed` y la fila de V12).
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (sólo la fila de V13).
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (el outbox, el apagado y el arranque sin volcado, el volcado viejo y la fila de V14). El contrato de `POST /v1/memories` que lo usa está en [`specs/ingest`](../ingest/spec.md).
- [embedder-idle-unload](../../changes/archive/2026-10-17-embedder-idle-unload/proposal.md), archivado el 2026-10-17 (el ciclo de vida del embedder y la reescritura de «El modelo se carga con pooling mean» para `E5Embedder`). El campo `embedder_resident` de `/health` está en [`specs/mcp`](../mcp/spec.md).
- [conflict-candidates](../../changes/archive/2026-10-18-conflict-candidates/proposal.md), archivado el 2026-10-18 (sólo la fila de V15 y «Una base en V14 abre con sus disputas como pares»). La tabla de pares y su conversión están en [`specs/disputes`](../disputes/spec.md).
- [topic-revisions](../../changes/archive/2026-10-18-topic-revisions/proposal.md), archivado el 2026-10-18 (sólo la fila de V16 y «Una base en V15 abre con sus filas sin clave»). Qué hace un save con clave está en [`specs/ingest`](../ingest/spec.md).
- [model-download](../../changes/archive/2026-10-19-model-download/proposal.md), archivado el 2026-10-19 (la «Descarga del modelo», y la recarga desde la caché y su reparación en «Recargar a demanda, una sola vez» y «Una recarga que falla no deja nada roto»). Lo que dice el error de una descarga que agota sus intentos está en [`specs/ingest`](../ingest/spec.md), «Estado en `/health` y `clawg status`».
- [embedder-recovery](../../changes/archive/2026-10-19-embedder-recovery/proposal.md), archivado el 2026-10-19 («Un pánico en la inferencia contesta y suelta la sesión», «Un hilo muerto se recupera en la misma llamada», y el `IdleEmbedder` sin barrido con ocio `0`).
- [outbox-lifecycle](../../changes/archive/2026-10-19-outbox-lifecycle/proposal.md), archivado el 2026-10-19 (el reintento de un fallo pasajero en la misma ejecución, la regla única de `failed`, la pausa del apagado entre pedazos y «Una fila `failed` se reintenta o se descarta»). Lo que contesta un save que falla a medias está en [`specs/ingest`](../ingest/spec.md).
- [shutdown-budget](../../changes/archive/2026-10-20-shutdown-budget/proposal.md), archivado el 2026-10-20 (SIGTERM como apagado ordenado, «Cada fase del apagado tiene tope» y «Las fases caben en `STOP_TIMEOUT`»). Lo que hace `clawg update` cuando `stop` vence está en [`specs/self-update`](../self-update/spec.md).
- [serve-log-rotation](../../changes/archive/2026-10-22-serve-log-rotation/proposal.md), archivado el 2026-10-22 («El log del daemon»).

## Definiciones

- **Save aceptado:** uno que respondió `201` o `202`: su fila está confirmada en el outbox, o ya terminó de guardarse ([`specs/ingest`](../ingest/spec.md), «Aceptar es durable»).
- **Outbox:** la tabla `ingest_outbox` de `memory.db` (migración V14, `crates/clawg-core/migrations/V14__ingest_outbox.sql`). Una fila por save aceptado y no terminado: `operation_id` y `agent_id` (únicos juntos), `content` (el texto ya enmascarado, en su propia columna para que `purge` lo encuentre con `instr`), `event` (el `MemoryEvent` en JSON con `content` vacío), `state` (`pending` o `failed`), `attempts`, `last_error`, `progress`, `created_at` y `updated_at`. El `id` es `AUTOINCREMENT` porque el worker recuerda por id las filas que ya tomó en esta ejecución, y un id reusado se leería como una ya vista.
- **Progreso:** la lista de posiciones de un save ya escritas (`NotedPiece`, `outbox.rs:86`): para cada una, su posición, el hash de su texto, la fila de `memory` que la tiene, si este save la insertó o la reforzó, y la cabecera con la que se embebió.
- **Tope de intentos (`MAX_ATTEMPTS`, `outbox.rs:26`):** 3.
- **Toma:** `outbox_claim` de una fila (`outbox.rs:299`), que gasta un intento.
- **Fallo pasajero:** `IngestFailure::EmbedderUnavailable`: el modelo no está, o su recarga no pudo hacerse.
- **Apagado ordenado:** el que empieza con Ctrl-C, con SIGTERM (Unix) o con `POST /v1/shutdown`. Un SIGKILL, o en Windows el fin del proceso, no es un apagado ordenado.
- **Plazo de drenaje (`DRAIN_DEADLINE`, `crates/clawg-core/src/runtime.rs:56`):** el tiempo máximo que el apagado dedica a ingerir lo encolado.
- **Volcado viejo:** el archivo `<data>/ingest-pending.jsonl` (`SPILL_FILE`, `crates/clawg-core/src/memory/spill.rs:11`), un `MemoryEvent` en JSON por línea, que las versiones anteriores al outbox escribían en un apagado ordenado. Ninguna versión actual lo escribe.
- **Directorio de backups:** `<data>/backups/` (`backups_dir`, `crates/clawg-core/src/paths.rs`).
- **Snapshot:** una copia completa y autónoma de `memory.db` hecha con `VACUUM INTO`. Incluye lo commiteado en el WAL y no tiene archivo WAL propio.
- **Motivo** de un snapshot (`Reason`, `crates/clawg-core/src/memory/backup.rs:129`): `pre-migration`, `pre-restore` y `pre-reembed` son **automáticos**; `manual` es el de `clawg backup`.
- **Plan de esquema** (`SchemaPlan`, `backup.rs:73`): la comparación entre el historial de `refinery_schema_history` y las migraciones embebidas en el binario. `Fresh` (sin historial), `UpToDate`, `Pending { from, to }` (un prefijo; faltan de `from + 1` a `to`), `Newer { db, binary }` (una versión aplicada mayor que la última del binario) o `Diverged { version }` (otro nombre o checksum, o un hueco).

## Apagado sin pérdida

### Un save aceptado está en el outbox hasta que termina

Un save que termina de guardarse MUST NOT dejar su fila. Cada pieza que el worker inserta o refuerza MUST anotar su posición en el progreso de la fila en la misma transacción que la escribe (`apply_note`, `crates/clawg-core/src/memory/outbox.rs:118`, llamado desde `insert_noting` y `reinforce_noting`). El vínculo de `memory_piece` ([`specs/pieces`](../pieces/spec.md)) y el borrado de la fila MUST ir en una transacción (`outbox_finish`, `outbox.rs:459`, que usa `link_in`, `crates/clawg-core/src/memory/pieces.rs:228`). Las copias repetidas dentro del mismo save se refuerzan después de anotar su posición y sin nota: un kill en ese punto pierde a lo sumo un refuerzo, nunca agrega uno.

#### Escenario
- GIVEN un daemon con el embedder listo
- WHEN un save de 3 piezas responde `201`
- THEN el outbox está vacío y `memory_piece` tiene las 3 piezas

**Tests:** `a_confirmed_save_leaves_no_outbox_row` (`crates/clawg-cli/tests/api.rs`; comprueba el outbox vacío, no el vínculo); `each_piece_is_noted_with_the_row_it_wrote` (`worker.rs`, mod tests); `finishing_links_the_pieces_and_drops_the_row_together` y `a_note_inside_a_transaction_is_the_progress_the_next_claim_reads` (`outbox.rs`, mod tests).

### Un replay retoma donde quedó

Al tomar una fila con progreso, el worker MUST saltar las posiciones anotadas, MUST NOT reforzar sus filas y MUST vincular cada una con la cabecera anotada (`ingest_pieces`, `worker.rs:715`). Si el progreso no encaja con el troceo de hoy (nombra una posición que no existe, repite una, o el hash anotado no es el del texto de esa posición), MUST descartarlo con un warn y seguir por el dedup (`resume_positions`, `outbox.rs:98`, pura): sin saber qué fila tiene cada pieza, el dedup es lo único que evita duplicarlas.

#### Escenario
- GIVEN un save de 3 piezas o más cuyo progreso anota las dos primeras, como si el proceso hubiera muerto antes de la tercera
- WHEN el worker toma la fila
- THEN inserta sólo la tercera, el `access_count` de cada pieza sigue en 1 y `memory_piece` tiene todas con la cabecera original en las piezas posteriores a la primera

**Tests:** `a_replayed_save_resumes_after_its_last_noted_piece` y `progress_that_does_not_fit_the_split_is_dropped` (`worker.rs`, mod tests); `resume_positions_places_each_noted_piece_and_refuses_one_that_does_not_fit` (`outbox.rs`, mod tests).

### Una entrega doble se salta

El worker MUST tomar cada fila a lo sumo una vez por ejecución (`take_row`): una fila que el handler escribió antes de la lectura de arranque llega también por el canal. Esa copia la reconoce `StartupRows` (`worker.rs:213`), que guarda sólo los ids que encontró la lectura de arranque y no crece en toda la ejecución, porque el replay corre antes que el canal y una fila escrita después de la lectura llega una sola vez. Una fila que ya no existe, o que está `failed`, MUST saltarse sin escribir nada (`Claimed::Skip`, `outbox.rs:140`). Si la segunda entrega trae un canal de respuesta y la primera vino de la lectura de arranque, el que espera recibe el reporte de la primera.

#### Escenario
- GIVEN una fila pendiente que llega por la lectura de arranque y por el canal
- THEN se procesa una vez y la fila del texto tiene `access_count = 1`

**Tests:** `a_row_delivered_twice_is_ingested_once` y `only_the_channel_copy_of_a_start_up_row_is_a_second_delivery` (`worker.rs`, mod tests); el salto de una fila inexistente o `failed` lo cubre `claiming_spends_an_attempt_and_gives_up_once_none_is_left` (`outbox.rs`, mod tests), a nivel del store y no del worker.

### Intentos y fallo a la vista

Tomar una fila MUST sumar un intento antes de procesarla (`outbox_claim`, `outbox.rs:299`). Si la fila ya tenía `MAX_ATTEMPTS`, MUST quedar `failed` con la causa `the daemon stopped N times while ingesting this save`, sin procesarse, y el worker MUST seguir con la siguiente (`claim_step`, `outbox.rs:65`, pura): tres cortes del proceso seguidos con el mismo save dicen que el save es lo que lo mata.

Un fallo del worker MUST anotar su causa en `last_error` (`outbox_fail`, `outbox.rs:346`). Una fila MUST pasar a `failed` sólo cuando gastó `MAX_ATTEMPTS` tomas que terminaron en un fallo no pasajero o en la muerte del proceso (`after_failure`, `outbox.rs:76`, pura, y `claim_step`), o cuando su `event` ya no parsea, que MUST darse por perdida con esa causa en vez de bloquear a las demás (`outbox_pending`, `outbox.rs:486`). Un fallo no pasajero que no llega al tope deja la fila `pending` para el próximo arranque, no para la misma ejecución, porque el mismo texto falla igual. Un fallo pasajero y una pausa del apagado (abajo) MUST devolver su intento.

Si el handler contesta al que guardó un fallo no pasajero sin pedazos guardados (`500`), MUST borrar la fila; con pedazos guardados o con un fallo pasajero MUST conservarla ([`specs/ingest`](../ingest/spec.md), «Un fallo del worker llega al que guardó»). Una fila `failed` MUST NOT borrarse sola: queda a la vista en `/health` ([`specs/ingest`](../ingest/spec.md), «Estado del outbox en `/health`») hasta que el usuario la reintenta o la descarta (abajo).

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

**Tests:** `claim_step_processes_until_the_attempts_run_out`, `after_failure_fails_a_row_only_at_the_cap_and_hands_back_a_transient_attempt` y `a_pause_hands_back_its_attempt` (`outbox.rs`, mod tests); `claiming_spends_an_attempt_and_gives_up_once_none_is_left`, `a_failure_keeps_its_cause_and_a_transient_one_its_attempt` y `an_event_that_does_not_parse_is_given_up_with_its_cause` (`outbox.rs`, mod tests); `a_row_that_stopped_the_process_three_times_fails_and_the_next_one_runs` y `a_failure_after_the_answer_stays_in_the_outbox_with_its_cause` (`worker.rs`, mod tests); `a_failure_the_caller_was_told_leaves_no_outbox_row` (`tests/api.rs`).

### Un fallo pasajero se reintenta en la misma ejecución

Tras un fallo pasajero de un save con fila, el worker MUST anotar la causa y devolver el intento, MUST contestar al que espera, si lo hay, y MUST tomar la misma fila otra vez tras `retry_delay(n)` (`worker.rs:114`, pura): 1 s la primera vez, el doble cada vez siguiente, a lo sumo 60 s (`MAX_RETRY_DELAY`, `worker.rs:110`, el mismo minuto en que `IdleEmbedder` recupera a lo sumo un hilo muerto). Mientras espera MUST NOT tomar otro evento (`memory_worker`, `worker.rs:299`): dos saves con la misma clave se aplican en el orden en que se aceptaron, y lo que espera detrás fallaría igual. Si en la espera empieza un drenaje, o ya no queda quien lo pida, el worker MUST dejar el save en su fila, con el intento ya devuelto, y terminar como con un evento sin empezar (`wait_to_retry`, `worker.rs:504`). Un evento sin fila MUST informarse como antes, sin reintento. No hay barrido del outbox: las filas que esperan en el canal también están `pending` y el worker no las ve hasta recibirlas.

Una fila de la lectura de arranque que se reintenta retiene también la copia que llegó por el canal con la respuesta de quien la guardó, y ese llamante ve `202 unconfirmed` al vencer su espera. Sólo ocurre con un save aceptado entre que el daemon escucha y que el worker lee el outbox.

#### Escenario: el modelo vuelve
- GIVEN un save con fila y un embedder que responde `NotReady` una vez y después funciona
- WHEN el worker lo procesa
- THEN el save queda guardado y su fila borrada sin reiniciar el worker

#### Escenario: el orden se conserva
- GIVEN dos saves con la misma `topic_key`, el primero con un fallo pasajero
- WHEN el worker termina los dos
- THEN la versión viva de la clave es la del segundo

#### Escenario: un drenaje durante la espera
- GIVEN un save que espera su reintento
- WHEN empieza un drenaje
- THEN el worker termina y la fila queda `pending` con los intentos que tenía antes de tomarla

**Tests:** `retry_delay_doubles_up_to_a_minute`, `a_transient_failure_is_retried_in_the_same_run`, `a_save_behind_a_retried_one_waits_for_it` y `a_drain_during_a_retry_leaves_the_row_pending` (`worker.rs`, mod tests).

### Una fila `failed` se reintenta o se descarta

`MemoryRuntime::retry_failed` y `discard_failed` (`crates/clawg-core/src/runtime.rs:267` y `:282`), detrás de `POST /v1/outbox/retry` y `POST /v1/outbox/discard` (`{"operation_id", "agent_id"?}`, `crates/clawg-cli/src/daemon/api.rs:1519` y `:1525`) y de `clawg outbox retry|discard <OPERATION_ID> [--agent NAME]` (`crates/clawg-cli/src/outbox.rs`), MUST actuar sobre la fila `failed` de ese `operation_id`: la del agente nombrado, o la única que lo tenga (`outbox_find_failed`, `outbox.rs:382`). MUST responder `404` sin fila, y `409` con una fila `pending` (el worker puede estar en ella), con un `operation_id` que tienen varios agentes (la frase de `error` los nombra) o con un evento que ya no parsea (`OutboxActionError`). Un `operation_id` inválido es un `400`, como en un save. El comando va por el daemon en marcha y MUST NOT arrancar uno.

- **Reintentar** MUST escribir otra vez la fila, en una transacción, con otro `id`, `pending`, `attempts = 0`, sin `last_error` y con su progreso (`outbox_requeue`, `outbox.rs:420`), y encolarla. Otro `id` porque el worker reconoce por `id` las filas de su lectura de arranque, y tomaría una que estaba `pending` al arrancar por la segunda entrega de sí misma. Si la cola no la admite, la fila queda `pending` para el próximo arranque y la respuesta lo dice (`queued: false`).
- **Descartar** MUST borrar suavemente, con el borrado de siempre (`delete_memory_core`), los pedazos que ese save insertó según su progreso y siguen vivos, y después la fila. Como el save no terminó, su revisión no corrió y la versión anterior de su clave sigue viva: borrar sus pedazos no revive nada. `clawg undelete` trae cada uno. La respuesta los nombra, y el `operation_id` queda libre otra vez.

#### Escenario: reintentar
- GIVEN una fila `failed` con `operation_id` `op-1` y un embedder que funciona
- WHEN se reintenta `op-1`
- THEN la memoria queda guardada y el outbox vacío

#### Escenario: descartar
- GIVEN una fila `failed` cuyo save insertó un pedazo
- WHEN se descarta
- THEN el pedazo queda borrado, la respuesta lo nombra, el outbox está vacío y un save con `op-1` vuelve a guardarse

**Tests:** `retrying_a_failed_row_writes_it_again_under_a_new_id`, `only_a_failed_row_is_retried_or_discarded` y `an_operation_id_two_agents_hold_is_ambiguous` (`outbox.rs`, mod tests); `a_failed_save_is_retried_by_its_operation_id`, `discarding_a_failed_save_deletes_the_pieces_it_inserted` y `a_pending_or_shared_operation_id_is_refused` (`tests/api.rs`); `outbox_takes_an_action_an_operation_id_and_an_agent` (`cli.rs`, mod tests); `outbox_reports_say_what_was_done` (`crates/clawg-cli/src/outbox.rs`, mod tests).

### Ningún save aceptado se pierde en un apagado ordenado

En un apagado ordenado, cada save aceptado MUST terminar en uno de dos lugares: guardado en la base (o absorbido por el gate de dedup), o en el outbox para el próximo arranque. El apagado MUST seguir drenando hasta `DRAIN_DEADLINE`, MUST NOT escribir `ingest-pending.jsonl` y MUST NOT descartar un save: lo que el worker no empezó queda en su fila.

La decisión de seguir drenando o dejar la cola es pura (`drain_step`, `crates/clawg-core/src/memory/worker.rs:235`; su variante `DrainStep::Spill` conserva el nombre de cuando el apagado volcaba a un archivo). `MemoryRuntime::shutdown` (`crates/clawg-core/src/runtime.rs:317`) espera al worker, y el daemon la llama antes de borrar su estado (`crates/clawg-cli/src/daemon/serve.rs:216`). Sólo un evento encolado sin fila (los caminos en proceso de tests y evaluación) puede quedar sin ingerir, y el apagado lo registra con un warn (`runtime.rs:345`).

Un save con fila que está en curso MUST mirar el plazo antes de insertar cada pedazo, salvo el primero que inserta cada toma (`pause_due`, `worker.rs:246`, pura, leída en `ingest_pieces`), porque el drenaje puede empezar después de tomarlo. Vencido, MUST detenerse con el progreso ya anotado (`IngestReport::paused`), MUST devolver el intento sin cambiar `last_error` (`outbox_pause`, `outbox.rs:371`), MUST NOT contestar al que espera (que ve `202 unconfirmed`) y MUST terminar como con un evento sin empezar; el próximo arranque retoma el save. La primera inserción de cada toma no se pausa: así una toma no tira entero el embed que pagó, un save de un solo pedazo termina, y un save largo avanza en cada arranque. Un evento sin fila termina siempre: retomarlo duplicaría lo guardado.

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

#### Escenario: un save largo al vencer el plazo
- GIVEN un save de 3 pedazos o más con los dos primeros guardados por una toma anterior, y un drenaje que vence mientras se embebe el resto
- WHEN el worker inserta el tercer pedazo y llega al cuarto
- THEN no lo inserta, la fila conserva su progreso y los intentos de antes de la toma
- AND otra ejecución termina el save con los mismos ids y el vínculo completo

**Tests:** `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_leaves_saves_held_for_the_embedder_in_the_outbox` y `shutdown_leaves_what_the_drain_deadline_left_unstarted_in_the_outbox` (`crates/clawg-core/tests/shutdown.rs`), más los unit de `drain_step` en `worker.rs` (`a_drain_keeps_ingesting_until_its_deadline` y `a_drain_without_an_embedder_spills_at_once`), y para la pausa `a_pause_is_due_once_the_drain_deadline_passes` y `a_drain_pauses_a_long_save_between_pieces` (`worker.rs`).

### Arranque: se ingiere lo pendiente antes que lo nuevo

Al arrancar, el worker (`ingest_worker_future`, `crates/clawg-core/src/runtime.rs:477`) MUST, en este orden:

1. importar `ingest-pending.jsonl` si existe (`import_spill`, `runtime.rs:520`): cada evento válido pasa a una fila, enmascarado, todos en una transacción con `synchronous = FULL` (`outbox_import`, `outbox.rs:281`); el archivo MUST borrarse sólo después del commit. Si no se pudo borrar, se avisa: una segunda importación la absorbe el dedup;
2. procesar las filas `pending` en orden de id antes que cualquier evento del canal (`outbox_pending`, `outbox.rs:486`; `Replay`, `worker.rs:179`).

Si no se puede leer el outbox, el worker lo registra y sigue con el canal: esas filas esperan al próximo arranque. Fuera de este arranque, el outbox sólo vuelve al worker por el reintento de un fallo pasajero y por `clawg outbox retry`.

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

Después de drenar y del flush del tracker, el apagado SHOULD hacer `PRAGMA wal_checkpoint(TRUNCATE)` (`checkpoint`, `runtime.rs:454`). Un fallo del checkpoint MUST registrarse y MUST NOT impedir el resto del apagado.

#### Escenario
- GIVEN una base en WAL con escrituras recientes
- WHEN se hace un apagado ordenado
- THEN `memory.db-wal` no existe o mide 0 bytes, y una copia de `memory.db` sola contiene las escrituras

**Test:** `shutdown_checkpoints_the_wal_into_memory_db` (`crates/clawg-core/tests/shutdown.rs`).

### El daemon cierra `memory.db` antes de salir

Después de un apagado que terminó dentro de su tope (`shutdown` devuelve `true`), `clawg serve` MUST cerrar la conexión del store y esperar el cierre (`MemoryRuntime::close`, `crates/clawg-core/src/runtime.rs:349`; en `crates/clawg-cli/src/daemon/serve.rs:255`), así SQLite quita `memory.db-wal` y `memory.db-shm` antes de que termine el proceso. Sin cerrarla, el cierre corre en el hilo propio de la conexión cuando cae su último handle, y la salida del proceso puede cortarlo después de quitar el índice del WAL y antes de quitar el WAL: queda un `memory.db-wal` vacío, y `clawg doctor`, que con un WAL presente abre sin `immutable`, crea un `memory.db-shm` junto a un store parado. Después de un apagado que el tope cortó no se cierra: una llamada puede tener todavía la conexión y el cierre esperaría detrás de ella, fuera de todo tope. Un cierre que falla MUST registrarse y nada más; SQLite reproduce lo que quede al abrir.

#### Escenario
- GIVEN un runtime con un save recién guardado
- WHEN se apaga y se cierra, con el runtime todavía vivo
- THEN no hay `memory.db-wal` ni `memory.db-shm` junto al store

**Tests:** `closing_after_the_shutdown_leaves_no_wal_beside_the_store` (`crates/clawg-core/tests/shutdown.rs`); `doctor_reads_a_running_daemons_store_without_changing_it` (`crates/clawg-cli/tests/binary.rs`), que fallaba de forma intermitente en Linux por el `-wal` vacío.

### SIGTERM es un apagado ordenado

En Unix el daemon MUST tratar SIGTERM como Ctrl-C (`shutdown_signal` y `terminated`, `crates/clawg-cli/src/daemon/serve.rs`): el mismo camino que `POST /v1/shutdown`, `daemon.json` y `token` borrados al salir y código de salida 0. Si no puede instalar el manejador, MUST registrarlo y seguir sirviendo. En Windows no se atiende ningún evento de consola ni de apagado: que un daemon lanzado sin consola los reciba no está verificado.

#### Escenario
- GIVEN un daemon corriendo
- WHEN recibe SIGTERM
- THEN sale con 0, no quedan `daemon.json` ni `token`, y el log dice que hizo el checkpoint

**Test:** `sigterm_stops_the_daemon_in_order` (`crates/clawg-cli/tests/binary.rs`, `#[cfg(unix)]`).

### Cada fase del apagado tiene tope

Un apagado ordenado tiene cuatro fases, y cada una MUST tener su tope en el código que la espera:

| Fase | Tope | Dónde | Al vencer |
|---|---|---|---|
| Las peticiones en vuelo | `HTTP_GRACE`, 3 s después de la señal | `within_grace`, `serve.rs` | el servidor se abandona y se registra un warn |
| El drenaje | `DRAIN_DEADLINE`, 10 s | el worker de ingest | lo no empezado queda en el outbox |
| El save en curso, el flush de accesos, el fin del mantenimiento y el checkpoint | `AFTER_DRAIN`, 7 s después del plazo de drenaje (`crates/clawg-core/src/runtime.rs`) | `shutdown_by` | se aborta el mantenimiento, se registra un warn y no se hace el checkpoint: el save en curso queda en su fila y el WAL se repite al abrir |
| Las tareas bloqueantes que el proceso espera, una carga del modelo incluida | `RUNTIME_TEARDOWN`, 2 s | `shutdown_timeout` en `main.rs`, sólo para `clawg serve` | se abandonan y el proceso sale |

En operación normal, sin señal, ninguna petición tiene tope por esto: un backup largo no se corta. Una petición cortada se ve como conexión cerrada; un save ya tiene su fila, y un snapshot sólo toma su nombre final al terminar (`snapshot_into`, `crates/clawg-core/src/memory/backup.rs:313-321`). `AFTER_DRAIN` cubre los 5 s que el checkpoint puede esperar en el busy handler de rusqlite si otro proceso tiene una lectura en curso.

#### Escenario: una petición que no termina
- GIVEN un servidor con una petición en vuelo que no termina
- WHEN llega la señal de apagado
- THEN el servidor se abandona `HTTP_GRACE` después y el apagado sigue

#### Escenario: un worker que no termina
- GIVEN un worker de ingest que no termina
- WHEN el runtime se apaga con un plazo de drenaje
- THEN el apagado vuelve a lo sumo `AFTER_DRAIN` después de ese plazo

#### Escenario: una carga del modelo que no vuelve
- GIVEN un daemon cuya carga del modelo bloquea su hilo y no vuelve
- WHEN corre `clawg stop`
- THEN sale con 0 y el proceso del daemon terminó

**Tests:** `a_server_that_does_not_finish_is_left_after_the_grace`, `a_server_that_finishes_in_time_gives_its_result` y `without_a_shutdown_the_server_has_no_deadline` (`serve.rs`, tiempo pausado); `shutdown_gives_up_on_a_worker_that_never_ends` (`crates/clawg-core/tests/shutdown.rs`, tiempo pausado); `stop_ends_a_daemon_whose_model_load_never_returns` (`binary.rs`, con `CLAWG_TEST_EMBEDDER=blocked`, cuya carga no vuelve).

### Las fases caben en `STOP_TIMEOUT`

`HTTP_GRACE + DRAIN_DEADLINE + AFTER_DRAIN + RUNTIME_TEARDOWN` más 5 s de holgura MUST NOT superar `STOP_TIMEOUT` (`crates/clawg-cli/src/stop.rs`). Hoy son 3 + 10 + 7 + 2 = 22 s, más 5 s, contra 30 s. El doc de `STOP_TIMEOUT` MUST nombrar las fases. Ningún valor está medido: son topes, y un apagado sin nada en curso termina en menos de un segundo.

**Test:** `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`stop.rs`).

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

Las migraciones pendientes de un arranque MUST aplicarse en una sola transacción junto con sus filas de historial (`configured`, `crates/clawg-core/src/memory/db.rs:394`, con `set_grouped(true)`). Si una falla, la base MUST quedar como estaba antes del arranque.

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

Los checksums de las migraciones publicadas (hoy V1–V18) MUST coincidir con una lista fija en el código (`PINNED`, `crates/clawg-core/src/memory/migrations.rs:44`). Cambiar una migración publicada MUST romper un test. Una migración nueva agrega su fila a la lista en el mismo commit.

**Test:** `published_migration_checksums_are_pinned` (`migrations.rs`).

### Una base de la versión anterior abre con el outbox vacío

Una base en V13 MUST abrir con V14 con cada fila idéntica (`memory`, `memory_piece` y `project` incluidas) y el outbox vacío: los saves aceptados vivían sólo en memoria antes de esta tabla, y no hay nada que traspasar.

#### Escenario
- GIVEN una base migrada hasta V13, con memorias, un vínculo de piezas y un proyecto
- WHEN se aplica V14
- THEN cada fila es igual a la de antes y `ingest_outbox` tiene 0 filas

**Test:** `a_database_from_the_previous_version_opens_with_an_empty_outbox` (`db.rs`). Llama a `run_migrations` directo, así que el snapshot `pre-migration` de la apertura lo cubre la regla general de «Snapshot antes de migrar».

### Una base en V14 abre con sus disputas como pares

V15 (`crates/clawg-core/migrations/V15__memory_conflict.sql`) agrega la tabla `memory_conflict` y no toca ninguna columna de `memory`. Una base en V14 MUST abrir con V15 con cada fila de `memory` igual (estado, `disputed_against`, contenido) y con un par `escalated` por cada id de cada `disputed_against`; un `disputed_against` que no es JSON válido MUST NOT impedir la migración ([`specs/disputes`](../disputes/spec.md), «Datos de la versión anterior»). Un binario anterior a V15 rechaza la base migrada con `SchemaNewer` («Verificación del esquema antes de migrar»): la vuelta atrás es el snapshot `pre-migration`.

**Test:** `a_database_with_disputes_opens_with_escalated_pairs` (`db.rs`).

### Una base en V15 abre con sus filas sin clave

V16 (`crates/clawg-core/migrations/V16__topic_key.sql`) agrega `memory.topic_key` (TEXT, nullable) y el índice parcial `idx_memory_topic` sobre `(agent_id, COALESCE(project_id, ''), topic_key)` para las filas no borradas con clave. El índice no es único: las piezas de un save comparten la clave, y la unicidad de lo vivo la mantienen el worker y `restore` ([`specs/ingest`](../ingest/spec.md), «Un save con clave reemplaza la fila viva de su clave»). El `COALESCE` es el de `idx_memory_dedup`, porque `project_id` es nullable y SQLite trata un NULL como distinto de otro.

Una base en V15 MUST abrir con V16 con cada fila de `memory` igual y con `topic_key = NULL`, y con los pares de `memory_conflict`, los vínculos de `memory_piece` y el índice de texto como estaban. La migración MUST NOT cambiar ninguna fila. Un binario anterior a V16 rechaza la base migrada con `SchemaNewer` («Verificación del esquema antes de migrar»): la vuelta atrás es el snapshot `pre-migration`.

#### Escenario
- GIVEN una base en V15 con filas, una de ellas `superseded`, piezas vinculadas, un par y el índice de texto
- WHEN se abre con la versión nueva
- THEN cada fila queda igual y con `topic_key = NULL`, y los pares, los vínculos y el índice de texto siguen iguales

**Tests:** `a_database_from_before_topic_keys_opens_unchanged` (`db.rs`), que siembra V15 con SQL directo; el checksum de V16 en `published_migration_checksums_are_pinned` (`migrations.rs`). La primera llama a `run_migrations` directo, así que el snapshot `pre-migration` de la apertura lo cubre la regla general de «Snapshot antes de migrar».

### Una base en V16 abre con sus causas de reemplazo

V17 (`crates/clawg-core/migrations/V17__revival.sql`) agrega a `memory` la columna `supersede_cause` (`revision` | `verdict`, nullable) y la columna `revived_from` (INTEGER, nullable), con el índice parcial `idx_memory_revived` sobre las no nulas. Después completa `supersede_cause` de cada fila `superseded` que tiene `superseded_by`: `revision` cuando su reemplazante es del mismo agente, del mismo proyecto (`COALESCE`), tiene la misma `topic_key` no nula y ningún par de `memory_conflict` entre las dos está juzgado `supersedes`; `verdict` en cualquier otro caso. Antes de V16 sólo un veredicto reemplazaba, y desde V16 una revisión no escribe par, así que un par `supersedes` juzgado sólo puede venir de un veredicto. Ninguna otra columna cambia, y `revived_from` queda `NULL` en todas las filas.

Una base en V16 MUST abrir con V17 con cada fila igual, salvo `supersede_cause`, y con esa causa completada según la regla de arriba; los pares, los vínculos y el índice de texto MUST seguir como estaban.

#### Escenario
- GIVEN una base en V16 con una revisión por clave, un veredicto `supersedes` entre dos filas sin clave, una revisión deshecha por un veredicto (misma clave, par `supersedes` juzgado), piezas, pares e índice de texto
- WHEN se abre con la versión nueva
- THEN la revisión queda `revision`, los dos veredictos `verdict`, la revisión deshecha `verdict`, y las demás filas sin causa ni `revived_from`

**Tests:** `a_database_from_before_revivals_opens_with_its_supersede_causes` (`crates/clawg-core/src/memory/db.rs:1152`), que siembra V16 con SQL directo; el checksum de V17 en `published_migration_checksums_are_pinned` (`migrations.rs`). Que `clawg update` aplique V17 sobre una base en V16 con revisiones y veredictos, y que el daemon nuevo actúe según esas causas, está en [`specs/self-update`](../self-update/spec.md), S10, «Escenario: migración pendiente sobre filas sin causa de reemplazo».

#### Vuelta atrás de V17

Un binario anterior a V17 MUST NOT abrir una base en V17: la rechaza sin tocarla con `SchemaNewer`, que nombra `clawg restore` («Verificación del esquema antes de migrar»). La vuelta atrás es la copia `pre-migration` que se escribió antes de V17. Quitar V17 a mano (`DROP INDEX idx_memory_revived; ALTER TABLE memory DROP COLUMN revived_from; ALTER TABLE memory DROP COLUMN supersede_cause; DELETE FROM refinery_schema_history WHERE version = 17;`) deja al binario anterior con las filas vivas que un borrado revivió, que es lo que ve de cualquier fila viva.

#### Bases de la versión anterior con borrados de la vigente

Una base escrita por una versión anterior a V17 puede tener una versión reemplazada por una fila borrada, de la misma clave, porque el borrado de la vigente no la revivía. La migración escribe `revision` en esa fila (su reemplazante tiene la clave), y no la revive: el primer paso de mantenimiento la trae de vuelta, `STARTUP_DELAY` después de arrancar el daemon (`crates/clawg-core/src/memory/maintenance.rs:25`). Esto se acepta como está diseñado: el efecto queda sólo en el log del daemon (`maintenance: brought back N rows ...`), `clawg update` no lo informa, y restaurar la fila borrada la vuelve a reemplazar. Ver [`specs/restore`](../restore/spec.md), «Filas de una versión anterior a V17».

### Una base en V17 abre con los índices de las sentencias de sesión

V18 (`crates/clawg-core/migrations/V18__session_indexes.sql`) sólo toca índices de `memory`: crea `idx_memory_session_summary` sobre `(project_id, created_at DESC)` para los `session_summary` no borrados, que lee LAST SESSION, e `idx_memory_agent_created` sobre `(agent_id, created_at DESC)` para las filas no borradas, que lee RECENT ([`specs/retrieval`](../retrieval/spec.md), «Las sentencias de sesión leen por índice»), y reconstruye `idx_memory_dedup` como `(agent_id, COALESCE(project_id, ''), content_hash, created_at)` con el mismo predicado parcial. El índice del agente entrega las filas de `agent_id = ?` por `created_at`, y sin el sello en el de dedup el planificador lo elegía para las búsquedas de dedup (`ORDER BY created_at ASC LIMIT 1`): cada guardado habría recorrido las filas vivas del agente. Con el sello, esas búsquedas (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs:300`; `TWIN_SQL` del import; `RESTORE_TWIN` del undelete) llegan por el índice de dedup sin ordenar. Ningún predicado nombra `status`: un índice parcial sólo sirve a una sentencia cuyo `WHERE` implica el del índice, y eso ataría el índice al texto de los estados.

Una base en V17 MUST abrir con V18 con cada fila igual y con los tres índices. La migración MUST NOT cambiar ninguna fila.

#### Escenario
- GIVEN un `memory.db` migrado hasta V17 con una fila
- WHEN `MemoryStore::open` lo abre
- THEN la fila sigue y `sqlite_master` tiene `idx_memory_session_summary` e `idx_memory_agent_created` sobre `memory`

#### Escenario: el dedup sigue en su índice
- GIVEN un store migrado
- WHEN se pide el plan de `REINFORCE_LOOKUP_SQL`
- THEN nombra `idx_memory_dedup` y ninguna línea nombra `TEMP B-TREE`

**Tests:** `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them` (`crates/clawg-core/src/memory/db.rs:1320`), que abre con `MemoryStore::open`, así que pasa también por el snapshot `pre-migration`; `reinforce_lookup_uses_dedup_index` (`store_insert.rs:2165`); el checksum de V18 en `published_migration_checksums_are_pinned` (`migrations.rs`).

#### Vuelta atrás de V18

Un binario anterior a V18 MUST NOT abrir una base en V18: la rechaza sin tocarla con `SchemaNewer` («Verificación del esquema antes de migrar»). La vuelta atrás es la copia `pre-migration` que se escribió antes de V18, que `clawg update` restaura sola si el binario viejo se niega ([`specs/self-update`](../self-update/spec.md)). Quitar V18 a mano no pierde ninguna fila: `DROP INDEX idx_memory_session_summary; DROP INDEX idx_memory_agent_created; DROP INDEX idx_memory_dedup; CREATE INDEX idx_memory_dedup ON memory (agent_id, COALESCE(project_id, ''), content_hash) WHERE deleted_at IS NULL AND content_hash IS NOT NULL; DELETE FROM refinery_schema_history WHERE version = 18;`.

### Verificación del esquema antes de migrar

La decisión es pura (`schema_plan`, `backup.rs:93`). `MemoryStore::open` MUST calcularla antes de cualquier pragma y antes del runner (`check_schema`, `crates/clawg-core/src/memory/db.rs:409`, llamada en `db.rs:195`). Con `Newer` o `Diverged`, `open` MUST devolver `MemoryError::SchemaNewer` o `MemoryError::SchemaDiverged` (`crates/clawg-core/src/memory/error.rs:58` y `:64`) y MUST NOT escribir nada en `memory.db`. Los mensajes MUST decir las versiones, que nada cambió, y qué hacer: instalar la versión que la escribió o `clawg restore` de un backup anterior.

#### Escenario: base más nueva
- GIVEN una base en la última versión del binario, N (hoy V18), con una fila N + 1 agregada en `refinery_schema_history`
- WHEN se abre
- THEN el error es `SchemaNewer { db: N + 1, binary: N }`, nombra `clawg restore`, y `memory.db` es byte a byte igual

#### Escenario: checksum divergente
- GIVEN una base al día con el checksum de V3 cambiado
- THEN el error es `SchemaDiverged { version: 3 }` y la base no cambió

**Tests:** `schema_plan_covers_every_history_shape`, `a_divergence_the_binary_knows_about_wins_over_newer` y `history_order_does_not_matter` (`backup.rs`); `a_newer_database_is_refused_untouched` y `a_diverged_database_is_refused_untouched` (`db.rs`).

### Snapshot antes de migrar

Con plan `Pending`, `open` MUST hacer un snapshot `pre-migration` antes de aplicar la primera migración pendiente (`db.rs:229`). Si falla, MUST devolver `MemoryError::Backup` y MUST NOT migrar. Con `Fresh` y `UpToDate` MUST NOT hacer snapshot.

#### Escenario
- GIVEN una base migrada hasta V7, con filas
- WHEN se abre con el binario actual
- THEN `backups/` tiene un snapshot `pre-migration` en V7 con esas filas, y la base quedó en la última versión

**Tests:** `opening_an_older_database_snapshots_it_before_migrating`, `opening_a_current_database_takes_no_snapshot`, `a_new_database_takes_no_snapshot` y `a_failed_snapshot_stops_the_migration` (`db.rs`).

## Contrato de embedding y pooling

### El modelo se carga con pooling mean

`E5Embedder` (`crates/clawg-core/src/memory/embedder.rs:397`) MUST embeber con pooling mean, el del entrenamiento de multilingual-e5-small: la media de los estados ocultos sobre la máscara de atención y después L2 (`pool_and_normalize`, `embedder.rs:277`). Con el token CLS, todos los pasajes de un corpus quedaban a 0,90–0,91 de coseno de cualquier pregunta. La inferencia la arma clawg sobre ONNX Runtime; las reglas de la ventana, el relleno y la salida están en «Los mismos vectores que fastembed», abajo.

El id del modelo en el contrato de embedding MUST ser `multilingual-e5-small/mean` (`E5_MODEL_ID`, `embedder.rs:133`). El id anterior, `multilingual-e5-small` (`LEGACY_CLS_MODEL_ID`, `embedder.rs:139`), nombra los vectores CLS: el archivo del modelo es el mismo, el espacio no. El contrato no tiene una columna de pooling; el id es lo que distingue espacios, y el índice de documentación de los proyectos, que guarda el `model_id` de cada doc, se reindexa solo cuando cambia ([`specs/project-docs`](../project-docs/spec.md)).

**Tests:** `mean_pooling_matches_fastembeds_formula_bit_for_bit` y `the_model_id_names_its_pooling_apart_from_the_cls_vectors_of_before` en `embedder.rs`.

### Re-embebido de un store CLS al arrancar

Al arrancar, después de cargar el modelo y dentro del bootstrap del embedder, el daemon verifica el contrato (`verify_or_reembed`, `crates/clawg-core/src/memory/bootstrap.rs:495`, cableado por `contract_verifier`, `bootstrap.rs:259`, en `production_deps`, `bootstrap.rs:289`, que `serve_deps`, `bootstrap.rs:373`, entrega al daemon en `crates/clawg-cli/src/daemon/serve.rs:102`). Si el contrato guardado tiene `LEGACY_CLS_MODEL_ID` y el embedder cargado tiene `E5_MODEL_ID`, con la misma dimensión y los mismos prefijos (`is_pooling_upgrade`, `bootstrap.rs:481`, pura), MUST:

1. escribir un snapshot `pre-reembed` en `<data>/backups`; si falla, no re-embebe, y el intento termina como un fallo de verificación, que se reintenta con la espera de siempre ([`specs/ingest`](../ingest/spec.md), «Reintento del bootstrap»);
2. re-embeber `memory`, `episodic_trajectory` y `procedural_skill` con el embedder cargado (`reembed_all` en modo `ReInfer`, `crates/clawg-core/src/memory/maintenance.rs:507`);
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

## Descarga del modelo

El daemon consigue el modelo con `ensure_model` (`crates/clawg-core/src/memory/model_download.rs:396`): en el primer arranque lo baja y en los siguientes lo lee de la caché. Es la única entrada de producción: arma un `ModelSource` (`:371`) con el origen, el oráculo de disco, la espera entre intentos, los plazos y el manifiesto, y llama a `ensure_model_from` (`:421`), la que un test usa con su propio `ModelSource`. La caché se lee y se verifica en un solo lugar, `read_verified` (`:605`).

**Definiciones:**
- **Caché:** los cinco archivos del modelo en `<data>/models/multilingual-e5-small/`, cada uno con el SHA-256 fijado en `ALL_FILES` (`model_download.rs:91`).
- **Intento:** una petición de un archivo, desde el envío hasta el último byte.
- **Corrida:** una llamada a `ensure`: el arranque, cada reintento del bootstrap ([`specs/ingest`](../ingest/spec.md), «Reintento del bootstrap») y cada reparación («Una recarga que falla no deja nada roto»).

### La caché primero, y sólo se baja lo que falta

Si los cinco archivos de la caché pasan su SHA-256, `ensure` MUST devolverlos sin emitir eventos de progreso, sin chequear el disco y sin tocar la red. Si falta alguno o no pasa, MUST pedir 600 MB libres (`MIN_FREE_BYTES`, `model_download.rs:223`) antes del primer byte y MUST bajar sólo esos (`fetch`, `:445`). Cada archivo MUST tener 3 intentos, con esperas de 5 y 30 s entre ellos (`RETRY_BACKOFF_SECS`, `:243`; `download_one`, `:654`). Cada intento MUST empezar de un `.partial` vacío y borrarlo si falla, y ninguno MUST quedar al terminar la corrida, bien o mal. El archivo bajado MUST reemplazar al de la caché sólo después de verificar su hash, renombrando el `.partial` (`commit_file`, `:875`). Un SHA-256 distinto MUST terminar la corrida sin gastar los intentos que quedan: el origen mandó otros bytes enteros y volvería a mandarlos. Una corrida que baja algo emite `Start`, `Chunk` y `Verify`, nunca `Done`, que es del bootstrap después de cargar; una que falla termina con un solo `Error` (`DownloadProgress`, `:330`).

#### Escenario: la caché completa
- GIVEN los cinco archivos con su SHA-256
- WHEN corre `ensure`
- THEN no hay ninguna petición ni ningún evento, y el disco no se mira

#### Escenario: un archivo alterado en el origen
- GIVEN un origen que manda otros bytes para `onnx/model.onnx`
- WHEN corre `ensure`
- THEN hay una sola petición de ese archivo, la corrida termina con `ChecksumMismatch` y no queda ningún `.partial`

**Tests:** `skips_download_when_all_files_cached`, `cached_path_skips_disk_check`, `downloads_only_missing_files`, `corrupt_cached_file_triggers_redownload`, `insufficient_disk_returns_err_no_http`, `sufficient_disk_passes_preflight`, `retry_succeeds_on_second_attempt`, `three_failures_returns_network_err_and_stops`, `sha_mismatch_after_retries_returns_checksum_err`, `network_failure_returns_err_no_partial_files`, `successful_download_leaves_no_partial_files`, `real_backoff_uses_correct_secs_table`, `lifecycle_events_in_expected_order` y `error_path_emits_exactly_one_terminal_error_event` (`model_download.rs`).

### Plazos de red

La descarga MUST usar un cliente con 30 s para conectar y 60 s por lectura (`DOWNLOAD_TIMEOUTS`, `model_download.rs:390`), armado una vez por corrida (`fetch`, `:477-481`). El plazo de lectura se reinicia con cada lectura y cubre también la espera de las cabeceras. MUST NOT haber un plazo total: un enlace lento que sigue mandando bytes termina. Un plazo vencido MUST terminar el intento como fallido, borrar su `.partial` y pasar al siguiente intento como cualquier otro error de red. Un cliente que no se puede armar MUST ser un error de la corrida (`DownloadError::Client`, `:344`), que el bootstrap reintenta.

#### Escenario: el servidor se calla a mitad
- GIVEN un servidor que responde `200` con cabeceras y unos bytes y después no manda nada sin cerrar
- WHEN `ensure` baja ese archivo
- THEN cada intento termina al vencer el plazo de lectura, y `ensure` vuelve con un error que lo dice y sin `.partial`

#### Escenario: el servidor acepta y no contesta
- GIVEN un servidor que acepta la conexión y nunca manda cabeceras
- WHEN `ensure` baja ese archivo
- THEN cada intento termina al vencer el plazo de lectura y `ensure` vuelve con error

**Tests:** `a_body_that_stops_arriving_fails_the_attempt`, `a_server_that_never_answers_fails_the_attempt` y `the_download_waits_30_s_to_connect_and_60_s_per_read` (`model_download.rs`). Con los plazos reales, un espejo en loopback que acepta y no contesta deja el embedder `failed` a los 3 min 35 s (tres intentos de 60 s y las esperas de 5 y 30 s), medido con el binario en la verificación del archivo.

### Un hash por archivo, fuera de los hilos del runtime

Leer y hashear la caché MUST correr en una tarea bloqueante (`check_cache_off_runtime`, `model_download.rs:532`), no en un hilo del runtime asíncrono, en el arranque y en cada recarga. El worker de ingest MUST leer, verificar y parsear `tokenizer.json` en una tarea bloqueante (`ingest_worker_future`, `crates/clawg-core/src/runtime.rs:477`). En una corrida, cada archivo MUST hashearse una sola vez: el de la caché al leerlo (`check_cache`, `:572`), el bajado mientras llegan sus bytes, sin volver a leerlo del disco. Los bytes que recibe la carga MUST ser los mismos que se hashearon. El cierre del archivo bajado (`sync_all` y el `rename`) MUST correr en una tarea bloqueante. Como el archivo bajado queda en memoria para entregarlo sin releerlo, un archivo que anuncia o trae más de 600 MB (`MAX_FILE_BYTES`, `:836`, lo que pide libre el chequeo de disco y más que cualquier archivo del modelo) MUST fallar sin reintentar y sin reservar esa memoria (`DownloadError::TooLarge`, `:350`).

#### Escenario: falta un archivo chico
- GIVEN la caché completa salvo `config.json`
- WHEN corre `ensure`
- THEN el ONNX se lee y se hashea una vez y sólo se pide `config.json`

**Tests:** `the_cache_check_leaves_the_runtime_thread_free`, `each_file_is_hashed_once_per_run` y `a_file_larger_than_any_model_file_is_refused_without_holding_it` (`model_download.rs`); `the_worker_resolves_the_tokenizer_off_the_runtime_thread` (`runtime.rs`).

### El origen: `CLAWG_HF_BASE_URL`

Sin la variable, vacía o con sólo espacios, el origen MUST ser `https://huggingface.co/intfloat/multilingual-e5-small/resolve/main` (`HF_BASE_URL`, `model_download.rs:26`). Con ella, MUST aceptar una URL `https://` con cualquier host y una `http://` sólo con host `localhost`, `127.0.0.1` o `[::1]` (`base_url_from`, `:204`, pura). Cualquier otro valor MUST ser un error que nombra la variable y lo que se acepta, MUST NOT caer en silencio al origen por defecto y MUST NOT chequear la caché ni el disco ni hacer ninguna petición (`ensure_model`, `:400-408`), así que falla aunque la caché esté completa. El error MUST NOT repetir el valor, que puede llevar credenciales: dice el esquema y el host. Una `/` final MUST ignorarse. Los SHA-256 fijados MUST verificarse con cualquier origen. La variable se lee en cada corrida, y un valor inválido es un error de descarga que el bootstrap reintenta como cualquier otro ([`specs/ingest`](../ingest/spec.md), «Reintento del bootstrap»).

#### Escenario: un valor que no es https ni loopback
- GIVEN `CLAWG_HF_BASE_URL=http://example.com`
- WHEN arranca el daemon
- THEN `embedder` queda `failed` y `embedder_error` nombra la variable y lo que se acepta, sin ninguna petición

**Tests:** `the_base_url_defaults_to_hugging_face`, `the_base_url_takes_https_and_loopback_http` y `a_base_url_that_is_not_https_or_loopback_is_refused` (`model_download.rs`). Que la corrida falla sin pedir nada no tiene test: lo comprobó el binario en la verificación del archivo.

## Ciclo de vida del embedder

El daemon envuelve el modelo en un `IdleEmbedder` (`crates/clawg-core/src/memory/idle_embedder.rs:206`), que lo suelta tras un tiempo sin uso y lo vuelve a cargar cuando una llamada necesita un vector, y que recarga también un modelo cuyo hilo de inferencia murió. Lo envuelve siempre, también con `CLAWG_EMBEDDER_IDLE_MINUTES=0`. Lo arma `unloading_when_idle` (`crates/clawg-core/src/memory/bootstrap.rs:406`), con la lectura de la caché para recargar y la descarga para repararla, cableado en `crates/clawg-cli/src/daemon/serve.rs:116`.

**Definiciones:**
- **Tiempo de ocio:** los minutos de `CLAWG_EMBEDDER_IDLE_MINUTES` (`IDLE_MINUTES_VAR`, `idle_embedder.rs:31`), 10 si falta o está vacía.
- **Residente:** el modelo está en memoria y una llamada no necesita cargarlo.
- **Uso:** una llamada de embedding, desde que pide el modelo (incluida su carga) hasta que vuelve.
- **Hilo muerto:** una llamada al modelo devuelve `EmbedError::WorkerGone` (`crates/clawg-core/src/memory/embedder.rs:191`): el hilo `embedder-worker`, dueño de la sesión, terminó o soltó la petición sin contestarla.
- **Línea de base:** la memoria privada del mismo build con el embedder falso (`CLAWG_TEST_EMBEDDER=fake`) y sólo `tokenizer.json` en su directorio de datos.

### Soltar el modelo ocioso

El daemon MUST soltar el modelo cuando pasó el tiempo de ocio desde el último uso y no hay ningún uso en curso (`should_unload`, `idle_embedder.rs:61`, pura). Un uso MUST reiniciar la cuenta. Con `CLAWG_EMBEDDER_IDLE_MINUTES=0` MUST NOT soltarlo nunca: el `IdleEmbedder` se arma sin barrido. Un valor que no es un número entero de minutos SHOULD avisar en el log y MUST usar 10, sin impedir el arranque. El barrido corre cada cuarto del tiempo de ocio, entre 1 y 60 s (`sweep_period`, `idle_embedder.rs:67`), y termina con el embedder.

#### Escenario: se suelta tras el ocio
- GIVEN un modelo residente con un tiempo de ocio de 10 minutos
- WHEN pasan 10 minutos sin usos
- THEN deja de ser residente

#### Escenario: un uso largo lo retiene
- GIVEN una llamada de embedding que dura más que el tiempo de ocio
- WHEN se cumple el tiempo de ocio durante la llamada
- THEN el modelo sigue residente, y la cuenta empieza cuando la llamada vuelve

**Tests:** `a_model_goes_only_after_the_idle_time_with_nothing_in_flight`, `an_unused_model_is_unloaded_after_the_idle_time`, `each_use_restarts_the_idle_time`, `a_call_in_flight_keeps_the_model_past_the_idle_time`, `the_idle_time_defaults_to_ten_minutes_and_zero_keeps_the_model`, `an_idle_time_that_is_not_whole_minutes_is_refused`, `with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered` y `the_sweeper_ends_with_the_embedder` (`idle_embedder.rs`).

### Recargar a demanda, una sola vez

Una llamada con el modelo suelto MUST cargarlo con la misma carga del arranque sobre los archivos de la caché, leídos y verificados otra vez (`cached` y `load`, en `unloading_when_idle`; `load_cached_model`, `crates/clawg-core/src/memory/model_download.rs:516`), y MUST devolver el mismo vector que antes de soltarlo. Una recarga MUST NOT tocar la red. Una recarga MUST NOT volver a parsear el tokenizer («Un solo tokenizer por proceso»). Las llamadas que llegan durante una carga MUST compartirla. `model_id`, `dimensions` y los prefijos MUST NOT cargar el modelo.

#### Escenario: varias llamadas, una carga
- GIVEN el modelo suelto
- WHEN llegan 8 llamadas a la vez
- THEN el modelo se carga una vez y las 8 responden

**Tests:** `a_call_after_an_unload_loads_the_model_again_and_gets_the_same_vector`, `calls_arriving_while_the_model_is_away_share_one_load` e `identity_and_prefixes_never_load_the_model` (`idle_embedder.rs`); `a_reload_does_not_parse_the_tokenizer_again` y `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` (`bootstrap.rs`, con una descarga que hace fallar el test si se llama); `the_cached_model_never_touches_the_network` (`model_download.rs`).

### Una recarga que falla no deja nada roto

Una recarga cuya caché no pasa (falta un archivo, no se lee o tiene otro SHA-256; `DownloadError::NotCached`, `crates/clawg-core/src/memory/model_download.rs:348`) MUST responder «no listo» (`EmbedError::NotReady`) sin esperar ninguna descarga, y MUST lanzar una reparación: `ensure` en una tarea del runtime, fuera del cerrojo del modelo, salvo que ya haya una en curso, así que hay a lo sumo una a la vez (`repair_in_background`, `bootstrap.rs:450`). La reparación no carga el modelo: la llamada siguiente que encuentre la caché reparada MUST cargarla. Una reparación que falla MUST dejarlo en el log, y la llamada siguiente MAY lanzar otra. Cualquier otra recarga que falla MUST responder «no listo» igual, y la llamada siguiente MUST intentar otra vez. Una recarga que trae otro `model_id` u otra dimensión MUST rechazarse igual: los vectores nuevos no pueden caer en otro espacio que los guardados. El estado del embedder MUST NOT pasar a `failed` por una recarga ni por una reparación.

El cerrojo del modelo queda tomado lo que tarda leer y hashear la caché, nunca lo que tarda la red: la lectura recorre los cinco archivos aunque el que no pasa sea chico, así que el «no listo» llega después de hashear el ONNX. Medido el 2026-10-19 con el modelo real, en un build debug en Windows: las dos búsquedas respondieron en 8,7 y 9,2 s, lo que una recarga buena tarda en leer y verificar la caché (9,5 s).

#### Escenario: un archivo borrado del modelo suelto
- GIVEN el modelo suelto, `tokenizer_config.json` borrado de la caché y una descarga que tarda
- WHEN llegan dos búsquedas seguidas
- THEN las dos responden `503 embedder_not_ready` (`embed_failure`, `crates/clawg-cli/src/daemon/api.rs:1224`) sin esperar la descarga, y hay una sola en curso
- AND cuando la descarga termina, la búsqueda siguiente carga el modelo y responde

**Tests:** `a_failed_load_is_not_ready_and_the_next_call_tries_again` y `a_model_that_comes_back_as_another_model_is_refused` (`idle_embedder.rs`); `a_reload_with_a_damaged_cache_is_not_ready_and_repairs_it_once_in_the_background` (`bootstrap.rs`); `a_search_whose_model_cannot_load_again_answers_not_ready` (`crates/clawg-cli/tests/api.rs`).

### Un pánico en la inferencia contesta y suelta la sesión

Cuando la inferencia de una petición entra en pánico, el hilo de inferencia (`serve_requests`, `crates/clawg-core/src/memory/embedder.rs:566`, con `catch_unwind`) MUST contestar esa petición con `EmbedError::InferenceFailed`, cuyo texto MUST decir `inference panicked` y la causa del pánico, MUST dejarlo en el log y MUST terminar, soltando la sesión. MUST NOT inferir otra petición con esa sesión. Las peticiones que esperaban en la cola MUST recibir `WorkerGone`. El hilo MUST NOT inferir una petición cuyo llamante ya soltó la respuesta cuando la toma; puede inferir una que se suelta durante la inferencia, a lo sumo un lote por llamante.

No se conoce un camino que haga entrar en pánico a la inferencia (ver [embedder-recovery](../../changes/archive/2026-10-19-embedder-recovery/exploration.md)): este requisito y el siguiente cubren un fallo posible, no uno observado. La llamada que entró en pánico ve el error: un `500` en búsqueda y `recall_context`, y en el ingest un save `failed` con `embed failed: inference panicked: …`.

#### Escenario: la primera petición entra en pánico
- GIVEN un hilo cuya inferencia entra en pánico
- WHEN llega una petición
- THEN su llamante recibe `InferenceFailed` con `inference panicked`
- AND la petición siguiente recibe `WorkerGone`, y el hilo terminó

#### Escenario: un llamante que se fue
- GIVEN una petición en cola cuyo llamante se fue y otra cuyo llamante espera
- WHEN el hilo las toma
- THEN la inferencia corre una vez, para la segunda

**Tests:** `a_panicking_inference_answers_its_caller_and_ends_the_worker` y `a_request_whose_caller_is_gone_is_not_inferred` (`embedder.rs`).

### Un hilo muerto se recupera en la misma llamada

Cuando el modelo devuelve `WorkerGone`, `IdleEmbedder` (`with_model` y `discard`, `idle_embedder.rs:158` y `:145`) MUST descartarlo, salvo que la celda ya tenga otro modelo (una recuperación concurrente, comparada por `Arc::ptr_eq`), MUST marcarlo no residente y MUST repetir la llamada una vez sobre un modelo cargado con la misma recarga del modelo suelto. Las llamadas que encuentran el mismo hilo muerto a la vez MUST compartir una sola recarga. Si la repetición también encuentra el hilo muerto, la llamada MUST responder `NotReady`. `WorkerGone` MUST NOT salir de `IdleEmbedder`, así que búsqueda y `recall_context` responden el `503 embedder_not_ready` de una recarga que falla, y el ingest deja el save pendiente. Con `CLAWG_EMBEDDER_IDLE_MINUTES=0` vale igual. `embedder` en `/health` MUST seguir en `ready`; `embedder_resident` es `false` mientras el modelo está descartado.

Una recarga que reemplaza un modelo muerto MUST NOT ocurrir menos de `RECOVERY_INTERVAL` (60 s, `idle_embedder.rs:41`) después de la anterior: entretanto el modelo queda descartado y toda llamada MUST responder `NotReady`; la primera llamada pasado el minuto MUST recargar. La primera recuperación MUST NOT esperar. La recarga del modelo soltado por ocio no cuenta y no espera, y una recarga de recuperación que falla no retiene las siguientes: un modelo que muere en cada llamada se lee, se verifica y se arma a lo sumo una vez por minuto, en vez de en cada llamada.

#### Escenario: el hilo murió entre dos llamadas
- GIVEN un modelo residente cuyo hilo terminó
- WHEN llega una búsqueda
- THEN el modelo se recarga una vez y la búsqueda responde

#### Escenario: dos llamadas a la vez
- GIVEN dos llamadas en curso sobre el mismo modelo cuyo hilo muere
- WHEN las dos reciben `WorkerGone`
- THEN el modelo se recarga una vez y las dos responden con el modelo nuevo

#### Escenario: un modelo que muere en cada recarga
- GIVEN un modelo cuyo hilo muere en cada llamada, también después de recargarlo
- WHEN llegan llamadas seguidas durante un minuto
- THEN el modelo se recarga una vez en ese minuto, y las llamadas responden `NotReady`
- AND la primera llamada pasado el minuto lo recarga otra vez

#### Escenario: ocio `0`
- GIVEN un `IdleEmbedder` sin tiempo de ocio
- WHEN pasan horas sin llamadas
- THEN el modelo sigue residente
- AND si su hilo muere, la llamada siguiente lo recarga y responde

**Tests:** `a_model_whose_worker_is_gone_is_loaded_again_within_the_call`, `calls_that_find_the_worker_gone_together_share_one_reload`, `a_model_that_keeps_dying_is_reloaded_at_most_once_a_minute` y `with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered` (`idle_embedder.rs`); `without_an_idle_time_the_loaded_model_is_still_wrapped` (`bootstrap.rs`).

### Las herramientas funcionan con el modelo suelto

Buscar, guardar, restaurar y `recall_context` con el modelo suelto MUST esperar la recarga y responder como con el modelo residente. `embedder` en `/health` MUST seguir en `ready`; si el modelo está en memoria lo dice `embedder_resident` ([`specs/mcp`](../mcp/spec.md), «El modelo en memoria en `/health`»).

**Tests:** `a_search_with_the_model_unloaded_loads_it_and_answers` y `a_save_with_the_model_unloaded_is_stored` (`crates/clawg-cli/tests/api.rs`).

### Registrar un proyecto despierta el modelo

`POST /v1/projects` MUST empezar a cargar un modelo suelto y MUST responder sin esperar esa carga (`embedder.wake()`, `api.rs:1699`; `wake`, `idle_embedder.rs:287`). Con el modelo residente MUST NOT cargar nada. Un modelo despertado MUST contar como recién usado.

#### Escenario: el puente empieza una sesión
- GIVEN el modelo suelto
- WHEN el puente registra su raíz
- THEN la respuesta llega enseguida y el modelo queda residente poco después, sin que nadie haya buscado

**Tests:** `wake_loads_the_model_without_a_caller_waiting` y `a_woken_model_gets_a_full_idle_time` (`idle_embedder.rs`); `registering_a_project_wakes_an_unloaded_model_without_waiting_for_it` (`crates/clawg-cli/tests/api.rs`).

### La memoria vuelve al sistema

Con el modelo suelto, la memoria privada del daemon MUST quedar cerca de la línea de base. En 10 recargas seguidas, la mediana MUST NOT superar la línea de base en más de 64 MB, y la mediana de las recargas 6 a 10 MUST NOT superar la de las recargas 1 a 5 en más de 16 MB. Vale en Windows y en Linux.

En Linux con glibc, dos cosas lo hacen posible:
- El hilo dueño de la sesión MUST devolver al sistema las páginas libres al soltarla (`return_freed_memory`, `embedder.rs:603`, un `malloc_trim(0)` después de `drop(inference.session)`).
- `clawg serve` MUST dejar todos sus hilos en una sola arena de `malloc` (`keep_one_malloc_arena`, `embedder.rs:626`, `mallopt(M_ARENA_MAX, 1)`), fijada en `main` antes de construir el runtime de tokio (`crates/clawg-cli/src/main.rs:42-46`), porque glibc le asigna la arena a un hilo en su primera asignación. Los demás comandos no la fijan (`Command::loads_the_model`, `crates/clawg-cli/src/cli.rs:176`, pura). Cada carga arma, usa y suelta la sesión en hilos nuevos para ella; con las arenas por hilo de glibc, cada recarga podía estrenar otra arena, y cada una retenía de 6 a 24 MB después del trim.

#### Escenario: diez recargas
- GIVEN `CLAWG_EMBEDDER_IDLE_MINUTES=1` y la línea de base medida en el mismo build
- WHEN diez veces una búsqueda semántica (`POST /v1/search`) carga el modelo y el tiempo de ocio lo suelta
- THEN la mediana de las diez muestras con el modelo suelto no supera la línea de base en más de 64 MB, y la mediana de las recargas 6 a 10 no supera la de las recargas 1 a 5 en más de 16 MB

#### Escenario: un parse en toda la corrida
- GIVEN la misma corrida de diez recargas
- WHEN termina
- THEN el log tiene una sola línea `tokenizer.json parsed`

**Medición, no test:** ningún test de la suite comprueba el trim ni la arena; la prueba es el procedimiento de [embedder-idle-unload, D5, «R7 sobre lo medido»](../../changes/archive/2026-10-17-embedder-idle-unload/design.md#r7-sobre-lo-medido), con el build de la suite y un `CLAWG_TEST_HOME` temporal, muestreando `PrivateMemorySize64` en Windows y `RssAnon` en Linux. La búsqueda es `POST /v1/search`, nunca `clawg memories search`, que es una búsqueda por texto y no carga el modelo. Medido el 2026-10-17: en Windows, 21,3 MB sobre la base y +2,4 entre mitades; en Linux, con la arena única, 7,6 MB sobre la base y +0,0 entre mitades (sin ella, entre 60 y 76 MB y +18). `only_serve_loads_the_model` (`cli.rs`) fija qué comando pide la arena.

### Un solo tokenizer por proceso

El daemon MUST parsear `tokenizer.json` a lo sumo una vez por proceso, y sólo de bytes cuyo SHA-256 verificó. El chunker, el índice de docs y el embedder MUST compartir esa copia, la celda `MemoryRuntime::tokenizer` (`crates/clawg-core/src/runtime.rs:143`). La llenan `resolve_tokenizer_and_chunker` (`crates/clawg-core/src/memory/worker.rs:984`), con `load_verified_tokenizer` (`crates/clawg-core/src/memory/model_download.rs:154`), y `model_loader` (`bootstrap.rs:237`); cada parse deja la línea `tokenizer.json parsed` (`model_download.rs:164` y `:191`). Una recarga MUST NOT volver a parsearlo. El embedder MUST NOT cambiar la configuración del tokenizer compartido. El chunker del worker MUST leer la celda en cada save (`ChatChunker::from_cell`, `crates/clawg-core/src/memory/chunker/chat.rs:64`): una celda que se llena después de construirlo cambia su troceo sin reiniciar, y un mismo save se corta entero con un solo splitter.

#### Escenario: arranque con el modelo en disco, en cualquier orden
- GIVEN el modelo en disco
- WHEN el worker y la carga del modelo piden el tokenizer, en cualquier orden
- THEN se parsea una vez y los dos reciben el mismo `Arc`

#### Escenario: primer arranque
- GIVEN un directorio sin modelo
- WHEN la carga del modelo baja y verifica los archivos
- THEN la carga parsea el tokenizer y llena la celda, y el chunker del worker, construido antes, parte por tokens desde el save siguiente

#### Escenario: `tokenizer.json` alterado
- GIVEN un `tokenizer.json` con otro SHA-256
- WHEN el worker intenta cargarlo
- THEN no lo parsea

#### Escenario: recarga
- GIVEN el tokenizer ya parseado y el modelo suelto
- WHEN una llamada recarga el modelo
- THEN no hay ningún parse

**Tests:** `the_worker_and_the_model_load_share_one_parse` y `a_reload_does_not_parse_the_tokenizer_again` (`bootstrap.rs`); `a_tokenizer_file_with_another_sha256_is_not_parsed` y `the_chunker_splits_by_tokens_once_the_tokenizer_loads_after_it` (`worker.rs`).

### Los mismos vectores que fastembed

Para el mismo texto y el mismo rol, el embedder MUST devolver los mismos bits que devolvió fastembed 5.13.4 en una CPU y un sistema operativo como los que generaron la referencia. ONNX Runtime elige sus kernels según la CPU y viene compilado aparte para cada sistema, así que en otra CPU o en otro sistema la diferencia máxima por componente MUST NOT pasar de 1e-6. Eso exige una ventana de `EMBED_MAX_TOKENS` (`embedder.rs:153`) que cuenta `<s>` y `</s>` (`window_encodings`, `embedder.rs:216`), relleno hasta el texto más largo del pedido con `pad_token_id` (`pad_right`, `embedder.rs:235`), `token_type_ids` cuando el modelo los pide, pooling por media sobre la máscara y L2. La carga (`from_parts`, `embedder.rs:530`) MUST rechazar (`LoadFailed`) un tokenizer con truncación o relleno propios, un token de `special_tokens_map.json` que el tokenizer no trata como especial, un `model_max_length` menor que `EMBED_MAX_TOKENS` (`check_tokenizer`, `embedder.rs:335`) y un modelo sin salida elegible (`pick_output`, `embedder.rs:260`). La sesión usa el proveedor de CPU sin arena, optimización de grafo nivel 3 y tantos hilos intra-op como tiene la máquina (`build_session`, `embedder.rs:500`).

#### Escenario: un texto largo conserva `</s>`
- GIVEN un texto de más de `EMBED_MAX_TOKENS` tokens
- WHEN se codifica
- THEN se corta a la ventana y `</s>` queda al final, dentro de los 512 tokens

#### Escenario: un lote de largos mezclados
- GIVEN un pedido con textos de largos distintos
- WHEN se codifica
- THEN cada texto se rellena hasta el más largo del pedido con `pad_token_id`, y ids, máscara y tipos son los de fastembed

#### Escenario: tokens especiales literales en el texto
- GIVEN un texto con `<s> </s> <pad> <unk> <mask>` escritos
- WHEN se codifica y se hace el pooling
- THEN el vector es el que dio fastembed

#### Escenario: otro sistema con la misma CPU
- GIVEN la fixture generada en Windows y una máquina Linux con la misma arquitectura, AVX2, AVX-512F y FMA
- WHEN se comparan los vectores
- THEN se comparan con tolerancia 1e-6, no por bits

**Tests:** `an_encoding_matches_the_tokenizer_configured_like_fastembed`, `a_long_text_is_cut_to_the_window_and_keeps_its_closing_token`, `mean_pooling_matches_fastembeds_formula_bit_for_bit`, `the_load_refuses_a_tokenizer_with_its_own_truncation_or_padding`, `the_load_refuses_a_special_token_the_tokenizer_does_not_treat_as_special`, `the_load_refuses_a_model_window_shorter_than_embed_max_tokens`, `the_output_is_picked_like_fastembed` y `the_cpu_provider_turns_its_arena_off` (`embedder.rs`); `the_vectors_are_the_ones_fastembed_gave` y `the_comparison_is_by_bits_only_on_the_cpu_that_made_the_fixture` (`crates/clawg-core/tests/e5_golden.rs`; el primero, ignorado por defecto, necesita los archivos del modelo por `CLAWG_E5_MODEL_DIR` y se corre con `--ignored`). Verificado el 2026-10-17: bits exactos en la CPU de Windows que generó la fixture, y en Linux (Xeon con AVX2, AVX-512F y FMA) una diferencia máxima de 8,9e-8 en los 32 vectores.

## Snapshots

### Un snapshot es completo y no se ve a medias

Un snapshot MUST contener todo lo commiteado, incluido lo que está sólo en el WAL, con las tablas vec0 y FTS consultables. MUST escribirse a un `.tmp` en el directorio de backups y renombrarse al terminar, y en Unix el directorio MUST sincronizarse después del `rename`, o un corte de luz podría dejar sin snapshot una migración ya aplicada (`sync_dir`, `backup.rs:433`); uno fallido MUST NOT dejar archivos (`snapshot_into`, `backup.rs:305`).

El nombre MUST ser `memory-<AAAA-MM-DD>T<hh-mm-ss.mmm>Z-<motivo>.db` en UTC, de una función pura (`snapshot_name`, `backup.rs:190`), de modo que el orden alfabético sea el cronológico. Por ejemplo, 1791827476123 ms y `manual` dan `memory-2026-10-12T17-51-16.123Z-manual.db`.

Un snapshot MUST incluir el outbox, y con él el texto enmascarado de los saves que esperaban o habían fallado. Un `restore` MUST reemplazar las filas pendientes por las del snapshot: las de la base reemplazada quedan sólo en el `pre-restore` que `restore` ya escribe, y no se traspasan, porque mezclar filas de dos bases rompería el orden y los ids del progreso. `clawg purge` nombra los snapshots con una fila del outbox que contiene el texto purgado ([`specs/secrets`](../secrets/spec.md), R6).

**Tests:** `a_snapshot_includes_writes_still_in_the_wal`, `a_snapshot_keeps_vectors_and_full_text_searchable`, `snapshot_names_sort_in_time_order`, `snapshot_names_parse_back_to_their_reason` y `a_failed_snapshot_leaves_no_file_behind` (`backup.rs`); `a_directory_syncs_and_a_missing_one_says_so` (`backup.rs`, sólo en Unix); `a_snapshot_does_not_wait_behind_the_stores_connection` y `a_snapshot_holds_the_rows_still_in_the_wal` (`runtime.rs`), que cubren la conexión propia. Que el snapshot lleve el outbox y que un `restore` lo reemplace no tienen test propio: es lo que hace copiar la base entera.

### Retención

Después de cada snapshot automático, el directorio de backups MUST conservar los 5 automáticos más nuevos (`AUTO_KEEP`, `backup.rs:241`) y, además, el más nuevo de cada motivo automático (`pre-migration`, `pre-restore`, `pre-reembed`), de modo que una racha de restores o re-embeds no saque el `pre-migration` que es la vuelta atrás de una actualización; los demás automáticos se borran. Los `manual` MUST NOT borrarse nunca, y un archivo con otro nombre MUST NOT tocarse. La elección es pura (`to_prune`, `backup.rs:248`); el borrado es best-effort.

#### Escenario
- GIVEN 7 snapshots automáticos, 2 manuales y un `notas.txt`
- WHEN se hace un snapshot automático más
- THEN quedan los 5 automáticos más nuevos, los 2 manuales y `notas.txt`

#### Escenario: una racha de otro motivo no saca al único `pre-migration`
- GIVEN un `pre-migration`, un `pre-reembed` y 7 `pre-restore` más nuevos
- WHEN se calcula qué podar
- THEN sólo se podan los 2 `pre-restore` más viejos; el `pre-migration` y el `pre-reembed` quedan

**Tests:** `pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one`, `pruning_keeps_the_newest_snapshot_of_every_automatic_reason` y `an_automatic_snapshot_prunes_older_automatic_ones` (`backup.rs`).

## Backup y restore

### `clawg backup`

`clawg backup [--profile P]` (`crates/clawg-cli/src/main.rs:140`) MUST producir un snapshot `manual` e imprimir su ruta.

- Con el daemon andando, MUST pedirlo por `POST /v1/backup` (con el token; `crates/clawg-cli/src/daemon/api.rs:1593`, ruta en `:873`), que lo hace en una conexión propia y corta, dentro de `spawn_blocking`, para que el `VACUUM INTO` no deje esperando detrás a cada llamada de la conexión del daemon (`MemoryStore::snapshot`, `crates/clawg-core/src/memory/db.rs:271`), y responde `{"path": "..."}`. El store en memoria de las pruebas, sin archivo, copia por su propia conexión.
- Sin daemon, MUST tomar el lock del store con el titular `Maintenance` (`crates/clawg-core/src/memory/store_lock.rs:59`), copiar sin migrar y soltar el lock (`backup_offline`, `backup.rs:339`).
- Sin `memory.db`, MUST decir que no hay base, sin crear el directorio de datos.

**Tests:** `backup_endpoint_writes_a_manual_snapshot` (`crates/clawg-cli/tests/api.rs`), `backup_takes_only_a_profile` (`cli.rs`), `an_offline_backup_copies_without_migrating`, `an_offline_backup_refuses_a_held_store` y `an_offline_backup_of_nothing_says_so` (`backup.rs`).

### `clawg restore`

`clawg restore <ARCHIVO> [--profile P]` (`main.rs:157`, `restore`, `backup.rs:362`) MUST, en este orden:

1. crear el directorio de datos si no existe, y tomar el lock; si lo tiene el daemon, MUST fallar diciendo que se pare con `clawg stop` (`crates/clawg-cli/src/restore.rs:23`);
2. rechazar un archivo con un `-wal` no vacío al lado: abierto, el archivo se lee con lo de su WAL, pero sólo se copia el archivo, así que esos commits pasarían la validación y se perderían. Después, validar el archivo en sólo lectura: `PRAGMA quick_check` = `ok`, con historial de refinery, y un plan que no sea `Newer`, `Diverged` ni `Fresh` (`check_restorable`, `backup.rs:444`). Si no pasa, MUST fallar sin tocar nada;
3. copiar el archivo a `memory.db.restore-tmp` con `fsync` por un handle abierto para escritura, porque en Windows uno de sólo lectura falla con "Access is denied" (`stage`, `backup.rs:383`), antes del paso 4, cuya poda puede borrar el archivo que se restaura;
4. si existe `memory.db`, hacer un snapshot `pre-restore` y un `wal_checkpoint(TRUNCATE)` (`keep_and_replace`, `backup.rs:390`). Si falla, MUST fallar sin reemplazar nada y sin dejar el temporal;
5. borrar `memory.db-wal` y `memory.db-shm` **antes** del `rename` del temporal sobre `memory.db` (`swap_in`, `backup.rs:411`): al revés, un corte dejaría el WAL viejo junto a la base nueva;
6. imprimir el archivo restaurado y la ruta del `pre-restore`.

Una base restaurada con plan `Pending` MUST abrir en el próximo arranque, que la migra con su propio snapshot.

#### Escenario: restore
- GIVEN una base con las filas A y B, y un snapshot que sólo tiene A
- WHEN se restaura el snapshot
- THEN la base tiene sólo A, `backups/` tiene un `pre-restore` con A y B, y no quedan temporales

#### Escenario: archivo inválido
- GIVEN un archivo de texto, una base SQLite sin historial, una base más nueva que el binario y una ruta que no existe
- THEN cada uno falla nombrando el archivo, y `memory.db` es byte a byte igual

#### Escenario: copia con commits en su WAL
- GIVEN la copia de una base que algo tenía abierta, con su `-wal` al lado
- WHEN se restaura
- THEN falla nombrando el `-wal`, y `memory.db` y `backups/` quedan como estaban

**Tests:** `restore_replaces_the_store_and_keeps_the_previous_one`, `restore_refuses_files_it_cannot_open_safely`, `restore_refuses_a_copy_with_commits_in_its_wal`, `restore_refuses_while_the_daemon_holds_the_store`, `a_restored_older_snapshot_migrates_on_the_next_open`, `restoring_the_oldest_kept_automatic_snapshot_survives_the_pruning` y `restore_onto_a_machine_without_a_data_directory` (`backup.rs`); `a_store_held_by_the_daemon_says_to_stop_it` y `restore_takes_a_file_and_a_profile` (`clawg-cli`).

## Mensajes y actualización

### `clawg status` muestra el esquema sin daemon

Sin daemon, el JSON de `clawg status` MUST tener un bloque `store` (`store_block`, `crates/clawg-cli/src/status.rs:48`) con `state` (`missing`, `fresh`, `up_to_date`, `pending`, `newer`, `diverged` o `unreadable`), `db_version`, `binary_version` y un `message` cuando el estado impide arrancar. Con daemon, `store` MUST ser `null` (`main.rs:109`).

La lectura (`probe`, `backup.rs:506`) MUST hacerse sin lock, sin migrar y sin escribir, lo que incluye no dejar `memory.db-wal` ni `memory.db-shm`: sin `-wal`, la base se abre con la URI `immutable=1` (`open_for_probe` e `immutable_uri`, `backup.rs:528` y `:499`).

**Tests:** `a_stopped_daemon_reports_the_store_schema` y `a_store_that_would_open_carries_no_message` (`status.rs`); `probing_a_newer_store_reads_without_writing`, `probing_without_a_store_creates_none`, `probing_a_file_that_is_not_a_store_says_unreadable` e `immutable_uris_escape_what_a_uri_cannot_hold` (`backup.rs`).

### MCP nombra `clawg status` cuando el daemon no arrancó

Con `StartTimeout`, el texto de la herramienta MUST decir que clawg intentó arrancar su daemon y que `clawg status` muestra la causa (`crates/clawg-cli/src/mcp/server.rs:553-557`). Con `NotRunning` (autoarranque apagado) MUST seguir pidiendo `clawg serve`.

**Tests:** `a_daemon_that_did_not_start_points_at_status` (`server.rs`) y `a_tool_call_when_the_daemon_never_comes_up_gives_the_readable_error` (`mcp/mod.rs`).

### El instalador aborta si el daemon sigue vivo

Después de `clawg stop --profile prod`, `scripts/install.ps1` MUST ejecutar `clawg status --profile prod` con el binario anterior (`install.ps1:266`). Si sale con 0, MUST abortar antes de mover el binario, diciendo que el daemon sigue corriendo y que el binario no se reemplazó.

**Verificación:** manual en Windows; no hay PowerShell en Linux ni en CI.

## El log del daemon

### `serve.log` se rota al abrirse

`spawn_serve_at` (`crates/clawg-cli/src/client.rs:1271`), que usan el autoarranque del puente y `clawg update`, MUST abrir `<data>/logs/serve.log` en append con `serve_log::open` (`crates/clawg-cli/src/serve_log.rs:21`), que antes lo rota si su tamaño pasa de `SERVE_LOG_LIMIT` (10 MiB, `serve_log.rs:14`): `serve.log.1`, si existe, MUST pasar a `serve.log.2`, reemplazando el que hubiera, y `serve.log` MUST pasar a `serve.log.1`. Con el tamaño igual o menor, o sin `serve.log`, MUST NOT renombrar nada. La decisión de qué renombrar MUST ser una función pura del tamaño, el tope y la cantidad de ficheros (`rotation`, `serve_log.rs:57`). El daemon que corre MUST NOT rotar su log: sólo hereda el fichero, así que sus `panic` y lo que ONNX Runtime escribe en stderr van ahí, y un daemon que vive semanas puede pasar el tope hasta el próximo arranque.

Un fichero que otro proceso tiene abierto MUST poder rotarse, y lo que ese proceso escribe después MUST quedar en el fichero renombrado, en Unix y en Windows (`rotate`, `serve_log.rs:39`: se renombra, no se copia).

Si la rotación falla, MUST abrir `serve.log` igual, en append, y MUST escribir en él una línea que nombre la causa antes de lanzar el daemon (`open_with`, `serve_log.rs:27`).

Un `clawg serve` lanzado a mano escribe donde lo redirija quien lo lanza, sin rotación.

#### Escenario: el autoarranque rota
- GIVEN un home de prueba con un `serve.log` de más de `SERVE_LOG_LIMIT`
- WHEN un puente con autoarranque recibe una llamada
- THEN `serve.log.1` tiene el contenido viejo y `serve.log` empieza con la línea de arranque del daemon

#### Escenario: bajo el tope
- GIVEN un `serve.log` de `SERVE_LOG_LIMIT` bytes
- WHEN se decide la rotación
- THEN no se renombra nada

#### Escenario: tres ficheros
- GIVEN `serve.log` sobre el tope, `serve.log.1` y `serve.log.2`
- WHEN se rota
- THEN el viejo `serve.log.2` desaparece, `serve.log.1` pasó a `.2` y `serve.log` a `.1`

#### Escenario: otro escritor
- GIVEN `serve.log` sobre el tope y abierto en append por otro manejador
- WHEN se rota y ese manejador escribe
- THEN lo escrito queda en `serve.log.1`

**Tests:** `nothing_rotates_up_to_the_limit`, `a_log_under_the_limit_is_appended_to`, `over_the_limit_each_file_moves_one_place`, `rotating_keeps_three_files`, `a_file_open_for_writing_still_rotates`, `a_failed_rotation_is_written_to_the_log` (`serve_log.rs`); `an_autostart_rotates_a_full_serve_log` (`tests/binary.rs`).

### La primera línea del daemon nombra su versión

`clawg serve` MUST registrar como su primera línea, con nivel `info` y antes de resolver el directorio de datos o abrir el store, `clawg serve <versión> starting, profile <perfil>`, con ` (<dist_target>)` después de la versión cuando el binario lo lleva (`starting_line`, `crates/clawg-cli/src/daemon/serve.rs:372`, registrada en `serve.rs:108`). La versión y el `dist_target` MUST ser los de `build_info`, así que en un build de test los del sello. Vale también cuando el daemon escribe en la terminal. Así un `serve.log` que `update` comparte entre binarios dice qué versión escribió cada tramo, también el de un arranque que falló antes de escuchar.

#### Escenario: un build de release
- GIVEN la versión `0.50.0` y el `dist_target` `x86_64-unknown-linux-gnu`
- WHEN se arma la línea para `dev`
- THEN es `clawg serve 0.50.0 (x86_64-unknown-linux-gnu) starting, profile dev`

#### Escenario: un daemon lanzado a mano
- GIVEN un home de prueba
- WHEN arranca `clawg serve` con su stderr en un fichero
- THEN la primera línea del fichero nombra la versión del binario

**Tests:** `the_starting_line_names_the_version_and_the_build` (`daemon/serve.rs`); `a_daemon_names_its_version_first` y `an_autostart_rotates_a_full_serve_log` (`tests/binary.rs`).

## Archivos del workspace

### Escritura atómica de `workspace.md`

`regenerate_auto_section` (`crates/clawg-core/src/memory/workspace.rs:108`) MUST escribir a un archivo temporal en el mismo directorio y hacer `rename` sobre `workspace.md` (`write_atomically`, `workspace.rs:482`). Si la escritura falla, `workspace.md` MUST quedar con su contenido anterior, sección USER incluida, y MUST NOT quedar ningún temporal.

#### Escenario
- GIVEN un `workspace.md` con una sección USER escrita a mano
- WHEN la escritura del temporal falla
- THEN `workspace.md` es idéntico al anterior y no queda ningún temporal

**Test:** `a_failed_workspace_write_leaves_the_previous_file_intact` (`workspace.rs`).
