# mcp — Especificación

Fuente de verdad del comportamiento actual del puente `clawg mcp`: cómo abre una sesión por stdio, en el ciclo legacy (`initialize`) y en el 2026-07-28 (`_meta` en cada request), y de qué nombre saca el `agent_id` de cada request. Qué filas lee, modifica y lista cada agente está en [`specs/visibility`](../visibility/spec.md), con el alcance que le da [`specs/sharing`](../sharing/spec.md); este dominio sólo decide qué nombre entra como `client` en `resolve_agent` y cómo las herramientas muestran lo compartido (último apartado). Cómo el puente vuelve a encontrar al daemon después de un reinicio está en el apartado «Reconectar con el daemon tras un reinicio». Cómo `recall_context` pide un presupuesto de bytes y avisa que no buscó memorias está en «El presupuesto de `recall_context`» y «`recall_context` dice si buscó memorias»; qué entra en el presupuesto y qué se corta, en [`specs/retrieval`](../retrieval/spec.md). Qué le dice el servidor al agente sobre guardar y leer memorias (las instrucciones, el protocolo y lo que declaran los schemas de `memory_type` y la descripción de `save_memory`) está en «El protocolo de memoria» y los apartados que lo siguen. De qué proyecto lee y en cuál guarda una herramienta que no nombra uno, con las carpetas que el cliente tiene abiertas, está desde «BI1» hasta «BI10». En qué idioma lee el agente todo eso, y en cuál le contesta el puente, está desde «LP8» hasta el final. La herramienta `search_project`, que busca en la documentación y el código del proyecto, está en [`specs/project-docs`](../project-docs/spec.md); acá sólo se la cuenta entre las herramientas que lista el puente.

