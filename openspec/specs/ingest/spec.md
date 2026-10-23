# ingest — Especificación

Fuente de verdad del comportamiento actual de la ingesta: el contrato de `POST /v1/memories` y de la herramienta `save_memory` (incluido el outbox donde queda un save aceptado, el `operation_id` y la sesión del host que se sella en cada fila), el estado del embedder y del outbox que publica `/health`, el reintento del bootstrap del modelo, la capacidad de la cola y qué chunker trocea un save. El índice de la documentación y el código de un proyecto, que se trocea aparte, está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [ingest-durability](../../changes/archive/2026-10-12-ingest-durability/proposal.md), archivado el 2026-10-12.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Troceo del save").
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (el outbox, el `operation_id` y el bloque `ingest` de `/health`; lo que el worker hace con una fila y el arranque y el apagado están en [`specs/storage`](../storage/spec.md), «Apagado sin pérdida»).
- [memory-protocol](../../changes/archive/2026-10-17-memory-protocol/proposal.md), archivado el 2026-10-17 (sólo «El tipo de memoria se valida al guardar» y la fila del `400` en «Texto que ve el modelo»).
- [conflict-candidates](../../changes/archive/2026-10-18-conflict-candidates/proposal.md), archivado el 2026-10-18 (sólo el campo `related` de la respuesta del save).
- [topic-revisions](../../changes/archive/2026-10-18-topic-revisions/proposal.md), archivado el 2026-10-18 (sólo «La clave de tema se valida y vuelve en la respuesta», «Un save con clave reemplaza la fila viva de su clave» y lo que `save_memory` dice de lo reemplazado). Lo que el agente ve de la clave en las demás herramientas está en [`specs/mcp`](../mcp/spec.md).
- [session-handoff](../../changes/archive/2026-10-19-session-handoff/proposal.md), archivado el 2026-10-19 (sólo «La sesión del host», sus R1 y R2). El bloque LAST SESSION está en [`specs/retrieval`](../retrieval/spec.md), y el registro desde los hooks en [`specs/hooks`](../hooks/spec.md).
- [model-download](../../changes/archive/2026-10-19-model-download/proposal.md), archivado el 2026-10-19 (sólo la causa de una descarga que agota sus intentos, en «Estado en `/health` y `clawg status`», y el origen inválido en «Reintento del bootstrap»). La descarga está en [`specs/storage`](../storage/spec.md), «Descarga del modelo».
- [outbox-lifecycle](../../changes/archive/2026-10-19-outbox-lifecycle/proposal.md), archivado el 2026-10-19 (sólo lo que contesta un save que falla con el que guardó esperando, `pieces` en el bloque `ingest` de `/health` y los textos nuevos de `save_memory` y `memory_status`). El reintento, la pausa del apagado y `clawg outbox` están en [`specs/storage`](../storage/spec.md).
- [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22 (el alcance de la clave y del dedup pasa de agente y proyecto a proyecto, dentro de lo que alcanza el que guarda).
- [save-bounds](../../changes/archive/2026-10-20-save-bounds/proposal.md), archivado el 2026-10-20 (sólo «El contenido: vacío o sobre el tope» y la fila del `400` por el tope en «Texto que ve el modelo»).

## Definiciones

- **Worker:** el único consumidor de la cola de ingesta (`memory_worker`, `crates/clawg-core/src/memory/worker.rs:303`). Ninguna ruta de ingesta agrega otro escritor: el dedup sólo es correcto con un consumidor secuencial.
- **Resultado de un chunk:** `inserted` (fila nueva) o `reinforced` (el dedup absorbió el chunk en una fila existente). En los dos casos hay un id de la tabla `memory` (`SavedChunk` y `ChunkOutcome`, `worker.rs:41` y `worker.rs:32`).
- **Reporte de un evento:** `IngestReport` (`worker.rs:82`): los chunks que quedaron en la base, en orden, y la causa que cortó el evento si la hubo. Lleva las dos partes a la vez porque un fallo a mitad de un evento deja chunks ya escritos. Lleva además `replaced`, las filas que el save reemplazó por su clave de tema (`worker.rs:90`), y `paused`, cuando un apagado lo detuvo entre dos pedazos ([`specs/storage`](../storage/spec.md)).
- **Estado del embedder:** `EmbedderStatus` (`crates/clawg-core/src/memory/bootstrap.rs:39`): `Loading`, `Ready` o `Failed { error, retryable }`. El `OnceCell` del embedder se llena antes de publicar `Ready`.
- **Outbox:** la tabla `ingest_outbox` de `memory.db` (migración V14, `crates/clawg-core/migrations/V14__ingest_outbox.sql`): un save aceptado, ya enmascarado, hasta que termina de guardarse. Su contenido (`content` es el texto enmascarado; `event`, el resto del evento en JSON; `progress`, las piezas ya guardadas) y lo que el worker hace con cada fila están en [`specs/storage`](../storage/spec.md), «Apagado sin pérdida».
- **Save aceptado:** uno que respondió `201` o `202`: tiene su fila confirmada en el outbox, o ya terminó de guardarse. Un save que el handler rechazó (`503 ingest_queue_full` o `500`) no es aceptado y no deja fila.
- **Fila `pending` / `failed`:** `state = 'pending'` (espera al worker) o `'failed'` (se la dio por perdida, con su causa en `last_error`).
- **`operation_id`:** el id con el que queda una fila; lo manda el que guarda o lo inventa el daemon (un UUID v4). Es único por agente (`UNIQUE (agent_id, operation_id)`).
- **Save confirmado:** el handler recibió el reporte del worker antes de `SAVE_CONFIRM_TIMEOUT`.
- **Timeout de confirmación (`SAVE_CONFIRM_TIMEOUT`, `crates/clawg-cli/src/daemon/api.rs:106`):** 10 s. Vive en `AppCtx::save_timeout` para que un test lo reduzca.
- **Clave de tema (`topic_key`):** el nombre estable con el que un save explícito dice de qué tema es una versión: segmentos no vacíos de `[a-z0-9._-]` separados por `/`, de 1 a 80 caracteres en total (`validate_topic_key`, `crates/clawg-core/src/memory/topic.rs:33`). Minúsculas solamente, para que un tema escrito en otra caja siga siendo una sola clave.
- **Alcance de una clave:** `(COALESCE(project_id, ''), topic_key)` dentro de lo que alcanza el que guarda ([`specs/sharing`](../sharing/spec.md)): las filas de un proyecto, o de ninguno, bajo una clave, de cualquier autor que `sharing.json` no le oculta. Es el mismo alcance del dedup, así que otro proyecto con la misma clave no se toca, y la clave nombra un tema del proyecto, no de un agente.
- **Fila viva:** una fila de `memory` sin `deleted_at` y con `status` `active` o `disputed`.
- **Capacidad de la cola (`INGEST_QUEUE_CAPACITY`, `crates/clawg-core/src/runtime.rs:49`):** 1024 eventos.

## Contrato de `POST /v1/memories`

### Aceptar es durable

Después de validar el pedido (contenido, `memory_type`, `operation_id`, `derived_from` y `topic_key`, cada uno un `400`) y de comprobar que el embedder no está `failed`, `POST /v1/memories` MUST pasar por `MemoryRuntime::accept` (`crates/clawg-core/src/runtime.rs:232`, llamado en `save_memory`, `api.rs:1364`), que en este orden:

1. enmascara el contenido (ver [`specs/secrets`](../secrets/spec.md), R2): la fila guarda el texto ya enmascarado;
2. escribe y confirma la fila del outbox (`outbox_record`, `crates/clawg-core/src/memory/outbox.rs:254`). El commit MUST hacerse con `synchronous = FULL` y la conexión MUST volver después a su valor (`durably`, `outbox.rs:229`): un save aceptado sobrevive también a un corte de luz, a costa de un fsync por save. El resto de la base, incluidos el progreso y el borrado final de la fila, sigue en `NORMAL`;
3. encola el evento con `try_send`.

La fila MUST estar confirmada antes de encolar el evento y antes de responder `201` o `202`. Si la cola rechaza el evento (llena o cerrada), el save MUST responder `503 ingest_queue_full` o `500` y MUST borrar la fila (`outbox_forget`, `outbox.rs:450`): un save rechazado no queda guardado. Un evento sin fila (`IngestRequest::outbox == None`) sólo existe en los caminos en proceso de los tests y de `EvalIngest` (`crates/clawg-core/src/eval_api.rs:482`), y se procesa como antes del outbox.

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

### El tipo de memoria se valida al guardar

Los tipos de memoria están en un solo lugar, `MemoryType::ALL` (`crates/clawg-core/src/memory/store_insert.rs:34`): `semantic_fact`, `user_preference`, `architectural_decision`, `session_summary`, `code_block`, `chat_message` y `tool_error`, en ese orden. La lista MUST ser exactamente lo que acepta `MemoryType::from_db_str` (`store_insert.rs:83`), que lee lo ya guardado y da `None` ante un valor desconocido. Un nombre que da quien llama se lee con `FromStr` (`store_insert.rs:104-110`), y uno desconocido es `UnknownMemoryType` (`store_insert.rs:97-100`), cuyo mensaje MUST nombrar el valor recibido y los siete tipos en ese orden:

```
unknown memory_type "preference"; expected one of semantic_fact, user_preference, architectural_decision, session_summary, code_block, chat_message, tool_error
```

`POST /v1/memories` con un `memory_type` que no es uno de los siete MUST responder `400` con ese mensaje (`save_memory`, `crates/clawg-cli/src/daemon/api.rs:1285-1287`), después de comprobar el contenido y antes de `check_operation_id` y del outbox: MUST NOT escribir la fila del outbox ni encolar el evento. Se valida en el handler y no en el worker porque el worker corre cuando quien guardó ya recibió su `201` o `202`, y un rechazo ahí no le llegaría. Un `memory_type` ausente MUST guardarse como `semantic_fact`, y uno válido como ese tipo. `PATCH /v1/memories/{kind}/{id}` rechaza con el mismo mensaje, porque `PatchError::UnknownType` envuelve el mismo error (`crates/clawg-core/src/memory/admin.rs:147`; [`specs/update`](../update/spec.md)): las dos rutas aceptan y rechazan las mismas cadenas.

Una fila del outbox con un tipo desconocido, escrita antes de esta validación, MUST seguir guardándose como `semantic_fact` al reproducirse (`store_insert.rs:187-192`).

#### Escenario: un tipo mal escrito
- GIVEN un daemon sin worker
- WHEN se guarda `{"content": "…", "agent_id": "a", "memory_type": "preference"}`
- THEN responde `400`, el mensaje nombra `"preference"` y los siete tipos
- AND el outbox queda vacío

#### Escenario: las dos puertas
- GIVEN las cadenas `preference`, `Semantic_Fact`, `""` y cada uno de los siete tipos
- WHEN cada una se manda a `POST /v1/memories` y a `PATCH /v1/memories/memory/{id}`
- THEN las dos rutas aceptan los siete tipos y rechazan las otras tres con `400`

**Tests:** `every_listed_type_parses_back_to_itself` y `an_unknown_type_names_the_accepted_ones` (`store_insert.rs`, mod tests); `a_save_with_an_unknown_memory_type_is_refused_before_the_outbox` y `save_and_patch_accept_the_same_memory_types` (`crates/clawg-cli/tests/api.rs`); la reproducción, `explicit_source_with_an_unknown_type_override_falls_back_to_semantic_fact` (`store_insert.rs`, mod tests).

### La clave de tema se valida y vuelve en la respuesta

`POST /v1/memories` MUST aceptar `topic_key`, opcional (`SaveRequest.topic_key`, `crates/clawg-cli/src/daemon/api.rs:386`). Una clave que no cumple el formato de «Definiciones» MUST responder `400` con el mensaje de `validate_topic_key`, que empieza con `topic_key`, después de comprobar el contenido y antes de escribir la fila del outbox ni encolar el evento (`save_memory`, `api.rs:1289-1290`). Se valida en el handler y no en el worker por la misma razón que el tipo: un rechazo del worker llegaría cuando quien guardó ya recibió su `201` o `202`. `validate_topic_key` es pura, y su error (`TopicKeyError`, `topic.rs:23`) usa `thiserror`.

Todo `201` y `202` de un save con clave MUST llevarla en `topic_key` (`SaveBody.topic_key`, `api.rs:591`); sin clave, el campo no va. Un daemon anterior ignora el campo del pedido (`SaveRequest` no usa `deny_unknown_fields`) y no lo devuelve: así el puente se entera de que su clave no se aplicó.

`MemoryEvent.topic_key` MUST ser `#[serde(default)]` (`crates/clawg-core/src/memory/events.rs:48-49`): un evento de la outbox escrito antes del campo se lee como un save sin clave. Sólo un save explícito (`MemorySource::Explicit`) lleva la clave a sus filas (`MemoryEvent::topic`, `events.rs:103`); un turno de chat o una compactación la ignoran.

#### Escenario: una clave mal escrita
- GIVEN un daemon
- WHEN se guarda `{"content": "…", "agent_id": "a", "topic_key": "Storage Engine"}`
- THEN responde `400` y el mensaje nombra `topic_key`
- AND el outbox queda vacío

#### Escenario: la clave vuelve
- GIVEN un daemon con el embedder listo
- WHEN se guarda un texto con `topic_key: "storage/engine"`
- THEN la respuesta es `201` con `topic_key: "storage/engine"`, y un save sin clave responde sin el campo

**Tests:** `topic_key_cases` y `the_error_names_the_field` (`topic.rs`, mod tests); `a_save_refuses_an_invalid_topic_key` y `a_save_echoes_its_topic_key` (`crates/clawg-cli/tests/api.rs`); `an_event_without_a_topic_key_still_parses` (`events.rs`, mod tests).

### Un save con clave reemplaza la fila viva de su clave

Un save explícito con clave, cuyas piezas se guardaron todas, MUST hacer cuatro cosas en la transacción que lo termina: la que enlaza sus piezas y, con una fila en el outbox, también la borra (`outbox_finish`, `crates/clawg-core/src/memory/outbox.rs:459`; sin fila, `MemoryStore::finish_save`, `topic.rs:174`; los dos llaman a `revise_in`, `topic.rs:111`):

1. dejar `superseded`, con `superseded_by` igual a la primera fila del save, cada fila viva del alcance de la clave que no es una de las filas del save. Una pieza que otro save necesita queda viva y sin clave ([`specs/pieces`](../pieces/spec.md), R10);
2. liberar a las filas que disputaban a las reemplazadas (`release_challengers`, [`specs/disputes`](../disputes/spec.md), R4);
3. dar la clave a las filas del save que no tenían ninguna, de modo que un save que el dedup absorbió en una fila sin clave deja la clave en esa fila; una fila con otra clave la conserva;
4. llevar a las filas que el save insertó el menor tier y el mayor `access_count` entre ellas y las reemplazadas.

Las filas reemplazadas conservan contenido, vector y texto: la historia de un tema es la cadena de sus filas `superseded`. Salen de la búsqueda por vectores y del índice de texto, como toda fila reemplazada.

Cada fila que una revisión reemplaza MUST quedar con `supersede_cause = 'revision'`, en el mismo `UPDATE` que pone `superseded_by` (`SUPERSEDE_SQL`, `crates/clawg-core/src/memory/dispute.rs:272`, llamada con `"revision"` desde `revise_in`, `topic.rs:131`). Un veredicto escribe `verdict` ([`specs/disputes`](../disputes/spec.md), R1). Esa causa es la que decide, al borrar la fila que reemplazó, si la anterior vuelve ([`specs/restore`](../restore/spec.md), «Borrar la vigente revive a la anterior»). Una fila que vuelve a reemplazarse por una revisión, después de revivir, pierde su `revived_from`.

**Test:** `a_revision_records_why_it_replaced` en `crates/clawg-core/src/memory/worker.rs`.

Se hace al terminar el save y no con la primera pieza porque un save que falla en la tercera dejaría la versión vieja reemplazada y la nueva a medias. Por eso un save que falla antes de terminar MUST NOT reemplazar nada. Un corte antes de terminar deja la fila del outbox con todas las piezas anotadas: el arranque siguiente no inserta nada y termina el save, reemplazando entonces.

La respuesta MUST listar las filas reemplazadas en `replaced`, cada una con su id y su primera línea (`ReplacedMemory`, `api.rs:618`; `SaveBody.replaced`, `api.rs:582`; `IngestReport.replaced`, `worker.rs:90`). Sin reemplazos, el campo no va. Un save con clave y sin filas anteriores MUST comportarse como uno sin clave, salvo que sus filas llevan la clave. Un save sin clave MUST NOT reemplazar nada.

**Tipo e importancia.** Si el save con clave no trae `memory_type` o `importance`, las filas nuevas MUST tomar los de la fila viva más antigua del alcance (`inherit`, `topic.rs:154`; el worker la lee con `topic_rows`, `topic.rs:164`, antes de guardar nada, `worker.rs:692-708`). En una reposición se lee otra vez y da lo mismo, porque la versión vieja sigue viva hasta el final.

**Alcance y orden.** Un save con clave MUST reemplazar a todo titular vivo de la clave en su proyecto que alcanza, también el de otro agente, y MUST NOT reemplazar ni heredar nada de otro proyecto ni de un autor que `sharing.json` le oculta. Dos agentes que se ocultan entre sí pueden quedar con un titular cada uno, que el doctor marca ([`specs/doctor`](../doctor/spec.md)). Dos saves con la misma clave en cola MUST aplicarse en orden, y la fila viva final es la del segundo: el worker es el único escritor de saves. No hay un índice único sobre la clave porque las piezas de un save la comparten (`idx_memory_topic` es parcial y no único, [`specs/storage`](../storage/spec.md)); la unicidad de lo vivo la mantienen el worker y `restore` ([`specs/disputes`](../disputes/spec.md), R4).

#### Escenario: una decisión revisada
- GIVEN una `architectural_decision` guardada con la clave `storage/engine`
- WHEN ese agente, u otro, guarda otro texto con esa clave en el mismo proyecto
- THEN hay una sola fila viva con la clave, la nueva
- AND la vieja queda `superseded` por la nueva, y ni la búsqueda por vectores ni el índice de texto la devuelven
- AND la respuesta del save lleva la vieja en `replaced`

#### Escenario: el mismo texto
- GIVEN una fila viva con la clave K
- WHEN se guarda el mismo texto con K
- THEN la fila se refuerza y no se reemplaza nada

#### Escenario: volver al texto anterior
- GIVEN la versión 1 de K reemplazada por la versión 2
- WHEN se guarda otra vez el texto de la versión 1 con K
- THEN se inserta una fila nueva que reemplaza a la versión 2, y la versión 1 sigue `superseded`

**Tests** (todos en `worker.rs`, mod tests): `a_keyed_save_replaces_the_live_row_of_its_key`, `the_same_text_under_its_key_only_reinforces`, `saving_an_earlier_text_again_is_another_revision`, `a_revision_inherits_tier_and_accesses`, `a_revision_releases_the_rows_disputing_the_old_one`, `a_failed_revision_replaces_nothing`, `a_reinforced_row_without_a_key_takes_it`, `a_save_without_a_key_replaces_nothing`, `a_revision_is_finished_with_its_outbox_row`, `a_revision_keeps_the_type_and_importance_it_does_not_name`, `a_key_is_scoped_to_its_project`, `a_keyed_save_replaces_every_holder_of_its_project`, `a_key_leaves_a_hidden_authors_holder` y `two_queued_revisions_apply_in_order`.

### Un save confirmado devuelve los ids

Con el embedder `ready`, `POST /v1/memories` MUST encolar el evento con un canal de respuesta, esperar el reporte del worker y responder `201` con un elemento por chunk, en el orden de los chunks (`save_memory`, `api.rs:1278`; `save_response`, `api.rs:1183`). Todo `201` lleva además el `operation_id` (ver «`operation_id`»):

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}], "operation_id": "3f0c…"}
```

`kind` es siempre `"memory"`, de modo que `kind` e `id` son exactamente lo que toma `DELETE /v1/memories/{kind}/{id}`. Las copias repetidas dentro del mismo save MUST NOT generar elementos extra: cuentan como refuerzo de la fila de su primera copia (`collapse_duplicate_chunks`, `worker.rs:166`, y `ingest_pieces`, `worker.rs:735`).

Si la ingesta enmascaró secretos, la respuesta lleva además `redacted` con sus tipos, y `save_memory` los nombra en su texto: ver [`specs/secrets`](../secrets/spec.md), R3.

Si el detector de contradicciones encontró memorias cercanas, la respuesta lleva además `related`, una por memoria vieja, y `save_memory` las nombra para que el agente las juzgue: ver [`specs/disputes`](../disputes/spec.md), «Las candidatas en la respuesta del save». Sin candidatas, el campo no va.

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

Un save cuyo contenido ya está vivo en su proyecto MUST responder `201` con `outcome: "reinforced"` y el id de la fila existente, la más vieja, sea de quien sea entre los autores que el que guarda alcanza (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs`, sobre `idx_memory_dedup` de V20). MUST NOT insertar otra fila. Una fila que `sharing.json` le oculta MUST NOT reforzarse: el save inserta la suya. Vale también para un gemelo que vuelve a estar vivo mientras el save se embebe, como un `restore_memory` del mismo texto: el insert lo busca de nuevo en su transacción y lo refuerza (`insert_unless_twin`, `crates/clawg-core/src/memory/store_insert.rs:443`).

