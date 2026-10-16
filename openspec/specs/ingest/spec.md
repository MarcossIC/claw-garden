# ingest — Especificación

Fuente de verdad del comportamiento actual de la ingesta: el contrato de `POST /v1/memories` y de la herramienta `save_memory` (incluido el outbox donde queda un save aceptado y el `operation_id`), el estado del embedder y del outbox que publica `/health`, el reintento del bootstrap del modelo, la capacidad de la cola y qué chunker trocea un save. El índice de la documentación y el código de un proyecto, que se trocea aparte, está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [ingest-durability](../../changes/archive/2026-10-12-ingest-durability/proposal.md), archivado el 2026-10-12.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Troceo del save").
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (el outbox, el `operation_id` y el bloque `ingest` de `/health`; lo que el worker hace con una fila y el arranque y el apagado están en [`specs/storage`](../storage/spec.md), «Apagado sin pérdida»).

## Definiciones

- **Worker:** el único consumidor de la cola de ingesta (`memory_worker`, `crates/clawg-core/src/memory/worker.rs:232`). Ninguna ruta de ingesta agrega otro escritor: el dedup sólo es correcto con un consumidor secuencial.
- **Resultado de un chunk:** `inserted` (fila nueva) o `reinforced` (el dedup absorbió el chunk en una fila existente). En los dos casos hay un id de la tabla `memory` (`SavedChunk` y `ChunkOutcome`, `worker.rs:38` y `worker.rs:29`).
- **Reporte de un evento:** `IngestReport` (`worker.rs:79`): los chunks que quedaron en la base, en orden, y la causa que cortó el evento si la hubo. Lleva las dos partes a la vez porque un fallo a mitad de un evento deja chunks ya escritos.
- **Estado del embedder:** `EmbedderStatus` (`crates/clawg-core/src/memory/bootstrap.rs:37`): `Loading`, `Ready` o `Failed { error, retryable }`. El `OnceCell` del embedder se llena antes de publicar `Ready`.
- **Outbox:** la tabla `ingest_outbox` de `memory.db` (migración V14, `crates/clawg-core/migrations/V14__ingest_outbox.sql`): un save aceptado, ya enmascarado, hasta que termina de guardarse. Su contenido (`content` es el texto enmascarado; `event`, el resto del evento en JSON; `progress`, las piezas ya guardadas) y lo que el worker hace con cada fila están en [`specs/storage`](../storage/spec.md), «Apagado sin pérdida».
- **Save aceptado:** uno que respondió `201` o `202`: tiene su fila confirmada en el outbox, o ya terminó de guardarse. Un save que el handler rechazó (`503 ingest_queue_full` o `500`) no es aceptado y no deja fila.
- **Fila `pending` / `failed`:** `state = 'pending'` (espera al worker) o `'failed'` (se la dio por perdida, con su causa en `last_error`).
- **`operation_id`:** el id con el que queda una fila; lo manda el que guarda o lo inventa el daemon (un UUID v4). Es único por agente (`UNIQUE (agent_id, operation_id)`).
- **Save confirmado:** el handler recibió el reporte del worker antes de `SAVE_CONFIRM_TIMEOUT`.
- **Timeout de confirmación (`SAVE_CONFIRM_TIMEOUT`, `crates/clawg-cli/src/daemon/api.rs:91`):** 10 s. Vive en `AppCtx::save_timeout` para que un test lo reduzca.
- **Capacidad de la cola (`INGEST_QUEUE_CAPACITY`, `crates/clawg-core/src/runtime.rs:47`):** 1024 eventos.

## Contrato de `POST /v1/memories`

### Aceptar es durable

Después de validar el pedido (contenido, `operation_id` y `derived_from`, cada uno un `400`) y de comprobar que el embedder no está `failed`, `POST /v1/memories` MUST pasar por `MemoryRuntime::accept` (`crates/clawg-core/src/runtime.rs:212`, llamado en `save_memory`, `api.rs:819`), que en este orden:

