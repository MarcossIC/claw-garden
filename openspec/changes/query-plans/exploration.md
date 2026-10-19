# query-plans — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (antes de 44; la migración se numera junto con 54 y 51) | exploración (borrador) |

**Tesis:** las dos consultas que se corren en cada contexto de sesión, LAST SESSION y RECENT, tienen un costo que crece con el número de filas de `memory` y no con lo que devuelven, porque ningún índice cubre `memory_type` ni `created_at`. Hoy una recorre todas las filas vivas y la otra todas las filas vivas del agente, y las dos ordenan el resultado en un árbol temporal. Con una migración de sólo dos índices parciales el plan pasa a ser una búsqueda por clave que entrega el orden pedido sin ordenar, y el texto de los dos bloques no cambia. Hay un detalle que decide si el índice sirve: la forma actual del predicado de proyecto de LAST SESSION **no usa el índice nuevo**, así que el cambio incluye reescribir esa condición. Van dos asuntos pequeños en el mismo cambio: preparar una vez (`prepare_cached`) las sentencias fijas que el cambio toca, y decidir si se adoptan estadísticas del planificador (`ANALYZE`, `PRAGMA optimize`), que hoy no existen y que pueden mover planes que las pruebas fijan. No hay cambio de ranking.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `LAST_SESSION_SQL` elige el último `session_summary` vivo del proyecto, `ORDER BY created_at DESC, id ASC` | **Cierta, con un predicado de más.** `crates/clawg-core/src/memory/prompt_builder.rs:709-718`. Además de `memory_type`, `deleted_at IS NULL` y el proyecto, filtra `status IN ('active', 'disputed')` (`:712`), que cualquier índice debe tener en cuenta. |
| `fetch_last_session` lee filas hasta la primera que el lector puede leer | **Cierta.** `:759-791`: `stmt.query` y un `while` que salta con `continue` las filas cuyo autor `reach.admits` rechaza (`:769-775`). Con el orden resuelto por un índice, la consulta se detiene en la primera admitida; hoy el ordenamiento temporal ya recorrió todo antes de entregar la primera fila. |
| `RECENT_ROWS_SQL` trae las filas propias del lector desde `?3`, con `LIMIT ?4` y pool de 200 | **Cierta.** `:804-819`, `RECENT_ROW_POOL` = 200 (`:819`). El predicado de proyecto es una cadena de `OR` (`?2 IS NULL OR project_id = ?2 OR project_id IS NULL OR memory_type = 'user_preference'`, `:812`). |
| Corren en cada `recall_context` / `POST /v1/context` | **Cierta, con dos caminos.** Con consulta: `build_memory_prompt` llama a `with_last_session` (`:248`); sin consulta: `build_session_prompt` llama a `fetch_recent_rows` (`:283`) y a `with_last_session` (`:288`). El handler elige uno u otro (`crates/clawg-cli/src/daemon/api.rs:1236-1256`). RECENT sólo corre sin consulta; LAST SESSION corre siempre. |
| Y en cada `clawg hook session-start` | **Cierta.** `hook.rs:166-167` pide `recent_context` (sin consulta) con `SESSION_BUDGET`; el plazo del hook es `HOOK_DEADLINE` = 3 s (`hook.rs:25`, usado en `main.rs:109`). **Esto no es un problema de plazo:** el costo medido abajo es de decenas de milisegundos a 50 000 filas, lejos de 3 s. Es un costo lineal en una conexión única (`store.conn.call`, `:728-730` y `:787`) que comparten los guardados, no un riesgo de que el hook se corte. |
| Ningún índice cubre `memory_type` ni `created_at` | **Cierta.** Los de `memory` son `idx_memory_agent_active (agent_id, last_accessed_at) WHERE deleted_at IS NULL` (`V1__init.sql:37-39`), `idx_memory_scope_project (scope, project_id)` (`:41-43`), `idx_memory_last_accessed`, `idx_memory_superseded`, `idx_memory_disputed`, `idx_memory_tier_status` (`:45-57`), `idx_memory_dedup` (`V6__content_hash.sql:14`), `idx_memory_topic` (`V16__topic_key.sql:13-15`) y `idx_memory_revived` (`V17__revival.sql:16`). Ninguno nombra `created_at` ni `memory_type`. La última migración es V17. |
| Planes esperados con SQLite 3.50.2 | **Reproducidos con otra versión; la autoridad es la prueba roja.** SQLite empaquetado: 3.50.2 (`libsqlite3-sys` 0.35.0, `Cargo.lock`; `#define SQLITE_VERSION "3.50.2"` en el `sqlite3.c` del crate; `rusqlite` con `bundled`, `Cargo.toml:32`). Reproduje `EXPLAIN QUERY PLAN` fuera del repo con las migraciones V1 a V17 reales (sin las sentencias `vec0` y con las funciones `content_hash`, `compute_tier` y `demote_tier` reemplazadas por stubs) en el SQLite de Python, que es **3.51.0**, no 3.50.2. Resultado: LAST SESSION `SCAN memory USING INDEX idx_memory_agent_active` + `USE TEMP B-TREE FOR ORDER BY`; RECENT `SEARCH memory USING INDEX idx_memory_agent_active (agent_id=?)` + `USE TEMP B-TREE FOR ORDER BY`. Coincide con lo esperado; que el plan sea el mismo en 3.50.2 lo fija la prueba roja de plan, no esta reproducción. |
| El costo es lineal en las filas | **Cierta como forma; las cifras no son de este repo.** Una medición sintética fuera del repo, con el SQLite empaquetado (3.50.2) y filas generadas, dio del orden de 0,6 / 9 / 47 ms para LAST SESSION y 0,6 / 5 / 21 ms para RECENT a 2 000 / 10 000 / 50 000 filas, lineal, con las filas `superseded` incluidas (nunca se borran). Mi reproducción con Python y filas generadas dio cifras menores (LAST SESSION 0,1 / 0,6 / 5 ms; RECENT 0,5 / 1,3 / 8 ms) con la misma forma lineal; no usa el mismo SQLite ni el mismo reparto de tipos. Sirve como orden de magnitud; la medición propia, con el SQLite empaquetado, es el primer paso. |
| Los dos índices parciales propuestos sirven tal cual | **Falsa para LAST SESSION; cierta para RECENT.** Con `(project_id, created_at DESC) WHERE memory_type = 'session_summary' AND deleted_at IS NULL`, la sentencia actual, con `(project_id = ?1 OR (?1 IS NULL AND project_id IS NULL))`, **no usa ese índice**: el planificador elige un `SCAN` de otro índice y conserva el árbol temporal (reproducido, 3.51.0). Escrita como `project_id IS ?1`, la misma sentencia da `SEARCH memory USING INDEX <nuevo> (project_id=?)` sin árbol temporal, tanto con proyecto como con `NULL`. RECENT sí usa `(agent_id, created_at DESC) WHERE deleted_at IS NULL`: `SEARCH ... (agent_id=? AND created_at>?)` sin árbol temporal, a pesar de la cadena de `OR` de proyecto. |
| El desempate `id ASC` se obtiene sin ordenar con un índice `created_at DESC` | **Hipótesis, apoyada por el plan pero no por los datos.** En la reproducción, el índice `(agent_id, created_at DESC)` no deja `TEMP B-TREE` en RECENT; el mismo índice en `ASC` deja `USE TEMP B-TREE FOR LAST TERM OF ORDER BY` (ordena sólo dentro de cada grupo de `created_at` iguales, pero está). La razón esperada es que un índice guarda las claves iguales por `rowid` ascendente, y `id` es el `INTEGER PRIMARY KEY AUTOINCREMENT` de `memory` (`V1__init.sql:4`). Que el **orden de salida** sea idéntico con sellos repetidos (los pedazos de un save comparten el sello, `:759-762`) lo tiene que comprobar una prueba con datos, no el plan. |
| No hay `ANALYZE` ni `PRAGMA optimize` | **Cierta.** `rg "ANALYZE\|optimize"` en `crates/` sólo da comentarios de `search.rs` y el `INSERT INTO memory_fts(memory_fts) VALUES ('optimize')` de `secrets.rs:123`, que es del índice FTS y no del planificador. El comentario de `search.rs:964-975` **depende** de esa ausencia: dice que «the schema carries no `ANALYZE`» y que por eso hace falta `NOT INDEXED` en la sentencia de puntaje léxico (`:1037`). |
| El SQLite empaquetado se compila con `SQLITE_ENABLE_STAT4` | **Cierta.** `libsqlite3-sys-0.35.0/build.rs:137`, en la rama que compila `sqlite3.c`. Con STAT4, `ANALYZE` guarda muestras de las claves y el planificador las usa para estimar. |
| `prepare(` supera a `prepare_cached(` en producción | **Cierta.** Fuera de los módulos de prueba (corté cada archivo en su `mod tests`; no excluye bloques `#[cfg(test)] impl` sueltos, así que son cifras aproximadas): **42** llamadas a `.prepare(` contra **9** a `prepare_cached(` (`revival.rs:95`, `store_insert.rs:235`, `:253`, `synthesis.rs:183`, `:341`, `:372`, `topic.rs:99`, `:138`, `:145`). No cuenta `execute` ni `query_row` de `Connection`, que también preparan la sentencia en cada llamada; por ejemplo `outbox_record` (`outbox.rs:208`) usa `query_row` y `execute`, no verifiqué cuántas sentencias de ese camino se preparan por guardado. |
| Una migración cuesta una copia y un camino de vuelta | **Cierta.** Abrir un store con migraciones pendientes escribe antes un snapshot `pre-migration` (`crates/clawg-core/src/memory/db.rs:223-230`, `backup::Reason::PreMigration`); un store de una versión más nueva se rechaza sin tocarlo (`SchemaNewer`, `db.rs:415`), y `schema_plan` mira sólo el historial de refinery, no los objetos reales (`backup.rs:93-110`), de modo que un índice creado fuera de refinery no lo vería ni lo rechazaría. `clawg update` restaura ese snapshot si el binario viejo se niega a abrir el store (`openspec/specs/self-update/spec.md:143`). `openspec/config.yaml` exige una prueba que siembre datos en la versión anterior; `seed_on_disk(dir, Some(17))` (`db.rs:455`) ya hace eso en las pruebas de migración. |
| El backlog «RECENT puede mostrar menos de diez saves» sigue vigente | **Cierta.** La sentencia y el pool están en `prompt_builder.rs:804-819` y la unión en saves en `:857` (`one_row_per_save`). El problema (el pool de 200 filas se junta en saves después del `LIMIT`) es el mismo. |

