# daemon-hardening — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — | exploración (borrador) |

**Tesis:** en la API HTTP de loopback, el token de portador es la única barrera. Nada comprueba a qué nombre de host iba dirigida la petición ni si viene de una página web, `/health` responde sin token y el archivo del token queda protegido sólo por los permisos por defecto de la plataforma. Una página abierta en el navegador del usuario puede mandar peticiones a `127.0.0.1:<puerto>`; con DNS rebinding el navegador trata un nombre controlado por el atacante como del mismo origen, y lo único que lo frena es que no puede leer el token. Validar `Host` y rechazar los `Origin` de navegador cierra toda esa clase de ataque aunque un endpoint futuro se olvide del token, y crear el archivo del token ya restringido cierra una ventana chica. Es un cambio chico: un middleware puro con su tabla de decisión, y un `OpenOptions` con modo.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El router tiene `/health` abierto y `/v1` detrás del token | **Cierta.** `router` arma `/v1` con la capa `require_bearer` y deja `/health` fuera (`crates/clawg-cli/src/daemon/api.rs:538-561`). `require_bearer` compara en tiempo constante (`api.rs:588-604`, `constant_time_eq` en `:250-255`). |
| El listener sólo escucha en 127.0.0.1 | **Cierta.** `TcpListener::bind(("127.0.0.1", port))` (`crates/clawg-cli/src/daemon/serve.rs:107`) y `("127.0.0.1", 0)` en el servidor efímero (`api.rs:574`). Es sólo IPv4: una dirección `[::1]` no llega a este socket. |
| No hay manejo de Host, Origin ni CORS | **Cierta.** `rg -i "host\|origin\|cors"` sobre `crates/clawg-cli/src` no da ningún uso de esas cabeceras; los aciertos son comentarios sobre el "host" MCP (`detach.rs`, `mcp/handshake.rs`). No hay `tower-http` ni capa CORS en `crates/clawg-cli/Cargo.toml`. |
| `/health` dice más de lo que haría falta | **Cierta, pero casi todo lo usa alguien.** `HealthBody` lleva `status`, `version`, `profile`, `pid`, `embedder`, `embedder_error`, `embedder_retryable` y `uptime_s` (`api.rs:408-425`; handler `api.rs:610-626`). Consumidores en la sección de abajo. |
| El token son 32 bytes aleatorios en hexadecimal | **Cierta** (`TOKEN_BYTES = 32`, `crates/clawg-cli/src/daemon/lifecycle.rs:31`; `generate_token`, `:71-77`). |
| `write_token` crea el archivo y después lo restringe, así que hay una ventana | **Cierta.** `File::create` con los permisos por defecto, escritura del token y recién entonces `restrict_to_owner` (`lifecycle.rs:121-127`). En Unix el archivo existe con el `umask` y ya contiene el secreto hasta el `chmod` a 0600 (`lifecycle.rs:79-92`). Un archivo preexistente conserva su modo al abrirlo con `create`. |
| En Windows `restrict_to_owner` no hace nada | **Cierta, y es una decisión escrita:** "A no-op on Windows, and deliberately", con la razón de que el directorio personal ya excluye a otros usuarios sin privilegios (`lifecycle.rs:79-87,94-97`). **No verifiqué la ACL real** de `~/.clawg*` (la regla del repo prohíbe tocar esos directorios en esta exploración). |
| `daemon.json` recibe el mismo trato | **Cierta** (`lifecycle.rs:99-108`), con la salvedad de que es "público" por diseño y no lleva secreto (`lifecycle.rs:6-10`). |
| El transporte MCP es sólo stdio | **Cierta.** `rmcp` se usa con `server`, `transport-io`, `macros` y `schemars` (`crates/clawg-cli/Cargo.toml:47`). La superficie HTTP expuesta es la API del daemon; el puente MCP no escucha en ningún puerto. |
| Un nombre de host de lista permitida rompería a algún cliente | **No, con los clientes de hoy.** Ver "Quién habla con el daemon". |

## Problema (verificado contra el código)

### Qué barrera hay y cuál falta

