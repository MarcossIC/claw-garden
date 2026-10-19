# daemon-access — Especificación

Fuente de verdad de qué peticiones llegan a la API HTTP del daemon antes de mirar el token, y de cómo nacen en disco el token y `daemon.json`. Qué hace cada ruta está en los dominios de cada una ([`specs/ingest`](../ingest/spec.md), [`specs/retrieval`](../retrieval/spec.md), [`specs/mcp`](../mcp/spec.md), …); el token en sí (32 bytes aleatorios en hexadecimal, regenerado en cada arranque) y `require_bearer` no cambian.

**Origen:**
- [daemon-hardening](../../changes/archive/2026-10-19-daemon-hardening/proposal.md), archivado el 2026-10-19. Los ids de requisito son los de esa spec; el porqué de cada decisión, y lo descartado (ACL en Windows, recortar `/health`, pipes o sockets, comprobar el puerto), está en su propuesta y su [exploración](../../changes/archive/2026-10-19-daemon-hardening/exploration.md).

## Definiciones

- **Nombre de loopback:** `127.0.0.1`, `localhost` o `[::1]`, comparados sin distinguir mayúsculas (`LOOPBACK_NAMES`, `crates/clawg-cli/src/daemon/guard.rs:19`).
- **Autoridades de la petición:** la cabecera `Host` y la autoridad de la URI (la `:authority` de HTTP/2, o la de una URI absoluta en HTTP/1.1), las que estén. El daemon sirve HTTP/2 en texto plano: `hyper-util` llega con `http2` a través de `reqwest`.
- **Guardia:** `admit_loopback` (`crates/clawg-cli/src/daemon/api.rs:883`), la capa que `router` (`api.rs:823`) aplica a todas sus rutas, `/health` y `/v1` incluidas, por fuera de `require_bearer` (`api.rs:846`).

## Límite

El guardia cierra el paso a una página web abierta en el navegador del usuario, con DNS rebinding o sin él. No es una barrera contra un proceso local: uno que puede leer el token manda también el `Host` correcto. Otro usuario local que llega al puerto sigue frenado sólo por el token.

## R1. La decisión es pura

`admits(host, uri_authority, origin)` (`guard.rs:28`) MUST admitir una petición sólo cuando nombra al menos una autoridad, toda autoridad que nombra es, sin el puerto, un nombre de loopback (`is_loopback_authority`, `guard.rs:36`), y no trae `Origin`. Un `Origin` presente MUST rechazarse con cualquier valor, `null` y vacío incluidos. Un puerto vacío o que no sea un número, o un nombre de loopback seguido de otra cosa (`127.0.0.1.evil.example`, `localhost.`), MUST rechazarse. El puerto MUST NOT compararse con el del socket.

| Autoridad | `Origin` | Resultado |
|---|---|---|
| `127.0.0.1:27380` | — | admitida |
| `localhost:27381` | — | admitida |
| `LOCALHOST` | — | admitida |
| `[::1]:27380` | — | admitida |
| sólo en la URI: `127.0.0.1:27380` | — | admitida |
| `evil.example:27380` | — | rechazada |
| `127.0.0.1.evil.example` | — | rechazada |
| `localhost.` | — | rechazada |
| `127.0.0.1:x` | — | rechazada |
| `Host: 127.0.0.1:27380`, URI `http://evil.example/` | — | rechazada |
| `127.0.0.1:27380` | `http://evil.example` | rechazada |
| `127.0.0.1:27380` | `null` | rechazada |
| — | — | rechazada |

Una cabecera `Host` u `Origin` que no es texto cuenta como presente y vacía.

**Tests:** `loopback_names_are_admitted_with_or_without_a_port`, `a_foreign_or_lookalike_host_is_refused`, `any_origin_is_refused`, `a_request_without_an_authority_is_refused`, `the_uri_authority_stands_in_for_a_missing_host`, `a_foreign_uri_authority_is_refused_behind_a_loopback_host` (`daemon/guard.rs`, mod tests).

## R2. El guardia corre en toda ruta y antes del token

Una petición que `admits` rechaza MUST recibir `403` con `{"error":"forbidden"}`, el mismo cuerpo para los dos motivos, en `/health` y en `/v1`, traiga o no un token válido. El handler MUST NOT correr. Como el guardia está dentro de `router`, lo pasan igual el daemon, `serve_ephemeral` y los tests de la API.

#### Escenario: DNS rebinding
- GIVEN un daemon en `127.0.0.1:<puerto>`
- WHEN llega `GET /health` con `Host: evil.example:<puerto>`
- THEN la respuesta es `403 {"error":"forbidden"}`

#### Escenario: una página con el token
- GIVEN un daemon y su token
- WHEN llega `POST /v1/search` con el token y `Origin: http://evil.example`
- THEN la respuesta es `403 {"error":"forbidden"}` y nada se busca

**Tests:** `a_foreign_host_is_forbidden_on_health_and_v1_with_or_without_a_token`, `an_origin_is_forbidden_even_with_the_token`, `both_refusals_answer_the_same_body` (`crates/clawg-cli/tests/api.rs`).

## R3. Los clientes de hoy pasan

`DaemonClient` (`crates/clawg-cli/src/client.rs`) MUST seguir guardando, buscando, leyendo `/health` y apagando contra el router real: dialoga con `http://127.0.0.1:<puerto>` y no manda `Origin`. Un cliente que integre con el daemon MUST usar un nombre de loopback y MUST NOT mandar `Origin`.

**Tests:** `a_daemon_client_passes_the_guard` (`tests/api.rs`); `tests/binary.rs` (`status`, `stop`, MCP por stdio) sin cambios.

## R4. Los rechazos se registran con tope

El guardia MUST registrar un `warn` por el primer rechazo y después MUST NOT registrar más de uno cada `REFUSAL_LOG_EVERY` (60 s, `guard.rs:23`), cada uno con cuántos rechazos hubo desde el anterior (`RefusalLog::refused`, `guard.rs:67`). MUST NOT escribir los valores de `Host` ni de `Origin`.

**Tests:** `the_first_refusal_is_logged_and_the_next_wait_a_minute` (`daemon/guard.rs`, mod tests).

## R5. El token y `daemon.json` nacen restringidos (Unix)

En Unix, `write_token` y `write_state` (`crates/clawg-cli/src/daemon/lifecycle.rs:137`, `:118`) MUST borrar el archivo anterior y crear uno nuevo con modo 0600 desde su creación (`create_owner_only`, `lifecycle.rs:95`), de modo que el contenido nuevo nunca esté en un archivo con otro modo ni lo lea quien tuviera abierto el anterior. En Windows MUST truncar el archivo en su lugar, sin ACL explícita (`lifecycle.rs:110`): lo sostiene la ACL del directorio personal, y un archivo borrado que otro proceso tiene abierto rechazaría el nuevo.

#### Escenario: un token anterior legible por todos
- GIVEN `<data>/token` con modo 0644, abierto por otro proceso
- WHEN el daemon escribe su token
- THEN `<data>/token` tiene modo 0600 y guarda el token nuevo
- AND quien tenía abierto el archivo anterior sigue leyendo el token viejo

**Tests:** `a_created_file_is_owner_only_before_anything_is_written`, `an_old_world_readable_token_is_replaced_by_an_owner_only_file`, `state_file_is_owner_only_on_unix`, `token_file_is_owner_only_on_unix` (`lifecycle.rs`, mod tests, Unix).