## Problema (verificado contra el código)

LAST SESSION y RECENT son lecturas de «lo más nuevo» y se pagan como lecturas de «todo»:

1. **LAST SESSION** (`prompt_builder.rs:709-718`). El filtro por tipo y proyecto no tiene índice, así que el plan recorre todo `memory` por `idx_memory_agent_active` (un índice que existe por otra razón y cuyo orden no sirve), descarta lo que no es un resumen de ese proyecto y ordena lo que queda. El resultado que interesa es **una** fila. El costo es proporcional a las filas vivas del store, de todos los agentes y proyectos.
2. **RECENT** (`:804-819`). Entra por `agent_id`, pero ese índice ordena por `last_accessed_at`, no por `created_at`, y el `created_at >= ?3` (siete días, `RECENT_WINDOW_MS`, `:46`) se aplica después: lee todas las filas vivas del agente, filtra y ordena para quedarse con 200. Con una importación grande de historial (44) el agente tiene miles de filas viejas que esta consulta lee para descartarlas.
3. **Las dos corren siempre en el camino caliente.** Un contexto sin consulta las corre a las dos; uno con consulta corre LAST SESSION. Comparten la conexión única de SQLite con los guardados y el tracker de accesos.
4. **El predicado de proyecto de LAST SESSION no es indexable como está.** `(project_id = ?1 OR (?1 IS NULL AND project_id IS NULL))` mezcla dos casos en una condición; en la reproducción, ni con el índice nuevo disponible el planificador lo usa. Es lo que hace que «agregar el índice» sin tocar la sentencia no arregle nada.
5. **Las filas `superseded` no se borran** (la historia de una clave es la cadena de filas reemplazadas), así que lo que recorren estas consultas sólo crece. LAST SESSION las descarta con `status`, pero después de leerlas.
6. **LAST SESSION filtra el autor en Rust y no tiene `LIMIT`.** La sentencia trae todos los resúmenes vivos del proyecto (`prompt_builder.rs:709-717`) y `fetch_last_session` los recorre hasta el primero que `reach.admits` deja leer (`:771-775`). Hoy eso no agrega costo, porque el orden ya obliga a leer y ordenar todo. Con el índice nuevo sí cuenta: la lectura para en la primera fila legible, así que su costo pasa a ser cuántos resúmenes más nuevos escribieron autores que el lector no lee, cada pedazo de un resumen largo como una fila. Llevar el predicado de compartición a la sentencia y ponerle `LIMIT 1` lo deja en una fila; desde `9f8fa90` sale de una sola macro (`reach_predicate!`, `crates/clawg-core/src/memory/search.rs:77`), que `the_reach_predicate_agrees_with_admits` ata a `Reach::admits`, así que esa sentencia no sumaría una copia más. Desde `8f710de`, con el resumen elegido `with_last_session` lee además su save entero (`read_save_core`, `crates/clawg-core/src/memory/pieces.rs:98`), por `save_id` y no por proyecto, así que esa segunda lectura no crece con el store.

