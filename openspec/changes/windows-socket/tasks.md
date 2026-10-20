# windows-socket — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Un lote, un commit convencional que compila y pasa la suite en Windows (`--target-dir target-test`) y no rompe Linux. Las líneas son las de `develop` en `3465cac`, verificadas otra vez al escribir este archivo.

Reglas que valen para todos los lotes:

- Linux es el CI que no se ve desde acá. Todo lo que sólo se construye en un sistema va detrás de `cfg(windows)` o `cfg(unix)` desde el primer commit; un `#[allow]` para esconder código muerto está prohibido (`tests/conventions.rs`). `SocketError::NotUnicode` y `SocketError::Unsupported` van con `cfg(windows)` desde el lote 1, con sus brazos en `serves_without_socket` (D1): Linux no carga ni prueba casos que no puede producir, y no depende de que `SocketError` siga siendo público para compilar sin `dead_code`. La tabla de `serves_without_socket` se parte igual.
- Ningún lote deja a Windows con el daemon tomando el puerto del perfil de la máquina: mientras `socket_only` (`tests/binary.rs:166`) no ponga `CLAWG_TEST_DEFAULT_PORT=off` en Windows, los escenarios sin `--port` se quedan bajo `#[cfg(unix)]`.
- Antes de empezar: `cargo clean --workspace --target-dir target-test`. Dentro de un lote el directorio se reusa.
- Tests: `cargo test --workspace --target-dir target-test`. Clippy: `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`.

## 1. El contrato común del módulo del socket (D1)

Cubre R7 (tope de ruta, `NotUnicode`) y R11 (la decisión de caer a TCP), sin socket nuevo todavía.

- [x] Tests primero (`daemon/socket/mod.rs`, hoy `socket.rs:118-158`): `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named` multiplataforma, con 108 también en Windows (R7, «ruta demasiado larga»); `a_non_ascii_path_counts_its_utf8_bytes` (R7); la tabla de `serves_without_socket(err, explicit_port)`, partida como en D1: un test común con `TooLong` con y sin `--port`, `Bind` y `Stale`, y uno de Windows con `NotUnicode` y `Unsupported`, con y sin `--port`. `TooLong` y `NotUnicode` caen a TCP sólo con `--port`, `Unsupported` siempre, `Bind` y `Stale` nunca (R11, «otra falla»).
- [x] `git mv daemon/socket.rs daemon/socket/mod.rs`; `unix.rs` con `admits_peer` y `PeerChecked` sin cambios (`socket.rs:20`, `:82-116`); `SUN_PATH_LEN` 108 en Linux y Windows, 104 en el resto (`socket.rs:26-29`); la función pura de bytes por sistema bajo `check_socket_path` (`socket.rs:54-61`); `SocketError::NotUnicode` y `SocketError::Unsupported`, las dos con `cfg(windows)`; `serves_without_socket` y su uso en `bind_profile_socket` (`serve.rs:313-325`).
- [x] `pub mod socket` sin `cfg(unix)` en `daemon/mod.rs:9-10`; en Windows sólo compila lo puro, `bind_socket` y `Listener` siguen siendo de Unix. Como `bind_profile_socket` también sigue siendo de Unix hasta el lote 2, en Windows `serves_without_socket` no tiene uso fuera de los tests: compila sin `dead_code` sólo porque es `pub`, alcanzable desde la raíz del crate (`lib.rs:29`, `daemon/mod.rs:10`), como el resto del módulo. Se queda `pub`, nunca `pub(crate)` ni `pub(super)`.
- Commit: `refactor(cli): split the socket module into a shared contract and one half per system`

## 2. El listener AF_UNIX de Windows (D2, D3, D7)

Cubre R7 (huérfano, apagado), R10 y R11.