1. enmascara el contenido (ver [`specs/secrets`](../secrets/spec.md), R2): la fila guarda el texto ya enmascarado;
2. escribe y confirma la fila del outbox (`outbox_record`, `crates/clawg-core/src/memory/outbox.rs:207`). El commit MUST hacerse con `synchronous = FULL` y la conexión MUST volver después a su valor (`durably`, `outbox.rs:182`): un save aceptado sobrevive también a un corte de luz, a costa de un fsync por save. El resto de la base, incluidos el progreso y el borrado final de la fila, sigue en `NORMAL`;
3. encola el evento con `try_send`.

La fila MUST estar confirmada antes de encolar el evento y antes de responder `201` o `202`. Si la cola rechaza el evento (llena o cerrada), el save MUST responder `503 ingest_queue_full` o `500` y MUST borrar la fila (`outbox_forget`, `outbox.rs:320`): un save rechazado no queda guardado. Un evento sin fila (`IngestRequest::outbox == None`) sólo existe en los caminos en proceso de los tests y de `EvalIngest` (`crates/clawg-core/src/eval_api.rs:468`), y se procesa como antes del outbox.

#### Escenario: con el modelo cargando
- GIVEN un daemon con el embedder `loading`
- WHEN se guarda un texto y responde `202 embedder_loading`
- THEN el outbox tiene una fila `pending` con ese texto
- AND si el proceso termina sin apagado ordenado y se abre otro runtime sobre el mismo directorio con el embedder listo, la memoria queda en la base y el outbox vacío

#### Escenario: sin confirmación a tiempo
- GIVEN un worker detenido y un plazo de confirmación de 100 ms
- WHEN se guarda un texto y responde `202 unconfirmed`
- THEN el outbox tiene una fila `pending` con ese texto

#### Escenario: cola llena
- GIVEN una cola de capacidad 2, sin worker, y el embedder `loading`
- WHEN se guardan 3 textos
- THEN el tercero responde `503 ingest_queue_full` y el outbox tiene 2 filas

**Tests:** `a_save_accepted_while_the_model_loads_survives_a_process_that_dies` (`crates/clawg-core/tests/shutdown.rs`); `a_save_accepted_while_the_model_loads_is_in_the_outbox_before_the_answer`, `an_unconfirmed_save_is_in_the_outbox` y `a_save_into_a_full_queue_leaves_no_outbox_row` (`crates/clawg-cli/tests/api.rs`); `accept_writes_the_row_before_it_queues_the_event` y `a_refused_event_leaves_no_row` (`runtime.rs`, mod tests); `durably_writes_under_full_and_puts_the_setting_back` y `recording_a_row_leaves_the_store_on_normal` (`outbox.rs`, mod tests). Que `FULL` sobreviva a un corte de luz no tiene test: se prueba que el pragma se pone y se devuelve.

### Un save confirmado devuelve los ids