## Lo que ya existe y se reutiliza

- Las consultas, sus constantes y su salida: `LAST_SESSION_SQL`, `RECENT_ROWS_SQL`, `RECENT_ROW_POOL`, `one_row_per_save`. Lo que se cambia es el acceso, no el resultado.
- La forma de una migración de sólo índices: `CREATE INDEX IF NOT EXISTS ... WHERE ...` en `V16__topic_key.sql:13-15` y `V17__revival.sql:16`, con el mismo estilo de índice parcial.
- Pruebas de plan que fijan por qué camino llega una sentencia: `reinforce_lookup_uses_dedup_index` (`store_insert.rs:2151`, lee `EXPLAIN QUERY PLAN` de `REINFORCE_LOOKUP_SQL`) y `the_scoring_statement_reaches_its_rows_by_id_at_every_candidate_count` (`search.rs:3795`, con `scoring_plan_lines_for`, `:3730`). Son el modelo de las pruebas rojas de acá.
- `seed_on_disk(dir, target)` (`db.rs:455`): construye un `memory.db` migrado hasta una versión y con una fila, para la prueba de V17 a V18.
- Las pruebas existentes de LAST SESSION y RECENT, que fijan el texto de los bloques (el resultado no debe cambiar). Las definen `openspec/specs/retrieval/spec.md` y `openspec/specs/hooks/spec.md` (session-handoff y agent-lifecycle-hooks, archivados).
- `prepare_cached` ya se usa en nueve sitios (arriba): no hay que introducir nada, sólo extender su uso.

