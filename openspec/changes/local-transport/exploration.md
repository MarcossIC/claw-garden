# local-transport — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | local-isolation (50), parte 1 (el directorio de datos en 0700); después de shutdown-budget (55) | exploración (borrador) |

**Tesis:** en Linux y macOS el daemon escucha en un socket de dominio Unix dentro del directorio de datos (`<data>/clawg.sock`) en lugar del puerto TCP fijo del perfil. Con eso otro usuario de la máquina no puede conectarse ni hacerse pasar por el daemon, porque para crear el socket hay que poder escribir en el directorio de datos, y dos usuarios dejan de chocar en el mismo puerto. El router no cambia: el guardia, el token y las rutas corren igual sobre cualquier listener. Cambian el listener, el descubrimiento (`daemon.json` dice que hay socket) y el constructor del cliente, que ya es un solo punto (`from_data_dir`). Lo difícil no es el socket sino la compatibilidad entre versiones: el `clawg update` que instala la versión nueva es el binario anterior, que lanza el daemon nuevo con `--port` y lo vigila por TCP, y un puente abierto en el editor también es anterior. Por eso el cambio sale en dos versiones: primero socket y TCP a la vez, y una minor después TCP sólo con `--port` explícito. Windows queda en windows-pipe (59).

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| Todos los clientes de producción construyen el cliente con `from_data_dir` | **Cierta.** El puente (`probe_running`, `client.rs:1220`; `start_daemon`, `:1252`; la reconexión de `mcp/startup.rs`), los hooks (`hook.rs:154`), `clawg status` (`main.rs:128-131`), `clawg stop` (`stop.rs:24-27`), los visores y `clawg update` (`update/mod.rs:129`, `update/effects.rs:272`). Ninguno arma la URL: `from_data_dir` fija `http://127.0.0.1:{port}` e ignora el host del archivo (`client.rs:599-610`). Las llamadas a `DaemonClient::new` son de tests y del arnés de `rag-eval`. |
| El router no depende de TCP | **Cierta.** `axum::serve` es genérico sobre el trait `Listener` (`axum-0.8.9/src/serve/listener.rs:9-24`), implementado para `TcpListener` (`:26`) y para `tokio::net::UnixListener` (`:46`); el router no usa `ConnectInfo`. El guardia vive dentro de `router` (`crates/clawg-cli/src/daemon/api.rs:792-823`). La razón con la que daemon-hardening (38, opción D) descartó los sockets, «la API HTTP deja de ser reutilizable tal cual», no vale para este stack. |
| reqwest habla por un socket Unix sin dependencias nuevas | **Cierta.** `ClientBuilder::unix_socket` (`reqwest-0.12.28/src/async_impl/client.rs:1809`, `#[cfg(unix)]`, desde 0.12.23) no pide ninguna feature más; con él no se usa proxy (`connect.rs:113-116`) ni DNS. tokio ya tiene `net` en `clawg-cli` (`Cargo.toml:41`). |
| El guardia de `Host` deja pasar al cliente sobre el socket | **Cierta en un prototipo fuera del repo** (Linux, mismas versiones de `Cargo.lock`, el `guard.rs` real incluido por `#[path]`): `http://localhost/health` y `http://127.0.0.1/health` dan 200; `http://clawg/health` y cualquier `Origin` dan 403. El cliente usa `http://localhost`. |
| Otro usuario no puede crear el socket ni conectarse | **Cierta en el prototipo, con dos cuentas sin privilegios.** Con el directorio en 0755 o en 0700, la segunda cuenta no pudo crear un socket en él (`Permission denied`); con el socket en 0755 no pudo conectar; con el socket forzado a 0777 y el directorio en 0755 conectó y el listener la descartó por uid (`Connection reset by peer`). Esa misma cuenta **sí** pudo hacer `bind` en `127.0.0.1:27380` y en el socket abstracto `@clawg-prod`. `unix(7)` advierte que el permiso sobre el socket no es portable: la garantía es el directorio en 0700 (parte 1 de 50). |
| El daemon toma el candado antes de abrir el listener | **Cierta.** `MemoryRuntime::open` (`serve.rs:92`) antes del `bind` (`:142`); `token` y `daemon.json` se publican después (`:150-163`). Es lo que hace seguro borrar un socket huérfano: con el candado tomado, ningún otro daemon vivo lo está sirviendo. |
| Un socket huérfano impide el `bind` | **Cierta en el prototipo.** Tras un SIGKILL el archivo queda y el `bind` da `AddrInUse`. Conectarse a él da `ECONNREFUSED`, que reqwest marca `is_connect()`, o sea `NotRunning` (`map_transport`, `client.rs:644`): el cliente autoarranca. |
| La ruta del socket tiene tope | **Cierta.** 108 bytes de `sun_path` en Linux (`unix(7)`); 104 en macOS, sin verificar. En el prototipo, una ruta de 151 bytes dio `InvalidInput: path must be shorter than SUN_LEN`. |
| Un clawg anterior lee un `daemon.json` con un campo nuevo | **Cierta mientras siga llevando `port`.** `DaemonState` (`crates/clawg-cli/src/daemon/lifecycle.rs:38-44`) no tiene `deny_unknown_fields`, así que ignora el campo nuevo, pero `port: u16` no tiene `serde(default)`: sin él, `read_state` (`:131`) devuelve `None` y ese cliente cree que no hay daemon. |
| `clawg update` lo corre el binario instalado, o sea la versión anterior | **Cierta.** Lee el puerto del daemon que corre (`running_daemon`, `update/mod.rs:129`; `Running.port`, `update/plan.rs:11-15`), lanza el binario nuevo con ese puerto (`update/effects.rs:163`, `:341`) y espera con `ask_health` (`:272`), que va por `from_data_dir`; `judge` (`plan.rs:202`) sólo da `Ready` cuando contesta la versión nueva. |
| Al deshacer, `update` vuelve a un binario anterior al socket | **Falsa.** `Undo::StartOldDaemon` arranca `self.paths.exe` después de `RestoreAside` (`update/effects.rs:358-365`), o sea la versión instalada, que es la del propio actualizador. Un actualizador que conoce el socket nunca vuelve a un binario que no lo conoce, así que no necesita `--port` para deshacer. |
| El puerto fijo por perfil choca entre dos usuarios | **Cierta por código, sin correr con dos daemons.** El defecto es 27380/27381 (`crates/clawg-core/src/paths.rs:139-144`) y el autoarranque no pasa puerto (`serve_args`, `client.rs:1112`). El daemon del segundo usuario toma el candado de su store y falla en el `bind` (`ServeError::Bind`, `serve.rs:142-144`); su puente espera `AUTOSTART_TIMEOUT` = 60 s (`client.rs:41`). Con un `daemon.json` viejo, `probe_running` acepta el `/health` del daemon del otro usuario y le manda el token: recibe 401, relee y falla. |
| El autoarranque no se puede probar a través del binario | **Cierta.** Es el «Límite conocido» de `openspec/specs/install/spec.md`: el daemon que lanza el puente usa el puerto fijo del perfil y chocaría con el daemon dev de la máquina. Con un socket por directorio de datos, cada `CLAWG_TEST_HOME` tiene el suyo. |
| La latencia decide | **Falsa.** En el prototipo, `/health` con el cliente reutilizado (el puente): p50 de 110–123 µs por el socket contra 138–150 µs por TCP; con un cliente nuevo por petición (un hook), 327–345 contra 408–418 µs; un POST de 64 KiB, 247–256 contra 249–268 µs. Son decenas de microsegundos contra los milisegundos de una búsqueda: no es un argumento ni a favor ni en contra. |

