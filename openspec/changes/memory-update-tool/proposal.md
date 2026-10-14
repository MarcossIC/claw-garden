# memory-update-tool — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | visibility-policy (scope, archivado) | ver [`state.yaml`](state.yaml) |

## Problema

El agente fija la importancia y el tipo al guardar, pero después no los puede corregir. Detalle en la [exploración](exploration.md); verificado de nuevo contra `main` (35e448c):

1. **No hay herramienta MCP para cambiar una memoria.** Las ocho que existen son `save_memory`, `search_memory`, `recall_context`, `list_memories`, `delete_memory`, `resolve_conflict`, `synthesize` y `memory_status` (`crates/clawg-cli/src/mcp/server.rs:654-784`). Si el usuario dice "eso no es tan importante" o "eso es una preferencia mía, no un dato", el agente sólo puede borrar y volver a guardar, y pierde los accesos, el tier ganado, las disputas y la procedencia de la fila.
2. **`PATCH /v1/memories/{kind}/{id}` sólo acepta `importance`** (`crates/clawg-cli/src/daemon/api.rs:394-396, 885-902`) y no la valida: la guarda tal como llega (`crates/clawg-core/src/memory/admin.rs:141-188`). Un `PATCH` con `importance: 50` deja la fila fuera de la escala 0–10 que el save sí recorta (`crates/clawg-core/src/memory/store_insert.rs:113-121, 147-151`), que `compute_tier` usa para el atajo de `>= 9` (`crates/clawg-core/src/memory/tier.rs:28-30`) y que el puntaje multiplica (`crates/clawg-core/src/memory/score.rs:302`).
3. **Bajar la importancia nunca baja el tier.** El `UPDATE` recalcula con `compute_tier`, que sólo sube (`tier.rs:15-26`, `admin.rs:131-134`). Una memoria guardada con 9,5 queda en tier 1, y por eso en el workspace (`crates/clawg-core/src/memory/workspace.rs:260`), aunque el usuario la baje a 2. Recién el decaimiento la saca, a los 90 días sin acceso (`tier.rs:80-90`), y hasta hoy no la sacaba nunca, porque con 9,5 no decae.
4. **El `PATCH` no deja rastro.** Escribe `last_modified_at` pero no `update_count`, que existe en las tres tablas desde V1 (`crates/clawg-core/migrations/V1__init.sql:19, 78, 129`) y que nada escribe.
5. **Re-guardar no corrige.** Un save con el mismo contenido sólo refuerza la fila (`store_insert.rs:280-328`): suma un acceso y no toca importancia ni tipo. La respuesta ya dice el id (`already stored as #N`, `server.rs:398-400`), pero no cómo cambiarla.

## Decisiones (cierran las abiertas de la exploración)

- **Campos editables: `importance` y `memory_type`.** El tipo sólo existe en `memory`; en `episodic` y `procedural` se rechaza. Cambiar el tipo cambia el decaimiento (`score.rs:221`, se lee en cada búsqueda) y puede cambiar el tier (D2 del diseño). El contenido no se edita: implica re-embed, dedup y la sonda de contradicción, y eso ya es "guardar el nuevo y borrar el viejo".
- **Tier: baja sólo lo que quitó el cambio** (pregunta al usuario en el hilo, opción recomendada). Si el campo que cambió era lo que le daba el tier a la fila, el tier vuelve al que la fila gana sin él; lo que ganó por accesos, lo que perdió por decaimiento y un tier que vino de otro lado (filas de la app de escritorio) no se tocan. Subir la importancia o pasar a un tipo de tier 1 promueve, como hoy.
- **Scope: el de visibility-policy.** El puente manda siempre su agente, y una fila de otro agente es "no existe" (`specs/visibility` R3 y R4). El `PATCH` ya acepta `?agent=`.
- **Re-guardar sigue sin cambiar la importancia.** El refuerzo es una señal de frecuencia; mezclarlo con una corrección haría que un agente que re-guarda por costumbre pise lo que el usuario fijó. La respuesta del refuerzo nombra `update_memory`.
- **Promoción de scope (agente → proyecto o global): fuera.** Es la pregunta 8 de visibility-policy y vive en [agent-sharing](../agent-sharing/exploration.md).

