# query-plans — Diseño

## D1. Dos índices parciales en una migración (R1, R3)

**Elegido:** `V18__session_indexes.sql` con

- `idx_memory_session_summary ON memory (project_id, created_at DESC) WHERE memory_type = 'session_summary' AND deleted_at IS NULL`, para LAST SESSION;
- `idx_memory_agent_created ON memory (agent_id, created_at DESC) WHERE deleted_at IS NULL`, para RECENT.

Un índice guarda las claves iguales por `rowid` ascendente, e `id` es el `INTEGER PRIMARY KEY` de `memory` (`V1__init.sql:4`), así que recorrer `created_at DESC` entrega el desempate `id ASC` sin ordenar. Con `created_at ASC` el planificador deja `USE TEMP B-TREE FOR LAST TERM OF ORDER BY` (exploración). Que el orden de salida sea ese, y no sólo el plan, lo fija una prueba con sellos repetidos (R2).

| Alternativa | Por qué no |
|---|---|
| Reescribir las consultas sobre los índices que hay | Ninguno ordena por `created_at` ni filtra por `memory_type`: el recorrido y el árbol temporal quedan |
| Crear los índices al abrir, fuera de refinery | Dos stores con el mismo historial tendrían esquemas distintos (`schema_plan` mira sólo el historial, `backup.rs:93`) y se salta la copia `pre-migration` |
| Índices completos, sin parcial | Indexan las filas borradas y, en LAST SESSION, todos los tipos, para servir a consultas que no los miran |
| Una caché del último resumen en el daemon | Siete módulos cambian qué es «el último» (`status`, `superseded_by`, `deleted_at`); un olvido deja un LAST SESSION viejo sin que nada lo muestre. El índice lo mantiene SQLite en la misma transacción |

**`status` fuera del predicado de los índices.** Sumar `status IN ('active', 'disputed')` los achica (deja afuera las `superseded`), pero ata el texto del índice al de cada sentencia: el planificador sólo usa un índice parcial cuando puede probar que el `WHERE` de la sentencia implica el del índice. Un estado nuevo, o un cambio de texto, sacaría la sentencia del índice sin que nada falle salvo la prueba de plan. Las `superseded` de un resumen o de un agente son pocas frente a lo que recorren hoy.