Con el embedder `ready`, `POST /v1/memories` MUST encolar el evento con un canal de respuesta, esperar el reporte del worker y responder `201` con un elemento por chunk, en el orden de los chunks (`save_memory`, `api.rs:763`; `save_response`, `api.rs:701`). Todo `201` lleva además el `operation_id` (ver «`operation_id`»):

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}], "operation_id": "3f0c…"}
```

`kind` es siempre `"memory"`, de modo que `kind` e `id` son exactamente lo que toma `DELETE /v1/memories/{kind}/{id}`. Las copias repetidas dentro del mismo save MUST NOT generar elementos extra: cuentan como refuerzo de la fila de su primera copia (`collapse_duplicate_chunks`, `worker.rs:142`, y `ingest_pieces`, `worker.rs:578`).

Si la ingesta enmascaró secretos, la respuesta lleva además `redacted` con sus tipos, y `save_memory` los nombra en su texto: ver [`specs/secrets`](../secrets/spec.md), R3.

#### Escenario: inserción
- GIVEN un daemon con el embedder listo y la base vacía
- WHEN se guarda `el tren sale los martes`
- THEN la respuesta es `201`, con un elemento `inserted` y su id
- AND `GET /v1/memories/memory/{id}` devuelve esa fila sin esperar ni hacer polling

#### Escenario: varios chunks
- GIVEN un contenido que el chunker parte en 3 chunks distintos
- THEN la respuesta tiene 3 elementos, todos `inserted`, con ids distintos

**Tests:** `a_save_answers_201_with_the_id_of_the_row_it_wrote`, `a_multi_chunk_save_returns_one_id_per_chunk` y `a_saved_memory_is_a_listable_row_when_the_save_returns` (`crates/clawg-cli/tests/api.rs`).

### Un duplicado devuelve la fila existente

Un save cuyo contenido ya está guardado MUST responder `201` con `outcome: "reinforced"` y el id de la fila existente. MUST NOT insertar otra fila.

#### Escenario
- GIVEN un save de `X` que devolvió el id 57
- WHEN se guarda `X` otra vez, con el mismo agente y proyecto
- THEN la respuesta tiene un elemento `{"id": 57, "outcome": "reinforced"}` y la tabla sigue con una fila

**Tests:** `saving_the_same_text_twice_returns_the_same_id_as_reinforced` (`tests/api.rs`) y `process_event_reports_reinforced_with_the_existing_id` (`worker.rs`, mod tests).

### Un fallo del worker llega al que guardó

Si el worker no puede guardar el evento (embedder caído, batch fallido, menos vectores que chunks, insert fallido), el save MUST responder con error y la causa (`save_response`, `api.rs:701`):

- `503` con `error: "embedder_unavailable"` y `detail` si el embedder no estaba disponible (`EmbedError::NotReady` o `WorkerGone`, o la celda vacía, en `ingest_pieces`);
- `500` con `error: "ingest_failed"` y un `detail` legible en los demás casos (`embed failed: …`, `embedder returned N vectors for M chunks`, `store insert failed: …`).

El worker MUST cortar el evento en el primer fallo: lo que queda guardado es siempre un prefijo del texto, nunca uno con un hueco en el medio. El cuerpo de error MUST incluir en `memories` los chunks que ya estaban en la base antes del fallo, sean insertados o reforzados, en orden. Los chunks que no se escribieron MUST NOT aparecer. El worker además registra el fallo con `log::warn!`.

Una respuesta que lleva un fallo llega a quien guardó, así que nadie necesita la fila para enterarse: el handler MUST borrar la fila del outbox cuando contesta `500` o `503` con el reporte del worker (`outbox_forget`, `api.rs:846`). Lo hace el handler y no el worker porque sólo el handler sabe si el reporte llegó antes de vencer la espera. Un save cuyo fallo ocurre después de haber contestado (`202`) no tiene a quién contestárselo: su fila queda con la causa y se ve en `/health` (ver «Estado del outbox en `/health`» y [`specs/storage`](../storage/spec.md), «Intentos y fallo a la vista»).

#### Escenario: embed fallido
- GIVEN un embedder de prueba cuyo `embed_batch` devuelve error
- WHEN se guarda un texto
- THEN la respuesta es `500 ingest_failed`, `memories` está vacío y la tabla no tiene filas

#### Escenario: insert parcial
- GIVEN un save de 2 chunks donde el insert del segundo falla
- THEN la respuesta es `500 ingest_failed` y `memories` tiene sólo el id del primero

#### Escenario: corte en el primer fallo
- GIVEN un save de 3 chunks donde el insert del segundo falla
- THEN el tercero no se intenta y la tabla tiene sólo la fila del primero

**Tests:** `a_failed_embed_reaches_the_caller_as_an_error` y `a_failure_the_caller_was_told_leaves_no_outbox_row` (`tests/api.rs`); `process_event_reports_the_rows_written_before_an_insert_failed`, `process_event_stops_at_the_first_failed_insert`, `a_failed_embed_batch_reports_the_failure_and_no_rows` y `an_unavailable_embedder_reports_embedder_unavailable` (`worker.rs`, mod tests); y los unit de `save_response` (`api.rs`, mod tests).

### Timeout de confirmación

Si el worker no contesta dentro de `AppCtx::save_timeout`, el save MUST responder `202` con:

```json
{"status": "queued", "reason": "unconfirmed", "operation_id": "3f0c…"}
```

El evento MUST seguir en la cola y su fila en el outbox: el handler deja de esperar, pero no retira ninguno de los dos. Cuando el worker lo procese, el reporte se descarta sin error.

Si el worker suelta el canal de respuesta sin contestar, el save MUST responder lo mismo que con el timeout, sin esperar a que venza. Es lo que ocurre cuando un apagado ordenado deja el evento sin empezar (ver [storage](../storage/spec.md), «Ningún save aceptado se pierde en un apagado ordenado»): su fila sigue en el outbox y se ingiere al próximo arranque.

#### Escenario: sin confirmación a tiempo
- GIVEN un worker detenido a propósito (el test no lo lanza) y un timeout de prueba de 100 ms
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "unconfirmed"`, en menos de 1 s

