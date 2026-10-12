# ingest — Especificación

Fuente de verdad del comportamiento actual de la ingesta: el contrato de `POST /v1/memories` y de la herramienta `save_memory`, el estado del embedder que publica `/health`, el reintento del bootstrap del modelo y la capacidad de la cola.

**Origen:** [ingest-durability](../../changes/archive/2026-10-12-ingest-durability/proposal.md), archivado el 2026-10-12.

## Definiciones

- **Worker:** el único consumidor de la cola de ingesta (`memory_worker`, `crates/clawg-core/src/memory/worker.rs:194`). Ninguna ruta de ingesta agrega otro escritor: el dedup sólo es correcto con un consumidor secuencial.
- **Resultado de un chunk:** `inserted` (fila nueva) o `reinforced` (el dedup absorbió el chunk en una fila existente). En los dos casos hay un id de la tabla `memory` (`SavedChunk` y `ChunkOutcome`, `worker.rs:36` y `worker.rs:27`).
- **Reporte de un evento:** `IngestReport` (`worker.rs:73`): los chunks que quedaron en la base, en orden, y la causa que cortó el evento si la hubo. Lleva las dos partes a la vez porque un fallo a mitad de un evento deja chunks ya escritos.
- **Estado del embedder:** `EmbedderStatus` (`crates/clawg-core/src/memory/bootstrap.rs:35`): `Loading`, `Ready` o `Failed { error, retryable }`. El `OnceCell` del embedder se llena antes de publicar `Ready`.
- **Save confirmado:** el handler recibió el reporte del worker antes de `SAVE_CONFIRM_TIMEOUT`.
- **Timeout de confirmación (`SAVE_CONFIRM_TIMEOUT`, `crates/clawg-cli/src/daemon/api.rs:80`):** 10 s. Vive en `AppCtx::save_timeout` para que un test lo reduzca.
- **Capacidad de la cola (`INGEST_QUEUE_CAPACITY`, `crates/clawg-core/src/runtime.rs:44`):** 1024 eventos.

## Contrato de `POST /v1/memories`

### Un save confirmado devuelve los ids

Con el embedder `ready`, `POST /v1/memories` MUST encolar el evento con un canal de respuesta, esperar el reporte del worker y responder `201` con un elemento por chunk, en el orden de los chunks (`save_memory`, `api.rs:578`; `save_response`, `api.rs:538`):

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}]}
```

`kind` es siempre `"memory"`, de modo que `kind` e `id` son exactamente lo que toma `DELETE /v1/memories/{kind}/{id}`. Las copias repetidas dentro del mismo save MUST NOT generar elementos extra: cuentan como refuerzo de la fila de su primera copia (`collapse_duplicate_chunks`, `worker.rs:134`, y `process_event`, `worker.rs:408`).

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

Si el worker no puede guardar el evento (embedder caído, batch fallido, menos vectores que chunks, insert fallido), el save MUST responder con error y la causa (`save_response`, `api.rs:538`):

- `503` con `error: "embedder_unavailable"` y `detail` si el embedder no estaba disponible (`EmbedError::NotReady` o `WorkerGone`, o la celda vacía, en `process_event`);
- `500` con `error: "ingest_failed"` y un `detail` legible en los demás casos (`embed failed: …`, `embedder returned N vectors for M chunks`, `store insert failed: …`).

El worker MUST cortar el evento en el primer fallo: lo que queda guardado es siempre un prefijo del texto, nunca uno con un hueco en el medio. El cuerpo de error MUST incluir en `memories` los chunks que ya estaban en la base antes del fallo, sean insertados o reforzados, en orden. Los chunks que no se escribieron MUST NOT aparecer. El worker además registra el fallo con `log::warn!`.

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

**Tests:** `a_failed_embed_reaches_the_caller_as_an_error` (`tests/api.rs`); `process_event_reports_the_rows_written_before_an_insert_failed`, `process_event_stops_at_the_first_failed_insert`, `a_failed_embed_batch_reports_the_failure_and_no_rows` y `an_unavailable_embedder_reports_embedder_unavailable` (`worker.rs`, mod tests); y los unit de `save_response` (`api.rs`, mod tests).

### Timeout de confirmación

Si el worker no contesta dentro de `AppCtx::save_timeout`, el save MUST responder `202` con:

```json
{"status": "queued", "reason": "unconfirmed"}
```

El evento MUST seguir en la cola: el handler deja de esperar, pero no lo retira. Cuando el worker lo procese, el reporte se descarta sin error.

Si el worker suelta el canal de respuesta sin contestar, el save MUST responder lo mismo que con el timeout, sin esperar a que venza. Es lo que ocurre cuando un apagado ordenado vuelca el evento a disco en lugar de procesarlo (ver [storage](../storage/spec.md), «Ningún evento aceptado se pierde en un apagado ordenado»): el evento se reinyecta al próximo arranque.

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
{"status": "queued", "reason": "embedder_loading"}
```