**Costo de escritura.** Cada inserción viva entra en los dos índices (el de LAST SESSION sólo si es un resumen). `last_accessed_at`, que el tracker de accesos actualiza, no está en ninguno, así que los accesos no los tocan. Medido con filas generadas, una inserción cuesta lo mismo antes y después, unos 0,1 ms ([verify](verify.md#tiempo)).

## D1b. `idx_memory_dedup` se reconstruye con `created_at` al final (R4)

Con `idx_memory_agent_created` en el esquema, `reinforce_lookup_uses_dedup_index` (`crates/clawg-core/src/memory/store_insert.rs:2165`) falló: el planificador pasó `REINFORCE_LOOKUP_SQL` (`store_insert.rs:300-307`, `agent_id = ?1 … ORDER BY created_at ASC LIMIT 1`) a `SEARCH memory USING INDEX idx_memory_agent_created (agent_id=?)`, porque ese índice le entrega el orden y el `LIMIT 1` promete parar en la primera fila. Pero la primera con el mismo hash puede estar al final: cada guardado recorrería las filas vivas del agente. Lo mismo vale para `TWIN_SQL` del import (`crates/clawg-core/src/memory/portability/import.rs:242-245`) y `RESTORE_TWIN` del undelete (`crates/clawg-core/src/memory/admin.rs:468-473`), que tienen la misma forma.

**Elegido:** V18 reconstruye `idx_memory_dedup` como `(agent_id, COALESCE(project_id, ''), content_hash, created_at)` con el mismo predicado parcial. Las tres sentencias quedan con igualdad en las tres primeras columnas y el orden de la cuarta: vuelven al índice de dedup y, además, pierden el `USE TEMP B-TREE FOR ORDER BY` que tenían antes ([verify](verify.md#planes-de-las-otras-sentencias)). El nombre no cambia, así que los comentarios y el doctor que lo nombran siguen valiendo.

| Alternativa | Por qué no |
|---|---|
| `ORDER BY created_at + 0` en las sentencias de dedup | Un truco de texto que el próximo índice puede volver a romper, y deja el árbol temporal |
| `INDEXED BY idx_memory_dedup` | Hace fallar la sentencia si el índice cambia de forma; ata el texto al nombre |
| Un índice de RECENT que no empiece por `agent_id` | `(created_at DESC)` recorrería la semana de todos los agentes, y el dedup podría igual elegirlo para ordenar |

La prueba de dedup suma que su plan no ordena, que es lo que la protege de la próxima vez.

## D2. `project_id IS ?1` (R1)

**Elegido:** una sola sentencia con `project_id IS ?1`. `IS` da verdadero para dos `NULL` y para dos valores iguales, que es exactamente lo que decía el `OR` de antes, y el planificador lo usa como igualdad sobre la primera columna del índice, con proyecto y sin él (la prueba de plan cubre los dos casos).

| Alternativa | Por qué no |
|---|---|
| Dejar `(project_id = ?1 OR (?1 IS NULL AND project_id IS NULL))` | Con el índice disponible el planificador no lo usa (exploración): agregar el índice sin tocar la sentencia no arregla nada |
| Dos sentencias, una por caso | Mismo plan, el doble de texto y un `match` en el llamador |

## D3. El autor sigue en Rust

`fetch_last_session` recorre las filas hasta la primera que `Reach::admits` deja leer (`prompt_builder.rs:834-840`). Con el índice, SQLite entrega las filas de a una en orden, así que la lectura se detiene ahí: su costo pasa a ser cuántos resúmenes más nuevos del proyecto son de autores que el lector no lee, cada pedazo como una fila. Eso sólo crece con `sharing.json` ocultando a un autor muy activo en el mismo proyecto. Llevar `reach_predicate!` a la sentencia con `LIMIT 1` lo deja en una fila, pero suma un bind y un JSON a una sentencia que hoy no lo necesita; queda como paso siguiente si alguna vez se mide ese caso.

## D4. `prepare_cached` sólo donde el cambio pone la mano

`LAST_SESSION_SQL` y `RECENT_ROWS_SQL` pasan a `prepare_cached`. Son texto fijo, corren en cada contexto, y la caché de rusqlite es por conexión (16 entradas por defecto): el daemon tiene una conexión de larga vida. Preparar una sentencia cuesta microsegundos frente a los milisegundos que quitan los índices, así que no se extiende a otras sentencias (los KNN de `search.rs`, el outbox) sin medirlas. Las sentencias con listas `IN` de largo variable no pasan por la caché: cada largo sería una entrada nueva.

## D5. Sin estadísticas del planificador

**Elegido:** no correr `ANALYZE` ni `PRAGMA optimize`.

Evaluación (ver [verify](verify.md#estadísticas)): con V18, filas generadas (2 000, 10 000 y 50 000) y `ANALYZE` aplicado, los planes de LAST SESSION, RECENT y del dedup no cambian y los tiempos tampoco. No hay nada que ganar en estas sentencias, y la decisión se apoya además en que:

- el comentario de `crates/clawg-core/src/memory/search.rs:970-981` sobre `NOT INDEXED` en la sentencia de puntaje léxico describe un plan que depende de que no haya estadísticas;
- las estadísticas cambian con los datos de cada store, así que un plan que las pruebas fijan en un store vacío dejaría de ser el que corre en el de un usuario, y ninguna prueba lo vería;
- los índices de D1 ganan sin estadísticas, que era la condición.

Si un cambio futuro necesita estimaciones reales, `PRAGMA optimize` en la conexión (no `ANALYZE` en una migración) es el mecanismo, con las pruebas de plan corridas sobre un store analizado.
