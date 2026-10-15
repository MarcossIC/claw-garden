# memory-provenance — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 4, 5 | S | — (lo usan 24 y 45, y lo produce 44; ver «Interacciones») | exploración (borrador) |

**Tesis:** cada memoria guardada por MCP queda sellada como dicha por el usuario, aunque el agente la haya inferido y aunque sea una conclusión de `synthesize`. La base afirma, fila por fila, algo que no sabe. Las columnas que describen el origen existen desde la primera migración, pero nadie las lee salvo el listado. Saber quién afirmó cada cosa es justo lo que ayuda a decidir en tres lugares:

- un conflicto entre lo que dijo el usuario y lo que dedujo un agente;
- una conclusión que se confunde con un hecho;
- una nota importada que se confunde con una guardada en sesión.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| Las columnas de origen existen | **Cierta.** En `memory`: `source TEXT NOT NULL DEFAULT 'agent'` (`crates/clawg-core/migrations/V1__init.sql:12`), `created_by_type TEXT NOT NULL DEFAULT 'agent'` (`:14`), `created_by_ref` (`:15`), `source_session_id` (`:16`) y `last_modified_by` (`:18`). No tienen CHECK, y ninguna migración posterior las toca. |
| Todo save por MCP se sella `explicit` / `user` | **Cierta.** El handler arma el evento con `role: Role::User` («Explicit saves are attributed to the user») y `source: MemorySource::Explicit` (`crates/clawg-cli/src/daemon/api.rs:715-730`). El insert fija `CreatedByType::User` para todo `Explicit` (`crates/clawg-core/src/memory/store_insert.rs:127-154`). `created_by_ref` repite `agent_id` (`store_insert.rs:185`). El DEFAULT del esquema (`'agent'`) no coincide con lo que se escribe. |
| Una conclusión de `synthesize` no se distingue | **Cierta.** Es un `save_memory` con `derived_from` (`crates/clawg-cli/src/mcp/tools.rs:187-194`) y queda `explicit` / `user` como cualquier otro. Lo único que la marca son sus filas en `memory_derivation` (`crates/clawg-core/src/memory/synthesis.rs:38-39`). |
| Nadie lee el origen | **Cierta.** Ni el puntaje, ni el detector, ni `resolve_conflict`, ni la síntesis lo consultan. Sólo se seleccionan y se muestran: `admin.rs:571`, `MemoryRow` en `admin.rs:399-401` y `:670-672`, y el visor imprime «created by» y «source» (`crates/clawg-cli/src/memories.rs:180-194`). El texto de las herramientas MCP no los muestra (`tools.rs`). |
| `resolve_conflict` no tiene precedencia por origen | **Cierta.** El agente elige `keep` y `other` (`crates/clawg-cli/src/mcp/server.rs:268-278`) y `resolve_dispute_core` sólo valida que sean distintos (`crates/clawg-core/src/memory/dispute.rs:198-205`). |
| El agente no puede declarar el origen | **Cierta.** `save_memory` recibe `content`, `memory_type`, `importance`, `project_path` y `derived_from` (`server.rs:123-146`). No hay forma de distinguir «el usuario lo dijo» de «lo inferí». |
| Hay valores reservados sin productor | **Cierta.** `MemorySource::Compaction` y `CodeFile` existen «reserved; nothing emits it yet» (`crates/clawg-core/src/memory/events.rs:55-57`). |

## Problema (verificado contra el código)

- **La base miente sobre su contenido.** «El usuario lo afirmó» está escrito en filas que el usuario nunca vio. Una conclusión sintetizada a partir de tres memorias queda igual que una preferencia que el usuario dictó.
- **Las decisiones que dependen del origen no tienen el dato:**
  - **Un conflicto.** Si el usuario dijo «usamos Postgres» y un agente infirió «parece que usan SQLite» de un archivo viejo, la del usuario es la que tiene que ganar, o al menos ir primero. Hoy las dos son `user`.
  - **Una conclusión.** `synthesize` la lista entre las conclusiones, pero `search_memory` y `recall_context` la muestran como un hecho más. El agente no sabe que es una generalización.
  - **Una importación** (44). Sin un valor propio, miles de notas importadas quedan selladas como dichas en sesión por el usuario.
