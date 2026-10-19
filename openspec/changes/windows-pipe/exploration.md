# windows-pipe — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M–L | local-transport (58); con puerta: una prueba en Windows antes de la propuesta | exploración (borrador) |

**Tesis:** en Windows no hay sockets Unix en este stack, así que el equivalente de local-transport (58) es una named pipe. El espacio de nombres `\\.\pipe\` es de la máquina, no del usuario: un nombre fijo choca entre usuarios igual que el puerto y queda para el primero que lo cree. Propuesta: un nombre aleatorio por arranque publicado en `daemon.json`, la primera instancia exclusiva, sin clientes remotos, y un cliente que comprueba que el servidor de la pipe es el dueño vivo del candado del store. Varias premisas salen de la documentación de Microsoft y del código de tokio y reqwest, sin ejecutar en Windows: el ACL por defecto de una pipe, si otra cuenta puede listar los nombres y qué pasa con `ERROR_PIPE_BUSY`, que reqwest no reintenta. Por eso el cambio empieza por una prueba en una máquina Windows que decide si sigue.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| tokio sirve y abre named pipes | **Cierta.** `tokio::net::windows::named_pipe` (tokio 1.51.0): `ServerOptions` con `reject_remote_clients(true)` por defecto (`named_pipe.rs:1773`), `first_pipe_instance` (`:2051`), `max_instances` sin límite por defecto (`:1774`), y un ACL propio sólo por `unsafe fn create_with_security_attributes_raw` (`:2304`); `ClientOptions` abre en lectura y escritura con `SECURITY_IDENTIFICATION` (`:2392-2400`). No hay ayudante para el pid del par. |
| reqwest habla por una pipe | **Cierta.** `ClientBuilder::windows_named_pipe` (`reqwest-0.12.28/src/async_impl/client.rs:1826`, `#[cfg(target_os = "windows")]`), sin proxy ni DNS. Abre con `ClientOptions::new().open(pipe)` sin reintento (`connect.rs:716-728`) y no expone el handle de la pipe. |
| axum sirve sobre una pipe | **Cierta con un `Listener` propio.** El trait pide `accept` y `local_addr` (`axum-0.8.9/src/serve/listener.rs:9-24`); al aceptar, el listener crea la instancia siguiente y devuelve la conectada, que es el patrón de la documentación de tokio. |
| AF_UNIX de Windows sería más simple | **Falsa para este stack.** mio 1.2.0 no tiene sockets Unix en `src/sys/windows/`, `UnixListener` y `unix_socket` de reqwest son `#[cfg(unix)]`, y en `std` es un PR abierto e inestable (rust-lang/rust#147335, feature `windows_unix_domain_sockets`, consultado 2026-10-19), con un MSRV del workspace de 1.77.2 (`Cargo.toml:11`). Haría falta un puente propio con hilos o IOCP. |
| El ACL por defecto impide que otro usuario mande una petición | **Sin verificar.** Según Microsoft (*Named Pipe Security and Access Rights*): control total a LocalSystem, a los administradores y al creador, y sólo lectura a Everyone y a anónimo; crear otra instancia de una pipe que existe exige `FILE_CREATE_PIPE_INSTANCE`. Como el cliente abre en lectura y escritura, otra cuenta no podría mandar nada ni interponerse mientras el daemon vive. Es lo primero que prueba la puerta. |
| Otra cuenta puede ocupar instancias abriendo sólo en lectura | **Plausible, sin ejecutar.** No filtra datos (el servidor sólo escribe respuestas a peticiones), pero cada conexión retiene una instancia y un handle, como las conexiones a medio abrir sobre TCP. |
| reqwest lee `ERROR_PIPE_BUSY` como «no hay daemon» | **Plausible, por código.** La documentación del cliente de tokio pide reintentar `ERROR_PIPE_BUSY` (`named_pipe.rs:943-972`) y reqwest no lo hace. El error sale del conector, que es lo que `is_connect()` marca, y `map_transport` (`crates/clawg-cli/src/client.rs:644`) lo convierte en `NotRunning`. Pasa cuando llega un cliente entre que el servidor acepta una conexión y crea la instancia siguiente: el puente con varias herramientas en paralelo y un hook a la vez. |
| `windows-sys` trae lo necesario sin crates nuevos | **Cierta.** `GetNamedPipeServerProcessId` y `GetNamedPipeClientProcessId` están en `Win32_System_Pipes` (windows-sys 0.61.2). `clawg-cli` ya usa windows-sys 0.61 con `Win32_Foundation`, `Win32_System_Threading` y `Win32_UI_WindowsAndMessaging` (`crates/clawg-cli/Cargo.toml:62`). Suma features (`Win32_System_Pipes`, y `Win32_Security` si se comparan SIDs) y `unsafe` nuevo. |
| `daemon.json` es privado en Windows | **Por la ACL del perfil, sin verificar.** Es la decisión escrita en `crates/clawg-cli/src/daemon/lifecycle.rs:87-93`. El nombre aleatorio se apoya en eso. |
| Una pipe huérfana impide arrancar | **Falsa.** Una named pipe es un objeto del kernel que desaparece cuando se cierra su último handle, así que un daemon muerto no deja nada que borrar. Es también lo que permite que otro la cree con el mismo nombre después. |

## Problema (verificado contra el código)

En Windows pasa lo mismo que 58 cierra en Unix: el daemon escucha en el puerto fijo del perfil (`crates/clawg-core/src/paths.rs:139-144`), otra cuenta puede ocuparlo cuando el daemon muere sin apagarse y recibir el token y los saves del siguiente cliente, `/health` es legible por cualquier cuenta y dos sesiones de usuarios distintos chocan en el mismo puerto. En un equipo de un solo usuario la ganancia de seguridad es chica; lo que sí gana cualquier máquina Windows es que el autoarranque se pueda probar a través del binario en el CI de Windows, sin el puerto fijo.

## Lo que ya existe y se reutiliza

- De local-transport (58): `endpoint`, el campo de `daemon.json`, la regla de versiones (`--port` explícito enciende TCP; transición con los dos; TCP sólo por opt-in una minor después) y el sello de `testing` que emula un clawg anterior.
- `StoreLock::current_owner` (`crates/clawg-core/src/memory/store_lock.rs:190-203`): distingue un dueño vivo de un resto de un proceso muerto, porque el SO suelta el candado con el proceso. Es contra lo que se compara el pid del servidor de la pipe.
- El autoarranque con `DETACHED_PROCESS` y su propio grupo de procesos (`client.rs:1158-1175`), independiente del transporte.
- El CI de Windows (`.github/workflows/ci.yml`), que corre la suite.

## Enfoques posibles

### El nombre

**`\\.\pipe\clawg-<perfil>-<128 bits aleatorios>`, regenerado en cada arranque y publicado en `daemon.json` (recomendado).** No choca entre usuarios ni entre perfiles, y otra cuenta no lo puede adivinar. Un nombre fijo (`clawg-prod`) se descarta: choca como el puerto y queda para quien lo cree primero.

### Defensas

| Defensa | API | Qué cierra | Coste |
|---|---|---|---|
| Rechazar clientes remotos | `reject_remote_clients(true)`, ya por defecto | Conexiones por SMB | Ninguno |
| Primera instancia exclusiva | `first_pipe_instance(true)` | Que el daemon arranque detrás de una pipe que ya creó otro (falla con `ERROR_ACCESS_DENIED`, según la documentación de `CreateNamedPipe`) | Ninguno |
| ACL por defecto | nada | Si la puerta lo confirma, que otra cuenta mande peticiones o agregue instancias | Ninguno |
| El cliente verifica al servidor (recomendado) | `GetNamedPipeServerProcessId` contra `StoreLock::current_owner` | Un impostor que creó la pipe con el nombre de un `daemon.json` viejo, después de que el daemon muriera | `unsafe` acotado. Como reqwest no expone el handle, es una conexión previa propia, en el mismo punto que construye el cliente, y otra vez en cada relectura de `daemon.json` del puente |
| El servidor verifica al cliente | `GetNamedPipeClientProcessId` y el SID del token del proceso | Que otra cuenta retenga instancias abriendo en lectura | Más `unsafe` (`OpenProcessToken`, `GetTokenInformation`, `EqualSid`); un proceso elevado del mismo usuario puede no dejarse abrir |
| ACL explícito, sólo el dueño | `create_with_security_attributes_raw` | La lectura de Everyone | `unsafe` y `Win32_Security`; local-isolation lo descartó sin evidencia de que la ACL del perfil falle |

La ventana entre la verificación y la conexión de reqwest no la aprovecha otra cuenta: mientras el daemon real vive, nadie sin `FILE_CREATE_PIPE_INSTANCE` agrega instancias (si la puerta lo confirma).

### `ERROR_PIPE_BUSY`

(a) El servidor mantiene varias instancias libres en lugar de una, sin `unsafe`. (b) `DaemonClient` reintenta un `is_connect()` sobre la pipe un par de veces, con 50 ms entre intentos; es seguro porque la petición nunca llegó al daemon. (c) Las dos (recomendado).

### TCP en Windows

La misma regla de versiones que 58. Para depurar, `curl` no habla named pipes, así que lo cómodo en Windows sigue siendo `clawg serve --port N`.

## Descartado

| Opción | Razón técnica |
|---|---|
| AF_UNIX en Windows | Sin soporte en tokio 1.51.0 ni en reqwest 0.12.28, inestable en `std`; un puente propio no gana nada sobre la pipe. |
| Un nombre de pipe fijo por perfil | El espacio de nombres es de la máquina: choca entre usuarios y queda para el primero que lo cree. |
| Caer a TCP cuando la pipe falla | Un proceso de otra cuenta en el puerto vuelve a ser el daemon. |
| ACL explícito sin evidencia | `unsafe` para cerrar algo que el ACL por defecto podría ya cerrar; se reabre sólo si la puerta muestra que otra cuenta escribe o retiene instancias. |

## Interacciones

- **local-transport (58).** Este cambio va después y reusa su descubrimiento, su regla de versiones y su sello de prueba; la spec `daemon-access` se reescribe primero allí y acá suma lo de Windows.
- **shutdown-budget (55).** Si un proceso lanzado con `DETACHED_PROCESS` recibe los eventos de apagado de Windows es una duda de 55. Con la pipe no hay nada que borrar al salir; sí siguen `daemon.json` y `token`.
- **Backlog «El cliente sólo pone timeout en `/health`».** El reintento de `ERROR_PIPE_BUSY` toca la misma `map_transport`: conviene mirarlos juntos para que ni un timeout ni una pipe ocupada se repitan donde no deben.
- **doctor-checks (31).** Puede informar el descriptor de seguridad de la pipe del daemon que corre.
- **`scripts/install.ps1`.** Llama al `stop` y al `status` del binario anterior (`:262-266`), que habla con el daemon que corre, de su misma versión: no cambia nada mientras la regla de versiones se respete.

## Alcance candidato

0. La puerta (Cómo se mide, paso 0).
1. Un `Listener` de axum sobre `NamedPipeServer`, con nombre aleatorio, primera instancia exclusiva e instancias libres.
2. El nombre publicado en `daemon.json` y `endpoint` con la variante `Pipe(name)`; el cliente con `windows_named_pipe` y `http://localhost`.
3. La verificación del servidor contra el dueño del candado, en el punto que construye el cliente y en cada relectura de `daemon.json`.
4. El reintento acotado de `ERROR_PIPE_BUSY`.
5. La verificación del cliente por SID, sólo si la puerta confirma que otra cuenta retiene instancias.
6. Specs y documentación (`specs/daemon-access`, `.claude/CLAUDE.md`, `README.md`).

### No-alcance

- AF_UNIX en Windows.
- Un ACL explícito, salvo que la puerta lo pida.
- WSL: una pipe de Windows no se alcanza desde Linux en WSL2 sin un relé, y no hay un puente de un lado contra el daemon del otro.

## Decisiones abiertas

Son del usuario.

- [ ] **La puerta.** La prueba con una segunda cuenta local es manual; lo demás se automatiza en el CI de Windows.
- [ ] **La verificación del servidor.** Siempre (recomendado) o sólo si otra cuenta puede listar `\\.\pipe\`.
- [ ] **La verificación del cliente por SID.** Sólo si se confirma la retención de instancias.
- [ ] **Cuántas instancias libres.** Lo decide la prueba de concurrencia.
- [ ] **En qué versión sale.** Junto con la segunda versión de 58 o en una aparte.

## Cómo se mide

0. **Puerta, en una máquina Windows, antes de la propuesta:**
   - El descriptor de seguridad de una pipe creada con `ServerOptions` por defecto, leído con `GetSecurityInfo`: automatizable en el CI.
   - Con una segunda cuenta local: abrir la pipe del daemon en lectura y escritura (se espera `ERROR_ACCESS_DENIED`) y sólo en lectura; listar `\\.\pipe\` (`[System.IO.Directory]::GetFiles("\\.\pipe\")`) y ver si aparece el nombre aleatorio.
   - 32 peticiones concurrentes con 1, 2 y 4 instancias libres, contando los `NotRunning`.
   - Criterio: si otra cuenta puede escribir en la pipe, el cambio necesita el ACL explícito o se cierra sin código; si no, sigue.
1. **CI de Windows:** la suite, y 32 peticiones concurrentes sin ningún `NotRunning`.
2. **Verificación del servidor:** un test crea, desde un proceso que no tiene el candado, una pipe con el nombre de `daemon.json`; el cliente no le manda el token.
3. **Primera instancia:** un segundo servidor con el mismo nombre falla.
4. **`binary.rs` en Windows:** `serve`, `status`, `stop`, el puente por stdio y el autoarranque llegan por la pipe.
5. **Versiones:** los escenarios del sello de 58 corren también en Windows.

`rag-eval` no aplica.

## Riesgos

- **Todo lo de Windows está sin ejecutar:** el ACL, el listado de nombres, `ERROR_PIPE_BUSY` y la entrega de eventos de apagado. Por eso la puerta va primero.
- **`unsafe` nuevo** para el pid del servidor y, si se decide, para el SID del cliente.
- **El CI no tiene dos cuentas:** lo que depende de otro usuario queda manual, en el `verify.md`.
- **Reconexión del puente:** si el daemon muere en mitad de una sesión, la relectura de `daemon.json` encuentra el mismo nombre viejo; la verificación del servidor tiene que correr también ahí o el impostor entra por la reconexión.
- **Un proceso elevado del mismo usuario** puede no dejar abrir su token para comparar el SID.
