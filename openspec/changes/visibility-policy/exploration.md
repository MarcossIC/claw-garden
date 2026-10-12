# visibility-policy — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | L (probablemente se parta) | project-identity, eval-golden-expansion | exploración (borrador) |

**Tesis:** el objetivo 4 pide un cerebro compartido **con control del usuario sobre qué ve cada agente**. Hoy hay aislamiento total en lectura y ningún control en borrado. Sacar el filtro por agente, la salida obvia, resuelve la mitad del objetivo y rompe la otra mitad.

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

### La columna `scope` ya existe, sin uso

- Las tres tablas tienen `scope` con su CHECK (`crates/clawg-core/migrations/V1__init.sql:6-7, 30-34`):
  - `agent` exige `agent_id` y admite `project_id`;
  - `project` exige `project_id` y **`agent_id IS NULL`**;
  - `global` exige los dos en NULL.
- El insert escribe siempre el literal `"agent"` (`crates/clawg-core/src/memory/store_insert.rs:177`).
- `enum Scope` (`crates/clawg-core/src/memory/record.rs:19`) se exporta (`memory/mod.rs:42`), pero nada lo usa. Los `Scope` de `clawg-cli` son otra struct: los filtros del visor (`crates/clawg-cli/src/cli.rs:133`).

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
   - Un `WHERE` sobre la tabla padre en la misma query del `MATCH` no alcanza: vec0 elige los k vecinos antes del join, y es el mismo crowding de hoy.
7. **Procedencia de lo compartido.** Hoy cada hit se muestra como `[fecha] SOURCE | TYPE` y `CONTENT` crudo, sin autor (`crates/clawg-core/src/memory/prompt_builder.rs:300-306`). Mientras search filtra por `agent_id`, el autor siempre es el mismo agente. Cuando se comparta:
   - el bloque de `recall_context` y `search_memory` tiene que decir **qué agente** guardó cada memoria ajena;
   - el contenido ajeno se presenta como dato, no como instrucción. Una memoria envenenada (por ejemplo, guardada después de leer un README malicioso) no debería llegarle a otro agente como una orden;
   - un secreto guardado por un agente viaja a otro cliente y a su proveedor. Ver secret-hygiene, que conviene cerrar antes de abrir la política.
8. **Compartir con la columna o con una regla.**
   - **Columna:** se pasa la fila a `scope = 'project'` o `'global'`. El invariante queda en la base, pero el CHECK obliga a poner `agent_id` en NULL, así que la autoría (pregunta 7) tiene que salir de `created_by_ref`, que hoy guarda el mismo `agent_id`. Es una migración de datos y lleva su test de preservación (`config.yaml`).
   - **Regla de lectura:** `scope` queda como está y la visibilidad se decide en la consulta, según la política configurada. No toca ninguna fila, pero el invariante vive en código y configuración, no en la base.
   - Principios candidatos, con cualquiera de las dos: una memoria tiene un solo scope (si algo vale para dos, son dos memorias con historia propia), y la promoción entre scopes es sólo manual, nunca automática.
9. **Pesos por scope en el ranking.** Por ejemplo agente 1,20, proyecto 1,00 y global 0,85: una relación de 1,41x, mayor que todo el rango de importancia (1,18x). Ensanchar la importancia ya bajó R@10 de 0,891 a 0,826 (ver memory-update-tool). Hoy no cambian nada, porque todas las filas son `agent`. Sólo tienen sentido cuando se comparta, y entran únicamente si `rag-eval --diff` lo justifica.

## Posible partición en cambios más chicos

- **visibility-identity**: la fuente de la identidad del agente (pregunta 1). Es requisito de mcp-protocol-upgrade.
- **visibility-rules**: la política y su configuración, más el scope de list/delete/update (preguntas 2 a 5).
- **vec-prefilter**: partition keys o metadata en vec0 (pregunta 6). Sólo se justifica si el golden set muestra crowding.

## Cómo se mide

- **Casos multi-agente y multi-proyecto** del golden set ampliado, cada uno con la visibilidad esperada.
- **Un test por regla de la política**, incluido "el agente B no puede borrar la memoria de A si la política lo prohíbe".
- **Test de regresión de identidad:** sin `clientInfo`, las memorias de un agente configurado siguen visibles.
- **Test de procedencia:** una memoria de A que le llega a B en `recall_context` y en `search_memory` lleva el autor A; una memoria propia no cambia de formato.
- **Si se elige la columna (pregunta 8):** un test que siembra filas `agent` en la versión anterior, las pasa a `project` y prueba que siguen visibles y conservan el autor.
- **Si se agregan pesos por scope (pregunta 9):** `rag-eval --diff` contra el baseline, sobre los casos multi-agente.