Hoy una petición al daemon se acepta o se rechaza por una sola razón: si trae `Authorization: Bearer <token>` correcto en `/v1` (`api.rs:588-604`). Todo lo demás es público: `/health` se contesta a cualquiera que llegue al puerto (`api.rs:559-561`, y el test lo fija, `api.rs:1287-1288`).

Hechos de la plataforma, no del proyecto, que importan acá:

- Una página web puede hacer que el navegador mande una petición a `http://127.0.0.1:<puerto>`. Las simples (por ejemplo un `POST` con `text/plain`) salen sin permiso previo; la página no puede leer la respuesta, pero la petición llega al servidor.
- Con DNS rebinding el nombre del atacante pasa a resolver a `127.0.0.1`. Desde ese momento el navegador la considera **del mismo origen** y la respuesta es legible. La cabecera `Host` de esas peticiones es el nombre del atacante, no un nombre de loopback.
- El navegador manda `Origin` en los `POST`, `PATCH` y `DELETE`, también entre orígenes y también en los del mismo origen; una petición `GET` simple entre orígenes no lo lleva. Un cliente no navegador, como `reqwest`, no manda `Origin`.
- Una petición con `Authorization` desde otro origen exige una consulta previa (`OPTIONS`) que el servidor tiene que autorizar con cabeceras CORS. Este servidor no manda ninguna.

Qué falla entonces, para el daemon tal como está:

1. **Lo que se puede hacer sin token se puede hacer desde una página.** Es sólo `/health` hoy (versión, pid, estado del embedder y tiempo de actividad), pero la garantía es "ningún endpoint olvida el token", y es una convención, no un mecanismo.
2. **Con rebinding, un endpoint sin token es legible y escribible desde la página.** Un endpoint nuevo en el router externo al `nest("/v1")` heredaría la exposición.
3. **El token es la única defensa de `/v1`.** Si alguna vez se filtra (un log, una captura, un archivo legible por otro usuario durante la ventana de creación), una página con rebinding tendría la API entera.

La barrera adicional no depende del secreto: comprobar el nombre al que se dirige la petición (`Host`) corta el rebinding en origen, y rechazar `Origin` corta las escrituras desde páginas incluso cuando el nombre fuera de loopback.

### El archivo del token

- Unix: `File::create` aplica el `umask` del proceso, se escribe el token y recién después `set_permissions(0o600)` (`lifecycle.rs:121-127`, `:88-92`). En el medio, el archivo es legible según el `umask` (típicamente por el grupo y por el resto). El test actual mira el modo después de `write_token` (`lifecycle.rs:252-263`) y por eso no ve la ventana.
- El directorio de datos se crea con `create_dir_all` y el `umask` por defecto (`serve.rs:69`); si fuera 0700, la ventana del archivo no importaría. Es una alternativa a cambiar la apertura del archivo (ver opciones).
- Windows: sin ACL explícita. Lo sostiene la ACL del directorio personal, un hecho del sistema que este repo no controla.

### Quién habla con el daemon

