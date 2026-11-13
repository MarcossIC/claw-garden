# core-use-cases — Especificación

Son requisitos sobre [`specs/ingest`](../../specs/ingest/spec.md), [`specs/synthesis`](../../specs/synthesis/spec.md) y [`specs/provenance`](../../specs/provenance/spec.md), y se consolidan al archivar. Lo que no se nombra acá sigue como está. Cubren la primera parte del cambio, el guardado (`POST /v1/memories`); las partes siguientes traen sus requisitos.

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
