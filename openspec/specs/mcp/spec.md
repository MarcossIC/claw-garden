# mcp — Especificación

Fuente de verdad del comportamiento actual del puente `clawg mcp`: cómo abre una sesión por stdio, en el ciclo legacy (`initialize`) y en el 2026-07-28 (`_meta` en cada request), y de qué nombre saca el `agent_id` de cada request. Qué filas modifica y lista cada agente está en [`specs/visibility`](../visibility/spec.md), y qué filas de otros agentes lee en [`specs/sharing`](../sharing/spec.md); este dominio sólo decide qué nombre entra como `client` en `resolve_agent` y cómo las herramientas muestran lo compartido (último apartado). Cómo el puente vuelve a encontrar al daemon después de un reinicio está en el apartado «Reconectar con el daemon tras un reinicio». La herramienta `search_project`, que busca en la documentación y el código del proyecto, está en [`specs/project-docs`](../project-docs/spec.md); acá sólo se la cuenta entre las herramientas que lista el puente.

**Origen:**
- [mcp-protocol-upgrade](../../changes/archive/2026-10-13-mcp-protocol-upgrade/proposal.md), archivado el 2026-10-13.
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo "Lo compartido en las herramientas").
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo el conteo y la lista de herramientas).
- [bridge-reconnect](../../changes/archive/2026-10-15-bridge-reconnect/proposal.md), archivado el 2026-10-15 (sólo el apartado de reconexión con el daemon).
- [project-root-guard](../../changes/archive/2026-10-15-project-root-guard/proposal.md), archivado el 2026-10-15 (sólo la colisión en «Registro del proyecto»).

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

**Releer** es sondear el daemon como lo hace `probe_running(base_dir, false)` (`crates/clawg-cli/src/client.rs:968`): leer `daemon.json` y `token` y pedir `/health`. Lo hace `ClientHandle::reresolve` (`crates/clawg-cli/src/mcp/startup.rs:102`). Sin daemon que conteste devuelve `NotRunning` (`client.rs:993`); si el almacén lo tiene otro proceso que no es un daemon, devuelve `StoreHeld`.

**Falla que se resuelve releyendo:** `ClientError::Unauthorized` (el daemon contestó 401), `ClientError::NotRunning` (la conexión no se abrió) o `ClientError::StartTimeout` (el arranque no terminó a tiempo). La decisión es la función pura `on_failure` (`startup.rs:39`): da `Next::Reresolve` para esas tres y `Next::Report` para cualquier otra, entre ellas `StoreHeld`, `Spawn`, `Daemon` y `Http`.

#### Escenario: qué falla se relee
- GIVEN cada variante de `ClientError`
- WHEN se decide qué hacer con ella
- THEN `Unauthorized`, `NotRunning` y `StartTimeout` se resuelven releyendo, y todas las demás se informan

**Test:** `only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved` (`mcp/startup.rs`).

**Releer y repetir una vez.** Cuando una herramienta falla con una falla que se resuelve releyendo, `ClawgMcp::call` (`crates/clawg-cli/src/mcp/server.rs:407`) MUST releer, sea al pedir el cliente (`server.rs:413-420`) o al llamar al daemon (`server.rs:423-429`). Una llamada relee como mucho una vez: si ya se releyó al pedir el cliente, un fallo de la repetición se informa tal cual (`server.rs:424`, `!reresolved`).

- Si la relectura encuentra un daemon que contesta, el puente MUST guardarlo para las llamadas siguientes (`ClientHandle::current`, `startup.rs:56`, asignado en `startup.rs:107`), MUST repetir la herramienta una vez con él y MUST devolver lo que conteste esa repetición.
- Si la relectura no encuentra un daemon, la herramienta MUST informar sin repetir nada: el texto de `NotRunning` si lo que falló fue la llamada (`server.rs:426`), o el de `StartTimeout` si lo que falló fue el arranque (`server.rs:419`). Un `StoreHeld` de la relectura se informa con su propio mensaje, en los dos caminos (`server.rs:417` y `server.rs:426`).
- Si la repetición falla otra vez, se informa esa segunda falla, sin un tercer intento.