## Problema (verificado contra el código)

1. **Otro usuario se hace pasar por el daemon.** Cuando el daemon muere sin apagarse quedan `daemon.json` y `token`; cualquier cuenta puede abrir el puerto del perfil, que es público. El siguiente cliente le manda el token y cada save, y lee sus respuestas como memorias (la secuencia completa está en la exploración de local-isolation, que proponía detectar al impostor comparando pids). Sobre un socket en un directorio privado, el impostor no puede ni crear el punto de conexión.
2. **`/health` es legible por cualquier cuenta.** Nombra agentes, operaciones y causas de fallo. Sobre el socket, otra cuenta no conecta.
3. **Dos usuarios no pueden usar el mismo perfil a la vez** en una máquina compartida (sección anterior).
4. **Conexiones a medio abrir de otro usuario.** `axum::serve` corre sin `Timer` (backlog «El daemon no pone tope a una conexión»); sobre el socket sólo las puede abrir el mismo usuario, que ya lee el token.
5. **El autoarranque no se prueba por el binario**, por el puerto fijo.

## Lo que ya existe y se reutiliza

- `from_data_dir` como único constructor de producción: la migración del cliente es un cambio en un lugar. Su comentario ya explica por qué no se confía en lo que dice el archivo sobre a dónde conectarse.
- El orden candado → `bind` → publicación de `serve.rs` y `clear_state` (`lifecycle.rs:156`), que se ejecuta en toda salida ordenada.
- `create_owner_only` (`lifecycle.rs:94-112`) para lo que se crea con modo restringido.
- `spawn_serve_at(…, None)` (`client.rs:1124`): el autoarranque ya lanza sin `--port`.
- El sello de `testing` (`crates/clawg-core/src/build_info.rs`) y su campo `migrations_up_to`, que hace que una copia del binario se comporte como una versión anterior: el modelo para emular un clawg que sólo habla TCP.
- `tests/binary.rs` (home temporal, modelo falso, cada hijo terminado por su handle) y los escenarios de `update` sobre copias selladas.
- `StoreHolder::Eval`: un clawg de evaluación que tiene el candado no crea socket, así que tampoco lo pisa.

