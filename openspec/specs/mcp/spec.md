# mcp — Especificación

Fuente de verdad del comportamiento actual del puente `clawg mcp`: cómo abre una sesión por stdio, en el ciclo legacy (`initialize`) y en el 2026-07-28 (`_meta` en cada request), y de qué nombre saca el `agent_id` de cada request. Qué filas modifica y lista cada agente está en [`specs/visibility`](../visibility/spec.md), y qué filas de otros agentes lee en [`specs/sharing`](../sharing/spec.md); este dominio sólo decide qué nombre entra como `client` en `resolve_agent` y cómo las herramientas muestran lo compartido (último apartado). Cómo el puente vuelve a encontrar al daemon después de un reinicio está en el apartado «Reconectar con el daemon tras un reinicio». Cómo `recall_context` pide un presupuesto de bytes y avisa que no buscó memorias está en «El presupuesto de `recall_context`» y «`recall_context` dice si buscó memorias»; qué entra en el presupuesto y qué se corta, en [`specs/retrieval`](../retrieval/spec.md). Qué le dice el servidor al agente sobre guardar y leer memorias (las instrucciones, el protocolo y lo que declaran los schemas de `memory_type` y la descripción de `save_memory`) está en «El protocolo de memoria» y los apartados que lo siguen. La herramienta `search_project`, que busca en la documentación y el código del proyecto, está en [`specs/project-docs`](../project-docs/spec.md); acá sólo se la cuenta entre las herramientas que lista el puente.

**Origen:**
- [mcp-protocol-upgrade](../../changes/archive/2026-10-13-mcp-protocol-upgrade/proposal.md), archivado el 2026-10-13.
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo "Lo compartido en las herramientas").
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo el conteo y la lista de herramientas).
- [bridge-reconnect](../../changes/archive/2026-10-15-bridge-reconnect/proposal.md), archivado el 2026-10-15 (sólo el apartado de reconexión con el daemon).
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (sólo la colisión en «Registro del proyecto»).
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (sólo el bloque `ingest` de `memory_status` y el texto de `save_memory` ante un `202`).
- [embedder-idle-unload](../../changes/archive/2026-10-17-embedder-idle-unload/proposal.md), archivado el 2026-10-17 (sólo «El modelo en memoria en `/health`»; el ciclo de vida del embedder está en [`specs/storage`](../storage/spec.md)).
- [context-budget](../../changes/archive/2026-10-17-context-budget/proposal.md), archivado el 2026-10-17 (sólo «El presupuesto de `recall_context`» y «`recall_context` dice si buscó memorias»).
- [memory-protocol](../../changes/archive/2026-10-17-memory-protocol/proposal.md), archivado el 2026-10-17 (desde «El protocolo de memoria» hasta el final).
- [conflict-candidates](../../changes/archive/2026-10-18-conflict-candidates/proposal.md), archivado el 2026-10-18 (sólo «Los conflictos en las herramientas», el largo de la apertura, el hábito de juzgar las memorias cercanas y qué dice de una decisión la descripción de `memory_type`).
- [topic-revisions](../../changes/archive/2026-10-18-topic-revisions/proposal.md), archivado el 2026-10-18 (sólo «La clave de tema en las herramientas» y lo que el protocolo completo dice de la clave).
- [search-abstention](../../changes/archive/2026-10-18-search-abstention/proposal.md), archivado el 2026-10-18 (sólo los conteos de `searched` y la frase de abstención en «`recall_context` dice si buscó memorias»).

## Definiciones

- **Ciclo de vida legacy:** la sesión empieza con `initialize` y el cliente se nombra en `params.clientInfo`.
- **Ciclo de vida 2026-07-28:** no hay `initialize`. Cada request lleva en `params._meta` `io.modelcontextprotocol/protocolVersion` y `io.modelcontextprotocol/clientCapabilities` (obligatorios) y `io.modelcontextprotocol/clientInfo` (SHOULD).
- **Antes de una sesión:** desde que abre stdio hasta que pasa a rmcp `initialize` o una request que no sea `ping` ni `server/discover`.
- **Nombre declarado:** `clientInfo.name` de la request (`_meta`) o, en el ciclo legacy y sin `_meta`, el de `initialize`. Un nombre en blanco no cuenta.

## R1. `server/discover`

El puente MUST contestar `server/discover` con `_meta` válido, antes de una sesión o durante ella, con:

- `supportedVersions` que incluye `2026-07-28` y las versiones legacy, entre ellas `2025-06-18`;
- `capabilities.tools`;
- las mismas `instructions` que `initialize`;
- `_meta["io.modelcontextprotocol/serverInfo"]` con el nombre MCP del perfil y la versión del binario.

Contestar MUST NOT empezar una sesión: después puede venir `initialize` o una request 2026-07-28.

#### Escenario: sonda con `_meta`
- GIVEN un puente recién abierto, perfil dev
- WHEN el host manda `server/discover` con `_meta` 2026-07-28
- THEN la respuesta trae `2026-07-28` y `2025-06-18` en `supportedVersions`, `capabilities.tools`, instrucciones que nombran `save_memory` y `serverInfo` `clawg-dev` con la versión del crate

Test: `a_discover_probe_is_answered_with_what_the_server_offers` (`handshake.rs`).

## R2. El ciclo legacy no cambia

