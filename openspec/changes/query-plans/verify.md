# query-plans — Verificación

## Linux (2026-10-21)

Sobre `76b513a`, en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info` (ver la última sección).

## RED antes del cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `the_session_statements_read_the_newest_rows_by_index_without_sorting`: `LAST SESSION in a project does not search idx_memory_session_summary: ["SCAN memory USING INDEX idx_memory_agent_active", "USE TEMP B-TREE FOR ORDER BY"]` |
| R2 | `the_session_statements_order_rows_of_one_stamp_by_id` pasaba antes y pasa después: fija que el índice no cambia el orden |
| R3 | `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them`: `left: []`, `right: ["idx_memory_agent_created", "idx_memory_session_summary"]` |
| R4 | `reinforce_lookup_uses_dedup_index`, con los dos índices nuevos y el de dedup sin tocar: `expected the planner to use idx_memory_dedup, got plan: SEARCH memory USING INDEX idx_memory_agent_created (agent_id=?)`. La aserción nueva (sin `TEMP B-TREE`) falla en `develop`, cuyo plan termina en `USE TEMP B-TREE FOR ORDER BY` |

## Planes de las otras sentencias

Un test temporal, no commiteado, pidió `EXPLAIN QUERY PLAN` en un store de prueba vacío de las sentencias de `memory` que filtran por agente u ordenan por `created_at`, con y sin V18 (SQLite empaquetado, 3.50.2):

| Sentencia | Antes | Con V18 |
|---|---|---|
| `REINFORCE_LOOKUP_SQL` (`store_insert.rs:300`) | `idx_memory_dedup` (3 columnas) + `USE TEMP B-TREE FOR ORDER BY` | `idx_memory_dedup` (3 columnas), sin ordenar |
| `TWIN_SQL` (`portability/import.rs:242`) | igual que la anterior | igual que la anterior |
| `RESTORE_TWIN` (`admin.rs:468`) | `t` por `idx_memory_dedup` + `USE TEMP B-TREE FOR ORDER BY` | `t` por `idx_memory_dedup`, sin ordenar |
| `PRESENT_SQL` (`portability/import.rs:236`) | `SCAN memory` | `SCAN memory` (sin cambio; no filtra `deleted_at`, así que ningún índice parcial le sirve) |
| Listado por agente (`admin.rs:850`) | `idx_memory_agent_active (agent_id=?)` + `USE TEMP B-TREE FOR ORDER BY` | `idx_memory_agent_created (agent_id=?)` + `USE TEMP B-TREE FOR LAST TERM OF ORDER BY` |
| Listado sin agente | `SCAN … idx_memory_agent_active` + `TEMP B-TREE` | `SCAN … idx_memory_agent_created` + `TEMP B-TREE` |
| Filas de síntesis (`synthesis.rs:162`) | `idx_memory_agent_active (agent_id=?)` + `TEMP B-TREE` | `idx_memory_agent_created (agent_id=?)` + `TEMP B-TREE FOR LAST TERM` |
| Clave de tema | `idx_memory_topic` | `idx_memory_topic` |
| Por `last_accessed_at` | `idx_memory_agent_active` | `idx_memory_agent_active` |

Ninguna empeora. El listado y la síntesis ordenan `id DESC` dentro de un sello, que el índice no da, pero ya no ordenan todo.

## Tiempo

Un test temporal, no commiteado, en `db.rs` (mod tests): un store de prueba con N filas generadas, cuatro agentes, cinco proyectos, una de cada diez `session_summary`, una de cada cinco `superseded`, sellos repartidos en 90 días; la media de 50 corridas de cada sentencia, con la sentencia de antes sobre el esquema de V17 y la de después sobre V18. Para LAST SESSION se lee la primera fila, como `fetch_last_session` cuando el autor es legible; para RECENT, el pool entero. «Inserción» es un `INSERT` de una fila viva. Es un orden de magnitud con el SQLite empaquetado en una máquina de CI, no un umbral.

| Filas | LAST SESSION antes | después | RECENT antes | después | Inserción antes | después |
|---|---|---|---|---|---|---|
| 2 000 | 0,85 ms | 0,027 ms | 0,36 ms | 0,11 ms | 0,10 ms | 0,11 ms |
| 10 000 | 3,9 ms | 0,027 ms | 1,5 ms | 0,19 ms | 0,10 ms | 0,11 ms |
| 50 000 | 27 ms | 0,026 ms | 10 ms | 0,81 ms | 0,17 ms | 0,11 ms |

LAST SESSION queda plano. RECENT crece con las filas de la semana del agente (unas 1 000 a 50 000 filas, de las que el predicado de proyecto descarta cuatro de cada cinco), no con el store. La inserción no cambia a esta escala.

## Estadísticas

El mismo test, con V18 y `ANALYZE` después de sembrar: los planes de LAST SESSION (`SEARCH … idx_memory_session_summary (project_id=?)`), RECENT (`SEARCH … idx_memory_agent_created (agent_id=? AND created_at>?)`) y del dedup (`SEARCH … idx_memory_dedup (…)`) son los mismos que sin estadísticas en los tres tamaños, y los tiempos del mismo orden (LAST SESSION 0,06 ms, RECENT 0,78 ms a 50 000). No hay nada que ganar en estas sentencias; la decisión de descartarlas está en el [diseño](design.md#d5-sin-estadísticas-del-planificador).