#### Escenario
- GIVEN un save de `X` que devolvió el id 57
- WHEN se guarda `X` otra vez en el mismo proyecto, con el mismo agente o con otro
- THEN la respuesta tiene un elemento `{"id": 57, "outcome": "reinforced"}` y la tabla sigue con una fila

**Tests:** `saving_the_same_text_twice_returns_the_same_id_as_reinforced` (`tests/api.rs`); `process_event_reports_reinforced_with_the_existing_id`, `a_twin_restored_while_the_save_is_embedded_is_reinforced_not_copied` y `the_same_text_from_another_agent_reinforces_the_row_already_there` (`worker.rs`, mod tests); `reinforce_if_duplicate_reinforces_another_agents_row` y `reinforce_if_duplicate_leaves_a_hidden_authors_row` (`store_insert.rs`).

### Un fallo del worker llega al que guardó

Si el worker no puede guardar el evento (embedder caído, batch fallido, menos vectores que chunks, insert fallido), el reporte llega al que espera y el save MUST responder con la causa (`save_response`, `api.rs:1183`):

- un fallo pasajero (`EmbedError::NotReady` o `WorkerGone`, en `ingest_pieces`): `202` con `status: "queued"`, `reason: "retrying"`, la causa en `detail` y el `operation_id`. El worker conserva la fila y la toma otra vez en la misma ejecución ([`specs/storage`](../storage/spec.md), «Un fallo pasajero se reintenta en la misma ejecución»);
- cualquier otro fallo sin chunks guardados: `500` con `error: "ingest_failed"` y un `detail` legible (`embed failed: …`, `embedder returned N vectors for M chunks`, `store insert failed: …`);
- cualquier otro fallo con chunks guardados: `202` con `status: "queued"`, `reason: "partially_saved"`, en `memories` los chunks ya guardados, la causa en `detail` y el `operation_id`. Su fila sigue `pending` con el progreso, y el próximo arranque termina el save después de lo guardado, sin duplicarlo ni reforzarlo, o la fila llega a `failed` y se reintenta o descarta ([`specs/storage`](../storage/spec.md)).