**Origen:**
- [mcp-protocol-upgrade](../../changes/archive/2026-10-13-mcp-protocol-upgrade/proposal.md), archivado el 2026-10-13.
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo "Lo compartido en las herramientas").
- [agent-authorship](../../changes/archive/2026-10-22-agent-authorship/proposal.md), archivado el 2026-10-22 ("Lo compartido en las herramientas", la clave en un hit de otro agente y los pares de `memory_status` por proyecto).
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo el conteo y la lista de herramientas).
- [project-reassign](../../changes/archive/2026-10-22-project-reassign/proposal.md), archivado el 2026-10-22 (sólo «Mover una memoria de proyecto»).
- [bridge-reconnect](../../changes/archive/2026-10-15-bridge-reconnect/proposal.md), archivado el 2026-10-15 (sólo el apartado de reconexión con el daemon).
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (sólo la colisión en «Registro del proyecto»).
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (sólo el bloque `ingest` de `memory_status` y el texto de `save_memory` ante un `202`).
- [outbox-lifecycle](../../changes/archive/2026-10-19-outbox-lifecycle/proposal.md), archivado el 2026-10-19 (sólo la línea de `memory_status` sobre los saves `failed`).
- [embedder-idle-unload](../../changes/archive/2026-10-17-embedder-idle-unload/proposal.md), archivado el 2026-10-17 (sólo «El modelo en memoria en `/health`»; el ciclo de vida del embedder está en [`specs/storage`](../storage/spec.md)).
- [context-budget](../../changes/archive/2026-10-17-context-budget/proposal.md), archivado el 2026-10-17 (sólo «El presupuesto de `recall_context`» y «`recall_context` dice si buscó memorias»).
- [memory-protocol](../../changes/archive/2026-10-17-memory-protocol/proposal.md), archivado el 2026-10-17 (desde «El protocolo de memoria» hasta el final).
- [conflict-candidates](../../changes/archive/2026-10-18-conflict-candidates/proposal.md), archivado el 2026-10-18 (sólo «Los conflictos en las herramientas», el largo de la apertura, el hábito de juzgar las memorias cercanas y qué dice de una decisión la descripción de `memory_type`).
- [topic-revisions](../../changes/archive/2026-10-18-topic-revisions/proposal.md), archivado el 2026-10-18 (sólo «La clave de tema en las herramientas» y lo que el protocolo completo dice de la clave).
- [search-abstention](../../changes/archive/2026-10-18-search-abstention/proposal.md), archivado el 2026-10-18 (sólo los conteos de `searched` y la frase de abstención en «`recall_context` dice si buscó memorias»).
- [agent-lifecycle-hooks](../../changes/archive/2026-10-18-agent-lifecycle-hooks/proposal.md), archivado el 2026-10-18 (sólo el estado `recent` en «`recall_context` dice si buscó memorias» y la `query` opcional de `POST /v1/context`).
- [session-handoff](../../changes/archive/2026-10-19-session-handoff/proposal.md), archivado el 2026-10-19 (sólo lo que el protocolo completo dice de LAST SESSION).
- [protected-verdicts](../../changes/archive/2026-10-20-protected-verdicts/proposal.md), archivado el 2026-10-20 (sólo `user_answered` en «Los conflictos en las herramientas», la respuesta del usuario en la apertura y su largo).
- [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23 (desde «LP8. La línea del otro idioma» hasta «LP11»). La preferencia está en [`specs/preferences`](../preferences/spec.md).
- [project-locators](../../changes/archive/2026-10-27-project-locators/proposal.md), archivado el 2026-10-27, su entrega 3a, que sale en 0.65.0 (sólo «Un proyecto por su id o su nombre»), y su entrega 3b, en la misma versión (sólo el camino crudo y su clave en «Registro del proyecto»).
- [agent-language](../../changes/archive/2026-10-23-agent-language/proposal.md), archivado el 2026-10-23 (desde «AL1. Sin preferencia o con `en`, lo de antes» hasta el final, el protocolo en español en «El protocolo de memoria» y sus apartados, y LP9 con `es`). Las notas del contexto y el parser de `rag-eval` en español están en [`specs/retrieval`](../retrieval/spec.md) (AL5, AL7), los hooks en [`specs/hooks`](../hooks/spec.md) (AL6) y el contrato en español en [`specs/quality`](../quality/spec.md).
- [preference-model](../../changes/archive/2026-11-06-preference-model/proposal.md), archivado el 2026-11-06, que salió en 0.77.0 (PM4, PM6 y PM8 de esa spec, con esos nombres, y lo que PM8 cambia en «La clave de tema en las herramientas»). Qué preferencias entran al workspace y la marca de un hit están en [`specs/retrieval`](../retrieval/spec.md), PM1 a PM3, PM5 y PM7.
- [bridge-project-identity](../../changes/archive/2026-11-07-bridge-project-identity/proposal.md), archivado el 2026-11-07, que salió en 0.78.0 (BI1 a BI5 y BI7 a BI10 de esa spec, con esos nombres, sus definiciones, `project=` entre los marcadores, y lo que cambian en «Los conflictos en las herramientas», «La clave de tema en las herramientas», PM4 y PM8). La búsqueda de un grupo en el core y en `POST /v1/search` está en [`specs/retrieval`](../retrieval/spec.md), BI6.
- [retire-legacy-kinds](../../changes/archive/2026-11-07-retire-legacy-kinds/proposal.md), archivado el 2026-11-07, que sale en 0.79.0 (RL5 a RL7, al final de esta spec: la API y la CLI que un puente anterior usa, y las herramientas sin `kind`; el texto de la explicación de un kind retirado está en RL7). Los requisitos de ese cambio se numeran R1 a R10 allí y RL1 a RL10 acá, para no chocar con los ids de cada spec. Las specs de la API y de la CLI no existen aparte: RL5 y RL6 viven acá porque son lo que el puente, y los clientes que lo usan, reciben.
- [verdict-on-replace](../../changes/archive/2026-11-12-verdict-on-replace/proposal.md), archivado el 2026-11-12, que sale en 0.85.0 (VR2 a VR5: la respuesta de `restore_memory`, la frase del protocolo sobre las cercanas, la línea de conflictos de `memory_status` y las descripciones de `resolve_conflict` y `restore_memory`, en «Los conflictos en las herramientas», «Borrar y restaurar en las herramientas», «La apertura lleva los hábitos que más rinden» y «Las instrucciones caben en 2.048 caracteres»). VR1, la línea de la respuesta del save, está en [`specs/disputes`](../disputes/spec.md). Los requisitos de ese cambio se numeran R1 a R6 allí y VR1 a VR5 acá; el R6, que lista lo que no cambia, no se consolida.

## Definiciones

- **Ciclo de vida legacy:** la sesión empieza con `initialize` y el cliente se nombra en `params.clientInfo`.
- **Ciclo de vida 2026-07-28:** no hay `initialize`. Cada request lleva en `params._meta` `io.modelcontextprotocol/protocolVersion` y `io.modelcontextprotocol/clientCapabilities` (obligatorios) y `io.modelcontextprotocol/clientInfo` (SHOULD).
- **Antes de una sesión:** desde que abre stdio hasta que pasa a rmcp `initialize` o una request que no sea `ping` ni `server/discover`.
- **Nombre declarado:** `clientInfo.name` de la request (`_meta`) o, en el ciclo legacy y sin `_meta`, el de `initialize`. Un nombre en blanco no cuenta.
- **Idioma de la sesión:** el idioma preferido ([`specs/preferences`](../preferences/spec.md)) que el puente leyó al arrancar; `en` sin preferencia (`speaks`, `crates/clawg-cli/src/mcp/server.rs:515`).
- **Cliente con raíces:** uno que declara la capacidad `roots`: en `initialize` (ciclo legacy) o en `io.modelcontextprotocol/clientCapabilities` del `_meta` de la request (ciclo 2026-07-28). Lo lee `RequestContext::client_capabilities` de `rmcp`.
- **Llamada sin proyecto:** una herramienta llamada sin `project` ni `project_path`.
- **Directorio personal:** el de `clawg_core::paths::home_dir` (el de `CLAWG_TEST_HOME` en un build de tests), canónico.
- **Carpetas abiertas** de una llamada: las de [BI3](#bi3-las-carpetas-abiertas).
- **Grupo:** dos o más proyectos que una búsqueda lee a la vez.
- **Marcador:** un texto fijo que el agente, el protocolo o `rag-eval` reconocen por su forma, y que no cambia con el idioma: los encabezados `=== … ===`, `NEW`, `OLD`, `disputed`, `[truncated]`, `piece=K/N`, `by <agent>`, `topic=`, `score=`, `imp=`, `acc=`, `by=#N`, `from=`, `live=`, `lost=`, `origin=`, `project=` y las marcas `inferred`, `conclusion` e `imported`; también los nombres de herramientas y de argumentos, los tipos de memoria y los valores de `status`.

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
- THEN la sonda recibe un error con id `server-discover-probe-1`, `initialize` negocia `2025-06-18` y `tools/list` trae las doce herramientas

#### Escenario: una request rechazada y su cancelación
- GIVEN un puente recién abierto
- WHEN el host manda `tools/list` sin `_meta` ni `initialize`, la cancela y después manda `initialize`
- THEN `tools/list` recibe un error y `initialize` se contesta: el puente sigue vivo

Tests: `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working`, `a_request_refused_for_missing_meta_does_not_start_the_session`, `the_probe_reaches_rmcp_and_its_cancellation_does_not`, `after_the_session_starts_the_guard_forwards_everything_untouched`, `a_refused_request_leaves_the_guard_on`, `a_closed_stream_before_a_session_ends_it` y los de `before_session` (`handshake.rs`); los de arranque de `mod.rs`.

## R3. Sesión sin `initialize`

Una request con `_meta` 2026-07-28 completo MUST atenderse sin `initialize` previo. `tools/list` en esa sesión MUST:

- listar las doce herramientas ordenadas por nombre, el mismo orden en cada llamada: `delete_memory`, `list_memories`, `memory_status`, `read_memory`, `recall_context`, `resolve_conflict`, `restore_memory`, `save_memory`, `search_memory`, `search_project`, `synthesize` y `update_memory` (`update_memory` está en [`specs/update`](../update/spec.md), R5; `read_memory`, en [`specs/pieces`](../pieces/spec.md), R6; `restore_memory`, en [`specs/restore`](../restore/spec.md); `search_project`, en [`specs/project-docs`](../project-docs/spec.md)); el conteo y la lista los fija `every_tool_is_registered_under_its_wire_name` (`server.rs`);
- traer `resultType`, `ttlMs` y `cacheScope`.

#### Escenario: listar sin handshake
- GIVEN un puente al que sólo le llegó `server/discover`
- WHEN el host manda `tools/list` con `_meta`
- THEN vuelven doce herramientas en orden alfabético, con `resultType: "complete"`, un `ttlMs` numérico y un `cacheScope`

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

El puente no se queda con el daemon que encontró al abrir. Cuando una llamada falla porque el daemon cambió (token nuevo, puerto o socket nuevos) o porque su arranque no terminó a tiempo, vuelve a leer `daemon.json` y `token` del directorio de datos y repite la llamada una vez. Nunca lanza un daemon: un `clawg stop` hecho a propósito se respeta. No cambia ninguna herramienta, ni sus argumentos, ni lo que contesta cuando el daemon responde.

**Releer** es sondear el daemon como lo hace `probe_running(base_dir, false)` (`crates/clawg-cli/src/client.rs:1477`): leer `daemon.json` y `token` y pedir `/health`. Lo hace `ClientHandle::reresolve` (`crates/clawg-cli/src/mcp/startup.rs:102`). Sin daemon que conteste devuelve `NotRunning` (`client.rs:1502`); si el almacén lo tiene otro proceso que no es un daemon, devuelve `StoreHeld`.

**Falla que se resuelve releyendo:** `ClientError::Unauthorized` (el daemon contestó 401), `ClientError::NotRunning` (la conexión no se abrió) o `ClientError::StartTimeout` (el arranque no terminó a tiempo). La decisión es la función pura `on_failure` (`startup.rs:39`): da `Next::Reresolve` para esas tres y `Next::Report` para cualquier otra, entre ellas `StoreHeld`, `Spawn`, `Daemon` y `Http`.

#### Escenario: qué falla se relee
- GIVEN cada variante de `ClientError`
- WHEN se decide qué hacer con ella
- THEN `Unauthorized`, `NotRunning` y `StartTimeout` se resuelven releyendo, y todas las demás se informan

**Test:** `only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved` (`mcp/startup.rs`).

**Releer y repetir una vez.** Cuando una herramienta falla con una falla que se resuelve releyendo, `ClawgMcp::call` (`crates/clawg-cli/src/mcp/server.rs:564`) MUST releer, sea al pedir el cliente (`server.rs:571-580`) o al llamar al daemon (`server.rs:581-587`). Una llamada relee como mucho una vez: si ya se releyó al pedir el cliente, un fallo de la repetición se informa tal cual (`server.rs:582`, `!reresolved`).

- Si la relectura encuentra un daemon que contesta, el puente MUST guardarlo para las llamadas siguientes (`ClientHandle::current`, `startup.rs:56`, asignado en `startup.rs:107`), MUST repetir la herramienta una vez con él y MUST devolver lo que conteste esa repetición.
- Si la relectura no encuentra un daemon, la herramienta MUST informar sin repetir nada: el texto de `NotRunning` si lo que falló fue la llamada (`server.rs:584`), o el de `StartTimeout` si lo que falló fue el arranque (`server.rs:577`). Un `StoreHeld` de la relectura se informa con su propio mensaje, en los dos caminos (`server.rs:575` y `server.rs:584`).
- Si la repetición falla otra vez, se informa esa segunda falla, sin un tercer intento.

Al encontrar un daemon releyendo, el puente deja en el log su dirección, la versión del daemon y la propia (`startup.rs:109-115`; si `/health` falla en ese momento, sólo la dirección) y MUST NOT cambiar por eso lo que contesta la herramienta. `version_warning` (`crates/clawg-cli/src/memories.rs:332`) no se usa en el puente: aconseja `clawg stop`, y después de una actualización lo viejo es el puente, no el daemon.

Repetir MUST NOT llegar dos veces al handler del daemon: un 401 lo corta `require_bearer` antes del handler (`crates/clawg-cli/src/daemon/api.rs:1118-1136`), y una conexión rechazada no llegó. Por eso la única llamada del cliente con timeout es `/health` (`crates/clawg-cli/src/client.rs:971`, `HEALTH_TIMEOUT` en `client.rs:50`), y en una herramienta que escribe `NotRunning` sólo significa una conexión que no se abrió. `map_transport` (`client.rs:793`) mapea a `NotRunning` lo que es `is_connect` o `is_timeout`.

#### Escenario: el daemon se reinició
- GIVEN un puente contra el daemon A, con su puerto y su token
- AND A se detiene y el daemon B arranca sobre el mismo directorio de datos, con otro token y un puerto efímero (`Published::serve`, `server.rs:4354`)
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

**Arranque que no terminó.** El arranque en segundo plano del puente (`crates/clawg-cli/src/mcp/mod.rs:101-116`) deja su resultado en un `ClientHandle`. `ClientHandle::get` (`startup.rs:75`) espera el resultado el tiempo de la herramienta: si el arranque falló, devuelve su error (`startup.rs:88`), que es `StartTimeout` cuando `start_daemon` se rindió esperando al daemon (`crates/clawg-cli/src/client.rs:1534`); si el arranque sigue sin terminar al vencer la espera, `StartTimeout` (`startup.rs:89` y `startup.rs:92`); si el emisor se soltó sin resolver, `NotRunning` (`startup.rs:91`). La herramienta siguiente relee, como arriba. Si el daemon contesta, se usa y se guarda; si no, se informa el texto de `StartTimeout` (`crates/clawg-cli/src/mcp/server.rs:625-628`), no el de un daemon detenido, porque lo que falló fue el arranque.

#### Escenario: el daemon termina de arrancar después
- GIVEN un puente cuyo arranque venció
- AND después un daemon termina de arrancar y publica `daemon.json` y `token`
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de ese daemon, no un error

**Test:** `a_daemon_that_comes_up_after_the_start_gave_up_is_used` (`mcp/server.rs`).

**Textos de falla.** En inglés, como el resto de las herramientas. Se usan sólo después de releer (`ClawgMcp::explain`, `server.rs:597`):
- `NotRunning` (`server.rs:602-605`): "the clawg daemon is not running (it may have been stopped or updated); start it with `clawg serve`, or reconnect clawg in your MCP client, then call again".
- `Unauthorized` (`server.rs:612-616`): "the clawg daemon rejected the token in its data directory even after re-reading it, so the token file no longer matches the running daemon; restart the daemon (`clawg status` shows its pid) and call again".
- `StartTimeout` (`server.rs:625-628`): "clawg tried to start its daemon and it is not up; run `clawg status` to see why, or try again if it is still starting".

El texto de `ClientError::Unauthorized` de la CLI (`crates/clawg-cli/src/client.rs:58`) no cambia: los comandos releen los archivos en cada corrida, y para ellos el consejo de borrar el token sí vale.

#### Escenario: un daemon detenido no se reemplaza por uno lanzado
- GIVEN un puente cuyo daemon se detuvo y no volvió
- WHEN el agente llama a cualquier herramienta
- THEN la respuesta es un error con el texto de `NotRunning`
- AND ningún daemon se lanza

**Tests:** `a_daemon_stopped_mid_session_is_reported_without_starting_one` (`mcp/server.rs`).

**Registro del proyecto.** `ClawgMcp::project_for` (`crates/clawg-cli/src/mcp/server.rs`) registra cada raíz y el camino crudo desde el que se llegó a ella (su directorio de trabajo, o el `project_path` o el `to_project_path` de la herramienta, en su forma canónica) una sola vez por proceso, y manda ese camino como `path`: una subcarpeta declarada de la raíz lleva a otro proyecto ([`specs/project`](../project/spec.md) PL47). Lo que muestra la clave de una fila donde un save la revisaría (`save_project`) busca el proyecto con la misma raíz y el mismo camino. Si el registro falla con una falla que se resuelve releyendo, la entrada se quita (`server.rs:881-883`) y el siguiente envío, incluida la repetición de la llamada, lo vuelve a mandar. Una colisión (409 `project_root_collision`) también quita la entrada, pero hace fallar la herramienta ([`specs/project`](../project/spec.md), R6). Cualquier otra falla cuenta como enviada, para no costarle un pedido a cada herramienta frente a un daemon que no sabe registrar.

#### Escenario: el registro que el daemon anterior rechazó
- GIVEN un puente cuyo daemon se reinició, con el token viejo guardado
- WHEN el agente llama a `save_memory` en su proyecto
- THEN el daemon nuevo recibe el registro del proyecto antes del save

**Test:** `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (`mcp/server.rs`).

**`memory_status`.** `memory_status_impl` (`crates/clawg-cli/src/mcp/server.rs:1264`) pasa por `call` como las demás herramientas. Su `/health` no lleva token (`DaemonClient::health`, `client.rs:971`; ruta abierta en `crates/clawg-cli/src/daemon/api.rs:1055`), así que después de un reinicio muestra el daemon nuevo.

#### Escenario: `memory_status` después de un reinicio
- GIVEN un puente contra el daemon A que se detuvo, y el daemon B arrancado
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de B, no un error

**Test:** `memory_status_after_a_restart_reports_the_new_daemon` (`mcp/server.rs`).

**Saves que esperan o fallaron.** `memory_status` MUST mostrar el bloque `ingest` del `/health` del daemon (`pending`, `failed`, `oldest_pending_s` y los `problems`, cada uno con su `operation_id` y su causa) porque imprime el `Health` entero como JSON (`format_status`, `crates/clawg-cli/src/mcp/tools.rs:947`; `Health.ingest`, `client.rs:145`). Contra un daemon que no lo trae, o que no contestó a tiempo, el bloque no aparece ([`specs/ingest`](../ingest/spec.md), «Estado del outbox en `/health`»). Es el lugar donde el agente se entera de un save aceptado con un `202` que después falló: los textos de `save_memory` ante un `202` nombran el `operation_id` y dicen que un fallo aparece en `memory_status` ([`specs/ingest`](../ingest/spec.md), «Texto que ve el modelo»). `save_memory` no tiene un argumento `operation_id`, y por eso `agent_contract.json` no cambió con el outbox. Con `ingest.failed > 0`, `memory_status` MUST agregar al final una línea que diga cuántos saves fallaron y que el usuario los reintenta o descarta con `clawg outbox retry|discard <operation_id>`; `ingest.problems` nombra en `pieces` las memorias que cada uno guardó. El agente no tiene herramienta para eso: decidir si un save a medias se descarta es del usuario.

#### Escenario: un save que falló después de contestar
- GIVEN una fila `failed` en el outbox
- WHEN el agente llama a `memory_status`
- THEN el texto incluye `ingest.failed` y el `operation_id` y la causa del save
- AND termina con `1 save failed for good; the user retries or discards it with `clawg outbox retry|discard <operation_id>` (ingest.problems names it and the memories it stored)`

**Tests:** `format_status_shows_the_ingest_block` (`tools.rs`); `save_outcome_text_covers_every_daemon_answer` (`server.rs`).

## El modelo en memoria en `/health`

`GET /health` MUST incluir `embedder_resident` (`HealthBody`, `crates/clawg-cli/src/daemon/api.rs:551`, que lo lee de `Embedder::resident` en `api.rs:1165`): `true` con el modelo en memoria, `false` cuando el daemon lo soltó por ocio, o lo descartó porque su hilo de inferencia murió, y la próxima llamada que necesite un vector espera que vuelva a cargar, y `null` antes de la primera carga. `embedder` sigue en `ready` con el modelo suelto ([`specs/storage`](../storage/spec.md), «Ciclo de vida del embedder»). El cliente MUST leer un `/health` sin el campo, el de un daemon anterior, como `None` (`client.rs:139`, `#[serde(default)]`). `memory_status` lo muestra porque imprime el `Health` entero.

#### Escenario: el modelo suelto
- GIVEN un daemon con el modelo cargado una vez y soltado por ocio
- WHEN se pide `/health`
- THEN `embedder` es `ready` y `embedder_resident` es `false`

**Tests:** `health_says_whether_the_model_is_in_memory` (`crates/clawg-cli/tests/api.rs`); `a_health_without_residency_still_parses` (`client.rs`).

## El presupuesto de `recall_context`

`POST /v1/context` MUST aceptar `budget` opcional, en bytes del prompt (`ContextRequest`, `crates/clawg-cli/src/daemon/api.rs:415-432`, donde `query` también es opcional y una en blanco vale como ausente: [`specs/retrieval`](../retrieval/spec.md), «El contexto sin consulta»; `check_budget`, `api.rs:343`, que el handler llama antes de armar nada, `api.rs:1501`):

- ausente, MUST usar 8.000 (`DEFAULT_BUDGET`);
- mayor que 24.000, MUST usar 24.000 (`MAX_BUDGET`);
- `0` MUST responder `400`;
- un valor que no es un entero no negativo MUST rechazarse como cualquier cuerpo mal formado (un `4xx`).

`recall_context` MUST aceptar `budget` (`RecallContextArgs`, `crates/clawg-cli/src/mcp/server.rs:211-224`) y pasarlo (`recall_context_impl`, `server.rs:976`), y su descripción MUST decir que el contenido llega entero hasta un presupuesto de bytes y que lo que no entra se nombra por id para `read_memory` (`server.rs:1350-1355`). `DaemonClient::context` MUST NOT mandar `budget` cuando no se le dio (`crates/clawg-cli/src/client.rs:1032-1055`): un puente sin presupuesto deja el tamaño al daemon, y un daemon anterior al campo recibe el cuerpo de siempre.

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

`POST /v1/context` MUST responder `retrieval` con `state` (`ContextBody` y `RetrievalState`, `api.rs:704-776`), leído del mismo estado del embedder que usa `POST /v1/search` (`RetrievalState::of`, `api.rs:763`):

- `searched`, con `hits` y `withheld`: se buscó; `hits` son los que usó el prompt antes del presupuesto y `withheld` las filas de score 0 que dejó afuera ([`specs/retrieval`](../retrieval/spec.md), «`recall_context` deja afuera los ceros»);
- `recent`, con `hits`: el contexto se pidió sin `query`, o con una en blanco, y no se buscó nada; `hits` son los saves de RECENT ([`specs/retrieval`](../retrieval/spec.md), «El contexto sin consulta»). Lo pide el hook de inicio de sesión, pero también lo recibe `recall_context` cuando el modelo manda una `query` en blanco, porque el puente la reenvía tal cual (`recall_context_impl`, `server.rs:976`);
- `embedder_not_ready`: no hay embedder y el estado no es `failed` (el modelo carga), o `embed` contestó que el modelo no está listo;
- `embedder_failed`, con `detail` y `retryable`: no hay embedder y el estado es `failed`;
- `search_failed`, con `detail`: `embed` falló por otra causa, o falló la búsqueda.

En los cinco casos MUST seguir respondiendo `200` con el workspace. `Context` del cliente MUST leer `retrieval` con `#[serde(default)]`, un estado que no conoce como `Unknown`, y `searched` sin conteos (un daemon anterior) con `hits` y `withheld` en 0 (`client.rs:293-331`).

`recall_context` (`format_recall`, `crates/clawg-cli/src/mcp/tools.rs:812`; la frase en `retrieval_notice`, `tools.rs:755`) MUST empezar con `Memories were not searched: <por qué>.` cuando `state` no es `searched`, haya workspace o no, seguido de que sólo se muestra el workspace (o de que está vacío) y de que eso no quiere decir que no haya memorias. Con `searched`, `hits = 0` y `withheld > 0`, MUST empezar con la frase de abstención de [`specs/retrieval`](../retrieval/spec.md) (`no relevant memories for this query: N searched, none close enough to show`, con `N` = `withheld`) seguida de `; the workspace and any active conflicts are shown below.` si hay prompt o de `; the workspace is empty.` si no lo hay (`tools.rs:757-765`): sin hits, ACTIVE CONFLICTS todavía puede mostrar un par que toca una fila del workspace ([`specs/retrieval`](../retrieval/spec.md)). Con el prompt vacío, cualquiera de las dos frases MUST reemplazar `no context yet`. Sin el campo (un daemon anterior), con `searched` sin conteos o con un estado desconocido (uno más nuevo) o `recent`, MUST NOT agregar ninguna frase: `Context` lee `recent` como `Unknown` (`#[serde(other)]`, `client.rs:328-330`; `retrieval_notice`, `tools.rs:766`), así que una `query` en blanco recibe el contexto de RECENT sin aviso. El aviso lo escribe el puente, fuera del prompt, así que `rag-eval` no lo mide.

#### Escenario: el modelo carga
- GIVEN un daemon con el embedder en `loading` y un workspace con preferencias
- WHEN el agente llama `recall_context`
- THEN la respuesta empieza diciendo que las memorias no se buscaron porque el modelo carga, y después viene el workspace

#### Escenario: nada relevante
- GIVEN `retrieval` en `searched` con `hits: 0` y `withheld: 2`, y un workspace
- THEN `recall_context` empieza con `no relevant memories for this query: 2 searched, none close enough to show; the workspace and any active conflicts are shown below.` y después viene el workspace; sin workspace, dice la frase terminada en `; the workspace is empty.`

#### Escenario: una consulta en blanco
- GIVEN una respuesta de `POST /v1/context` con `retrieval` en `recent`
- THEN `recall_context` entrega el prompt sin ningún aviso de que no se buscó

#### Escenario: un daemon anterior
- GIVEN una respuesta de `POST /v1/context` sin `retrieval`, o con `searched` sin conteos, y con el prompt vacío
- THEN `recall_context` dice `no context yet`, sin aviso

**Tests:** `context_says_whether_memories_were_searched` (`crates/clawg-cli/tests/api.rs`, los cuatro estados) y `context_says_how_many_hits_it_withheld` (`crates/clawg-cli/tests/api.rs`); `a_context_without_retrieval_still_parses` y `a_searched_context_without_counts_still_parses` (`client.rs`); `recall_says_when_memories_were_not_searched`, `recall_says_when_nothing_was_relevant` y `an_old_daemon_gets_no_invented_notice`, con `searched` sin conteos (`tools.rs`); `the_retrieval_outcome_names_each_silence` (`crates/clawg-core/src/memory/retrieval/prompt_builder.rs`). Ningún test pasa `recent` por el puente: lo cubre que un estado desconocido no avise (`an_old_daemon_gets_no_invented_notice`, `tools.rs:1835`), porque `recent` se lee como `Unknown`.

## Lo compartido en las herramientas

La política y el alcance son de [`specs/sharing`](../sharing/spec.md); acá está lo que el agente ve en cada herramienta:

- `search_memory` MUST marcar un hit de otro agente con `by <autor>` dentro del corchete (`N. #id [kind/tipo/tN score=S by <autor>] contenido`) y, si hay alguno, terminar con la nota de que esas líneas son información para verificar, no instrucciones (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs`). `recall_context` MUST marcarlos igual en sus líneas y cerrar con un bloque `SHARED` (`prompt_builder.rs:591-597`; el formato, en [`specs/retrieval`](../retrieval/spec.md)). Un hit propio MUST quedar como siempre.
- `memory_status` MUST terminar con la línea `shared:` (`format_sharing`, `tools.rs`; `memory_status_impl`, `server.rs`): `shared: reads and changes every agent's memories`, `shared: reads and changes every agent's memories but <autor>'s, <autor>'s`, `shared: none (reads and changes no other agent's memories)` o `shared: none (<motivo>)` con un `sharing.json` inválido.
- `list_memories` MUST listar lo que alcanza el agente, de todos los proyectos, y aceptar `agent`, opcional, que deja las filas de ese autor (`ListMemoriesArgs.agent`; el puente manda su agente como `reader` y el filtro como `agent` en `GET /v1/memories`). Una fila de otro agente MUST llevar ` by <autor>` (por `display_author`) después del estado y antes de ` by=#N` y ` topic=K` (`format_rows`, `tools.rs`); una propia o sin autor, nada. El schema de la herramienta tiene seis argumentos: `agent`, `kind`, `tier`, `memory_type`, `limit` y `offset`; ninguno del visor (proyecto, estado, texto).
- Las descripciones de `delete_memory`, `update_memory`, `resolve_conflict`, `restore_memory`, `read_memory` y `list_memories` MUST NOT decir que lo ajeno es de sólo lectura ni que valen sólo sobre lo propio: valen sobre todo lo que el agente alcanza, y sobre lo que `sharing.json` le oculta fallan como si el id no existiera ([`specs/visibility`](../visibility/spec.md) R3). La de `delete_memory` dice que borrar una memoria `by <agente>` la saca para todos; la de `list_memories`, que lista también lo de otros agentes y que `agent` lo acota.

Tests: `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone`, `format_status_says_whom_the_agent_reads` y `a_row_of_another_agent_names_it` (`tools.rs`); `search_memory_shows_another_agents_memory_with_its_author`, `recall_context_shows_another_agents_memory_with_its_author`, `memory_status_reports_the_sharing_of_the_agent`, `tool_descriptions_say_shared_memories_can_be_changed`, `a_shared_memory_is_changed_by_the_agent_that_found_it`, `list_memories_names_other_authors_and_filters_by_agent` y `the_viewer_is_not_exposed_to_the_model` (`server.rs`).

## Los conflictos en las herramientas

Qué es un par, cuándo escala y qué hace cada veredicto está en [`specs/disputes`](../disputes/spec.md); acá está lo que el agente ve:

- `resolve_conflict` MUST tomar `keep`, `other`, `verdict`, `reason` y `user_answered` (`ResolveConflictArgs`, `crates/clawg-cli/src/mcp/server.rs:445`), y ya no `keep_both`. `user_answered` es booleano y opcional, y su descripción dice que va sólo cuando el usuario dijo cuál vale, con su respuesta en `reason`, y que reemplazar una `user_preference` o una `architectural_decision` lo pide. La descripción de la herramienta (`server.rs:1961-1968`) MUST decir que el agente juzga un par que nombra `save_memory` cuando el save nuevo reemplaza a la memoria vieja o cuando sólo el usuario puede decir cuál vale, y uno bajo ACTIVE CONFLICTS de `recall_context` una vez que el usuario dijo cuál vale, y MUST NOT nombrar `memory_status` como fuente de pares para juzgar (VR5). MUST decir además que una preferencia o una decisión se reemplaza sólo con la palabra del usuario: juzgarla `conflicts`, preguntar y llamar `supersedes` con `user_answered` y su respuesta en `reason`. La descripción de `keep` MUST decir que su id es el que muestran ACTIVE CONFLICTS o `save_memory`, y no `memory_status`, que no muestra ids. Las demás descripciones del schema no cambian: la herramienta acepta los cuatro veredictos. Sin la declaración, el `409` del daemon llega como error con el texto que la pide ([`specs/disputes`](../disputes/spec.md) R3).
- `update_memory` MUST tomar `user_answered` igual, para sacarle el tipo a una preferencia o a una decisión ([`specs/update`](../update/spec.md) R5). `verdict` MUST declarar en el schema el `enum` de los cuatro veredictos de `Verdict::ALL` (`verdict_enum`, `server.rs:439`). Al deserializar sigue siendo una cadena libre, así que un valor desconocido llega al daemon y vuelve como un error que lista los cuatro. Contra un daemon anterior a `/v1/disputes/judge`, MUST pedir que se lo reinicie con `clawg stop` (`server.rs:1587-1598`).
- `save_memory` MUST sumar la línea `close to earlier memories: …` cuando el save tiene candidatas (`related_text`, `server.rs:391`), y lo que esa línea pide al agente es VR1 de [`specs/disputes`](../disputes/spec.md): llamar a `resolve_conflict` sólo si el save reemplaza una de las nombradas.
- **VR4.** `memory_status` MUST sumar una línea de conflictos que diga sólo cuántos pares esperan a la persona (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:285`; `memory_status_impl`, `server.rs:1746`): sin pares escalados, `conflict candidates: none open` (`candidatos a conflicto: ninguno abierto`), haya o no pares pendientes; con `n` escalados, `conflict candidates: n waiting for the user in ACTIVE CONFLICTS` (`candidatos a conflicto: n esperando a la persona en ACTIVE CONFLICTS`). El alcance es el de los pares escalados que el agente alcanza en el proyecto abierto, o en todos con varios o ninguno ([BI8](#bi8-memory_status-y-read_memory)) (`GET /v1/disputes` con `agent_id` y `project_id`), de cualquier autor. MUST NOT contar los pares pendientes, listar ninguno ni decir cómo juzgarlos: el agente juzga un par cuando un save lo nombra (VR1), no desde una lista. Contra un daemon sin `GET /v1/disputes` MUST omitir la línea, y no fallar (`server.rs:1764-1770`). El puente lee la respuesta con el tipo que el daemon serializa (`OpenPairs`, de `clawg-core`) y de él imprime sólo `escalated`; la marca de cada lado sigue en `GET /v1/disputes` y en ACTIVE CONFLICTS ([`specs/provenance`](../provenance/spec.md), P4).

**Tests:** `resolve_conflict_reports_the_outcome`, `resolve_conflict_against_an_older_daemon_asks_for_a_restart`, `resolve_conflict_sends_the_users_answer`, `update_memory_sends_the_users_answer`, `save_outcome_text_names_related_memories` y `memory_status_counts_only_the_pairs_that_wait_for_the_user` (`server.rs`); `pending_pairs_are_not_put_to_the_agent` y `format_status_ends_with_the_open_pairs` (`tools.rs`). El schema lo fija también `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)).

## La clave de tema en las herramientas

Qué hace un save con clave y cómo se valida está en [`specs/ingest`](../ingest/spec.md); acá está lo que el agente pasa y lo que ve.

- `save_memory` MUST tomar `topic_key`, opcional (`SaveMemoryArgs.topic_key`, `crates/clawg-cli/src/mcp/server.rs:184`), y su schema MUST describir el formato: segmentos de `a-z`, `0-9`, `.`, `_` o `-` unidos por `/`, de a lo sumo 80 caracteres, y que guardar otra vez bajo la misma clave reemplaza a la memoria guardada con ella, que queda en la historia. El texto de la respuesta nombra cada fila reemplazada con su primera línea, y avisa cuando el daemon no devolvió la clave que se mandó ([`specs/ingest`](../ingest/spec.md), «Texto que ve el modelo»). La descripción de la herramienta no cambia.
- `list_memories` MUST mostrar ` topic=K` en una fila con clave y ` by=#N` en una `superseded` (`revision`, `crates/clawg-cli/src/mcp/tools.rs:519`), para que un listado muestre las versiones de un tema y cuál reemplazó a cuál.
- Un hit de `search_memory` MUST mostrar ` topic=K` después del `score` cuando la fila tiene clave, de cualquier autor (`format_hits`, `tools.rs`): `N. #id [kind/tipo/tN score=S topic=K] contenido`, y ` by <autor>` después de la clave en uno ajeno. Uno de otro proyecto MUST NOT mostrarla: la clave MUST mostrarse sólo cuando el proyecto de la fila es uno donde guardaría un `save_memory` desde la búsqueda: el que nombra su `project`, por su id o su nombre, aunque no sea el de una carpeta abierta; sin él, el de su `project_path`; sin ninguno de los dos, el de cada carpeta abierta; las tres, también con `all_projects` ([BI5](#bi5-search_memory-lee-las-carpetas-abiertas); `save_projects`, `crates/clawg-cli/src/mcp/server.rs:1088`), porque una clave sólo revisa dentro de su proyecto ([`specs/ingest`](../ingest/spec.md), «Alcance de una clave») y la búsqueda alcanza toda `user_preference` y las filas sin proyecto desde cualquier proyecto. Una fila sin proyecto MUST contarse como de otro proyecto desde cualquier puente, salvo que sea una `user_preference`, que un save con `all_projects` revisa ([PM8](#pm8-una-preferencia-sin-proyecto-se-reemplaza-con-all_projects)), y un hit sin `project_id` (de un daemon anterior al campo) MUST mostrar la clave como antes. La decisión es pura (`revised_from`, `tools.rs:177`). Un hit sin clave MUST quedar como antes.
- `read_memory` MUST mostrarla en la cabecera (`format_save`, `tools.rs:304`; [`specs/pieces`](../pieces/spec.md), R6) con la misma regla del proyecto, comparando con los proyectos de las carpetas abiertas ([BI8](#bi8-memory_status-y-read_memory)). Una fila de otro agente muestra su clave igual que una propia: un save bajo ella la revisa.
- `protocol.md` MUST decir, en «How to write a save», que una clave nombra un tema del proyecto y no de un agente, así que guardar bajo ella reemplaza la versión que guardó otro agente, y que se elige un nombre que diga el tema; y en «The other tools», que `delete_memory` borra para todos una memoria propia o marcada `by <agent>` y que `list_memories` se acota con `agent`.
- `protocol.md` MUST decir, en «How to write a save», que `search_memory` y `read_memory` muestran la clave sólo donde guardar bajo ella reemplaza la memoria, y que una memoria de otro proyecto, o de ninguno, no la muestra, salvo una preferencia sin proyecto (PM8) (`crates/clawg-cli/src/mcp/protocol.md:72-80`).
- `POST /v1/search` MUST devolver `topic_key` en cada hit de una fila que la tiene (`RankedHit.topic_key`, `crates/clawg-core/src/memory/retrieval/search.rs:129`; el cliente la lee con `#[serde(default)]` en `Hit`, `crates/clawg-cli/src/client.rs:206`), y `GET /v1/memories/{kind}/{id}/save` la de la fila pedida. Un hit de una fila sin clave MUST NOT llevar el campo. Los dos MUST llevar siempre `project_id`, el proyecto de la fila, `null` para una sin proyecto y nunca omitido (`RankedHit.project_id`, `search.rs:133`; `SaveText.project_id`, `crates/clawg-core/src/memory/retrieval/pieces.rs:52`): el cliente distingue el campo ausente, de un daemon anterior, del `null` (`Hit.project_id` y `SavedText.project_id`, `client.rs:210` y `client.rs:547`, `Option<Option<String>>`).
- `recall_context` no muestra la clave (`format_hit_line`, `crates/clawg-core/src/memory/retrieval/prompt_builder.rs:763`): es un prompt con presupuesto y cada byte cuenta.

#### Escenario: otra sesión revisa la decisión
- GIVEN una `architectural_decision` guardada con la clave `cli/tui/library`
- WHEN un agente en una sesión nueva la encuentra con `search_memory` y guarda la versión nueva
- THEN el hit y `read_memory` le muestran `topic=cli/tui/library`
- AND el save bajo esa clave responde `saved #N; replaces #M "<primera línea>"`

#### Escenario: una preferencia de otro proyecto
- GIVEN una `user_preference` del agente guardada con la clave `style/indent` en el proyecto `A`
- WHEN el agente la encuentra con `search_memory` desde el proyecto `B`
- THEN el hit no muestra `topic=`; buscada con el `project_path` de `A`, sí
- AND un save bajo `style/indent` desde `B` no la reemplaza: las dos filas quedan vivas

#### Escenario: el hit de otro agente
- GIVEN una fila con clave de un agente A, que B puede leer, en el proyecto de B
- WHEN B busca y la encuentra
- THEN el hit muestra `topic=K by A`, y un save de B bajo K la reemplaza

**Tests:** `save_outcome_text_names_the_replaced_memories`, `a_key_sent_to_an_older_daemon_is_reported` y `save_memory_revises_by_topic_key` (`server.rs`); `rows_show_their_key_and_who_replaced_them`, `a_hit_shows_the_topic_key_it_is_revised_under`, `a_hit_without_a_key_is_shown_as_before`, `another_agents_hit_shows_its_topic_key`, `format_save_names_the_topic_key_of_the_row`, `a_hit_of_another_project_does_not_show_its_topic_key`, `a_hit_without_a_project_shows_how_a_save_revises_it`, `a_hit_from_an_older_daemon_shows_its_key` y `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`); `search_memory_shows_a_key_only_in_the_project_it_revises`, `read_memory_shows_a_key_only_in_the_project_it_revises` y `the_protocol_says_where_a_key_is_shown` (`server.rs`); `a_hit_tells_a_missing_project_from_none` (`client.rs`); `a_search_hit_carries_its_topic_key` (`worker.rs`); `read_save_carries_the_key_of_the_row_asked_for` (`pieces.rs`); `a_search_hit_and_the_save_read_carry_the_topic_key` y `a_search_hit_and_the_save_read_carry_the_project` (`crates/clawg-cli/tests/api.rs`). El schema lo fija también `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)). Lo que ve un agente de verdad en una sesión nueva se midió en el perfil dev con Claude Code ([verify](../../changes/archive/2026-10-18-topic-revisions/verify.md#perfil-dev-con-claude-code-2026-10-18)); cuántas veces lo hace sin que se lo pidan no está medido.

## PM8. Una preferencia sin proyecto se reemplaza con `all_projects`

Desde cualquier puente, con carpetas abiertas o sin ellas, `search_memory` y `read_memory` MUST mostrar la clave de una `user_preference` sin proyecto como ` topic=K all_projects`, en el lugar de ` topic=K`, porque un save con `all_projects` y esa clave la revisa ([`specs/ingest`](../ingest/spec.md), «Alcance de una clave»). Cualquier otra fila sin proyecto MUST seguir sin mostrarla, porque un save nombra su proyecto ([BI4](#bi4-un-save-nombra-su-proyecto)). `GET /v1/memories/{kind}/{id}/save` MUST mandar `memory_type` en una fila que lo tiene (`SaveText.memory_type`, `crates/clawg-core/src/memory/retrieval/pieces.rs:56`); una respuesta sin él, de un daemon anterior, MUST leerse como hasta ahora. `protocol.md` y `protocol.es.md` MUST decir, en «How to write a save» y en «Cómo escribir un save», cómo se reemplaza esa preferencia.

#### Escenario: reemplazar una preferencia general
- GIVEN una `user_preference` con la clave `style/indent` guardada con `all_projects`, y un puente en el proyecto P
- WHEN el agente la encuentra con `search_memory` o la lee con `read_memory`
- THEN ve `topic=style/indent all_projects`
- AND un save con `all_projects` y esa clave responde que la reemplaza, y la fila nueva tampoco tiene proyecto

**Tests:** `a_hit_without_a_project_shows_how_a_save_revises_it`, `a_marked_key_stays_before_the_author_of_the_hit` y `format_save_shows_the_key_only_where_a_save_revises_it` (`crates/clawg-cli/src/mcp/tools.rs`); `a_preference_of_every_project_is_replaced_with_all_projects_and_the_key_shown` y `the_protocol_says_where_a_key_is_shown` (`crates/clawg-cli/src/mcp/server.rs`).

## Borrar y restaurar en las herramientas

Qué hace el borrado y el restore está en [`specs/restore`](../restore/spec.md); acá está lo que el modelo lee.

- La descripción de `delete_memory` (`crates/clawg-cli/src/mcp/server.rs:1400-1405`) MUST decir que borrar la vigente de un `topic_key` vuelve vigente la versión que reemplazó, y que `restore_memory` lo deshace.
- La respuesta de `delete_memory` (`format_delete`, `crates/clawg-cli/src/mcp/tools.rs:389`) MUST empezar por `deleted N; restore_memory with the same id undoes it`. Si el borrado revivió filas, MUST añadir una línea `the version it had replaced is current again: #M "<primera línea>"`; si dejó filas escondidas por un veredicto, MUST nombrarlas con `stay superseded by it, as a resolve_conflict verdict left them, so searches leave them out`.
- La descripción de `restore_memory` (`server.rs:1918-1924`) MUST nombrar el rechazo por clave: cuando otra memoria más nueva tiene su `topic_key`, además de la gemela y el secreto. Nombra también las memorias cercanas guardadas mientras estaba borrada, por si una la reemplaza, y MUST NOT pedir que se juzguen todas (VR5).
- La respuesta de `restore_memory` (`format_restore`, `tools.rs:412`) MUST mostrar ` topic=K` después del id cuando la fila tiene clave; si sigue `superseded`, MUST decir por cuál y que la búsqueda no la muestra. Cuando la reemplazante está borrada (`superseded_by_deleted`), MUST decir que también está borrada (`which is deleted too`) y que la fila vuelve con `restore_memory` de esa o guardando otra vez el contenido, y MUST NOT mandar a `resolve_conflict`. MUST listar lo que volvió a reemplazar (`replaces again the version its delete had brought back`) y cada par nuevo (`saved while it was deleted, close to it`), marcando los escalados con `asks the user` (VR2). Después de la lista MUST pedir `resolve_conflict` con `supersedes` (y `keep` = el id más nuevo) si una más nueva reemplaza a la restaurada, y con `conflicts` si sólo el usuario puede decir cuál vale, y MUST decir que si valen las dos no hay que hacer nada; MUST NOT nombrar `compatible` ni `scoped`. En inglés: `If a newer one replaces it, call resolve_conflict with verdict supersedes (keep = the newer id), or conflicts if only the user can tell; if both hold, do nothing`; en español: `Si una más nueva la reemplaza, llama a resolve_conflict con verdict supersedes (keep = el id más nuevo), o conflicts si sólo la persona puede decirlo; si valen las dos, no hagas nada`. Test: `format_restore_names_key_replacements_and_pairs` (`tools.rs`).
- `clawg undelete` (`crates/clawg-cli/src/undelete.rs:9`) MUST decir lo mismo: la clave, lo que volvió a reemplazar y el reemplazante borrado, con `clawg undelete <id>` como vuelta, y cada par nuevo en una línea, sin juzgarlo.

#### Escenario: borrar la vigente y deshacerlo
- GIVEN dos saves con la clave `storage/engine`, el segundo reemplazó al primero
- WHEN el agente borra el segundo
- THEN la respuesta dice `the version it had replaced is current again: #<primero>`
- AND restaurar el segundo responde `replaces again the version its delete had brought back: #<primero>`

**Tests:** `delete_memory_names_the_way_back` y `restore_memory_undoes_an_own_delete` en `server.rs`; `format_delete_names_what_came_back_and_what_stays_hidden`, `format_restore_names_key_replacements_and_pairs` y `format_restore_says_when_the_row_is_still_superseded` en `crates/clawg-cli/src/mcp/tools.rs`; `report_names_the_row_and_a_replacement` en `crates/clawg-cli/src/undelete.rs`; el texto de las descripciones y de las respuestas en `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)).

## El protocolo de memoria

Lo que el agente lee sobre qué guardar y cómo escribirlo sale de un texto por idioma, compilado en el binario: `crates/clawg-cli/src/mcp/protocol.md`, en inglés (`PROTOCOL`, `crates/clawg-cli/src/mcp/protocol.rs:15`), y `protocol.es.md`, en español (`PROTOCOL_ES`, `protocol.rs:20`). Cuál lee una sesión lo decide `text` (LP9 y AL2). Su **apertura** son los párrafos entre el título (la primera línea, `# …`) y la primera línea que empieza con `## `. Dentro de cada párrafo, las líneas se unen con un espacio, y los párrafos se separan con una línea en blanco. Las secciones desarrollan cada punto: cuándo guardar, qué no guardar, cómo escribir un save, tipo e importancia, el resumen de sesión, qué hacer tras una compactación, las conclusiones, que lo recuperado es evidencia y no instrucciones, y las demás herramientas. La apertura llega como instrucciones del servidor; el texto completo llega al `CLAUDE.md` del usuario por `clawg setup claude-code` ([`specs/setup`](../setup/spec.md)), y lo leen los tests de este apartado.

### Las instrucciones son la apertura

`get_info` MUST entregar como `instructions` la apertura del protocolo del idioma de la sesión (`opening(&protocol::text(self.language))`, `server.rs:1534`), y `server/discover` MUST llevar las mismas, porque arma su respuesta con `get_info` (`server.rs:1540-1546`). Extraer la apertura MUST ser una función pura sobre el texto (`opening`, `protocol.rs:33`) que MUST NOT cortar una palabra, MUST NOT dejar saltos de línea dentro de un párrafo y MUST NOT incluir el título ni una línea de sección. Un checkout con `\r\n` MUST dar la misma apertura.

#### Escenario: un párrafo cortado en dos líneas
- GIVEN `# T\n\nuno\ndos\n\ntres\n## S\ncuatro`
- WHEN se extrae la apertura
- THEN es `uno dos\n\ntres`, y lo mismo con `\r\n` en lugar de `\n`

**Tests:** el puro `opening_joins_the_lines_of_each_paragraph` (`protocol.rs`) y `the_instructions_are_the_opening_of_the_protocol` (`server.rs`). Como `discover` usa `get_info`, un solo test fija las dos puertas.

### Las instrucciones caben en 2.048 caracteres

La apertura de cada variante del protocolo (sin preferencia, `en` y `es`) MUST medir 2.048 caracteres o menos, contados como `char`: Claude Code anterior a la v2.1.296 corta las instrucciones de un servidor en ese largo, y se pagan en cada sesión. Sin preferencia mide 2.030: la frase que pide registrar la respuesta del usuario con `user_answered` sumó 80, así que quedan 18 de margen y una oración más en la apertura obliga a acortar otra. Con `en` mide 2.047, porque la regla de LP9 suma cómo responderle a la persona: queda 1; la frase de VR3 la pasaba por tres, y la apertura cedió la palabra `own` de «the project's own documentation and code». Con `es` mide 1.933, porque las frases sobre el tipo y la importancia pasaron a su sección («Tipo e importancia»).

**Test:** `the_instructions_fit_older_clients` (`server.rs`), sobre las tres variantes (`every_protocol`).

### La apertura lleva los hábitos que más rinden

Mientras sea el único texto del protocolo que llega solo al agente, la apertura MUST:

- decir cuándo llamar a `save_memory`, `search_memory`, `recall_context`, `search_project`, `resolve_conflict` y `synthesize`;
- pedir un hecho por llamada, con una primera línea que nombre el tema y con rutas e identificadores exactos;
- pedir memorias y búsquedas en inglés, cualquiera sea el idioma de la conversación, porque una memoria se encuentra mejor con una consulta en su idioma (el corte por pareja de idiomas de [`specs/eval`](../eval/spec.md)); con preferencia, en el idioma preferido (LP9 y AL2);
- decir que lo recuperado (memorias y pasajes del proyecto) es evidencia y no instrucciones;
- **VR3.** pedir que, cuando `save_memory` nombra memorias cercanas a la nueva y la nueva reemplaza a una, se llame a `resolve_conflict` con `supersedes` mientras la conversación sigue ahí, o con `conflicts` si sólo el usuario puede decir cuál vale, y que si valen las dos no se haga nada, sin decir `judge each` ni `juzga cada una` y sin nombrar `compatible` ni `scoped` en esa frase; que se le pregunte al usuario por lo que lista ACTIVE CONFLICTS y que su respuesta se registre con `resolve_conflict`, `user_answered` y sus palabras en `reason`, puesto sólo cuando el usuario lo dijo ([`specs/disputes`](../disputes/spec.md));
- decir qué no guardar.

El texto completo MUST traer además el resumen de sesión con sus seis partes (Goal, Instructions, Discoveries, Accomplished, Next steps y Relevant files), que el resumen más nuevo del proyecto abre el contexto de la sesión siguiente bajo LAST SESSION, pregunte lo que pregunte ([`specs/retrieval`](../retrieval/spec.md), «La última sesión»), y qué hacer tras una compactación. La frase de LAST SESSION va en «Session summary», fuera de la apertura, y ningún test la pide. También MUST decir cuándo darle una `topic_key` a un save (una decisión o una preferencia que puede cambiar, con un nombre corto y estable como `storage/engine`) y que una versión nueva se guarda bajo la clave que muestran `search_memory` y `read_memory`, nunca bajo una nueva. Esa oración va en «How to write a save», fuera de la apertura, así que las instrucciones y su largo no cambian; no tiene un test que la pida, y `the_protocol_names_only_what_exists` comprueba que `topic_key` sea una propiedad del schema.

La apertura de `protocol.es.md` MUST llevar los mismos hábitos, en español (AL2).

**Tests:** `the_instructions_carry_the_saving_habits`, `the_spanish_instructions_carry_the_saving_habits` y `the_protocol_describes_the_session_summary` (`server.rs`); además, `get_info_advertises_tools_and_the_profiles_server_name` e `instructions_say_when_to_search_the_project` (`server.rs`) y `a_discover_probe_is_answered_with_what_the_server_offers` (`handshake.rs`) piden `save_memory`, `synthesize` y `search_project` en las instrucciones.

### El protocolo nombra sólo lo que existe

- Cada herramienta registrada en `tool_router` MUST aparecer entre comillas invertidas en el protocolo.
- Cada texto entre comillas invertidas que sea un nombre en minúsculas, con o sin `_` (`[a-z]+(_[a-z]+)*`), MUST ser una herramienta registrada, una propiedad del `inputSchema` de alguna herramienta o un tipo de memoria. Los ejemplos y las marcas (`` `Fixed lexical bonus ignoring paths` ``, `` `<private>` ``, `` `piece=K/N` ``, `` `by <agent>` ``) no cuentan.
- Cada tipo de memoria MUST aparecer en el protocolo.

Vale para cada variante: sin preferencia, `en` y `es`.

Juntar los textos entre comillas invertidas MUST ser una función pura (`code_spans` e `is_name`, en los tests de `server.rs`).

#### Escenario: una herramienta renombrada
- GIVEN el protocolo con `` `save_memories` ``
- THEN el test falla y nombra `save_memories`

**Tests:** el puro `backticked_names_reads_each_code_span` y `the_protocol_names_only_what_exists` (`server.rs`).

## PM6. El protocolo dice qué preferencias entran solas

`protocol.md`, en «Type and importance», y `protocol.es.md`, en «Tipo e importancia», MUST decir que una `user_preference` con 8 o más, guardada con `user_stated`, se imprime arriba del `recall_context` de su proyecto, o de todos si se guardó con `all_projects`, para cualquier agente que la lea, sólo mientras siga en tier 1 (la [preferencia elegible](../retrieval/spec.md#definiciones)), y que el presupuesto puede dejar afuera una elegible, que el bloque nombra entonces por id ([«El prompt cumple el presupuesto»](../retrieval/spec.md#el-prompt-cumple-el-presupuesto)); y que una búsqueda marca la que no declaró la persona ([`specs/retrieval`](../retrieval/spec.md), PM1 a PM3 y PM5). La línea de `user_preference` de la lista de tipos MUST decir cuándo usar `all_projects`. La apertura de los dos MUST NOT cambiar por esto.

**Tests:** `the_protocol_says_which_preferences_open_every_session` (`crates/clawg-cli/src/mcp/protocol.rs`), en los dos idiomas; `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one`; `the_instructions_fit_older_clients` y los dos contratos, que fijan las `instructions`.

## El schema de `memory_type` declara los tipos

En el `inputSchema` de `save_memory` y de `update_memory`, la propiedad `memory_type` MUST declarar `enum` con los siete tipos de `MemoryType::ALL`, en su orden, más `null`, para que el `default` declarado (`null`) siga siendo válido (`memory_type_enum`, `server.rs:148-152`, aplicado con `#[schemars(transform = …)]` en `server.rs:165` y `server.rs:324`). Sus descripciones MUST NOT repetir la lista: dicen que es uno de los valores listados y que cualquier otro se rechaza. La de `save_memory` MUST decir además que una `architectural_decision` es una elección de diseño y su motivo, y que lo hecho, hallado o medido es un `semantic_fact`, aunque siga a una decisión: el tipo decide qué par se le pregunta al usuario ([`specs/disputes`](../disputes/spec.md)).

Al deserializar, el argumento MUST seguir siendo una cadena libre (`Option<String>`): un valor desconocido llega al daemon y vuelve como `not saved: unknown memory_type …` con los siete tipos ([`specs/ingest`](../ingest/spec.md), «El tipo de memoria se valida al guardar»), no como un error de protocolo que el modelo no puede leer. El `enum` le muestra los tipos al modelo antes de llamar; la validación de verdad es la del daemon.

#### Escenario: un tipo inventado
- GIVEN un cliente que no valida el `enum`
- WHEN el agente llama `save_memory` con `memory_type: "bug_fix"`
- THEN la respuesta es un error que empieza con `not saved: unknown memory_type "bug_fix"` y nombra los siete tipos, y no se guarda nada

**Tests:** `the_save_memory_schema_names_the_accepted_memory_types` (`server.rs`), que compara el `enum` de las dos herramientas con `MemoryType::ALL`; `the_save_memory_schema_says_what_a_decision_is` (`server.rs`); `save_memory_says_an_unknown_type_was_not_saved` (`server.rs`). El schema lo fija también `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)).

## La descripción de `save_memory` pide título y "dónde"

La descripción de `save_memory` (`server.rs:1308-1316`) MUST pedir una primera línea que nombre el tema y, después, qué, por qué y dónde, con las rutas e identificadores exactos: las primeras palabras se embeben con cada pieza de un save largo, y un identificador que nombra la consulta es lo que marca un hit como `exact`. MUST conservar "one fact" y `read_memory`.

**Tests:** `save_memory_asks_for_a_title_and_where` y `save_memory_asks_for_one_fact_per_call` (`server.rs`).

## PM4. `save_memory` con `all_projects`

`save_memory` MUST tomar `all_projects`, un booleano opcional (`SaveMemoryArgs.all_projects`, `crates/clawg-cli/src/mcp/server.rs:186`). Con `true`, el save MUST llegar al daemon sin `project_id` (`server.rs:1088`). Con `project` o `project_path`, MUST negarse sin llamar al daemon, con una frase que nombra los argumentos, en el idioma de la sesión (`server.rs:1064-1067`); `new_project` ya se niega sin `project`. Su descripción MUST decir que guarda la memoria sin proyecto, así la lee cada proyecto, y que es para lo que vale en todo proyecto, como una preferencia que la persona tiene en todo lo que hace (en español, `crates/clawg-cli/src/mcp/tools_es.rs:179`). Ausente o `false`, el save MUST nombrar su proyecto con `project` o `project_path`, y sin ninguno de los tres se niega ([BI4](#bi4-un-save-nombra-su-proyecto)).

#### Escenario: una preferencia general
- GIVEN un puente en el proyecto P
- WHEN el agente llama a `save_memory` con `memory_type: "user_preference"` y `all_projects: true`
- THEN la fila guardada tiene `project_id` NULL

#### Escenario: dos lugares
- WHEN el agente llama a `save_memory` con `all_projects: true` y `project`, o con `all_projects: true` y `project_path`
- THEN la respuesta es un error que nombra los dos argumentos, y no se guarda nada

**Tests:** `a_save_for_every_project_has_no_project` y `all_projects_goes_without_a_project` (`server.rs`); el schema lo fijan `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`.

## Mover una memoria de proyecto

`update_memory` MUST tomar `to_project_path` (una ruta cuyo proyecto es el destino, registrado antes como toda ruta que nombra una herramienta, y que tiene que ser un directorio) o `clear_project` (sin proyecto), no los dos; ausentes, el proyecto no cambia. El nombre no es `project_path` porque en las otras herramientas su ausencia es el proyecto del puente. La descripción MUST decir que la memoria se mueve con su save y su historia, y no a donde el mismo texto o clave están vivos; `protocol.md` MUST decir cuándo usarla. Tras un movimiento, la respuesta MUST nombrar el proyecto en que quedó la fila (o que está sin proyecto, donde la encuentran todos) y las filas que se movieron con ella (`format_update`).

**Test:** `update_memory_moves_a_save_to_the_project_of_a_path`, `update_memory_clears_the_project` y `update_memory_refuses_both_destinations_and_a_path_that_is_not_a_directory` (`server.rs`); `format_update_names_where_the_row_ended` (`tools.rs`); `the_agent_contract_matches_its_file`.

## Un proyecto por su id o su nombre

`save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize` MUST tomar `project`, un id o un nombre, y `update_memory` MUST tomar `to_project`; cada uno se excluye con la ruta de la misma herramienta (`project_path`, `to_project_path` y `clear_project`), y la herramienta MUST negarse sin llamar al daemon cuando llegan los dos. El puente los resuelve con `POST /v1/projects/resolve` en cada llamada ([`specs/project`](../project/spec.md), PL32): sólo `save_memory` crea, antes de que el save entre al outbox, y acepta `new_project: true` para crear aparte de los proyectos con un nombre parecido (sin `project` se niega). Un rechazo MUST responder, sin guardar ni buscar, los candidatos con su id y su nombre, los borrados como tales, y qué preguntarle a la persona (`format_refusal`, `crates/clawg-cli/src/mcp/tools.rs`). `search_project` con `project` busca en su carpeta más vieja con `index_docs`, o en la más vieja, y un proyecto sin carpeta MUST responder que no tiene archivos en esta máquina (`no_folder`). Sin el argumento, cada herramienta lee las carpetas abiertas, como dicen BI4 a BI8.

`answer_suggestion` (`other`, `answer` merge o apart, `reason`, `user_answered`, y `project`, por defecto el de la carpeta abierta ([BI7](#bi7-las-herramientas-de-un-proyecto)), leído por su id para seguir una unión) MUST llamar a `POST /v1/projects/suggestions/answer` (PL37) y decir qué hizo: con una unión, su número y `clawg projects merge --undo N` (`format_answered`). `recall_context` MUST terminar con una línea por sugerencia pendiente del proyecto, que nombra el otro, el motivo y cómo responderla (`suggestion_lines`). `protocol.md` y `protocol.es.md` MUST decir, en su sección `## Projects` / `## Proyectos`, fuera de la apertura, cuándo usar `project` y que una sugerencia se le pregunta a la persona antes de `answer_suggestion`.

**Tests:** `saving_to_a_project_by_name_creates_it`, `a_similar_name_is_answered_with_its_candidates`, `project_and_project_path_are_refused_together`, `searching_the_docs_of_a_project_without_a_folder_says_so` y `answer_suggestion_says_what_it_did` (`server.rs`); `a_suggestion_is_shown_until_answered`, en los dos idiomas (`tools.rs`); `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`.

## BI1. Las raíces se piden en cada llamada

Ante cada llamada de `save_memory`, `search_memory`, `recall_context`, `search_project`, `synthesize`, `read_memory`, `answer_suggestion` o `memory_status`, también la que nombra su proyecto, el puente MUST mandar `roots/list` a un cliente con raíces, dentro de esa llamada, y esperar la respuesta como mucho `ROOTS_TIMEOUT` (2 s; `roots::asked`, `crates/clawg-cli/src/mcp/roots.rs`, y `ClawgMcp::asking`, `server.rs`). MUST NOT reusar la respuesta en otra llamada, ni esperar `notifications/roots/list_changed`. A un cliente sin raíces MUST NOT mandarle `roots/list`. Un error, un plazo vencido o una respuesta sin carpetas MUST contar como un cliente sin raíces; un error y un plazo vencido dejan un warn en el log del puente. `list_memories`, `delete_memory`, `restore_memory`, `update_memory` y `resolve_conflict` MUST NOT pedir raíces.

#### Escenario: las raíces cambian entre dos llamadas
- GIVEN una sesión `initialize` que declara `roots`, con `/work/a` abierta
- WHEN busca sin proyecto, el cliente abre además `/work/b` sin mandar `list_changed`, y guarda sin proyecto
- THEN la búsqueda lee el proyecto de `/work/a`, el save se rechaza nombrando `/work/a` y `/work/b`, y el cliente recibió dos `roots/list`

#### Escenario: un cliente sin raíces
- GIVEN una sesión que no declara `roots` y un puente que arrancó en `/proj-test`
- WHEN busca sin proyecto
- THEN el cliente no recibe ningún `roots/list` y la búsqueda lee el proyecto de `/proj-test`

**Tests:** `the_roots_are_asked_on_each_call`, `a_client_without_roots_is_not_asked`, `a_roots_error_counts_as_no_roots` (`server.rs`). Con Cursor 3.29.9 en Windows, cada ventana contesta `roots/list` a tiempo con sus propias carpetas, y no con las de las otras ventanas ([verify](../../changes/archive/2026-11-07-bridge-project-identity/verify.md#con-cursor-2026-11-06)).

## BI2. Una raíz es una carpeta

El `uri` de una raíz MUST leerse así: un `file:` con host vacío o `localhost` es el camino que sigue, con los `%XX` decodificados como UTF-8, y en Windows sin la barra que precede a la unidad; un `file:` con otro host MUST leerse en Windows como la carpeta compartida que nombra (`file://server/share` es `\\server\share`) e ignorarse en los demás sistemas; uno que no decodifica o un esquema que no es `file:` MUST ignorarse; cualquier otro texto es un camino si es absoluto en el sistema, en Windows sin una barra que preceda a la unidad (`/C:/x` es `C:\x`, como Cursor escribe las raíces de un hook), y si no MUST ignorarse. En Windows cada `/` MUST quedar `\`. La función MUST ser pura, también en el sistema que lee, que se le pasa, así las formas de cada uno se prueban en cualquiera (`folder_of`, `roots.rs`). Cada carpeta MUST resolverse con `ProjectIdentity::resolve`, como un `project_path`.

#### Escenario: Cursor en Windows
- GIVEN una carpeta `C:\work\repo` que existe
- WHEN una raíz llega como `c:\work\repo` y otra como `file:///C:/work/repo`
- THEN las dos dan el mismo proyecto

**Tests:** `a_root_is_read_as_a_uri_or_a_path`, `a_root_is_read_as_each_system_reads_it`, `a_root_as_a_path_and_as_a_uri_is_one_project` (`roots.rs`; el segundo con una carpeta temporal, que en Windows escribe la ruta con la unidad en minúscula y la URI con barras).

## BI3. Las carpetas abiertas

Las carpetas abiertas de una llamada MUST ser las raíces del cliente, resueltas, en su orden y sin repetir una carpeta. Dos carpetas de la misma raíz MUST quedar las dos, porque una subcarpeta declarada lleva a otro proyecto ([`specs/project`](../project/spec.md) PL47). Si no queda ninguna, el proyecto del directorio donde arrancó el puente; si tampoco, ninguna. **El directorio personal MUST NOT ser un proyecto:** una carpeta que es el directorio personal, o cuya raíz lo es, MUST quitarse, venga del cliente o del directorio del puente: el directorio personal puede estar dentro de un checkout, cuya raíz es otra. La decisión MUST ser pura, con el resolvedor y el directorio personal como argumentos (`open_folders`, `roots.rs`; el directorio personal canónico, `canonical_home`, `server.rs`).

Cuántos proyectos hay abiertos MUST contarse con el proyecto que el daemon le da a cada carpeta al registrarla. De cada proyecto queda su primera carpeta (`open_projects`, `server.rs`): dos carpetas del mismo repositorio son un proyecto, salvo que una esté en una subcarpeta declarada. Así cuentan la búsqueda (BI5), las herramientas de un proyecto (BI7) y `memory_status` (BI8). El rechazo de un save que no nombra su proyecto (BI4) no llama al daemon, así que lista cada carpeta.

#### Escenario: el puente arranca en el directorio personal
- GIVEN un puente que arrancó en el directorio personal y un cliente sin raíces
- WHEN busca sin proyecto
- THEN la búsqueda lee todos los proyectos y el daemon no registra ninguno

#### Escenario: dos subproyectos de un monorepo
- GIVEN un repositorio `/work/mono` con `apps/web` declarada como el proyecto `web`, y un cliente con `/work/mono/apps/web` y `/work/mono/apps/webapp` abiertas
- WHEN busca sin proyecto y llama a `recall_context` sin proyecto
- THEN la búsqueda lee `web` y el proyecto del repositorio, cada hit con su `project=`, y `recall_context` se niega nombrando las dos carpetas
- AND con `/work/mono` y `/work/mono/apps/webapp` abiertas, que llevan al mismo proyecto, ningún hit muestra `project=` y `recall_context` responde

**Tests:** `the_open_folders_are_the_roots_else_the_directory`, `the_home_directory_is_never_a_project` (`roots.rs`); `a_bridge_started_in_the_home_directory_registers_no_project`, `the_open_subfolders_of_one_repository_are_each_its_own_project` y `two_open_folders_of_one_project_count_once` (`server.rs`).

## BI4. Un save nombra su proyecto

`save_memory` sin `project`, `project_path` ni `all_projects: true` MUST negarse sin llamar al daemon (`SaveMemoryArgs::names_its_project`, `server.rs`; `no_project_named`, `crates/clawg-cli/src/mcp/tools.rs`). La frase MUST estar en el idioma de la sesión, nombrar los tres argumentos, y listar cada carpeta abierta como su camino resuelto, y no como su raíz, para pasarla en `project_path`: una subcarpeta declarada lleva a un proyecto al que su raíz no lleva ([`specs/project`](../project/spec.md) PL47). Sin carpetas abiertas, MUST decir que se pase la carpeta del proyecto. Con uno de los tres, el save MUST hacer lo de antes.

#### Escenario: dos carpetas abiertas
- GIVEN un cliente con `/work/a` y `/work/b` abiertas
- WHEN guarda sin proyecto
- THEN la respuesta es un error que empieza por `not saved:`, nombra `/work/a`, `/work/b`, `project_path`, `project` y `all_projects`, y el daemon no tiene ninguna fila

**Tests:** `a_save_that_names_no_project_is_refused_with_the_open_folders`, `a_save_without_roots_offers_the_directory`, `a_save_with_no_open_folder_asks_for_the_project_folder` (`server.rs`), `a_save_refusal_offers_each_open_folder_rather_than_its_root` (`tools.rs`), y el save de `the_roots_are_asked_on_each_call`.

## BI5. `search_memory` lee las carpetas abiertas

Una llamada sin proyecto ni `all_projects` MUST buscar: sin carpetas abiertas, en todos los proyectos; con una, en su proyecto, registrado como el del directorio del puente; con varias, en el grupo de sus proyectos ([`specs/retrieval`](../retrieval/spec.md), BI6), cada uno registrado (dos carpetas del mismo proyecto cuentan una vez). Cuando una llamada sin `project` ni `project_path` tiene abiertas carpetas de varios proyectos (BI3; un grupo, o todos con `all_projects`), cada hit cuya fila tiene proyecto MUST mostrar ` project=<id>` después del puntaje y de la clave, con un `[` o un `]` del id como paréntesis y un control o un salto de línea como espacio (`display_project`, `tools.rs`); un hit sin proyecto MUST NOT mostrarlo. Con un proyecto o ninguno, o con `project` o `project_path`, cada hit MUST mostrarse como antes. `topic=K` MUST mostrarse en una fila de cualquier proyecto de las carpetas abiertas (`revised_from` contra una lista: con un proyecto, la regla de antes), y con `all_projects` la lista es la de las carpetas abiertas. Una fila sin proyecto MUST NOT mostrar ` topic=K` desde ningún puente, con carpetas abiertas o sin ellas, porque un save nombra su proyecto y sólo uno con `all_projects` la revisa: una `user_preference` sin proyecto MUST mostrar ` topic=K all_projects` ([PM8](#pm8-una-preferencia-sin-proyecto-se-reemplaza-con-all_projects)), y cualquier otra, ninguna clave; un hit sin `project_id`, de un daemon anterior al campo, MUST mostrarla como antes. El puente MUST mandar el grupo en `project_ids` y, en `project_id`, su primer proyecto: un daemon anterior a los grupos ignora `project_ids` y busca en ese, más estrecho que el grupo y nunca en todos (`SearchArgs.project_ids` y `search_request`, `crates/clawg-cli/src/client.rs`).

#### Escenario: un workspace de dos repositorios
- GIVEN una memoria en el proyecto de `/work/a`, otra en el de `/work/b` y otra en el de `/work/c`, y un cliente con `/work/a` y `/work/b` abiertas
- WHEN busca sin proyecto algo que las tres responden
- THEN vuelven las dos primeras, cada una con su `project=`, y no la tercera

**Tests:** `a_search_with_one_root_reads_its_project`, `a_search_with_several_roots_reads_all_of_them_and_names_each`, `a_search_without_roots_reads_the_directory` (`server.rs`); `format_hits_names_the_project_of_each_hit_of_a_group`, `a_project_named_by_a_hit_keeps_the_hit_on_its_line`, `the_key_shows_on_a_row_of_any_open_folder`, `a_hit_without_a_project_shows_how_a_save_revises_it`, `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`); `a_search_sends_a_group_only_when_it_has_one`, `a_group_search_names_its_first_project_for_an_older_daemon` (`client.rs`).

## BI7. Las herramientas de un proyecto

Cuando las carpetas abiertas llevan a varios proyectos (BI3), una llamada sin proyecto de `recall_context`, de `synthesize` sin `all_projects`, de `search_project` y de `answer_suggestion` MUST negarse sin leer memorias, en el idioma de la sesión. La respuesta MUST listar la primera carpeta de cada proyecto como su camino resuelto, y no como su raíz, para `project_path` (o `project` en `answer_suggestion`, que no toma camino), y decir que `search_memory` busca en todas (`several_open` y `unnamed_with_several`, `server.rs`; `several_folders`, `tools.rs`). Con un proyecto MUST hacer lo de antes con el directorio del puente; sin ninguno, lo de antes sin proyecto.

#### Escenario: `recall_context` en un workspace de dos repositorios
- GIVEN un cliente con `/work/a` y `/work/b` abiertas
- WHEN llama a `recall_context` sin proyecto
- THEN la respuesta es un error que nombra `/work/a`, `/work/b`, `project_path` y `search_memory`

**Tests:** `a_one_project_tool_asks_which_folder_when_several_are_open` (`server.rs`, las cuatro herramientas); `the_open_subfolders_of_one_repository_are_each_its_own_project` y `two_open_folders_of_one_project_count_once` (`server.rs`, con `recall_context`).

## BI8. `memory_status` y `read_memory`

Los pares escalados que cuenta `memory_status` MUST ser los del proyecto abierto cuando las carpetas abiertas llevan a uno, y los de todos los proyectos cuando llevan a varios o no hay ninguna (`one_project`, `server.rs`; BI3). Si una carpeta no se puede registrar, MUST omitir la línea de los conflictos, y no fallar. `read_memory` MUST mostrar `topic=K` con la regla de BI5 contra los proyectos de las carpetas abiertas (`known_ids`, `server.rs`).

**Tests:** `memory_status_counts_every_project_with_several_folders`, `read_memory_shows_the_key_of_a_row_in_any_open_folder` (`server.rs`).

## BI9. Los documentos de las carpetas abiertas

Una carpeta abierta MUST registrarse con `index_docs` si `auto_indexable` la acepta (tiene `.git` o un manifiesto y no es el directorio personal), como el directorio del puente (`indexable_folders`, `server.rs`; `indexes_docs`, `tools.rs`; [`specs/project`](../project/spec.md), R6). Un camino que nombra el modelo y no es una carpeta abierta MUST seguir registrándose sin `index_docs`. El 403 de `search_project` (`not_accepted`, `crates/clawg-cli/src/daemon/api.rs`), `clawg projects add` en `docs/reference/commands.md` y `docs/reference/daemon.md` MUST decir que se lee sola una carpeta que el cliente MCP tiene abierta o en la que arrancó un puente ([`specs/project-docs`](../project-docs/spec.md), «Búsqueda»).

**Tests:** `a_root_the_client_declares_is_indexed_like_the_directory` (`server.rs`), `the_refusal_names_what_is_read_without_the_user_adding_it` (`api.rs`), `only_an_open_folder_is_indexed` (`tools.rs`); `a_save_elsewhere_registers_the_root_without_reading_it` y `a_bridge_in_a_folder_that_is_not_a_project_indexes_nothing`.

## BI10. Lo que ve el modelo

- La descripción de `project_path` de `save_memory` MUST decir que no tiene valor por defecto y que un save pasa `project_path`, `project` o `all_projects`; las de `project_path` de las herramientas que leen, que sin él leen las carpetas que el cliente tiene abiertas, o el directorio del puente.
- La de `search_memory` MUST decir que con varias carpetas busca en todas y que cada hit nombra su proyecto con `project=`, que es lo que `project` acepta.
- La apertura de `protocol.md` y de `protocol.es.md`, lo que lee un cliente anterior, MUST pedir que cada save nombre su proyecto con `project_path`, `project` o `all_projects`. Su sección de proyectos MUST decir cuándo pasar `project_path` y que, con varias carpetas abiertas, `recall_context`, `search_project`, `synthesize` y `answer_suggestion` piden en cuál se trabaja; «How to write a save» y «Cómo escribir un save», que un hit de una búsqueda en varios proyectos muestra su clave junto a su `project=`, y que un save con ese `project` la revisa. Cada variante de la apertura MUST seguir cabiendo en 2048 caracteres.
- `project=` es un marcador («Definiciones»).
- `agent_contract.json` y `agent_contract.es.json` MUST coincidir con lo que lista el servidor, bendecidos con `CLAWG_BLESS=1`.

**Tests:** `the_agent_contract_matches_its_file`, `the_spanish_agent_contract_matches_its_file`, los del largo de la apertura, `the_protocol_names_only_what_exists` y `the_protocol_says_where_a_call_without_a_project_goes` (`protocol.rs`, en los dos idiomas).

## LP8. La línea del otro idioma

Con preferencia, `search_memory` y `recall_context` con consulta MUST mandar `lang` y MUST agregar, al final de su respuesta, una línea que dice cuántas memorias del alcance están en el otro idioma y que, si lo mostrado no responde, se busque otra vez con la consulta en ese idioma, cuando `other_language.memories` es mayor que cero y la búsqueda mostró menos de `FEW_SHOWN` (3) memorias, ninguna incluida (`other_language_line`, `crates/clawg-cli/src/mcp/tools.rs:123-139`). En cualquier otro caso MUST NOT agregarla. `FEW_SHOWN` no está medido.

#### Escenario: una búsqueda pobre
- GIVEN la preferencia `es` y 12 memorias en inglés en el alcance
- WHEN `search_memory` muestra una memoria, o dice `no relevant memories for this query`
- THEN la respuesta termina con la línea, que nombra 12 y el inglés

#### Escenario: cuándo no sale
- WHEN la búsqueda muestra tres memorias o más, el conteo es cero, no hay preferencia, o el daemon no manda `other_language`
- THEN la respuesta es la de antes

**Test:** `a_poor_search_names_the_memories_in_the_other_language`, `a_search_that_shows_enough_says_nothing_of_the_other_language`, `no_memories_in_the_other_language_means_no_line`, `without_a_preference_no_line`, `a_daemon_without_the_count_means_no_line` y `recall_context_names_the_other_language_after_a_poor_context` (`tools.rs`); `search_memory_sends_the_bridges_language` (`server.rs`).

## LP9. Las instrucciones y el protocolo con preferencia

`crates/clawg-cli/src/mcp/protocol.rs` es dueño del texto (`PROTOCOL`, `PROTOCOL_ES`, `opening`, `text`), para el puente y para `setup`. Con la preferencia `en`, `text` (`protocol.rs:58`) MUST dar `protocol.md` con dos cambios: la frase de la apertura que pide guardar y buscar en inglés pide hacerlo en el idioma preferido y suma que se le responda a la persona en ese idioma salvo que escriba en otro; y la línea de «How to write a save» pide guardar y buscar en el idioma preferido y, cuando una búsqueda diga que quedan memorias en el otro idioma, buscar otra vez con la consulta en él. Cada una de las dos reglas MUST aparecer una sola vez en `protocol.md`, aunque cruce saltos de línea. Con `es`, `text` MUST dar `protocol.es.md` tal cual (AL2). Sin preferencia, `text` es `protocol.md` tal cual. Las instrucciones del servidor MUST ser la apertura de ese texto y MUST caber en 2.048 caracteres con cada idioma. El protocolo MUST seguir nombrando sólo herramientas, argumentos y tipos que existen. El puente MUST leer la preferencia una vez, al arrancar (`ClawgMcp::with_language`, `crates/clawg-cli/src/mcp/server.rs:506`, desde `crates/clawg-cli/src/mcp/mod.rs:133`).

#### Escenario: inglés
- GIVEN la preferencia `en`
- THEN el protocolo dice `Save memories and search in English:` y `Reply to the user in English unless`, y cuando quedan memorias en español pide buscar otra vez con la consulta en español; nada más cambia

#### Escenario: español
- GIVEN la preferencia `es`
- THEN las instrucciones son la apertura de `protocol.es.md`: dicen `Guarda las memorias y busca en español` y `Responde a la persona en español salvo que escriba en otro idioma`, no dicen `English` y miden 2.048 caracteres o menos

#### Escenario: la sesión no cambia sola
- GIVEN un puente arrancado con la preferencia `en`
- WHEN el archivo pasa a decir `es`
- THEN sus instrucciones y sus búsquedas siguen en `en`

**Test:** `the_instructions_follow_the_preferred_language`, `the_instructions_fit_older_clients` y `the_protocol_names_only_what_exists` para cada idioma, `without_a_preference_the_instructions_are_the_opening` y `the_bridge_keeps_the_language_it_started_with` (`server.rs`); `the_language_rules_occur_once_in_the_protocol`, `without_a_language_the_protocol_is_the_file` y `a_language_rewrites_both_rules_of_the_english_protocol` (`protocol.rs`). `agent_contract.json` fija el servidor sin preferencia, y `agent_contract.es.json` en español ([`specs/quality`](../quality/spec.md)).

## LP10. La descripción de `search_memory`

La descripción de `search_memory` MUST NOT decir que encuentra memorias escritas en otro idioma, y MUST decir que una memoria en otro idioma que la consulta suele no encontrarse. Vale con preferencia y sin ella.

**Test:** `search_memory_does_not_promise_other_languages` (`server.rs`); `the_agent_contract_matches_its_file`.

## LP11. El idioma en `memory_status`

Con preferencia al arrancar, `memory_status` MUST decir el idioma de la sesión. Cuando `preferences.json` dice otro idioma que el de la sesión (o una preferencia que la sesión no tenía, o ninguna) MUST decir el actual y que se aplica en una sesión nueva. Sin preferencia al arrancar ni ahora, MUST NOT agregar nada (`format_language`, `tools.rs:910`).

**Test:** `memory_status_names_the_sessions_language`, `memory_status_says_when_the_preference_changed` y `memory_status_without_a_preference_says_nothing_of_it` (`tools.rs`); `memory_status_names_the_language_of_the_session` (`server.rs`).

## AL1. Sin preferencia o con `en`, lo de antes

Sin preferencia o con `en`, las instrucciones (salvo las dos reglas de LP9 con `en`), `tools/list` y cada respuesta del puente MUST ser idénticas, byte a byte, a las de antes de que el puente hablara español: cada frase es un `match` sobre `Lang` cuya rama `Lang::En` es el texto de antes. Lo mismo vale para las notas del contexto ([`specs/retrieval`](../retrieval/spec.md), AL5) y los hooks ([`specs/hooks`](../hooks/spec.md), AL6).

**Test:** `the_agent_contract_matches_its_file` ([`specs/quality`](../quality/spec.md)), que pasó sin cambios en `agent_contract.json`; los tests anteriores de `tools.rs`, `server.rs`, `prompt_builder.rs` y `hook.rs` pasan con `Lang::En` sin cambiar el texto que esperan.

## AL2. El protocolo en español

Con `es`, `protocol::text` MUST ser `protocol.es.md` (`PROTOCOL_ES`, `protocol.rs:20`), escrito para el español y no traducido frase a frase, y las instrucciones del servidor su apertura, de 2.048 caracteres o menos. `protocol.es.md` MUST tener tantas secciones `##` como `protocol.md` (las nueve, en el mismo orden: «Cuándo guardar» es «When to save», y así hasta «Las otras herramientas») y el mismo conjunto de nombres y marcadores entre comillas invertidas: los textos sin espacio, o con `<` (`names_and_markers`, `protocol.rs:168`). Los ejemplos de un save, con espacios, se escriben en cada idioma. Los marcadores en prosa (`ACTIVE CONFLICTS`, `LAST SESSION`, los veredictos, `inferred`) MUST estar en los dos o en ninguno, y cada nombre entre comillas MUST ser una herramienta registrada, un argumento suyo o un tipo de memoria («El protocolo nombra sólo lo que existe»). Su apertura MUST pedir guardar y buscar en español, responder a la persona en español salvo que escriba en otro idioma, y nombrar `save_memory`, `search_memory`, `recall_context`, `search_project`, `resolve_conflict` y `synthesize`; y «Cómo escribir un save», buscar otra vez con la consulta en inglés cuando una búsqueda diga que quedan memorias en inglés.

Un protocolo por idioma, y no frases reescritas sobre el inglés como hace LP9 con `en`: el español no se escribe palabra por palabra como el inglés, y la apertura tiene que caber en 2.048 caracteres moviendo frases entre la apertura y las secciones. Lo que cuesta mantener dos textos lo vigilan los tests de paridad.

#### Escenario: setup escribe el protocolo en español
- GIVEN la preferencia `es`
- WHEN corre `clawg setup claude-code`
- THEN el bloque entre los marcadores es `protocol.es.md`, y `doctor` da `protocol_block` `ok` ([`specs/setup`](../setup/spec.md), [`specs/doctor`](../doctor/spec.md))

**Test:** `spanish_is_the_spanish_protocol` y `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one` (`protocol.rs`); `the_instructions_fit_older_clients` y `the_protocol_names_only_what_exists` sobre cada variante, `the_spanish_instructions_carry_the_saving_habits` y `the_instructions_follow_the_preferred_language` (`server.rs`); `the_block_follows_the_preferred_language` (`setup.rs`).

## AL3. Las herramientas en español

Con `es`, la descripción de cada herramienta y de cada argumento MUST ser la de la tabla `DESCRIPTIONS` (`crates/clawg-cli/src/mcp/tools_es.rs:9`: la herramienta, el argumento o `None` para la herramienta, y el texto), que `ClawgMcp::with_language` pone en el router una sola vez, al fijar el idioma (`server.rs:506`; `describe`, `server.rs:112`, reemplaza la descripción de la herramienta y la de cada propiedad de su `inputSchema`): `tools/list` y los tests leen la misma lista, y MUST coincidir con `agent_contract.es.json` ([`specs/quality`](../quality/spec.md)). La tabla MUST tener una entrada por herramienta registrada y por argumento suyo, y ninguna más; cada descripción en español MUST nombrar entre comillas invertidas los mismos textos que la inglesa. Los nombres, los tipos, los `default` y los `enum` de los esquemas MUST NOT cambiar. Sin preferencia y con `en` las descripciones son las de los atributos `#[tool]` y los comentarios `///` de cada `*Args`.

Una tabla en lugar de dos routers o de traducir en `list_tools`: el macro de `rmcp` une nombre, descripción y función, así que dos routers duplicarían las doce funciones, y cambiar el router una vez deja un solo camino para el cliente y para los tests.

**Test:** `the_spanish_table_covers_every_tool_and_argument`, `the_spanish_descriptions_name_what_the_english_ones_name`, `only_descriptions_change_with_the_language` y `the_spanish_agent_contract_matches_its_file` (`server.rs`).

## AL4. Las respuestas del puente

Con el idioma de la sesión `es`, la prosa de cada respuesta del puente MUST estar en español: cada `format_*` de `crates/clawg-cli/src/mcp/tools.rs`, `topic_key_warning`, `synthesis_nudge`, `review_nudge`, `other_language_line`, y las frases que compone `server.rs` (lo que responde un save, sus negativas, los rechazos del puente, `AgentSource::describe`, `server.rs:83`). Cada frase es un `match` sobre `Lang` con las dos ramas juntas, y cada función recibe el idioma de la sesión. Los marcadores MUST NOT cambiar. Una frase que el daemon devuelve en un error (el `message` de un 400, un 404 o un 409) y el `Display` de un error tipado MUST ir tal cual dentro de la frase del puente, que los enmarca (`not_saved`, `server.rs:376`: `no se guardó: {causa}`): los leen también los clientes de la API y el CLI, y mezclan códigos con texto.

#### Escenario: una búsqueda sin nada cerca
- GIVEN una sesión en `es` y una búsqueda cuyos hits puntúan 0
- THEN `search_memory` dice `no hay memorias relevantes para esta consulta: 3 buscadas, ninguna lo bastante cerca para mostrarla`

#### Escenario: un hit
- GIVEN una sesión en `es` y un hit de `cursor`, pieza 2 de 3, con la clave `api/port`
- THEN la línea del hit es `1. #4 [memory/semantic_fact/t3 piece=2/3 score=0.91 topic=api/port by cursor] El puerto es 8080`, igual que en inglés, y la nota de lo compartido que la sigue está en español

**Test:** `a_spanish_search_keeps_its_markers`, `the_spanish_answers_of_each_tool`, `the_spanish_synthesis_and_nudges`, `the_spanish_status_docs_and_recall` y los de la línea del otro idioma (`tools.rs`); `a_spanish_session_answers_in_spanish` y `memory_status_names_the_language_of_the_session` (`server.rs`).

## RL5. La API que habla un puente anterior

El segmento `{kind}` de `/v1/memories/{kind}/{id}`, `/save` y `/restore`, y el filtro `kind` de `GET /v1/memories`, MUST aceptar `memory`. Un kind retirado MUST dar un 400 con la explicación, que nombra `search_memory`; cualquier otro valor, un 400 que dice que `memory` es el único. Las respuestas MUST seguir llevando `kind` (`memory` en una fila, también las de `POST /v1/synthesis`; `Memory` en un hit), `GET /v1/stats` MUST seguir llevando `totals.episodic` y `totals.procedural` en 0, y la unión de una respuesta `merge` a una sugerencia, `trajectories` y `skills` en 0, porque un bridge de la versión anterior los exige. Las filas MUST NOT llevar `success_count`, `trigger` ni `source_trajectory_ids`.

**Test:** `a_retired_kind_is_refused_with_where_its_rows_went` y `the_answers_keep_what_a_bridge_of_the_previous_version_requires` (`crates/clawg-cli/tests/api.rs`); `parse_kind_takes_memory_alone` y `a_retired_kind_says_where_its_rows_went` (`crates/clawg-cli/src/daemon/api.rs`).

## RL6. La CLI

`--kind` de `clawg memories list` y `show` MUST quedar fuera de la ayuda, aceptar `memory` y rechazar un kind retirado con un error de uso que lleva la explicación y nombra `clawg memories search`. `memories stats`, los informes de `projects` y `purge --project-delete` MUST NOT nombrar trayectorias ni habilidades.

**Test:** `a_retired_kind_is_a_usage_error_that_says_where_its_rows_went` (`crates/clawg-cli/src/cli.rs`).

## RL7. `kind` fuera de las herramientas MCP

La **explicación** de un kind retirado (`episodic` o `procedural`) es el texto que dice que ese kind ya no existe, que sus filas son ahora filas de `memory` de tipo `semantic_fact` con otro id, y dónde se buscan.

`kind` MUST NOT estar en el esquema de `list_memories`, `delete_memory`, `restore_memory`, `read_memory` ni `update_memory`. Un llamado que igual lo manda MUST aceptar `memory`, y con un kind retirado MUST responder la explicación, en el idioma del bridge y nombrando `search_memory`, sin llamar al daemon. Las descripciones MUST NOT hablar de tablas ni de ids por tabla.

#### Escenario: un kind retirado en `read_memory`
- GIVEN un llamado a `read_memory` con `kind = "procedural"`
- WHEN el bridge lo recibe
- THEN responde la explicación nombrando `search_memory`, y el daemon no recibe ninguna petición

**Test:** `a_retired_kind_is_answered_with_where_its_rows_went` (`crates/clawg-cli/src/mcp/tools.rs`); `a_retired_kind_is_answered_without_calling_the_daemon` (`crates/clawg-cli/src/mcp/server.rs`); `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, bendecidos.
