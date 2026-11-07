# retire-legacy-kinds — Diseño

Los enfoques y las cuatro decisiones de fondo están en la [exploración](exploration.md). Acá van las decisiones de implementación.

## D1. V32

Una sola migración SQL, `V32__retire_legacy_kinds.sql`, en la transacción agrupada de las migraciones (`crates/clawg-core/src/memory/db.rs:451-462`) y después del snapshot `pre-migration`. SQL alcanza: el hash del texto lo da la UDF `content_hash`, registrada antes de migrar (`db.rs:434-439`), y refinery no corre código Rust en una migración, sólo el SQL que devuelve. El orden:

1. **Ids nuevos.** `legacy_kind_move (new_id PRIMARY KEY, tbl, old_id, UNIQUE (tbl, old_id))` recibe una fila por fila vieja: las trayectorias por id y después las habilidades, numeradas desde el mayor de `sqlite_sequence` y `MAX(id)` de `memory`. Un id que `memory` dio y después purgó no se vuelve a usar, como no lo reusa `AUTOINCREMENT`, y escribir un id explícito mayor adelanta `sqlite_sequence`.
2. **Proyectos borrados.** Se anotan en una tabla `temp` su `deleted_at` y su `reviewed` (ver [D3](#d3-los-triggers)).
3. **Filas.** Un `INSERT … SELECT` por tabla escribe cada fila migrada viva, con el texto de [D2](#d2-qué-va-a-cada-columna) y los ids que nombra ya reescritos por `legacy_kind_move`. Después, `content_hash = content_hash(content)` sobre las filas migradas.
4. **Vectores.** El de cada fila viva pasa a `memory_vectors` con el id nuevo. El de una borrada no pasa: se calcula al restaurarla, como el de cualquier fila borrada de `memory`.
5. **Borradas.** Un `UPDATE` les pone su `deleted_at`, y los proyectos anotados en el paso 2 vuelven a su `deleted_at` y su `reviewed`.
6. **Pares.** Cada id que nombra el `disputed_against` de una fila migrada es un par `escalated`, con la consulta de V15 (`crates/clawg-core/migrations/V15__memory_conflict.sql:30-34`) limitada a las filas migradas.
7. **Diarios** ([D4](#d4-los-diarios)).
8. **Tablas.** `DROP TABLE` de los dos `vec0` (que se llevan sus tablas sombra), de las dos tablas (que se llevan sus índices) y de la tabla `temp`.

`legacy_kind_move` queda: es lo único que dice qué id tenía una fila en la versión anterior, y no la lee el código, como `project_rekey`, que sólo leen los tests y la receta de su comentario (`crates/clawg-core/migrations/V9__project.sql:12-16`). Se descartó una tabla `temp` para el mapa: borraría el único registro de los ids viejos, y la explicación de un kind retirado dice que las filas tienen otro id.

## D2. Qué va a cada columna

Las columnas comunes (`scope`, `agent_id`, `project_id`, `created_at`, `created_by_type`, `created_by_ref`, `last_modified_at`, `last_modified_by`, `update_count`, `last_accessed_at`, `access_count`, `importance`, `tier`, `status`, `dispute_count`, `last_disputed_at`, `deleted_at`) pasan como están: migrar no es modificar una fila, así que `last_modified_at` y `update_count` no se tocan. `superseded_by` y `disputed_against` pasan reescritos. `memory_type = 'semantic_fact'` y `source = 'episodic_trajectory'` o `'procedural_skill'` (decisión 1); `topic_key`, `revived_from`, `origin_project` y `lang` quedan `NULL` (la pasada de mantenimiento clasifica el idioma, `crates/clawg-core/migrations/V22__memory_lang.sql:4-8`). `supersede_cause` es `verdict` en una reemplazada con reemplazante, la regla de V17 para una fila sin clave (`crates/clawg-core/migrations/V17__revival.sql:20-38`).

`session_id` de una trayectoria va a `source_session_id`. Lo demás va al texto, una línea por columna con su nombre, la primera sin nombre porque es el tema de la fila, como pide el protocolo a un save, y sin la línea de una columna `NULL`:

```
<task>
resolution: <resolution>
root_cause: <root_cause>
tool_calls: <tool_calls>
errors_encountered: <errors_encountered>
success_score: <success_score>
session_id: <session_id>
```

```
<skill_name>
trigger: <trigger>
tool_sequence: <tool_sequence>
success_count: <success_count>
promoted_at: <promoted_at>
source_trajectory_ids: <ids nuevos>
```

`session_id` va también al texto, aunque tiene columna: es lo único que distingue dos trayectorias de la misma tarea, y sin él dos de un proyecto serían filas vivas con un mismo `content_hash`, que `doctor` señala como duplicadas y que un save siguiente reforzaría como una sola.

Un id de `superseded_by`, `disputed_against` o `source_trajectory_ids` que no está en `legacy_kind_move` no nombraba ninguna fila de su tabla, y con el número viejo nombraría una fila cualquiera de `memory`: se quita. Un `disputed_against` que no es JSON válido no nombra nada para ningún lector (`V15__memory_conflict.sql:28-29`) y pasa como está.

El vector que pasa se calculó con el texto viejo (`task`, o `trigger` y `skill_name`), no con el nuevo. Es lo que pide la decisión de pasar el vector, y el primer re-embebido de `memory` lo rehace desde el texto, como a cualquier fila.

## D3. Los triggers

Escribir en `memory` dispara los triggers de V8 (índice de texto) y de V29 (proyectos), y la migración los respeta en vez de esquivarlos:

- **Índice de texto.** `memory_fts_ai` indexa toda fila insertada, borrada o no (`crates/clawg-core/migrations/V8__memory_fts.sql:58-66`), y restaurar vuelve a indexar (`crates/clawg-core/src/memory/admin.rs:568-574`). Una fila borrada insertada como borrada quedaría indexada dos veces al restaurarla. Por eso se inserta viva y después se borra, como una importación (`crates/clawg-core/src/memory/portability/import.rs:1148-1152`): el trigger de borrado le resta lo que el de inserción sumó.
- **Proyectos.** `memory_project_ai` adopta un id que `project` no tiene y trae de vuelta un proyecto borrado cuando llega una fila viva (`crates/clawg-core/migrations/V29__project_delete.sql:55-65`). V27 ya adoptó todo id de las dos tablas (`V27__project_registry.sql:59-69`), así que adoptar no pasa salvo en una base editada a mano. Traer de vuelta sí: cada fila se inserta viva. El paso 5 de D1 le devuelve a cada proyecto que estaba borrado su `deleted_at` y su `reviewed`, como hace la importación con una fila que llega borrada (`import.rs:1153-1158`). Una trayectoria viva bajo un proyecto borrado, que hoy `doctor` ya señala (`crates/clawg-cli/src/doctor/checks.rs:803-806`), sigue así: la migración no cambia el estado de ningún proyecto.

## D4. Los diarios

| Diario | Qué se hace |
|---|---|
| `project_delete_row` | `tbl = 'memory'`, `row_id` = id nuevo. Deshacer y purgar ya leen `memory`, y la fila migrada conserva el `project_id` y el `deleted_at` con los que deshacer la compara (`crates/clawg-core/src/memory/project_delete.rs:790-797`). |
| `project_rekey` | Lo mismo. El `CHECK (tbl IN (...))` de V9 sigue nombrando las tablas viejas: es más ancho de lo que el código escribe, y estrecharlo pide reconstruir la tabla sin que cambie ninguna fila. |
| `project_merge_change` | Cada cambio `trajectory` o `skill` pasa a una fila de `memory_move` de la unión (`from_project = source`, `to_project = target`, el alcance de la fila en los dos lados, `moved_at = merged_at`) y se borra. Una unión sin `move_id` recibe el siguiente libre, como lo da un movimiento (`crates/clawg-core/src/memory/relocate.rs:430`). |

Para las uniones se descartó reescribir el id y dejar los dos kinds: deshacer una unión tendría que seguir moviendo filas de `memory` por fuera de `memory_move`, una segunda forma de mover filas para un caso que sólo escribe una versión anterior. Con la fila en `memory_move`, deshacer la unión la devuelve con el mismo `undo_move_in` que al resto (`crates/clawg-core/src/memory/merge.rs:486-497`): queda donde está si se movió después, y deshacer ese movimiento solo sigue siendo `part_of_merge`.

Una entrada que nombra una fila que ya no existe (purgada) queda como está: no hay a qué id llevarla, y deshacer y purgar ya registran y saltean una tabla que no conocen (`project_delete.rs:777-781`).

## D5. Unificar `TableKind` y `MemoryKind`

Sin las dos tablas a los dos enums les queda una variante. Unificarlos es sacarlos: un enum de una variante sólo obliga a cada llamador a pasar el único valor posible. Las funciones de `admin.rs` y `pieces.rs` dejan de recibir `kind`; `RankedHit` y `MemoryAdminRow` dejan de llevarlo; `PatchError::TypeOnlyOnMemory`, `PatchError::ProjectOnlyOnMemory` y `RestoreError::OnlyMemory` desaparecen, porque ya no hay otra tabla. `EvalKind`, la copia de `eval_api.rs`, también.

## D6. Lo que ve un bridge de la versión anterior

Después de actualizar, el editor puede seguir teniendo abierto un bridge de la versión anterior contra el daemon nuevo. Sus structs exigen `kind` en un hit, una fila (también las de `POST /v1/synthesis`), un `PATCH` y un `/save` (`crates/clawg-cli/src/client.rs:209`, `:265`, `:404-411`, `:528`, `:599`), los dos conteos de `GET /v1/stats` (`client.rs:452-457`) y `trajectories` y `skills` en el `MergeReport` (`crates/clawg-core/src/memory/merge.rs:178-179`), que el bridge lee en la respuesta de `answer_suggestion` (`crates/clawg-cli/src/mcp/server.rs:1651`), la única ruta con una unión que llama; las líneas son las de la versión anterior. El daemon los sigue mandando:

- `kind`, con un envoltorio de `api.rs` que lo agrega a la fila que sale del núcleo (`#[serde(flatten)]`), `Memory` en un hit y `memory` en el resto, como ya lo escribe en `SavedMemory`, `ReplacedMemory` y `RelatedMemory` (`crates/clawg-cli/src/daemon/api.rs:708-724`, `:876`, `:909-914`). La respuesta de `synthesis` pasa por un cuerpo propio con las filas envueltas. El núcleo no lleva un campo constante: lo que pide el protocolo vive en el daemon.
- `totals.episodic` y `totals.procedural` en 0.
- `trajectories` y `skills` en 0 en la unión que hace una respuesta `merge` a una sugerencia. Las otras rutas que devuelven una unión (`POST /v1/projects/merge`, `POST /v1/projects/locate`) las llama la CLI, que es la versión nueva.

`success_count`, `trigger` y `source_trajectory_ids` salen de la fila: el bridge anterior los lee con `#[serde(default)]`. El formato de la línea de un hit (`#12 [Memory/…]`) no cambia, porque lo arma el bridge con el `kind` que manda el daemon y cambiarlo movería los tokens que mide `rag-eval --agent-view`.

## D7. Un kind retirado

Un `kind` viejo puede llegar de un bridge anterior, que lo manda sólo si el agente lo elige (`client.rs:1310-1312`), o de un script. Recibe la explicación desde esta versión (decisión 4):

- **API:** `parse_kind` acepta `memory`; `episodic` y `procedural` dan un 400 con la explicación; cualquier otro, un 400 que dice que `memory` es el único valor. No un 404: el bridge lo lee como un daemon que se fue (`client.rs:102-104`). No un 410: un 400 es lo que ya daba un kind que no se acepta, y el bridge anterior pasa al agente el texto de cualquier error del daemon (`client.rs:64-66`).
- **CLI:** `--kind` queda oculto con un `value_parser` propio que acepta `memory` y devuelve la explicación para los otros dos, así `clap` la muestra como error de uso en vez de su lista de valores posibles.
- **MCP:** `kind` sale del esquema (`#[schemars(skip)]`) pero se sigue leyendo (`#[serde(default)]`). Sin él, un agente que manda `delete_memory` con `kind: episodic` haría borrar la fila de `memory` de ese número; con él, el bridge responde la explicación, en su idioma, sin llamar al daemon. Se descartó dejar `kind` en el esquema con un solo valor: el modelo pagaría en cada `tools/list` un parámetro que no elige nada.

## D8. Búsqueda, mantenimiento y lo demás

- **Búsqueda.** `SearchScope` pierde `episodic` y `procedural`, y `search_all_memories` corre un KNN (`knn_memory`) y el léxico. `format_pool_survival` y `format_skill_hits` se van con lo que contaban, y `search.knn_retry` sólo registra `table=memory`. `ScoringConfig` pierde las dos tasas de decaimiento por tabla.
- **Mantenimiento.** La baja de tier, el barrido de vectores huérfanos, el conteo del corpus y el re-embebido corren sólo sobre `memory`; `ReembedTable` y `skill_embedding_text` se van.
- **Proyectos.** El registro sólo junta filas de `memory` (con `merge_in`); `project_rekey` ya no se escribe. Unión, borrado, purga y la deshechura de importaciones cuentan y mueven sólo `memory`; `Counts`, `MergeReport`, `IdRows` y los informes de la CLI pierden trayectorias y habilidades, y `Kept` y `LiveRow` su `table`. El código de un borrado deja de incluir las dos tablas: un código mostrado antes de actualizar no coincide después y el borrado lo dice con `code_changed`, que es lo que pasa con cualquier cambio del conjunto.
- **Espacio de trabajo.** La sección AUTO deja de imprimir las cinco habilidades más usadas: como `semantic_fact` sólo salen por búsqueda (lo aceptado en la decisión 1).
- **Exportación.** Una base anterior a V32, que la exportación lee sin el lock (`docs/reference/commands.md:13`), todavía tiene las tablas: si existen, las cuenta en `legacy_rows` y la CLI dice que no viajan; en V32 no existen y no se leen.

## D9. Cómo se prueba lo de la versión anterior

El código que escribía las filas viejas se va con este cambio, así que los tests siembran la base en V31 por SQL (`seed_on_disk`, `crates/clawg-core/src/memory/db.rs:512-540`, y después las filas), con lo que escribían `delete_core` y `merge_pass` de esta misma versión (`project_delete.rs:517-532`, `merge.rs:274-281` y `:349-357`), y la abren con `MemoryStore::open`, que migra. `doctor` se prueba sobre el binario: una copia instalada congelada en V31 (`migrations_up_to`, `crates/clawg-cli/tests/binary.rs:2726-2739`) crea la base, se siembra por SQL, y el binario nuevo la abre.