- **El dato es barato hoy, y caro después.** Cada fila que se guarda sin origen real es una fila que nunca lo va a tener. Las anteriores a este cambio quedan ambiguas para siempre.

## Lo que ya existe y se reutiliza

- Las columnas, sin migración: los valores nuevos son texto, y no hay CHECK que los rechace.
- `CreatedByType` (`User`, `Agent`, `System`) y `MemorySource` (`Explicit`, `Chat`, `Compaction`, `CodeFile`) en el núcleo (`store_insert.rs:164-169`; `events.rs`).
- La derivación de `created_by_type` en `NewMemoryRecord::from_event_and_chunk` (`store_insert.rs:127-190`): es pura y testeable.
- `derived_from` ya llega validado al handler (`api.rs:692-704`). El daemon sabe, sin preguntar, que un save con fuentes es una conclusión.
- El visor ya imprime las columnas (`memories.rs:180-194`): los valores nuevos se ven sin trabajo extra.

## Enfoques posibles

### A. El daemon sella sólo lo que sabe

- Un save con `derived_from` → `source = 'synthesis'`, `created_by_type = 'agent'`.
- Un save normal por MCP → `created_by_type = 'agent'`: el agente decidió guardarlo, y eso es lo único seguro.
- El importador (44) → `source = 'import'`.
- **A favor:** sin argumento nuevo y sin costo de contexto. Deja de afirmar lo que no sabe.
- **En contra:** pierde el único caso que hoy está bien: el usuario que dicta algo y el agente lo guarda tal cual.

### B. A más un argumento opcional en `save_memory` para declarar que el usuario lo dijo (recomendado)

- `user_stated: true` (nombre a decidir) cuando el contenido es lo que el usuario dijo, no una inferencia. Sin el argumento, vale A.
- **A favor:** recupera el caso que A pierde, y es lo que necesita la precedencia en conflictos.
- **En contra:** un argumento más en el esquema de una herramienta que se paga en cada sesión. Además, el agente puede marcarlo de más: es una declaración, no una prueba. memory-protocol (22) tiene que decir cuándo usarlo.

### C. Deducirlo del tipo

`user_preference` ⇒ el usuario.

- Descartado: una preferencia también se infiere («el usuario parece preferir X»). Mezclar tipo y origen hace que ninguno de los dos signifique lo que dice.

### Usos (se decide cuáles entran)

1. **Conflictos (24).** El origen como entrada de `needs_user` y del orden de los pares: entre una afirmación del usuario y una inferencia del agente, la inferencia es la candidata natural a quedar `superseded`. Nunca como resolución automática.
2. **Síntesis (45).** `synthesize` puede decir de qué tipo es cada fuente, y una conclusión se marca como conclusión al mostrarse.
3. **Líneas de búsqueda.** Una marca corta (`concl`, `import`) en `search_memory` y `recall_context`. Cuesta tokens en cada hit y hay que medirla con `rag-eval --agent-view`.
4. **Export e import (43).** El origen viaja y se conserva.

## Descartado

| Opción | Razón técnica |
|---|---|
| Reescribir las filas anteriores como `agent` | Sería adivinar: algunas sí las dijo el usuario. Se dejan como están y se documenta que el origen anterior a este cambio no es confiable. |
| Una precedencia automática que resuelva conflictos por origen | Un usuario puede cambiar de idea, y la inferencia nueva puede ser la correcta. El origen ordena y sugiere, pero no decide. La regla de no ocultar memorias sin permiso sigue valiendo. |
| Un CHECK sobre los valores | Obliga a una migración que recrea la tabla, y una versión anterior que escriba un valor viejo la rompería. Alcanza con validar en el código. |
| Usar `created_by_ref` para el autor real | Hoy repite `agent_id` (`store_insert.rs:185`), y el autor real de una fila ya es su `agent_id` (`specs/sharing`). No agrega nada. |