El worker MUST cortar el evento en el primer fallo: lo que queda guardado es siempre un prefijo del texto, nunca uno con un hueco en el medio. `memories` MUST incluir los chunks que ya estaban en la base antes del fallo, sean insertados o reforzados, en orden. Los chunks que no se escribieron MUST NOT aparecer. El worker además registra el fallo con `log::warn!`.

El handler MUST borrar la fila del outbox sólo cuando contesta `500` con el reporte del worker (`outbox_forget`, `api.rs:1397`): el que guardó se entera por la respuesta y no se guardó nada. Con chunks guardados la fila es lo único que permite terminar el save, y con un fallo pasajero el worker la sigue usando. Lo hace el handler y no el worker porque sólo el handler sabe si el reporte llegó antes de vencer la espera. Un save cuyo fallo ocurre después de haber contestado (`202`) no tiene a quién contestárselo: su fila queda con la causa y se ve en `/health` (ver «Estado del outbox en `/health`» y [`specs/storage`](../storage/spec.md), «Intentos y fallo a la vista»).

#### Escenario: embed fallido
- GIVEN un embedder de prueba cuyo `embed_batch` devuelve error
- WHEN se guarda un texto
- THEN la respuesta es `500 ingest_failed`, `memories` está vacío, la tabla no tiene filas y el outbox está vacío

