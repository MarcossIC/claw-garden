# windows-socket — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L | local-transport (58, archivado) | exploración cerrada: AF_UNIX nativo de Windows; la named pipe quedó descartada por la puerta |

**Tesis:** el equivalente de local-transport (58) en Windows es un socket AF_UNIX nativo en `<data>/clawg.sock`, no una named pipe. Lo decidió una puerta en Windows de cuatro rondas, con un criterio: ninguna alternativa entra si empeora lo que hay.

**La named pipe no pasó.**

- Su nombre vive en un espacio de nombres global, así que cualquier proceso lo lista. Cuando el daemon muere, quien recrea la pipe recibe la próxima petición del cliente con su token, y el cliente no ve ningún error.
- Cerrar eso exigía un conector que verificara cada conexión, `unsafe` nuevo, reintentos con backoff y un ACL explícito.
- En rendimiento no gana nada: empata con TCP en el tráfico normal, y en ráfagas es entre 8 y 18 veces más lenta porque no tiene backlog.

**AF_UNIX sí pasó.**

- El archivo del socket hereda el ACL del perfil, y conectar exige `FILE_WRITE_DATA`: otra cuenta recibe el error 10013.
- Para suplantar al daemon, otra cuenta tendría que borrar el socket huérfano y crear otro en el directorio de datos, igual que en Unix.
- Le gana a TCP en todos los escenarios medidos y no falla ni con 512 clientes simultáneos.
- No necesita permisos de administrador ni modo de desarrollador.

Lo que depende de una segunda cuenta local real no se pudo medir; va a `verify.md`.

## La puerta: qué se midió

El entorno:

- Windows 11 25H2 (10.0.26200), con un token de integridad media sin elevar (Administradores sólo como deny).
- tokio 1.51.0, axum 0.8.9, reqwest 0.12.28 y windows-sys 0.61.2; toolchain 1.99.1.
- Un binario descartable, fuera del repositorio.
- Todo con la misma cuenta, salvo donde se dice otra cosa.

### Seguridad de la pipe

