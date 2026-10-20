# windows-socket — Especificación

Delta contra `specs/daemon-access` (R7 a R9, más R10 a R14), `specs/self-update` (S8) y `specs/install` (el autoarranque). Es el gemelo de Windows de local-transport: lo que allí es del socket de Unix vale acá con AF_UNIX nativo. R5 y R6 siguen siendo sólo de Unix.

## Definiciones

- **Socket del perfil:** `<data>/clawg.sock`, ahora también en Windows: un socket AF_UNIX con ruta, derivado del directorio de datos (`socket_file`, `crates/clawg-core/src/paths.rs:231-233`).
- **Puerto del perfil, clawg anterior:** como en local-transport. En Windows, un clawg anterior habla sólo TCP (`endpoint` mira `unix_socket` sólo en Unix, `crates/clawg-cli/src/client.rs:621`).
- **Sistema sin AF_UNIX:** uno cuyo sistema operativo no puede crear un socket AF_UNIX.

## MODIFIED Requirements

### Requirement: R7. El socket del perfil (Unix y Windows)

(Previously: «El socket del perfil admite sólo al mismo uid (Unix)»; hoy el módulo es sólo de Unix, `crates/clawg-cli/src/daemon/mod.rs:9-10`, y fuera de Unix `has_socket` es falso, `crates/clawg-cli/src/daemon/serve.rs:167-168`.)

`clawg serve` MUST escuchar en el socket del perfil también en Windows. Con el candado del store tomado y antes de publicar `daemon.json`, MUST borrar el socket que haya dejado un daemon matado y hacer `bind`; `clear_state` MUST borrarlo al salir. `daemon.json` MUST llevar `unix_socket: true`. Una ruta de más de 107 bytes en UTF-8 (`sun_path` de 108; una ruta que no es UTF-8 también se rechaza) MUST impedir el arranque con un error que nombra la ruta y el límite, salvo con `--port`, que sirve sólo por TCP con un `warn`. Sobre el socket corren el mismo router, el mismo guardia (R2) y el mismo token. Un apagado MUST terminar el listener del socket dentro de los plazos que ya existen (`HTTP_GRACE`, `RUNTIME_TEARDOWN`, `serve.rs:74`, `:79`). El chequeo del uid del par es de Unix; el acceso en Windows lo da R10.

#### Escenario: un socket huérfano en Windows
- GIVEN un daemon terminado a la fuerza que dejó `clawg.sock`
- WHEN arranca otro `serve` sin `--port`
- THEN arranca, contesta por el socket y `daemon.json` lleva `unix_socket: true`

#### Escenario: una ruta demasiado larga en Windows
- GIVEN un home cuya ruta al socket pasa los 107 bytes en UTF-8
- WHEN corre `clawg serve` sin `--port`
- THEN sale con 1 y el error nombra la ruta; con `--port`, sirve sólo por TCP

#### Escenario: el apagado despierta al listener
- GIVEN un daemon sirviendo por el socket
- WHEN llega `POST /v1/shutdown`
- THEN el proceso sale con 0 sin pasar `STOP_TIMEOUT` y el socket se borra

**Tests (Windows, fallan hoy):** `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named` (cuenta bytes UTF-8 con límite 108), `a_socket_an_earlier_daemon_left_is_replaced` (`daemon/socket.rs`, hoy `cfg(unix)`); `clear_state_removes_the_files_and_the_socket_and_tolerates_absence` (`lifecycle.rs`); `a_daemon_without_a_port_serves_status_stop_bridge_and_hooks_over_its_socket`, `an_orphan_socket_left_by_a_kill_does_not_keep_the_next_daemon_out`, `a_socket_path_over_the_limit_is_refused_by_name` (`tests/binary.rs`, sin `cfg(unix)`, `:683-763`); nuevo `a_shutdown_ends_the_socket_listener_within_its_bounds` (`binary.rs`).

### Requirement: R8. TCP al lado del socket

(Previously: «sin socket (Windows, o un clawg anterior), el puerto del perfil, fatal», `openspec/specs/daemon-access/spec.md:135`.)

`tcp_plan` MUST decidir igual en Windows que en Unix: con `--port N`, `127.0.0.1:N` es obligatorio y un `bind` que falla es fatal; sin `--port` y con socket, el puerto del perfil es opcional y un `bind` que falla MUST registrarse con un `warn` y el daemon MUST seguir sólo por el socket sin publicar `port`; con `CLAWG_TEST_DEFAULT_PORT=off`, en un build `testing`, TCP no se enciende. Sólo sin socket (un sistema sin AF_UNIX, o un clawg anterior) el puerto del perfil es fatal. `daemon.json` MUST llevar `port` siempre que TCP esté encendido y `unix_socket: true` cuando lo esté el socket.