## Alcance

1. **Core** (`admin.rs`, `tier.rs`): `update_memory_core` reemplaza a `set_importance_core`. Acepta importancia y tipo, valida (escala 0–10, tipo conocido, tipo sólo en `memory`, al menos un campo), recalcula el tier con la regla de arriba, suma `update_count` y devuelve cómo quedó la fila. Todo en una transacción.
2. **HTTP**: `PATCH /v1/memories/{kind}/{id}` acepta `{importance?, memory_type?}`, rechaza con 400 lo inválido y responde además `importance`, `memory_type` y `tier` de la fila.
3. **MCP**: herramienta `update_memory(kind, id, importance?, memory_type?)` con el agente que llama. La respuesta del refuerzo en `save_memory` y la descripción de `save_memory` la nombran.

## Fuera de alcance

- Editar el contenido.
- Mover una memoria de agente a proyecto o global.
- Volver a correr el detector de contradicciones al cambiar el tipo. Una fila que pasa a un tipo que afirma algo queda sin disputas hasta su próximo save parecido; una disputada que pasa a uno que no afirma sigue disputada hasta resolverla. Es la misma regla de hoy: el detector corre al guardar.
- Cambiar el ranking: los pesos, el rango de importancia y los pisos de tier no cambian. El sweep de la exploración ya mostró que ensanchar el rango empeora la recuperación.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base, con los tests nuevos justificados en `verify.md`. Tests que fallan hoy:

- bajar la importancia de una fila que estaba en tier 1 sólo por ella la baja al tier que gana sin ella; una que lo ganó por accesos o por su tipo se queda;
- cambiar el tipo de `memory` cambia la columna y el tier con la misma regla; en `episodic` y `procedural` es 400 y no cambia nada;
- una importancia fuera de 0–10, un tipo desconocido o un cuerpo vacío son 400 y no cambian nada;
- cada cambio suma 1 a `update_count`;
- `update_memory` cambia una fila propia, falla como "no existe" con una de otro agente, y su respuesta dice cómo quedó.

No toca el ranking ni el prompt: el golden set de `rag-eval` no usa `PATCH`. Igual se corre `rag-eval --real --diff` en la verificación y se espera diff vacío.

## Datos guardados

- **Esquema:** no cambia. `update_count` ya existe en las tres tablas.
- **Estados e identificadores:** no cambian. El cambio sólo escribe `importance`, `memory_type`, `tier`, `last_modified_at` y `update_count` de la fila que se pidió, y sólo cuando se pide.
- **Datos de la versión anterior:** un test siembra con SQL directo filas de las tres tablas como las deja la versión actual (y las de la app de escritorio, con tiers que ninguna regla de hoy da) y prueba que `update_memory` las cambia y que un tier que no salió del campo cambiado se queda.
- **Rollback:** volver al binario anterior no necesita nada; la base es la misma. Lo que se cambió con `update_memory` queda cambiado, como con el `PATCH` de hoy.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| El agente cambia importancias sin que el usuario lo pida | Media | La descripción de la herramienta dice que es para cuando el usuario corrige; cada cambio queda en `update_count` y `last_modified_at`, que muestra el visor |
| Un tier que el usuario esperaba conservar baja | Baja | Sólo baja el que daba el campo cambiado; accesos, decaimiento y tiers de otra fuente quedan (spec R3) |
| Un cliente del `PATCH` mandaba importancias fuera de escala | Baja | El único llamador es `DaemonClient` (tests); el visor no escribe. Un 400 dice qué está mal |
| Un daemon viejo recibe el `PATCH` nuevo | Baja | Ignora `memory_type` (o da 422 si sólo viene ese campo) y responde sin `tier`; el cliente exige esos campos, así que el puente informa un error en vez de decir que cambió algo. Se corrige al reiniciar el daemon |