| Prueba | Resultado |
|---|---|
| DACL de una pipe creada con `ServerOptions::new()`, leído con `GetSecurityInfo` | `O:<usuario>G:<usuario>D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;<usuario>)(A;;FR;;;WD)(A;;FR;;;AN)`. Everyone y ANONYMOUS LOGON reciben `0x120089`, que es sólo lectura: sin `FILE_WRITE_DATA` (0x2) ni `FILE_CREATE_PIPE_INSTANCE` (0x4). El DACL es igual con `first_pipe_instance(true)` y no tiene etiqueta de integridad. |
| Segundo `create` con `first_pipe_instance(true)` sobre un nombre vivo | Error 5. Sin la marca, la misma cuenta agrega instancias. |
| `GetNamedPipeServerProcessId` y `GetNamedPipeClientProcessId` entre dos procesos | Los dos devuelven el pid correcto. |
| Cliente sólo de lectura (`ClientOptions::read(true).write(false)`) contra una sola instancia | Conecta y la ocupa. El siguiente cliente recibe 231, sea de lectura y escritura o no. |
| Listar `\\.\pipe\` con `read_dir`, `FindFirstFileW` y PowerShell | Unas 595 pipes, la nuestra entre ellas. |
| DACL explícito `D:P(A;;GA;;;SY)(A;;GA;;;<usuario>)`, pasado por `create_with_security_attributes_raw` | Se lee `D:P(A;;FA;;;SY)(A;;FA;;;<usuario>)`. La misma cuenta conecta y recibe 200 por axum, y `first_pipe_instance` sigue dando 5. Las instancias siguientes, creadas sin descriptor, heredan el de la primera. |
| Un cliente que conecta y no manda nada, bajo `axum::serve` | No se corta en 75 s, ni en lectura y escritura ni en sólo lectura. 200 conexiones así, una cada 15 ms, no impiden una petición legítima. La causa: `axum::serve` arma el `Builder` sin timer (`axum-0.8.9/src/serve/mod.rs:391`), y sin timer hyper no aplica su `header_read_timeout` de 30 s (`hyper-1.9.0/src/common/time.rs:70-76`). |
| El impostor: un cliente reqwest compartido habla con A, A muere y B crea el mismo nombre | B lo crea, con `first_pipe_instance(true)`, en el primer intento después de que A cierra sus handles. La primera petición tras la muerte, cuando B todavía no existe, falla con error 2 (`is_connect`). La siguiente, por el mismo cliente, recibe 200 de B, y B lee `Authorization: Bearer …`. Pasa lo mismo con una conexión del pool que estaba ociosa desde antes. |

### `ERROR_PIPE_BUSY` (231)

Cada celda son 640 peticiones: 32 simultáneas, cada una con su propio cliente reqwest, durante 20 rondas. K es la cantidad de instancias esperando conexión. Cada celda trae el porcentaje de éxito, primero con un handler que responde al instante y después con uno que tarda 20 ms. Las configuraciones de reintento:

- **Fijo:** 3 reintentos, 50 ms entre cada uno.
- **Backoff:** empieza en 5 ms y dobla hasta 100 ms, con jitter completo; a lo sumo 12 intentos y 1 s desde el primero.
- **`WaitNamedPipeW`:** espera 250 ms; a lo sumo 5 intentos.

| K | Sin reintento | Fijo | Backoff | `WaitNamedPipeW` |
|---|---|---|---|---|
| 1 | 15 % / 13 % | 51 % / 52 % | 100 %, p99 267 / 314 ms | 63 % / 63 % |
| 2 | 24 % / 23 % | 70 % / 70 % | 100 %, p99 141 / 187 ms | 85 % / 84 % |
| 4 | 35 % / 33 % | 93 % / 93 % | 100 %, p99 62 / 94 ms | 99 % / 100 % |
| 8 | 65 % / 61 % | 100 % | 100 %, p99 31 / 62 ms | 100 % |

También se midió una mezcla realista, de 240 peticiones por celda: 8 en paralelo sobre un cliente compartido (como el puente) más 4 con cliente propio (como los hooks). Sin reintento falla entre el 35 y el 52 % con K=1, y entre el 5 y el 9 % con K=4. Con el backoff llega al 100 % en todas las celdas, con p99 ≤ 124 ms.

Todas las fallas son el error 231. reqwest lo marca como `is_connect()`, así que `map_transport` lo convertiría en `NotRunning` (`crates/clawg-cli/src/client.rs:700-706`). `WaitNamedPipeW` sale peor porque despierta a todos los que esperan a la vez y vuelven a chocar.

### Rendimiento contra TCP

Las condiciones de la medición:

- Build optimizado del binario descartable.
- El mismo router y el mismo middleware de autenticación (bearer y Host) sobre los dos transportes.
- `GET /health` responde unos 60 bytes. `POST /search` recibe 4 KB y responde 8 KB.
- K=4 y el DACL explícito.
- Cada escenario corre 3 veces tras un precalentamiento, y se reporta la corrida mediana. No hubo fallas.
- «+ verificación» suma, en cada conexión nueva, una apertura propia con `GetNamedPipeServerProcessId`.

| Escenario | TCP | Pipe | Pipe + verificación |
|---|---|---|---|
| Puente secuencial con keep-alive, `GET /health` (media / p99) | 49 / 107 µs | 35 / 80 µs | 36 / 83 µs |
| Puente secuencial con keep-alive, `POST /search` | 52 / 123 µs | 54 / 121 µs | 69 / 151 µs |
| Hook: una conexión nueva por petición | 445 / 608 µs | 104 / 230 µs | 125 / 237 µs |
| Puente con 8 peticiones en paralelo | 89 / 238 µs | 90 / 197 µs | 81 / 142 µs |
| Ráfaga de 32 clientes nuevos simultáneos | 1,6 / 3,1 ms | 13,5 / 62,6 ms | 29,5 / 94,8 ms |

- **Throughput de TCP:** 20.541 peticiones por segundo secuenciales y 88.440 con 8 en paralelo.
- **Memoria:** el servidor con la pipe ocupa alrededor de 1 MB más que con TCP, tanto durante la medición como con 200 conexiones ociosas.
- **Costo de la verificación:** `GetNamedPipeServerProcessId` sobre un handle abierto cuesta unos 1,9 µs. Abrir, verificar y cerrar cuesta unos 45 µs.
- **Costo de lanzar un proceso en Windows:** entre 9 y 12 ms. Un hook paga eso antes de tocar el transporte, así que los 0,3 ms que gana con la pipe no se notan.
- **En las ráfagas,** TCP absorbe la carga con su backlog, y la pipe paga los reintentos del error 231.

### AF_UNIX nativo de Windows

Se midió en la misma máquina, sin permisos de administrador ni modo de desarrollador.

**Viabilidad**

- `std::os::windows::net::{UnixListener, UnixStream}` existe en 1.99.1, pero es inestable (feature `windows_unix_domain_sockets`, issue rust-lang/rust#150487).
- socket2 0.6.3, que ya está en `Cargo.lock`, tiene `Domain::UNIX` (`socket2-0.6.3/src/lib.rs:229`) y `SockAddr::unix` (`src/sockaddr.rs:219`). En Windows arma un `SOCKADDR_UN` con la ruta en UTF-8 (`src/sys/windows.rs:980`).
- El servidor de la puerta funciona así:
  1. un socket de socket2 escucha con backlog 128;
  2. un hilo dedicado llama a `accept` de forma bloqueante;
  3. cada conexión aceptada pasa a no bloqueante y se envuelve con `std::net::TcpStream::from_raw_socket` y `tokio::net::TcpStream::from_std`;
  4. el polling AFD de mio maneja sus lecturas y escrituras, y axum la sirve a través de un `Listener` propio.

  `peer_addr` y `local_addr` fallan sobre esas conexiones, porque std espera una dirección IP. axum no los usa.
- reqwest no tiene AF_UNIX en Windows. El cliente de la puerta es un conector de hyper de unas 40 líneas.
- `curl --unix-socket` anda con el curl de Windows (8.21.0).

**Seguridad**

Todo se probó con una sola cuenta, aproximando lo que vería otra con un deny explícito.

| Prueba | Resultado |
|---|---|
| El DACL del archivo del socket | Lo hereda del directorio padre al hacer el bind. En el perfil del usuario queda así: SYSTEM, Administradores y el usuario. |
| El DACL del archivo frente al `connect` | Lo gobierna. Conectar exige `FILE_WRITE_DATA` (0x2). Sin ese permiso, incluso con lectura, el error es 10013 (WSAEACCES). La pipe, en cambio, admitía a Everyone sólo en lectura. |
| El DACL del directorio frente al `connect` | No lo gobierna: con el directorio sin permisos para el usuario, conectar a un socket que ya existe sigue funcionando (privilegio de omitir la comprobación de recorrido). Cuenta el DACL del archivo, heredado al crearlo. |
| El socket huérfano | El archivo queda después de matar al servidor. Conectar da 10061, y un bind sobre esa ruta da 10048: hay que borrarlo antes, igual que en Unix. |
| El pid del par | `SIO_AF_UNIX_GETPEERPID` (`windows-sys-0.61.2/src/Windows/Win32/Networking/WinSock/mod.rs:4042`) da el pid del cliente en el servidor y el del servidor en el cliente, entre dos procesos. |
| El límite de la ruta | 107 bytes en UTF-8 por `SockAddr::unix` (`sun_path` es de 108), más un límite de unos 259 caracteres para la ruta absoluta resuelta. Una ruta con caracteres que no son ASCII llega antes al tope. |

Contra otra cuenta, el impostor queda cerrado por el sistema de archivos, igual que en Unix. Para suplantar al daemon caído habría que borrar el archivo huérfano y crear otro socket dentro del directorio de datos, y eso el ACL heredado del perfil no se lo permite a otra cuenta estándar. Esto es semántica estándar de NTFS para crear y borrar dentro de un directorio y no se midió; va en la prueba manual con una segunda cuenta. A diferencia de la pipe, el nombre no vive en un espacio de nombres global.

**Ráfagas**

Cada fila es una ráfaga de clientes nuevos simultáneos, con el mismo conector de hyper para los dos transportes y sin reintento, durante 20 rondas.

| Simultáneos | AF_UNIX | TCP |
|---|---|---|
| 32 | 0 fallas, 27 ms | 0 fallas, 35 ms |
| 128 | 0 fallas, 111 ms | 0 fallas, 172 ms |
| 256 | 0 fallas, 203 ms | 0 fallas, 10,9 s |
| 512 | 0 fallas, 501 ms | 4116 de 10.240 fallan con 10048 (puertos efímeros agotados por TIME_WAIT) |

**Rendimiento**

Es la mediana de 3 corridas, en µs por petición. AF_UNIX y «TCP hyper» usan el mismo conector de hyper; «TCP reqwest» es el cliente de hoy. El ruido entre corridas es alto en esta máquina: por ejemplo, 53 / 55 / 62 µs para TCP con reqwest en el puente con `GET /health`.

| Escenario | TCP reqwest | TCP hyper | AF_UNIX | Pipe |
|---|---|---|---|---|
| Puente secuencial, `GET /health` (media / p99) | 55 / 181 | 41 / 105 | 33 / 79 | 39 / 126 |
| Puente secuencial, `POST /search` | 55 / 152 | 45 / 118 | 41 / 118 | 62 / 162 |
| Hook: una conexión nueva por petición | 451 / 825 | 300 / 448 | 167 / 348 | 95 / 205 |
| Puente con 8 en paralelo | 135 / 306 | 77 / 192 | 71 / 177 | 103 / 346 |
| Ráfaga de 32 clientes nuevos | 2648 / 3742 | 1942 / 2958 | 820 / 1441 | 13.496 / 61.122 |

- **AF_UNIX contra TCP con el mismo conector:** gana en todos los escenarios. En el tráfico normal la diferencia es de entre 4 y 8 µs; con una conexión nueva por petición, 133 µs; en ráfagas, 1,1 ms.
- **Memoria:** con 200 conexiones ociosas, el servidor AF_UNIX pesa lo mismo que el TCP. Su pico de memoria privada es entre 1,7 y 2,2 MB mayor, por el hilo de `accept` y el canal.
- **TCP con una conexión nueva por petición agota los puertos efímeros:** una corrida dejó 15.741 sockets en TIME_WAIT y contaminó la siguiente. AF_UNIX no usa puertos.

**Lo que no se verificó**

- La build mínima de Windows (la de esta máquina es 10.0.26200).
- Una segunda cuenta real.
- Si otra cuenta, sin el privilegio de omitir el recorrido, se comporta distinto.

### Lo que la puerta no midió

- **Una segunda cuenta local estándar:** abrir la pipe en lectura y escritura y sólo en lectura, crear una instancia y listar, con el DACL por defecto y con el explícito. Hace falta crear esa cuenta. El binario de la puerta trae `hold [--owner-only]` y `probe` para hacerlo a mano.
- **Un daemon arrancado desde una consola elevada:** si su pipe queda con etiqueta de integridad alta, un puente sin elevar no podría escribir (no-write-up). Sin verificar.
- **Los eventos de apagado de Windows:** si un proceso lanzado con `DETACHED_PROCESS` los recibe. Sigue abierto en `openspec/specs/storage/spec.md:223`.

## Premisas

| Premisa | Resultado |
|---|---|
| tokio sirve y abre named pipes | **Cierta.** En `tokio-1.51.0/src/net/windows/named_pipe.rs`: `ServerOptions` trae `reject_remote_clients` en true por defecto (`:1773`), `max_instances` sin límite (`:1774`) y `first_pipe_instance` (`:2051`); un ACL propio sólo entra por `unsafe fn create_with_security_attributes_raw` (`:2304`). `ClientOptions::new` abre en lectura y escritura con `SECURITY_IDENTIFICATION` (`:2394-2402`) y expone `read` y `write`. |
| reqwest habla por una pipe | **Cierta.** `ClientBuilder::windows_named_pipe` (`reqwest-0.12.28/src/async_impl/client.rs:1825-1826`) abre con `ClientOptions::new().open(pipe)`, sin reintento (`reqwest-0.12.28/src/connect.rs:716-728`). |
| Al conector de reqwest se le puede agregar un reintento o una verificación | **Falsa.** `connector_layer` existe (`async_impl/client.rs:2449-2459`), pero la petición `Unnameable` no es `Clone` (`connect.rs:1289-1290`) y la respuesta `Conn` envuelve un `Box<dyn AsyncConnWithInfo>` privado (`connect.rs:1285`, `:1298-1304`). Una capa no puede reintentar el connect ni llegar al handle de la pipe. |
| El pool de reqwest sólo abre conexiones cuando se le pide | **Falsa.** Por defecto, hyper-util repite en una conexión nueva la petición cuya conexión ociosa resultó inservible (`hyper-util-0.1.20/src/client/legacy/client.rs:1034`, `:1545-1553`). |
| axum sirve sobre una pipe | **Cierta, con un `Listener` propio** (`axum-0.8.9/src/serve/listener.rs:9-24`). El de la puerta mantiene K instancias esperando con K tareas y un canal. |
| AF_UNIX no se puede usar en este stack | **Falsa.** mio 1.2.0 no tiene sockets Unix en `src/sys/windows/`, reqwest sólo tiene `unix_socket` en Unix (`async_impl/client.rs:1809`) y en `std` es inestable (issue rust-lang/rust#150487). Pero socket2 0.6.3, que ya está en `Cargo.lock`, lo abre en Windows, y tokio lo maneja envuelto como `TcpStream`. Ver la sección de AF_UNIX nativo. |
| El ACL por defecto impide que otra cuenta escriba | **Cierta según el DACL leído:** Everyone sólo tiene `0x120089`. Falta confirmarlo con una segunda cuenta. |
| Otra cuenta puede retener instancias | **Probable.** Everyone tiene lectura, y con la misma cuenta se midió que una apertura sólo de lectura retiene una instancia. Bajo `axum::serve` la retiene sin límite de tiempo. El DACL explícito lo cierra. |
| Un nombre aleatorio esconde la pipe a otra cuenta | **Falsa.** Cualquier proceso lista `\\.\pipe\`. |
| Alcanza con verificar al servidor al construir el cliente | **Falsa.** El cliente abre conexiones nuevas después, y hyper-util también por su cuenta. Muerto el daemon, la próxima va a quien cree el nombre. |
| reqwest lee el error 231 como «no hay daemon» | **Cierta, medida.** Es un `is_connect()` (`crates/clawg-cli/src/client.rs:700-706`). |
| Unos reintentos fijos alcanzan | **Falsa.** Ver la tabla de `ERROR_PIPE_BUSY`. |
| `windows-sys` trae lo necesario sin crates nuevos | **Cierta.** `GetNamedPipeServerProcessId` y `GetNamedPipeClientProcessId` están en `Win32_System_Pipes` (windows-sys 0.61.2), y el DACL explícito suma `Win32_Security` y `Win32_Security_Authorization`. `clawg-cli` ya depende de windows-sys 0.61 (`crates/clawg-cli/Cargo.toml:62`). |
| Una pipe huérfana impide arrancar | **Falsa.** La pipe desaparece con su último handle; en la puerta, B la crea en el primer intento tras la muerte de A. |
| `daemon.json` es privado en Windows | **Por la ACL del perfil**, sin un ACL propio (`crates/clawg-cli/src/daemon/lifecycle.rs:96-102`). Ningún test lo fija. |

## Problema (verificado contra el código)

En Windows el daemon escucha sólo por TCP, en el puerto del perfil, y un fallo al tomarlo es fatal:

- Fuera de Unix, `has_socket` es false (`crates/clawg-cli/src/daemon/serve.rs:167-168`).
- Con `has_socket` false, `tcp_plan` da `Required` (`serve.rs:286-295`).

Eso trae tres problemas:

- Si el daemon muere sin apagarse, otra cuenta puede ocupar el puerto y recibir el token y los saves del siguiente cliente.
- Cualquier cuenta puede leer `/health`.
- Dos usuarios chocan en el mismo puerto.

En un equipo de un solo usuario la ganancia de seguridad es chica. Lo que sí gana cualquier Windows es poder probar a través del binario el autoarranque y el arranque sin `--port`, como `binary.rs` ya hace en Unix: en Windows `socket_only` devuelve una lista vacía (`crates/clawg-cli/tests/binary.rs:163-172`).

## Lo que ya existe y se reutiliza (de local-transport, 58)

- **El destino del cliente:** `Endpoint { Unix(PathBuf), Tcp(u16) }` y `endpoint(state, base_dir, tcp_only)`, con `cfg!(unix)` (`crates/clawg-cli/src/client.rs:604-625`). `from_data_dir` deriva el destino y nunca lo lee de un archivo (`:654-666`; el porqué está en `:657-660`).
- **La marca en `daemon.json`:** `DaemonState.unix_socket` no es más que una marca (`crates/clawg-cli/src/daemon/lifecycle.rs:40-49`).
- **El arranque:** `tcp_plan` y su `TcpPlan::Optional` (`serve.rs:270-301`), y el hueco para un tercer servidor junto a TCP y el socket (`serve.rs:244-246`).
- **El sello de prueba:** `transport: "tcp"` y `tcp_only()` (`crates/clawg-core/src/build_info.rs:109-117`, `:142-152`), que hoy sólo apagan el socket.
- **La relectura del puente** ante `NotRunning` o un 401 (`crates/clawg-cli/src/mcp/startup.rs:39-44`, `:102-118`).
- **El dueño del candado:** `StoreLock::current_owner` (`crates/clawg-core/src/memory/store_lock.rs:190-203`) devuelve el dueño vivo del candado, o `None` si el registro es de un proceso muerto.
- **El handle de proceso:** `Watched::open` (`crates/clawg-cli/src/process.rs:43-50`) abre un handle `SYNCHRONIZE` sobre un pid. Mientras está abierto, ese pid no se reutiliza.
- **La actualización:** `serve_port` (`crates/clawg-cli/src/update/effects.rs:295-301`), que en Windows le pasa al daemon nuevo el puerto del viejo.
- **El socket de Unix:** `crates/clawg-cli/src/daemon/socket.rs`, que hoy es sólo de Unix, aporta:
  - `check_socket_path` y `SUN_PATH_LEN` (`:24-61`);
  - `bind_socket`, que borra el socket huérfano con el candado tomado (`:68-80`);
  - `PeerChecked`, el `Listener` de axum con el chequeo del par (`:83-116`).

## El enfoque elegido: AF_UNIX

Es lo mismo que 58. Lo que cambia en Windows:

- **Dónde:** `<data>/clawg.sock` (`paths::socket_file`), derivado del directorio de datos. `daemon.json` lleva la marca `unix_socket: true`, y R9 se mantiene sin cambios.
- **El servidor:** un socket de socket2 (`Domain::UNIX`) con backlog y un hilo dedicado de `accept`. Cada conexión se envuelve como `tokio::net::TcpStream`, que es lo que la puerta midió. El hilo tiene que despertarse en el apagado, dentro de los límites de shutdown.
- **El socket huérfano:** con el candado del store tomado, el daemon borra el socket que dejó uno muerto, igual que en Unix (`bind_socket`).
- **El tope de la ruta:** una ruta de más de 107 bytes en UTF-8 detiene el arranque, salvo con `--port` (`check_socket_path`, con 108 bytes en Windows).
- **El cliente:** reqwest no tiene AF_UNIX en Windows, así que hace falta un conector propio de hyper. `DaemonClient` manda sus peticiones por un embudo común a reqwest (TCP y Unix) y a ese conector.
- **TCP:** toma la semántica de 58.
  - Con el socket, el puerto del perfil pasa a `Optional`, y `--port` sigue siendo `Required`.
  - `serve_port` deja de pasar el puerto, salvo con `tcp_only`.
  - El sello `tcp` también apaga el socket de Windows.
- **Sin AF_UNIX:** si el sistema no lo soporta, el daemon sigue sólo por TCP en el puerto del perfil, como hoy. El diseño define cómo se detecta. La build mínima de Windows no se verificó.
- **Las pruebas:** `socket_only` con `CLAWG_TEST_DEFAULT_PORT=off` y `reach` por el socket, también en Windows. Los escenarios del socket de 58 corren en Windows.
- **Sin verificación del par:** el DACL heredado del archivo cumple el papel del directorio `0700` de Unix.

## Enfoques para la pipe (descartada)

Quedan como registro de lo que la pipe habría necesitado.

### El nombre

**(a) Derivado del token (recomendado).** El nombre sería `\\.\pipe\clawg-<perfil>-<32 hex de sha256("clawg-pipe" ‖ token)>`, y `daemon.json` llevaría sólo una marca `pipe: true`, como `unix_socket`.

- El token se genera en cada arranque (`generate_token`, `lifecycle.rs:80-86`), así que el nombre también cambia en cada arranque. Quien liste `\\.\pipe\` ve un hash, no el token.
- El cliente sigue derivando adónde va su token, como manda R9 (`openspec/specs/daemon-access/spec.md:146`), así que un `daemon.json` adulterado no lo desvía.
- `sha2` ya es dependencia de `clawg-cli` (`crates/clawg-cli/Cargo.toml:34`).

**(b) Aleatorio y publicado en `daemon.json`** (la versión anterior de esta exploración). Contradice R9 y `client.rs:657-660`. Contra un archivo adulterado sólo quedaría la verificación del servidor.

**(c) Fijo por perfil. Descartado.** El espacio de nombres es de la máquina: otra cuenta puede crear la pipe antes, y con `first_pipe_instance` el daemon ya no arranca.

### Verificar al servidor

La puerta mostró que la verificación tiene que correr en cada conexión. El impostor se queda con:

- el token del daemon muerto, que ningún daemon nuevo acepta, porque cada arranque genera uno;
- el contenido de las peticiones: saves y consultas;
- la posibilidad de contestarlas con lo que quiera: memorias que le llegan al modelo como evidencia.

La referencia es el pid de `daemon.json`, y sólo si `StoreLock::current_owner` lo confirma como dueño vivo del candado. El cliente guarda un `Watched` sobre ese pid para que no se reutilice. Hay dos formas de hacerlo:

**(a) Un conector propio en Windows (recomendado).** Un `Service<Uri>` que:

1. abre la pipe con `ClientOptions`;
2. reintenta el error 231 con el backoff;
3. compara `GetNamedPipeServerProcessId` con el pid verificado;
4. le entrega la conexión a hyper.

Cierra el agujero del todo. Además, el reintento queda en el mismo lugar y es transparente para las 22 llamadas de `DaemonClient` (los `.http` de `client.rs:714-1158`). Cuesta dos cosas:

- un segundo cliente HTTP en Windows, que convierte en dependencia directa a hyper-util (ya está en `Cargo.lock` por axum y reqwest);
- que `DaemonClient` mande sus peticiones por un embudo común a reqwest y a ese cliente.

**(b) reqwest, con una verificación previa y un chequeo en cada petición.**

- Al construir el cliente, una conexión propia compara el pid del servidor con el del dueño del candado.
- Antes de cada petición, `Watched` dice si ese proceso sigue vivo, sin `unsafe` nuevo.
- El reintento del error 231 envuelve cada `send`.

Deja una carrera abierta. El daemon puede morir entre el chequeo y la conexión que abre reqwest, o la que hyper-util abre por su cuenta al encontrar muerta la del pool. Si en ese instante otra cuenta crea el nombre, esa petición le llega a ella. En la puerta, B creó el nombre en el primer intento.

### Otras defensas

| Defensa | Qué cierra | Coste | Recomendación |
|---|---|---|---|
| `reject_remote_clients(true)` | Las conexiones por SMB | Ninguno: ya viene por defecto | Sí |
| `first_pipe_instance(true)` | Que el daemon arranque detrás de una pipe ajena (error 5, medido) | Ninguno | Sí |
| DACL explícito: SYSTEM y el usuario | Que otra cuenta abra la pipe, aunque sea sólo para leer, y retenga instancias sin límite de tiempo | `unsafe` acotado y sólo en la primera instancia (el SID del token, `ConvertStringSecurityDescriptorToSecurityDescriptorW`, `create_with_security_attributes_raw`), más `Win32_Security` y `Win32_Security_Authorization` | Sí. Ahora hay evidencia: Everyone tiene lectura y axum no corta las conexiones mudas |
| Verificar al cliente por su SID | Lo mismo que el DACL, pero después de aceptar la conexión | Más `unsafe`, y un proceso elevado puede no dejar abrir su token | No: el DACL lo cierra antes |

### `ERROR_PIPE_BUSY`

La recomendación es el backoff exponencial con jitter completo (5 ms doblando hasta 100 ms, a lo sumo 12 intentos y 1 s desde el primero), sólo ante el error 231, más K=4 instancias esperando.

- Reintentar es seguro porque la petición nunca llegó al daemon.
- No choca con la regla de que un timeout es `NotRunning` y el puente lo repite (`openspec/README.md:308`): lo que se reintenta es el connect, antes de mandar un byte.
- Un error 2 (la pipe no existe) sigue siendo `NotRunning` al instante.

### Conexiones mudas

`axum::serve` no corta una conexión que no habla. Eso vale igual para TCP y para el socket de Unix, así que no es de este cambio. El DACL explícito saca de la pipe a las otras cuentas. Un `header_read_timeout` para todos los transportes va al backlog; necesita un bucle de accept propio, con hyper-util y un timer.

### TCP en Windows

Con la pipe, Windows toma la semántica que 58 le dio a Unix:

- `tcp_plan` recibe `socket = true`, así que el puerto del perfil pasa a `Optional`; `--port` sigue siendo `Required`.
- `serve_port` deja de pasar el puerto, salvo con `tcp_only`.

Para depurar, `curl` no habla named pipes, así que sigue siendo `clawg serve --port N`.

## Descartado

| Opción | Razón técnica |
|---|---|
| La named pipe | No gana rendimiento frente a TCP y es entre 8 y 18 veces más lenta en ráfagas, porque no tiene backlog. Su nombre es global: cualquier proceso lo lista y lo recrea después de una caída. Cerrar eso habría exigido un conector que verificara cada conexión, `unsafe` nuevo, reintentos y un ACL explícito. |
| Seguir con TCP y cerrar el cambio sin código | Deja los cuatro problemas, y AF_UNIX le gana a TCP en todos los escenarios medidos. |
| Un nombre fijo por perfil | El espacio de nombres es de la máquina. |
| Caer a TCP cuando la pipe falla | Un proceso de otra cuenta en el puerto vuelve a hacerse pasar por el daemon. |
| Verificar al servidor sólo al construir el cliente | Medido: la conexión siguiente va al impostor sin ningún error. |
| `connector_layer` de reqwest | No puede reintentar el connect ni llegar al handle. |
| Reintentos fijos o `WaitNamedPipeW` | Medidos: con K bajo dejan fallas, y `WaitNamedPipeW` despierta a todos a la vez. |
| Verificar al cliente por su SID | Con AF_UNIX, el DACL heredado del archivo rechaza el `connect` de otra cuenta antes de aceptar la conexión. Compararlo exigiría más `unsafe`, y un proceso elevado puede no dejar abrir su token. |

## Ideas evaluadas

Una investigación externa sobre IPC de alto rendimiento para agentes locales propuso reemplazar HTTP sobre TCP por UDS + ZeroMQ + FlatBuffers + LanceDB. Se contrastó con estas mediciones y con el código.

Lo que la investigación muestra son diferencias relativas: en su ejemplo de PostgreSQL, 0,041 contra 0,089 ms. En absoluto son decenas de microsegundos, lo mismo que se midió acá. Además, el tráfico que la propia investigación le estima a un servicio de memoria («cientos de mensajes por segundo de pocos kilobytes») es unas 100 veces menor que lo que TCP sirvió en esta máquina.

| Propuesta | Veredicto |
|---|---|
| UDS en lugar de TCP | Ya está en Unix: lo hizo local-transport (58). En Windows, el equivalente es AF_UNIX nativo, que es lo que elige este cambio. |
| Named pipes con «poca multiplexación» | No aplica. Habla de FIFOs POSIX, que son unidireccionales; las pipes de Windows son dúplex y tienen una instancia por cliente. |
| Protocolo binario (Protobuf, FlatBuffers, MessagePack) en lugar de JSON | Ahorra unos pocos microsegundos por mensaje. El puente habla JSON-RPC con el cliente MCP de todos modos, y se perdería la API HTTP que comparten el puente, los hooks, la CLI, el guard y los tests. |
| ZeroMQ DEALER/ROUTER | Agrega una librería C que compilar en los dos CI y reemplaza la API HTTP. Su TCP tiene el mismo problema del puerto. Su `ipc://` en Windows no se verificó. |
| gRPC sobre UDS | Otra reescritura de la API: tonic, prost y protoc. En Windows, tokio tampoco tiene UDS. |
| SQLite en WAL, `synchronous=NORMAL` y un embudo único de escritura | Ya está hecho. WAL y NORMAL se leen de vuelta (`crates/clawg-core/src/memory/db.rs:196-213`), hay una sola conexión serializada (`tokio_rusqlite`, `db.rs:143`), un solo proceso es dueño del store y los saves pasan por el worker del outbox. |
| LanceDB en lugar de índices HNSW | No aplica. `vec0` de sqlite-vec 0.1.9 es una búsqueda por fuerza bruta sobre chunks (`vec0Filter_knn`, `sqlite-vec-0.1.9/sqlite-vec.c:7029`, que itera con `vec0Filter_knn_chunks_iter`, `:6624`), no HNSW. Cambiar de motor pondría en riesgo la base del usuario sin una medición que lo pida. |
| Memoria compartida con ring buffer | La propia investigación la llama excesiva para esta carga. En Windows, la sección compartida vive en un espacio de nombres global, con el mismo problema de impostor que la pipe. |

