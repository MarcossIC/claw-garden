# daemon-hardening — Especificación

## Definiciones

- **Nombre de loopback:** `127.0.0.1`, `localhost` o `[::1]`, comparados sin distinguir mayúsculas.
- **Autoridad de la petición:** la cabecera `Host` si está; si no, la autoridad de la URI (la `:authority` de HTTP/2).
- **Guardia:** la capa que `router` (`crates/clawg-cli/src/daemon/api.rs`) aplica a todas sus rutas, `/health` y `/v1` incluidas, antes de `require_bearer`.

## R1. La decisión es pura

`admits(host, uri_authority, origin)` MUST admitir una petición sólo cuando nombra al menos una autoridad, toda autoridad que nombra (la cabecera `Host` y la autoridad de la URI, que en una URI absoluta manda sobre `Host`) es, sin el puerto, un nombre de loopback, y no trae `Origin`. Un `Origin` presente MUST rechazarse con cualquier valor, `null` incluido. Sin autoridad MUST rechazarse. Un puerto que no sea un número, o un nombre de loopback seguido de otra cosa (`127.0.0.1.evil.example`, `localhost.`), MUST rechazarse.

| Autoridad | `Origin` | Resultado |
|---|---|---|
| `127.0.0.1:27380` | — | admitida |
| `localhost:27381` | — | admitida |
| `LOCALHOST` | — | admitida |
| `[::1]:27380` | — | admitida |
| `evil.example:27380` | — | rechazada |
| `127.0.0.1.evil.example` | — | rechazada |
| `localhost.` | — | rechazada |
| `127.0.0.1:x` | — | rechazada |
| `127.0.0.1:27380` | `http://evil.example` | rechazada |
| `127.0.0.1:27380` | `null` | rechazada |
| — | — | rechazada |
| sólo en la URI: `127.0.0.1:27380` | — | admitida |
| `Host: 127.0.0.1:27380`, URI `http://evil.example/` | — | rechazada |

**Tests:** `loopback_names_are_admitted_with_or_without_a_port`, `a_foreign_or_lookalike_host_is_refused`, `any_origin_is_refused`, `a_request_without_an_authority_is_refused`, `the_uri_authority_stands_in_for_a_missing_host`, `a_foreign_uri_authority_is_refused_behind_a_loopback_host` (`daemon/guard.rs`, mod tests).

## R2. El guardia corre en toda ruta y antes del token

Una petición que `admits` rechaza MUST recibir `403` con `{"error":"forbidden"}`, el mismo cuerpo para los dos motivos, en `/health` y en `/v1`, traiga o no un token válido. El handler MUST NOT correr.

#### Escenario: DNS rebinding
- GIVEN un daemon en `127.0.0.1:<puerto>`
- WHEN llega `GET /health` con `Host: evil.example:<puerto>`
- THEN la respuesta es `403 {"error":"forbidden"}`

#### Escenario: una página con el token
- GIVEN un daemon y su token
- WHEN llega `POST /v1/search` con el token y `Origin: http://evil.example`
- THEN la respuesta es `403 {"error":"forbidden"}` y nada se busca

**Tests:** `a_foreign_host_is_forbidden_on_health_and_v1_with_or_without_a_token`, `an_origin_is_forbidden_even_with_the_token`, `both_refusals_answer_the_same_body` (`tests/api.rs`).

## R3. Los clientes de hoy pasan

`DaemonClient` (`crates/clawg-cli/src/client.rs`) MUST seguir guardando, buscando, leyendo `/health` y apagando contra el router real en un puerto efímero: manda `Host: 127.0.0.1:<puerto>` y ningún `Origin`. Los tests de la API, los de `client.rs` y `tests/binary.rs` MUST pasar sin cambios en sus peticiones.

**Tests:** `a_daemon_client_passes_the_guard` (`tests/api.rs`); la suite existente.

## R4. Los rechazos se registran con tope

El guardia MUST registrar un `warn` por el primer rechazo y después MUST NOT registrar más de uno por minuto, cada uno con cuántos rechazos hubo desde el anterior. MUST NOT escribir los valores de `Host` ni de `Origin`.

**Tests:** `the_first_refusal_is_logged_and_the_next_wait_a_minute` (`daemon/guard.rs`, mod tests).

## R5. El token y `daemon.json` nacen restringidos (Unix)

En Unix, `write_token` y `write_state` (`crates/clawg-cli/src/daemon/lifecycle.rs`) MUST crear un archivo nuevo con modo 0600 desde su creación, también cuando ya existía uno, de modo que el contenido nuevo nunca esté en un archivo con otro modo. En Windows MUST seguir sin ACL explícita.

#### Escenario: un token anterior legible por todos
- GIVEN `<data>/token` con modo 0644
- WHEN el daemon escribe su token
- THEN `<data>/token` tiene modo 0600 y guarda el token nuevo
- AND quien tenía abierto el archivo anterior sigue leyendo el token viejo

**Tests:** `a_created_file_is_owner_only_before_anything_is_written`, `an_old_world_readable_token_is_replaced_by_an_owner_only_file`, `state_file_is_owner_only_on_unix` (`lifecycle.rs`, mod tests, Unix).