El worker MUST retener esos eventos hasta que el modelo cargue (`hold_for_embedder`, `worker.rs:321`). Un apagado ordenado que llega antes los vuelca (ver [storage](../storage/spec.md)).

#### Escenario
- GIVEN un daemon con el embedder `loading`
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "embedder_loading"`
- AND cuando el embedder pasa a `ready`, la fila aparece

**Test:** `a_save_while_the_model_loads_answers_202_and_lands_once_it_loads` (`tests/api.rs`).

### Embedder fallido

Con el embedder `failed`, el save MUST responder `503` sin encolar:

```json
{"error": "embedder_failed", "detail": "<causa>", "retryable": true}
```

`POST /v1/search` MUST responder el mismo cuerpo cuando el embedder está `failed`, y `embedder_not_ready` en los demás casos en que la celda está vacía (`search`, `api.rs:634`). `POST /v1/context` no responde ninguno de los dos: sin embedder cae a la recuperación léxica.

#### Escenario
- GIVEN un bootstrap cuya descarga falla
- WHEN se guarda un texto
- THEN la respuesta es `503 embedder_failed` con la causa, y la cola sigue vacía

**Tests:** `a_save_after_the_bootstrap_failed_is_503_with_the_cause` y `search_after_the_bootstrap_failed_says_failed_not_loading` (`tests/api.rs`).

### Cola acotada

La cola de ingesta MUST tener capacidad `INGEST_QUEUE_CAPACITY` (1024 eventos). Si está llena, el save MUST responder `503` con `error: "ingest_queue_full"` y MUST NOT bloquear el request: el handler usa `try_send`. Si el worker ya no existe (canal cerrado), el save responde `500`.

#### Escenario
- GIVEN una cola de capacidad de prueba 2, sin worker, y el embedder `loading`
- WHEN se guardan 3 textos
- THEN los dos primeros responden `202` y el tercero `503 ingest_queue_full`

**Tests:** `a_save_into_a_full_queue_is_503_queue_full` (`tests/api.rs`) y `the_production_ingest_queue_holds_1024_saves` (`crates/clawg-core/src/runtime.rs`, mod tests). La capacidad se inyecta con `MemoryRuntime::new_for_test_with_capacity` (`runtime.rs:270`).

### Contenido vacío

Un save con contenido vacío o sólo espacios MUST responder `400` antes de consultar el estado del embedder y antes de encolar. El mensaje nombra el campo (`content must not be empty`).

**Test:** `saving_empty_content_is_rejected_before_it_reaches_the_queue` (`tests/api.rs`).

## Estado del embedder y bootstrap

### Estado en `/health` y `clawg status`

`/health` MUST publicar `embedder` con uno de `loading`, `ready` o `failed` (`health`, `api.rs:510`; `HealthBody`, `api.rs:351`). Con `failed` MUST agregar `embedder_error` (texto) y `embedder_retryable` (booleano). Con los otros dos estados esos campos son `null`.

`clawg status` serializa el `Health` del cliente (`crates/clawg-cli/src/client.rs:114`), que tiene los mismos campos. `embedder_error` y `embedder_retryable` son `#[serde(default)]`: un cliente nuevo contra un daemon que no los publica sigue parseando.

#### Escenario: descarga fallida
- GIVEN un bootstrap cuya descarga falló con `connection refused`
- THEN `/health` tiene `embedder: "failed"`, `embedder_error` contiene `connection refused` y `embedder_retryable: true`