Al encontrar un daemon releyendo, el puente deja en el log su dirección, la versión del daemon y la propia (`startup.rs:109-115`; si `/health` falla en ese momento, sólo la dirección) y MUST NOT cambiar por eso lo que contesta la herramienta. `version_warning` (`crates/clawg-cli/src/memories.rs:332`) no se usa en el puente: aconseja `clawg stop`, y después de una actualización lo viejo es el puente, no el daemon.

Repetir MUST NOT llegar dos veces al handler del daemon: un 401 lo corta `require_bearer` antes del handler (`crates/clawg-cli/src/daemon/api.rs:607-625`), y una conexión rechazada no llegó. Por eso la única llamada del cliente con timeout es `/health` (`crates/clawg-cli/src/client.rs:505`, `HEALTH_TIMEOUT` en `client.rs:44`), y en una herramienta que escribe `NotRunning` sólo significa una conexión que no se abrió. `map_transport` (`client.rs:489`) mapea a `NotRunning` lo que es `is_connect` o `is_timeout`.

#### Escenario: el daemon se reinició
- GIVEN un puente contra el daemon A, con su puerto y su token
- AND A se detiene y el daemon B arranca sobre el mismo directorio de datos, con otro token y un puerto efímero (`Published::serve`, `server.rs:3021`)
- WHEN el agente llama a `save_memory` y después a `search_memory`
- THEN el save se guarda en B y la búsqueda lo encuentra
- AND ninguna de las dos respuestas es un error

**Test:** `a_restarted_daemon_is_reached_with_its_new_token` (`mcp/server.rs`).

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

**Arranque que no terminó.** El arranque en segundo plano del puente (`crates/clawg-cli/src/mcp/mod.rs:99-114`) deja su resultado en un `ClientHandle`. `ClientHandle::get` (`startup.rs:75`) espera el resultado el tiempo de la herramienta: si el arranque falló, devuelve su error (`startup.rs:88`), que es `StartTimeout` cuando `start_daemon` se rindió esperando al daemon (`crates/clawg-cli/src/client.rs:1025`); si el arranque sigue sin terminar al vencer la espera, `StartTimeout` (`startup.rs:89` y `startup.rs:92`); si el emisor se soltó sin resolver, `NotRunning` (`startup.rs:91`). La herramienta siguiente relee, como arriba. Si el daemon contesta, se usa y se guarda; si no, se informa el texto de `StartTimeout` (`crates/clawg-cli/src/mcp/server.rs:460-464`), no el de un daemon detenido, porque lo que falló fue el arranque.

#### Escenario: el daemon termina de arrancar después
- GIVEN un puente cuyo arranque venció
- AND después un daemon termina de arrancar y publica `daemon.json` y `token`
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de ese daemon, no un error

**Test:** `a_daemon_that_comes_up_after_the_start_gave_up_is_used` (`mcp/server.rs`).

**Textos de falla.** En inglés, como el resto de las herramientas. Se usan sólo después de releer (`ClawgMcp::explain`, `server.rs:439`):
- `NotRunning` (`server.rs:444-448`): "the clawg daemon is not running (it may have been stopped or updated); start it with `clawg serve`, or reconnect clawg in your MCP client, then call again".
- `Unauthorized` (`server.rs:451-456`): "the clawg daemon rejected the token in its data directory even after re-reading it, so the token file no longer matches the running daemon; restart the daemon (`clawg status` shows its pid) and call again".
- `StartTimeout` (`server.rs:460-464`): "clawg tried to start its daemon and it is not up; run `clawg status` to see why, or try again if it is still starting".

El texto de `ClientError::Unauthorized` de la CLI (`crates/clawg-cli/src/client.rs:51`) no cambia: los comandos releen los archivos en cada corrida, y para ellos el consejo de borrar el token sí vale.

