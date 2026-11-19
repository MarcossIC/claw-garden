# core-use-cases — Diseño

Los enfoques de fondo están en la [exploración](exploration.md#enfoques-posibles); se toma el **A**, un caso de uso por operación, empezando por el guardado. Acá van sus tres decisiones abiertas (D3, D4 y D6), qué handlers entran (D7) y las decisiones de implementación. D10 a D14 son de la segunda parte, las escrituras de proyectos; D15 a D19, de la tercera, las otras escrituras. Las citas `ruta:línea` son de `develop` al escribir cada parte.

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

- **El tope de contenido** sale del outbox: cada pieza de un save largo reescribe su fila con el progreso, y lo que escribe un save crece con el cuadrado de su largo (`api.rs:692-698`, [`specs/ingest`](../../../specs/ingest/spec.md)). Otro cliente que guarde sin HTTP tiene el mismo límite.
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
| `locate_project` (`api.rs:3097-3145`) | Parte 2 ([D12](#d12-ubicar-una-carpeta-en-dos-pasos)) | Decide una regla: una carpeta cuyo proyecto tiene otras no se fusiona (`409 folder_shared`, `:3131-3140`). Store-boundary ya bajó la cuenta (`own_folder_count`); falta la decisión, y va con las demás rutas de proyectos |
| `register_root` (`api.rs:2687-2757`) | Parte 2 ([D10](#d10-dónde-vive-cada-parte-de-los-proyectos)) | Casi todo ya es del núcleo: `register_project` devuelve un `RegisterError` tipado, y el handler lo traduce. Quedan dos reglas de formato (`root_path` vacío, `index_docs` sobre algo que no es un directorio) y efectos del daemon (despertar el embedder, refrescar los docs) |
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

## D10. Dónde vive cada parte de los proyectos

Las citas de D10 a D14 son de `develop` al empezar la segunda parte: `register_root` está en `api.rs:2580-2650`, `delete_projects` en `:2780-2805` y `locate_project` en `:2990-3038`.

| Parte | Lugar | Por qué |
|---|---|---|
| `check_root` y las variantes `RegisterError::EmptyRoot` y `NotADirectory` | `crates/clawg-core/src/memory/projects/project.rs`, junto a `register_project` | Son rechazos de un registro, y el handler ya traduce `RegisterError` (`api.rs:2612-2629`). Con dos variantes más, su `match` sigue sin comodín |
| `check_folder`, `locate_folder`, `Located` y `LocateError` | `crates/clawg-core/src/memory/projects/registry.rs`, junto a `own_folder_count` | Es lo que hace que una carpeta lleve a un proyecto, y `own_folder_count` (`registry.rs:816`), que sólo usa esta decisión, pasa a privada ahí. `registry` ya nombra `merge` (`canonical_in`), así que llamar a `merge_core` no abre ningún ciclo de `MEMORY_CYCLES` |
| `DeleteError::ConfirmWithoutRows` | `crates/clawg-core/src/memory/projects/project_delete.rs`, al entrar a `delete_core` | Es el núcleo el que hoy ignora el código ([D13](#d13-lo-que-deciden-las-demás-rutas-de-proyectos)) |

**Descartado:**

- **Las dos reglas de la raíz dentro de `register_project`.** El daemon lee el checkout de la raíz antes de llamarlo (`api.rs:2597-2608`): una raíz vacía se leería (un `.git` relativo al directorio del daemon) y dejaría su línea de log «is not in the checkout» antes del `400`. `check_root` va antes de esa lectura, en el lugar de hoy.
- **Un módulo `projects/locate.rs`.** Un módulo para una función y su error; `registry.rs` ya tiene la cuenta que la decide.
- **`Ground::is_dir`.** Cuatro implementaciones (`Disk`, `ReadAhead`, `FakeGround`, `Watched`) para una sola lectura que sólo hacen el registro y la ubicación del daemon ([D11](#d11-el-disco-entra-como-una-función)).

## D11. El disco entra como una función

`check_root` y `check_folder` reciben `is_dir: impl FnOnce(&Path) -> bool`; el daemon les pasa `Path::is_dir`. Así la decisión es pura y sus tests no tocan el disco, como pide `openspec/config.yaml`, y `check_root` llama a `is_dir` sólo con `index_docs`, que es el corte de hoy (`index_docs && !root.is_dir()`, `api.rs:2592`).

Los textos siguen la regla de [D3](#d3-un-error-con-el-texto-de-hoy-y-sus-datos): el `Display` de cada variante es el texto que hoy responde el daemon, y la variante lleva la raíz como `PathBuf`, como `RegisterError::NoFreeId`. El texto de hoy pone `root_path` tal como llegó; una `PathBuf` hecha de ese `String` se muestra igual con `display()`.

**Descartado:** un `is_dir: bool` calculado antes. El handler tendría que repetir el corte (`index_docs && root.is_dir()`), o leer el disco también sin `index_docs`.

## D12. Ubicar una carpeta en dos pasos

El orden de hoy es: la carpeta es un directorio, la subcarpeta (con `subfolder`), el nombrado (`find_project`), el registro, y la decisión con su proyecto. El registro es del daemon: `register_root` despierta el embedder y lanza el refresco de los docs (`api.rs:2638-2648`). Por eso, como en [D2](#d2-dos-pasos-y-la-sesión-entre-los-dos), el núcleo entra dos veces y el daemon queda en medio:

```rust
check_folder(&folder, Path::is_dir).map_err(located_refused)?;
// la subcarpeta y el nombrado, como hoy
let registered = register_root(&app, &body.root_path, None, &[], false, true).await?;
let located = locate_folder(store, &workspaces, &backups, &folder, &registered.project_id, &target).await.map_err(located_refused)?;
```

- **`Located` es un `enum`** (`Already`, `Merged(MergeReport)`) y no un par `already`/`merge`: no hay un `already` con una unión. El handler arma `LocatedBody` con el `root_id` del registro, como hoy.
- **`LocateError::Merge` lleva el `MergeError`**, y el handler lo traduce con `merge_error`, como ya hace `SubfolderError::Merge` (`api.rs:3073`).
- **`LocateError::Store` es `#[error(transparent)]` sobre `StorageError`,** el error de `own_folder_count` (SB3), así el `500` lleva el mismo texto que hoy da `ApiError::internal`.
- **`sessions_follow` queda en el handler:** `SessionRegistry` es estado del daemon ([D2](#d2-dos-pasos-y-la-sesión-entre-los-dos)).

**Descartado:**

- **Un `LocateError` para el nombrado que no existe o es ambiguo.** `find_project` ya responde un `Named` tipado, y las dos rutas que lo usan lo traducen distinto: `locate_project` responde `404` a un nombre que nadie tiene, y `target_named` (`api.rs:1883`) lo deja seguir para que el movimiento responda su propio `404`. Es traducción.
- **`check_folder` dentro de `locate_folder`.** `locate_folder` corre después del registro, y una carpeta que no es un directorio con un nombrado que no existe daría el `404` en lugar del `400` de hoy.

## D13. Lo que deciden las demás rutas de proyectos

Cada handler hace una de tres cosas: lee la **forma del cuerpo** (qué campos van juntos y qué operación pide), que es de HTTP y queda; **traduce** una respuesta tipada del núcleo, que queda; o decide una **regla** sobre lo que el núcleo escribe, que baja. Una regla baja cuando otro llamador del núcleo la necesitaría para no escribir algo que nadie pidió.

| Ruta | Qué decide el handler | Clase | Queda o baja |
|---|---|---|---|
| Renombrar (`rename`, `api.rs:2674-2688`) | nada: traduce `RenameError` | traducción | queda |
| Fusionar (`merge_projects`, `:2721-2739`) | `undo` o `source` y `target` (`400 name a source and a target, or a merge to undo`); que las sesiones sigan la unión | forma; estado del daemon | queda |
| Borrar (`delete_projects`, `:2780-2805`) | `undo` o `project_id`, y `dry_run` sólo con `undo` | forma | queda |
| | `confirm` sin `with_memories` (`:2792-2794`) | regla: `delete_core` con `(false, Some(_))` borra el proyecto como vacío e ignora el código (`project_delete.rs:592-593`) | baja ([R10](spec.md#r10-un-código-de-confirmación-va-con-las-filas)) |
| | `NeedsModel` responde con el estado del embedder (`delete_error`, `:2770`); el borrado quita las sesiones del proyecto | traducción; estado del daemon | queda |
| Mover (`move_memories`, `:1912-1928`) | `MoveRequest::plan` (`:985-1011`), puro y con su test `a_move_asks_for_one_target_or_an_undo`; `target_named` traduce `Named` | forma; traducción | queda |
| Subcarpetas (`subfolder_of` y `declare`, `:3043-3089`) | un camino en lo alto de un checkout o fuera de uno no es una subcarpeta: el `400` le dice al cliente que pidió la otra operación de la ruta («`locate` without `subfolder` locates the folder's root»); `declare` traduce `SubfolderError` | forma; traducción | queda |

Al revisar aparecieron dos rutas de proyectos fuera de esta lista con reglas del dominio: `answer_suggestion` (`:2911-2947`: la respuesta es `merge` o `apart`, responder es cosa de la persona y lleva `user_answered` con `reason`, y sólo se responde una sugerencia pendiente) y `resolve_projects` (`:2840-2853`: `new_project` va con `create`). Van con la parte 3 ([`tasks.md`](tasks.md#partes-siguientes)).

**Descartado:** bajar la regla de las subcarpetas a `subfolder.rs`. Lee el disco con `ProjectIdentity::resolve`, que sube por los padres del camino, y un test del núcleo sobre `test_dir()` daría otro resultado si el directorio temporal estuviera dentro de un checkout. Y no protege ninguna escritura: `declare_subfolder` recibe un `subpath` ya medido.

## D14. Cómo se comprueba la segunda parte

Como [D8](#d8-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia):

1. **Caracterización primero.** `each_refused_registration_keeps_its_status_and_body`, `each_located_folder_keeps_its_status_and_body` y `each_refused_project_delete_keeps_its_status_and_body`, en el `mod tests` de `api.rs`, llaman a `register`, `locate_project` y `delete_projects` con el `AppCtx` de `daemon_for_jobs` y comparan el estado y el cuerpo JSON entero: cada rechazo de R8 a R10 y su orden, las traducciones de `RegisterError` que quedan (los dos `detail` de `project_root_collision`), el nombrado que no existe o es ambiguo, una unión rechazada por un nombrado borrado, y los dos cuerpos de una ubicación que se acepta. Se commitean antes de tocar los handlers y pasan sobre el código de `develop`.
2. **Los tests de respuestas no se tocan:** `git diff develop...HEAD -- crates/clawg-cli/tests/ crates/clawg-cli/src/mcp/` vacío, y los dos contratos del agente pasan sin `CLAWG_BLESS`.
3. **El diff de `api.rs`** no cambia ningún `StatusCode` ni el texto de ningún `ApiError`: los textos que se van del handler llegan como `Display` de las variantes.
4. **Los tests del núcleo**, sin HTTP: `check_root` y `check_folder` con un `is_dir` de prueba; `locate_folder` y `delete_core` con `MemoryStore::new_for_test` y las filas sembradas con SQL, como los tests de `registry.rs` y `project_delete.rs`; el store que falla es uno cerrado.

## D15. Lo que deciden las otras escrituras

Las citas de D15 a D19 son de `develop` al empezar la tercera parte: `update_memory` está en `api.rs:1852-1879`, `delete_memory` en `:1931-1946`, `restore_memory` en `:1951-1968`, `resolve_dispute` en `:2013-2023` (con `resolve_error` en `:1999-2011`), `resolve_projects` en `:2837-2850` y `answer_suggestion` en `:2908-2944`.

La clasificación es la de [D13](#d13-lo-que-deciden-las-demás-rutas-de-proyectos): la **forma del cuerpo** y la **traducción** de una respuesta tipada quedan en el handler; una **regla** sobre lo que el núcleo escribe baja, cuando otro llamador del núcleo la necesitaría para no escribir algo que nadie pidió.

| Ruta | Qué decide el handler | Clase | Queda o baja |
|---|---|---|---|
| Corregir (`update_memory`) | `parse_kind` sobre el segmento de la ruta | forma | queda |
| | `project_id` o `no_project`, no los dos (`UpdateRequest::destination`, `400 pass project_id or no_project, not both`), con su test `an_update_names_one_destination_at_most` | forma: dos campos para un `Option<Option<String>>` | queda |
| | `target_named` traduce `Named` antes de armar el cambio, así un nombre ambiguo responde `409 project_ambiguous` aunque el cambio también esté mal | traducción | queda, con su orden |
| | el cambio vacío, la importancia fuera de 0 a 10 y el tipo: los decide `MemoryPatch::new` (`admin.rs:66-81`), que ya parsea `memory_type` una vez, guarda `Option<MemoryType>` y rechaza con `PatchError`; el handler responde `400` con su `Display` | ya en el núcleo; traducción | queda |
| | `UpdateError` a `409`, `move_error` o `500`; `None` del núcleo (la fila no está, está borrada o fuera del alcance) a `404 no memory row with id <id>` | traducción | queda |
| Borrar (`delete_memory`) | `parse_kind`; `deleted: 0` a `404 no memory row with id <id>` | forma; traducción | queda |
| Restaurar (`restore_memory`) | `parse_kind` antes del embedder; `ready_embedder` responde `503` con el estado del embedder | forma; estado del daemon | queda |
| | `RestoreError` a `404`, `409`, `embed_failure` o `500`. El `404` arma a mano el texto que ya es el `Display` de `NotFound`, y un comodín toma `Store` | traducción | queda, con el `Display` de `NotFound` y sin comodín |
| Resolver una disputa (`resolve_dispute`) | `keep_both` elige el `Outcome`; `resolve_error` traduce `ResolveError`. La misma fila, la fila que no está, el par que no está en disputa, la preferencia que sólo se reemplaza con la palabra de la persona: todo lo decide `judge_pair` | forma; traducción | queda |
| Responder una sugerencia (`answer_suggestion`) | la respuesta es `merge` o `apart` (`:2912-2916`) | regla: el vocabulario de la respuesta, como `Verdict` lo es de un juicio | baja ([R12](spec.md#r12-una-sugerencia-la-responde-la-persona)) |
| | responder es cosa de la persona: `user_answered` con un `reason` (`:2917-2921`) | regla: protege una unión de proyectos que nadie pidió | baja (R12) |
| | sólo se responde una sugerencia pendiente (`:2922-2929`) | regla: un agente no une proyectos que nadie sugirió | baja ([R13](spec.md#r13-sólo-se-responde-una-sugerencia-pendiente)) |
| | `apart` como `keep_apart`, `merge` como `merge_core` (`:2930-2942`) | el caso de uso | baja (R13) |
| | las sesiones siguen la unión (`sessions_follow`) | estado del daemon | queda |
| Resolver un proyecto (`resolve_projects`) | `new_project` va con `create` (`:2841-2843`) | regla: `resolve_project` con `(false, true)` ignora `new_project` y responde `Unknown` o `Similar` (`registry.rs:945-949`) | baja ([R14](spec.md#r14-new_project-va-con-create)) |
| | `ProjectResolveError` a `400`, `409 no_free_project_id` o `500` | traducción | queda |

**Descartado:**

- **Un `UpdateError::NotFound` y un `Deleted::NotFound`.** El núcleo ya responde tipado (`None`, `deleted: 0`), y `get_memory` y `get_save` responden el mismo `404` del mismo `None`: una variante para dos de las cuatro rutas partiría un texto en dos lugares. No protege ninguna escritura.
- **Comprobar el cambio antes de `target_named`.** Es el orden más natural, pero cambia qué responde un cuerpo con un nombre ambiguo y un cambio mal formado: hoy el `409`, y sería el `400`.
- **Bajar `ready_embedder` a `restore_memory_core`.** El núcleo ya pide un `&dyn Embedder` por su tipo; qué embedder hay y en qué estado es del daemon, como en `NeedsModel` ([D13](#d13-lo-que-deciden-las-demás-rutas-de-proyectos)).
- **Pasar `judge_dispute` por esta parte.** No está en el alcance; comparte `resolve_error` con `resolve_dispute`, que no cambia.

## D16. Dónde vive cada parte de la tercera

| Parte | Lugar | Por qué |
|---|---|---|
| `SuggestionAnswer`, `UserAnswer`, `AnswerError`, `Answered` y `answer_suggestion_core` | `crates/clawg-core/src/memory/projects/registry.rs`, junto a `suggestion_between` y `keep_apart` | Son los dos pasos que el handler encadena, y `registry` ya nombra `merge` (`merge_core`, por `locate_folder`): no abre ningún ciclo de `MEMORY_CYCLES` |
| `ResolveError::NewProjectWithoutCreate` | `registry.rs`, al entrar a `resolve_project` | Es el núcleo el que hoy ignora `new_project` sin `create` |

`SuggestionAnswer` pasa del daemon al núcleo con su `Serialize` en `snake_case`. Nadie fuera de `api.rs` la nombra, así que el daemon la importa y no la reexporta. `AnsweredBody` y `AnsweredMerge` quedan en el daemon: son la forma de la respuesta, con los `trajectories` y `skills` en 0 que pide un puente anterior a V32.

**Descartado:**

- **Un módulo `projects/suggestion.rs`.** Un módulo para una función y sus tipos; `registry.rs` ya tiene las sugerencias (`suggestion_between`, `keep_apart`, `pending_suggestions_in`).
- **Un `ResolveOptions::new` que compruebe la regla.** `ResolveOptions` tiene sus campos públicos y `Default`, y lo arman `eval_api` y los tests; un constructor obliga a cerrar los campos y a tocar esos llamadores, y la regla igual tiene que vivir en `resolve_project`, que es lo que escribe.

## D17. Responder una sugerencia en dos pasos

Como [D2](#d2-dos-pasos-y-la-sesión-entre-los-dos), un paso puro y uno que lee el store, y el daemon al final:

```rust
let answer = UserAnswer::new(&body.answer, body.user_answered, body.reason.as_deref()).map_err(answer_refused)?;
let answered = answer_suggestion_core(store, &workspaces, &backups, &body.project_id, &body.other_id, answer).await.map_err(answer_refused)?;
// Answered::Merged: sessions_follow, como hoy
```

- **`UserAnswer` tiene el campo privado.** `answer_suggestion_core` no se puede llamar sin pasar por las dos reglas de R12, que son las que protegen la unión: un llamador del núcleo no la salta. Es lo que hace `NewSave` en la parte 1.
- **El `reason` no viaja.** Hoy sólo se comprueba: ni `keep_apart` ni `merge_core` lo guardan. Llevarlo en `UserAnswer` sería un campo sin uso.
- **`Answered` es un `enum`** (`Apart`, `Merged(MergeReport)`), como `Located` en [D12](#d12-ubicar-una-carpeta-en-dos-pasos): no hay un `apart` con una unión. Lleva `#[expect(clippy::large_enum_variant)]` con el mismo motivo que `Located`. `Apart` no lleva el `KeptApart` de `keep_apart`, que el handler no usa.
- **`AnswerError::Pair` lleva el `KeepApartError`** de `suggestion_between` y de `keep_apart`, y el handler lo traduce con `kept_apart_error`, como hoy. `AnswerError::Merge` lleva el `MergeError`, que traduce `merge_error`.
- **`sessions_follow` queda en el handler:** `SessionRegistry` es estado del daemon.

**Descartado:**

- **Un `SuggestionAnswer: FromStr` y la regla de la persona dentro de `answer_suggestion_core`.** La respuesta se podría parsear sin la palabra de la persona, y la regla pura sólo se probaría con un store.
- **Una sola función con los textos del cuerpo.** Mezcla las reglas puras con la lectura del store, y sus tests necesitarían un store para comprobar un texto.

## D18. `new_project` sin `create`, en `resolve_project`

`resolve_project` rechaza `(create: false, new_project: true)` con `NewProjectWithoutCreate` antes de abrir su transacción, como `delete_core` con `ConfirmWithoutRows` ([R10](spec.md#r10-un-código-de-confirmación-va-con-las-filas)). El `match` de `refused` sigue igual: el brazo `(false, _, true)` deja de ver un `new_project` en `true`, pero el patrón no cambia.

`eval_api` llama a `resolve_project` con `ResolveOptions::default()`, que no lleva `new_project`, y su `match` ya tiene un comodín: no cambia. El handler agrega la variante al brazo del `400`, junto a `Name`, sin comodín.

## D19. Cómo se comprueba la tercera parte

Como [D14](#d14-cómo-se-comprueba-la-segunda-parte):

1. **Caracterización primero.** `each_memory_update_keeps_its_status_and_body`, `each_memory_delete_and_restore_keeps_its_status_and_body`, `each_resolved_dispute_keeps_its_status_and_body`, `each_answered_suggestion_keeps_its_status_and_body` y `each_resolved_project_keeps_its_status_and_body`, en el `mod tests` de `api.rs`, llaman a cada handler con el `AppCtx` de `daemon_for_jobs` (o de `daemon_embedding_with` con `FakeEmbedder`, para restaurar) y comparan el estado y el cuerpo JSON entero de cada rechazo, en su orden, y de cada respuesta aceptada. Un store cerrado da los `500`. Se commitean antes de tocar un handler y pasan sobre el código de `develop`.
2. **Los tests de respuestas no se tocan:** `git diff develop...HEAD -- crates/clawg-cli/tests/ crates/clawg-cli/src/mcp/` vacío, y los dos contratos del agente pasan sin `CLAWG_BLESS`.
3. **El diff de `api.rs`** no cambia ningún `StatusCode` ni el texto de ningún `ApiError`: los textos que se van del handler llegan como `Display` de las variantes.
4. **Los tests del núcleo**, sin HTTP: `UserAnswer::new` puro; `answer_suggestion_core` y `resolve_project` sobre `MemoryStore::new_for_test` con las filas sembradas con SQL, como los demás tests de `registry.rs`; el store que falla es uno cerrado.

## D20. Dónde van los archivos de `api`

Los archivos de dominio son **módulos hijos** de `api`, en `crates/clawg-cli/src/daemon/api/`; `api.rs` sigue siendo la raíz y los declara con `mod`. La línea que decía `crates/clawg-cli/src/daemon/routes/` en «Partes siguientes» se corrige: un hermano `routes/` no ve lo privado de `api.rs` (`AppCtx` y sus ayudas, `ApiError`, `ApiResult`, `reader`, `ready_embedder`) y habría que ensancharlo todo, y los caminos públicos `daemon::api::X` se moverían. Un hijo ve lo privado de su padre: nada se ensancha por tener que cruzar un módulo, y los caminos públicos no cambian.

| Archivo | Qué lleva |
|---|---|
| `api.rs` | `AppCtx`, la forma del error (`ErrorBody`, `ApiError`, `ApiResult`), las constantes y las puras compartidas, el router (`router`, `router_on`, `serve_ephemeral`, `admit_loopback`, `require_bearer`), las ayudas compartidas (`reader`, `named_reader`, `reach`, `ready_embedder`, `embed_failure`, `live_project`, `ReaderQuery`) y las rutas `health`, `shutdown`, `metrics` y `stats` |
| `api/memories.rs` | guardar, buscar, `context`, listar, leer, actualizar, mover, borrar, restaurar y la síntesis, con sus cuerpos y sus puras |
| `api/sessions.rs` | registrar y cerrar una sesión, contar una llamada y una búsqueda vacía, y `sessions_follow` |
| `api/outbox.rs` | reintentar y descartar un save del outbox |
| `api/disputes.rs` | resolver, juzgar y listar las disputas |
| `api/portability.rs` | `backup`, importar, los trabajos de importación y deshacer una importación |
| `api/projects.rs` | registrar, listar, renombrar, unir, borrar, mantener aparte, resolver, responder una sugerencia, ubicar una carpeta y declarar una subcarpeta |
| `api/docs.rs` | el refresco de los docs de un proyecto, la búsqueda en ellos y sus topes |
| `api/test_support.rs` | el arnés de los tests (D21) |

- **`portability.rs` y no `imports.rs`.** `daemon/imports.rs` ya existe (`ImportJobs`, `JobView`) y un hijo `api::imports` obligaría a escribir `crate::daemon::imports` en cada cita para no confundirlos. El dominio es el de `clawg_core::memory::portability`, y el nombre lo dice.
- **Los handlers son `pub(super)`**, tan anchos como los necesita el router, y un tipo privado que aparece en la firma de uno pasa a `pub(super)` con él. Los items que ya eran `pub` siguen siéndolo.
- **Lo que se usa fuera de `api` se reexporta desde `api.rs` con `pub use` explícito**, sin comodín: `DocsSearchBody`, `REFRESH_GRACE` y lo demás que usa el espacio de trabajo. `MAX_CONTENT_BYTES` ya se reexportaba.
- **`ReaderQuery` queda en la raíz**, con `reader`, `named_reader` y `reach`: es la forma de «quién pregunta» que usan la memoria, las disputas y `stats`.
- **`sessions_follow` es de `sessions.rs`** y lo llaman las rutas de proyectos y la de actualizar: `pub(super)`.

**Descartado:**

- **Un `daemon/routes/` hermano.** Por lo de arriba: ensancha la visibilidad de lo compartido y mueve los caminos públicos.
- **Un archivo por ruta.** Más de sesenta archivos de veinte líneas; el dominio es la unidad que se lee junta con sus cuerpos y sus puras.
- **Un `use super::*` en cada dominio.** Esconde de dónde viene cada nombre; cada archivo importa lo que usa.

## D21. Los tests

Cada test se muda al `#[cfg(test)] mod tests` del archivo cuyo código prueba, con su nombre intacto. El arnés que comparten (`daemon_on`, `daemon_for_jobs`, `daemon_embedding_with`, `daemon_without_embedder`, `status_and_body`, `answer_of`, `seed_memory`, `run_sql`, `count`, `reader_query`) va a `crates/clawg-cli/src/daemon/api/test_support.rs`, declarado en `api.rs` como `#[cfg(test)] mod test_support;`, con sus items `pub(super)`. Los tests de lo que se queda en la raíz (`serve_ephemeral`, `constant_time_eq`) se quedan en `api.rs`. Los ayudantes de un solo dominio (los de los trabajos de importación, `chunk`, `query`) van con sus tests. `test_support.rs` empieza con `#![cfg(test)]`: `tests/conventions.rs` mide el código de producción de un archivo hasta su `mod tests`, y este archivo es todo de tests (siembra filas con `rusqlite`). La convención reconoce esa línea y el archivo no cuenta como producción; es el único cambio de `tests/` de esta parte.

## D22. Las citas

Los documentos vivos que citan `crates/clawg-cli/src/daemon/api.rs` (`docs/reference/*.md` y `openspec/specs/*/spec.md`) pasan a citar el archivo donde vive cada item, por el nombre del item y **sin número de línea**: un número de línea se vuelve falso en la próxima edición. No se confunde con `crates/clawg-cli/tests/api.rs`, que no se mueve. Los cambios archivados (`openspec/changes/archive/`) y los demás cambios activos son historia: no se tocan. Las citas con línea de las partes 1 a 3 de este mismo cambio son del `develop` de entonces y quedan como están.

## D23. Cómo se comprueba la cuarta parte

1. **Los nombres de los tests.** Antes de empezar se guarda `cargo test -p clawg-cli --lib -- --list` ordenado; al terminar se compara, ignorando el camino del módulo. Son los mismos.
2. **Los tests de respuestas y los contratos no se tocan:** `git diff develop...HEAD -- crates/clawg-cli/tests/ crates/clawg-cli/src/mcp/` vacío.
3. **Cada commit compila sin avisos** y pasa `cargo test -p clawg-cli --lib daemon::api`; la suite entera y los dos clippy corren una vez, sobre el último commit de código.