## Enfoques posibles

### El listener

**A. `UnixListener` en `<data>/clawg.sock` dentro de un `Listener` propio que compara el uid del par con el propio (recomendado).** `UnixStream::peer_cred()` (tokio 1.51.0, `src/net/unix/stream.rs:939`, implementado para Linux y macOS en `ucred.rs:35-72`) y una función pura `admits_peer(peer_uid, own_uid)`; el listener descarta la conexión ajena antes de que hyper lea nada. Son unas 30 líneas en el prototipo. Con el directorio en 0700 sólo llegan el mismo uid y root, que leen `token` de todos modos; el chequeo es defensa en profundidad para un directorio que no esté en el modo esperado o un sistema que ignore el permiso del socket.

**B. `UnixListener` sin chequeo del par.** Confía sólo en el directorio. Más simple, pero queda abierto si el directorio no es 0700.

### El descubrimiento

`DaemonState` gana un campo con `serde(default)` que dice que el daemon escucha en el socket, y una función pura `endpoint(state, base_dir) -> Endpoint` (`Unix(path)` o `Tcp(port)`) elige: si el daemon publicó el socket, el cliente va por el socket y **nunca cae a TCP** (caer reabriría la suplantación); si no lo publicó (un daemon anterior), va por `Tcp(port)`. `from_data_dir` arma el `reqwest::Client` con `unix_socket` y la base `http://localhost`. La ruta la deriva el cliente del directorio de datos (`<data>/clawg.sock`); no la lee del archivo, por la misma razón por la que hoy ignora el host.

### Las versiones

Un daemon que sólo escucha en el socket deja sin daemon a dos clientes que son anteriores por construcción: el actualizador instalado y el puente abierto. Diseño que lo respeta:

