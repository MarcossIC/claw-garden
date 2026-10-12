# mcp-protocol-upgrade — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 (interoperabilidad) | M | visibility-policy, sólo para `--agent` (fuera de alcance, ver abajo) | verify |

## Problema

Hechos de la [exploración](exploration.md), verificados contra el código de `main` (1127b50) y contra el código fuente de rmcp 3.5.1:

1. **El puente habla una versión del protocolo que ya no es la vigente.** `crates/clawg-cli/Cargo.toml:47` pide rmcp 1.x, y el lock tiene 1.3.0, cuyo `LATEST` es 2025-06-18. La spec 2026-07-28 saca `initialize`, también en stdio, y exige `server/discover`.
2. **La compatibilidad de hoy es un parche atado a 1.x.** `PreInitGuard` (`crates/clawg-cli/src/mcp/handshake.rs`) contesta en nombre de rmcp todo lo que llega antes de `initialize`: `-32601` para métodos que rmcp 1.3 no conoce, como `server/discover` (`handshake.rs:63-67`), y `-32600` para el resto (`handshake.rs:68`). Con rmcp 3.x, `server/discover` es un método conocido y el guardia lo rechazaría como "enviado antes de `initialize`" (lo comprobé: el test `an_unknown_method_is_answered_as_method_not_found_under_its_own_id` pasa a dar `-32600`). El guardia no sobrevive a la actualización tal como está.
3. **La identidad del agente sale sólo del handshake.** `agent_id_for` lee `peer_info().client_info.name` (`crates/clawg-cli/src/mcp/server.rs:223-227`). En el ciclo de vida 2026-07-28 no hay handshake: rmcp 3.5.1 nunca llama a `set_peer_info` y el cliente se declara en el `_meta` de cada request (rmcp `src/service/server.rs:573-686`). Sólo con subir rmcp, una sesión nueva guardaría y buscaría bajo `"mcp"`, y **las memorias del cliente dejarían de verse**. Lo comprobé con los tests de identidad de este cambio contra la lectura de hoy: los tres fallan.
4. **El cliente real ya sondea.** Claude Code 2.1.285 abre cada sesión con `server/discover` y cae a `initialize` sólo si la sonda falla (comentario de `handshake.rs:5-13`). Con rmcp 3.5.1 la sonda va a tener respuesta, así que Claude Code puede pasar al ciclo de vida nuevo apenas se instale el binario.

## Decisiones (cierran las abiertas de la exploración)

- **Ahora, no cuando Claude Code adopte 2026-07-28.** El guardia es un parche sobre los internos de rmcp 1.x, y el cliente ya sondea. Esperar deja la zona frágil igual de frágil y suma dos mayores de atraso.
- **Directo a 3.5.1, sin etapa 2.x.** rmcp 2.x tampoco implementa `server/discover`, así que una etapa intermedia necesitaría su propio arreglo del guardia, que después se tira. La migración del código propio es chica: `Content` → `ContentBlock`, `ServerInfo` → `ServerConfig` y el `id` opcional de `ServerJsonRpcMessage::error`.
- **Se mantiene la versión en el `serverInfo`.** En `initialize` como hoy, y en `server/discover` bajo `_meta["io.modelcontextprotocol/serverInfo"]`, que es donde rmcp 3.5.1 la pone. `clawg status` y el usuario distinguen así un binario viejo de uno nuevo.

## Alcance

- rmcp 1.3.0 → 3.5.1.
- `server/discover` con las versiones soportadas (2026-07-28 y todas las legacy), las capacidades, las instrucciones y el `serverInfo` del perfil.
- Sesiones sin `initialize`: cada request con su `_meta` se atiende. `tools/list` en orden por nombre, con `resultType`, `ttlMs` y `cacheScope`.
- `initialize` sigue funcionando igual para los clientes legacy.
- El guardia se achica a lo único que rmcp 3.5.1 todavía no tolera antes de una sesión: notificaciones y respuestas sueltas, que lo terminan con `ExpectedInitializeRequest` (rmcp `src/service/server.rs:682`). La cancelación de la sonda es una de ellas.
- **Identidad sin handshake:** el nombre que declara la request, si no el último que declaró un cliente a este puente (también en `server/discover`), si no `"mcp"`.

## Fuera de alcance

- **`clawg mcp --agent <nombre>` y la precedencia configuración > clientInfo > fallback.** Es la pregunta 1 de visibility-policy ("visibility-identity"), que tiene su thread abierto. Este cambio deja la resolución en una función pura, `agent_id_from` (`server.rs`), donde esa configuración entra como un argumento más. Riesgo que queda: un cliente 2026-07-28 que **nunca** declara `clientInfo` (es SHOULD) guarda y busca bajo `"mcp"`. Es lo mismo que pasa hoy con un cliente sin nombre, pero ahora es un caso posible del protocolo. Lo cierra visibility-identity.
- Roots, Sampling y Logging: el puente no usa ninguno (`rg -i "roots|sampling|set_level" crates/clawg-cli/src/mcp` sólo encuentra un comentario sobre raíces de proyecto).
- El transporte Streamable HTTP y `Mcp-Session-Id`: el daemon habla HTTP propio, no MCP.
- Cambios a los datos guardados: ninguno. No hay migración ni cambia ningún `agent_id` ya escrito.

## Cómo se mide

Tests que fallan hoy (en `main` o contra la lectura de identidad de hoy) y pasan con el cambio:

- `a_discover_probe_is_answered_with_what_the_server_offers`: hoy la sonda recibe `-32601`.
- `a_session_without_initialize_lists_the_tools_in_name_order`: hoy rmcp 1.3 no acepta una request sin `initialize`.
- `a_session_without_initialize_saves_under_the_client_it_declares`, `a_request_without_client_info_keeps_the_name_declared_on_discover` y `memories_saved_through_initialize_stay_visible_without_it`: con la lectura de hoy guardan y buscan bajo `"mcp"`.

Siguen pasando los tests del handshake legacy (`crates/clawg-cli/src/mcp/mod.rs`, el de la sonda de Claude Code) y el resto del suite. `rag-eval` no aplica: no cambia nada del core ni del ranking.

**Verificación manual con Claude Code real, contra el perfil dev** (máquina de Marcos): conecta, lista las siete herramientas y las memorias guardadas antes del cambio se siguen encontrando.

## Vuelta atrás

No toca la base. Volver al binario anterior alcanza: las memorias guardadas con este cambio llevan como `agent_id` el nombre que declaró el cliente, igual que hoy. Un cliente que se llama igual en `initialize` y en `_meta` ve las mismas memorias con los dos binarios.
