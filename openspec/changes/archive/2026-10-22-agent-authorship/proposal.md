# agent-authorship — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2, 4 | M–L | nada pendiente (visibility-policy, agent-sharing, topic-revisions, memory-provenance y protected-verdicts archivados) | ver [`state.yaml`](state.yaml) |

## Problema

`agent_id` hace dos trabajos: dice quién escribió una fila y decide qué puede hacer con ella cada agente. El detalle, con cada lugar citado, está en la [exploración](exploration.md#problema-verificado-contra-el-código). En resumen:

1. **Un agente ve lo ajeno y no puede trabajar con ello.** `search_memory` le muestra la fila de otro agente, pero `read_memory` la responde como inexistente (`read_save_core`, `crates/clawg-core/src/memory/pieces.rs:107-113`), y `update_memory`, `delete_memory`, `restore_memory` y `resolve_conflict` la rechazan (`UPDATE_READ_*`, `OWNED_ELSEWHERE_*` y `RESTORE_READ` en `crates/clawg-core/src/memory/admin.rs`; `SELECT_PAIR_ROW` en `crates/clawg-core/src/memory/dispute.rs`). Una fila sin autor queda fuera de todos, porque `agent_id = ?` es falso con NULL.
2. **Las reglas que comparan un save con lo guardado miran sólo lo propio.** La dedup (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs`), el detector (`Reach::OwnOnly`, `crates/clawg-core/src/memory/contradiction.rs`), la clave de tema (`SCOPE_ROWS_SQL`, `crates/clawg-core/src/memory/topic.rs`, y el revivir de `revival.rs`) y `synthesize` (`PENDING` y el alcance, `crates/clawg-core/src/memory/synthesis.rs`) filtran por agente. El mismo texto guardado por dos agentes queda dos veces, y una clave revisada por otro agente deja dos versiones vigentes.
3. **history-import (44) lo necesita:** trae miles de notas bajo el autor `migrated`, más de la mitad más largas que una pieza, que hoy el agente del usuario sólo vería como fragmentos.

## Decisiones

Tomadas antes de la exploración (ver [«Decisiones tomadas»](exploration.md#decisiones-tomadas)):

- `agent_id` dice quién escribió una fila y nunca quién puede leerla o cambiarla.
- Cualquier agente lee entera, cambia, borra, restaura y juzga toda fila que alcanza, las sin autor incluidas.
- Dedup, detector, clave de tema (con el revivir al borrar) y síntesis se aplican por proyecto, no por agente y proyecto.
- Siguen por agente el `workspace.md`, RECENT, la sesión registrada y el aviso del hook.
- El autor se sigue mostrando (`by <autor>`).

Las [decisiones abiertas](exploration.md#decisiones-abiertas) se cierran con las opciones recomendadas:

1. **`sharing.json` acota también lo que un agente modifica (1.a).** Lo que un agente alcanza (`Reach`) es el único criterio para leer y para modificar, y las reglas que comparan un save se aplican al proyecto *dentro del alcance del que guarda*. Nada que la política le oculta se imprime en `replaced`, `related`, ACTIVE CONFLICTS ni en el 400 de `derived_from`. Con `clawg sharing hide '*' '*'` vuelve el aislamiento de antes, ahora también al modificar. Un `sharing.json` ilegible deja a cada agente con lo suyo y lo que no tiene autor, como ya hace la búsqueda.
2. **`?agent=` y los `agent_id` del cuerpo se siguen aceptando y nombran al lector (2.a).** Sin ellos la operación es del usuario y no tiene frontera, como hoy. Un puente anterior sigue funcionando sin un 400.
3. **Los pares se cuentan por proyecto (3.a).** `memory_status` cuenta los pares cuya fila nueva alcanza el proyecto del puente (el predicado de proyecto de la búsqueda; sin proyecto, todo el store) y cuyos dos lados alcanza el lector; `GET /v1/disputes` suma `project_id`. ACTIVE CONFLICTS quita la condición de agente, conserva la de mostrar sólo pares de filas del prompt y exige que el lector alcance los dos lados. `unreviewed` y `conclusions_to_review` se cuentan por proyecto.
4. **`list_memories` lista todo lo que alcanza, lo más nuevo primero, con ` by <autor>` en lo ajeno y un argumento opcional `agent` (4.a).** `GET /v1/memories` suma `reader` (el alcance) junto al `agent` de siempre (el filtro de autor).
5. **El doctor conserva `memory.topic` y `memory.content_hash` y agrupa por proyecto (5.a)**, nombrando a los autores en `agents`. El hallazgo `fallback_agent` se queda con otro texto.
6. **Sin guarda de `user_answered` para borrar o cambiar la importancia (6.a).** Borrar es un soft delete que `restore_memory` y `undelete` deshacen.

## Alcance

1. `Reader` (agente y alcance) en `crates/clawg-core/src/memory/sharing.rs`, con un único lector de la política para el daemon y el worker. Los núcleos que hoy reciben `owner: Option<&str>` reciben `Option<&Reader>`: `read_save_core`, `update_memory_core`, `delete_memory_core`, `restore_memory_core`, `judge_pair`, `resolve_dispute_core`, `existing_memory_ids` y `replaced_memory_ids`.
2. Las reglas por proyecto dentro del alcance del que guarda: dedup (y la gemela al restaurar), detector (sin `OwnOnly`, también al restaurar), clave de tema (el titular al restaurar y el revivir al borrar) y síntesis (pendientes, conclusiones, revisión y recordatorio por proyecto). El import aplica las suyas por proyecto, sin alcance: es del usuario.
3. Migración V20: `idx_memory_dedup` e `idx_memory_topic` empiezan por `COALESCE(project_id, '')`.
4. Las vistas: ACTIVE CONFLICTS, los pares de `memory_status`, `list_memories` con el autor y `agent`, la clave visible en cualquier hit del proyecto donde guardaría el save, y la línea `shared:`.
5. Lo que ve el modelo: las descripciones de `delete_memory`, `restore_memory`, `read_memory`, `update_memory`, `resolve_conflict` y `list_memories`, `SHARED_NOTE`, `protocol.md` (la clave nombra un tema del proyecto) y `agent_contract.json` bendecido.
6. El doctor: `memory.topic`, `memory.content_hash` y el texto de `fallback_agent`.
7. Las specs que fijan la regla de hoy y `.claude/CLAUDE.md`.

## Fuera de alcance

- Mover filas entre proyectos (project-reassign, 77).
- Adoptar filas en un agente, renombrar autores o un alias de identidades.
- Un mecanismo de restricción nuevo (grupos, permisos de escritura, escritores en `sharing.json`).
- Reconciliar lo que una base existente tiene repetido entre agentes: sólo el doctor lo dice.
- Rellenar pares entre filas viejas de agentes distintos.
- Una guarda de `user_answered` para borrar.
- El ranking y la búsqueda: `rag-eval` no aplica.

## Datos guardados

- **Esquema:** V20 reemplaza dos índices. Ninguna fila cambia.
- **Antes de migrar** se escribe el snapshot `pre-migration` (`crates/clawg-core/src/memory/db.rs`).
- **Rollback:** un binario anterior rechaza la base en V20 sin tocarla (`SchemaNewer`). Se vuelve con `clawg restore` del snapshot `pre-migration`, o con:

  ```sql
  DROP INDEX idx_memory_dedup;
  CREATE INDEX idx_memory_dedup ON memory (agent_id, COALESCE(project_id, ''), content_hash, created_at)
      WHERE deleted_at IS NULL AND content_hash IS NOT NULL;
  DROP INDEX idx_memory_topic;
  CREATE INDEX idx_memory_topic ON memory (agent_id, COALESCE(project_id, ''), topic_key)
      WHERE deleted_at IS NULL AND topic_key IS NOT NULL;
  DELETE FROM refinery_schema_history WHERE version = 20;
  ```

  Lo que las reglas nuevas hicieron mientras tanto (una fila reforzada por otro agente, una versión reemplazada por la clave de otro agente) queda: son filas y estados que la versión anterior ya sabe leer.
- **Una base existente** puede tener dos titulares vivos de una clave o dos filas vivas de un mismo texto en un proyecto, de agentes distintos. Siguen vivas y alcanzables: el próximo save con esa clave reemplaza a las dos, y el doctor las marca. `synthesis_review` por agente no se toca; la revisión nueva se escribe por proyecto (`agent_id = ''`), así que el primer `recall_context` puede avisar una vez de lo pendiente hasta el primer `synthesize`.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)), con dos agentes A y B:

- B lee entero un save largo de A y una fila sin autor; cambia, borra, restaura y juzga filas de A y sin autor; con `hide B→A` todo eso es «no existe» y no cambia nada;
- B guarda el texto de A en el proyecto y refuerza la fila de A; en otro proyecto, o con `hide`, se inserta;
- el save con clave de B reemplaza al titular de A y lo lista en `replaced`; borrar el de B revive el de A;
- el detector empareja el save de B con una fila de A y con una sin autor; con `hide`, no;
- `synthesize` de B lista las pendientes de A, una conclusión de B las cubre para los dos, y la revisión es del proyecto;
- ACTIVE CONFLICTS muestra a B un par entre A y B; con `hide`, no imprime el lado oculto;
- `list_memories` lista lo ajeno con ` by <autor>` y `agent` filtra;
- la base sembrada en V19 abre con los índices nuevos, cada fila igual y alcanzable, y el plan de la dedup y de la clave los usa;
- las descripciones, `SHARED_NOTE` y `protocol.md` no dicen que lo ajeno es de sólo lectura;
- el doctor agrupa `memory.topic` y `memory.content_hash` por proyecto y nombra a los autores.

Siguen pasando sin tocarlos los tests del workspace, RECENT, la sesión registrada y el aviso del hook. `rag-eval` no aplica; `conflict-eval` (un solo agente) da los mismos números.