#### Escenario: el puerto del perfil tomado en Windows
- GIVEN un listener ajeno en el puerto del perfil
- WHEN corre `clawg serve` sin `--port`
- THEN contesta por el socket y `daemon.json` no tiene `port`

#### Escenario: `--port` ocupado
- GIVEN un listener ajeno en el puerto N
- WHEN corre `clawg serve --port N`
- THEN sale con error y no publica estado

**Tests:** `the_tcp_plan_follows_the_port_and_the_socket` (`serve.rs`, con las filas de Windows: hoy `has_socket` es siempre falso); `the_start_line_names_every_listener`; `a_taken_profile_port_leaves_the_daemon_on_its_socket` (`binary.rs:764`, sin `cfg(unix)`); `socket_only` de `binary.rs` pone `CLAWG_TEST_DEFAULT_PORT=off` también en Windows (`:166-172`).

### Requirement: R9. El cliente va por el socket y nunca cae a TCP (Unix y Windows)

(Previously: «MUST dar el socket ... en Unix»; `endpoint` usa `cfg!(unix)`, `client.rs:621`, y `socket_http` no existe fuera de Unix, `client.rs:640-643`.)

`endpoint(state, base_dir, tcp_only)` MUST dar el socket del perfil cuando `state.unix_socket` es verdadero y no hay `tcp_only`, en Unix y en Windows, aunque también haya `port`. La ruta MUST derivarse del directorio de datos y MUST NOT leerse de `daemon.json`. Un socket publicado que no conecta MUST leerse como `NotRunning` (`map_transport`, `client.rs:700`) y el cliente MUST NOT probar el puerto. Con 32 clientes simultáneos por el socket, ninguno MUST leerse como `NotRunning`. El sello `transport: "tcp"` MUST apagar también el socket de Windows, tanto en el daemon como en el cliente.

#### Escenario: cliente por el socket en Windows
- GIVEN un daemon con `unix_socket: true` y `port`
- WHEN `DaemonClient::from_data_dir` hace una petición
- THEN llega por el socket

#### Escenario: socket publicado sin nadie detrás
- GIVEN `unix_socket: true`, un `port` abierto por otro y ningún daemon
- WHEN el cliente llama
- THEN responde `NotRunning` y no toca el puerto

#### Escenario: ráfaga de 32
- GIVEN un daemon en Windows
- WHEN 32 clientes llaman a la vez por el socket
- THEN las 32 llamadas contestan

**Tests:** `the_socket_wins_when_published_and_tcp_is_used_otherwise` (`client.rs:1605`, la rama `cfg!(unix)`), `a_client_reaches_a_daemon_over_its_published_socket`, `a_client_on_a_published_socket_with_nothing_behind_is_not_running` (`client.rs`, hoy `cfg(unix)`); nuevo `thirty_two_simultaneous_clients_over_the_socket_are_none_not_running`; `a_bridge_starts_its_daemon_through_the_binary` (`binary.rs:779`); `a_stamp_can_restrict_the_binary_to_tcp` (`build_info.rs`, caso de Windows).

## ADDED Requirements

### Requirement: R10. El archivo del socket admite sólo a quien el ACL del perfil admite (Windows)

El socket MUST heredar el DACL del directorio de datos al crearse; no se le aplica un ACL propio ni se compara el SID del par. Un `connect` de una cuenta sin `FILE_WRITE_DATA` sobre el archivo MUST fallar (10013) antes de aceptar la conexión.

#### Escenario: deny de escritura
- GIVEN un daemon y un deny de `FILE_WRITE_DATA` para la propia cuenta sobre `clawg.sock`
- WHEN el cliente conecta
- THEN `connect` falla y se lee `NotRunning`

#### Escenario: el DACL se hereda
- GIVEN un directorio de datos con un DACL dado
- WHEN el daemon crea el socket
- THEN el DACL del archivo contiene las entradas heredadas del directorio

**Tests (fallan hoy: no hay socket):** nuevos `a_socket_whose_file_denies_write_data_refuses_the_connect`, `the_socket_file_inherits_the_data_directory_acl` (`daemon/socket.rs`, Windows). Con una segunda cuenta, R14.

### Requirement: R11. Sin AF_UNIX, TCP en el puerto del perfil

En un sistema sin AF_UNIX el daemon MUST servir TCP en el puerto del perfil (obligatorio, fatal si falla) y MUST NOT publicar `unix_socket`. Cualquier otra falla del socket MUST detener el arranque con un error que nombra la ruta; MUST NOT caer a TCP.

