# query-plans — Especificación

## R1. LAST SESSION y RECENT llegan a lo más nuevo por índice y sin ordenar

`LAST_SESSION_SQL` MUST llegar a sus filas por `idx_memory_session_summary`, con proyecto y sin él, y `RECENT_ROWS_SQL` por `idx_memory_agent_created`, los dos con `SEARCH memory USING INDEX` en `EXPLAIN QUERY PLAN` y sin ninguna línea `TEMP B-TREE`, sin estadísticas del planificador. El predicado de proyecto de LAST SESSION MUST ser `project_id IS ?1`, que selecciona las mismas filas que el anterior para un valor y para `NULL`. Las dos sentencias MUST prepararse con `prepare_cached`.

#### Escenario: LAST SESSION en un proyecto
- GIVEN un store migrado
- WHEN se pide el plan de `LAST_SESSION_SQL` con `?1 = 'proj-a'`
- THEN una línea es `SEARCH memory USING INDEX idx_memory_session_summary (project_id=?)` y ninguna nombra `TEMP B-TREE`

#### Escenario: LAST SESSION sin proyecto
- GIVEN el mismo store
- WHEN se pide el plan con `?1 = NULL`
- THEN el plan es el mismo

#### Escenario: RECENT
- GIVEN el mismo store
- WHEN se pide el plan de `RECENT_ROWS_SQL` con agente, proyecto, `since` y el pool
- THEN una línea busca por `idx_memory_agent_created` y ninguna nombra `TEMP B-TREE`

**Test:** `the_session_statements_read_the_newest_rows_by_index_without_sorting` (`prompt_builder.rs`, mod tests).

## R2. La salida no cambia

Las dos sentencias MUST devolver las mismas filas en el mismo orden que antes: `created_at DESC` y, dentro de un mismo sello (los pedazos de un save lo comparten), `id ASC`. El texto de LAST SESSION y RECENT, el pool de 200 filas, la ventana de siete días y el filtro de autores por `Reach::admits` MUST quedar como están ([`specs/retrieval`](../../specs/retrieval/spec.md), «El contexto sin consulta» y «La última sesión»).

#### Escenario: sellos repetidos
- GIVEN seis resúmenes de un agente en un proyecto, insertados con dos sellos alternados
- WHEN se leen con `LAST_SESSION_SQL` y con `RECENT_ROWS_SQL`
- THEN las dos dan primero los del sello más nuevo y después los del más viejo, cada grupo por `id` ascendente

**Tests:** `the_session_statements_order_rows_of_one_stamp_by_id` (`prompt_builder.rs`, mod tests), que pasa antes y después; las pruebas existentes de LAST SESSION y RECENT, sin cambios.

## R3. Un store en V17 abre en V18 con sus filas y los índices

V18 (`crates/clawg-core/migrations/V18__session_indexes.sql`) MUST sólo crear `idx_memory_session_summary` e `idx_memory_agent_created` y reconstruir `idx_memory_dedup` (R4), y MUST NOT cambiar ninguna fila. Un store en V17 MUST abrir con V18 con cada fila y con los dos índices, después de la copia `pre-migration` de toda migración. Un binario anterior rechaza el store migrado con `SchemaNewer`; la vuelta atrás es esa copia.

#### Escenario
- GIVEN un `memory.db` migrado hasta V17 con una fila
- WHEN `MemoryStore::open` lo abre
- THEN la fila sigue y `sqlite_master` tiene los dos índices sobre `memory`

**Tests:** `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them` (`db.rs`, mod tests); el checksum de V18 en `published_migration_checksums_are_pinned` (`migrations.rs`); la copia `pre-migration` la cubre `opening_an_older_database_snapshots_it_before_migrating` (`db.rs`).

## R4. Las búsquedas de dedup siguen en su índice y sin ordenar

`idx_memory_dedup` MUST ser `(agent_id, COALESCE(project_id, ''), content_hash, created_at) WHERE deleted_at IS NULL AND content_hash IS NOT NULL`. `REINFORCE_LOOKUP_SQL` MUST llegar a sus filas por `idx_memory_dedup` y sin ninguna línea `TEMP B-TREE`: cualquier índice que entregue las filas de `agent_id = ?` por `created_at` compite por su `ORDER BY created_at ASC LIMIT 1`, y sin el sello en el índice de dedup el planificador elige ése y recorre las filas vivas del agente en cada guardado.

#### Escenario
- GIVEN un store migrado
- WHEN se pide el plan de `REINFORCE_LOOKUP_SQL`
- THEN nombra `idx_memory_dedup` y ninguna línea nombra `TEMP B-TREE`

**Test:** `reinforce_lookup_uses_dedup_index` (`store_insert.rs`, mod tests). Con sólo los dos índices de R1 fallaba por la primera aserción (`SEARCH memory USING INDEX idx_memory_agent_created (agent_id=?)`); antes de V18, por la segunda (`USE TEMP B-TREE FOR ORDER BY`).