## Interacciones

- **conflict-candidates (24).** Es el consumidor principal (uso 1). El veredicto lo sigue eligiendo el agente o el usuario.
- **memory-protocol (22).** Si entra B, el protocolo dice cuándo declarar que el usuario lo dijo: una cita textual o una instrucción explícita, nunca una deducción.
- **quality-guards (32).** Si entra B, el esquema de `save_memory` cambia, y se ve como diff a propósito en el snapshot del contrato.
- **history-import (44).** Produce `source = 'import'`.
- **conclusion-upkeep (45).** Usa el sello de conclusión.
- **session-handoff (28).** Escribe `source_session_id`, otra columna de origen que hoy no tiene escritor. Son independientes, pero comparten la regla: el daemon sella, el modelo no inventa.
- **topic-revisions (27).** Decisión: ¿una revisión hereda el origen de la anterior, o lleva el suyo? Lleva el suyo, porque la revisión la escribió quien la escribió.

## Alcance candidato

1. Una regla pura de sello: (evento, `derived_from`, declaración) → (`source`, `created_by_type`), con su tabla de casos.
2. Un save con `derived_from` se sella como conclusión. Un save normal, según A o B.
3. Si entra B, el argumento opcional en `save_memory`, `SaveRequest` y `MemoryEvent` con `#[serde(default)]`.
4. Los usos elegidos de la lista anterior.
5. Docs: `.claude/CLAUDE.md` (herramienta y API) y una spec nueva de origen, o una sección en `specs/ingest`.

### No-alcance

- Reescribir el origen de las filas existentes.
- Resolver conflictos de forma automática.
- `last_modified_by`, que no tiene escritor ni lector (lo trata 28).

## Decisiones abiertas

Son del usuario.

- [ ] **Enfoque.** ¿A (sólo lo que el daemon sabe) o B (más la declaración del agente, recomendado)?
- [ ] **Nombre y forma de la declaración** en B: ¿un booleano, o un valor de origen?
- [ ] **Usos.** ¿Cuáles de los cuatro entran en este cambio? Se recomiendan 1 y 2. El 3 se mide antes.
- [ ] **Valores.** ¿`source` gana `synthesis` e `import`, o la conclusión se marca sólo con `created_by_type`?
- [ ] **Filas anteriores.** ¿Se dejan como están (recomendado) y se documenta?

## Cómo se mide

Tests RED primero:

1. Un save normal por MCP deja `created_by_type = 'agent'`. Hoy deja `user`.
2. Un save con `derived_from` queda sellado como conclusión.
3. Con B, un save declarado queda `user`, y el esquema de la herramienta nombra el argumento.
4. Un evento sin el campo (spill o cliente viejo) se sella como hoy, sin fallar.
5. Una base sembrada en la versión anterior abre con todas sus filas, y su origen sin cambios. No hay migración, pero la regla de `openspec/config.yaml` pide probar que los datos guardados siguen alcanzables.
6. Si entra el uso 1: en un par, lo dicho por el usuario va antes que lo inferido, y ninguno cambia de estado por eso.

`rag-eval`: sin `--diff` si sólo cambian las columnas. Si entra el uso 3, `rag-eval --real --agent-view` antes y después: el `found` no baja, y se reporta el aumento de tokens.

## Riesgos

- **Declaraciones de más.** Un agente que marca todo como dicho por el usuario vuelve al estado de hoy. Lo mitiga el texto del protocolo. No hay forma de verificarlo.
- **Costo de contexto** del argumento (B) y de la marca en las líneas (uso 3).
- **Un valor nuevo que una versión anterior no conoce.** Las columnas son texto libre y sólo se muestran, así que una versión anterior las imprime tal cual.