- `initialize` con `protocolVersion` `2025-06-18` MUST negociar `2025-06-18` y devolver el `serverInfo` del perfil.
- Una sonda `server/discover` sin `_meta` MUST recibir un error con su propio id, y el puente MUST seguir vivo.
- Antes de una sesión, una notificación o una respuesta suelta MUST descartarse sin terminar el proceso. Después de empezada, MUST llegar intacta a rmcp.
- Una request que rmcp rechaza por falta de `_meta` MUST NOT contar como inicio de sesión: lo que llegue después se sigue tratando como antes de una sesión.
- `initialize` MUST contestarse aunque el daemon todavía esté arrancando (ya lo exige `initialize_is_answered_while_the_daemon_is_still_starting`).

#### Escenario: la apertura de Claude Code 2.1.285
- GIVEN un puente recién abierto
- WHEN el host manda la sonda sin `_meta`, la cancela con `notifications/cancelled`, manda `initialize`, `notifications/initialized` y `tools/list`
- THEN la sonda recibe un error con id `server-discover-probe-1`, `initialize` negocia `2025-06-18` y `tools/list` trae las once herramientas

#### Escenario: una request rechazada y su cancelación
- GIVEN un puente recién abierto
- WHEN el host manda `tools/list` sin `_meta` ni `initialize`, la cancela y después manda `initialize`
- THEN `tools/list` recibe un error y `initialize` se contesta: el puente sigue vivo

Tests: `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working`, `a_request_refused_for_missing_meta_does_not_start_the_session`, `the_probe_reaches_rmcp_and_its_cancellation_does_not`, `after_the_session_starts_the_guard_forwards_everything_untouched`, `a_refused_request_leaves_the_guard_on`, `a_closed_stream_before_a_session_ends_it` y los de `before_session` (`handshake.rs`); los de arranque de `mod.rs`.

## R3. Sesión sin `initialize`

Una request con `_meta` 2026-07-28 completo MUST atenderse sin `initialize` previo. `tools/list` en esa sesión MUST:

- listar las once herramientas ordenadas por nombre, el mismo orden en cada llamada: `delete_memory`, `list_memories`, `memory_status`, `read_memory`, `recall_context`, `resolve_conflict`, `save_memory`, `search_memory`, `search_project`, `synthesize` y `update_memory` (`update_memory` está en [`specs/update`](../update/spec.md), R5; `read_memory`, en [`specs/pieces`](../pieces/spec.md), R6; `search_project`, en [`specs/project-docs`](../project-docs/spec.md)); el conteo y la lista los fija `every_tool_is_registered_under_its_wire_name` (`server.rs`);
- traer `resultType`, `ttlMs` y `cacheScope`.

#### Escenario: listar sin handshake
- GIVEN un puente al que sólo le llegó `server/discover`
- WHEN el host manda `tools/list` con `_meta`
- THEN vuelven once herramientas en orden alfabético, con `resultType: "complete"`, un `ttlMs` numérico y un `cacheScope`

Tests: `a_session_without_initialize_lists_the_tools_in_name_order` (`handshake.rs`) y `the_viewer_is_not_exposed_to_the_model` (`server.rs`, que compara la lista ordenada).

## R4. De dónde sale el `agent_id`

Toda herramienta MUST usar, en este orden:

1. `--agent`, si se configuró (lo define visibility-policy);
2. el nombre declarado en la request;
3. si no hay, el último nombre que declaró un cliente a este proceso, en cualquier request, `server/discover` incluido;
4. si no hay ninguno, `"mcp"` (`FALLBACK_AGENT_ID`).

Los pasos 2 y 3 MUST vivir en una función pura (`client_name`), y su resultado entra como `client` en `resolve_agent`.

#### Escenario: sesión sin handshake que se nombra
- GIVEN una sesión 2026-07-28 cuyo `_meta` declara `probe-host`
- WHEN guarda una memoria
- THEN la fila queda con `agent_id = probe-host` y ninguna con `"mcp"`

#### Escenario: el nombre sólo en la sonda
- GIVEN una sesión que declaró `probe-host` en `server/discover`
- WHEN guarda una memoria con un `_meta` sin `clientInfo`
- THEN la fila queda con `agent_id = probe-host`

Tests: `the_name_the_request_declares_wins`, `a_request_that_declares_no_name_keeps_the_one_declared_earlier`, `a_blank_name_counts_as_none`, `agent_id_falls_back_when_no_client_has_identified_itself`, `a_session_without_initialize_saves_under_the_client_it_declares` y `a_request_without_client_info_keeps_the_name_declared_on_discover` (`server.rs`).

## R5. Las memorias de antes se siguen viendo

Lo que un cliente guardó por el ciclo legacy MUST encontrarse cuando el mismo cliente, con el mismo nombre, vuelve por el ciclo 2026-07-28.

#### Escenario: de `initialize` a `_meta`
- GIVEN una memoria guardada en una sesión con `initialize` de `probe-host`
- WHEN otra sesión, sin `initialize` y con `_meta` de `probe-host`, la busca con `search_memory`
- THEN la memoria aparece

Test: `memories_saved_through_initialize_stay_visible_without_it` (`server.rs`).

## Reconectar con el daemon tras un reinicio

El puente no se queda con el daemon que encontró al abrir. Cuando una llamada falla porque el daemon cambió (token nuevo, puerto nuevo) o porque su arranque no terminó a tiempo, vuelve a leer `daemon.json` y `token` del directorio de datos y repite la llamada una vez. Nunca lanza un daemon: un `clawg stop` hecho a propósito se respeta. No cambia ninguna herramienta, ni sus argumentos, ni lo que contesta cuando el daemon responde.

