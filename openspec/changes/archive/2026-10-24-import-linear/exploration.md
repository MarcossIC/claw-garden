# import-linear — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (va antes de import-decision-pass) | exploración (borrador) |

**Tesis:** el tiempo de una importación crece con el cuadrado de las filas porque la consulta que decide si una fila ya está aquí (`PRESENT_SQL`) no puede usar ningún índice: no lleva `deleted_at IS NULL`, y el único índice que cubre su hash y su fecha, `idx_memory_dedup`, es parcial (`WHERE deleted_at IS NULL AND content_hash IS NOT NULL`). El planificador recorre entonces toda la tabla `memory` por cada fila del archivo, dos veces (el pase de decisión y el de escritura), y la tabla crece con lo que el propio pase inserta. Es la causa de los ~800 s sin respuesta del pase de decisión y de los ~920 s de escritura de una importación real de 52 080 filas, y de que 2 000 notas (15 047 filas) tarden 9,5 veces lo que 1 000 (4 655 filas) con 3,2 veces las filas. El cambio reconstruye `idx_memory_dedup` sin su `WHERE` (misma lista de columnas), de modo que sirva a `PRESENT_SQL` sin dejar de servir a las búsquedas del worker, y fija con una prueba de plan que ninguna consulta por fila de la importación recorre `memory`. No toca el modelo de escritura, el orden de los pases ni el ranking. Es de tamaño S: una migración de un índice, una prueba de planes y una prueba de migración.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `PRESENT_SQL` no tiene término `deleted_at` y por eso no puede usar el índice parcial | **Cierta.** `crates/clawg-core/src/memory/portability/import.rs:494-497`: `agent_id IS ?1 AND COALESCE(project_id, '') = COALESCE(?2, '') AND content_hash = ?3 AND created_at = ?4`. Debe ver también las filas borradas (una fila borrada aquí cuenta como presente, `find` lo trata en `:443-446`), así que agregar `deleted_at IS NULL` cambiaría el resultado. |
| `idx_memory_dedup` es parcial y lleva la expresión `COALESCE(project_id, '')` | **Cierta.** `crates/clawg-core/migrations/V20__project_scoped_indexes.sql:8-11`: `(COALESCE(project_id, ''), content_hash, created_at) WHERE deleted_at IS NULL AND content_hash IS NOT NULL`. Es la tercera forma del índice (V6 `:14`, V18 `:21-24`, V20). |
| Las líneas citadas de `import.rs` siguen vigentes | **Cierta.** `ORIGIN_SQL` `:405`, `REVISED_SQL` `:409-410`, `find` `:415`, `present_at` `:476`, `PRESENT_SQL` `:494`, `TWIN_SQL` `:501`, `TOPIC_HOLDERS_SQL` `:508`, `place` `:552-650`, `place_rows` `:656-714`, `write_links` `:720`, `note_batches` `:938`, `write` `:957-1038`. `PRESENT_SQL` se citaba en `:236` en el cambio archivado de la importación; hoy es `:494`. |
| Las consultas por fila usan `tx.query_row`, que prepara de nuevo cada vez | **Cierta, con tres sitios y no dos.** `PRESENT_SQL` (`:572`) y `TWIN_SQL` (`:579`) en `place`, y `TWIN_SQL` otra vez en `present_at` (`:481`). `ORIGIN_SQL`, `TOPIC_HOLDERS_SQL`, `INSERT_SQL` y `REVISED_SQL` ya usan `prepare_cached` (`:417`, `:584`, `:602`, `:709`). |
| El pase de decisión corre en una sola transacción que se descarta | **Cierta.** `import.rs:966-977`: `record_projects`, `place_rows` sobre todas las filas y `write_links` dentro de un `conn.call`, y el `Ledger` se devuelve sin `commit`. Por eso sus propias inserciones agrandan la tabla que sus consultas recorren. Cambiar eso es de import-decision-pass, no de este cambio. |
| La escritura repite `place_rows` por lote | **Cierta.** `import.rs:1008-1024`, lotes de `WRITE_BATCH` = 256 (`:55`) con un `commit` cada uno; los vectores salen de `to_embed` (`:984-997`, `EMBED_BATCH` = 32, `:57`). |
| Las demás consultas de la importación ya buscan por índice | **Plausible por construcción, sin plan medido hoy.** `ORIGIN_SQL` implica el `WHERE source = 'import'` de `idx_memory_import_origin` (`V23__origin_project.sql:18-19`); `TOPIC_HOLDERS_SQL` implica el de `idx_memory_topic` (`V20:14-16`); `TWIN_SQL` lleva `deleted_at IS NULL` y `content_hash = ?`, que implica `IS NOT NULL`, y la spec ya lo nombra como usuario de `idx_memory_dedup` (`openspec/specs/storage/spec.md:397`). `write_links` sólo toca `memory` por `id` (rowid), `memory_piece` por `idx_memory_piece_memory` (`V11:20`) o su clave primaria (`MAX(save_id)`), `memory_derivation` y `memory_conflict` por clave primaria. **Ninguna de estas tiene hoy una prueba de plan**; la prueba nueva las cubre. |
| El worker y otros lectores del índice cambian de plan si deja de ser parcial | **No, según los planes que fijan sus pruebas.** `REINFORCE_LOOKUP_SQL` (`store_insert.rs:308-318`, usada en `:394` y `:469`) pide igualdad sobre `COALESCE(project_id, '')` y `content_hash` y ordena por `created_at`; el índice sirve las tres cosas con o sin `WHERE` (`reinforce_lookup_uses_dedup_index`, `store_insert.rs:2215-2239`, exige `idx_memory_dedup` y ningún `TEMP B-TREE`). Lo mismo `RESTORE_TWIN` (`admin.rs:540-547`) y `TARGET_TWIN_SQL` (`relocate.rs:267-272`). La diferencia es que las entradas de filas borradas o sin hash quedan en el índice y se descartan como filtro residual. |
| El esquema no tiene `ANALYZE`, así que el planificador elige por heurística | **Cierta.** El comentario de `search.rs:986` y el de `:3800` lo dicen. Es la razón de que la prueba de plan sea la red de seguridad: el plan de una consulta nueva sobre un índice nuevo no se supone, se lee. |
| `db.rs:1407` fija la forma del índice | **Cierta, y no se rompe.** Es la prueba de V20 (`a_store_from_before_project_scoped_rules_keeps_its_rows`): lee `sqlite_master` y afirma `contains("(COALESCE(project_id, ''), content_hash, created_at)")` (`db.rs:1422`). No mira el `WHERE`; abre el store con el último esquema, así que con V25 sigue pasando si se conserva la lista de columnas. |
| `dedup.rs:153` depende del índice | **No.** Es una prueba (`backfilled_row_becomes_lookup_visible`) con su propia copia de la sentencia: mira que una fila con `content_hash` NULL sea invisible a la consulta, no el índice. |
| `doctor` revisa índices | **No.** `rg` sobre `crates/clawg-cli/src/doctor/` no encuentra `idx_` ni «index». Lo más cercano es `hash_duplicates` (`crates/clawg-core/src/memory/inspect.rs:303-311`), una ventana sobre `COALESCE(project_id, ''), content_hash` de las filas vivas; un índice con esa misma clave sólo puede ayudarle. |
| La última migración es V24 | **Cierta.** `crates/clawg-core/migrations/V24__project_remote.sql`; la nueva es V25. Tres pruebas cuentan migraciones y suben a 25 en el mismo commit: `db.rs:702`, `:2130` y `:2591`; y `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs:43-81`) gana su fila. |
| El tamaño del índice no importa | **Verificado a la baja.** Cada entrada lleva el proyecto (`COALESCE(project_id, '')`), el hash de 64 caracteres hexadecimales (`dedup.rs:99`), `created_at` y el rowid: del orden de 100 bytes, unos 5 MB para 52 000 filas. Lo que el índice no parcial suma son las filas borradas y las de `content_hash` NULL; el respaldo de V6 dejó hash a todas (`dedup.rs:214`) y las importadas lo llevan, así que sólo las borradas pesan, y son una minoría. |
| Una prueba de tiempo puede ser la RED de este cambio | **No.** Un cociente de tiempos es sensible a la máquina y al perfil de compilación. La RED determinista es el plan (`SCAN memory`); el tiempo es la medición de aceptación. |

