# windows-socket — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L | local-transport (58, archivado) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y mediciones en la [exploración](exploration.md); verificado otra vez contra `develop` (`e57a563`, con #65 fusionado).

En Windows el daemon escucha sólo en `127.0.0.1:<puerto del perfil>`, y un `bind` que falla detiene el arranque: fuera de Unix `has_socket` es falso (`crates/clawg-cli/src/daemon/serve.rs:167-168`) y, sin socket, `tcp_plan` da `Required` (`serve.rs:290-295`), que termina en `ServeError::Bind` (`serve.rs:45-50`, `:171`). El cliente va siempre al `port` de `daemon.json`: `endpoint` mira la marca del socket sólo en Unix (`crates/clawg-cli/src/client.rs:621-626`) y `socket_http` no existe fuera de Unix (`client.rs:641-644`). De ahí salen los cuatro problemas que local-transport cerró en Unix:

1. **Otra cuenta se hace pasar por el daemon.** Uno que muere sin apagarse deja `daemon.json` y `token`, porque `clear_state` sólo corre en la salida ordenada (`serve.rs:262-264`). Cualquier cuenta puede tomar el puerto, y el siguiente cliente le manda el token (`client.rs:751`) y cada save.
2. **`/health` es legible por cualquier cuenta,** sin token (`crates/clawg-cli/src/daemon/api.rs:871-874`).
3. **Dos usuarios no pueden usar el mismo perfil a la vez:** el puerto es fijo por perfil (`crates/clawg-core/src/paths.rs:139-144`), el autoarranque lanza `serve` sin `--port` (`client.rs:1218`, `:1222-1231`) y el daemon del segundo falla en el `bind`.
4. **El autoarranque no se prueba por el binario:** sin socket, `CLAWG_TEST_DEFAULT_PORT=off` no apaga el puerto del perfil (`serve.rs:291-294`), así que `socket_only` no pone nada en Windows (`crates/clawg-cli/tests/binary.rs:163-172`), y los escenarios del socket, del autoarranque y del sello `tcp` siguen bajo `#[cfg(unix)]` (`binary.rs:721-833`, `:1832-1836`, `:1842-1886`).

## Objetivo que sirve

Transversal, como 58: el daemon de una cuenta sólo lo alcanza esa cuenta (y los administradores), ahora también en Windows, sin dejar sin daemon a los clientes de versiones anteriores. Lo que protege está debajo de todos los objetivos: que el token, los saves y las consultas lleguen al daemon del usuario y a ningún otro proceso.

## Decisiones