**Releer** es sondear el daemon como lo hace `probe_running(base_dir, false)` (`crates/clawg-cli/src/client.rs:1122`): leer `daemon.json` y `token` y pedir `/health`. Lo hace `ClientHandle::reresolve` (`crates/clawg-cli/src/mcp/startup.rs:102`). Sin daemon que conteste devuelve `NotRunning` (`client.rs:1147`); si el almacén lo tiene otro proceso que no es un daemon, devuelve `StoreHeld`.

**Falla que se resuelve releyendo:** `ClientError::Unauthorized` (el daemon contestó 401), `ClientError::NotRunning` (la conexión no se abrió) o `ClientError::StartTimeout` (el arranque no terminó a tiempo). La decisión es la función pura `on_failure` (`startup.rs:39`): da `Next::Reresolve` para esas tres y `Next::Report` para cualquier otra, entre ellas `StoreHeld`, `Spawn`, `Daemon` y `Http`.

#### Escenario: qué falla se relee
- GIVEN cada variante de `ClientError`
- WHEN se decide qué hacer con ella
- THEN `Unauthorized`, `NotRunning` y `StartTimeout` se resuelven releyendo, y todas las demás se informan

**Test:** `only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved` (`mcp/startup.rs`).

**Releer y repetir una vez.** Cuando una herramienta falla con una falla que se resuelve releyendo, `ClawgMcp::call` (`crates/clawg-cli/src/mcp/server.rs:475`) MUST releer, sea al pedir el cliente (`server.rs:481-488`) o al llamar al daemon (`server.rs:491-497`). Una llamada relee como mucho una vez: si ya se releyó al pedir el cliente, un fallo de la repetición se informa tal cual (`server.rs:492`, `!reresolved`).

- Si la relectura encuentra un daemon que contesta, el puente MUST guardarlo para las llamadas siguientes (`ClientHandle::current`, `startup.rs:56`, asignado en `startup.rs:107`), MUST repetir la herramienta una vez con él y MUST devolver lo que conteste esa repetición.
- Si la relectura no encuentra un daemon, la herramienta MUST informar sin repetir nada: el texto de `NotRunning` si lo que falló fue la llamada (`server.rs:494`), o el de `StartTimeout` si lo que falló fue el arranque (`server.rs:487`). Un `StoreHeld` de la relectura se informa con su propio mensaje, en los dos caminos (`server.rs:485` y `server.rs:494`).
- Si la repetición falla otra vez, se informa esa segunda falla, sin un tercer intento.

Al encontrar un daemon releyendo, el puente deja en el log su dirección, la versión del daemon y la propia (`startup.rs:109-115`; si `/health` falla en ese momento, sólo la dirección) y MUST NOT cambiar por eso lo que contesta la herramienta. `version_warning` (`crates/clawg-cli/src/memories.rs:332`) no se usa en el puente: aconseja `clawg stop`, y después de una actualización lo viejo es el puente, no el daemon.

Repetir MUST NOT llegar dos veces al handler del daemon: un 401 lo corta `require_bearer` antes del handler (`crates/clawg-cli/src/daemon/api.rs:760-778`), y una conexión rechazada no llegó. Por eso la única llamada del cliente con timeout es `/health` (`crates/clawg-cli/src/client.rs:601`, `HEALTH_TIMEOUT` en `client.rs:45`), y en una herramienta que escribe `NotRunning` sólo significa una conexión que no se abrió. `map_transport` (`client.rs:589`) mapea a `NotRunning` lo que es `is_connect` o `is_timeout`.

#### Escenario: el daemon se reinició
- GIVEN un puente contra el daemon A, con su puerto y su token
- AND A se detiene y el daemon B arranca sobre el mismo directorio de datos, con otro token y un puerto efímero (`Published::serve`, `server.rs:3546`)
- WHEN el agente llama a `save_memory` y después a `search_memory`
- THEN el save se guarda en B y la búsqueda lo encuentra
- AND ninguna de las dos respuestas es un error

**Tests:** `a_restarted_daemon_is_reached_with_its_new_token` (`mcp/server.rs`); con el binario, un `clawg stop` y otro `clawg serve`, `a_bridge_left_open_reaches_the_daemon_after_a_restart` (`crates/clawg-cli/tests/binary.rs`, ver [`specs/install`](../install/spec.md)).

#### Escenario: el token se relee una sola vez
- GIVEN un daemon que contesta 401 a toda ruta con token
- WHEN el agente llama a `save_memory`
- THEN el daemon recibe exactamente dos pedidos a `/v1/memories`
- AND la respuesta es un error con el texto del token rechazado (abajo), que no manda a borrar el archivo del token

**Tests:** `a_refused_token_is_reread_once` y `the_refused_token_text_does_not_say_to_delete_it` (`mcp/server.rs`).

#### Escenario: un error que el daemon contestó no se repite
- GIVEN un daemon que contesta 500 a `/v1/memories`
- WHEN el agente llama a `save_memory`
- THEN el daemon recibe exactamente un pedido a `/v1/memories`

**Tests:** `an_error_the_daemon_answered_reaches_it_once` (`mcp/server.rs`); `only_health_sets_a_timeout` (`client.rs`).