## Interacciones

- **local-transport (58, archivado).** Este cambio reusa su socket, su `Endpoint`, la marca en `daemon.json`, `tcp_plan`, el sello y sus escenarios. La spec `daemon-access` suma lo de Windows:
  - el análogo de R7 (sólo el mismo uid) es el DACL heredado del archivo;
  - R8 y R9 valen también en Windows;
  - R5 y R6 siguen siendo sólo de Unix.
- **self-update.** `serve_port` y su spec (`openspec/specs/self-update/spec.md:122`) cambian en Windows.
- **install.** Hay que reescribir `openspec/specs/install/spec.md:112` («En Windows no hay socket y las pruebas pasan un puerto»).
- **`scripts/install.ps1`.** Llama al `stop` y al `status` del binario anterior (`:265-266`), que habla con un daemon de su misma versión. No cambia nada mientras se respete la regla de versiones.
- **Backlog «El cliente sólo pone timeout en `/health`»** (`openspec/README.md:308`). AF_UNIX tiene backlog, así que no hace falta reintentar conexiones, y `map_transport` no cambia.
- **doctor-checks (31).** Puede informar el DACL del archivo del socket del daemon que corre.
- **shutdown-budget (55, archivado).** Sigue sin verificar que un daemon lanzado con `DETACHED_PROCESS` reciba los eventos de apagado (`openspec/specs/storage/spec.md:223`).