- [x] Dependencias de `crates/clawg-cli/Cargo.toml` (D4): `socket2` en `[target.'cfg(windows)'.dependencies]` y `Win32_Networking_WinSock` en la línea de `windows-sys` (`Cargo.toml:62`), porque los usa el producto; `hyper`, `hyper-util` y `http-body-util` en `[target.'cfg(windows)'.dev-dependencies]`, porque en este lote sólo los usa el apretón de manos de `#[cfg(test)]` (pasan a `dependencies` en el lote 4, con el `Connector` y `tower-service`). `cargo update --workspace` y `Cargo.lock` (lista de `clawg-cli`, hoy `:345`, que nombra también las dependencias de desarrollo) en el mismo commit, porque el CI usa `--locked`. `tests/conventions.rs` no necesita cambios: sólo busca `testing` fuera de las tablas que terminan en `dev-dependencies]`.
- [x] Tests primero (`daemon/socket/windows.rs`, Windows):
  - `dropping_the_listener_closes_its_socket`: tras el `Drop`, `connect` da `ConnectionRefused` en menos de 1 s (R7, «el apagado despierta al listener»);
  - el router por AF_UNIX detrás del guardia, con un apretón de manos de hyper que sólo existe en `#[cfg(test)]`: otro `Host` da 403, un `Origin` da 403, `/v1` sin token da 401 (R7);
  - `a_socket_an_earlier_daemon_left_is_replaced` (hoy `socket.rs:151`) multiplataforma (R7, «un socket huérfano»);
  - `a_socket_whose_file_denies_write_data_refuses_the_connect`, con `icacls` y sólo el código de salida: `connect` da 10013 con el deny y entra tras `/remove:d` (R10, «deny de escritura»);
  - `the_socket_file_inherits_the_data_directory_acl` (R10, «el DACL se hereda»);
  - `lacks_af_unix`: 10047 da sí, 10013, 10048 y 10022 dan no (R11, «sin AF_UNIX»).
- [x] `windows.rs`: `Socket::new(Domain::UNIX, …)` con `bind`, `listen(128)`, hilo de std `clawg-accept` con `blocking_send` a un `mpsc(128)`, `AfUnixListener` que implementa el `Listener` de axum, política de errores de `accept` de `axum-0.8.9/src/serve/listener.rs:140-167`, el `Drop` que cierra el canal y se conecta a su ruta, `connect`, `lacks_af_unix`, que decide cuándo `bind_socket` da el `SocketError::Unsupported` del lote 1; `bind_socket` y `type Listener` en `mod.rs`.
- [x] `serve.rs`: `bind_profile_socket` en todos los sistemas, `Unsupported` registra un `warn` y da `None`; fuera los `cfg(unix)` de `:26-27`, `:51-53`, `:163-168`, `:234-245` y `:313`. El `terminated()` de `:388-405` sigue siendo de Unix (SIGTERM). `unix_socket` en `daemon.json` vale `true` en Windows; `clear_state` ya borra el socket (`lifecycle.rs:167-171`).
- [x] Test del cableado (`serve.rs`): `the_tcp_plan_follows_the_port_and_the_socket` con las filas de Windows y `the_start_line_names_every_listener` (R8).
- Nota: el cliente todavía va por `port`, así que `binary.rs` en Windows sigue igual y pasa.
- Commit: `feat(cli): listen on the profile socket on Windows`

## 3. Un embudo para las llamadas del cliente (D4, sólo reqwest)

Refactor sin cambio de conducta, en todos los sistemas, salvo el vencimiento de `/health` mientras llega el cuerpo, que pasa de `Http` a `NotRunning` (D4).

- [x] Tests primero (`client.rs`, módulo `tests`): `read_response` pura sobre `Answer` (401, error con cuerpo, error sin JSON, 2xx inválido); `call_url` comparada con `reqwest::RequestBuilder::query().build()` (consulta sin `?` cuando no hay pares, codificación igual); el clasificador puro de `map_transport` (`client.rs:701-707`: conexión o timeout dan `NotRunning`).
- [x] Antes del refactor, reescribir `only_health_sets_a_timeout` (`client.rs:1569-1576`). Hoy exige que la única línea con `.timeout(` del código que no es de test sea `            .timeout(HEALTH_TIMEOUT)`, dentro de `pub async fn health`, y el refactor le cambia la forma; pasa a exigir que el único `timeout(` del código que no es de test esté dentro de `health`. Va en el mismo commit que el refactor.
- [x] `Call` (sin campo de timeout), `Answer`, `DaemonClient::send` (públicos) y `Transport::Http(reqwest::Client)`; `send` no aplica ningún timeout, y `health` envuelve su llamada en `tokio::time::timeout(HEALTH_TIMEOUT, self.send(call))`, con el vencido como `ClientError::NotRunning`; las 23 llamadas de `DaemonClient` (`client.rs:713-1187`, de `health` a `shutdown`) pasan por `send`; `read_response` (`:684-696`) deja de recibir un `reqwest::Response`; `register_session` (`:835-857`) usa la misma comprobación de estado en lugar de repetirla a mano. Los demás tests por TCP y por el socket de Unix siguen en verde.
- Commit: `refactor(cli): send every daemon call through one function`