#### Escenario: insert parcial
- GIVEN un save de varios chunks donde el insert de uno de los últimos falla
- WHEN se guarda con el embedder listo
- THEN la respuesta es `202 partially_saved` con los ids de los anteriores, y el outbox tiene la fila con esos chunks en su progreso

#### Escenario: el embedder no pudo tomarlo
- GIVEN un worker que contesta que el embedder no pudo tomar el save
- WHEN se guarda un texto
- THEN la respuesta es `202` con `reason: "retrying"` y el outbox tiene la fila

#### Escenario: corte en el primer fallo
- GIVEN un save de 3 chunks donde el insert del segundo falla
- THEN el tercero no se intenta y la tabla tiene sólo la fila del primero

**Tests:** `a_failed_embed_reaches_the_caller_as_an_error`, `a_failure_the_caller_was_told_leaves_no_outbox_row`, `a_partial_save_answers_202_and_keeps_its_row` y `a_save_the_model_cannot_take_answers_202_retrying_and_keeps_its_row` (`tests/api.rs`); `process_event_reports_the_rows_written_before_an_insert_failed`, `process_event_stops_at_the_first_failed_insert`, `a_failed_embed_batch_reports_the_failure_and_no_rows` y `an_unavailable_embedder_reports_embedder_unavailable` (`worker.rs`, mod tests); y `save_response_covers_every_worker_answer` (`api.rs`, mod tests).

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