## Alcance candidato

1. Un `Listener` de axum sobre AF_UNIX en Windows: socket2, el hilo de `accept` y su despertar en el apagado.
2. El socket en `<data>/clawg.sock`, compartido con Unix: la marca `unix_socket`, el borrado del huérfano con el candado tomado y el tope de la ruta.
3. El conector de hyper para AF_UNIX en el cliente de Windows, y el embudo común en `DaemonClient`.
4. `endpoint`, `tcp_plan`, `serve_port` y el sello `tcp`, con la semántica de Unix también en Windows.
5. `binary.rs` en Windows:
   - `reach` por el socket;
   - `socket_only` con `CLAWG_TEST_DEFAULT_PORT=off`;
   - los escenarios del socket: status, stop, el puente, los hooks, el autoarranque, el huérfano, la ruta larga, el puerto tomado y los clientes anteriores.
6. Specs y documentación: `daemon-access`, `self-update`, `install`, `.claude/CLAUDE.md` y `README.md`.

### No-alcance

- La named pipe.
- Un timeout para las conexiones mudas: afecta a todos los transportes y va al backlog.
- WSL: un puente de un lado contra el daemon del otro. No se verificó si WSL2 comparte los sockets AF_UNIX de Windows.
- Verificar al cliente por su SID.