| Cliente | Cómo llega | Cabecera `Host` que manda |
|---|---|---|
| Puente MCP, `clawg status`, `clawg stop`, `clawg memories`, el autoarranque | `DaemonClient::from_data_dir` arma `http://127.0.0.1:{puerto}` con el puerto de `daemon.json` y "siempre loopback" (`crates/clawg-cli/src/client.rs:429-444`) | `127.0.0.1:<puerto>` |
| `clawg stop` | usa `/health` para el pid (`crates/clawg-cli/src/stop.rs:25-27`) | idem |
| Aviso de daemon viejo | `version_warning` sobre `health.version` (`crates/clawg-cli/src/memories.rs:364-365`) | idem |
| Autoarranque del puente | `probe_running` decide que hay daemon sólo si `health()` responde (`client.rs:937-945,989`) | idem |
| `memory_status` | `health()` (`crates/clawg-cli/src/mcp/server.rs:767`) | idem |
| Arnés `rag-eval --agent-view` | `serve_ephemeral` y `DaemonClient::new(format!("http://{addr}"), ...)` (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:107`) | `127.0.0.1:<puerto efímero>` |
| Tests de la API | `reqwest` contra `http://{addr}` de un listener `127.0.0.1:0` (`crates/clawg-cli/tests/api.rs:207-220`) | idem |
| Hooks `clawg hook <evento>` (25, planificado) | por `DaemonClient`, por el diseño recomendado; la opción de hook `http` directo quedó descartada como primera opción por necesitar el token en la configuración (`openspec/changes/agent-lifecycle-hooks/exploration.md:94-99`) | `127.0.0.1:<puerto>` si usa el cliente; **sin verificar** si un hook HTTP del host agregaría otras cabeceras |

Ningún cliente actual manda `Origin` (`reqwest` no lo agrega) y ninguno usa un nombre distinto de `127.0.0.1`. **Una lista de `Host` y el rechazo de `Origin` no rompen a nadie hoy.**

### Qué consumen de `/health`

- `Health` del cliente exige `status`, `version`, `profile`, `pid`, `embedder` y `uptime_s`; sólo `embedder_error` y `embedder_retryable` tienen `#[serde(default)]` (`client.rs:116-130`).
- `pid`: `clawg stop` (`stop.rs:27`). `version`: el aviso de daemon viejo (`memories.rs:364-365`). `embedder`: `memory_status` y la espera del embedder (`mcp/server.rs:767`). Todo `Health`: `clawg status` lo vuelca entero (`crates/clawg-cli/src/status.rs:30-41`). El resto de las llamadas solo mira `is_ok()` (`client.rs:941,989`).
- `health()` **no manda token** hoy (`client.rs:490-498`).

Consecuencia: recortar `/health` sin token obliga a que `health()` mande el token, y un cliente nuevo contra un daemon viejo, o al revés, deja de interpretarse. Peor: un `clawg` viejo que recibe un `/health` recortado falla al deserializar (faltan `pid`, `embedder`, `uptime_s`), y `probe_running` lo lee como "no hay daemon" y puede intentar arrancar otro (`client.rs:937-945`). Es un riesgo real para una ganancia que el filtro de `Host` ya da.

## Lo que ya existe y se reutiliza

- La capa `axum::middleware::from_fn_with_state(app.clone(), require_bearer)` como patrón (`api.rs:538-561`): el nuevo guardia es una función del mismo estilo, aplicada al router externo para cubrir `/health`.
- `ApiError::new(StatusCode::..., "...")` para una respuesta uniforme (`api.rs:604`).
- `serve_ephemeral` (`api.rs:565-585`) y el listener de `tests/api.rs:207`, que arman el mismo `router(...)`: poner el guardia **dentro** de `router` hace que lo ejerciten el daemon, el arnés y los tests de la API sin cambiar ninguno.
- `restrict_to_owner` y su test Unix (`lifecycle.rs:79-97,252-263`), que se conservan para el archivo preexistente.
- `windows-sys` ya es dependencia de Windows (`crates/clawg-cli/Cargo.toml:56-57`), con las características `Win32_Foundation`, `Win32_System_Threading` y `Win32_UI_WindowsAndMessaging`.
- La separación de lo puro: una función `(host, origin) -> Allow | Reject(motivo)` sin E/S, como `Profile::resolve` (CLAUDE.md del repo).
- El límite escrito en `project-root-guard`: un agente con shell puede leer el token y llamar al daemon (`openspec/changes/project-root-guard/exploration.md:86`).

## Enfoques posibles

### A. Middleware de Host y Origin (recomendado)

Una capa en `router` que, **para toda ruta incluida `/health`**, rechaza con 403:

- una petición cuyo `Host` no sea un nombre de loopback de la lista (`127.0.0.1`, `localhost`, y quizá `[::1]`), con o sin puerto;
- una petición que traiga cualquier cabecera `Origin`.

La decisión es pura: `fn admits(host: Option<&str>, origin: Option<&str>) -> Result<(), Rejection>`, con la tabla de abajo.

| `Host` | `Origin` | Resultado |
|---|---|---|
| `127.0.0.1:<puerto>` | ausente | permitido |
| `localhost:<puerto>` | ausente | permitido |
| `evil.example:<puerto>` | cualquiera | 403 |
| `127.0.0.1:<puerto>` | presente (cualquier valor, incluido `null`) | 403 |
| ausente | ausente | decisión abierta (propuesta: 403) |
| `LOCALHOST` | ausente | permitido (los nombres no distinguen mayúsculas) |

- **Puerto: comprobarlo o no.** El DNS rebinding no se defiende con el puerto sino con el nombre, y la lista se evalúa sin conocer el puerto, así que `--port 0` (efímero) no afecta nada. Comprobarlo exigiría un campo nuevo en `AppCtx`, que hoy se construye en ocho lugares (`client.rs:1029`, `tests/api.rs:198`, `api.rs:1274`, `serve.rs:131`, `mcp/server.rs:1034`, `mcp/mod.rs:198`, `stop.rs:110` y el arnés `runner.rs:98`). En el daemon el puerto efectivo ya se conoce al construirlo (sale del bind, `serve.rs:107-113`, y `AppCtx` se arma en `:131`), pero en `serve_ephemeral` y en los tests el contexto se construye **antes** de abrir el socket (`api.rs:565-585`, `tests/api.rs:198-210`), así que ahí el puerto no existe todavía y habría que cambiar la firma. Es posible, pero cuesta ocho cambios más una firma por una garantía que el nombre ya da. Recomendado: sólo el nombre.
- A favor: cierra la clase entera, incluido un endpoint futuro sin token; no cambia el protocolo ni la API; ningún cliente actual se rompe; todo es puro y testeable sin red.
- En contra: sólo defiende contra navegadores. Cualquier proceso local, de cualquier usuario, que conecte al puerto con el `Host` correcto sigue limitado únicamente por el token. Si un cliente legítimo futuro manda `Origin` (por ejemplo un hook HTTP del host), hay que verlo antes. Un puerto de escucha IPv6 no existe, así que `[::1]` en la lista es inofensivo pero inútil.

### B. Crear el token con permisos desde el inicio

- **B1 (Unix): `OpenOptions::new().write(true).create(true).truncate(true).mode(0o600)`** para el token y para `daemon.json`, y conservar `restrict_to_owner` después, porque `mode` solo aplica al crear un archivo y un archivo preexistente (de una versión anterior, o dejado a mano) conserva el suyo. Así el secreto nunca está en disco con permisos más abiertos que 0600.
- **B2 (Unix): directorio de datos a 0700** en `create_dir_all`. Cubre también `memory.db`, los backups y los logs, que hoy heredan el `umask`. Es más amplio que el problema del token y toca la creación del directorio.
- **B3 (Windows): ACL explícita de dueño único.**
  - Con `windows-sys`: habría que sumar las características de seguridad (`Win32_Security` y `Win32_Security_Authorization` para fijar la DACL del archivo) a una dependencia que ya existe, con `unsafe` y una descripción de seguridad que construir a mano. No es una crate nueva, pero sí superficie `unsafe` nueva.
  - Con `icacls` como proceso externo: frágil (depende del idioma de la salida y de que exista).
  - Mantener la ACL del directorio personal: es lo que hace hoy, y es defendible por la razón escrita en `lifecycle.rs:79-87`.
- **Recomendación:** B1 sí. B2 queda como decisión abierta (más amplio). **B3: no por ahora.** Sumar `unsafe` por una garantía que la ACL del directorio personal ya da, sin haberla verificado en una máquina real, no se justifica en un cambio S. Si más adelante un chequeo de `doctor` la verifica o la contradice, se reabre con evidencia.

### C. Recortar lo que `/health` dice sin token

`/health` sin token devolvería sólo `{status}`; con token, todo.

- A favor: menos información para quien no tiene el token.
- En contra: obliga a que `health()` mande el token; rompe en ambos sentidos con versiones mezcladas (ver arriba); y la información que filtra hoy (versión, pid, uptime) la deja de poder leer una página una vez que A está puesto. **Descartado como parte de este cambio**: A le quita a C casi todo el valor, y C tiene un costo de compatibilidad concreto. Se reabre si alguna vez `/health` suma un dato sensible.

### D. Pipes con nombre o sockets Unix

Cambiar el transporte para que no haya puerto TCP.

- A favor: sin puerto, no hay navegador ni rebinding.
- En contra: Windows primero (pipes con nombre), una abstracción nueva de transporte, `DaemonClient`, el autoarranque, el arnés y los tests apoyados en `127.0.0.1:0`, y la API HTTP deja de ser reutilizable tal cual. Es un cambio L, no S, para una superficie que A cierra. **Descartado por ahora.**

**Recomendación:** A y B1 en el mismo cambio. B2 a decidir. B3, C y D fuera.

## Descartado

| Opción | Razón técnica |
|---|---|
| Comprobar el puerto además del nombre | Ocho construcciones de `AppCtx`, y `serve_ephemeral` y los tests arman el contexto antes de tener socket. Sin ganancia sobre el nombre. |
| Agregar cabeceras CORS permisivas "para que funcione" | Haría legibles las respuestas desde cualquier origen. Es lo contrario. |
| Pedir token también a `/health` | Rompe `probe_running` y `clawg stop` entre versiones (ver "Qué consumen de `/health`"). |
| Rotar el token en cada petición o expirarlo | No hay amenaza que lo justifique: el token vive con el proceso y se regenera en cada arranque (`serve.rs:117-118`, `generate_token` y `write_token` tras el bind). |
| Un secreto por cliente | El modelo es de un usuario, un token. Cambiarlo es un cambio de producto, no un endurecimiento. |
| Validar la IP del peer | El peer de un ataque de rebinding **es** `127.0.0.1`. No distingue. |

## Interacciones

- **project-root-guard (29):** define qué raíces se aceptan y deja escrito que un agente con shell lee el token. Este cambio no mueve ese límite; sólo cierra navegadores. Los dos coinciden en no venderse como barrera contra un proceso local.
- **agent-lifecycle-hooks (25) y client-setup (26):** los hooks usan `DaemonClient` y por lo tanto mandan `Host: 127.0.0.1:<puerto>` sin `Origin`. Si se eligiera la opción de hook HTTP directo, hay que comprobar sus cabeceras antes de activar el rechazo de `Origin`. `clawg setup` registra el servidor MCP por stdio (ver su exploración), no el puerto del daemon, así que no hay un `Host` configurado que revisar; si en algún momento escribiera la dirección del daemon en la configuración del host, tendría que usar `127.0.0.1`.
- **doctor-checks (31):** podría sumar un chequeo del modo del archivo del token en Unix. No es parte de este cambio y no está en su alcance actual.
- **`rag-eval --agent-view`:** usa `serve_ephemeral` y `http://{addr}`: pasa el guardia sin cambios y es parte de la verificación.

## Alcance candidato

1. Función pura `admits(host, origin)` con su tabla de casos, y su capa en `router` aplicada a todas las rutas (A).
2. Respuesta 403 con el cuerpo de error uniforme; un registro de nivel `warn` por rechazo, sin volcar valores arbitrarios largos (decisión abierta).
3. Apertura del token y de `daemon.json` con modo 0600 desde la creación en Unix, conservando `restrict_to_owner` (B1).
4. `CLAUDE.md` del repo: una línea en "Daemon HTTP API" que diga que el daemon rechaza `Host` que no sea de loopback y toda petición con `Origin`, `/health` incluida.
5. Comentario de `restrict_to_owner` y del módulo `lifecycle`: la ventana ya no existe en Unix; Windows sigue igual y por qué.

### No-alcance

- Un límite de seguridad contra un agente o proceso con shell del mismo usuario (puede leer el token). Otro usuario local sigue pudiendo conectar al puerto; sólo el token lo frena.
- Qué raíces de proyecto se permiten: `project-root-guard` (29).
- ACL explícita en Windows (B3), recortar `/health` (C), pipes o sockets (D).
- Cambios en la autenticación (rotación, tokens por cliente) o en el formato de `daemon.json`.
- Cifrado del tráfico: es loopback.
- Ocultar `/health` por perfil o por agente.

## Decisiones abiertas

Son del usuario.

- [ ] **Qué nombres admite `Host`.** `127.0.0.1` y `localhost` seguro; ¿también `[::1]`, aunque el socket es IPv4? ¿`localhost.` con punto final?
- [ ] **`Host` ausente.** ¿Se rechaza (propuesta) o se admite? HTTP/1.1 lo exige; HTTP/2 usa `:authority`. **A verificar:** si `axum::serve` con las características actuales admite HTTP/2 y, si es así, leer el autoridad de la URI cuando falte la cabecera.
- [ ] **Comprobar el puerto.** ¿Sólo el nombre (propuesta) o nombre y puerto?
- [ ] **Código de respuesta.** 403 (propuesta) o 421.
- [ ] **`Origin: null` y cualquier `Origin`.** ¿Todo `Origin` se rechaza (propuesta) o sólo los que no sean de loopback? Rechazar todo es lo más simple y no rompe a ningún cliente actual.
- [ ] **B2.** ¿Directorio de datos a 0700 en Unix, aunque excede el token?
- [ ] **Archivo preexistente en Unix.** ¿Se confía en `restrict_to_owner` posterior o se borra y recrea?
- [ ] **Windows.** ¿Se conserva la ACL del directorio personal sin más, o se pide un chequeo (en `doctor-checks`) antes de decidir B3?
- [ ] **Registro de rechazos.** ¿`warn` por rechazo (puede inundar el log si un sitio insiste) o contado una vez por minuto?

## Cómo se mide

Tests RED primero (cada uno falla hoy, porque nada rechaza):

1. **Decisión pura:** la tabla de arriba como casos; `Host` en mayúsculas; `Host` con puerto y sin él; `Host` ausente; `Origin` presente con cualquier valor.
2. **`Host: evil.example` → 403** en `/v1/stats` **y** en `/health`, con y sin token válido. Hoy da 200 y 401.
3. **`Origin: http://evil.example` → 403** con un token válido, sobre `POST /v1/search` y sobre `/health`. Hoy pasa.
4. **Petición normal del puente → OK:** `DaemonClient` contra el router real (el listener de `tests/api.rs`) sigue guardando, buscando, pidiendo `/health` y apagando.
5. **`clawg status` y `clawg stop` siguen funcionando** de punta a punta contra un daemon efímero.
6. **Puerto efímero:** con `serve_ephemeral` y con `--port 0`, el cliente con `Host: 127.0.0.1:<puerto efímero>` pasa.
7. **Unix, desde la creación:** una función que abre el archivo (la que usa `write_token`) lo crea con modo 0600 **sin** llamar a `restrict_to_owner`; el test actual (`lifecycle.rs:252-263`) mira después y no cubre la ventana. Variante: con un `umask` de 0 el archivo recién creado igual es 0600.
8. **Unix, archivo preexistente:** un token con modo 0644 queda en 0600 después de `write_token`.
9. **`daemon.json`:** mismo test (B1).
10. **El 403 usa el cuerpo de error uniforme** y no revela cuál de las dos condiciones falló (mismo criterio que el 401, `api.rs:601-604`).

`rag-eval --diff` no es requisito: no toca ranking ni recuperación. Sí hay que correr `rag-eval --agent-view` una vez, porque usa `serve_ephemeral` y es el único cliente HTTP fuera de `clawg-cli`. Línea base de tests: 1864 pasan.

No cambia el esquema, los ids, los estados ni el contrato de embedding: no hace falta test de base sembrada.

## Riesgos

- **Falsa sensación de seguridad.** Esto cierra navegadores; un proceso local con acceso al archivo del token sigue entrando. Hay que decirlo en la documentación, igual que en `project-root-guard`.
- **Romper a un cliente legítimo que mande `Origin` o un `Host` distinto.** Con los clientes de hoy no pasa; los hooks de 25 y cualquier integración futura hay que probarlos contra el guardia.
- **Rechazar `Host` ausente rompe a un cliente HTTP/1.0 o HTTP/2 mal formado.** A verificar; es una decisión abierta, no un supuesto.
- **Windows sigue igual.** La ventana de creación no existe en Unix, pero en Windows el archivo sigue dependiendo de la ACL del directorio personal, que no verifiqué.
- **Ruido de registro.** Un sitio que insiste produciría un `warn` por petición.
- **Olvidar el guardia en un router nuevo.** Por eso va dentro de `router`, no en `serve`: el arnés y los tests lo ejercitan sin enterarse.