#### Escenario: respuesta soltada
- GIVEN un worker que suelta el canal de respuesta sin contestar
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "unconfirmed"`, en menos de la mitad de `SAVE_CONFIRM_TIMEOUT`

**Tests:** `a_save_the_worker_does_not_confirm_in_time_answers_202_unconfirmed` y `a_save_whose_reply_is_dropped_answers_202_unconfirmed` (`tests/api.rs`).

### Embedder cargando

Con el embedder `loading`, el save MUST encolar el evento sin canal de respuesta y responder `202` al instante, sin esperar al worker:

```json
{"status": "queued", "reason": "embedder_loading", "operation_id": "3f0c…"}
```

El worker MUST retener esos eventos hasta que el modelo cargue (`hold_for_embedder`, `worker.rs:389`). Un apagado ordenado que llega antes los deja en el outbox (ver [storage](../storage/spec.md)). Como la fila ya está confirmada, un `202 embedder_loading` sobrevive a un proceso que muere antes de que el modelo cargue.

#### Escenario
- GIVEN un daemon con el embedder `loading`
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "embedder_loading"` y un `operation_id`
- AND cuando el embedder pasa a `ready`, la fila aparece

**Test:** `a_save_while_the_model_loads_answers_202_and_lands_once_it_loads` (`tests/api.rs`).

### Embedder fallido

Con el embedder `failed`, el save MUST responder `503` sin encolar y sin escribir fila:

```json
{"error": "embedder_failed", "detail": "<causa>", "retryable": true}
```

`POST /v1/search` MUST responder el mismo cuerpo cuando el embedder está `failed`, y `embedder_not_ready` en los demás casos en que la celda está vacía (`search`, `api.rs:863`). `POST /v1/context` no responde ninguno de los dos: sin embedder cae a la recuperación léxica.

#### Escenario
- GIVEN un bootstrap cuya descarga falla
- WHEN se guarda un texto
- THEN la respuesta es `503 embedder_failed` con la causa, y la cola sigue vacía

**Tests:** `a_save_after_the_bootstrap_failed_is_503_with_the_cause` y `search_after_the_bootstrap_failed_says_failed_not_loading` (`tests/api.rs`).

### Cola acotada

La cola de ingesta MUST tener capacidad `INGEST_QUEUE_CAPACITY` (1024 eventos). Si está llena, el save MUST responder `503` con `error: "ingest_queue_full"` y MUST NOT bloquear el request: el handler usa `try_send`. Si el worker ya no existe (canal cerrado), el save responde `500`. En los dos casos el save no se acepta y su fila se borra (ver «Aceptar es durable»).

#### Escenario
- GIVEN una cola de capacidad de prueba 2, sin worker, y el embedder `loading`
- WHEN se guardan 3 textos
- THEN los dos primeros responden `202` y el tercero `503 ingest_queue_full`, y el outbox tiene las filas de los dos primeros

**Tests:** `a_save_into_a_full_queue_is_503_queue_full` (`tests/api.rs`) y `the_production_ingest_queue_holds_1024_saves` (`crates/clawg-core/src/runtime.rs`, mod tests). La capacidad se inyecta con `MemoryRuntime::new_for_test_with_capacity` (`runtime.rs:336`). Las filas las cubre `a_save_into_a_full_queue_leaves_no_outbox_row` (`tests/api.rs`).

### `operation_id`

Toda respuesta `201` y `202` de `POST /v1/memories` MUST llevar `operation_id`. Sin uno en el pedido, el daemon MUST emitir un UUID v4. Con uno (`SaveRequest.operation_id`, `api.rs:336`), MUST validarlo (1 a 128 caracteres ASCII visibles, `MAX_OPERATION_ID_LEN`; si no, `400`; `check_operation_id`, `api.rs:483`) y usarlo. `SaveRequest` no usa `deny_unknown_fields`, así que el campo es aditivo.