**Arranque que no terminó.** El arranque en segundo plano del puente (`crates/clawg-cli/src/mcp/mod.rs:99-114`) deja su resultado en un `ClientHandle`. `ClientHandle::get` (`startup.rs:75`) espera el resultado el tiempo de la herramienta: si el arranque falló, devuelve su error (`startup.rs:88`), que es `StartTimeout` cuando `start_daemon` se rindió esperando al daemon (`crates/clawg-cli/src/client.rs:1154`); si el arranque sigue sin terminar al vencer la espera, `StartTimeout` (`startup.rs:89` y `startup.rs:92`); si el emisor se soltó sin resolver, `NotRunning` (`startup.rs:91`). La herramienta siguiente relee, como arriba. Si el daemon contesta, se usa y se guarda; si no, se informa el texto de `StartTimeout` (`crates/clawg-cli/src/mcp/server.rs:528-532`), no el de un daemon detenido, porque lo que falló fue el arranque.

#### Escenario: el daemon termina de arrancar después
- GIVEN un puente cuyo arranque venció
- AND después un daemon termina de arrancar y publica `daemon.json` y `token`
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de ese daemon, no un error

**Test:** `a_daemon_that_comes_up_after_the_start_gave_up_is_used` (`mcp/server.rs`).

**Textos de falla.** En inglés, como el resto de las herramientas. Se usan sólo después de releer (`ClawgMcp::explain`, `server.rs:507`):
- `NotRunning` (`server.rs:512-516`): "the clawg daemon is not running (it may have been stopped or updated); start it with `clawg serve`, or reconnect clawg in your MCP client, then call again".
- `Unauthorized` (`server.rs:519-524`): "the clawg daemon rejected the token in its data directory even after re-reading it, so the token file no longer matches the running daemon; restart the daemon (`clawg status` shows its pid) and call again".
- `StartTimeout` (`server.rs:528-532`): "clawg tried to start its daemon and it is not up; run `clawg status` to see why, or try again if it is still starting".

El texto de `ClientError::Unauthorized` de la CLI (`crates/clawg-cli/src/client.rs:53`) no cambia: los comandos releen los archivos en cada corrida, y para ellos el consejo de borrar el token sí vale.

#### Escenario: un daemon detenido no se reemplaza por uno lanzado
- GIVEN un puente cuyo daemon se detuvo y no volvió
- WHEN el agente llama a cualquier herramienta
- THEN la respuesta es un error con el texto de `NotRunning`
- AND ningún daemon se lanza

**Tests:** `a_daemon_stopped_mid_session_is_reported_without_starting_one` (`mcp/server.rs`).

**Registro del proyecto.** `ClawgMcp::project_for` (`crates/clawg-cli/src/mcp/server.rs:658`) registra cada raíz y su id heredado una sola vez por proceso. Si el registro falla con una falla que se resuelve releyendo, la entrada se quita (`server.rs:685-687`) y el siguiente envío, incluida la repetición de la llamada, lo vuelve a mandar. Una colisión (409 `project_root_collision`) también quita la entrada, pero hace fallar la herramienta ([`specs/project`](../project/spec.md), R6). Cualquier otra falla cuenta como enviada, para no costarle un pedido a cada herramienta frente a un daemon que no sabe registrar.

#### Escenario: el registro que el daemon anterior rechazó
- GIVEN un puente cuyo daemon se reinició, con el token viejo guardado
- WHEN el agente llama a `save_memory` en su proyecto
- THEN el daemon nuevo recibe el registro del proyecto antes del save

**Test:** `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (`mcp/server.rs`).

**`memory_status`.** `memory_status_impl` (`crates/clawg-cli/src/mcp/server.rs:981`) pasa por `call` como las demás herramientas. Su `/health` no lleva token (`DaemonClient::health`, `client.rs:601`; ruta abierta en `crates/clawg-cli/src/daemon/api.rs:730`), así que después de un reinicio muestra el daemon nuevo.

#### Escenario: `memory_status` después de un reinicio
- GIVEN un puente contra el daemon A que se detuvo, y el daemon B arrancado
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de B, no un error

**Test:** `memory_status_after_a_restart_reports_the_new_daemon` (`mcp/server.rs`).

**Saves que esperan o fallaron.** `memory_status` MUST mostrar el bloque `ingest` del `/health` del daemon (`pending`, `failed`, `oldest_pending_s` y los `problems`, cada uno con su `operation_id` y su causa) porque imprime el `Health` entero como JSON (`format_status`, `crates/clawg-cli/src/mcp/tools.rs:456`; `Health.ingest`, `client.rs:140`). Contra un daemon que no lo trae, o que no contestó a tiempo, el bloque no aparece ([`specs/ingest`](../ingest/spec.md), «Estado del outbox en `/health`»). Es el lugar donde el agente se entera de un save aceptado con un `202` que después falló: los textos de `save_memory` ante un `202` nombran el `operation_id` y dicen que un fallo aparece en `memory_status` ([`specs/ingest`](../ingest/spec.md), «Texto que ve el modelo»). `save_memory` no tiene un argumento `operation_id`, y por eso `agent_contract.json` no cambió con el outbox.

#### Escenario: un save que falló después de contestar
- GIVEN una fila `failed` en el outbox
- WHEN el agente llama a `memory_status`
- THEN el texto incluye `ingest.failed` y el `operation_id` y la causa del save

**Tests:** `format_status_shows_the_ingest_block` (`tools.rs`); `save_outcome_text_covers_every_daemon_answer` (`server.rs`).

## El modelo en memoria en `/health`

`GET /health` MUST incluir `embedder_resident` (`Health`, `crates/clawg-cli/src/daemon/api.rs:466`, que lo lee de `Embedder::resident` en `api.rs:807`): `true` con el modelo en memoria, `false` cuando el daemon lo soltó por ocio y la próxima llamada que necesite un vector espera que vuelva a cargar, y `null` antes de la primera carga. `embedder` sigue en `ready` con el modelo suelto ([`specs/storage`](../storage/spec.md), «Ciclo de vida del embedder»). El cliente MUST leer un `/health` sin el campo, el de un daemon anterior, como `None` (`client.rs:134`, `#[serde(default)]`). `memory_status` lo muestra porque imprime el `Health` entero.

