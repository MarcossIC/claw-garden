# ingest-durability — Especificación

**Dominio destino:** `specs/ingest/spec.md` (se fusiona al archivar; hoy `openspec/specs/` está vacío, así que es una spec completa, no un delta).

**Alcance:** el contrato de `POST /v1/memories` y de la herramienta `save_memory`, el estado del embedder que publica `/health`, el reintento del bootstrap y la capacidad de la cola de ingesta. No cambia el esquema ni ningún dato guardado.

## Definiciones

- **Worker:** el único consumidor de la cola de ingesta (`crates/clawg-core/src/memory/worker.rs:137-175`). Sigue siendo único: ningún requisito agrega otro escritor.
- **Resultado de un chunk:** `inserted` (fila nueva) o `reinforced` (el dedup absorbió el chunk en una fila existente, `worker.rs:231-241`). En los dos casos hay un id de la tabla `memory`.
- **Estado del embedder:** `loading`, `ready` o `failed`. `failed` lleva la causa y si se va a reintentar (`retryable`).
- **Save confirmado:** el handler recibió el resultado del worker antes del timeout.
- **Timeout de confirmación:** 10 s por defecto. Es una constante; el valor se ajusta con la medición de la propuesta.

## R1. Save confirmado

Con el embedder `ready`, `POST /v1/memories` MUST esperar el resultado del worker para ese evento y responder `201` con un elemento por chunk, en el orden de los chunks:

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}]}
```

Las copias repetidas dentro del mismo save MUST NOT generar elementos extra: cuentan como refuerzo de la fila de su primera copia (`worker.rs:303-307`, `:373-377`).

#### Escenario: inserción
- GIVEN un daemon con el embedder listo y la base vacía
- WHEN se guarda `el tren sale los martes`
- THEN la respuesta es `201`, con un elemento `inserted` cuyo id
- AND `GET /v1/memories/memory/{id}` devuelve esa fila sin esperar ni hacer polling

#### Escenario: varios chunks
- GIVEN un contenido que el chunker parte en 3 chunks distintos
- THEN la respuesta tiene 3 elementos, todos `inserted`, con ids distintos

**Tests:** `a_save_answers_201_with_the_id_of_the_row_it_wrote` y `a_multi_chunk_save_returns_one_id_per_chunk` en `crates/clawg-cli/tests/api.rs`. Fallan hoy: el save responde `202` sin id (`api.rs:460`). El test existente `a_saved_memory_is_queued_then_becomes_a_listable_row` (`tests/api.rs:252`) se reescribe: el polling deja de hacer falta.

## R2. Duplicados

Un save cuyo contenido ya está guardado MUST responder `201` con `outcome: "reinforced"` y el id de la fila existente. MUST NOT insertar otra fila.

#### Escenario
- GIVEN un save de `X` que devolvió el id 57
- WHEN se guarda `X` otra vez, con el mismo agente y proyecto
- THEN la respuesta tiene un elemento `{"id": 57, "outcome": "reinforced"}` y la tabla sigue con una fila

**Test:** `saving_the_same_text_twice_returns_the_same_id_as_reinforced` en `tests/api.rs`. Falla hoy: no hay id.

## R3. Fallo en el worker

Si el worker no puede guardar el evento (embedder caído, batch fallido, menos vectores que chunks, insert fallido), el save MUST responder con error y la causa:

- `503` con `error: "embedder_unavailable"` si el embedder no estaba disponible (`EmbedError::NotReady` o `WorkerGone`, `worker.rs:324-327`);
- `500` con `error: "ingest_failed"` y un `detail` legible en los demás casos.

El worker MUST cortar el evento en el primer fallo: hoy sigue con los chunks siguientes después de un insert fallido (`worker.rs:369-372`), lo que deja un hueco en el medio del texto. Si algún chunk ya se había escrito antes del fallo (el insert va chunk por chunk, `worker.rs:358-378`), la respuesta MUST incluir esos elementos en `memories`. Los chunks que no se escribieron MUST NOT aparecer.

El log actual (`log::warn!`) MAY quedar, pero ya no es el único rastro del fallo.

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

**Tests:** `a_failed_embed_reaches_the_caller_as_an_error` en `tests/api.rs`, y `process_event_reports_the_rows_written_before_an_insert_failed` en `worker.rs` (mod tests). Fallan hoy: `process_event` devuelve `Ok(())` en los dos caminos (`worker.rs:323-331`, `:369-372`).

## R4. Timeout de confirmación

Si el worker no contesta dentro del timeout, el save MUST responder `202` con:

```json
{"status": "queued", "reason": "unconfirmed"}
```

El evento MUST seguir en la cola: el handler deja de esperar, pero no lo retira. Cuando el worker lo procese, el resultado se descarta sin error.

Si el worker suelta el canal de respuesta sin contestar (por ejemplo, porque el apagado de data-preservation vuelca el evento a disco en lugar de procesarlo), el save MUST responder lo mismo que con el timeout, sin esperar a que venza.

#### Escenario
- GIVEN un worker detenido a propósito (el test no lo lanza) y un timeout de prueba de 100 ms
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "unconfirmed"`, en menos de 1 s