Mientras exista una fila con ese `operation_id` para ese agente, un save con el mismo MUST NOT crear otra ni encolar nada (`Accepted::Existing`, `runtime.rs:74`): con la fila `pending` responde `202` con `reason: "already_queued"`; con la fila `failed`, `500 ingest_failed` con su causa en `detail`. Después de completado el save, la fila ya no existe: un reenvío vuelve a guardar y el dedup lo absorbe como `reinforced`. El `operation_id` identifica el save, no su texto: un reenvío con otro contenido y el mismo id no guarda el contenido nuevo mientras la fila exista.

La herramienta `save_memory` MUST NOT tener un argumento `operation_id`: el modelo no tiene un id estable que repetir, y el esquema pagaría tokens en cada sesión.

#### Escenario: id inventado, dado e inválido
- GIVEN un daemon con el embedder listo
- WHEN se guarda un texto sin `operation_id`, otro con `hook-1` y otro con `has space`
- THEN el primero responde `201` con un UUID v4, el segundo `201` con `hook-1` y el tercero `400`

#### Escenario: el mismo id dos veces
- GIVEN un daemon con el embedder `loading` y un save con `operation_id: "op-1"` aceptado
- WHEN se guarda otra vez con `op-1`
- THEN la respuesta es `202` con `reason: "already_queued"` y `operation_id: "op-1"`, y el outbox tiene una fila

**Tests:** `a_save_answers_with_an_operation_id` y `an_operation_id_sent_twice_keeps_one_row` (`tests/api.rs`); `operation_ids_are_checked` y `a_confirmed_save_names_its_operation_and_what_was_masked_before_it_was_recorded` (`api.rs`, mod tests); `an_operation_id_already_recorded_is_not_queued_again` (`runtime.rs`, mod tests); `an_operation_id_is_one_row_per_agent` (`outbox.rs`, mod tests). La respuesta `500 ingest_failed` a un id con fila `failed` no tiene test propio.

### Contenido vacío

Un save con contenido vacío o sólo espacios MUST responder `400` antes de consultar el estado del embedder y antes de encolar o escribir una fila. El mensaje nombra el campo (`content must not be empty`).

**Test:** `saving_empty_content_is_rejected_before_it_reaches_the_queue` (`tests/api.rs`).

## Estado del embedder y bootstrap

### Estado del outbox en `/health`

`/health` MUST publicar `ingest` (`HealthBody.ingest`, `api.rs:447`; `outbox_summary`, `outbox.rs:380`) con `pending` y `failed` (cantidad de filas de cada estado), `oldest_pending_s` (edad en segundos de la fila `pending` más vieja, `null` sin ninguna) y hasta 5 `problems` (`PROBLEMS_SHOWN`, `outbox.rs:27`): las filas con `last_error`, las modificadas más recientemente primero, cada una con `operation_id`, `agent_id`, `state`, `attempts` y `error`. MUST NOT llevar el texto de ninguna fila.

La lectura MUST NOT hacer esperar a `/health` más de 250 ms (`HEALTH_INGEST_WAIT`, `api.rs:660`): `/health` lo consulta `probe_running` antes de cada autoarranque con un tope de 1 s (`HEALTH_TIMEOUT`, `client.rs:45`), y la conexión del store es una sola, así que una escritura larga haría esperar a `/health`. Si la lectura vence o falla, el campo `ingest` MUST omitirse de la respuesta (no se publica como `null`). El cliente lo lee como `Option` con `#[serde(default)]` (`Health.ingest`, `client.rs:135`): un daemon sin outbox, o uno ocupado, no lo trae y el cliente no inventa datos. `memory_status` lo muestra porque imprime el `Health` entero ([`specs/mcp`](../mcp/spec.md)).

#### Escenario: un save fallido
- GIVEN una fila `failed` con `operation_id` `op-1`, `attempts = 3` y una causa
- THEN `/health` tiene `ingest.pending = 0`, `ingest.failed = 1` y `ingest.problems[0]` con `operation_id: "op-1"`, `agent_id` y la causa, sin ningún campo `content`

#### Escenario: un daemon anterior
- GIVEN un `/health` sin `ingest`
- THEN el cliente lo parsea y `ingest` queda vacío