#### Escenario: el modelo suelto
- GIVEN un daemon con el modelo cargado una vez y soltado por ocio
- WHEN se pide `/health`
- THEN `embedder` es `ready` y `embedder_resident` es `false`

**Tests:** `health_says_whether_the_model_is_in_memory` (`crates/clawg-cli/tests/api.rs`); `a_health_without_residency_still_parses` (`client.rs`).

## El presupuesto de `recall_context`

`POST /v1/context` MUST aceptar `budget` opcional, en bytes del prompt (`ContextRequest`, `crates/clawg-cli/src/daemon/api.rs:371-380`; `check_budget`, `api.rs:310`, que el handler llama antes de armar nada, `api.rs:1096`):

- ausente, MUST usar 8.000 (`DEFAULT_BUDGET`);
- mayor que 24.000, MUST usar 24.000 (`MAX_BUDGET`);
- `0` MUST responder `400`;
- un valor que no es un entero no negativo MUST rechazarse como cualquier cuerpo mal formado (un `4xx`).

`recall_context` MUST aceptar `budget` (`RecallContextArgs`, `crates/clawg-cli/src/mcp/server.rs:209-221`) y pasarlo (`recall_context_impl`, `server.rs:774`), y su descripción MUST decir que el contenido llega entero hasta un presupuesto de bytes y que lo que no entra se nombra por id para `read_memory` (`server.rs:1057-1063`). `DaemonClient::context` MUST NOT mandar `budget` cuando no se le dio (`crates/clawg-cli/src/client.rs:668-682`): un puente sin presupuesto deja el tamaño al daemon, y un daemon anterior al campo recibe el cuerpo de siempre.

#### Escenario: sin presupuesto y con uno chico
- GIVEN una memoria que entra en 8.000 bytes
- WHEN el agente llama `recall_context` sin `budget`, y después con `budget` 300
- THEN la primera respuesta no lleva bloque `BUDGET`, y la segunda ocupa como mucho 300 bytes y dice `Cut to fit 300 bytes`

#### Escenario: presupuesto fuera de rango
- GIVEN un contexto de más de 24.000 bytes
- WHEN se pide `POST /v1/context` sin `budget`, con 2000 y con 1000000
- THEN el prompt se corta a 8.000, 2.000 y 24.000 bytes; con `budget` 0, la respuesta es `400`

**Tests:** `context_takes_a_budget_and_clamps_it` y `context_refuses_a_zero_budget` (`crates/clawg-cli/tests/api.rs`); `recall_context_passes_its_budget` (`server.rs`).

## `recall_context` dice si buscó memorias

`POST /v1/context` MUST responder `retrieval` con `state` (`ContextBody` y `RetrievalState`, `api.rs:584-620`), leído del mismo estado del embedder que usa `POST /v1/search` (`RetrievalState::of`, `api.rs:1133`):

- `searched`, con `hits` y `withheld`: se buscó; `hits` son los que usó el prompt antes del presupuesto y `withheld` las filas de score 0 que dejó afuera ([`specs/retrieval`](../retrieval/spec.md), «`recall_context` deja afuera los ceros»);
- `embedder_not_ready`: no hay embedder y el estado no es `failed` (el modelo carga), o `embed` contestó que el modelo no está listo;
- `embedder_failed`, con `detail` y `retryable`: no hay embedder y el estado es `failed`;
- `search_failed`, con `detail`: `embed` falló por otra causa, o falló la búsqueda.

En los cuatro casos MUST seguir respondiendo `200` con el workspace. `Context` del cliente MUST leer `retrieval` con `#[serde(default)]`, un estado que no conoce como `Unknown`, y `searched` sin conteos (un daemon anterior) con `hits` y `withheld` en 0 (`client.rs:264-296`).

`recall_context` (`format_recall`, `crates/clawg-cli/src/mcp/tools.rs:383`; la frase en `retrieval_notice`, `tools.rs:360`) MUST empezar con `Memories were not searched: <por qué>.` cuando `state` no es `searched`, haya workspace o no, seguido de que sólo se muestra el workspace (o de que está vacío) y de que eso no quiere decir que no haya memorias. Con `searched`, `hits = 0` y `withheld > 0`, MUST empezar con la frase de abstención de [`specs/retrieval`](../retrieval/spec.md) (`no relevant memories for this query: N searched, none close enough to show`, con `N` = `withheld`) seguida de `; only the workspace is shown below.` si hay prompt o de `; the workspace is empty.` si no lo hay (`tools.rs:362-365`). Con el prompt vacío, cualquiera de las dos frases MUST reemplazar `no context yet`. Sin el campo (un daemon anterior), con `searched` sin conteos o con un estado desconocido (uno más nuevo), MUST NOT agregar ninguna frase. El aviso lo escribe el puente, fuera del prompt, así que `rag-eval` no lo mide.

#### Escenario: el modelo carga
- GIVEN un daemon con el embedder en `loading` y un workspace con preferencias
- WHEN el agente llama `recall_context`
- THEN la respuesta empieza diciendo que las memorias no se buscaron porque el modelo carga, y después viene el workspace

