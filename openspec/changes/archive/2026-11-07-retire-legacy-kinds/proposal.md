# retire-legacy-kinds — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M–L | — | ver [`state.yaml`](state.yaml) |

## Problema

`episodic_trajectory` y `procedural_skill` no tienen escritor desde que se fue la app de escritorio, pero cada búsqueda, cada pasada de mantenimiento y cada operación sobre proyectos las sigue leyendo. Detalle verificado en la [exploración](exploration.md#problema-verificado-contra-el-código):

1. **Nadie escribe.** `insert_episodic_trajectory` y `promote_to_procedural_skill` sólo existen para los tests (`crates/clawg-core/src/memory/store_insert.rs:651-659`, `:775`), y `export.rs:41-42` ya lo dice.
2. **Todos pagan.** `SearchScope::all_default` pide cinco filas a cada tabla (`crates/clawg-core/src/memory/search.rs:42-51`), así que cada búsqueda corre tres KNN y, como las dos de más vuelven vacías, dos consultas de elegibles (`search.rs:1399-1412`). Mantenimiento, proyectos (registro, unión, borrado, purga), la exportación, `doctor` y la deshechura de importaciones también las recorren.
3. **Son visibles por fuera.** La API acepta `episodic` y `procedural` (`crates/clawg-cli/src/daemon/api.rs:214-221`), la CLI también (`crates/clawg-cli/src/cli.rs:950-955`) y el contrato del agente las ofrece en cinco herramientas.
4. **Los datos guardan los nombres.** Tres diarios guardan ids de estas tablas: `project_delete_row`, `project_merge_change` (`kind` `trajectory` o `skill`) y `project_rekey`.

En las dos bases contadas no hay ninguna fila ni entrada de diario ([resultado de la puerta](exploration.md#resultado-de-la-puerta-2026-11-03)), pero la base de otra persona puede tenerlas, así que la retirada se escribe sin pérdida.

## Enfoque

El **A** de la exploración: una migración pasa cada fila a `memory` y borra las dos tablas con sus `vec0`, y después se saca todo el código que las leía. Las cuatro [decisiones](exploration.md#decisiones) ya están tomadas y no se reabren:

1. Una fila migrada es `semantic_fact`, con su tabla de origen en `source`.
2. Lo que nombra el id viejo se reescribe al nuevo en la misma transacción.
3. Una fila borrada que ningún diario nombra también pasa, con su `deleted_at` y sin vector.
4. Un `kind` viejo recibe, desde esta versión, un error que dice adónde fueron sus filas, en la API, la CLI y el MCP.

## Alcance

1. **V32** (`V32__retire_legacy_kinds.sql`): cada fila pasa a `memory` con un id nuevo, su texto armado con las columnas que `memory` no tiene, su vector si está viva y su entrada en el índice de texto; se reescriben `superseded_by`, `disputed_against` y los `source_trajectory_ids` de una habilidad; `project_delete_row` y `project_rekey` pasan a nombrar `memory` con el id nuevo, y los cambios `trajectory` y `skill` de una unión pasan a la `memory_move` de esa unión. `legacy_kind_move` guarda de dónde vino cada fila. Después se borran las dos tablas y sus `vec0`. Ningún proyecto cambia de estado. Detalle en el [diseño](design.md#d1-v32).
2. **Núcleo:** búsqueda con un solo KNN; mantenimiento, registro de proyectos, unión, borrado, purga, deshechura de importaciones, `inspect` y espacio de trabajo sólo sobre `memory`; `TableKind` y `MemoryKind` se van (con una sola variante, unificarlos es sacarlos) y las funciones de `admin.rs` y `pieces.rs` dejan de recibir `kind`.
3. **API:** el segmento `{kind}` y el filtro `?kind=` aceptan sólo `memory`; `episodic` y `procedural` dan un 400 que explica adónde fueron sus filas. Las respuestas siguen llevando `kind` (y `GET /v1/stats` sus dos conteos en 0) para un bridge de la versión anterior.
4. **CLI:** `--kind` queda oculto, acepta `memory` y rechaza los otros dos con la misma explicación. `memories stats`, `projects` y `purge` dejan de nombrar trayectorias y habilidades.
5. **MCP:** `kind` sale del esquema de `list_memories`, `delete_memory`, `restore_memory`, `read_memory` y `update_memory`; un `kind` viejo recibe la explicación en el idioma del bridge, sin llamar al daemon. Los dos contratos se bendicen.
6. **Eval:** `rag-eval` deja de pedir y de registrar los dos pools.
7. **Docs:** `docs/reference/` y `README.md`.

## Fuera de alcance

Lo que la exploración pone en [«No-alcance»](exploration.md#no-alcance):

- Cambiar cómo se guardan o clasifican las memorias normales. Por eso tampoco cambia el formato de los hits (`#12 [Memory/…]`): la línea la arma el bridge con el `kind` que manda el daemon.
- Una función nueva de habilidades o trayectorias.

Y lo que esta tarea deja afuera: subir la versión, archivar el cambio, `openspec/README.md`, `openspec/ROADMAP.md` y las specs principales (se consolidan al archivar).

## Datos guardados

- **Esquema:** V32 crea `legacy_kind_move (tbl, old_id, new_id)` y borra `episodic_trajectory`, `procedural_skill`, `episodic_vectors` y `procedural_vectors`. Antes de migrar se escribe la copia `pre-migration`, y todo corre en la transacción agrupada de las migraciones: si algo falla, la base queda como estaba (`a_failing_migration_rolls_back_every_migration_of_its_run`, `crates/clawg-core/src/memory/db.rs:680`).
- **Filas:** ninguna se pierde. Cada columna de una fila vieja queda en una columna de `memory` o en una línea de su texto ([D2](design.md#d2-qué-va-a-cada-columna)). Las filas de `memory` que ya estaban no cambian.
- **Rollback:** un binario anterior rechaza la base en V32 sin tocarla (`SchemaNewer`). Se vuelve con `clawg restore` del snapshot `pre-migration`; lo guardado después de migrar no está en él. No hay receta SQL de vuelta: las tablas se borran, y reconstruirlas desde el texto de las filas migradas no devolvería los ids que les dio la versión anterior.

## Cómo se mide

Los cinco tests en rojo de la [exploración](exploration.md#cómo-se-mide), con nombre en la [spec](spec.md):

- Una base en V31 con filas de los dos tipos abre con la versión nueva, y cada fila viva se encuentra por búsqueda.
- Un proyecto borrado antes de la migración se deshace con la versión nueva y recupera sus trayectorias y habilidades.
- Una unión hecha antes de la migración se deshace con la versión nueva y esas filas vuelven al proyecto de origen.
- Una fila de cada tipo borrada a mano antes de la migración se restaura en la versión nueva, y también se puede purgar.
- Pedir `episodic` o `procedural` a la API, la CLI o el MCP da el error que explica adónde fueron esas filas.

Además, `doctor` y las invariantes pasan sobre una base recién migrada, y ningún archivo de `src/` fuera de los permitidos nombra las tablas.

Pendiente en local, con lo que pide cada paso (en [`verify.md`](verify.md)): la migración sobre una copia de `prod` y otra de `dev`; `search.duration` antes y después sobre una copia (una búsqueda hace dos KNN y dos consultas de elegibles menos); y `rag-eval --diff` con el modelo real y el golden de `docs/eval/`, con el umbral fijado acá: **ningún caso se mueve**, porque el golden no tiene filas de estos tipos y el pool de `memory` no cambia.