## Decisiones

- [x] **El criterio: ninguna alternativa entra si empeora lo que hay.** Cada candidata se mide contra TCP en rendimiento, con un umbral comprometido de antemano, y además tiene que ganar en costo contra beneficio.
- [x] **El transporte de Windows: AF_UNIX nativo**, en `<data>/clawg.sock`, como en Unix. Le gana a TCP en todos los escenarios medidos y cierra la suplantación por otra cuenta con el sistema de archivos. Se descartaron la named pipe y seguir con TCP (ver Descartado).
- [x] **Sin verificación del par, más allá del ACL.** El DACL que el archivo hereda del perfil ya rechaza el `connect` de otra cuenta (10013, medido con un deny sobre la misma cuenta); cumple el papel del directorio `0700` de Unix. Se descartó que el servidor compare el SID del cliente. Se reabre si la prueba con una segunda cuenta muestra que el DACL no alcanza.
- [x] **La prueba con una segunda cuenta va en `verify.md`, contra el daemon real.** Se prueba conectar, borrar el socket y crear otro en el directorio de datos.
- [x] **El cambio se llama windows-socket** (antes, windows-pipe).

Las decisiones que se habían tomado para la pipe (el nombre derivado del token, un conector que verifica cada conexión, el DACL explícito y K = 4) quedan como registro: no se aplican a AF_UNIX.