**Tests:** `health_reports_a_failed_save_with_its_operation_id` (`tests/api.rs`); `the_summary_counts_rows_and_lists_the_latest_causes` (`outbox.rs`, mod tests); `a_health_without_ingest_still_parses` (`client.rs`, mod tests). El tope de 250 ms y la omisión del campo cuando vence no tienen test.

### Estado en `/health` y `clawg status`

`/health` MUST publicar `embedder` con uno de `loading`, `ready` o `failed` (`health`, `api.rs:662`; `HealthBody`, `api.rs:426`). Con `failed` MUST agregar `embedder_error` (texto) y `embedder_retryable` (booleano). Con los otros dos estados esos campos son `null`.

`clawg status` serializa el `Health` del cliente (`crates/clawg-cli/src/client.rs:117`), que tiene los mismos campos. `embedder_error` y `embedder_retryable` son `#[serde(default)]`: un cliente nuevo contra un daemon que no los publica sigue parseando.

#### Escenario: descarga fallida
- GIVEN un bootstrap cuya descarga falló con `connection refused`
- THEN `/health` tiene `embedder: "failed"`, `embedder_error` contiene `connection refused` y `embedder_retryable: true`

#### Escenario: contrato distinto
- GIVEN un bootstrap cuyo contrato no coincide
- THEN `embedder: "failed"` y `embedder_retryable: false`

**Tests:** `health_reports_failed_with_the_cause_after_a_failed_bootstrap` (`tests/api.rs`) y `a_failed_embedder_shows_in_the_status_report` (`crates/clawg-cli/src/status.rs`).

### Reintento del bootstrap

Un bootstrap que falla por descarga, carga o verificación del contrato (error al leer la base) MUST reintentarse. Un contrato que no coincide MUST NOT reintentarse (`classify`, `bootstrap.rs:68`).

La espera tras el intento fallido `n` (1, 2, …) MUST ser `min(30 s · 2^(n-1), 30 min)`, con saturación para no desbordar (`retry_delay`, `bootstrap.rs:86`). No hay tope de intentos. Durante la espera el estado es `failed` con `retryable: true`; durante un intento es `loading`. El bucle es `run_embedder_bootstrap_with_retry` (`bootstrap.rs:550`), que el daemon lanza con `tokio::time::sleep` (`crates/clawg-cli/src/daemon/serve.rs:93`); `classify` y `retry_delay` son funciones puras y la espera se inyecta, así que los tests corren sin dormir.

#### Escenario: se recupera
- GIVEN un `ensure` falso que falla la primera vez y funciona la segunda
- WHEN corre el bucle de bootstrap con esperas de prueba en cero
- THEN el embedder queda `ready` después del segundo intento

#### Escenario: contrato
- GIVEN un `verify` que devuelve `Mismatch`
- THEN hay un único intento y el estado queda `failed` con `retryable: false`

#### Escenario: espera
- THEN la espera es 30 s, 60 s y 120 s tras los intentos 1, 2 y 3, y 30 min a partir del intento 7

**Tests:** `a_failed_download_is_retried_until_it_succeeds`, `a_contract_mismatch_is_never_retried`, `retry_delay_doubles_from_30s_and_caps_at_30min` y `classify_retries_everything_but_a_contract_mismatch` (`crates/clawg-core/src/memory/bootstrap.rs`, mod tests).

## Herramienta MCP `save_memory`

### Texto que ve el modelo

`save_memory` MUST traducir cada respuesta del daemon a un texto para el modelo (`save_outcome_text`, `crates/clawg-cli/src/mcp/server.rs:495`). La función es pura:

| Respuesta | Resultado MCP | Texto |
|---|---|---|
| `201`, un elemento `inserted` | éxito | `saved #57` |
| `201`, varios elementos | éxito | `saved as N pieces: #57, #58, #59; read_memory with any of them returns the whole text` |
| `201`, un elemento `reinforced` | éxito | `already stored as #57; reinforced it instead of saving a copy (update_memory changes its importance or type)` |
| `202 unconfirmed` | éxito | `accepted as operation <id> but not confirmed yet; memory_status lists it if it fails, list_memories shows it once stored` |
| `202 embedder_loading` | éxito | `accepted as operation <id>; it will be stored once the embedding model finishes loading (memory_status lists it if it fails)` |
| `202 already_queued` | éxito | `accepted as operation <id> earlier and still waiting to be stored` |
| `503 embedder_failed` | error | `not saved: the embedding model failed to load (<causa>)`, más `; clawg will retry` si es reintentable |
| `500 ingest_failed` / `503 embedder_unavailable` | error | `not saved: <detalle>`, más `; partially saved as #…` si hubo filas |
| `503 ingest_queue_full` | error | `not saved: the ingest queue is full; try again shortly` |