## Enfoques posibles

### Índices

**A. Una migración con dos índices parciales y reescribir el predicado de LAST SESSION (recomendado).**

- `ON memory (project_id, created_at DESC) WHERE memory_type = 'session_summary' AND deleted_at IS NULL`, para LAST SESSION.
- `ON memory (agent_id, created_at DESC) WHERE deleted_at IS NULL`, para RECENT.
- `LAST_SESSION_SQL` pasa de `(project_id = ?1 OR (?1 IS NULL AND project_id IS NULL))` a `project_id IS ?1`: es lo mismo para valor y para `NULL` (`IS` compara `NULL` con `NULL` como verdadero) y es lo que el planificador sí indexa (reproducido con y sin proyecto).
- **A favor:** el plan pasa a `SEARCH ... USING INDEX <nuevo>` sin árbol temporal; LAST SESSION se detiene en la primera fila admitida; el costo deja de depender del tamaño del store. No cambia ninguna salida. El orden `created_at DESC, id ASC` lo entrega el índice (hipótesis de la tabla).
- **En contra:** una migración (copia `pre-migration`, vuelta atrás por el snapshot, y el binario anterior se niega a abrir el store migrado) para un cambio de rendimiento. Cada guardado actualiza dos índices más (los índices parciales sólo cambian cuando se inserta una fila, cambia `created_at`, `memory_type` o `deleted_at`; `last_accessed_at` del tracker no toca estos). No medí el costo de escritura ni el tiempo de construir el índice sobre un store grande.
- **Variantes a decidir:** poner o no `status IN ('active', 'disputed')` en el predicado de cada índice (la reproducción mostró que el planificador lo acepta cuando el texto coincide con el de la sentencia; deja los índices más chicos, pero acopla el texto del índice al de la sentencia y a los estados); y `IS ?1` en una sola sentencia frente a dos sentencias (proyecto y `NULL`).