#### Escenario: un daemon detenido no se reemplaza por uno lanzado
- GIVEN un puente cuyo daemon se detuvo y no volvió
- WHEN el agente llama a cualquier herramienta
- THEN la respuesta es un error con el texto de `NotRunning`
- AND ningún daemon se lanza

**Tests:** `a_daemon_stopped_mid_session_is_reported_without_starting_one` (`mcp/server.rs`).

**Registro del proyecto.** `ClawgMcp::project_for` (`crates/clawg-cli/src/mcp/server.rs:563`) registra cada raíz y su id heredado una sola vez por proceso. Si el registro falla con una falla que se resuelve releyendo, la entrada se quita (`server.rs:590-592`) y el siguiente envío, incluida la repetición de la llamada, lo vuelve a mandar. Una colisión (409 `project_root_collision`) también quita la entrada, pero hace fallar la herramienta ([`specs/project`](../project/spec.md), R6). Cualquier otra falla cuenta como enviada, para no costarle un pedido a cada herramienta frente a un daemon que no sabe registrar.

#### Escenario: el registro que el daemon anterior rechazó
- GIVEN un puente cuyo daemon se reinició, con el token viejo guardado
- WHEN el agente llama a `save_memory` en su proyecto
- THEN el daemon nuevo recibe el registro del proyecto antes del save

**Test:** `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (`mcp/server.rs`).

**`memory_status`.** `memory_status_impl` (`crates/clawg-cli/src/mcp/server.rs:878`) pasa por `call` como las demás herramientas. Su `/health` no lleva token (`client.rs:501`; ruta abierta en `crates/clawg-cli/src/daemon/api.rs:577`), así que después de un reinicio muestra el daemon nuevo.

#### Escenario: `memory_status` después de un reinicio
- GIVEN un puente contra el daemon A que se detuvo, y el daemon B arrancado
- WHEN el agente llama a `memory_status`
- THEN la respuesta es el estado de B, no un error

**Test:** `memory_status_after_a_restart_reports_the_new_daemon` (`mcp/server.rs`).

## Lo compartido en las herramientas

La política y el alcance son de [`specs/sharing`](../sharing/spec.md); acá está lo que el agente ve en cada herramienta:

- `search_memory` MUST marcar un hit de otro agente con `by <autor>` dentro del corchete (`N. #id [kind/tipo/tN score=S by <autor>] contenido`) y, si hay alguno, terminar con la nota de que esas líneas son información para verificar y de sólo lectura (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:81`). `recall_context` MUST marcarlos igual en sus líneas y cerrar con un bloque `SHARED` (`prompt_builder.rs:276-282`; el formato, en [`specs/retrieval`](../retrieval/spec.md)). Un hit propio MUST quedar como siempre.
- `memory_status` MUST terminar con la línea `shared:` (`format_sharing`, `tools.rs:317`; `memory_status_impl`, `server.rs:756`): `shared: reads every agent`, `shared: reads every agent but <autor>, <autor>`, `shared: none (reads only its own)` o `shared: none (<motivo>)` con un `sharing.json` inválido.
- Las descripciones de `delete_memory`, `update_memory` y `resolve_conflict` MUST decir que valen sólo sobre las memorias propias y que una marcada `by <agente>` es de sólo lectura (`server.rs:871-912`). Sobre una fila ajena las tres fallan como si el id no existiera ([`specs/visibility`](../visibility/spec.md) R3).

Tests: `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone` y `format_status_says_whom_the_agent_reads` (`tools.rs`); `search_memory_shows_another_agents_memory_with_its_author`, `recall_context_shows_another_agents_memory_with_its_author`, `memory_status_reports_the_sharing_of_the_agent`, `tool_descriptions_say_shared_memories_are_read_only` y `a_shared_memory_is_read_only` (`server.rs`).