1. **Regla permanente:** `--port` explícito enciende TCP además del socket, y `daemon.json` lleva `port`. Así cualquier actualizador anterior, de cualquier versión, puede instalar y vigilar a cualquier versión nueva.
2. **Versión de transición:** sin `--port`, el daemon escucha en el socket **y** en el puerto del perfil, y publica los dos; los clientes nuevos van por el socket y los anteriores por TCP. **El `bind` del puerto por defecto no es fatal en esta versión:** si falla (otro usuario, otro programa en el puerto), el daemon lo registra, sigue sólo con el socket y no publica `port`. Si fuera fatal, el choque entre usuarios seguiría abierto durante toda la transición. Con `--port` explícito, fallar sí es fatal, como hoy.
3. **Una minor después:** sin `--port`, sólo socket; `port` sale de `daemon.json`. Un puente anterior que siga abierto falla como hoy cuando no hay daemon, hasta que se reinicie el editor; los clientes de vida corta (hooks, CLI) ya son nuevos. Al arrancar con TCP, un `warn` dice que está abierto y por qué. Para depurar en Unix no hace falta TCP: `curl --unix-socket <data>/clawg.sock http://localhost/health` (curl 7.40.0 o posterior).

**`update` y TCP.** Todo actualizador anterior pasa `--port` (`effects.rs:163`, `:341`), así que con la regla 1 cada daemon que lance tendrá TCP encendido. Si además el actualizador nuevo copiara el puerto del daemon viejo al nuevo, TCP quedaría encendido de actualización en actualización, sin que nadie lo haya pedido. Opciones:

- **(i) El actualizador nuevo nunca pasa `--port` (recomendado).** El daemon nuevo arranca igual que en el autoarranque, y lo vigila por el socket. Quien quiera TCP lo pide otra vez con `clawg serve --port`. Deshacer tampoco lo necesita (premisa falsa de arriba).
- **(ii) Lo pasa si el daemon viejo tenía TCP pedido a mano**, registrado en `daemon.json`. No distingue a un usuario de un actualizador anterior que lo pasó: TCP se arrastra igual.
- **(iii) Lo pasa si el puerto difiere del defecto del perfil.** Heurística: conserva un puerto elegido a mano, pierde un `--port 27380` explícito.

### El socket huérfano y la ruta larga

Borrar `<data>/clawg.sock` antes del `bind`, siempre después de `MemoryRuntime::open`, y borrarlo también en `clear_state`. Si la ruta pasa el tope de `sun_path`, `serve` falla al arrancar con un error que nombra la ruta y el límite, y dice que `--port` sigue disponible; no se inventa otra ubicación, porque el cliente no la encontraría.

### El token

Se conserva en todos los transportes. Sobre el socket no agrega seguridad (sólo el mismo usuario y root llegan, y leen `token`), pero mantiene un solo camino de autenticación y los clientes anteriores lo mandan igual.

## Descartado

| Opción | Razón técnica |
|---|---|
| Sockets abstractos de Linux (`@nombre`) | `unix(7)`: «Socket permissions have no meaning for abstract sockets». En el prototipo, otra cuenta ocupó `@clawg-prod` sin ningún permiso: reabren la suplantación. |
| Caer a TCP cuando el socket falla | Un proceso de otro usuario en el puerto vuelve a ser el daemon. |
| TCP por defecto «por las dudas» | Devuelve la suplantación, el `/health` abierto y el choque de puertos. |
| Quitar el token sobre el socket | No agrega seguridad, pero sacarlo bifurca la autenticación por transporte y rompe a los clientes anteriores. Por ahora no. |
| Leer la ruta del socket de `daemon.json` | Un archivo alterado mandaría el token a otro socket; es la razón por la que `from_data_dir` ya ignora el host. |
| Otra ubicación del socket cuando la ruta es larga | El cliente no la encontraría. |
| Pasar el arnés de `rag-eval` y `tests/api.rs` al socket | `serve_ephemeral` (`api.rs:833-847`) corre en proceso, sin estado en disco; no gana nada. |
| Un puerto efímero en el autoarranque | Ya descartado en local-isolation (B4); con el socket deja de importar. |

## Interacciones