Cerradas en la exploración, con su porqué y sus mediciones ([Decisiones](exploration.md#decisiones), [El enfoque elegido](exploration.md#el-enfoque-elegido-af_unix)):

- **AF_UNIX nativo en `<data>/clawg.sock`** (`paths::socket_file`, `crates/clawg-core/src/paths.rs:231-233`), con el diseño de 58: la marca `unix_socket: true` en `daemon.json`; R9 vale igual en Windows; el huérfano se borra con el candado del store tomado; `check_socket_path` con 108 bytes; `tcp_plan` recibe `socket = true`, así que el puerto del perfil pasa a `Optional` y `--port` sigue `Required`; `serve_port` no pasa puerto salvo con `tcp_only` o cuando el daemon que corría no publicó su socket (ver «Un fallo de `update` que el cambio cierra»); el sello `tcp` también apaga el socket de Windows.
- **Servidor:** un socket de socket2 (`Domain::UNIX`) con un hilo dedicado de `accept` bloqueante. Cada conexión se envuelve con la conversión segura `From<Socket> for std::net::TcpStream` de socket2 (`socket2-0.6.3/src/socket.rs:2445`) y `tokio::net::TcpStream::from_std`, que es lo que midió la puerta. El hilo se despierta en el apagado, dentro de los plazos que ya existen.
- **Cliente:** reqwest no tiene AF_UNIX en Windows (`reqwest-0.12.28/src/async_impl/client.rs:1808-1809`), así que lleva un conector propio de hyper. Las 23 llamadas de `DaemonClient` (`client.rs:713-1187`) pasan por un embudo común sobre reqwest (TCP y socket de Unix) y ese conector.
- **Sin verificación del par más allá del ACL:** el archivo hereda el DACL del perfil y `connect` exige `FILE_WRITE_DATA` (10013 sin él, medido). Comparar el SID en el servidor queda descartado; se reabre sólo si la prueba con una segunda cuenta muestra un hueco.
- **Sin AF_UNIX,** el daemon sirve TCP en el puerto del perfil como hoy, sin la marca. Cómo se detecta lo define el diseño. Sólo esa causa cae a TCP: cualquier otra falla del socket detiene el arranque, como en Unix (`serve.rs:318-325`).

## Alcance

1. El listener AF_UNIX para axum en Windows: socket2, el hilo de `accept` y su despertar en el apagado.
2. El socket compartido con Unix: la marca, el huérfano y el tope de la ruta. Hoy el módulo es sólo de Unix (`crates/clawg-cli/src/daemon/mod.rs:9-10`), `check_socket_path` cuenta bytes con `OsStrExt` de Unix (`daemon/socket.rs:54-61`) y `SUN_PATH_LEN` vale 104 fuera de Linux (`socket.rs:26-29`).
3. El conector de hyper en el cliente de Windows y el embudo común en `DaemonClient`.
4. `endpoint`, `tcp_plan`, `serve_port` y el sello `tcp` con la semántica de Unix también en Windows, y la detección de un sistema sin AF_UNIX. Incluye el arreglo de un fallo de `clawg update` que ya está en Unix: `serve_port(old_port, socket, tcp_only)` recibe si el daemon que corría publicó su socket, leído de `daemon.json`, en lugar de `cfg!(unix)`, y conserva su puerto cuando no lo publicó (ver «Un fallo de `update` que el cambio cierra»).
5. `binary.rs` en Windows: `reach` por el socket (hoy sólo Unix, `binary.rs:342-352`), `socket_only` con `CLAWG_TEST_DEFAULT_PORT=off` y los escenarios de 58 sin `#[cfg(unix)]`.
6. Docs y specs: `.claude/CLAUDE.md` y `README.md` (`:514`, `:516`); al archivar, `specs/daemon-access` (el «(Unix)» de R7, `spec.md:117`; R8 nombra a Windows como el caso sin socket, `:135`; R9 dice «en Unix», `:146`), `specs/self-update` (`spec.md:122`) y `specs/install` (`spec.md:112`).

## Fuera de alcance

- La named pipe, descartada por la puerta.
- Un tope para las conexiones que no hablan: afecta a todos los transportes y sigue en el backlog (`openspec/README.md:306`).
- Apagar TCP por defecto, también en Windows: va con el backlog «TCP sólo con `--port`» (`openspec/README.md:305`). Hasta entonces otra cuenta sigue leyendo `/health` por el puerto del perfil, así que el problema 2 se cierra con ese cambio, como en Unix.
- WSL: un puente de un lado contra el daemon del otro.
- Verificar al cliente por su SID.
- Los eventos de apagado de un daemon sin consola, que siguen sin verificar (`openspec/specs/storage/spec.md:223`).

## Cómo se mide el éxito

Tests que fallan hoy, con el modelo falso, sin red ni perfiles reales:

- `binary.rs` en Windows: `serve`, `status`, `stop`, el puente, los hooks y el autoarranque llegan por el socket; con el puerto del perfil tomado, el daemon sigue por el socket y no publica `port`; el socket que deja un daemon matado no impide el siguiente arranque; una ruta demasiado larga se rechaza nombrándola. Son los escenarios de 58 (`binary.rs:721-833`).
- Con un deny de `FILE_WRITE_DATA` para la propia cuenta sobre el archivo del socket, `connect` falla.
- 32 conexiones simultáneas por el socket, ninguna leída como `NotRunning` (`map_transport` convierte todo error de conexión en `NotRunning`, `client.rs:701-707`).
- Versiones: los escenarios del sello `tcp` (`binary.rs:1842-1886`) y la actualización completa sin `port` (`binary.rs:1832-1836`) corren en Windows.
- `clawg update` de un daemon que sirve sólo TCP porque la ruta de su socket no entra: el daemon nuevo recibe el puerto del viejo y contesta, también el que arranca al deshacer. Hoy falla en Unix (ver «Un fallo de `update` que el cambio cierra»).

**El rendimiento es la barra del cambio, fijada de antemano.** En Windows, con un build `testing` (el modelo falso lo pide) y contra el daemon real arrancado con `--port`, que sirve los dos transportes a la vez, se mide AF_UNIX contra TCP con `DaemonClient`, lo que usan el puente y los hooks, en cinco escenarios: el puente secuencial con keep-alive, una conexión nueva por petición, 8 peticiones en paralelo y una ráfaga de 32 clientes, los cuatro con `health()`, y el puente secuencial con keep-alive con una respuesta de unos 8 KB (el diseño elige la llamada y cómo se siembra con el modelo falso). La media de AF_UNIX no puede superar la de TCP en más de un 10 % en ninguno, y la ráfaga tiene que terminar sin fallas. El método es el de la puerta (la mediana de 3 corridas tras un precalentamiento) y el resultado va en `verify.md`. **Si no se cumple, el cambio no sale:** no se integra y Windows sigue sólo por TCP. Una alternativa que no mejora o que degrada el rendimiento no vale el cambio. La puerta midió AF_UNIX por delante de TCP en todos los escenarios, pero con un binario descartable; la barra se mide sobre el producto.

**Manual, en `verify.md`, con una segunda cuenta local estándar y el daemon real:** conectar al socket (se espera 10013), borrar el socket y crear otro en el directorio de datos (se espera acceso denegado).

`rag-eval` no aplica: no cambia el ranking ni lo que ve el modelo.

## Versiones

Un clawg anterior, en Windows, habla sólo TCP: no mira `unix_socket` (`endpoint` lo hace sólo en Unix, `client.rs:622`) y lee `port`.

| Caso | Qué pasa |
|---|---|
| Puente anterior y daemon nuevo | El daemon nuevo enciende también el puerto del perfil (`Optional`) y publica `port`, así que el puente llega por TCP. Si otra cuenta tiene el puerto, `daemon.json` no lleva `port` y el puente ve «no hay daemon» hasta que se reinicie el editor, como en Unix. |
| Puente nuevo y daemon anterior | El daemon anterior no publica la marca (`false`, o el campo ausente, que se lee falso, `crates/clawg-cli/src/daemon/lifecycle.rs:48-49`), y el cliente nuevo va por `port`. |
| `clawg update` desde un binario anterior | Pasa `--port` con el puerto del daemon viejo (`crates/clawg-cli/src/update/effects.rs:295-301`, llamado con `cfg!(unix)` en `:431` y `:458`). El daemon nuevo enciende TCP en ese puerto (`Required`) y el socket, y el actualizador lo ve contestar por TCP. TCP queda encendido hasta la actualización siguiente. |
| `clawg update` desde el binario nuevo | No pasa `--port`, tampoco en Windows, salvo cuando el daemon que corría no publicó su socket (`daemon.json` sin `unix_socket`): entonces pasa su puerto. El daemon nuevo, y el viejo al deshacer, arrancan como en el autoarranque y se vigilan por el socket. |
| `scripts/install.ps1` | Llama al `stop` y al `status` del binario anterior (`scripts/install.ps1:265-266`), que habla con el daemon de su misma versión: por TCP si es anterior, por el socket si es nuevo. Nada cambia mientras se respete la regla de versiones. |

### Un fallo de `update` que el cambio cierra

Desde 0.37.0, en Unix, un daemon lanzado con `--port N` cuya ruta de socket no entra en `sun_path` sirve sólo TCP (`crates/clawg-cli/src/daemon/serve.rs:320-322`). `clawg update` lo reinicia con `serve_port(old.port, cfg!(unix), …)`, que en Unix da `None` (`crates/clawg-cli/src/update/effects.rs:295-301`, llamado en `:431` y `:458`). Sin `--port`, la ruta demasiado larga es fatal (`serve.rs:324`): el daemon nuevo no arranca, y el que arranca al deshacer tampoco. El cambio lo cierra con `serve_port(old_port, socket, tcp_only)`, donde `socket` es si el daemon que corría publicó su socket, leído de `daemon.json`: el puerto del viejo se conserva siempre que no lo publicó, además de con `tcp_only`. Se arregla en el mismo cambio porque Windows hereda el mismo caso (la ruta de 107 bytes) y la misma función.

## Datos y vuelta atrás

No toca `memory.db`, el esquema, los identificadores ni el contrato de embedding: no hay migración. `daemon.json` no gana campos: `unix_socket` ya existe con `serde(default)` y `port` ya es opcional (`lifecycle.rs:46-49`), y `DaemonState` no rechaza campos desconocidos (`lifecycle.rs:43-44`). En Windows `unix_socket` pasa a valer `true`. Volver atrás es instalar el binario anterior: escucha por TCP como hoy, y un `clawg.sock` que quede es inerte (`clear_state` de 0.37.0 ya lo borra al salir, `lifecycle.rs:167-171`, y el próximo daemon nuevo lo borra al arrancar con el candado tomado).

## Dependencias

Verificado en `Cargo.lock` y en los manifiestos. Acá «dependencia nueva» es una línea nueva en `crates/clawg-cli/Cargo.toml` (hoy no está ninguna de estas, `:24-70`) y en la lista de `clawg-cli` en `Cargo.lock` (`:345-373`), que se commitea con ella porque el CI usa `--locked`. Ninguna suma un crate, una versión ni una feature que el build de Windows no compile ya:

| Crate | En `Cargo.lock` | Ya se compila por | Lo que se usaría ya está |
|---|---|---|---|
| socket2 | 0.6.3, la única versión | tokio `net`, que `clawg-cli` pide (`tokio-1.51.0/Cargo.toml:96-107`, `:873-876`, con `all`), y hyper-util `client-legacy` (`hyper-util-0.1.20/Cargo.toml:57-63`) | `Domain::UNIX` y `SockAddr::unix` no piden feature (`socket2-0.6.3/src/lib.rs:228-229`, `src/sockaddr.rs:218-219`) |
| hyper | 1.9.0 | axum `http1` (`axum-0.8.9/Cargo.toml:97-101`) y reqwest (`reqwest-0.12.28/Cargo.toml:279-284`) | `client` y `http1`, por reqwest |
| hyper-util | 0.1.20 | reqwest (`reqwest-0.12.28/Cargo.toml:299-307`) y axum (`axum-0.8.9/Cargo.toml:190-197`) | `client-legacy`, `http1` y `tokio`, por reqwest |
| http-body-util | 0.1.3 | axum (`axum-0.8.9/Cargo.toml:183-184`) y reqwest (`reqwest-0.12.28/Cargo.toml:276-277`) | — |
| tower-service | 0.3.3 | axum (`axum-0.8.9/Cargo.toml:309-310`) y reqwest (`reqwest-0.12.28/Cargo.toml:396-397`) | — |

Cuáles entran, y si van bajo `[target.'cfg(windows)'.dependencies]`, lo decide el diseño. `windows-sys` 0.61.2 ya compila `Win32_Networking_WinSock` por socket2 (`socket2-0.6.3/Cargo.toml:96-104`), así que nombrar un código de WinSock no suma nada; el camino de producción no necesita otra feature. Si la prueba del deny arma el ACL con la API en lugar de `icacls`, sumaría `Win32_Security_Authorization`, que hoy ningún crate del build activa (tokio la pide sólo en sus dev-dependencies, `tokio-1.51.0/Cargo.toml:935-939`).

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Envolver un `SOCKET` AF_UNIX como `TcpStream` se apoya en que mio lo maneje sin mirar su familia. No está documentado: mio 1.2.0 no tiene sockets Unix en Windows (`mio-1.2.0/src/sys/windows/` no trae `uds`), y una actualización de tokio o de mio podría romperlo. | El CI de Windows corre la suite y la prueba de 32 conexiones por el socket. |
| `peer_addr` y `local_addr` fallan sobre esas conexiones, porque std espera una dirección IP. | Nada en clawg los llama (no usa `ConnectInfo`). El `Connection` de hyper-util para `TcpStream` tolera que fallen (`hyper-util-0.1.20/src/client/legacy/connect/http.rs:568-571`). |
| El hilo de `accept` es bloqueante: un apagado que no lo despierta pasa los plazos (`HTTP_GRACE` y `RUNTIME_TEARDOWN`, `serve.rs:74`, `:79`, que `STOP_TIMEOUT` suma). | El diseño define el despertar, y un test lo prueba dentro de esos plazos. |
| Sin `unsafe` nuevo en clawg: la conversión del `SOCKET` a `std::net::TcpStream` es el `From<Socket>` seguro de socket2 (`socket2-0.6.3/src/socket.rs:2445`), que hace el `from_raw_socket` por dentro. | Queda el riesgo de semántica de la primera fila, no uno de memoria. |
| Dos clientes HTTP en `DaemonClient`. | Un embudo único, para que `read_response` y `map_transport` (`client.rs:684-707`) no se separen por transporte. |
| La build mínima de Windows con AF_UNIX no se verificó. | Sin AF_UNIX, TCP como hoy. |
| El tope de la ruta: 107 bytes en UTF-8 (`sun_path` de 108, `windows-sys-0.61.2/src/Windows/Win32/Networking/WinSock/mod.rs:4331`; socket2 rechaza además una ruta que no es UTF-8, `socket2-0.6.3/src/sys/windows.rs:992-1004`), más unos 259 caracteres para la ruta absoluta resuelta (medido en la puerta). | `check_socket_path` nombra la ruta y el límite, y `--port` sigue disponible. |
| El CI no tiene una segunda cuenta. | El deny sobre la propia cuenta corre en CI; la otra cuenta, a mano en `verify.md`. |
| Un directorio de datos cuyo ACL se abrió: el socket lo hereda, como hoy el token (`lifecycle.rs:96-102`). | Igual que hoy. doctor-checks (31) puede informar el DACL del socket. |

## Tamaño y orden

L. Depende sólo de local-transport (58, archivado), cuyo socket, `Endpoint`, marca, `tcp_plan`, sello y escenarios reusa. No toca la memoria ni el ranking.
