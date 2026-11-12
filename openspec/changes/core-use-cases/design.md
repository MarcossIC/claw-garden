# core-use-cases — Diseño

Los enfoques de fondo están en la [exploración](exploration.md#enfoques-posibles); se toma el **A**, un caso de uso por operación, empezando por el guardado. Acá van sus tres decisiones abiertas (D3, D4 y D6), qué handlers entran (D7) y las decisiones de implementación. Las citas `ruta:línea` son de `develop` al escribir esto.

## D1. Dónde vive cada parte

El guardado tiene dos clases de reglas: las de formato, que sólo miran el cuerpo, y las que leen el store o el embedder. Van a dos lugares:

| Parte | Lugar | Por qué |
|---|---|---|
| `SaveInput`, `NewSave::parse`, `SaveRejected`, `check_operation_id`, `Admitted` y los topes | `crates/clawg-core/src/memory/ingest/save.rs`, reexportado como `clawg_core::memory::save` | Es del guardado, que es la carpeta `ingest`. Lo que nombra (`domain::record::MemoryType`, `revision::topic`, `domain::events`, `store::error`) está en la fila de `ingest` de `MEMORY_FOLDERS` (`crates/clawg-cli/tests/conventions.rs:309`) |
| `MemoryRuntime::admit` | `crates/clawg-core/src/runtime.rs`, junto a `accept` | Necesita `existing_memory_ids` y `replaced_memory_ids` (`upkeep/synthesis.rs`) y `EmbedderStatus` (`upkeep/bootstrap.rs`), y `ingest` no puede nombrar `upkeep`. El runtime está fuera de `memory` y ya tiene el store, el estado del embedder y el directorio de `sharing.json` |

**Descartado:**

- **Mudar `existing_memory_ids` y `replaced_memory_ids` a `ingest`.** Responden sobre conclusiones, y sus tests usan los ayudantes de `synthesis.rs` (`seed_as`, `supersede`). Moverlos no saca ninguna regla del handler.
- **Una carpeta `use_cases`.** Sería una abstracción con un solo caso; `MemoryRuntime::accept` ya es la entrada del guardado en el núcleo.
- **Todo en `runtime.rs`.** Mezcla el parseo puro con un archivo de mil líneas de I/O, y la regla «la lógica pura separada de la I/O» de `openspec/config.yaml` pide lo contrario.

## D2. Dos pasos, y la sesión entre los dos

El orden de hoy es: formato, fuentes, embedder, proyecto vivo, sesión, `accept`. La sesión la da `SessionRegistry` (`crates/clawg-cli/src/daemon/sessions.rs`), que es estado del daemon, y se elige con el proyecto vivo. Por eso el caso de uso se parte en dos llamadas del núcleo, y el daemon pone la sesión en medio:

```rust
let save = NewSave::parse(body.into()).map_err(save_refused)?;
let admitted = app.runtime.admit(save).await.map_err(save_refused)?;
let session_id = app.sessions.current(admitted.agent_id(), admitted.project_id());
let (event, operation_id) = admitted.into_event(session_id, now_ms());
app.runtime.accept(event, operation_id, reply).await
```

`NewSave` y `Admitted` tienen los campos privados al crate: un `Admitted` sólo sale de `admit`, y un `NewSave` sólo de `parse`. `accept` no cambia: lo usan también los tests y otros caminos con un `MemoryEvent` armado a mano.

**Descartado:**

- **Un `admit` que reciba una clausura para la sesión.** Una abstracción para un solo llamador.
- **Elegir la sesión antes de `admit`, con el proyecto del cuerpo.** Un bridge abierto antes de una fusión manda el id viejo, y la sesión se registra con el vivo.
- **Seguir el proyecto en el handler antes de `admit`.** Cambia qué error ve el cliente cuando el store falla y el cuerpo también está mal: hoy es el `400`, y sería el `500`.

## D3. Un error con el texto de hoy y sus datos

**Decisión abierta 1: qué errores llevan un texto listo y cuáles un dato que el handler traduce.** Todos llevan las dos cosas: el `Display` de cada variante es el texto que hoy responde el daemon, y la variante lleva los datos (`bytes`, `count`, los ids, los pares reemplazados, la causa del embedder).

- La API sólo habla inglés. `Lang` (`crates/clawg-core/src/preferences.rs`) elige el idioma de la CLI y del puente, y el puente muestra el mensaje de un `400` tal cual (`not saved: <mensaje>`, `crates/clawg-cli/src/mcp/server.rs:3747`). Traducirlo en el handler cambiaría una respuesta.
- Con los datos en la variante, una traducción futura no tiene que parsear el texto.
- El handler traduce por variante a un estado, y sólo `EmbedderFailed` usa los datos: su cuerpo es `embedder_failed` con la causa en `detail` (`ApiError::embedder_failed`, `api.rs:169`). `Store` es `#[error(transparent)]`, así el `500` lleva el texto del `MemoryError`, como hoy `ApiError::internal`.

El `match` del handler nombra cada variante del `400`, sin comodín: una variante nueva no compila hasta que alguien decida su estado.

## D4. El formato de un save es del dominio

**Decisión abierta 2: si `check_content` y `check_operation_id` son de HTTP o de dominio.** Del dominio, los dos:

- **El tope de contenido** sale del outbox: cada pieza de un save largo reescribe su fila con el progreso, y lo que escribe un save crece con el cuadrado de su largo (`api.rs:692-698`, [`specs/ingest`](../../specs/ingest/spec.md)). Otro cliente que guarde sin HTTP tiene el mismo límite.
- **El `operation_id`** es la identidad de una fila del outbox (`outbox_record`), y las rutas del outbox lo comprueban «como un save» (`requested_operation`, `api.rs:2088-2090`). `check_operation_id` queda público para ellas.

Los tres topes (`MAX_CONTENT_BYTES`, `MAX_OPERATION_ID_LEN`, `MAX_SOURCES`) se mudan a `save.rs`. `crates/clawg-cli/tests/api.rs:16` y `crates/clawg-cli/src/mcp/server.rs:2096` importan `MAX_CONTENT_BYTES` de `clawg_cli::daemon::api`, y esos tests no se editan, así que el daemon lo reexporta con `pub use`. Los otros dos sólo los usan los tests que se mudan.

## D5. El tipo se parsea, y el evento lleva su nombre

`NewSave` guarda `Option<MemoryType>`. `MemoryEvent::memory_type` sigue siendo `Option<String>`:

- `MemoryEvent` se guarda serializado en `ingest_outbox` y se lee al arrancar. Una fila escrita antes de que el save validara el tipo puede llevar un nombre que no es uno de los siete, y hoy el worker la guarda como `semantic_fact` (`NewMemoryRecord::from_event_and_chunk`, `crates/clawg-core/src/memory/ingest/store_insert.rs:82-86`). Con `Option<MemoryType>` esa fila no se podría leer, y el save se perdería. La regla de `openspec/config.yaml` es que ningún cambio pierde memorias.
- `into_event` escribe `MemoryType::as_str`, que es el texto que mandó el cliente: `from_db_str` sólo acepta los siete nombres exactos (`crates/clawg-core/src/memory/domain/record.rs:156`). El evento queda igual byte a byte.

Lo que deja de pasar es el segundo parseo de `api.rs:1482`, que tiraba el error: la métrica de la sesión lee `Admitted::memory_type`.

**Descartado:** cambiar el campo del evento a `MemoryType` con un `Deserialize` que caiga a `None` ante un nombre desconocido. El worker guardaría lo mismo, pero es un cambio del contrato guardado que pide su test sobre filas de la versión anterior, y este cambio no lo necesita. Le toca a domain-types.

## D6. `eval_api` no cambia

**Decisión abierta 3.** `EvalIngest` (`crates/clawg-core/src/eval_api.rs:727`) arma sus eventos y los encola sin `accept` ni outbox. Sus saves los escribe el propio código de evaluación desde un golden, no un cliente: no hay un `derived_from` que validar ni un embedder que pueda estar en falla. Pasarlo por `admit` no cambiaría lo que mide y lo ataría al outbox. Si un golden empieza a guardar conclusiones, el cambio que lo haga usa `admit`.

## D7. Qué handlers entran

| Handler | ¿Entra? | Motivo |
|---|---|---|
| `save_memory` (`api.rs:1420-1552`) | Sí, en este PR | Decide cuatro reglas de dominio y parsea el tipo dos veces |
| `locate_project` (`api.rs:3097-3145`) | Parte 2 | Decide una regla: una carpeta cuyo proyecto tiene otras no se fusiona (`409 folder_shared`, `:3131-3140`). Store-boundary ya bajó la cuenta (`own_folder_count`); falta la decisión, y va con las demás rutas de proyectos |
| `register_root` (`api.rs:2687-2757`) | Parte 2 | Casi todo ya es del núcleo: `register_project` devuelve un `RegisterError` tipado, y el handler lo traduce. Quedan dos reglas de formato (`root_path` vacío, `index_docs` sobre algo que no es un directorio) y efectos del daemon (despertar el embedder, refrescar los docs) |
| `context` (`api.rs:1690-1779`) | No | Es de sólo lectura y no rechaza nada: sus decisiones son cómo se degrada una respuesta (una cuenta que falla es 0, un `regen` que falla no emite ids). La exploración deja fuera las rutas de sólo lectura sin reglas; si storage-read-side le da un caso de uso, es ese cambio |

Bajar también `locate_project` y `register_root` en este PR duplicaría el diff sin que el guardado dependa de ellos, y el pedido es aplicar primero la parte coherente. Las partes siguientes están en [`tasks.md`](tasks.md#partes-siguientes).

## D8. Cómo se comprueba que nada de lo que ve un cliente cambia

1. **Caracterización primero.** `each_refused_save_keeps_its_status_and_body`, en el `mod tests` de `crates/clawg-cli/src/daemon/api.rs`, llama a `save_memory` con un `AppCtx` de prueba (el de `daemon_embedding_with`) y compara el estado y el cuerpo JSON entero de cada rechazo: las siete reglas de R1, las fuentes que faltan, las reemplazadas, el embedder `failed` y un store cerrado. Se escribe y se commitea antes de tocar el handler, sobre el código de `develop`. No va en `crates/clawg-cli/tests/api.rs` porque ese archivo es de los tests de respuestas, que este cambio no edita.
2. **Los tests de respuestas no se tocan.** `git diff develop...HEAD -- crates/clawg-cli/tests/ crates/clawg-cli/src/mcp/` vacío, y los dos contratos del agente pasan sin `CLAWG_BLESS`.
3. **Los tests puros que se mudan** conservan su nombre y sus literales: `content_is_checked_in_bytes`, `sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `replaced_sources_name_what_replaced_them`, `missing_sources_are_the_ones_that_were_not_found` y `operation_ids_are_checked`. Donde hoy comparan `ApiError::status` con `400`, comparan la variante; donde comparan `.message`, el `Display`. Las specs principales los citan por nombre, y al archivar cambia el archivo que se cita.
4. **El diff de `api.rs`** no cambia ningún `StatusCode` ni el texto de ningún `ApiError`.

## D9. Los tests del núcleo, sin HTTP

- **`save.rs`:** tests puros sobre `NewSave::parse` y `check_operation_id`, más el texto de cada variante.
- **`runtime.rs`:** `MemoryRuntime::new_for_test` sobre `crate::test_dir::test_dir()`, con las filas sembradas con SQL como los demás tests del archivo, `sharing.json` escrito en el directorio del runtime, el estado del embedder cambiado con `embedder_status.send`, y un proyecto fusionado como una fila de `project` con `merged_into`. Cada test cierra el runtime (`close`).
- El store que falla es un runtime cerrado: `admit` con `derived_from` da `Store`.