## 4. El cliente por el socket en Windows (D4, D5)

Cubre R9 y los escenarios del sello de R9.

- [x] Tests primero (`client.rs`, Windows, fallan por no compilar o por ir a `port`):
  - `the_socket_wins_when_published_and_tcp_is_used_otherwise` (`:1622`) sin la rama `cfg!(unix)` de `:1626` (R9, «cliente por el socket»);
  - `a_client_reaches_a_daemon_over_its_published_socket` (`:1658`) y `serve_on_socket` (`:1637`) multiplataforma (R9);
  - `a_client_on_a_published_socket_with_nothing_behind_is_not_running` (`:1674`), con el archivo huérfano también (R9, «socket publicado sin nadie detrás»);
  - `thirty_two_simultaneous_clients_over_the_socket_are_none_not_running` (R9, «ráfaga de 32»);
  - un `health` contra un socket que nunca contesta da `NotRunning` dentro de `HEALTH_TIMEOUT` (`:48`);
  - `a_stamp_can_restrict_the_binary_to_tcp` (`build_info.rs`, caso de Windows: el sello `tcp` apaga también el socket del daemon y del cliente, R9).
- [x] `Connector` (`tower_service::Service<Uri>` sobre `connect` y `TcpStream::from_std`, `pub(crate)`) en `daemon/socket/windows.rs`; `Transport::AfUnix(hyper_util::client::legacy::Client<…>)` bajo `cfg(windows)`; el bearer como cabecera sensible, `content-type` con cuerpo; la rama no aplica ningún timeout (el de `health` envuelve a `send`, D4). `hyper`, `hyper-util` y `http-body-util` pasan de `[target.'cfg(windows)'.dev-dependencies]` a `[target.'cfg(windows)'.dependencies]`, con la misma versión y las mismas features, y `tower-service` entra allí; `cargo update --workspace` suma `tower-service` a la lista de `clawg-cli` de `Cargo.lock`, y el paso de tabla no la cambia.
- [x] `endpoint` (`client.rs:621-626`) sin `cfg!(unix) &&`; `socket_http` (`:636-644`) deja de ser `None` fuera de Unix; `from_data_dir` (`:655-664`) arma el transporte del socket de cada sistema. Los tests del router de `windows.rs` pasan a usar el `Connector`.
- Commit: `feat(cli): reach the daemon over its socket on Windows`

## 5. `binary.rs` en Windows (D6)

Cubre R7, R8 e install (el autoarranque). Estos tests son los que fallan hoy en Windows por falta de socket.

