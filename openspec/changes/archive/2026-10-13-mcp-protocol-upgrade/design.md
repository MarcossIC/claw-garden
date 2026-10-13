# mcp-protocol-upgrade — Diseño

Las decisiones de cuándo, cómo subir y qué publicar están en la [propuesta](proposal.md). Acá van las de implementación. Las referencias a rmcp son al código fuente de 3.5.1 publicado en crates.io.

## D1. Qué queda del guardia

rmcp 3.5.1, antes de una sesión (`src/service/server.rs:573-686`):

- contesta `ping`;
- contesta `server/discover` sin elegir ciclo de vida;
- a una request sin el `_meta` obligatorio le contesta `-32602` y sigue esperando;
- con `initialize` empieza el ciclo legacy, y con otra request con `_meta` completo, el 2026-07-28;
- **con cualquier otra cosa (notificación o respuesta) termina** con `ExpectedInitializeRequest` (`:682`).

| Opción | A favor | En contra |
|---|---|---|
| a. Borrar el guardia | Menos código | Una sonda cancelada (`notifications/cancelled`) vuelve a matar el puente: es la misma falla de `CONNECT_TIMEOUT` que el guardia arregló |
| b. Guardia que descarta notificaciones y respuestas hasta que pasa la primera request que puede empezar una sesión | Simple | Si rmcp rechaza esa request por falta de `_meta`, no hay sesión pero el guardia ya se apagó: una cancelación después mata el puente (lo encontró la review del PR, y lo reproduce `a_request_refused_for_missing_meta_does_not_start_the_session`) |
| **c. Igual que b, pero la sesión cuenta como empezada recién cuando rmcp contesta esa request con un resultado** | Exacto: un rechazo deja el guardia prendido | El guardia mira también lo que rmcp manda, con un id pendiente de estado |

**Elegida c.** `before_session` es pura: `Start` para cualquier request que no sea `ping` ni `server/discover`, `Forward` para esas dos y `Discard` para notificaciones y respuestas. Con `Start` el guardia anota el id. Si rmcp contesta ese id con un resultado, el guardia se apaga. Si lo contesta con un error, sigue prendido.

Costo aceptado: en una sesión 2026-07-28, una notificación que llega mientras la primera request todavía no tiene respuesta se descarta. La única que puede llegar ahí es la cancelación de esa misma request, y descartarla sólo hace que la request termine normalmente.

Comprobé que el guardia hace falta: sin él, `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working` falla.

## D2. Leer la identidad del `RequestContext`, no del `Peer`

rmcp 3.5.1 ofrece `RequestContext::client_info()` (`src/service.rs:1243`): en el ciclo 2026-07-28 lee sólo el `_meta` de la request; en el legacy, el `_meta` y si no hay, el handshake. Las tres herramientas que usan el `agent_id` pasan de recibir `Peer<RoleServer>` a recibir `RequestContext<RoleServer>`, que el macro `#[tool]` ya sabe extraer.

La alternativa, leer `_meta` a mano y caer a `peer_info()`, repite lo que rmcp ya resuelve, incluida la regla de no mezclar ciclos.

## D3. Recordar el último nombre declarado

`clientInfo` es SHOULD por request. Un cliente puede nombrarse en `server/discover` o en la primera request y omitirlo después. Sin memoria, esa request guardaría bajo `"mcp"`: la memoria del usuario se partiría en dos y la mitad dejaría de verse, que es justo lo que prohíbe `config.yaml`.

| Opción | A favor | En contra |
|---|---|---|
| a. Sólo la request, si no `"mcp"` | Sin estado | El caso de arriba esconde memorias |
| **b. La request, si no el último nombre declarado a este proceso, si no `"mcp"`** | Un puente stdio sirve a un solo cliente, así que el nombre anterior es del mismo cliente | Un `Mutex<Option<String>>` compartido por los clones del servidor, como ya pasa con `registered` |
| c. Exigir `clientInfo` y rechazar la request | Explícito | Rompe a un cliente que cumple la spec, porque es SHOULD |

**Elegida b.** `server/discover` también recuerda: el servidor sobrescribe `discover` con la respuesta de rmcp más el registro del nombre. La regla queda en `client_name(declarado, recordado)`, pura y probada sin transporte. Su resultado es el `client` de `resolve_agent(flag, client)`, que trajo visibility-policy con `--agent`: el flag gana, después el nombre, después `"mcp"`.

## D4. Sin cambios a lo que rmcp genera

- **Versiones:** `supported_protocol_versions` queda en el default, `ProtocolVersion::KNOWN_VERSIONS` (de 2024-11-05 a 2026-07-28). Achicarlo dejaría afuera a clientes legacy que hoy conectan.
- **`tools/list`:** `ToolRouter::list_all` ya ordena por nombre (`src/handler/server/router/tool.rs:588`), y `#[tool_handler]` pone `ttlMs: 0` y `cacheScope` para clientes 2026-07-28 (rmcp-macros `src/tool_handler.rs:79-81`). `ttlMs: 0` le dice al host que no guarde la lista. Es correcto para este puente: la lista cambia con el binario, y pedirla de nuevo no cuesta nada.
- **`resultType`:** rmcp lo pone sólo en sesiones 2026-07-28 y lo omite en las legacy, así que lo que ve un cliente legacy no cambia.

Los tests de R1 y R3 fijan estos valores en el cable, para que una actualización de rmcp que los cambie no pase sin que nadie lo vea.
