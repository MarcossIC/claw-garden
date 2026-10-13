# mcp-protocol-upgrade — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 (interoperabilidad) | M | visibility-policy (fuente de identidad) | exploración (cerrada: ver [proposal.md](proposal.md)) |

**Tesis:** la spec MCP vigente (2026-07-28) elimina el handshake `initialize`, también en stdio. Hoy no se rompe nada, porque rmcp 1.3.0 negocia `initialize` y los clientes nuevos hacen fallback. El riesgo aparece al actualizar rmcp o cuando un cliente deje de hacer fallback.

## Hechos (verificados contra fuentes primarias)

### Spec 2026-07-28

Fuente: [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog.md).

- **SEP-2567 y SEP-2575** (estado final) eliminan `Mcp-Session-Id`, que era del transporte Streamable HTTP. Stdio nunca lo tuvo.
- **Se elimina `initialize` / `notifications/initialized`.**
- **Cada request lleva en `_meta`:**
  - `io.modelcontextprotocol/protocolVersion` y `io.modelcontextprotocol/clientCapabilities` (obligatorios);
  - `io.modelcontextprotocol/clientInfo`, que es **SHOULD**, no obligatorio.
- **`server/discover` es MUST.** En stdio, el cliente lo puede usar como sonda de compatibilidad.
- **Cambios a la respuesta de `tools/list`:**
  - orden determinístico (SHOULD);
  - `ttlMs` y `cacheScope` obligatorios;
  - `resultType` obligatorio en todos los resultados.
- **Roots, Sampling y Logging quedan deprecados.** En lugar de Roots se recomiendan "tool parameters o server configuration".

### rmcp

- El proyecto usa **1.3.0**: su `LATEST` es 2025-06-18.
- La última es **3.5.1** (2026-10-05). Soporta 2026-07-28 y conserva `initialize` para clientes legacy (3.2.0). `ServerHandler::negotiate_initialize` se agregó en 3.3.0.
- **Cruza dos mayores:**
  - 2.0.0 alinea los tipos con 2025-11-25;
  - 3.0.0 trae discovery SEP-2575, MRTR, la extensión de tasks y `_meta`.
- El blog oficial dice que el soporte del SDK de Rust para la spec nueva está en "beta".

### Hoy en clawg

- La identidad del agente sale de `peer_info().client_info.name`, que se lee del handshake (`crates/clawg-cli/src/mcp/server.rs:199-203`).
- `Peer::peer_info()` se documenta como "Snapshot of the peer's handshake info". No está documentado que se complete desde `_meta` cuando el cliente no manda `initialize`. **Hay que probarlo.**

### Historial

- El handshake ya se rompió una vez, con Claude Code 2.1.285 (`fa048ad`, `1dd91b5`, `e9e6f61`).
- Es una zona frágil y requiere verificación manual con el cliente real.

### No verificado

- Si Claude Code ya habla 2026-07-28.

## Alcance candidato

1. Actualizar rmcp 1.3.0 → 3.5.1, manteniendo `initialize` para clientes legacy.
2. Implementar `server/discover`, y leer `protocolVersion`, `clientCapabilities` y `clientInfo` desde `_meta` en cada request.
3. Que la identidad del agente **no dependa** del handshake. Lo decide visibility-policy: configuración de lanzamiento > clientInfo > fallback.
4. Agregar `ttlMs`/`cacheScope` y `resultType`, y garantizar el orden determinístico en `tools/list`.

## Decisiones abiertas

- [x] **¿Cuándo?** ¿Ahora, o cuando Claude Code adopte 2026-07-28? Hay que averiguar el estado actual de Claude Code. → Ahora ([propuesta](proposal.md#decisiones-cierran-las-abiertas-de-la-exploración)).
- [x] **¿Directo a 3.5.1 o por etapas** (2.x primero)? → Directo.
- [x] **¿Se mantiene la versión en el `ServerInfo`** que hoy se publica? → Sí.

## Cómo se mide

- Siguen pasando los tests del handshake que ya existen.
- **Tests nuevos:**
  - `server/discover`;
  - una request sin `initialize` con `_meta`;
  - una request sin `clientInfo`: la identidad sale de la configuración y las memorias siguen visibles.
- **Verificación manual con Claude Code real**, contra el perfil dev, como en las regresiones anteriores.