- [x] `socket_only` (`binary.rs:166-172`) devuelve `CLAWG_TEST_DEFAULT_PORT=off` en todos los sistemas, y su comentario deja de decir que Windows no tiene socket (R8, install). Tiene que ir en este lote, antes de los escenarios sin `--port`.
- [x] `reach` (`:342-352`), el `reqwest::Client` de `Daemon` (`:199`, que crea `Daemon::watch` en `:216` y reemplaza `Daemon::answering` en `:237`), `published_health` (`:1363`) y `published_send` (`:1397`) pasan a `DaemonClient::from_data_dir` y `send`; `published` sigue comprobando antes el pid de `daemon.json`. `Daemon::health`, `post` y `list` (`:250-278`) dejan de usar `self.http` y `self.base`.
- [x] Sin `cfg(unix)`: `serve_as` (`:105`), `bridge_starting_its_daemon` (`:131`), `SERVE_WITHOUT_PORT` y los cinco escenarios de la sección «The data directory's socket» (`:721-833`, que deja de decir «(Unix)»): `a_daemon_without_a_port_serves_status_stop_bridge_and_hooks_over_its_socket` (R7), `an_orphan_socket_left_by_a_kill_does_not_keep_the_next_daemon_out` (R7, «un socket huérfano»), `a_socket_path_over_the_limit_is_refused_by_name` (R7, «una ruta demasiado larga»; en Windows el home cabe en `MAX_PATH`), `a_taken_profile_port_leaves_the_daemon_on_its_socket` (R8, «el puerto del perfil tomado») y `a_bridge_starts_its_daemon_through_the_binary` (install, «autoarranque en Windows»).
- [x] Sin `cfg(unix)`: `OLDER_TRANSPORT` y `an_older_updater_installs_the_socket_version_and_sees_it_answer` y `an_older_bridge_reaches_a_new_daemon_over_the_profile_port` (`:1842-1886`; S8, «un actualizador anterior en Windows»).
- [x] Escribir `a_taken_explicit_port_is_fatal_and_publishes_nothing` en `binary.rs` (R8, «`--port` ocupado»), para los dos sistemas: hoy ese escenario no tiene test en ninguno. Ocupa un puerto N, corre `serve --port N` y espera una salida con error sin `daemon.json` ni `token`, que `run` escribe después del `bind` (`serve.rs:183-197`). No comprueba que falte el socket: `run` lo enlaza (`serve.rs:164`) antes de que el `bind` de TCP vuelva con `?` (`:171`), y ningún `Drop` lo borra. No hace daño: sin `daemon.json` ningún cliente se conecta a él, y el próximo arranque lo borra con el candado tomado.
- [x] El bloque `#[cfg(unix)]` de `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` (`:1832-1836`) se queda como está hasta el lote 6: sin `serve_port` nuevo, Windows pasaría el puerto del viejo.
- Siguen sólo en Unix: el modo del directorio (`:488-526`), el `0600` del export (`:696`), SIGTERM (`:598`), el `chmod` de `write_executable` (`:1097`), los escenarios de `setup` con un `claude` de shell (desde `:1916`), `admits_peer` y su test.
- Commit: `test(cli): run the socket scenarios of the binary on Windows`

## 6. `serve_port` lee la marca del daemon (D5)

Cubre S8, incluido el fallo que ya hay en Unix.

- [x] Tests primero: `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` (`update/effects.rs:496-500`, el segundo argumento pasa a ser `socket` y gana la fila que devuelve el puerto del viejo cuando no publicó su socket); los `Running` de `update/plan.rs:305` y `:451` llevan `socket`; `an_update_keeps_the_port_of_a_daemon_that_serves_tcp_alone` (`binary.rs`, Unix y Windows; falla hoy en Unix porque el daemon nuevo se lanza sin `--port` y se niega a arrancar, `serve.rs:320-324`; S8, «un daemon que sirve sólo TCP porque su ruta no entra», incluido el que arranca al deshacer); la comprobación de `port` de `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` (`:1832-1836`) sin `cfg(unix)` (S8, «un actualizador nuevo en Windows»).
- [x] `serve_port(old_port, socket, tcp_only)` (`effects.rs:295-301`, llamado en `:431` y `:458` con `cfg!(unix)`); `Running.socket: bool` (`plan.rs:11-16`); `running_daemon` (`update/mod.rs:129-136`) lo lee de `daemon.json` junto con el puerto; el comentario de `wait_for_end` vuelve a su función (`effects.rs:287-289`).
- Commit: `fix(cli): keep the port of a daemon that serves TCP alone when updating`

## 7. El apagado despierta al listener (D9)

- [x] Test primero: `a_shutdown_ends_the_socket_listener_within_its_bounds` (`binary.rs`; R7, «el apagado despierta al listener»): `stop` sale con 0 dentro de `STOP_TIMEOUT`, el proceso con 0 y `clawg.sock` desaparece.
- [x] Si falla por el despertar, el ajuste va en `windows.rs`; si pasa de entrada el commit es sólo el test, y la nota «sin verificar» de D2 queda cerrada.
- Commit: `test(cli): pin that a shutdown ends the socket listener on Windows`

