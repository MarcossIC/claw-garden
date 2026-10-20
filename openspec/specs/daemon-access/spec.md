# daemon-access — Especificación

Fuente de verdad de qué peticiones llegan a la API HTTP del daemon antes de mirar el token, de cómo nacen en disco el token y `daemon.json`, y del modo del directorio de datos. Qué hace cada ruta está en los dominios de cada una ([`specs/ingest`](../ingest/spec.md), [`specs/retrieval`](../retrieval/spec.md), [`specs/mcp`](../mcp/spec.md), …); el token en sí (32 bytes aleatorios en hexadecimal, regenerado en cada arranque) y `require_bearer` no cambian.

**Origen:**
- [daemon-hardening](../../changes/archive/2026-10-19-daemon-hardening/proposal.md), archivado el 2026-10-19. Los ids de requisito son los de esa spec; el porqué de cada decisión, y lo descartado (ACL en Windows, recortar `/health`, pipes o sockets, comprobar el puerto), está en su propuesta y su [exploración](../../changes/archive/2026-10-19-daemon-hardening/exploration.md).
- [local-isolation](../../changes/archive/2026-10-19-local-isolation/proposal.md), archivado el 2026-10-19: agrega R6 y cambia el `Límite` y R5.
- [local-transport](../../changes/archive/2026-10-20-local-transport/proposal.md), archivado el 2026-10-20: agrega R7 a R9 (el socket del directorio de datos, TCP a su lado y el cliente) y cambia el `Límite` y R3. Por qué el chequeo de uid además del directorio, la marca en `daemon.json` en lugar de la ruta y TCP no fatal sin `--port`, en su [diseño](../../changes/archive/2026-10-20-local-transport/design.md); la medición con dos cuentas, en su [verify](../../changes/archive/2026-10-20-local-transport/verify.md). Por qué el directorio y no cada archivo, sin `umask`, y por qué no poder apretar no detiene a clawg, en su [diseño](../../changes/archive/2026-10-19-local-isolation/design.md); la medición con dos cuentas, `memory.lock` incluido, en su [verify](../../changes/archive/2026-10-19-local-isolation/verify.md).

## Definiciones

- **Nombre de loopback:** `127.0.0.1`, `localhost` o `[::1]`, comparados sin distinguir mayúsculas (`LOOPBACK_NAMES`, `crates/clawg-cli/src/daemon/guard.rs:19`).
- **Autoridades de la petición:** la cabecera `Host` y la autoridad de la URI (la `:authority` de HTTP/2, o la de una URI absoluta en HTTP/1.1), las que estén. El daemon sirve HTTP/2 en texto plano: `hyper-util` llega con `http2` a través de `reqwest`.
- **Directorio de datos:** `<home>/.clawg` o `<home>/.clawg-dev` (`data_dir_in`, `crates/clawg-core/src/paths.rs:159`).
- **Modo privado:** en Unix, un directorio sin ningún bit de grupo ni de otros (`modo & 0o077 == 0`).
- **Socket del perfil:** `<data>/clawg.sock` (`socket_file`, `crates/clawg-core/src/paths.rs`), un socket de dominio Unix con ruta, nunca abstracto.
- **Guardia:** `admit_loopback` (`crates/clawg-cli/src/daemon/api.rs:906`), la capa que `router` (`api.rs:846`) aplica a todas sus rutas, `/health` y `/v1` incluidas, por fuera de `require_bearer` (`api.rs:869`).

## Límite

El guardia cierra el paso a una página web abierta en el navegador del usuario, con DNS rebinding o sin él. No es una barrera contra un proceso local: uno que puede leer el token manda también el `Host` correcto. En Unix otro usuario local no lee nada del directorio de datos (R6), ni el token, ni llega al socket del perfil (R7); un cliente que publica el socket nunca dialoga por TCP (R9), así que un proceso ajeno en el puerto del perfil ya no se hace pasar por el daemon ante él. Mientras TCP siga encendido para los clientes anteriores (R8), otro usuario llega a ese puerto y lee `/health`, y lo demás lo frena el token. Nada de esto frena a un proceso del mismo usuario ni a root.

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