## Problema (verificado contra el código)

1. **`PRESENT_SQL` recorre `memory` por cada fila.** Sin término `deleted_at`, el índice parcial no es candidato (el planificador sólo usa un índice parcial cuando el `WHERE` de la consulta implica el del índice). Los otros índices que empiezan por `agent_id` (`idx_memory_agent_active`, `V1__init.sql:37-39`, y `idx_memory_agent_created`, `V18:13-15`) son parciales también y no ayudan. El plan es `SCAN memory`. Se ejecuta una vez por fila que `place` no resuelve antes (`import.rs:571-573`), y `find` ya no la ahorra para las notas nuevas (`Found::New` cae en `place`, `:693`).
2. **Dos pases, ambos cuadráticos.** El pase de decisión (`:966-977`) coloca todas las filas, y su tabla crece con cada inserción de la propia transacción; el de escritura (`:1008-1024`) lo repite por lote. Con N filas hacen del orden de N² visitas de fila cada uno.
3. **Coincide con la medición.** Importación real de una exportación completa (3 814 notas, 52 080 filas) en un perfil de desarrollo, compilación de depuración de la 0.56.0, en Windows: 910 s en seco y 3 078 s real. Durante el pase de decisión (~800 s) toda petición al store venció, porque ocupa la única conexión (`db.rs:149`) con un `call` (el `/health` contestaba `ok` sin su bloque `ingest`); durante la escritura (~920 s) las búsquedas subieron de 2,9 s a más de 15 s a partir del minuto 42. Con el embedder falso en un `CLAWG_TEST_HOME` descartable: 1 000 notas / 4 655 filas en 10,3 s y 2 000 notas / 15 047 filas en 97,8 s. Con un índice de prueba `(content_hash, created_at)` creado a mano en ese store: 5,1 s y 16,4 s, lineal, con el mismo informe byte por byte; `EXPLAIN QUERY PLAN` pasó de `SCAN memory` a `SEARCH … USING INDEX`.
4. **Nada de lo demás es cuadrático.** Las 5,1 s y 16,4 s dan ~1,1 ms por fila para los dos pases juntos en depuración y con embedder falso. Extrapolado (no medido) a 52 080 filas, el trabajo de SQLite serían del orden de 1 minuto, frente a los ~1 720 s que hoy suman los dos pases; lo que quedaría de la importación son las traducciones (~75 s) y los embeddings (~1 280 s), que son de otros cambios.