#### Escenario: nada relevante
- GIVEN `retrieval` en `searched` con `hits: 0` y `withheld: 2`, y un workspace
- THEN `recall_context` empieza con `no relevant memories for this query: 2 searched, none close enough to show; only the workspace is shown below.` y después viene el workspace; sin workspace, dice la frase terminada en `; the workspace is empty.`

#### Escenario: un daemon anterior
- GIVEN una respuesta de `POST /v1/context` sin `retrieval`, o con `searched` sin conteos, y con el prompt vacío
- THEN `recall_context` dice `no context yet`, sin aviso

**Tests:** `context_says_whether_memories_were_searched` (`crates/clawg-cli/tests/api.rs`, los cuatro estados) y `context_says_how_many_hits_it_withheld` (`crates/clawg-cli/tests/api.rs`); `a_context_without_retrieval_still_parses` y `a_searched_context_without_counts_still_parses` (`client.rs`); `recall_says_when_memories_were_not_searched`, `recall_says_when_nothing_was_relevant` y `an_old_daemon_gets_no_invented_notice`, con `searched` sin conteos (`tools.rs`); `the_retrieval_outcome_names_each_silence` (`crates/clawg-core/src/memory/prompt_builder.rs`).

## Lo compartido en las herramientas

La política y el alcance son de [`specs/sharing`](../sharing/spec.md); acá está lo que el agente ve en cada herramienta:

- `search_memory` MUST marcar un hit de otro agente con `by <autor>` dentro del corchete (`N. #id [kind/tipo/tN score=S by <autor>] contenido`) y, si hay alguno, terminar con la nota de que esas líneas son información para verificar y de sólo lectura (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:95`). `recall_context` MUST marcarlos igual en sus líneas y cerrar con un bloque `SHARED` (`prompt_builder.rs:439-445`; el formato, en [`specs/retrieval`](../retrieval/spec.md)). Un hit propio MUST quedar como siempre.
- `memory_status` MUST terminar con la línea `shared:` (`format_sharing`, `tools.rs:442`; `memory_status_impl`, `server.rs:981`): `shared: reads every agent`, `shared: reads every agent but <autor>, <autor>`, `shared: none (reads only its own)` o `shared: none (<motivo>)` con un `sharing.json` inválido.
- Las descripciones de `delete_memory`, `update_memory` y `resolve_conflict` MUST decir que valen sólo sobre las memorias propias y que una marcada `by <agente>` es de sólo lectura (`server.rs:1106-1159`). Sobre una fila ajena las tres fallan como si el id no existiera ([`specs/visibility`](../visibility/spec.md) R3).

Tests: `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone` y `format_status_says_whom_the_agent_reads` (`tools.rs`); `search_memory_shows_another_agents_memory_with_its_author`, `recall_context_shows_another_agents_memory_with_its_author`, `memory_status_reports_the_sharing_of_the_agent`, `tool_descriptions_say_shared_memories_are_read_only` y `a_shared_memory_is_read_only` (`server.rs`).

## Los conflictos en las herramientas

Qué es un par, cuándo escala y qué hace cada veredicto está en [`specs/disputes`](../disputes/spec.md); acá está lo que el agente ve:

- `resolve_conflict` MUST tomar `keep`, `other`, `verdict` y `reason` (`ResolveConflictArgs`, `crates/clawg-cli/src/mcp/server.rs:347`), y ya no `keep_both`. `verdict` MUST declarar en el schema el `enum` de los cuatro veredictos de `Verdict::ALL` (`verdict_enum`, `server.rs:341`). Al deserializar sigue siendo una cadena libre, así que un valor desconocido llega al daemon y vuelve como un error que lista los cuatro. Contra un daemon anterior a `/v1/disputes/judge`, MUST pedir que se lo reinicie con `clawg stop` (`server.rs:917-922`).
- `save_memory` MUST sumar la línea `close to earlier memories: …` cuando el save tiene candidatas (`related_text`, `server.rs:325`).
- `memory_status` MUST sumar `conflict candidates: none open`, o los pares pendientes y escalados del agente con los pendientes más viejos y cómo juzgarlos (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:151`). Contra un daemon sin `GET /v1/disputes` MUST omitir la línea, y no fallar (`server.rs:992-994`).