`DaemonClient` (`crates/clawg-cli/src/client.rs`) MUST seguir guardando, buscando, leyendo `/health` y apagando contra el router real: dialoga con `http://127.0.0.1:<puerto>`, o con `http://localhost` por el socket, y no manda `Origin`. Un cliente que integre con el daemon MUST usar un nombre de loopback y MUST NOT mandar `Origin`.

**Tests:** `a_daemon_client_passes_the_guard` (`tests/api.rs`); `tests/binary.rs` (`status`, `stop`, MCP por stdio) sin cambios.

## R4. Los rechazos se registran con tope

El guardia MUST registrar un `warn` por el primer rechazo y después MUST NOT registrar más de uno cada `REFUSAL_LOG_EVERY` (60 s, `guard.rs:23`), cada uno con cuántos rechazos hubo desde el anterior (`RefusalLog::refused`, `guard.rs:67`). MUST NOT escribir los valores de `Host` ni de `Origin`.

**Tests:** `the_first_refusal_is_logged_and_the_next_wait_a_minute` (`daemon/guard.rs`, mod tests).

## R5. El token y `daemon.json` nacen restringidos (Unix)

En Unix, `write_token` y `write_state` (`crates/clawg-cli/src/daemon/lifecycle.rs:137`, `:118`) MUST borrar el archivo anterior y crear uno nuevo con modo 0600 desde su creación (`create_owner_only`, `lifecycle.rs:95`), de modo que el contenido nuevo nunca esté en un archivo con otro modo ni lo lea quien tuviera abierto el anterior. En Windows MUST truncar el archivo en su lugar, sin ACL explícita (`lifecycle.rs:110`): lo sostiene la ACL del directorio personal, y un archivo borrado que otro proceso tiene abierto rechazaría el nuevo. Con R6, el modo 0600 del token y de `daemon.json` es una segunda capa; la primera es el directorio.

#### Escenario: un token anterior legible por todos
- GIVEN `<data>/token` con modo 0644, abierto por otro proceso
- WHEN el daemon escribe su token
- THEN `<data>/token` tiene modo 0600 y guarda el token nuevo
- AND quien tenía abierto el archivo anterior sigue leyendo el token viejo

**Tests:** `a_created_file_is_owner_only_before_anything_is_written`, `an_old_world_readable_token_is_replaced_by_an_owner_only_file`, `state_file_is_owner_only_on_unix`, `token_file_is_owner_only_on_unix` (`lifecycle.rs`, mod tests, Unix).

## R6. El directorio de datos es privado (Unix)

`ensure_private_dir(dir)` (`crates/clawg-core/src/private_dir.rs:21`) MUST crear `dir` y cada antecesor que falte con modo 0700, sea cual sea el `umask`. Si `dir` ya existe y no tiene modo privado, MUST quitarle los bits de grupo y de otros, conservar los demás (`tightened`, `private_dir.rs:56`), y registrar un `warn` con la ruta, el modo anterior y el nuevo. MUST NOT cambiar el modo de un antecesor que ya existía ni el de ningún archivo. Si no puede cambiar el modo (el directorio es de otro usuario, o el sistema de archivos no lo admite), MUST registrar un `warn` con la causa y seguir. Si no puede crearlo, MUST devolver el error. En Windows MUST sólo crear lo que falte (`private_dir.rs:49`).