## Lo que ya existe y se reutiliza

- El patrón de prueba de plan: `reinforce_lookup_uses_dedup_index` (`store_insert.rs:2215-2239`) explica la constante de producción, nunca una transcripción, y exige un índice por nombre y ningún `TEMP B-TREE`; `plan_of` (`prompt_builder.rs:3828-3838`) devuelve el `detail` de un plan y `the_session_statements_read_the_newest_rows_by_index_without_sorting` (`:3844`) recorre varias sentencias en una tabla de casos. El cambio archivado query-plans (`openspec/changes/archive/2026-10-21-query-plans/`) hizo lo mismo con V18 y también reconstruyó `idx_memory_dedup`.
- `rusqlite::StatementStatus::FullscanStep` (`rusqlite-0.37.0/src/statement.rs:886`, `Statement::get_status`, `:725`): un contador determinista de pasos de recorrido completo, sin tiempo, para una sentencia preparada. Sirve de segunda aserción independiente del texto del plan.
- La siembra de una base en una versión anterior: `seed_on_disk(dir, Some(n))` (`db.rs:478`), usada por `a_database_from_the_previous_version_opens_with_no_remote_recorded` (`db.rs:1613-1664`, con `Some(23)`), que compara las filas antes y después y confirma que migrar no cambia ninguna.
- Las pruebas de importación de `import.rs` (`mod tests`, `:1149`): `run` (`:1211`) con `FakeEmbedder`, `destination` (`:1180`) y las fábricas de notas de `translate::tests` (`export`, `note`), para construir un archivo de N notas sin red ni modelo.
- El mismo patrón de migración de sólo índices con vuelta atrás escrita (V18, V20): sin filas tocadas, `pre-migration` antes (`CLAUDE.md`, «Data profiles»), `SchemaNewer` en un binario anterior y la copia como camino de vuelta.