**Tests:** `resolve_conflict_reports_the_outcome`, `resolve_conflict_against_an_older_daemon_asks_for_a_restart`, `save_outcome_text_names_related_memories` y `memory_status_counts_pending_candidates` (`server.rs`). El schema lo fija también `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)).

## La clave de tema en las herramientas

Qué hace un save con clave y cómo se valida está en [`specs/ingest`](../ingest/spec.md); acá está lo que el agente pasa y lo que ve.

- `save_memory` MUST tomar `topic_key`, opcional (`SaveMemoryArgs.topic_key`, `crates/clawg-cli/src/mcp/server.rs:182`), y su schema MUST describir el formato: segmentos de `a-z`, `0-9`, `.`, `_` o `-` unidos por `/`, de a lo sumo 80 caracteres, y que guardar otra vez bajo la misma clave reemplaza a la memoria guardada con ella, que queda en la historia. El texto de la respuesta nombra cada fila reemplazada con su primera línea, y avisa cuando el daemon no devolvió la clave que se mandó ([`specs/ingest`](../ingest/spec.md), «Texto que ve el modelo»). La descripción de la herramienta no cambia.
- `list_memories` MUST mostrar ` topic=K` en una fila con clave y ` by=#N` en una `superseded` (`revision`, `crates/clawg-cli/src/mcp/tools.rs:246`), para que un listado muestre las versiones de un tema y cuál reemplazó a cuál.
- Un hit de `search_memory` MUST mostrar ` topic=K` después del `score` cuando la fila es del lector y tiene clave (`format_hits`, `tools.rs:95`): `N. #id [kind/tipo/tN score=S topic=K] contenido`. Uno de otro agente MUST NOT mostrarla, porque un save con esa clave sólo revisa filas propias y la clave prometería una revisión que el lector no puede hacer. Un hit sin clave MUST quedar como antes.
- `read_memory` MUST mostrarla en la cabecera (`format_save`, `tools.rs:179`; [`specs/pieces`](../pieces/spec.md), R6). Lee siempre filas del lector, así que no necesita la regla de los hits ajenos.
- `POST /v1/search` MUST devolver `topic_key` en cada hit de una fila que la tiene (`RankedHit.topic_key`, `crates/clawg-core/src/memory/search.rs:98`; el cliente la lee con `#[serde(default)]` en `Hit`, `crates/clawg-cli/src/client.rs:188`), y `GET /v1/memories/{kind}/{id}/save` la de la fila pedida. Un hit de una fila sin clave, y el de `episodic_trajectory` o `procedural_skill`, que no tienen la columna, MUST NOT llevar el campo.
- `recall_context` no muestra la clave (`format_hit_line`, `crates/clawg-core/src/memory/prompt_builder.rs:530`): es un prompt con presupuesto y cada byte cuenta.

#### Escenario: otra sesión revisa la decisión
- GIVEN una `architectural_decision` guardada con la clave `cli/tui/library`
- WHEN un agente en una sesión nueva la encuentra con `search_memory` y guarda la versión nueva
- THEN el hit y `read_memory` le muestran `topic=cli/tui/library`
- AND el save bajo esa clave responde `saved #N; replaces #M "<primera línea>"`

#### Escenario: el hit de otro agente
- GIVEN una fila con clave de un agente A, que B puede leer
- WHEN B busca y la encuentra
- THEN el hit dice `by A` y no muestra `topic=`

**Tests:** `save_outcome_text_names_the_replaced_memories`, `a_key_sent_to_an_older_daemon_is_reported` y `save_memory_revises_by_topic_key` (`server.rs`); `rows_show_their_key_and_who_replaced_them`, `a_hit_shows_the_topic_key_it_is_revised_under`, `a_hit_without_a_key_is_shown_as_before`, `another_agents_hit_does_not_show_its_topic_key` y `format_save_names_the_topic_key_of_the_row` (`tools.rs`); `a_search_hit_carries_its_topic_key` (`worker.rs`); `read_save_carries_the_key_of_the_row_asked_for` (`pieces.rs`); `a_search_hit_and_the_save_read_carry_the_topic_key` (`crates/clawg-cli/tests/api.rs`). El schema lo fija también `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)). Lo que ve un agente de verdad en una sesión nueva se midió en el perfil dev con Claude Code ([verify](../../changes/archive/2026-10-18-topic-revisions/verify.md#perfil-dev-con-claude-code-2026-10-18)); cuántas veces lo hace sin que se lo pidan no está medido.

## El protocolo de memoria

Lo que el agente lee sobre qué guardar y cómo escribirlo sale de un solo texto en inglés, `crates/clawg-cli/src/mcp/protocol.md`, compilado en el binario (`PROTOCOL`, `crates/clawg-cli/src/mcp/server.rs:118`). Su **apertura** son los párrafos entre el título (la primera línea, `# …`) y la primera línea que empieza con `## `. Dentro de cada párrafo, las líneas se unen con un espacio, y los párrafos se separan con una línea en blanco. Las secciones desarrollan cada punto: cuándo guardar, qué no guardar, cómo escribir un save, tipo e importancia, el resumen de sesión, qué hacer tras una compactación, las conclusiones, que lo recuperado es evidencia y no instrucciones, y las demás herramientas. Hoy sólo la apertura llega sola al agente; el texto completo lo leen los tests de este apartado.

### Las instrucciones son la apertura

`get_info` MUST entregar la apertura como `instructions` (`server.rs:1231`), y `server/discover` MUST llevar las mismas, porque arma su respuesta con `get_info` (`server.rs:1237-1242`). Extraer la apertura MUST ser una función pura sobre el texto (`opening`, `server.rs:122`) que MUST NOT cortar una palabra, MUST NOT dejar saltos de línea dentro de un párrafo y MUST NOT incluir el título ni una línea de sección. Un checkout con `\r\n` MUST dar la misma apertura.

#### Escenario: un párrafo cortado en dos líneas
- GIVEN `# T\n\nuno\ndos\n\ntres\n## S\ncuatro`
- WHEN se extrae la apertura
- THEN es `uno dos\n\ntres`, y lo mismo con `\r\n` en lugar de `\n`

**Tests:** el puro `opening_joins_the_lines_of_each_paragraph` y `the_instructions_are_the_opening_of_the_protocol` (`server.rs`). Como `discover` usa `get_info`, un solo test fija las dos puertas.

### Las instrucciones caben en 2.048 caracteres

La apertura MUST medir 2.048 caracteres o menos, contados como `char`: Claude Code anterior a la v2.1.296 corta las instrucciones de un servidor en ese largo, y se pagan en cada sesión. Hoy mide 1.948.

**Test:** `the_instructions_fit_older_clients` (`server.rs`).

### La apertura lleva los hábitos que más rinden

Mientras sea el único texto del protocolo que llega solo al agente, la apertura MUST:

