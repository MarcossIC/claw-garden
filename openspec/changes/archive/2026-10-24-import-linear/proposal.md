# import-linear — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (va antes de import-decision-pass) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`38ceb07`).

1. **`PRESENT_SQL` recorre `memory` por cada fila.** `crates/clawg-core/src/memory/portability/import.rs:494-497` busca la fila de un agente, un proyecto, un hash y una fecha. No lleva término `deleted_at`, porque una fila borrada aquí también cuenta como presente (`find`, `:443-446`). El único índice con esas columnas, `idx_memory_dedup` (`crates/clawg-core/migrations/V20__project_scoped_indexes.sql:8-11`), es parcial (`WHERE deleted_at IS NULL AND content_hash IS NOT NULL`), y SQLite sólo usa un índice parcial cuando el `WHERE` de la consulta implica el del índice. El plan es `SCAN memory`.
2. **La consulta se repite en los dos pases, y los dos son cuadráticos.** `place` la corre por cada fila que no resolvió antes (`:571-573`): en el pase de decisión (`:966-977`), que es una transacción que crece con sus propias inserciones, y otra vez en el de escritura (`:1008-1024`), por lotes de `WRITE_BATCH`.
3. **Medido.** Una exportación completa (3 814 notas, 52 080 filas) tardó 3 078 s en importarse con una compilación de depuración en Windows. De ese total, ~800 s son el pase de decisión, durante el que el store no contesta, y ~920 s la escritura. Con el embedder falso, 1 000 notas (4 655 filas) tardan 10,3 s y 2 000 notas (15 047 filas) 97,8 s. Con un índice de prueba que la consulta sí usa bajan a 5,1 s y 16,4 s, y el informe es el mismo.

## Objetivo que sirve

1: guardar es eficiente. Lo que cuesta importar una fila tiene que depender de esa fila, no del tamaño del store.

## Decisiones

- **Índice:** V25 reconstruye `idx_memory_dedup` sin `WHERE`, con las mismas columnas. Un solo índice sirve a `PRESENT_SQL` y también a las búsquedas del texto vivo que hacen el worker, `restore`, el movimiento de filas y la propia importación (`REINFORCE_LOOKUP_SQL`, `RESTORE_TWIN`, `TARGET_TWIN_SQL`, `TWIN_SQL`), cada una con un solo plan posible. Se descarta un segundo índice no parcial con la misma clave por dos razones: cada inserción tendría que mantener los dos, y sin estadísticas del planificador no se sabría cuál usa cada consulta (enfoques A y B de la exploración).
- **Banco de escala:** una prueba `#[ignore]` en `import.rs`. Usa `FakeEmbedder` y no necesita el binario.
- **Umbral del banco:** con el doble de notas, el tiempo puede llegar como mucho al triple.
- **`prepare_cached`** en `PRESENT_SQL` y `TWIN_SQL`: entra sólo si, con el índice puesto, la medición muestra lo que cuesta prepararlas en cada fila.
- **Verificación con el perfil de desarrollo:** necesita autorización explícita, que se pide al llegar a verify.

## Alcance

1. `crates/clawg-core/migrations/V25__dedup_index_all_rows.sql`: `idx_memory_dedup` sin `WHERE`, con la misma lista de columnas. Sólo cambia el índice; ninguna fila cambia.
2. Las dos consultas de `memory_piece` que `write_links` hace por save (`import.rs:850` y `:855`) pasan a ser constantes, para que la prueba explique las sentencias de producción y no una copia de ellas.
3. Pruebas: el plan de cada búsqueda que la importación hace por fila o por save, y la apertura de un store en V24.
4. El banco de escala, ignorado.
5. Docs. `.claude/CLAUDE.md` no nombra índices, así que no cambia. Al archivar se actualizan `openspec/specs/storage` (V25 y su vuelta atrás; `idx_memory_dedup` deja de describirse como parcial en `:375` y `:397`) y `openspec/specs/portability` (la importación busca sus filas por índice).

## Fuera de alcance

- El orden y el contenido de los pases, la conexión que el pase de decisión retiene y su transacción descartada: son de import-decision-pass.
- Índices para otras consultas de `memory`, y cambiar la forma de `idx_memory_topic` o de `idx_memory_import_origin`.
- `prepare_cached` en las sentencias del worker.
- Embeddings, piezas y ranking. No se corre `rag-eval`, porque ninguna salida cambia.
- `ANALYZE` y `PRAGMA optimize`.

## Migración y vuelta atrás

V25 sólo reconstruye un índice. Antes de migrar un store en V24 se escribe la copia `pre-migration`, como en toda migración (`crates/clawg-core/src/memory/db.rs:228-238`). Un binario anterior rechaza un store en V25 sin tocarlo (`SchemaNewer`). La vuelta atrás es esa copia, que `clawg update` restaura sola si el binario viejo se niega a abrir el store. También se puede volver a mano sin perder ninguna fila: `DROP INDEX idx_memory_dedup; CREATE INDEX idx_memory_dedup ON memory (COALESCE(project_id, ''), content_hash, created_at) WHERE deleted_at IS NULL AND content_hash IS NOT NULL; DELETE FROM refinery_schema_history WHERE version = 25;`.

El índice pasa a incluir también las filas borradas y las que no tienen hash. Filas sin hash prácticamente no hay, porque el respaldo de V6 se lo calculó a toda fila con texto (`crates/clawg-core/migrations/V6__content_hash.sql:7`), y las borradas son una minoría. Las búsquedas del texto vivo descartan esas entradas como filtro residual.

## Cómo se mide el éxito

Pruebas que fallan hoy (ver [spec](spec.md)), sin modelo real, sin red y sin perfil:

- `the_import_lookups_seek_an_index` (`import.rs`): cada búsqueda que la importación hace por fila o por save nombra su índice en `EXPLAIN QUERY PLAN`, ninguna línea del plan recorre una tabla, y al ejecutarla contra un store con filas no da ningún paso de recorrido completo (`StatementStatus::FullscanStep`). Hoy falla en `PRESENT_SQL`, cuyo plan es `SCAN memory`.
- `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows` (`db.rs`): un store en V24 abre en V25 con todas sus filas tal como estaban, el índice sin `WHERE` encuentra una fila borrada y queda la copia `pre-migration`. Hoy el esquema no llega a 25 y el índice conserva su `WHERE`.
- `published_migration_checksums_are_pinned` gana la fila de V25, y las tres pruebas que cuentan migraciones pasan a 25.
- Tienen que seguir pasando sin cambios `reinforce_lookup_uses_dedup_index`, `a_store_from_before_project_scoped_rules_keeps_its_rows`, las pruebas de plan de `topic.rs`, `language.rs`, `search.rs` y `prompt_builder.rs`, y todas las de la importación, `a_second_import_adds_nothing` entre ellas.

Medición de aceptación, que no es una prueba de CI:

- El banco `twice_the_notes_import_in_at_most_three_times_the_time`: con el doble de notas, el tiempo puede llegar como mucho al triple. Se corre antes y después del cambio en la misma compilación, y el informe de importación tiene que salir igual.
- Con autorización: una importación real en el perfil de desarrollo, con compilación de depuración, para comparar con los 3 078 s.
