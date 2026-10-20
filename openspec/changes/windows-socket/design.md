# windows-socket — Diseño

Reusa el diseño de local-transport (58) tal cual: la marca `unix_socket` (D2), `endpoint` sin caída a TCP (D3), `tcp_plan` (D4), el orden de arranque con el huérfano (D5) y los stand-ins (D7). Acá va sólo lo que cambia para Windows. Las líneas de `client.rs` y `binary.rs` son las de `develop` con #65 fusionado.

Marcas: **[código]** verificado en este repo o en el registro de crates; **[puerta]** medido en la puerta ([exploración](exploration.md#af_unix-nativo-de-windows)); **[sin verificar]**.

## D1. El módulo del socket: un contrato común y una mitad por sistema

`daemon/socket.rs` pasa a ser `daemon/socket/`, con el estilo `mod.rs` del crate:

| Archivo | Qué tiene |
|---|---|
| `mod.rs` (todos) | `SUN_PATH_LEN`, `SocketError`, `check_socket_path`, el borrado del huérfano, `bind_socket`, `serves_without_socket` y `type Listener` |
| `unix.rs` | `admits_peer` y `PeerChecked`, sin cambios (`socket.rs:20-22`, `:82-116`) |
| `windows.rs` | el listener (D2), `connect`, el `Connector` del cliente (D4) y `lacks_af_unix` (D3) |

- `SUN_PATH_LEN` vale 108 en Linux y en Windows (`windows-sys-0.61.2/src/Windows/Win32/Networking/WinSock/mod.rs:4331`) y 104 en el resto. Hoy vale 104 en todo lo que no es Linux (`socket.rs:26-29`) **[código]**.
- Lo que mide `check_socket_path` sale de una función pura por sistema. En Unix son los bytes del `OsStr` (`socket.rs:55-56`). En Windows son los bytes UTF-8, porque socket2 arma `sun_path` desde `to_str()` y rechaza lo que no es UTF-8 (`socket2-0.6.3/src/sys/windows.rs:992-1005`) **[código]**. Una ruta que no es UTF-8 da `SocketError::NotUnicode`, que nombra la ruta igual que `TooLong`. Las rutas son absolutas (`paths.rs:231-233`), así que el tope de unos 259 caracteres para la ruta resuelta **[puerta]** nunca llega antes que los 107 bytes.
- `serves_without_socket(err, explicit_port) -> bool` es la decisión que hoy está en el `match` de `bind_profile_socket` (`serve.rs:318-325`), sacada a una función pura con test. Dan `true` `TooLong` o `NotUnicode` con `--port`, y `Unsupported` (D3) siempre. Todo lo demás detiene el arranque.
- Lo que sólo pasa en Windows existe sólo en Windows: `NotUnicode` y `Unsupported` van con `cfg(windows)`, y con ellas sus brazos en `serves_without_socket`, la medida UTF-8 de la ruta y `lacks_af_unix`. Linux nunca las construye, así que su código no carga ni prueba casos que allí no pueden darse, y no depende de que `SocketError` siga siendo público para compilar sin `dead_code`, que `tests/conventions.rs` no deja esconder con `#[allow]`. La tabla de `serves_without_socket` se parte igual: un test común con `TooLong` con y sin `--port`, `Bind` y `Stale`, y uno de Windows con `NotUnicode` y `Unsupported`, con y sin `--port`.
- `bind_socket` hace lo mismo en los dos sistemas: chequeo, borrado del huérfano (`socket.rs:70-75`) y el `bind` de cada uno. `type Listener` es `PeerChecked` en Unix y `AfUnixListener` en Windows, así que `serve.rs` no necesita `cfg`.

| Alternativa | Por qué no |
|---|---|
| Un `src/afunix.rs` en la raíz para el listener y el conector | El `connect` lo comparten el despertar del listener (D2) y el cliente. En `daemon/socket/windows.rs`, el disfraz de AF_UNIX como `TcpStream` queda en un solo archivo, y `client.rs` ya importa de `crate::daemon` (`client.rs:32-33`). |
| Un solo `socket.rs` lleno de `cfg` | Las dos mitades no comparten más que el contrato, y el archivo se volvería una alternancia de `cfg`. |

## D2. El listener de Windows: socket2, un hilo propio y un despertar por autoconexión

```text
hilo "clawg-accept" (std)                                runtime
accept ─ set_nonblocking ─ TcpStream::from(socket) ─ mpsc(128) ─> Listener::accept ─ TcpStream::from_std ─> axum
```

- **socket2 con backlog 128.** `Socket::new(Domain::UNIX, Type::STREAM, None)`, `bind(&SockAddr::unix(path)?)` y `listen(128)`. Es el backlog que mio le da hoy al `TcpListener` de Windows (`mio-1.2.0/src/net/tcp/listener.rs:85-94`) **[código]**, así los dos transportes de Windows encolan lo mismo. También es el que midió la puerta: 512 clientes simultáneos sin una falla **[puerta]**.
- **Un hilo de std, no `spawn_blocking`.** `runtime.shutdown_timeout(RUNTIME_TEARDOWN)` espera a las tareas bloqueantes de tokio (`main.rs:90-93`): un `accept` que no vuelve se comería los 2 s en cada apagado. Al hilo de std no lo espera nadie, y termina con el proceso.
- **El canal.** Un `tokio::sync::mpsc` acotado a 128, el backlog, con `blocking_send` desde el hilo. El hilo entrega un `std::net::TcpStream` ya no bloqueante, y `accept` lo registra con `tokio::net::TcpStream::from_std`, que necesita el runtime.
- **Sin `unsafe` en clawg.** La conversión es el `From<Socket> for std::net::TcpStream` de socket2 (`socket2-0.6.3/src/socket.rs:2445`, que hace el `from_raw_socket` por dentro, `src/lib.rs:108-123`) **[código]**. Lo que queda es un riesgo de semántica, no de memoria: que mio maneje un `SOCKET` AF_UNIX sin mirar su familia **[puerta]**. socket2 crea sin herencia de handle tanto el socket como cada conexión aceptada (`src/socket.rs:763-765`, `:289-292`) **[código]**.
- **Errores de `accept`.** Siguen la política que axum aplica al TCP y al socket de Unix (`axum-0.8.9/src/serve/listener.rs:140-167`). Una conexión rechazada, abortada o reseteada se reintenta en el acto. Cualquier otro error se registra con `error` y el hilo duerme 1 s. Una conexión que no pasa a no bloqueante se suelta con un `warn`. El hilo no tiene `unwrap`: sólo termina cuando falla el envío.
- **El despertar.** axum suelta el listener apenas llega la señal (`axum-0.8.9/src/serve/mod.rs:284-297`) **[código]**, dentro de `HTTP_GRACE`. El `Drop` de `AfUnixListener` cierra el canal y se conecta a su propia ruta con `connect`. El hilo acepta esa conexión, el envío falla y el hilo termina soltando el socket que escucha. **[sin verificar]**: el hilo de la puerta corría hasta el fin del proceso y nunca se despertó; lo prueba un test (D9). Si esa conexión propia fallara, el hilo quedaría bloqueado hasta el fin del proceso, y eso no demora la salida porque nadie lo espera.

| Despertar | Contra |
|---|---|
| **Autoconexión (elegido)** | Depende de que entre el `connect` propio. Es código seguro y tarda microsegundos. |
| Cerrar el socket desde otro hilo | Exige un `closesocket` sobre un handle que el hilo del `accept` todavía posee: `unsafe`, doble cierre y reutilización del handle. |
| Listener no bloqueante con `WSAPoll` y timeout | Exige `unsafe` de FFI. El despertar tarda lo que el timeout, y el hilo da una vuelta en cada uno. |
| Un `tokio::net::TcpListener` sobre el socket | El `accept` de std devuelve un `std::net::SocketAddr`, que sólo puede ser IPv4 o IPv6: con un par AF_UNIX falla y cierra la conexión ya aceptada. |

## D3. Un Windows sin AF_UNIX

- Sólo cae a TCP el caso en que `Socket::new(Domain::UNIX, …)` falla con `WSAEAFNOSUPPORT` (10047, `windows-sys-0.61.2/.../WinSock/mod.rs:5218`) **[código]**. Según el contrato de `WSASocketW`, es el error de una familia sin proveedor. `lacks_af_unix(&io::Error) -> bool` compara `raw_os_error()`; es pura y se prueba con `io::Error::from_raw_os_error`.
- Ese caso es `SocketError::Unsupported`. Cualquier otro error (al crear el socket, en `bind` o `listen`, o al lanzar el hilo) es `SocketError::Bind`, que nombra la ruta y detiene el arranque como en Unix (`serve.rs:324`).
- Con `Unsupported`, `bind_profile_socket` registra un `warn` y devuelve `None`, con `--port` o sin él. `has_socket` queda falso y `tcp_plan(None, puerto, false, _)` da `Required` (`serve.rs:290-295`): es el Windows de hoy. Con el stand-in `off` también da `Required`, porque nunca queda sin transporte (`serve.rs:451`).
- **[sin verificar]** en una build sin AF_UNIX (anteriores a Windows 10 1803, como LTSC 2016 y Server 2016). Si una de ellas devolviera otro código, el daemon no arrancaría ni con `--port`. No se amplía a 10043 ni a 10044: esos códigos dicen que la familia existe, y la proposición sólo deja caer a TCP el sistema sin AF_UNIX.

## D4. El cliente: un embudo y dos transportes

```rust
enum Transport {
    Http(reqwest::Client), // TCP, y el socket en Unix
    #[cfg(windows)]
    AfUnix(hyper_util::client::legacy::Client<Connector, Full<Bytes>>),
}
pub struct Call<'a> { method: Method, path: String, query: Vec<(&'a str, String)>,
                      body: Option<Vec<u8>>, bearer: bool }
pub struct Answer { pub status: StatusCode, pub body: Vec<u8> }
impl DaemonClient { pub async fn send(&self, call: Call<'_>) -> Result<Answer, ClientError> }
fn read_response<T: DeserializeOwned>(answer: Answer) -> Result<T, ClientError>
fn call_url(base: &str, call: &Call) -> reqwest::Url
```

- Las 23 llamadas (`client.rs:713-1187`) arman un `Call` y pasan por `send`. Ninguna toca un cliente HTTP. `read_response` (`:684-696`) pasa a ser pura sobre `Answer`, y `register_session`, que hoy la repite a mano (`:849-857`), usa la misma comprobación de estado.
- `call_url` arma la URL con `reqwest::Url` y `query_pairs_mut`, que codifica igual que `.query()` de reqwest, sin `?` cuando no hay pares. La usan las dos ramas, así que la consulta no puede codificarse distinto según el transporte.
- **La rama hyper.**
  - El `Bearer` va como cabecera sensible, y `content-type: application/json` va cuando hay cuerpo.
  - `Host` lo pone hyper-util desde la URI (`hyper-util-0.1.20/src/client/legacy/client.rs:298-310`): es `localhost`, que el guardia admite (`guard.rs:20`, `:29-35`).
  - La petición sale en forma de origen porque la conexión no es de un proxy (`hyper-util-0.1.20/src/client/legacy/client.rs:312-319`). Ese cliente sólo usa el conector que recibe y no lee variables de proxy.
  - El pool es el de reqwest: 90 s ocioso con `TokioTimer` (`reqwest-0.12.28/src/async_impl/client.rs:301`, `:973-974`) **[código]**.
- **Errores.**
  - reqwest da `is_connect()` cuando el error de hyper-util de su cadena lo es (`reqwest-0.12.28/src/error.rs:147-158`). La rama hyper mira ese mismo `is_connect()` (`hyper-util-0.1.20/src/client/legacy/client.rs:1642-1644`), y todo error del conector es de esa clase (`:517`) **[código]**.
  - Las dos ramas terminan en un clasificador puro, que es el `map_transport` de hoy (`client.rs:701-707`): conexión o timeout dan `NotRunning`, y cualquier otra cosa `Http`. Un fallo al leer el cuerpo es `Http`, como hoy en `read_response`.
- **El timeout: sólo `health`, y por fuera de `send`.**
  - `send` no aplica ningún timeout, en ninguna de las dos ramas. `health` es la única que envuelve su llamada, con `tokio::time::timeout(HEALTH_TIMEOUT, self.send(call))`, y el vencido da `ClientError::NotRunning`. Funciona igual sobre la rama de reqwest (TCP, o el socket en Unix) y sobre la de hyper (AF_UNIX en Windows).
  - La regla de que sólo `/health` puede vencer existe porque el puente repite una llamada que falló con `NotRunning` (`on_failure`, `mcp/startup.rs:36-41`), y un guardado vencido pudo haber llegado ya al daemon. Con el timeout fuera de la firma de `send`, ninguna llamada puede pedir uno a través del embudo: la regla la sostiene la estructura. `only_health_sets_a_timeout` (`client.rs:1569-1576`) queda de resguardo contra un `tokio::time::timeout` puesto a mano alrededor de otra llamada.
  - Cubre el mismo tramo que hoy. El `.timeout()` de reqwest corre desde que la petición empieza a conectar hasta que termina el cuerpo de la respuesta (`reqwest-0.12.28/src/async_impl/request.rs:286-291`, y `ClientBuilder::timeout` en `src/async_impl/client.rs:1435-1441`) **[código]**, y `send` lee el cuerpo entero en `Answer`, así que un timeout alrededor de `send` cubre lo mismo. Cambia sólo cómo se lee un vencimiento mientras llega el cuerpo de `/health`: hoy `read_response` lo da como `Http`, y pasa a dar `NotRunning`, como cualquier otro vencimiento de `health`. `/health` no escribe nada, así que repetirlo no duplica nada.
- **El `Connector`** (`daemon/socket/windows.rs`) implementa `tower_service::Service<Uri>` con `connect(path)` y `TcpStream::from_std`. Devuelve `TokioIo<tokio::net::TcpStream>`, que ya es `Connection` (`hyper-util-0.1.20/src/client/legacy/connect/http.rs:568-580`, `:598-605`) **[código]**. Allí `peer_addr` falla y queda un `Connected` simple, así que no hace falta envoltorio. El `connect` es bloqueante dentro del runtime, como el conector de la puerta **[puerta]**: un AF_UNIX local queda encolado sin esperar al `accept`. Llega con la rama hyper de `Transport`, que es su único uso: es `pub(crate)`, y sin ella sería `dead_code` en Windows. Los tests del listener (D2) hablan HTTP con un apretón de manos de hyper sobre `connect` que sólo existe en `#[cfg(test)]`, y pasan al `Connector` cuando llega.

| Opción | Contra |
|---|---|
| **Un `send` interno (elegido)** | — |
| Un `enum` que cada llamada desarma | 23 `match`: el mapeo de errores y la URL se separan por transporte, que es lo que la proposición pide evitar. |
| Un trait con dos implementaciones | Una abstracción para dos casos cerrados que un `enum` privado ya cubre. |
| `Call.timeout`, con un `.with_timeout()` que sólo usa `health` | La regla de que sólo `/health` vence volvería a descansar en un test que busca en el texto del código: cualquier llamada podría pedir su timeout con un método del mismo `Call`. |
| hyper para todos los transportes | Reescribe el camino de TCP y de Unix que 58 cerró y verificó; la proposición deja reqwest para ellos. |
| Conectar en `spawn_blocking` | Un salto de hilo en cada conexión nueva, que es justo el escenario del hook. |
| `connect` no bloqueante y esperar `writable` | No se verificó que el polling AFD avise la escritura de un AF_UNIX mientras conecta. |

`Call`, `Answer` y `send` son públicos porque los usa `binary.rs` (D6). `Connector` y `connect` son `pub(crate)`.

**Dependencias.** Sólo las usa la mitad de Windows, así que van en tablas `[target.'cfg(windows)']`, y el binario de Linux conserva las mismas dependencias directas:

- `socket2 = "0.6"`;
- `hyper = { version = "1", features = ["client", "http1"] }`;
- `hyper-util = { version = "0.1", features = ["client-legacy", "http1", "tokio"] }`;
- `http-body-util = "0.1"`;
- `tower-service = "0.3"`;
- `Win32_Networking_WinSock` en las features de `windows-sys` (`crates/clawg-cli/Cargo.toml:62`), que socket2 ya compila (`socket2-0.6.3/Cargo.toml:96-104`).

Las versiones y las features son las que el lock ya resuelve (tabla de la proposición). Cada una entra con su primer uso, en la tabla que ese uso pide:

- Con el listener (D2), `socket2` y la feature de WinSock van en `[target.'cfg(windows)'.dependencies]`: los usa el producto, en el listener y en `lacks_af_unix` (D3).
- Con el listener también, `hyper`, `hyper-util` y `http-body-util` van en `[target.'cfg(windows)'.dev-dependencies]`. Hasta que llega el `Connector` sólo los usa el apretón de manos de `#[cfg(test)]` de los tests del router, y el producto no depende de lo que no usa.
- Con el `Connector` (D4), los tres pasan a `[target.'cfg(windows)'.dependencies]` con la misma versión y las mismas features, y entra `tower-service`.

`Cargo.lock` nombra las dependencias directas de `clawg-cli` en una sola lista, las de desarrollo y las de cada sistema incluidas, sin decir de qué clase es cada una: `minisign` y `tempfile`, que sólo están en `[dev-dependencies]`, y `libc`, que sólo es de Unix, figuran al lado de `reqwest` (`Cargo.lock:345-373`) **[código]**. Así que suma cada nombre en el commit que lo trae, y el paso de `dev-dependencies` a `dependencies` no lo cambia. Se escribe con `cargo update --workspace`, que no compila, y va en ese mismo commit. De los manifiestos, `tests/conventions.rs` sólo mira que `testing` se encienda en una tabla que termina en `dev-dependencies]` (`testing_outside_dev_dependencies`, `:89-104`), y su propio test ya incluye una tabla `[target.'cfg(unix)'.dev-dependencies]` (`:113`): la tabla de Windows no lo toca.

## D5. `endpoint`, `tcp_plan`, `serve_port` y `tcp_only` leen la marca del daemon, no el sistema

- **`endpoint(state, base_dir, tcp_only)`** (`client.rs:621-626`) pierde el `cfg!(unix) &&`. La firma no cambia y la función sigue siendo pura.
- **`tcp_plan(explicit, profile_port, socket, stand_in)`** (`serve.rs:286-301`) no cambia. Cambia quién calcula `socket`: ahora es `bind_profile_socket` en todos los sistemas. Se van los `cfg(unix)` de `serve.rs:26-27`, `:51-53`, `:163-168`, `:234-245` y `:313`.
- **`serve_port(old_port, socket, tcp_only)`** (`update/effects.rs:295-301`). El segundo argumento era `cfg!(unix)` (`:431`, `:458`) y pasa a decir si el daemon que corría publicó su socket. `Running` (`update/plan.rs:11-16`) gana `socket: bool`, que `running_daemon` lee de `daemon.json` junto con el puerto (`update/mod.rs:133-134`).
  - Así se cubre el daemon que sirve sólo TCP con `--port` porque su ruta no entra. Hoy en Unix `serve_port` también le da `None` (`effects.rs:296`), y el daemon nuevo, al no tener `--port`, se niega a arrancar (`serve.rs:320-324`). El mismo cambio cierra ese hueco de Unix.
  - El comentario de `wait_for_end` que quedó encima de `serve_port` (`effects.rs:287-289`) vuelve a su función.
  - La comprobación de `binary.rs` de que el daemon nuevo de la actualización completa no publica `port` (`:1832-1836`) deja de ser de Unix con este cambio, y no antes: hasta entonces Windows le pasa al daemon nuevo el puerto del viejo (D6).
- **`tcp_only()`** (`build_info.rs:142-152`) ya era de todos los sistemas. `bind_profile_socket` lo mira primero (`serve.rs:315-317`), y desde ahora también en Windows.

| Alternativa para `serve_port` | Por qué no |
|---|---|
| `cfg!(any(unix, windows))`, o quitar el argumento | Pierde el caso del daemon que sirve sólo TCP: arrancaría sin `--port` y fallaría. |
| Probar AF_UNIX al actualizar | Mete I/O en la decisión y tampoco cubre la ruta demasiado larga. |

## D6. `binary.rs` en Windows

- `socket_only()` (`binary.rs:166-172`) devuelve `CLAWG_TEST_DEFAULT_PORT=off` en todos los sistemas.
- Pasan a `DaemonClient::from_data_dir` y `send` (D4): `reach` (`:342-352`), el `reqwest::Client` de `Daemon` (`:199`, que crea `Daemon::watch`, `:216`), `published_health` (`:1362-1374`) y `published_send` (`:1396-1409`). El test llega como el producto, por el socket o por el puerto que elige `endpoint`, sin una segunda copia de esa elección. `published` sigue comprobando antes el pid de `daemon.json`.
- Pasan a correr en los dos sistemas:
  - `serve_as` (`:105-116`), que no tiene nada de uid: son argumentos y entorno;
  - `bridge_starting_its_daemon` (`:131-134`);
  - `SERVE_WITHOUT_PORT` y los cinco escenarios del socket (`:721-833`), cuya sección deja de decir «(Unix)»;
  - `OLDER_TRANSPORT` y sus dos escenarios (`:1842-1886`).
- La comprobación de que el daemon nuevo de la actualización completa no publica `port` (`:1832-1836`) conserva su `cfg(unix)` hasta que `serve_port` lea la marca (D5), y lo pierde en ese mismo commit: antes, Windows le pasa al daemon nuevo el puerto del viejo, y `daemon.json` lo publica.
- Quedan sólo en Unix:
  - el modo del directorio de datos (`:488-526`) y el `0600` del export (`:696-697`);
  - SIGTERM (`:596-613`);
  - el `chmod` de `write_executable` (`:1097-1101`);
  - los escenarios de `setup` con un `claude` de shell (desde `:1916`);
  - en `socket/unix.rs`, `admits_peer` y el test por reqwest con `unix_socket`.

  En `binary.rs` no hay ningún escenario de uid.

| Opción | Contra |
|---|---|
| **`DaemonClient` con `send` (elegido)** | Tres ítems públicos más (`Call`, `Answer`, `send`). |
| Un cliente hyper en el test, con el `Connector` público | Otra pila HTTP y otra copia de `endpoint` en el test. |
| HTTP/1.1 a mano sobre el stream | Un parser frágil escrito sólo para los tests. |

## D7. El deny de `FILE_WRITE_DATA`: con `icacls`

Es un test unitario en `socket/windows.rs`:

1. hace `bind` en un tempdir;
2. corre `icacls <sock> /deny <USERDOMAIN>\<USERNAME>:(WD)`;
3. comprueba que `connect` falla con 10013 (`WSAEACCES`, `.../WinSock/mod.rs:5215`);
4. corre `icacls <sock> /remove:d <USERDOMAIN>\<USERNAME>` y comprueba que `connect` entra. Eso prueba que el fallo vino del ACE y deja que el `Drop` despierte al hilo.

| Opción | Contra |
|---|---|
| **`icacls` (elegido)** | Lanza un proceso de `System32`. Sólo se mira el código de salida, no la salida, que depende del idioma. |
| La API (`SetNamedSecurityInfoW` y el SID del token) | Necesita `Win32_Security_Authorization`, que ningún crate del build activa (tokio la pide sólo en dev-dependencies, `tokio-1.51.0/Cargo.toml:935-939`), y suma `unsafe` de FFI en un test. |

**[sin verificar]**: la puerta midió el deny con un token de integridad media sin elevar, y el runner de CI corre como administrador. Un ACE deny sobre el SID del usuario también aplica a un token elevado, y `connect` no usa semántica de backup, pero eso no se midió.

## D8. El arnés de la barra: un test `#[ignore]` de Windows

Vive en `crates/clawg-cli/tests/transport_bar.rs`, con `#![cfg(windows)]` e `#[ignore = "…"]`, como el precedente de `crates/clawg-core/tests/e5_golden.rs:265`. El CI no pasa `--ignored` (`.github/workflows/ci.yml:67`).

- **El daemon.** Arranca uno de `CARGO_BIN_EXE_clawg`, que es un build `testing`, con `serve --profile dev --port 0`, `CLAWG_TEST_HOME` en un tempdir y el modelo falso. Espera a que `daemon.json` tenga `unix_socket: true` y `port`, y a que el embedder esté `ready`.
- **Los clientes.** Los dos lados usan `DaemonClient::from_data_dir`, como un hook (`hook.rs:154`): AF_UNIX sobre el directorio del daemon, y TCP sobre una vista con una copia del `token` y un `daemon.json` que sólo lleva `port`. Así los dos lados pagan las mismas lecturas de archivo en los escenarios de conexión nueva, y no hace falta ninguna API más.
- **Los escenarios.** Cinco, con la misma barra del 10 %. Los cuatro primeros usan `health()`:
  - secuencial, sobre un solo cliente;
  - un cliente nuevo por petición;
  - 8 tareas sobre un cliente compartido;
  - una ráfaga de 32 clientes nuevos con barrera, durante 20 rondas.

  El quinto lleva un cuerpo real: el puente secuencial con un cliente reutilizado y una respuesta de unos 8 KB. La llamada es `DaemonClient::list` sobre memorias sembradas: antes de medir, el arnés guarda con `DaemonClient::save` memorias de texto distinto (el modelo falso las embebe sin red), espera a que `health().ingest` no tenga pendientes y calibra el `limit` y la cantidad sembrada hasta que `send` sobre `GET /v1/memories` devuelva un `Answer.body` de entre 7 y 9 KB. El arnés comprueba ese tamaño y falla si no entra. Los dos transportes leen el mismo cuerpo del mismo daemon.

  Cada uno empieza con un precalentamiento y sigue con 3 corridas por transporte, alternadas; vale la corrida de media mediana. Hay pocas conexiones TCP por corrida, para no agotar los puertos efímeros con TIME_WAIT, que fue lo que contaminó una corrida de la puerta.
- **El resultado.** Imprime la tabla: media y p99 por transporte y la razón entre las dos. Falla si alguna de las cinco medias de AF_UNIX supera a la de TCP en más de un 10 %, o si la ráfaga de AF_UNIX tiene una sola falla. Las fallas de TCP se informan sin hacer fallar el test.
- **Cómo se corre.** Optimizado, como en la puerta: `cargo test -p clawg-cli --release --test transport_bar --target-dir target-test -- --ignored --nocapture`.

Se descartaron un binario de clawg-eval, que no tiene `CARGO_BIN_EXE_clawg` y cuyo daemon dentro del proceso no es el daemon real que pide la barra, y un `example`, que tampoco tiene la ruta del binario y que `cargo test` compila igual.

## D9. Apagado y limpieza

- **Apagado ordenado.**
  1. Llega la señal.
  2. axum suelta el listener (D2), y el hilo termina y cierra el socket que escucha.
  3. Las peticiones en vuelo siguen dentro de `HTTP_GRACE`: las conexiones aceptadas no dependen del listener.
  4. Se drena la cola.
  5. `clear_state` borra `clawg.sock` con el candado tomado (`lifecycle.rs:163-171`).

  Para entonces el hilo ya terminó. Si no, `remove_file` falla en silencio y el próximo arranque borra el archivo como huérfano.
- **Proceso matado.** El archivo queda: conectarse da 10061 y un `bind` encima da 10048 **[puerta]**. El próximo daemon lo borra con el candado tomado (`bind_socket`).
- **`stop`, `update` e `install.ps1`** no cambian, salvo que ahora llegan por el socket. `STOP_TIMEOUT` y sus fases tampoco (`stop.rs:67-72`): el hilo no suma una fase.
- **[sin verificar]**: si se puede borrar el archivo mientras el socket que escucha sigue abierto. El orden de arriba hace que no haga falta.

## Archivos

| Archivo | Cambio |
|---|---|
| `crates/clawg-cli/src/daemon/socket.rs` | Pasa a `daemon/socket/mod.rs` (D1), con `unix.rs` y `windows.rs` (D2, D3, D4, D7) |
| `crates/clawg-cli/src/daemon/mod.rs` | `pub mod socket` sin `cfg(unix)` (`:9-10`) |
| `crates/clawg-cli/src/daemon/serve.rs` | `bind_profile_socket` en todos los sistemas, con `serves_without_socket`; se quitan los `cfg` (D5) |
| `crates/clawg-cli/src/client.rs` | `Transport`, `Call`, `Answer`, `send`, `call_url` y `read_response` pura; las 23 llamadas; `endpoint` sin `cfg!(unix)`; `socket_http` pasa a ser el transporte del socket de cada sistema (D4, D5) |
| `crates/clawg-cli/src/update/{effects,plan,mod}.rs` | `serve_port` con la marca y `Running.socket` (D5) |
| `crates/clawg-cli/tests/binary.rs` | D6, el escenario de apagado (D9) y el de `update` con un daemon que sirve sólo TCP (D5) |
| `crates/clawg-cli/tests/transport_bar.rs` | Nuevo (D8) |
| `crates/clawg-cli/Cargo.toml`, `Cargo.lock` | Dependencias (D4): `socket2` y WinSock con el listener; `hyper`, `hyper-util` y `http-body-util` en `dev-dependencies` hasta el `Connector`, y en `dependencies` con él y con `tower-service` |
| `crates/clawg-core/src/paths.rs` | El comentario de `socket_file` deja de decir «Unix» (`:225`) |
| `.claude/CLAUDE.md`, `README.md` | El socket y su prueba también en Windows (proposición, Alcance 6). Las specs se tocan al archivar |

## Pruebas, en orden de TDD

Cada paso empieza por su test en rojo y es un commit que compila y pasa la suite.

1. **D1.** Hacer multiplataforma `the_socket_path_fits_under_the_limit_and_a_longer_one_is_named` (con 108 en Windows, `socket.rs:138-148`). Sumar `a_non_ascii_path_counts_its_utf8_bytes`, que vale para los dos sistemas, y la tabla de `serves_without_socket`, partida por sistema (D1). En Windows el módulo compila sólo lo puro: `bind_socket` y `type Listener` siguen siendo de Unix hasta el paso 2. Hasta entonces, en Windows, `serves_without_socket` sólo tiene uso en los tests, y compila sin `dead_code` porque es `pub`, como el resto del módulo.
2. **D2.** Sumar en `windows.rs`:
   - el router por AF_UNIX detrás del guardia, con un apretón de manos de hyper sobre `connect` que sólo existe en `#[cfg(test)]`: otro `Host` da 403, un `Origin` da 403 y `/v1` sin token da 401;
   - `dropping_the_listener_closes_its_socket`: después del `Drop`, `connect` da `ConnectionRefused` en menos de 1 s;
   - el deny de D7, `a_socket_whose_file_denies_write_data_refuses_the_connect`, y a su lado `the_socket_file_inherits_the_data_directory_acl` (R10).

   Con `bind_socket` en los dos sistemas, `a_socket_an_earlier_daemon_left_is_replaced` (`socket.rs:150-158`) se vuelve multiplataforma.
3. **D3 y el cableado.** Sumar `lacks_af_unix` (10047 da sí; 10013, 10048 y 10022 dan no) y el daemon de Windows con su socket. El cliente todavía va por `port`, así que nada se rompe.
4. **D4, sólo con reqwest.** Sumar los tests puros de `read_response` (401, error con cuerpo, error sin JSON, 2xx inválido) y `call_url` comparado con `reqwest::RequestBuilder::query().build()`. Reescribir `only_health_sets_a_timeout` (`client.rs:1569-1576`), que hoy exige que la única línea con `.timeout(` del código que no es de test sea `            .timeout(HEALTH_TIMEOUT)`, dentro de `pub async fn health`: pasa a exigir que el único `timeout(` del código que no es de test esté dentro de `health`. Va antes del refactor y en el mismo commit, porque el refactor le cambia la forma a esa línea. Después, el refactor del embudo, sin cambiar comportamiento salvo el vencimiento de `/health` mientras llega el cuerpo (D4). Toda la suite por TCP sigue en verde.
5. **D4 y D5, AF_UNIX y `endpoint`.** Se vuelven multiplataforma `the_socket_wins_when_published_and_tcp_is_used_otherwise`, `a_client_reaches_a_daemon_over_its_published_socket` y `a_client_on_a_published_socket_with_nothing_behind_is_not_running`, este último también con el huérfano (`client.rs:1621-1685`). Se suman `thirty_two_simultaneous_clients_over_the_socket_are_none_not_running` y un `health` contra un socket que nunca contesta, que da `NotRunning` dentro de `HEALTH_TIMEOUT`. El `Connector` llega en este paso, con `tower-service`; `hyper`, `hyper-util` y `http-body-util` pasan de `dev-dependencies` a `dependencies`, y los tests del router de `windows.rs` pasan a usarlo.
6. **D6.** Cambiar `socket_only` y `reach`, y quitar los `cfg` de los escenarios, salvo la comprobación de `port` de la actualización completa (`binary.rs:1832-1836`), que espera al paso 7. Tiene que ir antes que el paso 7, o en el mismo commit: si no, el daemon que lanza `update` en Windows tomaría el puerto del perfil de la máquina. Se escribe además `a_taken_explicit_port_is_fatal_and_publishes_nothing`, el escenario «`--port` ocupado» de R8, que hoy no tiene test en ningún sistema: ocupa un puerto N, corre `serve --port N` y espera una salida con error sin `daemon.json` ni `token`, que `run` escribe después del `bind` (`serve.rs:183-197`). No mira el socket: `run` lo enlaza (`serve.rs:164`) antes de que el `bind` de TCP vuelva con `?` (`:171`), y ningún `Drop` lo borra. Eso no hace daño: sin `daemon.json` ningún cliente se conecta a él, y el próximo arranque lo borra con el candado tomado.
7. **D5, `serve_port`.** El caso «sin socket conserva el puerto» (`effects.rs:496-500`, ahora con `socket` en lugar de `unix`) y los `Running` de `plan.rs:305` y `:451`, con `socket`. A nivel del binario, `an_update_keeps_the_port_of_a_daemon_that_serves_tcp_alone` (`binary.rs`, Unix y Windows): un daemon con `--port` y una ruta que no entra sirve sólo TCP, `update` lo reinicia con su puerto y el daemon nuevo contesta. Falla hoy en Unix. En el mismo commit, la comprobación de `port` de `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` (`binary.rs:1832-1836`) pierde su `cfg(unix)`.
8. **D9.** `a_shutdown_ends_the_socket_listener_within_its_bounds` en `binary.rs`: `stop` sale con 0, dentro de `STOP_TIMEOUT`, y el socket desaparece.
9. **D8.** El arnés y la documentación.

## Sin verificar

- El despertar por autoconexión (D2): lo cubre el paso 2.
- 10047 en una build real sin AF_UNIX (D3).
- Qué hace un `connect` bloqueante con el backlog lleno: si espera o si rechaza. Con 512 clientes la puerta no tuvo fallas.
- El deny con `icacls` bajo el token elevado del CI (D7).
- Borrar el archivo con el socket que escucha todavía abierto (D9).
- Que mio siga manejando un AF_UNIX como `TcpStream` en futuras versiones de tokio o mio. Lo cubre el CI de Windows.
- La segunda cuenta y la barra sobre el producto: van a `verify.md`.
