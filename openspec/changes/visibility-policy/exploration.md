# visibility-policy — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | L (probablemente se parta) | project-identity, eval-golden-expansion | exploración (borrador) |

**Tesis:** el objetivo 4 pide un cerebro compartido **con control del usuario sobre qué ve cada agente**. Hoy hay aislamiento total en lectura y ningún control en borrado. El análisis propone sacar el filtro por agente, y eso resuelve la mitad del objetivo y rompe la otra mitad.

## Problema (verificado)

### Predicado de search/recall

`crates/clawg-core/src/memory/search.rs:119-122`:

```
m.agent_id = ?3
AND (?4 IS NULL OR m.project_id = ?4 OR m.project_id IS NULL OR m.memory_type = 'user_preference')
```

- Las tablas episodic y procedural usan el mismo predicado, sin la cláusula de `user_preference`.
- Hoy no se comparte nada entre agentes. `user_preference` cruza proyectos, pero nunca agentes.

### Identidad del agente

- Es `clientInfo.name` del handshake, con fallback a `"mcp"`, y no se normaliza (`crates/clawg-cli/src/mcp/server.rs:199-203`).
- Si el cliente cambia de nombre, aunque sea en mayúsculas o espacios, queda como otro agente.

### Operaciones sin scope

- `list_memories` y `delete_memory` (`server.rs:337-373`).
- `PATCH` y `DELETE /v1/memories/{kind}/{id}` (`crates/clawg-cli/src/daemon/api.rs:584-616`).
- **Cualquier agente puede borrar cualquier memoria** conociendo `(kind, id)`.
- `crates/clawg-core/src/memory/admin.rs:327-328` lo dice explícitamente: `agent_id` "is not a visibility rule" para listar.

### Crowding

- El filtro de agente y proyecto se aplica **después** de que vec0 corta a k=30 (`search.rs:97-123`).
- Hay un solo reintento a 4× (`search.rs:276-281, 400-411`).
- Cuantas más memorias haya de otros agentes y proyectos, menos lugar queda en el pool para las propias.

### Spec MCP 2026-07-28

- `clientInfo` pasa a ser opcional y autodeclarado, y los servidores "SHOULD NOT" usarlo para cambiar comportamiento.
- Cuando el cliente deje de mandar `initialize` y se actualice rmcp, `peer_info()` puede venir vacío. Todo cae al fallback `"mcp"` y las memorias existentes **dejan de verse sin ningún error**.

### Seguridad de la ruta

- `agent_id` se usa crudo en la ruta del workspace (`crates/clawg-core/src/memory/workspace.rs:43-46`).
- Severidad baja, porque es el mismo usuario.

## Preguntas de diseño

1. **De dónde sale la identidad:**
   - de la configuración de lanzamiento (`clawg mcp --agent <nombre>` en el registro MCP, que controla el usuario);
   - de `clientInfo`;
   - de las dos, con prioridad config > clientInfo > fallback.
2. **Política por defecto.** ¿Compartido por proyecto entre agentes, con `agent_id` como autoría? ¿Con listas por agente (allow/deny) para el control del usuario?
3. **Dónde configura el usuario.** ¿Un archivo en el directorio de datos? ¿Un comando `clawg agents ...`?
4. **Global entre proyectos (objetivo 2).** ¿Qué tipos cruzan proyectos? Hoy cruzan `user_preference` y las filas sin proyecto. ¿Decisiones de arquitectura no?
5. **Scope de list/delete/update.** ¿Siguen la misma política que search?
6. **Filtrar antes del corte k:**
   - sqlite-vec ≥ 0.1.6 soporta partition keys y metadata columns en vec0. El proyecto está en 0.1.9 (`Cargo.lock`).
   - Límites: las partition keys sólo aceptan TEXT o INTEGER, sin `IN`. Los NULL no están soportados en metadata, y `project_id` puede ser NULL, así que hace falta un sentinela.
   - Hay que recrear las tablas vec0 y re-insertar los vectores.

## Posible partición en cambios más chicos

- **visibility-identity**: la fuente de la identidad del agente (pregunta 1). Es requisito de mcp-protocol-upgrade.
- **visibility-rules**: la política y su configuración, más el scope de list/delete/update (preguntas 2 a 5).
- **vec-prefilter**: partition keys o metadata en vec0 (pregunta 6). Sólo se justifica si el golden set muestra crowding.

## Cómo se mide

- **Casos multi-agente y multi-proyecto** del golden set ampliado, cada uno con la visibilidad esperada.
- **Un test por regla de la política**, incluido "el agente B no puede borrar la memoria de A si la política lo prohíbe".
- **Test de regresión de identidad:** sin `clientInfo`, las memorias de un agente configurado siguen visibles.