Los textos de un `202` MUST nombrar el `operation_id` (sin él, dicen sólo `accepted`) y decir que un fallo aparece en `memory_status`. MUST NOT contener la palabra `queued` sola. Un save «exitoso» sólo se reporta como `saved` cuando el daemon confirmó las filas. Los ids MUST servir tal cual para `delete_memory` y `update_memory` con `kind: "memory"` (ver [`specs/update`](../update/spec.md), R5). Cualquier otro error pasa por `explain` (`server.rs:439`), que con `embedder_failed` MUST NOT prometer que la búsqueda va a funcionar en un minuto.

#### Escenario: id usable
- GIVEN `save_memory` devolvió `saved #57`
- WHEN se llama `delete_memory` con `kind: "memory"` e `id: 57`
- THEN el borrado tiene éxito

#### Escenario: bootstrap fallido
- GIVEN el embedder `failed`
- THEN `save_memory` devuelve un resultado de error que nombra la causa

**Tests:** `save_outcome_text_covers_every_daemon_answer`, `save_memory_returns_the_id_that_delete_memory_accepts`, `save_memory_is_an_error_when_the_embedder_failed` y `explain_does_not_promise_search_when_the_embedder_failed` (`server.rs`, mod tests).

### El cliente HTTP devuelve el resultado parseado

`DaemonClient::save` (`client.rs:521`) MUST devolver `SaveOutcome` (`Saved { memories, redacted }` o `Queued { reason, operation_id }`, `client.rs:367`), no `()`. `operation_id` es `None` con un daemon anterior al outbox. Los errores son `ClientError::Daemon { status, message, details }`, y `details` (`ErrorDetails`, `client.rs:84`) MUST conservar `detail`, `retryable` y `memories` del cuerpo de error para que la herramienta los muestre.

**Test:** `save_returns_the_ids_the_daemon_reported` (`client.rs`, mod tests).

## Troceo del save

### Hay un solo chunker, también para el código

El worker MUST trocear el contenido de cada save con el chunker de chat (`ChatChunker::split_save`, `crates/clawg-core/src/memory/chunker/chat.rs:88`), sea cual sea su `memory_type` y su `source`, incluido `code_block`. Lo único que la elección por evento decide es si el evento se descarta: `select_chunker` (`crates/clawg-core/src/memory/chunker/mod.rs:96`) devuelve `ChunkerKind::Skip` para un contenido vacío y para el rol `System`, y `ChunkerKind::Chat` para el resto (`ingest_pieces`, `worker.rs:600`); un evento de fuente `Compaction` ni consulta la elección y siempre va al chat chunker. No hay chunker de código en la ingesta: el código de un proyecto se trocea para su propio índice, que no es `memory` ([`specs/project-docs`](../project-docs/spec.md)).

Una fila guardada con `source = 'code_file'` o `memory_type = 'code_block'` MUST seguir leyéndose y listándose como cualquier otra (`MemorySource::CodeFile`, `events.rs:58`; `MemoryType::CodeBlock`, `store_insert.rs:23`).

#### Escenario: un save de código
- GIVEN un `save_memory` con `memory_type = "code_block"` y un bloque de Rust de 3 líneas
- WHEN lo procesa el worker
- THEN queda una fila con ese contenido tal cual y ese tipo

**Test:** `a_code_block_save_is_chunked_like_any_other` en `crates/clawg-core/src/memory/worker.rs`.

## Invariantes

- El worker es el único escritor de la ingesta: el handler nunca inserta en `memory`, sólo registra la fila del outbox, encola y espera.
- El dedup, el chunker, la detección de contradicciones y el orden FIFO del worker no dependen de que el que guarda espere: un evento sin canal de respuesta se procesa igual.