- **local-isolation (50).** Su parte 1 (`ensure_private_dir`) es requisito: la garantía portable del socket es el directorio en 0700. La comparación de pid, el desafío HMAC y la división de `/health` salieron de 50 porque este cambio cierra la suplantación y el `/health` abierto en Unix, y windows-pipe (59) en Windows.
- **shutdown-budget (55).** Es el dueño del SIGTERM. Este cambio va después para no tocar dos veces `shutdown_signal`, `clear_state` y los plazos de `start_and_wait`; cuando entre, el test de SIGTERM de 55 suma que no quede `<data>/clawg.sock`. Con dos listeners, los dos `axum::serve` comparten la señal de apagado y tienen que terminar dentro del presupuesto de 55.
- **windows-pipe (59).** Comparte `endpoint`, el campo de `daemon.json` y la regla de versiones. Hasta 59, Windows sigue en TCP.
- **release-and-self-update (47, archivado).** La regla del `--port` explícito y lo que hace `update` con TCP se escriben en `openspec/specs/self-update/spec.md`.
- **doctor-checks (31).** Puede informar el modo del directorio y el largo de la ruta del socket frente al tope.
- **Backlog «El daemon no pone tope a una conexión».** Con este cambio sólo aplica al modo TCP y al mismo usuario; baja de prioridad.
- **El sandbox de Bash de Claude Code.** Cubre sólo los comandos de shell; los servidores MCP y los hooks corren fuera de él (documentación de Claude Code, *Configure the sandboxed Bash tool*, consultada 2026-10-19), así que el puente y los hooks no cambian. Lo que cambia es un `clawg memories` que el agente corra por Bash dentro del sandbox: hoy en Linux no llega al daemon, porque el `localhost` del comando es privado y `allowLocalBinding` no tiene efecto ahí, y en macOS necesita `allowLocalBinding`, que abre todo el `localhost` de la máquina. Con el socket, en macOS alcanza `network.allowUnixSockets` con su ruta, que la documentación describe sólo para macOS; en Linux los sockets Unix sólo se bloquean con el filtro seccomp opcional, y para abrirlos no hay una regla por ruta (`allowAllUnixSockets` los abre todos). Nada de esto se probó.
- **Specs y documentación a reescribir al archivar:** `specs/daemon-access` (el párrafo `Límite`, R3 «los clientes de hoy pasan», R5 y requisitos nuevos: el socket con chequeo de uid, el huérfano borrado sólo con el candado, TCP sólo con `--port`), `specs/install` (sale el «Límite conocido»), `specs/mcp` (la reconexión habla de «token nuevo, puerto nuevo», `openspec/specs/mcp/spec.md:116`), `specs/self-update`, `.claude/CLAUDE.md` (perfiles, `serve --port`, la API) y `README.md` (`:456`, `:486`).

## Alcance candidato

1. `admits_peer` pura y el `Listener` con chequeo de uid (`#[cfg(unix)]`).
2. `serve`: borrar el huérfano con el candado tomado, `bind` del socket, TCP según la regla de versiones (en la transición, el puerto por defecto no fatal), los dos `axum::serve` con la misma señal, el socket borrado en `clear_state`, el error de ruta larga.
3. `DaemonState` con el campo nuevo (`serde(default)`) y `port` escrito siempre que TCP esté encendido.
4. `endpoint` pura y `from_data_dir` con `unix_socket` y `http://localhost`, sin caída a TCP.
5. `update`: `Running.port` opcional y el daemon nuevo arrancado según la decisión de arriba.
6. Un campo nuevo del sello de `testing` (por ejemplo `transport: "tcp"`) que hace que una copia del binario se comporte como un clawg anterior: sólo TCP y `daemon.json` sin el campo nuevo.
7. Escenarios de `binary.rs` (sección siguiente). Los ayudantes que hoy leen `port` de `daemon.json` (`answering`, `crates/clawg-cli/tests/binary.rs:188-208`; `:303`; `published_health`, `:1026`; `published_send`, `:1060`) necesitan una variante por el socket; los escenarios que pasan `--port 0` siguen en TCP sin cambios.
8. Specs y documentación (Interacciones).
9. La minor siguiente: TCP sólo con `--port`, `port` fuera de `daemon.json` cuando TCP está apagado, el `warn` al arrancar con TCP.