#### Escenario: contrato distinto
- GIVEN un bootstrap cuyo contrato no coincide
- THEN `embedder: "failed"` y `embedder_retryable: false`

**Tests:** `health_reports_failed_with_the_cause_after_a_failed_bootstrap` (`tests/api.rs`) y `a_failed_embedder_shows_in_the_status_report` (`crates/clawg-cli/src/status.rs`).

### Reintento del bootstrap

Un bootstrap que falla por descarga, carga o verificación del contrato (error al leer la base) MUST reintentarse. Un contrato que no coincide MUST NOT reintentarse (`classify`, `bootstrap.rs:66`).

La espera tras el intento fallido `n` (1, 2, …) MUST ser `min(30 s · 2^(n-1), 30 min)`, con saturación para no desbordar (`retry_delay`, `bootstrap.rs:84`). No hay tope de intentos. Durante la espera el estado es `failed` con `retryable: true`; durante un intento es `loading`. El bucle es `run_embedder_bootstrap_with_retry` (`bootstrap.rs:416`), que el daemon lanza con `tokio::time::sleep` (`crates/clawg-cli/src/daemon/serve.rs:88`); `classify` y `retry_delay` son funciones puras y la espera se inyecta, así que los tests corren sin dormir.

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

`save_memory` MUST traducir cada respuesta del daemon a un texto para el modelo (`save_outcome_text`, `crates/clawg-cli/src/mcp/server.rs:260`). La función es pura:

| Respuesta | Resultado MCP | Texto |
|---|---|---|
| `201`, un elemento `inserted` | éxito | `saved #57` |
| `201`, varios elementos | éxito | `saved as N pieces: #57, #58, #59` |
| `201`, un elemento `reinforced` | éxito | `already stored as #57; reinforced it instead of saving a copy` |
| `202 unconfirmed` | éxito | `accepted but not confirmed yet; check with list_memories before relying on it` |
| `202 embedder_loading` | éxito | `accepted; it will be stored once the embedding model finishes loading` |
| `503 embedder_failed` | error | `not saved: the embedding model failed to load (<causa>)`, más `; clawg will retry` si es reintentable |
| `500 ingest_failed` / `503 embedder_unavailable` | error | `not saved: <detalle>`, más `; partially saved as #…` si hubo filas |
| `503 ingest_queue_full` | error | `not saved: the ingest queue is full; try again shortly` |

Los textos MUST NOT contener la palabra `queued` sola. Un save «exitoso» sólo se reporta como `saved` cuando el daemon confirmó las filas. Los ids MUST servir tal cual para `delete_memory` con `kind: "memory"`. Cualquier otro error pasa por `explain` (`server.rs:230`), que con `embedder_failed` MUST NOT prometer que la búsqueda va a funcionar en un minuto.

#### Escenario: id usable
- GIVEN `save_memory` devolvió `saved #57`
- WHEN se llama `delete_memory` con `kind: "memory"` e `id: 57`
- THEN el borrado tiene éxito

#### Escenario: bootstrap fallido
- GIVEN el embedder `failed`
- THEN `save_memory` devuelve un resultado de error que nombra la causa

**Tests:** `save_outcome_text_covers_every_daemon_answer`, `save_memory_returns_the_id_that_delete_memory_accepts`, `save_memory_is_an_error_when_the_embedder_failed` y `explain_does_not_promise_search_when_the_embedder_failed` (`server.rs`, mod tests).

### El cliente HTTP devuelve el resultado parseado

`DaemonClient::save` (`client.rs:403`) MUST devolver `SaveOutcome` (`Saved { memories }` o `Queued { reason }`), no `()`. Los errores son `ClientError::Daemon { status, message, details }`, y `details` (`ErrorDetails`, `client.rs:81`) MUST conservar `detail`, `retryable` y `memories` del cuerpo de error para que la herramienta los muestre.

**Test:** `save_returns_the_ids_the_daemon_reported` (`client.rs`, mod tests).

## Invariantes

- El worker es el único escritor de la ingesta: el handler nunca inserta, sólo encola y espera.
- El dedup, el chunker, la detección de contradicciones y el orden FIFO del worker no dependen de que el que guarda espere: un evento sin canal de respuesta se procesa igual.
