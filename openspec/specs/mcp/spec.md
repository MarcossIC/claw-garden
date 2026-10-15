# mcp — Especificación

Fuente de verdad del comportamiento actual del puente `clawg mcp`: cómo abre una sesión por stdio, en el ciclo legacy (`initialize`) y en el 2026-07-28 (`_meta` en cada request), y de qué nombre saca el `agent_id` de cada request. Qué filas modifica y lista cada agente está en [`specs/visibility`](../visibility/spec.md), y qué filas de otros agentes lee en [`specs/sharing`](../sharing/spec.md); este dominio sólo decide qué nombre entra como `client` en `resolve_agent` y cómo las herramientas muestran lo compartido (último apartado). La herramienta `search_project`, que busca en la documentación y el código del proyecto, está en [`specs/project-docs`](../project-docs/spec.md); acá sólo se la cuenta entre las herramientas que lista el puente.

**Origen:**
- [mcp-protocol-upgrade](../../changes/archive/2026-10-13-mcp-protocol-upgrade/proposal.md), archivado el 2026-10-13.
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo "Lo compartido en las herramientas").
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo el conteo y la lista de herramientas).

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

## Lo compartido en las herramientas

La política y el alcance son de [`specs/sharing`](../sharing/spec.md); acá está lo que el agente ve en cada herramienta:

- `search_memory` MUST marcar un hit de otro agente con `by <autor>` dentro del corchete (`N. #id [kind/tipo/tN score=S by <autor>] contenido`) y, si hay alguno, terminar con la nota de que esas líneas son información para verificar y de sólo lectura (`format_hits`, `crates/clawg-cli/src/mcp/tools.rs:81`). `recall_context` MUST marcarlos igual en sus líneas y cerrar con un bloque `SHARED` (`prompt_builder.rs:276-282`; el formato, en [`specs/retrieval`](../retrieval/spec.md)). Un hit propio MUST quedar como siempre.
- `memory_status` MUST terminar con la línea `shared:` (`format_sharing`, `tools.rs:317`; `memory_status_impl`, `server.rs:756`): `shared: reads every agent`, `shared: reads every agent but <autor>, <autor>`, `shared: none (reads only its own)` o `shared: none (<motivo>)` con un `sharing.json` inválido.
- Las descripciones de `delete_memory`, `update_memory` y `resolve_conflict` MUST decir que valen sólo sobre las memorias propias y que una marcada `by <agente>` es de sólo lectura (`server.rs:871-912`). Sobre una fila ajena las tres fallan como si el id no existiera ([`specs/visibility`](../visibility/spec.md) R3).

Tests: `format_search_marks_shared_hits_and_adds_the_note`, `format_search_leaves_own_hits_alone` y `format_status_says_whom_the_agent_reads` (`tools.rs`); `search_memory_shows_another_agents_memory_with_its_author`, `recall_context_shows_another_agents_memory_with_its_author`, `memory_status_reports_the_sharing_of_the_agent`, `tool_descriptions_say_shared_memories_are_read_only` y `a_shared_memory_is_read_only` (`server.rs`).