#### Escenario: sin AF_UNIX
- GIVEN un sistema sin AF_UNIX
- WHEN corre `clawg serve` sin `--port`
- THEN sirve en el puerto del perfil y `daemon.json` no lleva `unix_socket`

#### Escenario: otra falla
- GIVEN un `bind` del socket que falla por otra causa
- WHEN corre `clawg serve`
- THEN sale con 1 y no abre TCP

**Tests:** nuevos `a_system_without_af_unix_serves_tcp_on_the_profile_port`, `any_other_socket_failure_stops_the_start` (decisión pura, `serve.rs`).

### Requirement: R12. El rendimiento de AF_UNIX no empeora el de TCP (Windows, sólo verify)

Con un build `testing` y un daemon arrancado con `--port`, `DaemonClient` por AF_UNIX MUST NOT superar la media de `DaemonClient` por TCP en más de un 10 % en ninguno de cuatro escenarios: secuencial con conexión reutilizada, conexión nueva por petición, 8 en paralelo y ráfaga de 32 clientes (mediana de 3 corridas tras un precalentamiento). La ráfaga MUST terminar sin fallas. Si no se cumple, el cambio MUST NOT integrarse.

#### Escenario: la barra
- GIVEN el daemon con los dos transportes
- WHEN se miden los cuatro escenarios
- THEN ninguna media de AF_UNIX pasa 1,10 veces la de TCP y la ráfaga no falla

**Verificación:** sólo en `verify.md`; no tiene test automático.

### Requirement: R13. Una segunda cuenta no alcanza ni reemplaza el socket (Windows, manual)

Con una segunda cuenta local estándar y el daemon real, conectar al socket MUST dar 10013; borrar el socket y crear otro en el directorio de datos MUST dar acceso denegado.

#### Escenario: la otra cuenta
- GIVEN el daemon corriendo y una cuenta estándar distinta
- WHEN conecta, borra el socket o crea otro en el directorio de datos
- THEN recibe 10013 la primera y acceso denegado las otras dos

**Verificación:** manual, en `verify.md`; no se automatiza (el CI no tiene dos cuentas).

## MODIFIED Requirements (self-update)

### Requirement: S8. Plan e intercambio, el puerto del daemon nuevo

(Previously: «En Windows, único transporte TCP, MUST recibir el puerto del daemon viejo», `openspec/specs/self-update/spec.md:122`; `serve_port` recibe `cfg!(unix)`, `crates/clawg-cli/src/update/effects.rs:295`, `:431`, `:458`.)

En Unix y en Windows el daemon nuevo, y el viejo al deshacer, MUST lanzarse sin `--port` y vigilarse por `from_data_dir`, que va por el socket; sólo con `tcp_only` (un clawg anterior al socket) recibe el puerto del viejo. Un actualizador anterior pasa `--port`, así que el daemon nuevo enciende TCP (obligatorio) y el socket, y ese actualizador lo ve contestar.

#### Escenario: un actualizador nuevo en Windows
- GIVEN un daemon con TCP en un puerto efímero
- WHEN la versión actual instala la siguiente
- THEN el daemon nuevo no recibió `--port` y `daemon.json` no tiene `port`

#### Escenario: un actualizador anterior en Windows
- GIVEN un clawg anterior (sello `tcp`) con su daemon
- WHEN instala la versión nueva
- THEN termina con 0 y el daemon nuevo contesta por el socket y por TCP

**Tests:** `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` (`effects.rs`, cambia en Windows); `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` (la comprobación de `port` sin `cfg(unix)`, `binary.rs:1792-1796`); `an_older_updater_installs_the_socket_version_and_sees_it_answer`, `an_older_bridge_reaches_a_new_daemon_over_the_profile_port` (`binary.rs:1806-1846`, sin `cfg(unix)`).

## MODIFIED Requirements (install)

### Requirement: el autoarranque se prueba a través del binario

(Previously: «En Windows no hay socket y las pruebas pasan un puerto», `openspec/specs/install/spec.md:112`.)

Un daemon que la suite no lanza con `--port` MUST NOT tomar el puerto del perfil de la máquina: cada hijo lleva `CLAWG_TEST_DEFAULT_PORT=off` en Unix y en Windows, y escucha sólo en el socket de su home. La línea sobre Windows se elimina.

#### Escenario: autoarranque en Windows
- GIVEN un home vacío
- WHEN un puente con autoarranque recibe una llamada
- THEN arranca un daemon que escucha sólo en su socket y la llamada lo alcanza

**Tests:** `a_bridge_starts_its_daemon_through_the_binary`, `a_daemon_without_a_port_serves_status_stop_bridge_and_hooks_over_its_socket` (Windows); `a_default_port_override_is_off_or_a_port` (`build_info.rs`).
