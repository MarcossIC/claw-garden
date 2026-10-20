# local-transport — Especificación

## Definiciones

- **Socket del perfil:** `<data>/clawg.sock`, un socket de dominio Unix con ruta (nunca abstracto), derivado del directorio de datos (`socket_file`, `crates/clawg-core/src/paths.rs`).
- **Puerto del perfil:** 27380 (`prod`) o 27381 (`dev`); en un build `testing`, lo que diga `CLAWG_TEST_DEFAULT_PORT`.
- **Clawg anterior:** uno que sólo habla TCP y lee `port` de `daemon.json`; en los tests, una copia del binario sellada con `transport: "tcp"`.

## T1. El listener del socket admite sólo al mismo uid (Unix)

`admits_peer(peer_uid, own_uid)` MUST admitir sólo cuando los dos son iguales. El listener del socket MUST pedir el uid del par a cada conexión aceptada y cerrar, sin leer nada, la que no admite o cuyo uid no puede leer; MUST registrarlo con un `warn` como mucho una vez por minuto. Sobre el socket corren el mismo router, el mismo guardia y el mismo token que sobre TCP.

#### Escenario: el cliente propio por el socket
- GIVEN el router sobre el listener del socket
- WHEN llega `GET /health` con `Host: localhost`
- THEN contesta 200; con `Host: clawg` o con un `Origin`, 403

**Tests:** `the_same_uid_is_admitted_and_another_is_not`, `the_router_answers_over_the_socket_behind_the_guard` (`daemon/socket.rs`, Unix).

## T2. `serve` escucha en el socket (Unix)

Con el candado del store tomado, y antes de publicar `daemon.json`, `serve` MUST borrar un `clawg.sock` que haya quedado y hacer `bind` del socket. Una ruta cuyo largo en bytes no cabe en `sun_path` (108 en Linux, 104 en los demás Unix) MUST fallar al arrancar con un error que nombra la ruta y el límite y dice que `--port` sigue disponible; con `--port`, el daemon MUST servir sólo por TCP con un `warn`. Ninguna otra ubicación se intenta. `clear_state` MUST borrar también el socket. Un clawg anterior no crea socket.

#### Escenario: un socket huérfano
- GIVEN un daemon terminado con SIGKILL que dejó `clawg.sock`
- WHEN arranca otro `serve`
- THEN arranca y contesta por el socket

#### Escenario: una ruta demasiado larga
- GIVEN un home cuya ruta al socket pasa el tope
- WHEN corre `clawg serve` sin `--port`
- THEN sale con 1 y el error nombra la ruta

**Tests:** `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named`, `a_socket_an_earlier_daemon_left_is_replaced` (`daemon/socket.rs`); `clear_state_removes_the_files_and_the_socket_and_tolerates_absence` (`lifecycle.rs`); `a_daemon_without_a_port_serves_status_stop_bridge_and_hooks_over_its_socket`, `an_orphan_socket_left_by_a_kill_does_not_keep_the_next_daemon_out`, `a_socket_path_over_the_limit_is_refused_by_name` (`binary.rs`, Unix).

## T3. TCP en la transición

- Con `--port N` el daemon MUST escuchar también en `127.0.0.1:N` y un `bind` que falla MUST ser fatal, como hoy.
- Sin `--port`, en Unix, MUST intentar además el puerto del perfil; si el `bind` falla MUST registrarlo con un `warn`, seguir sólo por el socket y no publicar `port`. En Windows, o en un clawg anterior, sin socket, el puerto del perfil es fatal como hoy.
- Los dos servidores MUST terminar con la misma señal y dentro de la misma `HTTP_GRACE`.
- `daemon.json` MUST llevar `port` siempre que TCP esté encendido, y `unix_socket: true` cuando el socket lo está.

#### Escenario: el puerto del perfil ocupado
- GIVEN un listener ajeno en el puerto del perfil
- WHEN corre `clawg serve` sin `--port`
- THEN contesta por el socket y `daemon.json` no tiene `port`

**Tests:** `the_tcp_plan_follows_the_port_and_the_socket`, `the_start_line_names_every_listener` (`serve.rs`); `a_record_without_the_socket_mark_reads_as_tcp_and_one_without_a_port_has_none` (`lifecycle.rs`); `a_taken_profile_port_leaves_the_daemon_on_its_socket` (`binary.rs`, Unix).

## T4. El cliente va por el socket y nunca cae a TCP

`endpoint(state, base_dir)` MUST dar el socket del perfil cuando `state.unix_socket` es verdadero (Unix), aunque también haya `port`, y `127.0.0.1:port` cuando no lo es. Un estado sin socket y sin `port` es "no hay daemon". `from_data_dir` MUST construir el cliente sobre ese punto, sin proxy, con la base `http://localhost` en el socket; la ruta MUST NOT leerse de `daemon.json`. Un socket que no conecta MUST leerse como `NotRunning`, como hoy un puerto cerrado.

#### Escenario: autoarranque a través del binario
- GIVEN un home vacío y un puente con autoarranque
- WHEN el puente recibe una llamada
- THEN arranca un daemon que escucha sólo en el socket y la llamada lo alcanza

**Tests:** `the_socket_wins_when_published_and_tcp_is_used_otherwise` (`client.rs`); `a_client_reaches_a_daemon_over_its_published_socket`, `a_client_on_a_published_socket_with_nothing_behind_is_not_running` (`client.rs`, Unix); `a_bridge_starts_its_daemon_through_the_binary` (`binary.rs`, Unix).

## T5. Versiones

- Un clawg anterior MUST seguir leyendo `daemon.json` mientras TCP esté encendido.
- En Unix, `clawg update` MUST lanzar el daemon nuevo, y el viejo al deshacer, sin `--port`, y vigilarlos por `from_data_dir`. En Windows MUST pasar el puerto del daemon viejo.
- Un actualizador anterior pasa `--port`, así que el daemon nuevo enciende TCP y el actualizador lo ve contestar.

#### Escenario: un actualizador anterior
- GIVEN un clawg anterior con su daemon
- WHEN instala la versión nueva
- THEN termina con 0 y el daemon nuevo contesta por el socket y por TCP

#### Escenario: un puente anterior
- GIVEN un daemon nuevo sin `--port` en el puerto del perfil
- WHEN un puente anterior lo llama
- THEN llega por TCP

#### Escenario: un actualizador nuevo
- GIVEN un daemon con TCP en un puerto efímero
- WHEN la versión actual instala la siguiente
- THEN el daemon nuevo no recibió `--port`

**Tests:** `an_older_updater_installs_the_socket_version_and_sees_it_answer`, `an_older_bridge_reaches_a_new_daemon_over_the_profile_port`, `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` (que ahora comprueba que `daemon.json` no tiene `port`) (`binary.rs`, Unix); `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` (`update/effects.rs`).

## T6. Los stand-ins sólo existen en `testing`

`CLAWG_TEST_DEFAULT_PORT` (`off` o un puerto) y el campo `transport` del sello MUST leerse sólo en un build con `testing`; un build sin él usa el puerto del perfil y habla por el socket.

**Tests:** `a_default_port_override_is_off_or_a_port`, `a_stamp_can_restrict_the_binary_to_tcp` y, del lado `cfg(not(feature = "testing"))`, `update_overrides_are_read_only_in_a_test_build` (`build_info.rs`).