**Test:** `a_save_the_worker_does_not_confirm_in_time_answers_202_unconfirmed` en `tests/api.rs`. Falla hoy: no hay `reason`. El timeout se inyecta en `AppCtx` para que el test no espere 10 s.

## R5. Embedder cargando

Con el embedder `loading`, el save MUST encolar el evento y responder `202` al instante, sin esperar al worker:

```json
{"status": "queued", "reason": "embedder_loading"}
```

El worker MUST seguir reteniendo esos eventos hasta que el modelo cargue (`worker.rs:191-206`), igual que hoy.

#### Escenario
- GIVEN un daemon con el embedder `loading`
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "embedder_loading"`
- AND cuando el embedder pasa a `ready`, la fila aparece

**Test:** `a_save_while_the_model_loads_answers_202_and_lands_once_it_loads` en `tests/api.rs`. Falla hoy: no hay `reason`.

## R6. Embedder fallido

Con el embedder `failed`, el save MUST responder `503` sin encolar:

```json
{"error": "embedder_failed", "detail": "<causa>", "retryable": true}
```

`POST /v1/search` y `POST /v1/context` MUST distinguir `embedder_failed` de `embedder_not_ready` con el mismo cuerpo.

#### Escenario
- GIVEN un bootstrap cuya descarga falla
- WHEN se guarda un texto
- THEN la respuesta es `503 embedder_failed` con la causa, y la cola sigue vacía

**Tests:** `a_save_after_the_bootstrap_failed_is_503_with_the_cause` y `search_after_the_bootstrap_failed_says_failed_not_loading` en `tests/api.rs`. Fallan hoy: el save responde `202` y la búsqueda `embedder_not_ready` (`api.rs:471`).

## R7. Estado en `/health` y `clawg status`

`/health` MUST publicar `embedder` con uno de `loading`, `ready` o `failed`. Con `failed` MUST agregar `embedder_error` (texto) y `embedder_retryable` (booleano). Con los otros dos estados esos campos MUST ser `null`. `clawg status` serializa el `Health` del cliente (`crates/clawg-cli/src/status.rs:34`, `crates/clawg-cli/src/client.rs:83-90`), así que ese struct MUST sumar los dos campos para que `status` los muestre.

`Health::embedder` ya es un `String` en el cliente (`client.rs:88`): un cliente viejo contra un daemon nuevo sigue parseando, y los campos nuevos MUST ser opcionales (`#[serde(default)]`) para que un cliente nuevo contra un daemon viejo también.

#### Escenario
- GIVEN un bootstrap cuya descarga falló con `connection refused`
- THEN `/health` tiene `embedder: "failed"`, `embedder_error` contiene `connection refused` y `embedder_retryable: true`