El worker MUST retener esos eventos hasta que el modelo cargue (`hold_for_embedder`, `worker.rs:482`). Un apagado ordenado que llega antes los deja en el outbox (ver [storage](../storage/spec.md)). Como la fila ya está confirmada, un `202 embedder_loading` sobrevive a un proceso que muere antes de que el modelo cargue.

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

`POST /v1/search` MUST responder el mismo cuerpo cuando el embedder está `failed`, y `embedder_not_ready` en los demás casos en que la celda está vacía (`ready_embedder`, `api.rs:1414`, que llama `search`). `POST /v1/context` no responde ninguno de los dos: sin embedder cae a la recuperación léxica.

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

**Tests:** `a_save_into_a_full_queue_is_503_queue_full` (`tests/api.rs`) y `the_production_ingest_queue_holds_1024_saves` (`crates/clawg-core/src/runtime.rs`, mod tests). La capacidad se inyecta con `MemoryRuntime::new_for_test_with_capacity` (`runtime.rs:379`). Las filas las cubre `a_save_into_a_full_queue_leaves_no_outbox_row` (`tests/api.rs`).

### `operation_id`

Toda respuesta `201` y `202` de `POST /v1/memories` MUST llevar `operation_id`. Sin uno en el pedido, el daemon MUST emitir un UUID v4. Con uno (`SaveRequest.operation_id`, `api.rs:382`), MUST validarlo (1 a 128 caracteres ASCII visibles, `MAX_OPERATION_ID_LEN`; si no, `400`; `check_operation_id`, `api.rs:664`) y usarlo. `SaveRequest` no usa `deny_unknown_fields`, así que el campo es aditivo.

Mientras exista una fila con ese `operation_id` para ese agente, un save con el mismo MUST NOT crear otra ni encolar nada (`Accepted::Existing`, `runtime.rs:76`): con la fila `pending` responde `202` con `reason: "already_queued"`; con la fila `failed`, `500 ingest_failed` con su causa en `detail`. Después de completado el save, la fila ya no existe: un reenvío vuelve a guardar y el dedup lo absorbe como `reinforced`. El `operation_id` identifica el save, no su texto: un reenvío con otro contenido y el mismo id no guarda el contenido nuevo mientras la fila exista.

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

### El contenido: vacío o sobre el tope

`check_content` (`crates/clawg-cli/src/daemon/api.rs:645`), una función pura, es lo primero que `save_memory` comprueba (`api.rs:1282`): antes del tipo, la clave, el `operation_id` y `derived_from`, antes de consultar el estado del embedder y antes de encolar o escribir una fila.

- Un `content` vacío o de sólo espacios MUST responder `400` con `content must not be empty`.
- Un `content` de más de `MAX_CONTENT_BYTES` (65 536) bytes en UTF-8, contados como llegan y antes de enmascarar (`api.rs:640`), MUST responder `400` con el tamaño recibido, el tope y qué hacer:

```
content is 70000 bytes; at most 65536 are accepted: split it into several saves, one fact each
```

Uno de exactamente `MAX_CONTENT_BYTES` bytes MUST aceptarse como cualquier otro. Se rechaza y no se trunca, porque un save truncado guardaría un texto que quien guardó no escribió, ni se parte en el handler, porque un save es un `operation_id`, una `topic_key` y un vínculo de pedazos.