## Enfoques posibles

### A. Un segundo índice no parcial `(COALESCE(project_id, ''), content_hash, created_at)` y dejar el del worker

PRESENT_SQL pasa a usar el nuevo; el parcial sigue sirviendo al worker. Es el cambio más conservador respecto de los planes del worker, pero deja **dos índices con la misma clave** que se mantienen en cada inserción (el costo de escritura de la importación y del worker se duplica en este par) y, sin estadísticas, el planificador puede elegir cualquiera de los dos para las consultas del worker y de `TWIN_SQL`, con lo que sus planes dejan de ser deterministas.

### B. Reconstruir `idx_memory_dedup` sin `WHERE`, con la misma clave (recomendado)

`V25__dedup_index_all_rows.sql`: `DROP INDEX IF EXISTS idx_memory_dedup; CREATE INDEX IF NOT EXISTS idx_memory_dedup ON memory (COALESCE(project_id, ''), content_hash, created_at);`. Un solo índice sirve a `PRESENT_SQL` (igualdad en las tres columnas; el orden por `id` es el del rowid dentro de una clave igual, sin ordenar), a `TWIN_SQL`, a `REINFORCE_LOOKUP_SQL`, a `RESTORE_TWIN` y a `TARGET_TWIN_SQL` (que ordenan por `created_at` y lo reciben de la clave). Costo: las filas borradas y las de hash NULL entran en el índice (ver la tabla de premisas), y los lookups del worker descartan por filtro residual las entradas borradas de un mismo texto, que son pocas. Una sola definición, un solo plan por consulta. **Los nombres de las pruebas y de la spec no cambian**; cambia el predicado que dicen las notas de V18/V20 («parcial»), que se actualiza al archivar.

### C. Cambiar `PRESENT_SQL` para que el índice parcial le sirva

Dos consultas, una con `deleted_at IS NULL` y otra con `deleted_at IS NOT NULL` (para ésta haría falta otro índice parcial): es el enfoque A con más código. Se descarta.

### D. Resolver las presencias en memoria (cargar los hashes del proyecto en un `HashSet` antes del pase)

No arregla la consulta, mueve la cuenta a RAM (decenas de miles de hashes de 64 caracteres por proyecto) y tiene que mantenerse al día con las inserciones del propio pase. Es una solución posible dentro de import-decision-pass, que es dueño de lo que el pase hace; aquí se descarta.

### E. `prepare_cached` en las consultas por fila

`query_row` prepara cada vez; `prepare_cached` evita eso en `PRESENT_SQL` y `TWIN_SQL` (`import.rs:481`, `:572`, `:579`). Con el índice puesto, el ahorro es la preparación de una sentencia de un `SELECT` simple por fila, que el banco de 5,1 s y 16,4 s ya muestra lineal **sin** ese cambio. Ninguna prueba falla hoy por esto (es refactor bajo verde, no tiene RED). Se incluye sólo si, con el índice, la medición de aceptación muestra la preparación en el perfil; si no, se omite.