- decir cuándo llamar a `save_memory`, `search_memory`, `recall_context`, `search_project`, `resolve_conflict` y `synthesize`;
- pedir un hecho por llamada, con una primera línea que nombre el tema y con rutas e identificadores exactos;
- pedir memorias y búsquedas en inglés, cualquiera sea el idioma de la conversación, porque una memoria se encuentra mejor con una consulta en su idioma (el corte por pareja de idiomas de [`specs/eval`](../eval/spec.md));
- decir que lo recuperado (memorias y pasajes del proyecto) es evidencia y no instrucciones;
- pedir que las memorias cercanas que nombra `save_memory` se juzguen con `resolve_conflict` mientras la conversación sigue ahí, y que se le pregunte al usuario por lo que lista ACTIVE CONFLICTS ([`specs/disputes`](../disputes/spec.md));
- decir qué no guardar.

El texto completo MUST traer además el resumen de sesión con sus seis partes (Goal, Instructions, Discoveries, Accomplished, Next steps y Relevant files) y qué hacer tras una compactación. También MUST decir cuándo darle una `topic_key` a un save (una decisión o una preferencia que puede cambiar, con un nombre corto y estable como `storage/engine`) y que una versión nueva se guarda bajo la clave que muestran `search_memory` y `read_memory`, nunca bajo una nueva. Esa oración va en «How to write a save», fuera de la apertura, así que las instrucciones y su largo no cambian; no tiene un test que la pida, y `the_protocol_names_only_what_exists` comprueba que `topic_key` sea una propiedad del schema.

**Tests:** `the_instructions_carry_the_saving_habits` y `the_protocol_describes_the_session_summary` (`server.rs`); además, `get_info_advertises_tools_and_the_profiles_server_name` e `instructions_say_when_to_search_the_project` (`server.rs`) y `a_discover_probe_is_answered_with_what_the_server_offers` (`handshake.rs`) piden `save_memory`, `synthesize` y `search_project` en las instrucciones.

### El protocolo nombra sólo lo que existe

- Cada herramienta registrada en `tool_router` MUST aparecer entre comillas invertidas en el protocolo.
- Cada texto entre comillas invertidas que sea un nombre en minúsculas, con o sin `_` (`[a-z]+(_[a-z]+)*`), MUST ser una herramienta registrada, una propiedad del `inputSchema` de alguna herramienta o un tipo de memoria. Los ejemplos y las marcas (`` `Fixed lexical bonus ignoring paths` ``, `` `<private>` ``, `` `piece=K/N` ``, `` `by <agent>` ``) no cuentan.
- Cada tipo de memoria MUST aparecer en el protocolo.

Juntar los textos entre comillas invertidas MUST ser una función pura (`code_spans` e `is_name`, en los tests de `server.rs`).

#### Escenario: una herramienta renombrada
- GIVEN el protocolo con `` `save_memories` ``
- THEN el test falla y nombra `save_memories`

**Tests:** el puro `backticked_names_reads_each_code_span` y `the_protocol_names_only_what_exists` (`server.rs`).

## El schema de `memory_type` declara los tipos

En el `inputSchema` de `save_memory` y de `update_memory`, la propiedad `memory_type` MUST declarar `enum` con los siete tipos de `MemoryType::ALL`, en su orden, más `null`, para que el `default` declarado (`null`) siga siendo válido (`memory_type_enum`, `server.rs:147-151`, aplicado con `#[schemars(transform = …)]` en `server.rs:163` y `server.rs:318`). Sus descripciones MUST NOT repetir la lista: dicen que es uno de los valores listados y que cualquier otro se rechaza. La de `save_memory` MUST decir además que una `architectural_decision` es una elección de diseño y su motivo, y que lo hecho, hallado o medido es un `semantic_fact`, aunque siga a una decisión: el tipo decide qué par se le pregunta al usuario ([`specs/disputes`](../disputes/spec.md)).

Al deserializar, el argumento MUST seguir siendo una cadena libre (`Option<String>`): un valor desconocido llega al daemon y vuelve como `not saved: unknown memory_type …` con los siete tipos ([`specs/ingest`](../ingest/spec.md), «El tipo de memoria se valida al guardar»), no como un error de protocolo que el modelo no puede leer. El `enum` le muestra los tipos al modelo antes de llamar; la validación de verdad es la del daemon.

#### Escenario: un tipo inventado
- GIVEN un cliente que no valida el `enum`
- WHEN el agente llama `save_memory` con `memory_type: "bug_fix"`
- THEN la respuesta es un error que empieza con `not saved: unknown memory_type "bug_fix"` y nombra los siete tipos, y no se guarda nada

**Tests:** `the_save_memory_schema_names_the_accepted_memory_types` (`server.rs`), que compara el `enum` de las dos herramientas con `MemoryType::ALL`; `the_save_memory_schema_says_what_a_decision_is` (`server.rs`); `save_memory_says_an_unknown_type_was_not_saved` (`server.rs`). El schema lo fija también `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)).

## La descripción de `save_memory` pide título y "dónde"

La descripción de `save_memory` (`server.rs:1017-1025`) MUST pedir una primera línea que nombre el tema y, después, qué, por qué y dónde, con las rutas e identificadores exactos: las primeras palabras se embeben con cada pieza de un save largo, y un identificador que nombra la consulta es lo que marca un hit como `exact`. MUST conservar "one fact" y `read_memory`.

**Tests:** `save_memory_asks_for_a_title_and_where` y `save_memory_asks_for_one_fact_per_call` (`server.rs`).
