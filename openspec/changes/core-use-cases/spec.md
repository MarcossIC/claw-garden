# core-use-cases — Especificación

Son requisitos sobre [`specs/ingest`](../../specs/ingest/spec.md), [`specs/synthesis`](../../specs/synthesis/spec.md), [`specs/provenance`](../../specs/provenance/spec.md) y [`specs/project`](../../specs/project/spec.md), y se consolidan al archivar. Lo que no se nombra acá sigue como está. R1 a R7 cubren la primera parte del cambio, el guardado (`POST /v1/memories`); R8 a R11, la segunda, las escrituras de proyectos ([«Parte 2»](#parte-2-las-escrituras-de-proyectos)). Las partes siguientes traen sus requisitos.

Términos:

- **Un save:** el cuerpo de `POST /v1/memories`, como lo lee el daemon (`SaveRequest`).
- **El núcleo:** `crates/clawg-core`. **Sin HTTP:** un test del núcleo que no construye un `AppCtx`, un router ni un `StatusCode`.
- **Un rechazo:** un `SaveRejected`. Su texto es su `Display`.

## R1. El formato de un save se comprueba en el núcleo

`NewSave::parse(SaveInput)` (`crates/clawg-core/src/memory/ingest/save.rs`) MUST ser una función pura que devuelve un `NewSave` o el primer rechazo, en este orden, que es el de hoy:

| # | Regla | Rechazo | Texto |
|---|---|---|---|
| 1 | `content` sin nada más que espacios | `EmptyContent` | `content must not be empty` |
| 2 | `content` de más de `MAX_CONTENT_BYTES` (65 536) bytes en UTF-8 | `ContentTooLong { bytes }` | `content is <bytes> bytes; at most 65536 are accepted: split it into several saves, one fact each` |
| 3 | `memory_type` que no es uno de los siete | `UnknownType` | el de `UnknownMemoryType`: `unknown memory_type "<valor>"; expected one of …` |
| 4 | `operation_id` vacío, de más de `MAX_OPERATION_ID_LEN` (128) bytes o con un carácter que no es ASCII visible | `OperationId` | `operation_id must be 1 to 128 visible ASCII characters` |
| 5 | `topic_key` que no cumple el formato | `TopicKey` | el de `TopicKeyError` |
| 6 | `derived_from` con más de `MAX_SOURCES` (50) ids distintos | `TooManySources { count }` | `derived_from names <count> memories; at most 50 are accepted` |
| 7 | `user_stated` con `derived_from` no vacío | `StatedConclusion` | `user_stated cannot go with derived_from: a conclusion is drawn by the agent, not stated by the user` |

`derived_from` MUST quedar con cada id una vez y en orden ascendente. `check_operation_id` MUST ser la regla 4 sola, y las rutas del outbox (`POST /v1/outbox/retry`, `POST /v1/outbox/discard`) MUST usarla.

#### Escenario: el orden
- GIVEN un save con `content` vacío y `memory_type` `"preference"`
- WHEN se parsea
- THEN el rechazo es `EmptyContent`

#### Escenario: los bytes
- GIVEN un `content` de `MAX_CONTENT_BYTES / 2 + 1` veces `ñ`
- WHEN se parsea
- THEN el rechazo es `ContentTooLong { bytes: MAX_CONTENT_BYTES + 2 }`, y uno de `MAX_CONTENT_BYTES / 2` veces `ñ` pasa

**Tests** (`crates/clawg-core/src/memory/ingest/save.rs`, sin HTTP): `content_is_checked_in_bytes`, `operation_ids_are_checked`, `sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `the_format_is_checked_in_order`, `each_format_refusal_keeps_its_text` y `a_conclusion_cannot_be_stated_by_the_user`. Los cuatro primeros tienen el nombre y los literales de los tests puros que hoy están en el `mod tests` de `crates/clawg-cli/src/daemon/api.rs`. Hoy ninguno compila: el módulo no existe.

## R2. El tipo se parsea una vez

`NewSave` MUST guardar el `memory_type` como `Option<MemoryType>`. Lo que sigue al parseo MUST leer ese valor y no el texto que mandó el cliente:

- `Admitted::memory_type` lo devuelve, y el daemon cuenta el save en su sesión con él (`Activity::Save`), o con `UNSPECIFIED` sin tipo, como hoy;
- el `MemoryEvent` que sale de `Admitted::into_event` MUST llevar `MemoryType::as_str` en `memory_type`, que es el mismo texto que el cliente mandó, porque la regla 3 de R1 sólo acepta los siete nombres exactos.

#### Escenario
- GIVEN un save con `memory_type` `"user_preference"`
- WHEN se parsea, se admite y se pasa a evento
- THEN el `NewSave` tiene `Some(MemoryType::UserPreference)` y el evento tiene `Some("user_preference")`

**Tests:** `a_parsed_save_keeps_its_type` (`save.rs`) y `an_admitted_save_becomes_the_event_the_daemon_queued` (`crates/clawg-core/src/runtime.rs`). Hoy no compilan.

## R3. Las fuentes de una conclusión están vivas y al alcance

`MemoryRuntime::admit(NewSave)` MUST, cuando `derived_from` no está vacío, leer la política de `sharing.json` del directorio del runtime para el agente del save y rechazar, en este orden:

1. con `MissingSources(ids)` si algún id no nombra una fila no borrada que el agente alcanza: `derived_from names memories that do not exist or were deleted: <ids separados por ", ">`, en el orden de `derived_from`. Una fila que la política oculta MUST contar como inexistente.
2. con `ReplacedSources(pares)` si alguno nombra una fila `superseded`: `derived_from names memories that were replaced: <id (by <id>), …>; draw the conclusion from the current ones`.

Sin `derived_from`, `admit` MUST NOT leer `sharing.json` ni consultar las fuentes. Un error del store MUST ser `SaveRejected::Store`, con el texto del `MemoryError`.

#### Escenario
- GIVEN una fila viva de `a`, una borrada, una de `b` que `sharing.json` le oculta a `a`, y una reemplazada por otra
- WHEN `a` admite una conclusión de cada una
- THEN las tres primeras dan `MissingSources` con los ids que faltan, la reemplazada da `ReplacedSources` con su reemplazo, y la viva sola se admite

**Tests:** `a_conclusion_from_missing_or_hidden_sources_is_refused`, `a_conclusion_from_a_replaced_source_is_refused` y `a_store_that_fails_refuses_the_save_with_its_error` (`runtime.rs`, sin HTTP). Hoy no compilan.

## R4. Sin embedder que venga, el save no se acepta

Después de las fuentes, `admit` MUST leer el estado del embedder:

- `failed` MUST dar `EmbedderFailed { error, retryable }`, con la causa y si se reintenta tal como están en el estado;
- `loading` MUST admitir el save con `Admitted::waits` en `false`: el daemon no espera al worker y responde `202 embedder_loading`, como hoy;
- `ready` MUST admitirlo con `waits` en `true`.

#### Escenario
- GIVEN un runtime cuyo embedder pasó a `failed` con `disk full`, reintentable
- WHEN admite un save
- THEN el rechazo es `EmbedderFailed { error: "disk full", retryable: true }`

**Tests:** `a_save_is_refused_while_the_embedder_failed` y `a_save_waits_only_for_a_ready_embedder` (`runtime.rs`). Hoy no compilan.

## R5. El save va al proyecto vivo

Después del embedder, `admit` MUST seguir el `project_id` del save hasta su proyecto vivo (`merge::canonical`), y `Admitted::project_id` MUST ser ese. Un save sin proyecto MUST seguir sin proyecto.

#### Escenario
- GIVEN el proyecto `app` fusionado en `web`
- WHEN se admite un save de `app`
- THEN `Admitted::project_id` es `web`

**Test:** `a_save_of_a_merged_project_goes_to_its_target` (`runtime.rs`). Hoy no compila.

## R6. El evento de un save admitido

`Admitted::into_event(session_id, timestamp_ms)` MUST devolver el `MemoryEvent` que hoy arma `save_memory`: `role` `User`, `is_error` `None`, `source` `Explicit`, y el contenido, el agente, el proyecto vivo, el tipo (R2), la importancia, las fuentes de R1, la clave, `user_stated`, la sesión y el instante que recibe. Con él MUST devolver el `operation_id` del save, que `accept` recibe aparte. La sesión la MUST elegir el daemon, con el agente y el proyecto vivo de `Admitted`, antes de `into_event`.

**Test:** `an_admitted_save_becomes_the_event_the_daemon_queued` (`runtime.rs`).

## R7. Lo que ve un cliente no cambia

Es de conservación: sus tests pasan antes y después.

`save_memory` MUST traducir cada rechazo así, y no decidir ninguna regla:

| Rechazo | Respuesta |
|---|---|
| R1 (las siete), `MissingSources`, `ReplacedSources` | `400 {"error": "<texto>"}` |
| `EmbedderFailed { error, retryable }` | `503 {"error": "embedder_failed", "detail": "<error>", "retryable": <retryable>}` |
| `Store` | `500 {"error": "<texto del MemoryError>"}` |

Ninguna respuesta del daemon (estado, cuerpo), del puente MCP (texto, esquema, descripción) ni de la CLI MUST cambiar. `clawg_cli::daemon::api::MAX_CONTENT_BYTES` MUST seguir existiendo, con el mismo valor, porque los tests de respuestas lo importan de ahí.

**Tests:**
- `each_refused_save_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`), que llama al handler con un `AppCtx` de prueba y compara el estado y el cuerpo entero de un caso por fila de la tabla y por regla. Se escribe primero y pasa sobre el código de `develop`.
- La suite entera, sin editar `crates/clawg-cli/tests/api.rs` ni los tests de `crates/clawg-cli/src/mcp/`.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`crates/clawg-cli/src/mcp/server.rs`), sin `CLAWG_BLESS`.

## Parte 2. Las escrituras de proyectos

Términos:

- **Un registro:** el cuerpo de `POST /v1/projects` (`RegisterRequest`), y el registro que `POST /v1/projects/locate` hace de la raíz que ubica.
- **Ubicar una carpeta:** `POST /v1/projects/locate` sin `subfolder` (`LocateRequest`, [`specs/project`](../../specs/project/spec.md), PL39).
- **Su proyecto:** el que el registro le da a la carpeta que se ubica. **El nombrado:** el proyecto al que lleva el `project` del cuerpo (`find_project`).

## R8. La raíz de un registro se comprueba en el núcleo

`check_root(root_path, index_docs, is_dir)` (`crates/clawg-core/src/memory/projects/project.rs`) MUST devolver la raíz que nombra `root_path` o el primer rechazo, en este orden, que es el de hoy:

| # | Regla | Rechazo | Texto |
|---|---|---|---|
| 1 | `root_path` sin nada más que espacios | `RegisterError::EmptyRoot` | `root_path must not be empty` |
| 2 | con `index_docs`, una raíz que `is_dir` no da por directorio | `RegisterError::NotADirectory(raíz)` | `<root_path> is not a directory, so it has no files to index` |

`is_dir` es lo único que lee el disco, y sin `index_docs` MUST NOT llamarse. El daemon MUST llamar a `check_root` antes de leer el checkout de la raíz y antes de `register_project`, así un `InvalidLegacyId` sigue después de las dos reglas.

#### Escenario: el orden
- GIVEN un registro con `root_path` `"  "` e `index_docs`
- WHEN se comprueba
- THEN el rechazo es `EmptyRoot`, y `is_dir` no se llamó

#### Escenario: sólo la raíz que se indexa
- GIVEN una raíz que no es un directorio
- WHEN se comprueba sin `index_docs`
- THEN pasa, y `is_dir` no se llamó; con `index_docs` el rechazo es `NotADirectory`

**Tests** (`project.rs`, sin HTTP): `an_empty_root_is_not_registered`, `only_a_root_to_index_must_be_a_directory` y `each_root_refusal_keeps_its_text`. Hoy no compilan: `check_root` no existe.

## R9. Ubicar una carpeta se decide en el núcleo

`check_folder(folder, is_dir)` (`crates/clawg-core/src/memory/projects/registry.rs`) MUST rechazar con `LocateError::NotADirectory(folder)` (`<root_path> is not a directory: name a directory of the project`) una carpeta que `is_dir` no da por directorio. El daemon MUST llamarla antes que cualquier otra cosa de la ruta, como hoy.

`locate_folder(store, workspaces, backups, folder, own, target)` (`registry.rs`), con `own` su proyecto y `target` el nombrado, MUST:

1. con `own` igual a `target`, devolver `Located::Already` sin leer el store;
2. si `own` tiene más de una carpeta (`own_folder_count`, SB3 de `specs/project`), rechazar con `LocateError::FolderShared { folder, own, folders, target }`, cuyo texto es el `detail` de hoy: `<root_path> leads to project <own>, which has <N> folders; `clawg projects merge <own> --into <target>` merges them all`;
3. si no, unir `own` en `target` como `merge_core` sin prueba en seco, y devolver `Located::Merged(informe)`. Una unión rechazada MUST ser `LocateError::Merge(MergeError)`, con el error de la unión tal cual.

Un error de la cuenta MUST ser `LocateError::Store(StorageError)`, con el texto del `StorageError`. `own_folder_count` MUST quedar privada de `registry.rs`: el daemon no la nombra.

#### Escenario
- GIVEN el proyecto `a` con una carpeta, el `b` con dos y el nombrado `ideas`
- WHEN se ubica la carpeta de `b`, la de `a` y otra vez la de `a`
- THEN la de `b` da `FolderShared` con 2 carpetas y no une nada, la de `a` da `Merged` con `a` unido en `ideas`, y la segunda vez su proyecto ya es `ideas` y da `Already`

**Tests** (`registry.rs`, sin HTTP): `a_folder_that_is_not_a_directory_is_not_located`, `a_folder_already_in_the_named_project_is_left_as_it_is`, `a_folder_shared_by_its_project_is_not_merged`, `the_only_folder_of_a_project_merges_it_into_the_named_one`, `a_refused_merge_refuses_the_locate` y `a_store_that_fails_refuses_the_locate`. Hoy no compilan: `locate_folder` no existe.

## R10. Un código de confirmación va con las filas

`delete_core` (`crates/clawg-core/src/memory/projects/project_delete.rs`) con `with_memories` en `false` y un `confirm` MUST rechazar con `DeleteError::ConfirmWithoutRows` (`confirm goes with with_memories: it is the code of the report of a delete with its rows`) antes de leer el store. Hoy el núcleo ignora el código y borra el proyecto como vacío, y sólo el handler lo impide.

#### Escenario
- GIVEN un proyecto que no existe
- WHEN se borra sin `with_memories` y con un `confirm`
- THEN el rechazo es `ConfirmWithoutRows`, no `UnknownProject`

**Test:** `a_confirm_without_the_rows_is_refused_before_anything_is_read` (`project_delete.rs`). Hoy no compila: la variante no existe.

## R11. Lo que ve un cliente de los proyectos no cambia

Es de conservación: sus tests pasan antes y después.

Los handlers MUST traducir cada rechazo así, y no decidir ninguna de las reglas de R8 a R10:

| Rechazo | Respuesta |
|---|---|
| `RegisterError::EmptyRoot`, `NotADirectory`, `InvalidLegacyId` | `400 {"error": "<texto>"}` |
| `RegisterError::RootCollision`, `NoFreeId`, `Store` | como hoy: `409 project_root_collision` con uno de sus dos `detail`, `409 no_free_project_id`, `500` |
| `LocateError::NotADirectory` | `400 {"error": "<texto>"}` |
| `LocateError::FolderShared` | `409 {"error": "folder_shared", "detail": "<texto>"}` |
| `LocateError::Merge(e)` | lo que `merge_error` responde por `e` en `POST /v1/projects/merge` |
| `LocateError::Store` | `500 {"error": "<texto del StorageError>"}` |
| `DeleteError::ConfirmWithoutRows` | `400 {"error": "<texto>"}` |

`Located::Already` y `Located::Merged` MUST dar el mismo `LocatedBody` de hoy: `already` en `true` sin `merge`, o en `false` con el informe de la unión. Las sesiones siguen la unión en el daemon (`sessions_follow`), como hoy.

Ninguna respuesta del daemon (estado, cuerpo), del puente MCP (texto, esquema, descripción) ni de la CLI MUST cambiar.

**Tests:**
- `each_refused_registration_keeps_its_status_and_body`, `each_located_folder_keeps_its_status_and_body` y `each_refused_project_delete_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`), que llaman al handler con un `AppCtx` de prueba y comparan el estado y el cuerpo entero. Se escriben primero y pasan sobre el código de `develop`.
- La suite entera, sin editar `crates/clawg-cli/tests/api.rs` ni los tests de `crates/clawg-cli/src/mcp/`; entre ellos `locating_a_folder_merges_its_own_project_into_the_named_one`, `a_folder_shared_by_its_project_is_not_located` y `registering_a_missing_root_to_index_it_is_a_bad_request`.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, sin `CLAWG_BLESS`.