#### Escenario: contrato distinto
- GIVEN un bootstrap cuyo contrato no coincide
- THEN `embedder: "failed"` y `embedder_retryable: false`

**Tests:** `health_reports_failed_with_the_cause_after_a_failed_bootstrap` en `tests/api.rs` y `a_failed_embedder_shows_in_the_status_report` en `status.rs`. Fallan hoy: `/health` sólo conoce `ready` y `loading` (`api.rs:414`).

## R8. Reintento del bootstrap

Un bootstrap que falla por descarga, carga o verificación del contrato (error al leer la base, `bootstrap.rs:303-317`) MUST reintentarse. Un contrato que no coincide (`bootstrap.rs:281-302`) MUST NOT reintentarse.

La espera antes del intento `n` (1, 2, …) MUST ser `min(30 s · 2^(n-1), 30 min)`. No hay tope de intentos. Durante la espera el estado es `failed` con `retryable: true`; durante un intento es `loading`. El cálculo de la espera y la clasificación del error MUST ser funciones puras.

#### Escenario: se recupera
- GIVEN un `ensure` falso que falla la primera vez y funciona la segunda
- WHEN corre el bucle de bootstrap con esperas de prueba en cero
- THEN el embedder queda `ready` después del segundo intento

#### Escenario: contrato
- GIVEN un `verify` que devuelve `Mismatch`
- THEN hay un único intento y el estado queda `failed` con `retryable: false`

#### Escenario: espera
- THEN la espera es 30 s, 60 s y 120 s para los intentos 1, 2 y 3, y 30 min a partir del intento 7

**Tests:** `a_failed_download_is_retried_until_it_succeeds`, `a_contract_mismatch_is_never_retried`, `retry_delay_doubles_from_30s_and_caps_at_30min` en `bootstrap.rs` (mod tests). Fallan hoy: no hay bucle (`serve.rs:84-91` lanza un único intento).

## R9. Cola acotada

La cola de ingesta MUST tener una capacidad de 1.024 eventos. Si está llena, el save MUST responder `503` con `error: "ingest_queue_full"` y MUST NOT bloquear el request.

#### Escenario
- GIVEN una cola de capacidad de prueba 2, sin worker, y el embedder `loading`
- WHEN se guardan 3 textos
- THEN los dos primeros responden `202` y el tercero `503 ingest_queue_full`

**Test:** `a_save_into_a_full_queue_is_503_queue_full` en `tests/api.rs`. Falla hoy: la cola no tiene límite (`crates/clawg-core/src/runtime.rs:95`).

## R10. Herramienta MCP `save_memory`

`save_memory` MUST traducir cada respuesta del daemon a un texto para el modelo:

| Respuesta | Resultado MCP | Texto |
|---|---|---|
| `201`, un elemento `inserted` | éxito | `saved #57` |
| `201`, varios elementos | éxito | `saved as N pieces: #57, #58, #59` |
| `201`, `reinforced` | éxito | `already stored as #57; reinforced it instead of saving a copy` |
| `202 unconfirmed` | éxito | `accepted but not confirmed yet; check with list_memories before relying on it` |
| `202 embedder_loading` | éxito | `accepted; it will be stored once the embedding model finishes loading` |
| `503 embedder_failed` | error | `not saved: the embedding model failed to load (<causa>)`, más `clawg will retry` si es reintentable |
| `500` / `503` del worker | error | `not saved: <detalle>`, más `partially saved as #…` si hubo filas |
| `503 ingest_queue_full` | error | `not saved: the ingest queue is full; try again shortly` |

Los textos MUST NOT contener la palabra `queued` sola. `explain` (`crates/clawg-cli/src/mcp/server.rs:229-242`) MUST dejar de prometer "search will work in a minute" cuando el embedder está `failed`.

Los ids MUST servir tal cual para `delete_memory` con `kind: "memory"`.