### Recomendación: B, con la prueba de planes de toda la importación, y E sólo si la medición lo pide

1. **Prueba de planes (RED).** Una prueba que explica `PRESENT_SQL`, `TWIN_SQL`, `TOPIC_HOLDERS_SQL`, `ORIGIN_SQL` y `REVISED_SQL` de `import.rs`, y las demás sentencias por fila de `write_links` y de `place` que tocan `memory` por columna que no es el `id`; falla con `SCAN memory` y exige el índice esperado por consulta (dedup, tema, origen). Hoy falla sólo para `PRESENT_SQL` (las otras, por construcción, ya pasan; la prueba lo confirma o lo desmiente). El `REINFORCE_LOOKUP_SQL` del worker queda con su prueba actual.
2. **Migración V25** con su prueba de siembra en V24.
3. **Aceptación** con la medición de abajo.

## Descartado

| Opción | Razón técnica |
|---|---|
| Un índice `UNIQUE` y `INSERT … ON CONFLICT DO NOTHING` | Las reglas de dedup y de `topic_key` descansan en un escritor serializado y no son únicas en el modelo de datos (`V6__content_hash.sql:9-13`, `store_insert.rs:356-369`): filas de autores distintos con el mismo texto conviven en bases antiguas, y un `UNIQUE` rechazaría abrirlas. Ya descartado con evidencia. |
| Segundo índice no parcial junto al parcial (A) | Dos índices con la misma clave: doble costo de escritura y plan no determinista sin estadísticas (el esquema no tiene `ANALYZE`, `search.rs:986`). |
| Hacer que `PRESENT_SQL` implique el índice parcial (C) | Cambia el resultado (las filas borradas deben contar como presentes) o exige otro índice parcial para el caso contrario. |
| `ANALYZE` / `PRAGMA optimize` | El esquema no lo usa; el cambio archivado query-plans lo evaluó y lo descartó. Con el índice presente el plan ya es el correcto, y las estadísticas habría que mantenerlas en cada store. |
| `busy_timeout` | Ya hay 5 s por conexión; no es una falta de espera, es trabajo cuadrático. |
| Reescribir el pase de decisión | Es el objeto de import-decision-pass. Este cambio hace lineales las consultas; aquél decide qué hace el pase con ellas. |
| Una prueba de tiempo como RED | Depende de la máquina y del perfil de compilación; el plan y el contador de recorridos son deterministas. |

## Interacciones

- **import-decision-pass (82).** Va después y se apoya en consultas que ya buscan por índice: con ellas lineales, lo que queda del pase es insertar ~52 000 filas dentro de una transacción que se descarta y retener la conexión mientras tanto (`import.rs:966-977`). Este cambio **no** quita esa retención (las peticiones al store siguen esperando detrás del `call`); sólo la acorta de ~800 s a decenas de segundos. 82 decide cómo no hacer esa pasada o cómo no bloquear.
- **read-connections (57).** Su puerta mide la espera detrás de una operación larga. Con 81 la operación larga más frecuente de una importación deja de ser de minutos; conviene que 57 se mida después, y la importación sirve de caso de prueba para la espera residual. 81 no la resuelve ni la requiere.
- **query-plans (56, archivada).** Antecedente directo: V18 reconstruyó el mismo índice (`V18__session_indexes.sql:21-24`) para que el dedup no perdiera su plan frente a un índice nuevo, y dejó fuera «índices para otras consultas de `memory`» (su propuesta, «Fuera de alcance»). 81 es esa otra consulta. La spec de storage (`openspec/specs/storage/spec.md:375`, `:397`) describe `idx_memory_dedup` como parcial: al archivar se agrega la sección de V25 y se corrige esa descripción.
- **perf-bench (74).** El banco de importación de abajo es un escenario natural de 74 (siembra con semilla, binario real, embedder falso, JSON con entorno). Hasta que 74 exista, la medición vive como prueba ignorada de este cambio; cuando 74 llegue, la adopta. Ninguno bloquea al otro.
- **piece-boundaries (83) e import-undo (84).** Pueden agregar migraciones. El número V25 se toma al integrar: el que llegue segundo renumera el suyo y su checksum fijado, y las tres pruebas que cuentan migraciones suben con cada uno. No hay dependencia lógica.
- **embed-throughput (85), import-job (87).** Sin relación con el índice. 85 reduce lo que quedará como mayor parte del tiempo (los embeddings); 87 es el trabajo en segundo plano de la importación.
- **Specs:** `openspec/specs/storage/spec.md` (V25 y su vuelta atrás; el índice deja de describirse como parcial) y `openspec/specs/ingest/spec.md` (la importación hace sus consultas por índice), al archivar.