**B. Reescribir las consultas sobre los índices que ya hay.** No quita el recorrido: ningún índice existente ordena por `created_at` ni por `memory_type`. Descartado.

### Preparación de sentencias

**C. `prepare_cached` en las sentencias fijas que el cambio toca (recomendado), y opcionalmente en otras fijas calientes.** `LAST_SESSION_SQL` (`:731`) y `RECENT_ROWS_SQL` (`:789`) pasan a `prepare_cached`. Opcional: los KNN de `search.rs:533` (`table.sql`, una sentencia fija por tabla) y las sentencias de `outbox_record`. Preparar una sentencia fija cuesta decenas de microsegundos contra milisegundos de recorrido (orden de magnitud, no medido acá), así que **el beneficio de C es chico y el de A es el grande**; se hace sólo donde el cambio ya pone la mano.
- **No** pasan por la caché las sentencias con una lista `IN` de largo variable: el puntaje léxico (`search.rs:1125`, `lexical_score_sql(n)`) y el vaciado del tracker de accesos (`access.rs:237`, `prepare(&sql)`), porque cada largo sería una entrada nueva.
- **En contra:** la caché es por conexión y de tamaño acotado (`STATEMENT_CACHE_DEFAULT_CAPACITY` = 16 en `rusqlite-0.37.0/src/lib.rs:152`); hay que no llenarla con sentencias poco usadas.

### Estadísticas del planificador

**D. `PRAGMA optimize` al abrir y periódicamente (decisión abierta, sin recomendación todavía).** La documentación de SQLite recomienda `PRAGMA optimize=0x10002` al abrir una conexión de larga vida, `PRAGMA optimize` periódicamente y después de crear índices; no está verificado contra el código de SQLite. El SQLite empaquetado tiene `STAT4` (`build.rs:137`).
- **A favor:** estimaciones reales en lugar de las fijas; evita que otro cambio dependa de `NOT INDEXED` como el de `search.rs:964-975`.
- **En contra:** las estadísticas pueden cambiar planes hoy deterministas que las pruebas fijan. En la reproducción, con datos y `ANALYZE`, la sentencia actual de RECENT pasó de `SEARCH memory USING INDEX idx_memory_agent_active` a `SCAN memory`, y LAST SESSION con `IS` pasó a `idx_memory_scope_project`; con los índices nuevos y `ANALYZE`, la forma con `OR` de LAST SESSION siguió sin buscar por clave. Son datos sintéticos y SQLite 3.51.0, no una predicción para el store real, pero muestran que el riesgo existe. El comentario de `search.rs:964-975` describe justamente un plan que depende de que no haya estadísticas.
- **Cómo decidir sin suponer:** correr las pruebas de plan existentes (`search.rs:3795`, `store_insert.rs:2151`, más las nuevas) sobre un store con `ANALYZE` aplicado. Si algún plan fijado cambia, se descarta y queda anotado con el motivo; si ninguno cambia, se evalúa adoptarlo. Que el cambio funcione sin D es una condición: los índices de A deben ganar sin estadísticas.

## Descartado

| Opción | Razón técnica |
|---|---|
| Reescribir las consultas sobre los índices existentes (B) | Ningún índice de `memory` ordena por `created_at` ni por `memory_type`; el recorrido y el ordenamiento temporal quedan. |
| Guardar en el daemon el último resumen por proyecto en una caché | Tiene que invalidarse en cada camino que cambia qué es «el último»: guardado, borrado, restauración, revival, `purge`, veredictos y reemplazo por clave (siete módulos escriben `status`, `superseded_by` y `deleted_at`, `openspec/changes/doctor-checks/exploration.md:41`). Un error deja un LAST SESSION viejo sin que nada lo muestre; el índice lo mantiene SQLite en la misma transacción. |
| Crear los índices al abrir, fuera de refinery | Esquema que no está en el historial: `schema_plan` compara sólo migraciones (`backup.rs:93`), así que dos stores con el mismo historial tendrían esquemas distintos y el repo perdería su registro de qué versión tiene qué. Además salta la copia `pre-migration`. |
| Índice sin parcial (`(project_id, created_at)` completo) | Indexa todas las filas, incluidas las borradas y las de otros tipos, para servir a una consulta que sólo mira resúmenes; crece con el store entero. |
| Un índice `created_at ASC` | Deja `USE TEMP B-TREE FOR LAST TERM OF ORDER BY` por el desempate `id ASC` (reproducido); el `DESC` no. |
| `ANALYZE` en una migración | Congela estadísticas de un momento en un store que cambia; el mecanismo correcto, si se adopta, es `PRAGMA optimize` en la conexión, no una migración. |

