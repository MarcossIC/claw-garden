# query-plans — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (antes de 44) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`cbfcee7`).

1. **LAST SESSION recorre todo `memory`.** `LAST_SESSION_SQL` (`crates/clawg-core/src/memory/prompt_builder.rs:772-780` en `cbfcee7`) filtra por `memory_type = 'session_summary'` y proyecto y ordena por `created_at DESC, id ASC`, pero ningún índice de `memory` nombra `memory_type` ni `created_at` (`V1__init.sql:37-57`, `V6__content_hash.sql:14`, `V16__topic_key.sql:13-15`, `V17__revival.sql:16`). El plan con el SQLite empaquetado es `SCAN memory USING INDEX idx_memory_agent_active` + `USE TEMP B-TREE FOR ORDER BY`: lee todas las filas vivas del store para devolver una.
2. **RECENT recorre todas las filas vivas del agente.** `RECENT_ROWS_SQL` (`prompt_builder.rs:868-881`) entra por `idx_memory_agent_active (agent_id, last_accessed_at)`, cuyo orden no sirve, aplica `created_at >= ?3` después y ordena en un árbol temporal para quedarse con 200.
3. **Las dos corren en el camino caliente.** Todo `POST /v1/context` corre LAST SESSION; uno sin consulta, que es el de cada `clawg hook session-start`, corre las dos. Comparten la conexión única con los guardados. Las filas `superseded` nunca se borran, así que lo recorrido sólo crece, y 44 (history-import) trae miles de filas de una vez.
4. **El predicado de proyecto de LAST SESSION no es indexable.** `(project_id = ?1 OR (?1 IS NULL AND project_id IS NULL))` (`:778`) hace que el planificador no use un índice por `project_id` aunque exista.

Medido con el SQLite empaquetado y filas generadas (ver [verify](verify.md#tiempo)): el costo de las dos crece linealmente con las filas antes del cambio y queda plano después.

## Objetivo que sirve

1: guardar y recuperar es eficiente. El costo de abrir una sesión debe depender de lo que se muestra, no del tamaño del store.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Predicado de LAST SESSION:** `project_id IS ?1`, una sola sentencia para valor y `NULL`.
- **`status` en los índices:** fuera. Los índices quedan desacoplados del texto de los estados.
- **`prepare_cached`:** sólo en `LAST_SESSION_SQL` y `RECENT_ROWS_SQL`.
- **Estadísticas del planificador:** descartadas, con la evaluación escrita en el diseño.
- **Migración:** V18 (la última es V17; 51 y 54 se archivaron sin migración).
- **Backlog «RECENT puede mostrar menos de diez saves»:** fuera.
- **Autor de LAST SESSION:** sigue filtrándose en Rust; con el índice la lectura se detiene en la primera fila admitida.
- **Medición:** se anota en [verify](verify.md); no se commitea como prueba.

## Alcance

1. `V18__session_indexes.sql`: `idx_memory_session_summary ON memory (project_id, created_at DESC) WHERE memory_type = 'session_summary' AND deleted_at IS NULL` e `idx_memory_agent_created ON memory (agent_id, created_at DESC) WHERE deleted_at IS NULL`, y `idx_memory_dedup` reconstruido con `created_at` al final, porque el índice del agente le ganaba las búsquedas de dedup ([diseño](design.md#d1b-idx_memory_dedup-se-reconstruye-con-created_at-al-final-r4)). Sólo índices: ninguna fila cambia.
2. `LAST_SESSION_SQL` con `project_id IS ?1`.
3. `prepare_cached` en las dos sentencias.
4. Pruebas: plan de las dos sentencias, orden dentro de un mismo sello y apertura de un store en V17.
5. Docs: `.claude/CLAUDE.md` no nombra índices, así que no cambia; al archivar, `openspec/specs/storage` (V18) y `openspec/specs/retrieval` (los planes).

## Fuera de alcance

- Arreglar «RECENT puede mostrar menos de diez saves».
- Cambiar el pool, la ventana de siete días, el orden o el texto de los bloques.
- Ranking: no se corre `rag-eval`, porque ninguna salida cambia.
- Una caché del último resumen en el daemon.
- Índices para otras consultas de `memory`.
- `ANALYZE` o `PRAGMA optimize`.

## Migración y vuelta atrás

V18 sólo crea dos índices parciales. Abrir un store en V17 escribe antes la copia `pre-migration` (`crates/clawg-core/src/memory/db.rs:223-230`), como toda migración. Un binario anterior rechaza un store en V18 sin tocarlo (`SchemaNewer`); la vuelta atrás es esa copia, que `clawg update` restaura sola si el binario viejo se niega, o a mano: `DROP INDEX idx_memory_session_summary; DROP INDEX idx_memory_agent_created; DROP INDEX idx_memory_dedup; CREATE INDEX idx_memory_dedup ON memory (agent_id, COALESCE(project_id, ''), content_hash) WHERE deleted_at IS NULL AND content_hash IS NOT NULL; DELETE FROM refinery_schema_history WHERE version = 18;`, que no pierde ninguna fila.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- `the_session_statements_read_the_newest_rows_by_index_without_sorting`: LAST SESSION con proyecto y sin él, y RECENT, buscan por su índice y no ordenan; hoy recorren `idx_memory_agent_active` con árbol temporal;
- `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them`: un store en V17 abre en V18 con su fila y los dos índices;
- `the_session_statements_order_rows_of_one_stamp_by_id`: con sellos repetidos, las dos sentencias dan `created_at DESC, id ASC`; pasa hoy y tiene que seguir pasando con los índices;
- `reinforce_lookup_uses_dedup_index`, con la aserción nueva de que no ordena: hoy su plan lleva `USE TEMP B-TREE FOR ORDER BY`;
- las pruebas existentes de LAST SESSION y RECENT pasan sin tocarlas.