#### Escenario: id usable
- GIVEN `save_memory` devolvió `saved #57`
- WHEN se llama `delete_memory` con `kind: "memory"` e `id: 57`
- THEN el borrado tiene éxito

#### Escenario: bootstrap fallido
- GIVEN el embedder `failed`
- THEN `save_memory` devuelve un resultado de error que nombra la causa

**Tests:** en `server.rs` (mod tests): `save_memory_returns_the_id_that_delete_memory_accepts` (reescribe `save_then_list_then_search_then_delete_through_the_tools`, que hoy asierta `queued`, `server.rs:774`), `save_memory_is_an_error_when_the_embedder_failed`, `explain_does_not_promise_search_when_the_embedder_failed`. Para el texto, `save_outcome_text_covers_every_daemon_answer` como test puro sobre la función que arma el texto. Fallan hoy: la herramienta devuelve `queued` siempre.

## R11. Cliente HTTP

`DaemonClient::save` (`client.rs:337-348`) MUST devolver el resultado parseado (`Saved { memories }` o `Queued { reason }`) en lugar de `()`. Los errores siguen siendo `ClientError::Daemon { status, message }`, y el cuerpo de error MUST conservar `memories`, `detail` y `retryable` para que la herramienta MCP los muestre.

**Test:** `save_returns_the_ids_the_daemon_reported` en `client.rs` (mod tests). Falla hoy: `save` devuelve `()`.

## R12. Sin cambios

- Contenido vacío: `400` antes de encolar (`api.rs:437-439`, test `tests/api.rs:282`).
- El dedup, el chunker, la detección de contradicciones y el orden FIFO del worker no cambian.
- El worker sigue siendo el único escritor de la ingesta.
- Ningún dato guardado cambia; no hay migración.
- Lo que se pierde al parar el daemon (eventos con `202`) sigue fuera de alcance: data-preservation, parte A.

**Test:** la base de 1.304 tests sigue verde, con los tests de `tests/api.rs` y `server.rs` reescritos como dicen R1 y R10.

## Trazabilidad

| Requisito | Test(s) | Archivo |
|---|---|---|
| R1 | `a_save_answers_201_with_the_id_of_the_row_it_wrote`, `a_multi_chunk_save_returns_one_id_per_chunk` | `crates/clawg-cli/tests/api.rs` |
| R2 | `saving_the_same_text_twice_returns_the_same_id_as_reinforced` | `tests/api.rs` |
| R3 | `a_failed_embed_reaches_the_caller_as_an_error`; `process_event_reports_the_rows_written_before_an_insert_failed` | `tests/api.rs`; `worker.rs` |
| R4 | `a_save_the_worker_does_not_confirm_in_time_answers_202_unconfirmed` | `tests/api.rs` |
| R5 | `a_save_while_the_model_loads_answers_202_and_lands_once_it_loads` | `tests/api.rs` |
| R6 | `a_save_after_the_bootstrap_failed_is_503_with_the_cause`, `search_after_the_bootstrap_failed_says_failed_not_loading` | `tests/api.rs` |
| R7 | `health_reports_failed_with_the_cause_after_a_failed_bootstrap`; `a_failed_embedder_shows_in_the_status_report` | `tests/api.rs`; `status.rs` |
| R8 | `a_failed_download_is_retried_until_it_succeeds`, `a_contract_mismatch_is_never_retried`, `retry_delay_doubles_from_30s_and_caps_at_30min` | `bootstrap.rs` |
| R9 | `a_save_into_a_full_queue_is_503_queue_full` | `tests/api.rs` |
| R10 | `save_memory_returns_the_id_that_delete_memory_accepts`, `save_memory_is_an_error_when_the_embedder_failed`, `explain_does_not_promise_search_when_the_embedder_failed`, `save_outcome_text_covers_every_daemon_answer` | `server.rs` |
| R11 | `save_returns_the_ids_the_daemon_reported` | `client.rs` |