## Interacciones

- **history-import (44).** Multiplica las filas que estas consultas recorren: miles de notas de un agente en un solo día, con 552 resúmenes de sesión en el volumen que midió 44 (`openspec/changes/history-import/exploration.md:17`). Este cambio va antes de 44 para que la primera importación grande no encuentre un recorrido lineal en cada inicio de sesión.
- **session-handoff y agent-lifecycle-hooks (archivados).** Definen LAST SESSION y RECENT (`openspec/specs/retrieval/spec.md`, `openspec/specs/hooks/spec.md`). La salida no debe cambiar: orden, desempate, filtro de autores por `Reach`, ventana de siete días. Este cambio no agrega ni modifica requisitos de esas specs; lo que agrega es la prueba de plan.
- **Backlog «RECENT puede mostrar menos de diez saves» (`openspec/README.md`).** Toca la misma sentencia pero es otro problema: el pool de 200 filas se agrupa en saves después del `LIMIT`, y arreglarlo exige juntar los pedazos por `memory_piece` en SQL o cambiar el pool. **Recomiendo dejarlo afuera:** mezclarlo cambiaría la salida de RECENT, justo lo que este cambio promete no tocar, y su prueba es de otra naturaleza (resultado, no plan). El índice `(agent_id, created_at DESC)` sirve igual a cualquier reescritura posterior que mantenga el orden por `created_at`.
- **save-bounds (54) y secret-coverage (51).** Ambos consideran una V18 (`openspec/changes/save-bounds/exploration.md:68`, opción C; `openspec/changes/secret-coverage/exploration.md:94`, opción I). Esta migración sólo crea índices y no depende de aquéllas ni al revés. La numeración la fija el orden de archivo: el primero que se archive toma V18 y los otros siguen. El nombre del archivo y el número no deben asumirse al escribir las tareas.
- **doctor-checks (31).** Si agrega consultas de invariantes sobre `memory` (`openspec/changes/doctor-checks/exploration.md:41`), esas consultas recorren la tabla y podrían usar los índices parciales o pedir los suyos; no se coordina nada ahora, pero conviene que 31 mire los planes de sus consultas con la misma prueba.
- **outbox-lifecycle (53) y model-reembed.** No tocan estas consultas.

## Alcance candidato

1. Prueba roja de plan para LAST SESSION (con proyecto y con `NULL`) y para RECENT: exigen `SEARCH memory USING INDEX <índice nuevo>` y ningún `TEMP B-TREE`.
2. Migración V18 (o la siguiente libre) con los dos índices parciales, y prueba que siembra un store en V17 con `seed_on_disk`, lo abre en V18 y comprueba que cada fila sigue y los planes usan los índices.
3. `LAST_SESSION_SQL` con `project_id IS ?1` (o dos sentencias, según decisión).
4. `prepare_cached` en esas dos sentencias, y en las otras sentencias fijas calientes sólo si se decide.
5. Prueba de salida idéntica con sellos repetidos (pedazos de un save con el mismo `created_at`) para el desempate `id ASC`.
6. Evaluar las estadísticas (D) con las pruebas de plan; adoptar o descartar con la razón escrita.

### No-alcance

- Arreglar «RECENT puede mostrar menos de diez saves».
- Cambiar el pool, la ventana de siete días, el orden o el texto de los bloques.
- Cualquier cambio de ranking; no se corre `rag-eval` para decidir.
- Una caché del último resumen en el daemon.
- Índices para otras consultas de `memory` (el dedup, la clave de tema y los KNN ya tienen los suyos o no son de este cambio).
- Pasar por la caché las sentencias de lista `IN` variable.

## Decisiones abiertas

Son del usuario.