## Alcance candidato

1. `crates/clawg-core/migrations/V25__dedup_index_all_rows.sql`: `idx_memory_dedup` sin `WHERE`, misma lista de columnas. Sólo índices, ninguna fila cambia. Con la fila nueva de `published_migration_checksums_are_pinned`, el toque a `migrations.rs` que fuerza el recompilado (su cabecera lo exige) y las tres cuentas de migraciones (`db.rs:702`, `:2130`, `:2591`) subidas a 25.
2. Prueba de migración: `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows`, con `seed_on_disk(dir, Some(24))`, filas vivas, una borrada y una de hash NULL; abre con `MemoryStore::open`; compara las filas antes y después; el `sql` del índice en `sqlite_master` ya no contiene `WHERE`; hay una copia `pre-migration`.
3. Prueba de planes de la importación: ninguna sentencia por fila de `import.rs` produce `SCAN memory`, y cada una nombra su índice. Misma tabla de casos que `prompt_builder.rs:3844`.
4. Prueba del contador de recorridos (`FullscanStep` = 0) de `PRESENT_SQL` y `TWIN_SQL` ejecutadas contra un store con filas.
5. Banco de escala, ignorado: importa N y 2N notas con el embedder falso y compara tiempos y que el informe sea el mismo.
6. Si la medición lo pide: `prepare_cached` en `import.rs:481`, `:572`, `:579`.
7. Spec de storage e ingest, al archivar.

### No-alcance

- El orden y el contenido de los pases de la importación, la retención de la conexión, la transacción que se descarta (import-decision-pass).
- Índices para otras consultas de `memory` que la importación no usa.
- `prepare_cached` en las sentencias del worker (`store_insert.rs:468`), que no están en el camino cuadrático.
- Embeddings, piezas, ranking: no se corre `rag-eval`, porque ninguna salida cambia.
- Cambiar la forma de `idx_memory_topic` o `idx_memory_import_origin`.
- Deshacer una importación.

## Decisiones abiertas

Son del usuario.

- [ ] **Índice.** B (reconstruir `idx_memory_dedup` sin `WHERE`, recomendado: un solo índice, un solo plan por consulta) o A (un segundo índice, deja intacto el plan del worker a costa de escrituras dobles y de un plan sin determinar).
- [ ] **Dónde vive la medición de escala.** Una prueba `#[ignore]` en `import.rs` (recomendado: sin binario, con `FakeEmbedder`, y se corre con `cargo test -p clawg-core --release --lib -- --ignored`), una prueba `#[ignore]` en `clawg-cli/tests` a la manera de `transport_bar.rs` (binario real, más cercana a `clawg import`), o esperar al escenario de 74 y medir a mano mientras tanto.
- [ ] **Umbral del banco.** Qué cociente cuenta como lineal. Candidato: duplicar las notas no más que triplica el tiempo (hoy lo multiplica por ~9,5 en depuración; lineal daría ~2).
- [ ] **`prepare_cached`.** Incluirlo siempre (coherente con el resto del archivo, sin RED), sólo si la medición lo pide (recomendado) u omitirlo.
- [ ] **Verificación con el perfil de desarrollo.** Una exportación nueva (`engram export --all`, guardada fuera del repositorio y borrada al terminar, porque contiene el texto sin enmascarar) importada en el perfil de desarrollo con compilación de depuración, para comparar con los 3 078 s. Pide autorización explícita para usar ese perfil.