## Cómo se mide

1. **CI de Windows:** la suite, más un test de 32 conexiones simultáneas por el socket sin ningún `NotRunning`.
2. **Rendimiento, el criterio del cambio, con el umbral fijado de antemano:** en Windows, contra el daemon real con el modelo falso, se mide AF_UNIX contra TCP (con `--port`) en cuatro escenarios:
   - el puente secuencial con keep-alive;
   - una conexión nueva por petición;
   - 8 peticiones en paralelo;
   - una ráfaga de 32 clientes.

   La media de AF_UNIX no puede superar la de TCP en más de un 10 % en ninguno, y la ráfaga tiene que terminar sin fallas. Va en `verify.md`.
3. **El socket huérfano:** después de matar a un daemon, el siguiente arranca sobre el socket que quedó, y lo borra con el candado tomado.
4. **El DACL frente al `connect`:** con un deny de `FILE_WRITE_DATA` para el usuario sobre el archivo del socket, conectar falla.
5. **`binary.rs` en Windows:**
   - serve, status, stop, el puente, los hooks y el autoarranque llegan por el socket;
   - con el puerto del perfil tomado, el daemon sigue por el socket;
   - una ruta demasiado larga se rechaza nombrándola.
6. **Versiones:** los escenarios del sello `tcp` corren también en Windows.
7. **Manual, en `verify.md`, con una segunda cuenta estándar:**
   - conectar al socket del daemon (se espera el error 10013);
   - borrar el socket y crear otro en el directorio de datos (se espera acceso denegado).

`rag-eval` no aplica.

## Riesgos

- **Envolver un socket AF_UNIX como `TcpStream`** se apoya en que mio use el `SOCKET` sin mirar su familia. No está documentado, así que una actualización de tokio o de mio podría romperlo; el CI de Windows lo detectaría.
- **`peer_addr` y `local_addr` fallan** sobre esas conexiones: nada en el camino de una petición puede llamarlos.
- **El hilo de `accept` es bloqueante:** el apagado tiene que despertarlo sin pasarse de `RUNTIME_TEARDOWN`.
- **`unsafe` nuevo:** `from_raw_socket`.
- **Dos clientes HTTP en `DaemonClient`:** reqwest para TCP y Unix, y hyper para AF_UNIX en Windows.
- **La build mínima de Windows** con AF_UNIX no se verificó, y el tope de unos 259 caracteres para la ruta resuelta se suma al de 107 bytes.
- **El ACL del directorio de datos:** si alguien lo abrió, el socket hereda ese ACL, igual que el token hoy (`crates/clawg-cli/src/daemon/lifecycle.rs:96-102`).
- **El CI no tiene dos cuentas:** lo que depende de otro usuario queda como prueba manual.