Todo lugar de clawg que crea el directorio de datos o un subdirectorio suyo MUST hacerlo con el ayudante: el de datos en `clawg serve` (`crates/clawg-cli/src/daemon/serve.rs:80`), en el autoarranque del puente antes del daemon (`start_daemon`, `crates/clawg-cli/src/client.rs:1315`), al abrir el store (`MemoryStore::open`, `crates/clawg-core/src/memory/db.rs:186`), en `restore` (`crates/clawg-core/src/memory/backup.rs:365`), al guardar `sharing.json` (`crates/clawg-core/src/memory/sharing.rs:186`) y en el candado de `update` (`crates/clawg-cli/src/update/lock.rs:33`); `logs/` (`client.rs:1190`), `backups/` (`snapshot_into`, `backup.rs:310`), `backups/setup/` (`crates/clawg-cli/src/setup.rs:781`), `workspaces/…` (`crates/clawg-core/src/memory/workspace.rs:90`, `:134`) y `models/…` (`crates/clawg-core/src/memory/model_download.rs:451`, `:669`). `clawg setup` MUST pasar por el ayudante también el directorio de datos antes de copiar un archivo (`setup.rs:779`). Un error al crear el directorio al abrir el store MUST devolverse como `MemoryError::DataDir`.

#### Escenario: una instalación nueva
- GIVEN un home sin directorio de datos y `umask` 022
- WHEN un puente arranca el daemon
- THEN el directorio de datos y `logs/` son 0700, y no se registra que se apretó nada

#### Escenario: una instalación anterior
- GIVEN un directorio de datos 0755 con un `memory.db` poblado
- WHEN arranca el daemon
- THEN el directorio de datos es 0700, el registro dice `0755` y `0700`, y el store devuelve las mismas filas

#### Escenario: un directorio ya privado
- GIVEN un directorio de datos 0700
- WHEN arranca el daemon
- THEN su modo sigue 0700 y no se registra nada

#### Escenario: sólo el directorio pedido
- GIVEN un directorio 0755 que contiene un archivo 0644, dentro de un padre 0755
- WHEN se llama al ayudante sobre él
- THEN el directorio es 0700, y el archivo y el padre conservan su modo

Un descriptor de `memory.lock` que otro usuario abrió mientras el directorio estaba abierto sobrevive hasta que su proceso termina, y mientras tanto el daemon responde `StoreInUse`; después ya no puede volver a abrirlo.

**Tests:** `a_missing_directory_and_its_parents_are_created_private`, `an_open_directory_is_tightened_and_the_rest_left_alone`, `a_private_directory_is_left_as_it_is`, `only_group_and_other_bits_are_taken` (`private_dir.rs`, Unix); `save_creates_a_private_data_directory` (`sharing.rs`), `a_snapshot_directory_is_created_private`, `restore_creates_a_private_data_directory` (`backup.rs`), `opening_a_store_makes_its_directory_private`, `a_data_directory_that_cannot_be_created_is_an_error` (`db.rs`), `workspace_directories_are_created_private` (`workspace.rs`), `the_model_cache_is_created_private` (`model_download.rs`), `the_update_lock_makes_the_data_directory_private` (`update/lock.rs`), `setup_copies_go_in_a_private_directory` (`setup.rs`), `autostart_creates_a_private_data_directory`, `the_serve_log_goes_in_a_private_directory` (`client.rs`); `a_daemon_on_a_fresh_home_leaves_a_private_data_directory`, `an_open_data_directory_is_tightened_and_keeps_its_rows` (`tests/binary.rs`, Unix).

## R7. El socket del perfil admite sólo al mismo uid (Unix)

En Unix, `clawg serve` MUST escuchar en el socket del perfil. Con el candado del store tomado y antes de publicar `daemon.json`, MUST borrar el socket que haya dejado un daemon anterior y hacer `bind` (`bind_socket`, `crates/clawg-cli/src/daemon/socket.rs`); `clear_state` (`crates/clawg-cli/src/daemon/lifecycle.rs`) MUST borrarlo al salir, con el candado todavía tomado. Una ruta cuyo largo en bytes no cabe en `sun_path` (108 en Linux, 104 en los demás Unix, `SUN_PATH_LEN`) MUST impedir el arranque con un error que nombra la ruta y el límite, salvo con `--port`, que sirve sólo por TCP con un `warn`; ninguna otra ubicación se intenta. El listener (`PeerChecked`) MUST pedir el uid del par a cada conexión y cerrar, sin leer un byte, la de otro uid o la que no deja leerlo (`admits_peer`), con un `warn` como mucho una vez por minuto. Sobre el socket corren el mismo router, el mismo guardia (R2) y el mismo token. Un clawg de evaluación que tiene el candado no crea socket.