## Cómo se mide

RED que fallan hoy:

1. `the_import_statements_seek_an_index` (nombre candidato): la tabla de casos explica cada constante de `import.rs` por fila y afirma el índice y la ausencia de `SCAN memory`. Falla hoy en `PRESENT_SQL` (plan `SCAN memory`).
2. `present_lookup_does_not_scan` (nombre candidato): `PRESENT_SQL` preparada y ejecutada contra un store con filas da `FullscanStep == 0`. Falla hoy (un recorrido completo por ejecución).
3. `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows`: falla hoy porque el esquema no llega a 25 y el índice conserva su `WHERE`.
4. Las pruebas existentes que no se tocan y deben seguir pasando: `reinforce_lookup_uses_dedup_index`, `a_store_from_before_project_scoped_rules_keeps_its_rows`, `a_second_import_adds_nothing`, `a_live_twin_absorbs_the_imported_row`, `a_taken_topic_key_supersedes_the_imported_row`.

Medición de aceptación (no es una prueba de CI):

- **Banco con embedder falso:** 1 000 y 2 000 notas (y 4 000 si cabe) en un `CLAWG_TEST_HOME` descartable, en depuración como la medición de origen. Esperado: tiempo lineal (referencia 5,1 s y 16,4 s con un índice de prueba) y el **mismo informe** que antes del cambio, comparado campo a campo.
- **Importación real en el perfil de desarrollo,** compilación de depuración, de la misma exportación completa; se compara con los 3 078 s y los 910 s en seco. Se espera que las fases de decisión (~800 s) y de escritura (~920 s) caigan a una fracción; las de traducción y embeddings, no. Se registra también la sonda de latencia por fase (`/health`, `GET /v1/memories?limit=1`, `POST /v1/search`) para 82 y 57.
- Los tiempos se anotan con la compilación, el sistema y la máquina; no se mezclan con los de un build de release.

## Riesgos

- **El planificador elige otro índice para `PRESENT_SQL`.** `agent_id IS ?1` podría atraer un índice por agente; sin `ANALYZE` la elección es heurística (`search.rs:986`). Mitigación: la prueba de plan nombra el índice, no sólo la ausencia de `SCAN`.
- **Los lookups del worker leen entradas de filas borradas del mismo texto.** Se descartan como filtro residual. Sólo importa con miles de copias borradas o reemplazadas de un mismo texto en un proyecto (hoy las reemplazadas ya están en el índice parcial). `reinforce_lookup_uses_dedup_index` sigue exigiendo el plan sin ordenar.
- **Reconstruir el índice en un store grande.** Una pasada sobre la tabla dentro de la transacción de la migración, después de la copia `pre-migration`; del orden de segundos para 50 000 filas, a medir en el banco.
- **Un binario anterior no abre el store migrado.** Rechaza V25 sin tocarlo (`SchemaNewer`); la vuelta atrás es la copia `pre-migration`, que `clawg update` restaura si el binario viejo se niega, o a mano sin perder filas: `DROP INDEX idx_memory_dedup; CREATE INDEX idx_memory_dedup ON memory (COALESCE(project_id, ''), content_hash, created_at) WHERE deleted_at IS NULL AND content_hash IS NOT NULL; DELETE FROM refinery_schema_history WHERE version = 25;`.
- **Que lo cuadrático no sea sólo `PRESENT_SQL`.** Las 5,1 s y 16,4 s medidas con un índice de prueba lo descartan para esos tamaños, pero a 52 000 filas puede aparecer otro término. La aceptación con el perfil real es la que lo confirma; si queda un residuo, es de 82.
- **Medición sólo en depuración.** Los números absolutos de la compilación de depuración no se trasladan a release; la comparación válida es antes y después en la misma compilación.
