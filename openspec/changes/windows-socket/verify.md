# windows-socket — Verificación

Modo: Strict TDD (`openspec/config.yaml`). Veredicto local: **pasa con advertencias**, sin hallazgos críticos. El cambio no se archiva hasta que estén en verde las dos secciones pendientes del final: el CI (Linux y Windows) y la prueba manual de R13.

## Windows (2026-10-20)

Sobre `0ecaeb1` (el último commit de código es `b992382`), en `target-test`, con el compilador de `rust-toolchain.toml` (1.99.1), en Windows 11 con una cuenta estándar sin elevar. Después de corregir W1 y W2 (`b47006b`, `8c72eab`, el último commit de código), `cargo test --workspace --target-dir target-test` salió otra vez con 0, con todas las líneas `test result: ok`, y clippy con 0.

- `cargo clean --workspace --target-dir target-test` antes de compilar.
- `cargo test --workspace --target-dir target-test`: sale con 0; cada binario de test y cada doc-test termina en `test result: ok`, con 0 fallas. Sólo quedan ignorados los dos que lo están por diseño: `the_socket_is_not_slower_than_tcp` (`transport_bar.rs`, que corre abajo) y `the_vectors_are_the_ones_fastembed_gave` (`e5_golden.rs`, que necesita el modelo real).
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info --target-dir target-test` pasa (el lado `cfg(not(feature = "testing"))`, `a_stamp_can_restrict_the_binary_to_tcp` incluido).
- `git diff 9f7287f~1 HEAD -- crates/clawg-cli/src/mcp/agent_contract.json` está vacío, y `the_agent_contract_matches_its_file` pasa sin bendecir: nada de lo que ve el modelo cambió.
- En la raíz del repositorio no hay otro directorio de build que `target` y `target-test`, ni al lado del repositorio; `git worktree list` sólo da el principal. Ningún `clawg.exe` se detuvo y nada tocó `~/.clawg*`.

## La barra de rendimiento (R12)

`cargo test -p clawg-cli --release --test transport_bar --target-dir target-test -- --ignored --nocapture`: un build `testing` del arnés, optimizado (2 min 11 s de compilación), no un release que se publica. El daemon es `serve --profile dev --port 0` con el modelo falso; cada fila es la corrida de media mediana de 3 tras un precalentamiento. Una sola corrida: ninguna razón quedó cerca del tope. Corrida de nuevo tras corregir W2 (`8c72eab`): las fallas son la suma del precalentamiento y de las 3 corridas de cada transporte.

| Escenario | AF_UNIX media | AF_UNIX p99 | TCP media | TCP p99 | Razón | Fallas (unix/tcp) |
|---|---|---|---|---|---|---|
| secuencial | 92 µs | 229 µs | 120 µs | 316 µs | 0,77 | 0/0 |
| cliente nuevo por petición | 314 µs | 554 µs | 2912 µs | 4446 µs | 0,11 | 0/0 |
| 8 tareas en paralelo | 125 µs | 273 µs | 155 µs | 612 µs | 0,80 | 0/0 |
| ráfaga de 32, 20 rondas | 1343 µs | 2580 µs | 2147 µs | 3849 µs | 0,63 | 0/0 |
| lista de unos 8 KB | 344 µs | 828 µs | 453 µs | 992 µs | 0,76 | 0/0 |

`test the_socket_is_not_slower_than_tcp ... ok`, en 6,22 s. La barra se cumple: ninguna media de AF_UNIX pasa 1,10 veces la de TCP (la mayor es 0,80) y la ráfaga de AF_UNIX no tuvo fallas en ninguna corrida.

## Cumplimiento por requisito

Cada fila dice si el test afirma lo que pide el escenario (GIVEN/WHEN/THEN), no sólo si existe. Todos los tests nombrados corrieron y pasaron en la corrida de arriba.

| Requisito | Escenario | Test | Qué afirma | Resultado |
|---|---|---|---|---|
| R7 | un socket huérfano | `binary.rs` `an_orphan_socket_left_by_a_kill_does_not_keep_the_next_daemon_out`; `socket/mod.rs` `a_socket_an_earlier_daemon_left_is_replaced` | un daemon sin `--port` matado con `TerminateProcess` deja `clawg.sock`, y el segundo arranca y contesta `/health`. Con `CLAWG_TEST_DEFAULT_PORT=off` no hay TCP, así que contestar es contestar por el socket y que `daemon.json` lleve `unix_socket: true`, aunque esto último no se compara explícitamente | ✅ |
| R7 | una ruta demasiado larga | `binary.rs` `a_socket_path_over_the_limit_is_refused_by_name`; `socket/mod.rs` `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named`, `a_non_ascii_path_counts_its_utf8_bytes`, `a_path_that_is_not_unicode_is_named` | con una ruta de 130 bytes, sin `--port` sale con 1 y nombra la ruta; con `--port 0` sirve, `unix_socket: false` y el log dice `serving over TCP alone`. Los unitarios fijan el tope de 108 bytes UTF-8 y que el error nombra ruta, tope y `--port` | ✅ |
| R7 | el apagado despierta al listener | `binary.rs` `a_shutdown_ends_the_socket_listener_within_its_bounds`; `socket/windows.rs` `dropping_the_listener_closes_its_socket` | `clawg stop` (que manda `POST /v1/shutdown` por el socket) sale con 0 antes de `STOP_TIMEOUT`, el proceso sale con 0 y `clawg.sock` desaparece; el unitario prueba el despertar en sí: tras el `Drop`, `connect` da `ConnectionRefused` en menos de 1 s | ✅ |
| R7 | router, guardia y token sobre AF_UNIX | `socket/windows.rs` `the_router_answers_over_the_socket_behind_the_guard` | por el `Connector`: `Host` `clawg` da 403, un `Origin` da 403, `/v1/stats` sin token da 401, `/health` da 200 | ✅ |
| R8 | el puerto del perfil tomado | `binary.rs` `a_taken_profile_port_leaves_the_daemon_on_its_socket` | con el puerto ocupado, el daemon contesta por el socket, `unix_socket: true`, sin `port`, y el log nombra el puerto | ✅ |
| R8 | `--port` ocupado | `binary.rs` `a_taken_explicit_port_is_fatal_and_publishes_nothing` | sale con 1, nombra el puerto, sin `daemon.json` ni `token` | ✅ |
| R8 | la decisión | `serve.rs` `the_tcp_plan_follows_the_port_and_the_socket`, `the_start_line_names_every_listener` | `tcp_plan` no depende del sistema: sus filas ya cubrían los dos casos y ahora corren en Windows con `has_socket` real; la línea de arranque, con una ruta de Windows | ✅ |
| R9 | cliente por el socket | `client.rs` `a_client_reaches_a_daemon_over_its_published_socket`, `the_socket_wins_when_published_and_tcp_is_used_otherwise` | con `unix_socket: true` y un `port` donde nadie escucha, `health`, `stats` y `search` (con cuerpo) contestan: sólo pudieron ir por el socket | ✅ |
| R9 | socket publicado sin nadie detrás | `client.rs` `a_client_on_a_published_socket_with_nothing_behind_is_not_running` | sin archivo y con un huérfano da `NotRunning`, y el puerto publicado, abierto por otro, no recibió ninguna conexión (`accept` da `WouldBlock`) | ✅ |
| R9 | ráfaga de 32 | `client.rs` `thirty_two_simultaneous_clients_over_the_socket_all_answer`; además, la ráfaga del arnés contra el binario | 32 `DaemonClient` con barrera contra el router servido por el listener AF_UNIX real: las 32 contestan `ok` (más fuerte que «ninguna es `NotRunning`») | ✅ |
| R9 | `/health` que nunca contesta | `client.rs` `a_health_against_a_socket_that_never_answers_is_not_running_within_its_timeout` | da `NotRunning`; el tope del test es 3 × `HEALTH_TIMEOUT`, holgura para el CI | ✅ |
| R9 | el sello `transport: "tcp"` | `binary.rs` `an_older_updater_installs_the_socket_version_and_sees_it_answer`; `client.rs` `the_socket_wins_when_published_and_tcp_is_used_otherwise`; `build_info.rs` `a_stamp_can_restrict_the_binary_to_tcp` | lado del daemon, de punta a punta en Windows: la copia sellada publica `unix_socket: false`. Lado del cliente: la fila `endpoint(.., true)` da TCP con el socket publicado, y `from_data_dir` le pasa `tcp_only()`; ningún escenario del binario lo discrimina (S4) | ✅ |
| R10 | deny de escritura | `socket/windows.rs` `a_socket_whose_file_denies_write_data_refuses_the_connect` | con `icacls /deny <cuenta>:(WD)` sobre el archivo, con el router servido detrás, `connect` da 10013 y `health()` de un `DaemonClient` da `NotRunning`; entra tras `/remove:d` | ✅ |
| R10 | el DACL se hereda | `socket/windows.rs` `the_socket_file_inherits_the_data_directory_acl` | un deny heredable sólo para los hijos (`(OI)(CI)(IO)(WD)`) puesto en el directorio antes del `bind` hace que `connect` dé 10013: la entrada llegó al archivo. Lo prueba por su efecto, no leyendo el DACL | ✅ |
| R11 | sin AF_UNIX | `socket/mod.rs` `a_path_that_is_not_unicode_or_a_system_without_sockets_is_decided_apart`; `socket/windows.rs` `only_an_address_family_without_a_provider_lacks_af_unix`; la fila `Required` de `tcp_plan` | son los tests que pide la spec: `Unsupported` cae a TCP con y sin `--port`, sólo 10047 es `lacks_af_unix`, y sin socket el puerto del perfil es `Required`. No hay un Windows sin AF_UNIX donde probarlo de punta a punta (D3) | ✅ |
| R11 | otra falla | `socket/mod.rs` `only_a_path_that_is_too_long_falls_back_to_tcp_and_only_with_a_port`; `binary.rs` `a_socket_path_over_the_limit_is_refused_by_name` | `Bind` y `Stale` detienen el arranque con y sin `--port`; `bind_profile_socket` corre antes del `bind` de TCP (`serve.rs:161`), así que no se abre TCP; el binario muestra la salida con 1 de un error de socket sin caída | ✅ |
| R12 | la barra | `transport_bar.rs` `the_socket_is_not_slower_than_tcp` | las cinco medias, la mayor razón 0,80; la ráfaga sin fallas en el precalentamiento ni en ninguna de las 3 corridas | ✅ |
| R13 | la otra cuenta | — | manual, pendiente (abajo) | ⏳ |
| S8 | un actualizador nuevo en Windows | `binary.rs` `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` | el viejo corre con `--port 0` y socket; tras `update`, `daemon.json` no tiene `port` (la comprobación ya no es `cfg(unix)`) y el puente abierto alcanza al daemon nuevo | ✅ |
| S8 | un daemon que sirve sólo TCP porque su ruta no entra | `binary.rs` `an_update_keeps_the_port_of_a_daemon_that_serves_tcp_alone`, `an_undone_update_restarts_a_daemon_that_served_tcp_alone_on_its_port`; `effects.rs` `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` | el viejo sin `unix_socket`; el nuevo recibe el mismo puerto y contesta 0.19.0; al deshacer (`serve_fault` `before_open`) el viejo vuelve con ese puerto y contesta 0.18.0 | ✅ |
| S8 | un actualizador anterior en Windows | `binary.rs` `an_older_updater_installs_the_socket_version_and_sees_it_answer` | el actualizador sellado `tcp` sale con 0, el daemon nuevo publica `unix_socket: true` y el mismo `port`; contesta por el socket al test y por TCP al actualizador, que es `tcp_only` y lo vigiló hasta verlo contestar | ✅ |
| install | autoarranque en Windows | `binary.rs` `a_bridge_starts_its_daemon_through_the_binary`; `build_info.rs` `a_default_port_override_is_off_or_a_port` | el puente arranca un daemon que publica `unix_socket: true` y no `port`, y `memory_status` lo alcanza; `socket_only` pone `CLAWG_TEST_DEFAULT_PORT=off` en los dos sistemas | ✅ |

Resumen: 17 de 18 escenarios cumplen y 1 está pendiente (R13), más las filas del router, la decisión de `tcp_plan`, el vencimiento de `/health` y el sello, que cumplen.

## Diseño

Se siguió en todo, sin alternativas descartadas metidas por la ventana:

- D1: `daemon/socket/` con `mod.rs`, `unix.rs` y `windows.rs`; `SUN_PATH_LEN` 108 en Linux y Windows; `NotUnicode` y `Unsupported` con `cfg(windows)`, con sus brazos y su tabla aparte.
- D2: socket2, `listen(128)`, el hilo `clawg-accept` con `blocking_send` a un `mpsc(128)`, la política de errores de axum y el despertar por autoconexión, sin `unsafe`.
- D3: sólo 10047 es `Unsupported`; el resto es `Bind`.
- D4: `Call` sin timeout, `Answer`, `send` y `Transport::AfUnix` con el `Connector`; el único `timeout(` del código está en `health` (`only_health_sets_a_timeout`). Las dependencias están en `[target.'cfg(windows)'.dependencies]` y el lock sólo suma sus nombres a la lista de `clawg-cli`.
- D5: `serve_port(old.port, old.socket, tcp_only)`, con `Running.socket` leído de `daemon.json`.
- D6: `binary.rs` llega por `DaemonClient::from_data_dir` y `send`.
- D7: `icacls`, leyendo sólo el código de salida.
- D8: el arnés, salvo el detalle de W2.
- D9: el orden del apagado.

## RED antes de cada cambio

La apply no dejó una tabla de rojos por tarea: cada lote es un commit con el test y el código juntos. Lo que sigue sale del diff de cada commit, no de una corrida en rojo registrada.

| Lote | Cómo fallaba antes |
|---|---|
| 1 (`9f7287f`) | en Windows el módulo `socket` no compilaba (`#[cfg(unix)] pub mod socket`); `a_non_ascii_path_counts_its_utf8_bytes` y la tabla de `serves_without_socket` no compilaban sin la función |
| 2 (`1bf233d`) | los tests de `socket/windows.rs` no compilaban sin el módulo; en Windows `has_socket` era siempre `false` |
| 3 (`6e79631`) | `read_response` sobre `Answer`, `call_url` y `classify_transport` no compilaban; `only_health_sets_a_timeout` se reescribió en el mismo commit |
| 4 (`8457c46`) | `endpoint` tenía `cfg!(unix) &&`, así que en Windows `the_socket_wins_when_published_and_tcp_is_used_otherwise` daba `Tcp` donde se espera el socket; los tests del cliente por el socket eran `#[cfg(unix)]` |
| 5 (`296ccc5`) | los escenarios del binario se habilitaron en Windows después del código que prueban y no se corrieron en rojo; `a_taken_explicit_port_is_fatal_and_publishes_nothing` fija una conducta que ya existía |
| 6 (`ed1c584`) | en Windows, el daemon nuevo recibía el puerto del viejo (`serve_port(.., cfg!(unix), ..)`), y por eso la comprobación de `port` de la actualización completa era `cfg(unix)`; en Unix, `an_update_keeps_the_port_of_a_daemon_that_serves_tcp_alone` fallaba porque el daemon nuevo arrancaba sin `--port`. Ese rojo es de Unix y nunca se observó: Linux no se compiló acá |
| 7 (`4dfe0d8`) | pasó de entrada, como preveía la tarea: el commit es sólo el test |

## Los puntos a juzgar

- **El test de la ráfaga cambió de nombre.** `thirty_two_simultaneous_clients_over_the_socket_all_answer` corre contra un daemon vivo y exige que las 32 contesten, que es lo que dice el THEN de R9 («las 32 llamadas contestan») y más que «ninguna es `NotRunning`». El nombre de la lista de R9 se corrige al archivar. Sin objeción.
- **El caso de Windows de `a_stamp_can_restrict_the_binary_to_tcp` no se escribió.** El sello se lee igual en todos los sistemas, así que un test por sistema no prueba nada nuevo. La frase de R9 queda cubierta: del lado del daemon, de punta a punta en Windows por `an_older_updater_installs_the_socket_version_and_sees_it_answer`, que exige `unix_socket: false` a la copia sellada; del lado del cliente, por la fila `endpoint(.., true)` y por `bind_profile_socket`, que mira `tcp_only()` antes que nada. Al archivar, la lista de R9 nombra esa cobertura en lugar del test inexistente. La casilla de `tasks.md` (lote 4) está marcada aunque el test no se escribió como dice; se aclara en el mismo paso.
- **La carrera tras el `Drop` del listener.** Un cliente que conecta entre `AfUnixListener::drop` y el cierre del socket por el hilo conecta, y su conexión se suelta sin respuesta: se lee como `Http`, no como `NotRunning`. No viola R9, que pide `NotRunning` para un socket publicado que no conecta, y acá sí conectó. Tampoco viola R7: el listener termina dentro de los plazos, y el unitario lo mide por debajo de 1 s. Es la misma clase de error que recibe una conexión TCP o Unix que estaba en el backlog cuando su listener se cierra; en Windows la ventana dura lo que tarda la autoconexión. Leerla como `NotRunning` sería un error, porque `NotRunning` debe seguir significando que la petición nunca llegó al daemon: es lo que deja al puente repetirla sin duplicar un guardado. Solo pasa durante un apagado ordenado, y la repetición tampoco habría encontrado daemon. Se acepta como está (S3).
- **Los tests de ACL** corrieron y pasaron acá sin elevación. El runner de Windows de GitHub corre elevado, y el deny con un token elevado sigue [sin verificar] (D7): lo dice el CI.
- **Linux** no se compiló acá. Lo revisado a mano: los `cfg` de `SocketError`, `serves_without_socket`, `Connector`, `socket_transport`, `send_over_af_unix` y de los imports de test de `socket/mod.rs`. El CI tiene la última palabra.
- **R13** queda para la persona que lo corra, con una segunda cuenta.

## Hallazgos

**CRITICAL:** ninguno.

**WARNING:**

- **W1 (corregida en `b47006b`: el test ahora afirma `NotRunning` en `health()` con el deny puesto). R10, «deny de escritura»: la lectura `NotRunning` no se prueba con este error.** `a_socket_whose_file_denies_write_data_refuses_the_connect` afirma que `connect` da 10013, pero no pasa por `DaemonClient`. Que ese error llegue como `NotRunning` depende de que todo error del `Connector` sea `is_connect()` en hyper-util, camino que `a_client_on_a_published_socket_with_nothing_behind_is_not_running` sí recorre, con otro error. Basta con un `health()` de un `DaemonClient` sobre el socket con el deny.
- **W2 (corregida en `8c72eab`: las fallas se suman sobre el precalentamiento y las 3 corridas de cada transporte, y la tabla imprime esa suma). El arnés de la barra mira las fallas de la ráfaga sólo en la corrida mediana.** `the_socket_is_not_slower_than_tcp` elige `median(unix_runs)` por media y toma `burst_failures = u.failures` de esa corrida; las fallas de las otras dos corridas medidas no se cuentan, y una petición fallida queda fuera de las muestras. R12 pide que la ráfaga termine sin fallas, y eso se comprueba en una de tres corridas. Se arregla sumando las fallas de todas las corridas de cada transporte. Las medias, que son la barra, no dependen de esto.
- **W3. El rojo no se vio en todos los lotes.** Cada lote se commiteó con su test y su código; el rojo que se corrió antes del código, en Windows, fue:

  | Lote | Rojo observado |
  |---|---|
  | 1 | `cargo test` no compila: no existe la variante `SocketError::NotUnicode` ni `serves_without_socket` |
  | 2 | `cargo test -p clawg-cli --lib socket` no compila: E0425 `bind`, `connect` y `lacks_af_unix`, E0432 `AfUnixListener`. `the_tcp_plan_follows_the_port_and_the_socket` y `the_start_line_names_every_listener` ya pasaban: la decisión no depende del sistema |
  | 3 | `cargo test -p clawg-cli --lib --no-run`: 17 errores (`Answer`, `Call`, `call_url`, `check_status`, `classify_transport`, `read_response`) |
  | 4 | cinco tests de `client::tests` fallan por aserción, porque van al puerto (`left: "http://127.0.0.1:9"`, `right: "http://localhost"`) o porque `from_data_dir` da `None` con el huérfano |
  | 5 | ninguno: los ocho escenarios de `binary.rs` pasaron al quitarles el `cfg(unix)`, porque el socket ya estaba desde los lotes 2 y 4 |
  | 6 | `the_new_daemon_gets_a_port_only_where_tcp_is_the_only_transport` y los `Running` de `plan.rs` no compilan (sin campo `socket`); `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` falla por aserción (`"port":53302,"unix_socket":true`). Los dos escenarios nuevos del daemon que sirve sólo TCP pasan en Windows antes del arreglo, porque Windows ya pasaba el puerto; su rojo es de Unix y no se vio |
  | 7 | ninguno: el apagado ya despertaba al listener (cierra la nota «sin verificar» de D2) |

  Quedan sin rojo visto el lote 5, el 7 y el rojo de Unix del lote 6. Se califica de advertencia y no de crítica: los `verify.md` archivados ya aceptaron escenarios del binario no corridos en rojo (local-transport, memory-portability), y el rojo de Unix del lote 6 no se puede observar sin un Linux: el CI sólo muestra el verde del commit final.

**SUGGESTION:**

- **S1 (aplicada en `spec.md` del cambio y en `tasks.md` §11). La spec, al archivar.** El encabezado dice «R7 a R9, más R10 a R14», R10 termina en «Con una segunda cuenta, R14» y `tasks.md` §11 archiva «R10 a R14», pero la spec sólo define hasta R13, que es la de la segunda cuenta. Además, en la lista de R9 van el nombre nuevo de la ráfaga y la cobertura real del sello. En la de R8, «con las filas de Windows» de `the_tcp_plan_follows_the_port_and_the_socket` no pedía filas nuevas: la función no depende del sistema.
- **S2 (aplicada: `tasks.md` dice ahora cómo quedó cubierto). `tasks.md`.** La casilla del caso de Windows de `a_stamp_can_restrict_the_binary_to_tcp` (lote 4) está marcada sin ese test; el archivo debería decir cómo quedó cubierto.
- **S3. La carrera tras el `Drop`.** No se cambia; a lo sumo, una línea en la documentación del socket de Windows.
- **S4. El sello del lado del cliente no tiene un escenario que lo discrimine.** `an_older_bridge_reaches_a_new_daemon_over_the_profile_port` pasaría también si el puente sellado fuera por el socket, porque el daemon sirve los dos. Un test que lo distinga (por ejemplo, un puente sellado frente a un `daemon.json` con `unix_socket: true` cuyo socket no contesta y un `port` que sí) cerraría el hueco.

## Pendiente: CI (Linux y Windows)

**Sin correr.** `ci.yml` corre en los pull requests y en los push a `main`; el pull request de `develop` a `main` todavía no está abierto. Tiene que dar en verde:

- Linux: la compilación con los `cfg` de este cambio (Linux nunca se compiló acá), clippy con `-D warnings` (sin `dead_code` ni `unused` en lo que sólo usa un sistema) y la suite, con los escenarios del socket de `binary.rs`, que ahora corren en los dos sistemas;
- Windows: la suite en el runner elevado, en especial `a_socket_whose_file_denies_write_data_refuses_the_connect` y `the_socket_file_inherits_the_data_directory_acl` (D7, [sin verificar] con token elevado);
- en los dos: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info`.

| Job | Resultado |
|---|---|
| Linux | pendiente |
| Windows | pendiente |

## Pendiente: R13 manual

**Sin correr.** Necesita una segunda cuenta local estándar, sin privilegios, creada para la prueba y borrada al final. Va con el binario de `target-test/debug` (build `testing`, modelo falso), perfil `dev` y `CLAWG_TEST_HOME` dentro del perfil de la primera cuenta (A), para que el directorio de datos herede el ACL de ese perfil como en una instalación real y no se toque ningún `~/.clawg*`.

| Paso | Esperado | Resultado |
|---|---|---|
| A corre `clawg serve --profile dev` sin `--port` | `daemon.json` con `unix_socket: true`; `<data>\clawg.sock` creado; `clawg status` de A da `running: true` | pendiente |
| B conecta al socket de A (por ejemplo `curl.exe --unix-socket <data>\clawg.sock http://localhost/health`) | falla con 10013 (`WSAEACCES`) | pendiente |
| B borra `<data>\clawg.sock` | acceso denegado; el archivo sigue y A sigue contestando | pendiente |
| B crea un socket en `<data>` | acceso denegado | pendiente |
| `clawg stop` de A | sale con 0; no quedan `clawg.sock` ni `daemon.json` | pendiente |

## Lo que no se midió

- `rag-eval`: no cambian el ranking ni lo que ve el modelo (`agent_contract.json` sin cambios).
- Un Windows sin AF_UNIX (anterior a Windows 10 1803): 10047 queda [sin verificar] fuera de la decisión pura (D3).
- Qué hace un `connect` bloqueante con el backlog lleno; la ráfaga de 32 y la de la exploración, de 512, no llegaron a ese punto.
- Si Windows deja borrar el archivo de un socket que todavía escucha (D9). El orden del apagado lo evita, y por eso `a_shutdown_ends_the_socket_listener_within_its_bounds` sólo detectaría un listener que nunca se cierra si Windows negara ese borrado; el despertar lo prueba `dropping_the_listener_closes_its_socket`.
- macOS: el tope de 104 bytes sigue sin probarse en una máquina.
