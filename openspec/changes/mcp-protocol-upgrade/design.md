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
| **b. Guardia que sólo descarta notificaciones y respuestas antes de una sesión** | Cubre lo único que rmcp no tolera; todo lo demás lo contesta rmcp, con sus códigos | Sigue habiendo un envoltorio del transporte |
| c. Guardia que además mira las respuestas de rmcp para saber si una request sin `_meta` empezó la sesión o no | Exacto en el caso raro de una request rechazada seguida de una notificación | Más estado y más código para un host que, después de un error, igual reintenta antes de mandar otra cosa |

**Elegida b.** `before_session` es pura: `Start` para cualquier request que no sea `ping` ni `server/discover`, `Forward` para esas dos, `Discard` para notificaciones y respuestas. Comprobé que hace falta: sin el guardia, `a_bare_probe_and_its_cancellation_leave_the_legacy_handshake_working` falla.

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

**Elegida b.** `server/discover` también recuerda: el servidor sobrescribe `discover` con la respuesta de rmcp más el registro del nombre. La regla queda en `agent_id_from(declarado, recordado, fallback)`, pura y probada sin transporte.

Cuando visibility-identity agregue `--agent`, entra como primer argumento de esa misma función. No hace falta tocar cómo se lee el contexto.

## D4. Sin cambios a lo que rmcp genera

- **Versiones:** `supported_protocol_versions` queda en el default, `ProtocolVersion::KNOWN_VERSIONS` (de 2024-11-05 a 2026-07-28). Achicarlo dejaría afuera a clientes legacy que hoy conectan.
- **`tools/list`:** `ToolRouter::list_all` ya ordena por nombre (`src/handler/server/router/tool.rs:588`), y `#[tool_handler]` pone `ttlMs: 0` y `cacheScope` para clientes 2026-07-28 (rmcp-macros `src/tool_handler.rs:79-81`). `ttlMs: 0` le dice al host que no guarde la lista. Es correcto para este puente: la lista cambia con el binario, y pedirla de nuevo no cuesta nada.
- **`resultType`:** rmcp lo pone sólo en sesiones 2026-07-28 y lo omite en las legacy, así que lo que ve un cliente legacy no cambia.

Los tests de R1 y R3 fijan estos valores en el cable, para que una actualización de rmcp que los cambie no pase sin que nadie lo vea.