#### Escenario: otra cuenta con los permisos abiertos
- GIVEN el directorio de datos en 0755 y el socket en 0777
- WHEN otra cuenta conecta al socket
- THEN la conexión se cierra sin respuesta y el daemon registra el rechazo

#### Escenario: un socket huérfano
- GIVEN un daemon terminado con SIGKILL que dejó `clawg.sock`
- WHEN arranca otro `serve`
- THEN arranca y contesta por el socket

**Tests:** `the_same_uid_is_admitted_and_another_is_not`, `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named`, `a_socket_an_earlier_daemon_left_is_replaced`, `the_router_answers_over_the_socket_behind_the_guard` (`daemon/socket.rs`); `clear_state_removes_the_files_and_the_socket_and_tolerates_absence` (`lifecycle.rs`); `a_daemon_without_a_port_serves_status_stop_bridge_and_hooks_over_its_socket`, `an_orphan_socket_left_by_a_kill_does_not_keep_the_next_daemon_out`, `a_socket_path_over_the_limit_is_refused_by_name` (`tests/binary.rs`, Unix). Con dos cuentas reales, en el [verify](../../changes/archive/2026-10-20-local-transport/verify.md).

## R8. TCP al lado del socket

`tcp_plan` (`crates/clawg-cli/src/daemon/serve.rs`) MUST decidir: con `--port N`, `127.0.0.1:N` y un `bind` que falla es fatal; sin `--port` y con socket, el puerto del perfil, y un `bind` que falla MUST registrarse con un `warn` y el daemon MUST seguir sólo por el socket sin publicar `port`; sin socket (Windows, o un clawg anterior), el puerto del perfil, fatal. Los dos servidores MUST terminar con la misma señal y dentro de una sola `HTTP_GRACE`. `daemon.json` MUST llevar `port` siempre que TCP esté encendido y `unix_socket: true` cuando lo esté el socket; los dos campos se leen con valor por defecto, así un archivo anterior se lee sin socket y uno sin `port` es un daemon sin TCP.

#### Escenario: el puerto del perfil tomado por otra cuenta
- GIVEN el daemon de otra cuenta en el puerto del perfil
- WHEN corre `clawg serve` sin `--port`
- THEN arranca, contesta por el socket y `daemon.json` no tiene `port`

**Tests:** `the_tcp_plan_follows_the_port_and_the_socket`, `the_start_line_names_every_listener` (`serve.rs`); `a_record_without_the_socket_mark_reads_as_tcp_and_one_without_a_port_has_none` (`lifecycle.rs`); `a_taken_profile_port_leaves_the_daemon_on_its_socket` (`tests/binary.rs`, Unix).

## R9. El cliente va por el socket y nunca cae a TCP

`endpoint(state, base_dir, tcp_only)` (`crates/clawg-cli/src/client.rs`) MUST dar el socket del perfil cuando `state.unix_socket` es verdadero, en Unix, aunque también haya `port`; `127.0.0.1:port` cuando no; y nada sin ninguno de los dos. `from_data_dir` MUST construir el cliente sobre ese punto, sin proxy, con la base `http://localhost` en el socket. La ruta MUST derivarse del directorio de datos y MUST NOT leerse de `daemon.json`. Un socket publicado que no conecta, falte el archivo o no tenga a nadie detrás, MUST leerse como `NotRunning`, y el cliente MUST NOT probar entonces el puerto.

**Tests:** `the_socket_wins_when_published_and_tcp_is_used_otherwise`, `a_client_reaches_a_daemon_over_its_published_socket`, `a_client_on_a_published_socket_with_nothing_behind_is_not_running` (`client.rs`); `a_bridge_starts_its_daemon_through_the_binary` (`tests/binary.rs`, Unix).