- [ ] **Forma del predicado de LAST SESSION.** `project_id IS ?1` en una sola sentencia (recomendado: mismo resultado para valor y `NULL`, plan reproducido) o dos sentencias, una por caso.
- [ ] **`status` en el predicado de los índices.** Dejarlo fuera (más simple, el índice incluye las filas `superseded` que la sentencia descarta) o sumar `status IN ('active', 'disputed')` (índices más chicos, texto acoplado).
- [ ] **Alcance de `prepare_cached`.** Sólo las dos sentencias del cambio, o también los KNN de `search.rs` y las de `outbox_record`.
- [ ] **Estadísticas.** Evaluar `PRAGMA optimize` / `ANALYZE` con las pruebas de plan existentes y adoptar sólo si ninguna cambia; o descartarlo de entrada y dejar escrito por qué.
- [ ] **Número de la migración** (V18 o la que quede libre) según el orden de archivo respecto de 54 y 51.
- [ ] **Backlog de RECENT.** Dejarlo afuera (recomendado) o absorberlo.
- [ ] **Autor de LAST SESSION en SQL.** Dejar el filtro en Rust (alcanza mientras los resúmenes ocultos sean pocos) o llevar el predicado de compartición a `LAST_SESSION_SQL` con `LIMIT 1`, con una prueba que lo compare con `Reach::admits`.
- [ ] **Medición propia.** Cuánto de la medición con el SQLite empaquetado y filas generadas se escribe como prueba o `#[ignore]` y cuánto sólo se anota, y si se mide sobre un perfil real (requiere un perfil de datos y su autorización).

## Cómo se mide

Todo con el embedder falso y sin red ni perfil real.

1. **Plan (RED).** Una prueba por sentencia, leyendo `EXPLAIN QUERY PLAN` de la constante de producción, como `reinforce_lookup_uses_dedup_index`: para LAST SESSION con proyecto y con `NULL`, y para RECENT, exige `SEARCH memory USING INDEX <nuevo>` y la ausencia de `TEMP B-TREE`. Hoy falla: el plan es el de `idx_memory_agent_active` con árbol temporal. Es la prueba que fija el plan en 3.50.2, no la reproducción hecha con Python.
2. **Migración.** Sembrar un store en V17 (`seed_on_disk(dir, Some(17))`), abrirlo con V18: toda fila sigue, el `pre-migration` se escribe, y los planes del punto 1 usan los índices.
3. **Salida idéntica.** Las pruebas existentes de LAST SESSION y RECENT pasan sin tocarlas; una nueva con filas del mismo `created_at` (pedazos de un save, `id ASC` dentro del sello) compara el orden con el de la sentencia antigua, en datos y no en el plan.
4. **Estadísticas.** Las pruebas de plan existentes (`search.rs:3795`, `store_insert.rs:2151` y las nuevas) sobre un store al que se le aplicó `ANALYZE`: si ninguna cambia, se evalúa adoptar; si alguna cambia, se descarta con su razón.
5. **Tiempo.** Con el SQLite empaquetado, filas generadas y 2 000 / 10 000 / 50 000 filas, antes y después, para LAST SESSION y RECENT; es una medición de orden de magnitud, no un umbral de aprobación.
6. **`rag-eval --agent-view`** (opcional): mostraría los prompts idénticos byte a byte; no hace falta para decidir porque nada del ranking cambia.

## Riesgos

- **Que el índice no se use.** Ya pasó en la reproducción con el predicado actual de LAST SESSION; por eso el plan se fija con una prueba y no se da por supuesto.
- **Una migración para un cambio S.** Copia `pre-migration` y un binario anterior que ya no abre el store; el camino de vuelta es el snapshot que `clawg update` restaura. No hay pérdida de datos, pero el costo de la migración es el de cualquiera.
- **Costo de escritura y de construcción del índice sin medir.** Dos índices más por guardado; la construcción sobre un store grande ocurre durante el arranque, detrás del estado de carga. Se mide en el paso 5.
- **Orden de salida con sellos repetidos.** El desempate `id ASC` lo cubre la hipótesis del índice `DESC`; si los datos la desmienten, se vuelve a una de las dos formas con árbol temporal o se agrega `id` al índice.
- **Estadísticas.** Pueden cambiar planes fijados por pruebas y por el comentario de `search.rs:964-975`; por eso es una decisión abierta con criterio de descarte, no una mejora por defecto.
- **La reproducción no es la prueba.** SQLite 3.51.0, datos sintéticos y funciones simuladas: sirven para elegir la forma del índice y de la sentencia, no para afirmar el plan del binario.
