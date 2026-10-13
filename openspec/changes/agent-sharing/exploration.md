# agent-sharing — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | L | visibility-policy (A + B), secret-hygiene | exploración (borrador) |

**Origen:** las partes C (compartir) y D (prefiltro en vec0) de [visibility-policy](../archive/2026-10-12-visibility-policy/exploration.md). Las partes A (identidad) y B (scope de lo que modifica) están archivadas en [`specs/visibility`](../../specs/visibility/spec.md).

**Tesis:** el objetivo 4 pide un cerebro compartido **con control del usuario sobre qué ve cada agente**. Después de A y B, cada agente tiene una identidad estable (`clawg mcp --agent`) y sólo lista, borra, cambia o resuelve lo propio. La lectura sigue aislada del todo: search y recall filtran por `agent_id`. Falta decidir **qué** se comparte, **cómo lo controla el usuario** y **cómo llega lo ajeno** al contexto de otro agente.

## Punto de partida (verificado al archivar visibility-policy)

- **Predicado de search/recall:** `m.agent_id = ?3`. `user_preference` cruza proyectos, pero nunca agentes. Las tablas episodic y procedural usan el mismo predicado, sin la cláusula de `user_preference`. Ver `crates/clawg-core/src/memory/search.rs`.
- **La columna `scope` existe y no se usa:** las tres tablas tienen su CHECK (`crates/clawg-core/migrations/V1__init.sql`):
  - `agent` exige `agent_id`;
  - `project` exige `project_id` y `agent_id IS NULL`;
  - `global` exige los dos en NULL.

  El insert escribe siempre `"agent"`.
- **`enum Scope`** (`crates/clawg-core/src/memory/record.rs`) se exporta y nada lo usa. Su destino se decide acá (pregunta 8). Viene del backlog del mapa.
- **secret-hygiene está mergeado (#16), pero no archivado:** un save enmascara los secretos de formato conocido, y existen `clawg secrets` y `clawg purge`. Le falta cerrar su verify (Windows, `rag-eval --real`, perfil dev). El mapa pide que cierre antes de abrir lo compartido.
- **Identidad con mayúsculas:** NTFS no distingue mayúsculas, así que en Windows `Cursor` y `cursor` comparten la carpeta del workspace, aunque en la base sean dos agentes. Si se comparte por identidad, hay que decidir si se normaliza el nombre (ver el verify de visibility-policy).

Antes de escribir la propuesta, revalidar cada `ruta:línea` contra el código de ese momento.

## Preguntas de diseño

Numeradas como en la exploración original.

2. **Política por defecto.** ¿Compartido por proyecto entre agentes, con `agent_id` como autoría? ¿Con listas por agente (allow/deny) para el control del usuario?
3. **Dónde configura el usuario.** ¿Un archivo en el directorio de datos? ¿Un comando `clawg agents ...`? (visibility-policy D1 dejó el archivo de política para esta parte.)
4. **Global entre proyectos (objetivo 2).** ¿Qué tipos cruzan proyectos? Hoy cruzan `user_preference` y las filas sin proyecto. ¿Las decisiones de arquitectura no?
7. **Procedencia de lo compartido.** Hoy cada hit se muestra como `[fecha] SOURCE | TYPE` y `CONTENT` crudo, sin autor (`crates/clawg-core/src/memory/prompt_builder.rs`). Cuando se comparta:
   - el bloque de `recall_context` y `search_memory` tiene que decir **qué agente** guardó cada memoria ajena;
   - el contenido ajeno se presenta como dato, no como instrucción. Una memoria envenenada (por ejemplo, guardada después de leer un README malicioso) no debería llegarle a otro agente como una orden;
   - un secreto que el enmascarado no reconozca viaja igual a otro cliente y a su proveedor.
8. **Compartir con la columna o con una regla.**
   - **Columna:** se pasa la fila a `scope = 'project'` o `'global'`. El invariante queda en la base, pero el CHECK obliga a poner `agent_id` en NULL, así que la autoría (pregunta 7) tiene que salir de `created_by_ref`. Es una migración de datos y lleva su test de preservación (`config.yaml`).
   - **Regla de lectura:** `scope` queda como está y la visibilidad se decide en la consulta, según la política configurada. No toca ninguna fila, pero el invariante vive en código y configuración, no en la base.
   - Principios candidatos, con cualquiera de las dos: una memoria tiene un solo scope (si algo vale para dos, son dos memorias con historia propia), y la promoción entre scopes es sólo manual, nunca automática.
   - Lo que se decida define qué hacer con `enum Scope`: usarlo o borrarlo.
9. **Pesos por scope en el ranking.** Por ejemplo, agente 1,20, proyecto 1,00 y global 0,85: una relación de 1,41x, mayor que todo el rango de importancia (1,18x). Ensanchar la importancia ya bajó R@10 de 0,891 a 0,826 (ver memory-update-tool). Sólo tienen sentido cuando se comparta, y entran únicamente si `rag-eval --diff` lo justifica.

Y una que agregaron A y B:

10. **Scope de lo que modifica, con lo compartido.** Hoy un agente sólo borra, cambia o resuelve lo propio (`specs/visibility` R3 y R4). Si B ve una memoria de A, ¿puede corregirla o borrarla, o sólo A? ¿Y `resolve_conflict` entre una fila propia y una ajena?

## Parte D: prefiltro en vec0

Sólo se justifica si el golden set muestra crowding. Hoy no hay medición que lo diga.

- El filtro de agente y proyecto se aplica **después** de que vec0 corta a k=30, con un solo reintento a 4× (`search.rs`). Cuantas más memorias haya de otros agentes y proyectos, menos lugar queda para las propias. Compartir agrava o alivia esto según la política.
- sqlite-vec ≥ 0.1.6 soporta partition keys y metadata columns en vec0, y el proyecto está en 0.1.9. Tiene límites:
  - las partition keys sólo aceptan TEXT o INTEGER, sin `IN`;
  - metadata no admite NULL, y `project_id` puede serlo, así que hace falta un sentinela;
  - hay que recrear las tablas vec0 y re-insertar los vectores. Es una migración: necesita la red de data-preservation y su test de preservación.
- Un `WHERE` sobre la tabla padre en la misma query del `MATCH` no alcanza: vec0 elige los k vecinos antes del join.

## Cómo se mide

- **Casos multi-agente y multi-proyecto** en el golden set, cada uno con la visibilidad esperada.
- **Un test por regla de la política.**
- **Test de procedencia:** una memoria de A que le llega a B en `recall_context` y en `search_memory` lleva el autor A; una memoria propia no cambia de formato.
- **Si se elige la columna (pregunta 8):** un test que siembra filas `agent` en la versión anterior, las pasa a `project` y prueba que siguen visibles y conservan el autor.
- **Si se agregan pesos por scope (pregunta 9) o el prefiltro (parte D):** `rag-eval --diff` contra el baseline, sobre los casos multi-agente.