## 8. La barra de rendimiento (D8, R12)

- [x] `crates/clawg-cli/tests/transport_bar.rs` nuevo, con `#![cfg(windows)]` e `#[ignore = "…"]`: daemon de `CARGO_BIN_EXE_clawg` con `serve --profile dev --port 0`, `CLAWG_TEST_HOME` y modelo falso; dos `DaemonClient::from_data_dir` (el directorio del daemon y una vista con `token` y un `daemon.json` que sólo lleva `port`); los cinco escenarios (secuencial, cliente nuevo por petición, 8 tareas, ráfaga de 32 con barrera en 20 rondas, y `DaemonClient::list` sobre memorias sembradas con un `Answer.body` de 7 a 9 KB que el arnés comprueba); precalentamiento y 3 corridas alternadas, la mediana; imprime media, p99 y razón; falla si una media de AF_UNIX pasa 1,10 veces la de TCP o la ráfaga de AF_UNIX tiene una falla.
- [ ] El CI no pasa `--ignored` (`.github/workflows/ci.yml:67`), así que no entra a la suite. Se corre en la verificación: `cargo test -p clawg-cli --release --test transport_bar --target-dir target-test -- --ignored --nocapture` (build `testing`, no es un release que se publica) y el resultado va en `verify.md`. Si la barra no se cumple, el cambio no sale.
- Commit: `test(cli): add the transport benchmark harness for Windows`

## 9. Docs

- [ ] `.claude/CLAUDE.md`: el párrafo del socket (`On Unix the daemon listens on…`, `:30`) y la línea de `serve` (`:36`) valen para Unix y Windows (AF_UNIX nativo, el tope de 107 bytes, la ruta no UTF-8, sin AF_UNIX TCP en el puerto del perfil); `daemon.json` y `unix_socket`; «On Windows TCP is the only transport» deja de existir; la línea de `update` (`serve_port` por la marca del socket, no por el sistema); la lista de `binary.rs` en Testing (los escenarios del socket y del sello `tcp` corren en Windows; `CLAWG_TEST_DEFAULT_PORT=off` sin la mención de Unix) y el sello `transport: "tcp"`, que apaga también el socket de Windows; `transport_bar.rs` como arnés `#[ignore]`.
- [ ] `README.md:514` y `:516`: el socket y `clawg.sock` también en Windows.
- [ ] `crates/clawg-core/src/paths.rs:225`: el comentario de `socket_file` deja de decir «Unix».
- [ ] Las specs (`daemon-access` R7 a R9, `self-update` S8, `install`) se tocan al archivar, no acá.
- Commit: `docs: say that the profile socket and its tests also run on Windows`

## 10. Verificación

- [ ] `cargo clean --workspace --target-dir target-test`, `cargo test --workspace --target-dir target-test` y clippy (`-A clippy::all -D warnings`) en Windows sobre el último commit.
- [ ] `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info` (lo que construye el release), y la suite en Linux: el CI del pull request en verde en Linux y Windows. Revisar `dead_code` y `unused` de las variantes y los imports que sólo usa un sistema.
- [ ] Barra de rendimiento del lote 8, con la tabla en `verify.md` (R12).
- [ ] Manual, en `verify.md`, con una segunda cuenta local estándar y el daemon real (R13): conectar al socket da 10013; borrar el socket y crear otro en el directorio de datos da acceso denegado.
- [ ] `rag-eval` no aplica: no cambia el ranking ni lo que ve el modelo; `agent_contract.json` no cambia.
- [ ] Ningún directorio de build fuera de `target` y `target-test`; ningún `clawg.exe` detenido; nada en `~/.clawg*`.

## 11. Archive

- [ ] `specs/daemon-access` (R7 a R9 y R10 a R14), `specs/self-update` (S8, `:122`), `specs/install` (`:112`); `git mv` a `changes/archive/`; `state.yaml`; fila y orden de ejecución del `README.md` de openspec; subir el minor y `cargo update --workspace` en un commit `chore: bump version to X.Y.Z`.