### No-alcance

- Windows (59).
- WSL: un Claude Code dentro de WSL corre el clawg de Linux con su propio directorio de datos; no hay un puente de un lado contra el daemon del otro.
- Quitar el token.
- Los tests en proceso y `rag-eval`, que siguen en TCP efímero.
- macOS en CI (no hay runner); su `getpeereid` lo implementa tokio y queda sin ejecutar.

## Decisiones abiertas

Son del usuario.

- [ ] **Chequeo del uid en el listener (A) o sólo el directorio (B).** Recomendado A.
- [ ] **Qué publica `daemon.json`.** Una marca de que hay socket, con la ruta derivada del directorio (recomendado), o la ruta misma.
- [ ] **`update` y TCP.** (i) nunca pasar `--port` al daemon nuevo (recomendado), (ii) conservarlo si se pidió a mano, o (iii) conservarlo si difiere del defecto.
- [ ] **El puerto por defecto en la versión de transición.** `bind` no fatal (recomendado) o fatal como hoy.
- [ ] **Cuándo sale la versión que apaga TCP por defecto.** Una minor después (recomendado) o más tarde.
- [ ] **Probar el autoarranque por el binario durante la transición.** En esa versión el daemon sin `--port` también toma el puerto del perfil, así que un test que autoarranca chocaría con un daemon dev de la máquina. O un stand-in de `testing` que apague el TCP por defecto, o el test llega recién con la segunda versión.
- [ ] **Si `clawg status` muestra el transporte.**

## Cómo se mide

Todo con el modelo falso, sin red y sin perfiles reales.

1. **Pura:** `admits_peer` acepta el uid propio y rechaza otro; `endpoint` da `Unix` cuando el estado publica el socket, aunque también tenga `port`, y `Tcp(port)` cuando no lo publica.
2. **Unix, en proceso:** el router sobre el listener con chequeo de uid contesta 200 a `/health` por `http://localhost` y 403 con `Host: clawg` o con un `Origin`.
3. **`binary.rs` (Unix):** `serve` sin `--port` crea `<data>/clawg.sock`; `status`, `stop`, el puente por stdio y los hooks llegan por el socket; `stop` lo borra.
4. **`binary.rs`:** el autoarranque del puente a través del binario (hoy no se puede probar; ver la decisión abierta).
5. **`binary.rs`:** un socket huérfano tras matar al hijo por su handle; `serve` arranca y contesta.
6. **Ruta larga:** `serve` con un home cuya ruta pasa el tope falla con un error que nombra la ruta.
7. **Versiones, con el sello `transport: "tcp"`:** un actualizador «anterior» instala la versión con socket y termina en `Ready`; un puente «anterior» abierto sigue llegando al daemon nuevo en la transición; y con la decisión (i), un actualizador nuevo deja al daemon nuevo sin TCP.
8. **Transición:** con el puerto ocupado por un listener del test, `serve` sin `--port` arranca por el socket y no publica `port`. Es la regla que cierra el choque entre usuarios en esa versión.
9. **Manual, en el `verify.md`:** en Linux, con dos cuentas, la segunda no conecta ni crea el socket, y su daemon arranca aunque el de la primera esté corriendo.

`rag-eval` no aplica: no cambia el ranking ni lo que ve el modelo.

## Riesgos

- **Ignorar la compatibilidad.** El actualizador anterior nunca ve contestar al daemon nuevo, vence su espera y deshace (restaura el binario y, si el daemon nuevo migró, el snapshot `pre-migration`), de forma repetible. Lo cubren la regla permanente y el test con el sello.
- **TCP arrastrado por `update`**, si se elige (ii).
- **Un puente anterior abierto** falla en la segunda versión hasta que se reinicie el editor; por eso esa versión va una minor después.
- **El permiso del socket no es portable;** la garantía depende de la parte 1 de 50.
- **macOS sin ejecutar.**
- **Windows no gana nada con este cambio.** En una máquina Windows sigue todo en TCP hasta 59.
