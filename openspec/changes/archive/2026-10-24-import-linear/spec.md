# import-linear — Especificación

Suma requisitos a [`specs/portability`](../../../specs/portability/spec.md) (R1 y R5, como IL1 e IL5) y a [`specs/storage`](../../../specs/storage/spec.md) (R2 y R3, como IL2 e IL3). R4 no suma ninguno: lo que la importación hace sigue en R3 a R7 y HI2 a HI18 de `specs/portability`.

## R1. Las búsquedas de la importación llegan a sus filas por índice

Toda sentencia que la importación (`crates/clawg-core/src/memory/portability/import.rs`) ejecuta por fila o por save, y que busca por una columna o por el rowid, MUST tener en `EXPLAIN QUERY PLAN` una línea `SEARCH` que nombre su índice, y MUST NOT tener ninguna línea `SCAN` que recorra una tabla. `SCAN CONSTANT ROW`, la fila única de un `SELECT EXISTS (…)` sin `FROM`, no recorre ninguna tabla. Esto vale sin estadísticas del planificador. Ejecutada contra un store con filas, ninguna de esas sentencias MUST dar un paso de recorrido completo (`StatementStatus::FullscanStep` igual a 0). El índice que corresponde a cada una:

| Sentencia | Índice |
|---|---|
| `ORIGIN_SQL` | `idx_memory_import_origin` |
| `PRESENT_SQL`, con proyecto y sin él | `idx_memory_dedup` |
| `TWIN_SQL`, con proyecto y sin él | `idx_memory_dedup` |
| `TOPIC_HOLDERS_SQL` | `idx_memory_topic` |
| `REVISED_SQL` | la clave primaria de `memory` (`INTEGER PRIMARY KEY`) |
| las dos consultas de `memory_piece` de `write_links` (si la fila ya es una pieza, y el próximo `save_id`) | `idx_memory_piece_memory` y la clave primaria de `memory_piece` |

`PRESENT_SQL` MUST seguir encontrando la fila con el agente, el proyecto, el hash y la fecha dados, esté borrada o no.

#### Escenario: la fila presente, por índice
- GIVEN un store migrado y con filas
- WHEN se pide el plan de `PRESENT_SQL` con un agente, un proyecto (o `NULL`), un hash y una fecha
- THEN una línea es `SEARCH memory USING INDEX idx_memory_dedup (<expr>=? AND content_hash=? AND created_at=?)` y ninguna recorre una tabla
- AND al ejecutarla no da ningún paso de recorrido completo

#### Escenario: las demás búsquedas
- GIVEN el mismo store
- WHEN se pide el plan de cada una de las otras sentencias de la tabla y se la ejecuta
- THEN cada plan nombra el índice de su fila de la tabla, ninguna línea recorre una tabla y ninguna ejecución da un paso de recorrido completo

**Test:** `the_import_lookups_seek_an_index` (`import.rs`, mod tests). Antes del cambio fallaba sólo en `PRESENT_SQL`, con y sin proyecto: el plan era `SCAN memory` y su ejecución daba 16 pasos de recorrido completo sobre un store de 17 filas. Las demás sentencias ya buscaban por su índice.

## R2. Un store en V24 abre en V25 con todas sus filas

V25 (`crates/clawg-core/migrations/V25__dedup_index_all_rows.sql`) MUST sólo reconstruir `idx_memory_dedup` como `(COALESCE(project_id, ''), content_hash, created_at)`, sin `WHERE`, y MUST NOT cambiar ninguna fila. Un store en V24 MUST abrir con V25 con cada fila tal como estaba, después de la copia `pre-migration` que se escribe antes de toda migración. El índice MUST tener una entrada por cada fila, también por las borradas. Un binario anterior rechaza el store migrado con `SchemaNewer`, y la vuelta atrás es esa copia.

#### Escenario
- GIVEN un `memory.db` migrado hasta V24, con una fila viva, una borrada y otra sin `content_hash`
- WHEN `MemoryStore::open` lo abre
- THEN las tres filas siguen con las mismas columnas
- AND el `sql` de `idx_memory_dedup` en `sqlite_master` tiene la misma lista de columnas y no lleva `WHERE`
- AND una consulta `INDEXED BY idx_memory_dedup` encuentra la fila borrada por su proyecto, su hash y su fecha
- AND en `backups` hay una copia `pre-migration`

**Tests:** `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows` (`db.rs`, mod tests); el checksum de V25 en `published_migration_checksums_are_pinned` (`migrations.rs`); las tres pruebas de `db.rs` que cuentan migraciones, en 25.

## R3. Las búsquedas del texto vivo y de la clave no cambian de índice

`REINFORCE_LOOKUP_SQL` (`crates/clawg-core/src/memory/store_insert.rs`) MUST seguir llegando por `idx_memory_dedup` sin ninguna línea `TEMP B-TREE`. `SCOPE_ROWS_SQL` (`crates/clawg-core/src/memory/topic.rs`) y `TOPIC_HOLDERS_SQL` MUST seguir llegando por `idx_memory_topic`, aunque `idx_memory_dedup`, ahora que ya no es parcial, también sirva a cualquier consulta con una igualdad sobre `COALESCE(project_id, '')`. Las entradas de filas borradas que ahora están en el índice se descartan como filtro residual. Ninguna de estas consultas MUST devolver otra fila que la que devolvía antes.

#### Escenario
- GIVEN un store migrado
- WHEN se pide el plan de `REINFORCE_LOOKUP_SQL`, de `SCOPE_ROWS_SQL` y de `TOPIC_HOLDERS_SQL`
- THEN el primero nombra `idx_memory_dedup` sin `TEMP B-TREE`, y los otros dos nombran `idx_memory_topic`

**Tests:** `reinforce_lookup_uses_dedup_index` (`store_insert.rs`) y `the_scope_lookup_uses_the_topic_index` (`topic.rs`), sin cambios; `TOPIC_HOLDERS_SQL` en `the_import_lookups_seek_an_index` (R1).

## R4. Lo que la importación hace no cambia

Las filas que la importación inserta, encuentra, fusiona o deja reemplazadas, sus enlaces y su informe MUST ser los mismos que antes del cambio ([`specs/portability`](../../../specs/portability/spec.md)).

**Tests:** las pruebas existentes de `import.rs`, sin cambios, `a_second_import_adds_nothing`, `a_live_twin_absorbs_the_imported_row` y `a_taken_topic_key_supersedes_the_imported_row` entre ellas.

## R5. El tiempo de una importación crece con sus filas, no con su cuadrado

Importar el doble de notas en un store vacío SHOULD tardar como mucho tres veces lo que tarda importar la mitad. Hoy, en depuración, tarda unas nueve veces y media.

#### Escenario
- GIVEN dos stores vacíos
- WHEN se importan N notas de una fila en el primero y 2N en el segundo, con el embedder falso
- THEN las dos importaciones insertan todas sus notas
- AND la segunda tarda como mucho tres veces lo que la primera

**Test:** `twice_the_notes_import_in_at_most_three_times_the_time` (`import.rs`, mod tests, `#[ignore]`). Depende de la máquina, así que queda fuera de CI. Se compara antes y después del cambio en la misma compilación, y es la medición de aceptación.