El tope existe porque cada pedazo de un save largo reescribe su fila del outbox con todo el progreso anterior, y lo que un save escribe crece con el cuadrado de su largo, en la única conexión de escritura y el único worker. Medido con el embedder falso: 64 KiB son 228 pedazos y 8,3 MB de `progress`; 256 KiB, 910 pedazos y 133 MB ([verify](../../changes/archive/2026-10-20-save-bounds/verify.md#medición-del-progreso)). El tope deja pasar la nota más larga de la historia que trae history-import (45 372 caracteres). Lo acota el daemon, no el puente: cualquier cliente con el token pasa por el handler, y su frase llega al modelo como `not saved: …` («Texto que ve el modelo»). La descripción de `content` en `save_memory` MUST nombrar el tope (64 KiB), y `protocol.md` lo dice en «How to write a save».

Una fila del outbox escrita antes del tope, de cualquier tamaño, MUST seguir guardándose al reproducirse: el tope está en el handler y no en el worker.

#### Escenario: sobre el tope
- GIVEN un daemon sin worker
- WHEN se guarda un `content` de 65 537 bytes
- THEN responde `400`, el mensaje nombra `65537` y `65536`
- AND el outbox queda vacío

#### Escenario: en el tope
- GIVEN un daemon con el embedder listo
- WHEN se guarda un `content` de 65 536 bytes
- THEN el save se acepta y sus pedazos quedan guardados

#### Escenario: bytes, no caracteres
- GIVEN un texto de 32 769 `ñ` (65 538 bytes)
- WHEN `check_content` lo evalúa
- THEN lo rechaza; uno de 32 768 `ñ` (65 536 bytes) pasa

**Tests:** `saving_empty_content_is_rejected_before_it_reaches_the_queue`, `a_save_over_the_content_limit_is_refused_before_the_outbox` y `a_save_at_the_content_limit_is_stored` (`tests/api.rs`); `content_is_checked_in_bytes` (`api.rs`, mod tests); `save_memory_says_a_save_over_the_limit_was_not_saved` (`server.rs`, mod tests); la descripción, `the_agent_contract_matches_its_file`.

## La sesión del host

### `POST /v1/sessions` registra la sesión

`POST /v1/sessions`, detrás del token, con `{agent_id, project_id?, session_id}` (`SessionRequest`, `crates/clawg-cli/src/daemon/api.rs:390-395`; `register_session`, `api.rs:1576`), MUST recordar `session_id` para el par `(agent_id, project_id)` y responder `204`. Un registro nuevo del mismo par MUST reemplazar al anterior, y otro agente u otro proyecto MUST NOT verse afectados (`SessionRegistry`, `crates/clawg-cli/src/daemon/sessions.rs:33`). Un `session_id` vacío, de más de 128 caracteres o con algo fuera del ASCII visible MUST ser un `400` (`check_session_id`, `sessions.rs:22`; `MAX_SESSION_ID_LEN`, `:13`). El registro vive en memoria, en `AppCtx` (`api.rs:98`): MUST NOT escribirse en el store y se pierde al reiniciar el daemon, porque una sesión vieja después de un reinicio sería peor que ninguna. No vence: con los hooks instalados se renueva en cada mensaje del usuario ([`specs/hooks`](../hooks/spec.md), R6).

El par lleva el `project_id` tal como lo resuelven el hook y el puente desde el directorio de trabajo (`ProjectIdentity::resolve`), así que el registro y los saves del puente coinciden sin que el daemon resuelva nada. Con dos sesiones del mismo agente en el mismo proyecto gana la última registrada, que es la que recibió el último mensaje; la ambigüedad que queda, dos sesiones trabajando a la vez sin mensajes nuevos, sólo afecta la atribución y nunca el contenido.

#### Escenario: dos sesiones seguidas
- GIVEN el registro `s1` para (`a`, `p`) y luego `s2` para el mismo par
- WHEN `a` guarda en `p`
- THEN la fila lleva `s2`; un save de `b` en `p` o de `a` en `q` no lleva ninguna

**Tests:** `a_session_registry_keeps_the_latest_per_agent_and_project` y `a_session_id_is_checked` (`daemon/sessions.rs`, mod tests); `a_save_carries_the_registered_session` (`crates/clawg-cli/tests/api.rs`).

### El save sella la sesión

Al aceptar un save, `save_memory` MUST leer el registro de su `(agent_id, project_id)` y ponerlo en `MemoryEvent.session_id` antes de `MemoryRuntime::accept`, o nada si no hay registro (`api.rs:1337`, el campo en `:1355`). Se lee en el handler y no en el worker: una reanudación después de un kill corre en un daemon nuevo, con el registro vacío o con otra sesión, y el outbox tiene que guardar la que valía al aceptar. Cada fila que el save inserta, todos sus pedazos, MUST llevar ese id en `source_session_id` (`NewMemoryRecord.source_session_id`, `crates/clawg-core/src/memory/store_insert.rs:153`, copiado del evento en `:234` y escrito por el `INSERT` en `:557` y `:589`). Un save que refuerza una fila existente (mismo texto) no cambia su sesión: la fila sigue siendo la del primer save, porque sólo el `INSERT` escribe la columna.

`MemoryEvent.session_id` MUST ser `#[serde(default)]` (`crates/clawg-core/src/memory/events.rs:59-60`): un evento de la outbox escrito antes del campo se guarda sin sesión. Las filas sin sesión, las de antes y las de un daemon sin registro, MUST leerse, buscarse y mostrarse como siempre; `clawg memories show` imprime `session: -` (`crates/clawg-cli/src/memories.rs:226`). `save_memory` MUST NOT tener un argumento de sesión: el modelo no la conoce, podría inventarla, y la herramienta se describe en cada sesión.

Con Claude Code sobre el perfil dev, cada save llevó el `session_id` que el host mandó a los hooks de su sesión, también el de una sesión que empezó sin daemon: el registro de `UserPromptSubmit` llegó con el daemon que el puente acababa de arrancar ([verify](../../changes/archive/2026-10-19-session-handoff/verify.md#claude-code-sobre-el-perfil-dev-2026-10-19)).

#### Escenario: sin hooks
- GIVEN un daemon sin registro
- WHEN llega un save
- THEN la fila tiene `source_session_id` `NULL` y se lee, busca y muestra como antes

**Tests:** `an_event_without_a_session_reads_as_none` (`events.rs`, mod tests); `a_save_writes_its_session_on_every_row` (`worker.rs`, mod tests, con todos los pedazos de un save largo y un save sin sesión); `a_save_carries_the_registered_session`, `a_queued_save_keeps_its_session_in_the_outbox` y `rows_without_a_session_stay_reachable` (`crates/clawg-cli/tests/api.rs`); `the_agent_contract_matches_its_file`, que pasa sin bendecir: `save_memory` no ganó argumentos.

## Estado del embedder y bootstrap

### Estado del outbox en `/health`

`/health` MUST publicar `ingest` (`HealthBody.ingest`, `api.rs:557`; `outbox_summary`, `outbox.rs:521`) con `pending` y `failed` (cantidad de filas de cada estado), `oldest_pending_s` (edad en segundos de la fila `pending` más vieja, `null` sin ninguna) y hasta 5 `problems` (`PROBLEMS_SHOWN`, `outbox.rs:29`): las filas con `last_error`, las modificadas más recientemente primero, cada una con `operation_id`, `agent_id`, `state`, `attempts`, `error` y `pieces`, los ids de las memorias que ese save insertó según su progreso (omitido si no hay; es lo que un descarte borraría). MUST NOT llevar el texto de ninguna fila.

La lectura MUST NOT hacer esperar a `/health` más de 250 ms (`HEALTH_INGEST_WAIT`, `api.rs:1141`): `/health` lo consulta `probe_running` antes de cada autoarranque con un tope de 1 s (`HEALTH_TIMEOUT`, `client.rs:50`), y la conexión del store es una sola, así que una escritura larga haría esperar a `/health`. Si la lectura vence o falla, el campo `ingest` MUST omitirse de la respuesta (no se publica como `null`). El cliente lo lee como `Option` con `#[serde(default)]` (`Health.ingest`, `client.rs:145`): un daemon sin outbox, o uno ocupado, no lo trae y el cliente no inventa datos. `memory_status` lo muestra porque imprime el `Health` entero ([`specs/mcp`](../mcp/spec.md)).

#### Escenario: un save fallido
- GIVEN una fila `failed` con `operation_id` `op-1`, `attempts = 3` y una causa
- THEN `/health` tiene `ingest.pending = 0`, `ingest.failed = 1` y `ingest.problems[0]` con `operation_id: "op-1"`, `agent_id` y la causa, sin ningún campo `content`

#### Escenario: un daemon anterior
- GIVEN un `/health` sin `ingest`
- THEN el cliente lo parsea y `ingest` queda vacío

**Tests:** `health_reports_a_failed_save_with_its_operation_id` y `discarding_a_failed_save_deletes_the_pieces_it_inserted` (`tests/api.rs`); `the_summary_counts_rows_and_lists_the_latest_causes` (`outbox.rs`, mod tests); `a_health_without_ingest_still_parses` (`client.rs`, mod tests). El tope de 250 ms y la omisión del campo cuando vence no tienen test.

### Estado en `/health` y `clawg status`

`/health` MUST publicar `embedder` con uno de `loading`, `ready` o `failed` (`health`, `api.rs:1143`; `HealthBody`, `api.rs:532`). Con `failed` MUST agregar `embedder_error` (texto) y `embedder_retryable` (booleano). Con los otros dos estados esos campos son `null`.

`clawg status` serializa el `Health` del cliente (`crates/clawg-cli/src/client.rs:122`), que tiene los mismos campos. `embedder_error` y `embedder_retryable` son `#[serde(default)]`: un cliente nuevo contra un daemon que no los publica sigue parseando.

Cuando un archivo del modelo agota sus intentos, el error MUST nombrar el archivo, el host del origen, la cantidad de intentos y la causa del último: el estado HTTP (`HTTP 503 Service Unavailable`) o el error de red con su cadena de causas, que es donde están `operation timed out`, `connection refused` o el error de TLS (`DownloadError::RetryExhausted`, `crates/clawg-core/src/memory/model_download.rs:357`; `describe`, `:862`). MUST NOT contener la URL entera ni las credenciales que traiga: del origen va el host, con su puerto si lo tiene (`host_of`, `:847`). Cada intento fallido MUST dejar un `warn!` con el archivo, el host, el número de intento y su causa (`download_one`, `:682` y `:825`). El texto llega sin cambios a `embedder_error` y al `503 embedder_failed` de un save («Embedder fallido»).

#### Escenario: descarga fallida
- GIVEN un bootstrap cuya descarga falló con `connection refused`
- THEN `/health` tiene `embedder: "failed"`, `embedder_error` contiene `connection refused` y `embedder_retryable: true`

#### Escenario: un 503
- GIVEN un origen que responde `503` a todo
- WHEN `ensure` agota los intentos
- THEN el error contiene `503` y el host, y no la ruta del archivo en el origen

#### Escenario: contrato distinto
- GIVEN un bootstrap cuyo contrato no coincide
- THEN `embedder: "failed"` y `embedder_retryable: false`

**Tests:** `health_reports_failed_with_the_cause_after_a_failed_bootstrap` (`tests/api.rs`) y `a_failed_embedder_shows_in_the_status_report` (`crates/clawg-cli/src/status.rs`); `an_exhausted_download_names_the_last_status_and_the_host` y `the_cause_never_carries_the_credentials_of_the_url` (`crates/clawg-core/src/memory/model_download.rs`).

### Reintento del bootstrap

Un bootstrap que falla por descarga, carga o verificación del contrato (error al leer la base) MUST reintentarse. Un contrato que no coincide MUST NOT reintentarse (`classify`, `bootstrap.rs:70`). Un `CLAWG_HF_BASE_URL` inválido es un error de descarga y se reintenta igual: el bootstrap no distingue configuración de red, y mientras tanto `embedder_error` dice qué corregir ([`specs/storage`](../storage/spec.md), «El origen: `CLAWG_HF_BASE_URL`»).

La espera tras el intento fallido `n` (1, 2, …) MUST ser `min(30 s · 2^(n-1), 30 min)`, con saturación para no desbordar (`retry_delay`, `bootstrap.rs:88`). No hay tope de intentos. Durante la espera el estado es `failed` con `retryable: true`; durante un intento es `loading`. El bucle es `run_embedder_bootstrap_with_retry` (`bootstrap.rs:690`), que el daemon lanza con `tokio::time::sleep` (`crates/clawg-cli/src/daemon/serve.rs:132`); `classify` y `retry_delay` son funciones puras y la espera se inyecta, así que los tests corren sin dormir.

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

`save_memory` MUST traducir cada respuesta del daemon a un texto para el modelo (`save_outcome_text`, `crates/clawg-cli/src/mcp/server.rs:688`). La función es pura:

| Respuesta | Resultado MCP | Texto |
|---|---|---|
| `201`, un elemento `inserted` | éxito | `saved #57` |
| `201`, varios elementos | éxito | `saved as N pieces: #57, #58, #59; read_memory with any of them returns the whole text` |
| `201`, un elemento `reinforced` | éxito | `already stored as #57; reinforced it instead of saving a copy (update_memory changes its importance or type)` |
| `201` con `replaced` | éxito | el texto del save más `; replaces #12 "<primera línea>"`, una por fila reemplazada, separadas por coma |
| `201`, se mandó una `topic_key` y la respuesta no la trae | éxito | el texto del save más `; the running clawg daemon predates topic_key, so this replaced nothing: restart it with the installed clawg (`clawg stop`, then save again)` |
| `202 unconfirmed` | éxito | `accepted as operation <id> but not confirmed yet; memory_status lists it if it fails, list_memories shows it once stored` |
| `202 embedder_loading` | éxito | `accepted as operation <id>; it will be stored once the embedding model finishes loading (memory_status lists it if it fails)` |
| `202 already_queued` | éxito | `accepted as operation <id> earlier and still waiting to be stored` |
| `202 retrying` | éxito | `accepted as operation <id>; the embedding model could not take it (<detail>), and clawg keeps retrying until it is stored (memory_status lists it if it fails)` |
| `202 partially_saved` | éxito | `partially saved as #57, #58 (<detail>); the rest waits as operation <id>, which clawg retries when the daemon next starts (memory_status lists it if it fails)` |
| `400` (un `memory_type` desconocido, entre otros) | error | `not saved: <mensaje del daemon>`, por ejemplo `not saved: unknown memory_type "preference"; expected one of semantic_fact, …`, o `not saved: content is 70000 bytes; at most 65536 are accepted: …` |
| `503 embedder_failed` | error | `not saved: the embedding model failed to load (<causa>)`, más `; clawg will retry` si es reintentable |
| `500 ingest_failed` / `503 embedder_unavailable` | error | `not saved: <detalle>`, más `; partially saved as #…` si hubo filas (un daemon anterior contestaba así un fallo pasajero o uno con filas guardadas) |
| `503 ingest_queue_full` | error | `not saved: the ingest queue is full; try again shortly` |

El aviso del daemon anterior sale sólo en un save confirmado (`201`): un `202` no promete ningún reemplazo, y su texto no cambia. Las dos frases de arriba se agregan al texto de `saved`, de `saved as N pieces` y de `already stored`, y van antes de las candidatas de `related` ([`specs/disputes`](../disputes/spec.md), «Las candidatas en la respuesta del save»).

Los textos de un `202` MUST nombrar el `operation_id` (sin él, dicen sólo `accepted`) y decir que un fallo aparece en `memory_status`. MUST NOT contener la palabra `queued` sola. Un save «exitoso» sólo se reporta como `saved` cuando el daemon confirmó las filas. Los ids MUST servir tal cual para `delete_memory` y `update_memory` con `kind: "memory"` (ver [`specs/update`](../update/spec.md), R5). Cualquier otro error pasa por `explain` (`server.rs:597`), que con `embedder_failed` MUST NOT prometer que la búsqueda va a funcionar en un minuto.

#### Escenario: id usable
- GIVEN `save_memory` devolvió `saved #57`
- WHEN se llama `delete_memory` con `kind: "memory"` e `id: 57`
- THEN el borrado tiene éxito

#### Escenario: bootstrap fallido
- GIVEN el embedder `failed`
- THEN `save_memory` devuelve un resultado de error que nombra la causa

**Tests:** `save_outcome_text_covers_every_daemon_answer`, `save_memory_returns_the_id_that_delete_memory_accepts`, `save_memory_is_an_error_when_the_embedder_failed`, `save_memory_says_an_unknown_type_was_not_saved` y `explain_does_not_promise_search_when_the_embedder_failed` (`server.rs`, mod tests); para lo reemplazado, `save_outcome_text_names_the_replaced_memories`, `a_key_sent_to_an_older_daemon_is_reported` y `save_memory_revises_by_topic_key`.

### El cliente HTTP devuelve el resultado parseado

`DaemonClient::save` (`client.rs:981`) MUST devolver `SaveOutcome` (`Saved { memories, redacted, related, replaced, key_dropped }` o `Queued { reason, operation_id, memories, detail }`, `client.rs:569`), no `()`. `key_dropped` vale `true` cuando el pedido llevó una `topic_key` y la respuesta confirmada no la trae (`client.rs:1013`). `operation_id` es `None` con un daemon anterior al outbox. Los errores son `ClientError::Daemon { status, message, details }`, y `details` (`ErrorDetails`, `client.rs:89`) MUST conservar `detail`, `retryable` y `memories` del cuerpo de error para que la herramienta los muestre.

**Test:** `save_returns_the_ids_the_daemon_reported` (`client.rs`, mod tests).

## Troceo del save

### Hay un solo chunker, también para el código

El worker MUST trocear el contenido de cada save con el chunker de chat (`ChatChunker::split_save`, `crates/clawg-core/src/memory/chunker/chat.rs:114`), sea cual sea su `memory_type` y su `source`, incluido `code_block`. Lo único que la elección por evento decide es si el evento se descarta: `select_chunker` (`crates/clawg-core/src/memory/chunker/mod.rs:96`) devuelve `ChunkerKind::Skip` para un contenido vacío y para el rol `System`, y `ChunkerKind::Chat` para el resto (`ingest_pieces`, `worker.rs:765`); un evento de fuente `Compaction` ni consulta la elección y siempre va al chat chunker. No hay chunker de código en la ingesta: el código de un proyecto se trocea para su propio índice, que no es `memory` ([`specs/project-docs`](../project-docs/spec.md)).

Una fila guardada con `source = 'code_file'` o `memory_type = 'code_block'` MUST seguir leyéndose y listándose como cualquier otra (`MemorySource::CodeFile`, `events.rs:75`; `MemoryType::CodeBlock`, `store_insert.rs:25`).

#### Escenario: un save de código
- GIVEN un `save_memory` con `memory_type = "code_block"` y un bloque de Rust de 3 líneas
- WHEN lo procesa el worker
- THEN queda una fila con ese contenido tal cual y ese tipo

**Test:** `a_code_block_save_is_chunked_like_any_other` en `crates/clawg-core/src/memory/worker.rs`.

## Invariantes

- El worker es el único escritor de la ingesta: el handler nunca inserta en `memory`, sólo registra la fila del outbox, encola y espera.
- El dedup, el chunker, la detección de contradicciones y el orden FIFO del worker no dependen de que el que guarda espere: un evento sin canal de respuesta se procesa igual.
