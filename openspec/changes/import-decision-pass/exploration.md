# import-decision-pass — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar y clasificar), importación de un historial grande | M | import-linear (el índice que vuelve lineal cada consulta del pase); se coordina con read-connections, shutdown-budget (archivado) e import-undo | exploración (borrador) |

**Tesis:** el pase de decisión de una importación recorre todas las filas del archivo dentro de **una sola** llamada a la conexión única del daemon (`crates/clawg-core/src/memory/portability/import.rs:966-977`), así que su duración crece con el archivo y todo lo demás queda detrás: un guardado, una búsqueda, el `/health` y el hook `session-start`, que tiene 3 s en total. Ese pase es además **sólo informativo**: abre una transacción, inserta filas en `memory` y la suelta sin confirmar (`:969-974`); lo único que sale de él es el `Ledger` (dónde cae cada fila). Ninguna de sus escrituras influye en el resultado salvo para que una fila vea a las anteriores del mismo archivo. El cambio quita al pase la conexión y la transacción: un **planificador en memoria** aplica las mismas reglas de `place()` sobre estado Rust que se carga con lecturas acotadas (cada una su propia llamada corta, que se intercala con el resto del tráfico) y que se actualiza al aceptar cada fila. La tabla y la transacción dejan de existir en el pase de decisión, y con ellas el riesgo que descarta partir el pase en lotes sobre una transacción abierta. Es de tamaño M: un asiento (trait) para que las reglas sigan siendo una sola, la carga acotada, la separación de `write_links` y un diseño de pruebas de equivalencia contra el pase actual. La alternativa de abrir otra conexión (C) se descarta como solución principal por lo que hace sin WAL y por lo que cuesta al `checkpoint`.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El pase de decisión es una sola `conn.call` cuya transacción se descarta | **Cierta.** `write` (`import.rs:957`) hace `record_projects`, `place_rows` sobre `0..rows.len()` y `write_links` dentro de `store.conn.call` (`:966-975`) y devuelve el `Ledger` sin `commit` (`:973`). |
| La conexión es una sola y atiende en orden de llegada | **Cierta.** `pub conn: Connection` (`crates/clawg-core/src/memory/db.rs:149`); `tokio-rusqlite` manda cada clausura a un canal sin tope (`tokio-rusqlite-0.7.0/src/lib.rs:290-304`, `:376`) y un hilo las corre en orden. |
| Partir el pase en lotes con la transacción abierta entre llamadas no sirve | **Cierta.** La conexión es de todos: lo que entre en la cola entre dos lotes corre dentro de la transacción abierta y un guardado hecho en el medio se pierde con el `ROLLBACK`. No hay forma de pedir «mi transacción» entre dos `call`. |
| El pase tarda del orden de un minuto en debug con 52 080 filas | **Consistente con el banco, no medida directamente.** El banco con embedder falso y el índice de prueba dio 16,4 s para 15 047 filas, es decir ~1,1 ms por fila; 52 080 filas dan ~57 s. Los números no están en el repo todavía. La medición real sin índice fue ~800 s (el pase) en una importación de 52 080 filas. |
| `MAX_IMPORT_BYTES` deja un archivo unas 18 veces mayor | **Sólo el límite se verifica:** `256 * 1024 * 1024` (`import.rs:51`), aplicado por el daemon (`crates/clawg-cli/src/daemon/api.rs:1898`) con un cuerpo de hasta el doble (`:1866`). El tamaño del archivo medido no consta en el repo, así que el «18 veces» no se puede comprobar aquí. Lineal en filas, un archivo así daría ~17 min de pase en debug. |
| El hook se queda sin respuesta si la cola se atrasa | **Cierta.** `HOOK_DEADLINE` es 3 s (`crates/clawg-cli/src/hook.rs:26`); al vencer imprime nada y sale con 0 (`.claude/CLAUDE.md`, `clawg hook`). |
| El pase escribe `project`, `project_remote`, `memory` (y sus triggers), enlaces y el `Ledger` | **Cierta, y casi nada de eso cambia el `Ledger`.** `record_projects` inserta `project` y `project_remote` (`:526-546`) pero sólo devuelve `taken`, que sale de leer `project` (`:533-536`). `write_links` hace `UPDATE`/`INSERT` (`:734-866`) pero lo que mete en `dropped` depende de `ledger.placed`, `ledger.absent` y de una sola lectura de `memory_piece` (`:850`). Cada `INSERT` en `memory` dispara además el trigger de FTS (`crates/clawg-core/migrations/V8__memory_fts.sql:63-66`): trabajo que el pase tira. |
| Las filas insertadas en el pase las ven las filas siguientes | **Cierta y es lo que obliga al estado.** `TWIN_SQL` (`:501`) y `TOPIC_HOLDERS_SQL` (`:508`) leen filas que `place` insertó antes (`:602-629`); `REVISED_SQL` (`:409`) pasa filas vivas a `superseded` y las deja de ver el `TWIN_SQL` siguiente; `ORIGIN_SQL` (`:405`) ve las filas de un mismo origen insertadas antes; el `INSERT` de una fila borrada en el origen se hace vivo y luego se borra (`:631-637`), así que `PRESENT_SQL` (`:494`, sin término `deleted_at`) la ve y `TWIN_SQL` no. |
| El `dry_run` y la importación real comparten el pase | **Cierta, y la prueba exige ids idénticos.** `write` es la misma función; `dry_run` sale con el `Ledger` del pase (`:979-981`). Los ids del informe (`merged.into`, `topic_taken.stored_as` y `by`) son los de las filas del pase, y `a_dry_run_reports_without_writing` (`import.rs:1384-1394`), `an_imported_note_has_the_rows_of_a_save_of_its_text` (`:1843-1855`) y `an_engram_import_stores_notes_and_a_dry_run_reports_the_same` (`crates/clawg-cli/tests/api.rs:3646-3670`) comparan el informe de la corrida en seco con el de la real campo a campo. Los ids salen de `AUTOINCREMENT` (`crates/clawg-core/migrations/V1__init.sql:5`), que se revierte con la transacción: el planificador debe partir de `sqlite_sequence`, no de `MAX(id)`, o difiere en cuanto se haya purgado una fila. |
| El pase escribe en la base lo que luego se embebe | **Falsa en lo que importa.** Del `Ledger` sólo se usa `placed` para elegir las filas a embeber (`:984-988`); el vector no se inserta en el pase (`vectors` vacío y `needs_vector = false`, `:971`). Nada de lo escrito en el pase sobrevive al `drop`. |
| Una segunda conexión con tablas TEMP necesita `sqlite-vec` o las UDF | **No.** `sqlite-vec` se registra con `sqlite3_auto_extension` para toda apertura posterior (`db.rs:128-136`) y `backup::open_existing` ya lo asegura (`crates/clawg-core/src/memory/backup.rs:607`). Las UDF (`register_udfs`, `db.rs:400-405`: `content_hash`, `compute_tier`, `demote_tier`) no las llama el pase: el hash se calcula en Rust (`:305`) y el `INSERT` (`:516-521`) no usa ninguna. Pero `open_existing` abre con `READ_WRITE` y sin `query_only` (`backup.rs:608-611`). |
| Sin WAL el sistema sigue andando | **Cierta y decide contra C.** `open` sólo avisa y sigue con el diario de reversa (`db.rs:217-220`). En ese modo una conexión que lee un minuto sostiene el bloqueo compartido y el `COMMIT` del worker de ingest espera hasta el `busy_timeout` de 5 s de rusqlite (`rusqlite-0.37.0/src/inner_connection.rs:119`) y luego falla. |
| El estado de lo que ya hay en la base no cambia durante el pase | **Falsa hoy y no empeora.** Entre el pase y la escritura pasan los minutos del embedding, y la escritura vuelve a decidir (`place_rows` por lote, `:1008-1024`), con `needs_vector` para la fila que ya no tiene vector (`:594-596`, `:693-701`, «the store changed while importing»). La decisión del pase ya es una instantánea vieja; sirve para elegir qué se embebe y para el informe de la corrida en seco. |

## Problema (verificado contra el código)

### 1. La longitud del pase crece con el archivo y ocupa la única conexión

`place_rows` recorre todas las filas (`import.rs:656-714`). Por fila hace, con el SQL de hoy, una consulta de `PRESENT_SQL` y una de `TWIN_SQL` preparadas de nuevo (`tx.query_row`, `:572`, `:579`), una de `TOPIC_HOLDERS_SQL` si hay clave (`:584`), la de `ORIGIN_SQL` en el primer renglón de cada nota (`:417`) y el `INSERT` con su trigger de FTS (`:602-629`, V8). Con el índice de import-linear cada una es una búsqueda y el pase es lineal (~1,1 ms por fila en debug, banco); sin él, `PRESENT_SQL` recorre `memory` entera porque no tiene el término `deleted_at` del índice parcial `idx_memory_dedup` (`V20__project_scoped_indexes.sql:9-11`). Linealizarlo no lo acorta lo bastante: lo que se pide aquí es que **no ocupe la conexión**, sea cual sea su duración.

### 2. Quién paga

- El hook `session-start`: 3 s (`hook.rs:26`); si la cola está tomada, imprime nada y la sesión arranca sin memoria.
- La confirmación de un guardado: el worker y `accept` usan la misma conexión (`crates/clawg-core/src/runtime.rs:225`, según la exploración de read-connections), y el handler espera 10 s (`api.rs:106`).
- El `/health` pierde su bloque `ingest` y las búsquedas agotan el plazo: medido en la importación real (todo agotó el plazo durante el pase, ~800 s).
- El apagado: la clausura del pase no se cancela cuando el futuro se suelta (`call_raw` manda antes de esperar, `tokio-rusqlite-0.7.0/src/lib.rs:297-304`), y `checkpoint` (`runtime.rs:455-471`) va detrás en la misma cola, dentro de `AFTER_DRAIN` de 7 s (`runtime.rs:63`).

### 3. Qué decide realmente el pase (la parte que hay que reproducir)

Reglas, todas en `place` y `find`/`present_at`, con los lugares que leen:

| Regla | Lee | Efecto en el `Ledger` |
|---|---|---|
| La nota ya está aquí o es una reescritura (`find`, `:415-466`) | filas `source='import'` de su origen, también las insertadas antes en el pase | `Found::{New, Present, Revision}`, `taken`, `absent` |
| La fila ya está (`PRESENT_SQL`, `:494`) | cualquier fila (también borrada) con agente, proyecto objetivo, `content_hash` y `created_at` | `Placement::Present` |
| Gemelo vivo (`TWIN_SQL`, `:501`) | filas vivas del proyecto objetivo con ese hash; la más antigua por `created_at` | `Placement::Merged` |
| Quién tiene la clave (`TOPIC_HOLDERS_SQL`, `:508`) | filas vivas del proyecto objetivo con esa clave, salvo las del origen de la nota; el primero por `id` que no sea del mismo agente del archivo | `TopicTaken`, `Older` |
| Reescritura más nueva (`REVISED_SQL`, `:409`, `:707-711`) | escribe | las filas reemplazadas dejan de ser vivas para el `TWIN_SQL` siguiente |
| Enlaces que no se pueden escribir (`write_links`) | `memory_piece`, y las filas ya enlazadas por una guardado anterior del mismo pase | `dropped` |

El `Ledger` también lleva `import_agents`, `notes`, `invalid` y `taken`; todas se calculan sin escribir.

## Lo que ya existe y se reutiliza

- `place_rows`, `place`, `find` y `present_at` ya separan la **regla** de la **fuente** en una sola dirección: la regla toma un `&Transaction` y consulta `PRESENT_SQL`, `TWIN_SQL`, `TOPIC_HOLDERS_SQL` y `ORIGIN_SQL` en cuatro sitios (`:417`, `:481`, `:572`, `:579`, `:584`). Un asiento de cuatro lecturas y dos escrituras (insertar, reemplazar) deja las reglas en un sitio.
- `Prepared` y `Ledger` ya son datos puros construidos antes de cualquier transacción (`:171-205`, `:352-368`). El planificador recibe `&Prepared` y devuelve `Ledger`.
- `prepare` corre fuera de la conexión (`:251-316`) y `import_foreign` ya delega el trabajo largo en `spawn_blocking` (`:923-928`): es el precedente de trabajo de CPU sin tocar la cola.
- La escritura mantiene lotes por nota, `note_batches` (`:938-952`), cada uno su transacción y su `commit` (`:1013-1021`): es el patrón que ya cede la conexión entre lotes. Se queda como está.
- `MemoryStore::snapshot` (`db.rs:294-309`) es el precedente de una segunda conexión corta, y `backup::open_existing` (`backup.rs:602-612`) su apertura. Sólo para el enfoque C.
- Las pruebas de importación del propio `import.rs` y de `tests/api.rs` y `tests/binary.rs` son el oráculo (ver «Cómo se mide»).

## Enfoques posibles

### A. Planificador en memoria con carga acotada (recomendado)

1. **Cargar** lo que las reglas necesitan con lecturas por lotes: cada lote es una `conn.call` corta, así que se intercala con el resto de la cola (no hay transacción abierta entre llamadas). Primero las filas de los orígenes del archivo (`ORIGIN_SQL` con `IN`, índice `idx_memory_import_origin`, `V23__origin_project.sql:18-19`); con ellas, `find` es puro y dice el proyecto objetivo de cada nota; después las filas con los hashes del archivo y las que tienen sus claves en el proyecto objetivo (el índice por hash lo trae import-linear). También `sqlite_sequence` de `memory`, las filas de `project` de las raíces y los `memory_id` de `memory_piece` que se nombran.
2. **Decidir** en `spawn_blocking`, sin conexión: las mismas reglas, con un `Lookup` que lee de índices en memoria (hash, clave, origen) y un `Sink` que asigna ids consecutivos desde la secuencia y actualiza esos índices al aceptar una fila o reemplazar otra.
3. **Un asiento, dos implementaciones.** `find`, `present_at`, `place` y `place_rows` se escriben sobre un trait con lecturas (`origin_rows`, `present`, `twin`, `holders`) y escrituras (`insert`, `revise`). La escritura real sigue con SQL sobre la transacción (`SqlLookup`); el pase de decisión usa `Planned`. Las reglas no se duplican.
4. **Separar `write_links`** en lo que calcula `dropped` (puro, con las filas ya enlazadas como estado) y lo que escribe. El pase de decisión sólo calcula.
5. **`record_projects` de lectura:** el pase calcula `taken` leyendo `project`; la escritura sigue insertando.

- **A favor:** la conexión sólo ve lecturas de milisegundos; el pase pasa de ~1 ms por fila a microsegundos; no mantiene ningún snapshot de WAL ni transacción; funciona igual sin WAL; la regla es una sola; el planificador es lógica pura que se prueba sin base (convención del proyecto: `.claude/CLAUDE.md`, «Code conventions»); y `read-connections`, si llega, puede mandar esas lecturas a su lector sin tocar nada aquí.
- **En contra:** un asiento nuevo (justificado porque hay dos implementaciones reales); hay que reproducir, no heredar, el estado que hoy da la base gratis (ids por `AUTOINCREMENT`, orden por `created_at` y `id` en empates, visibilidad de lo insertado en el pase); la memoria del estado cargado crece con el archivo (del orden de lo que ya ocupa `Prepared`, cuyo comentario lo advierte en `import.rs:49-50`).

### B. Tabla TEMP «sombra» en la misma conexión

El pase inserta en una tabla TEMP en lugar de `memory`, y cada consulta lee `memory` más la sombra.

- **A favor:** reaprovecha más SQL.
- **En contra:** cada una de las cuatro consultas pasa a ser una unión de dos ramas, más una antijunta con una tercera tabla de «reemplazadas» (el `REVISED_SQL` modifica filas de `memory` que la sombra no puede tocar); el plan de cada consulta se vuelve a validar; sigue siendo una `conn.call` larga (misma conexión, mismo problema) o un transacción abierta entre llamadas (descartada arriba). No resuelve el problema; sólo cambia dónde se escribe.

### C. El pase en una conexión propia (patrón de `snapshot`)

Un `spawn_blocking` abre `open_existing` con `query_only`, lee una instantánea de WAL y escribe sólo en TEMP.

- **A favor:** no toma la conexión del daemon ni el bloqueo de escritura de `memory` (las escrituras a TEMP no lo piden); no necesita ceder entre lotes; reutiliza las reglas con SQL si se arma la sombra de B en TEMP.
- **En contra:** (1) sigue necesitando la sombra de B, con su costo; (2) sostiene una lectura de ~1 min (hoy) o ~17 min (archivo en el límite), y un lector así **frena el checkpoint** y el `TRUNCATE` del apagado, que espera a los lectores en curso (`runtime.rs:455-471` ya avisa «blocked by another connection»; el efecto se explica en la exploración de read-connections); (3) **sin WAL es peor que hoy**: el lector sostiene el bloqueo compartido y el `COMMIT` del worker falla tras 5 s de espera (premisas), con el intento del outbox gastado; (4) abre una escritura posible (`open_existing` es `READ_WRITE`, `backup.rs:608-611`) que hay que cerrar con `query_only`; (5) no reduce el trabajo del pase, sólo lo mueve de hilo. Es la solución si el costo de la carga de A resultara ser el cuello; hoy no hay motivo para creerlo.

### Recomendación: A, con el asiento, y C sólo como respaldo medido

1. **A** con un trait de lecturas/escrituras para `place`/`find` y la separación de `write_links`.
2. **Carga acotada** con un tamaño de lote fijo (constante nombrada) medido para que una llamada dure decenas de ms, no más que una fracción del `HOOK_DEADLINE`.
3. **El planificador corre en `spawn_blocking`** y mira un indicador de parada cada cierto número de filas, para que un apagado no espere el pase entero (`RUNTIME_TEARDOWN`, 2 s, espera las tareas bloqueantes según `.claude/CLAUDE.md`).
4. **La escritura queda como está** (SQL por lote, ya con el índice de import-linear): debe ver la base real al confirmar.
5. C no se hace salvo que la carga de A, medida, resulte larga: la medida de salida de A es la regla.

## Descartado

| Opción | Razón técnica |
|---|---|
| Partir el pase en lotes sobre la misma conexión manteniendo la transacción abierta | Lo que entra en la cola entre dos lotes corre dentro de la transacción: un guardado se pierde con el `ROLLBACK`, y el cuerpo de los otros se lee con filas del pase. |
| Partir el pase en lotes confirmados y deshacerlos | Escribe filas que luego hay que borrar de verdad (con trigger de FTS y vectores); rompe la regla de no perder ni mostrar filas del usuario. |
| Quitar el pase y decidir sólo al escribir | Pierde la corrida en seco (informe sin escribir ni embeber) y la elección de qué se embebe antes del snapshot y de la escritura (`:984-997`). |
| Una tabla TEMP sombra en la misma conexión (B) | Misma duración en la cola o transacción abierta; consultas en unión. |
| Una conexión propia como solución principal (C) | Frena el checkpoint, falla sin WAL, necesita la sombra de B (ver arriba). |
| Reescribir también la escritura con el planificador | La escritura debe decidir con la base al confirmar cada lote; la decisión vieja ya se corrige con `needs_vector` (`:594-596`). |
| Duplicar las reglas en un planificador aparte | Dos implementaciones de una regla se separan; sólo se acepta con una prueba diferencial permanente (ver «Decisiones abiertas»). |

## Interacciones

- **import-linear.** Da el índice por `content_hash` sin el que la carga de A recorre `memory` en cada lote (el parcial `idx_memory_dedup` lleva `COALESCE(project_id,'')` primero, `V20:9-11`, y `PRESENT_SQL` ni siquiera lo puede usar). Si import-linear cambia el texto de `PRESENT_SQL`, `TWIN_SQL` o `TOPIC_HOLDERS_SQL`, el `SqlLookup` hereda el cambio sin tocar las reglas. Sin ese índice, la alternativa es una carga de una sola pasada con una tabla TEMP de claves, que a su vez es una llamada de la longitud del almacén.
- **read-connections.** La carga son lecturas puras: si ese cambio llega, van al lector sin otra modificación; si no, van a la conexión única en lotes cortos. Este cambio **no depende** de él, y es la razón por la que C no hace falta. Si ese cambio llega, hay que fijar que la carga (varias llamadas) no ve una instantánea única: es aceptable por la misma razón que lo es hoy (la decisión es vieja al escribir).
- **shutdown-budget (archivado, `openspec/changes/archive/2026-10-20-shutdown-budget`).** Hoy la clausura del pase no se cancela y se come el `AFTER_DRAIN` (`runtime.rs:63`) del apagado. Con A la parte larga está en `spawn_blocking` con indicador de parada; una carga en curso es una llamada corta. Con C, el lector largo retrasaría el `TRUNCATE`.
- **import-undo.** Lee el mismo `Ledger`: las ids de `Inserted`, `TopicTaken` y `Older` son las que se podrán deshacer. El planificador no cambia las variantes de `Placement` ni cuáles están `owned` (`:342-348`); si import-undo marca las filas con un identificador de importación en el `INSERT` (`:516-521`), eso va sólo en la escritura, que no cambia aquí. Los ids del pase de decisión son indicativos (corrida en seco), no se guardan.
- **Spec:** el cambio no altera comportamiento visible. `openspec/specs/` documenta la importación; si fija que el pase usa una transacción descartada, esa frase cambia.
- **Ranking:** no se toca; no se corre `rag-eval`.

## Alcance candidato

1. Un planificador puro (`Planned`) y un trait de lecturas y escrituras que comparten `find`, `present_at`, `place` y `place_rows`, con `SqlLookup` para la escritura.
2. Carga acotada de orígenes, hashes, claves, `project`, `sqlite_sequence` y `memory_piece`, en lotes de una constante nombrada, cada uno una `conn.call` corta; sin transacción abierta entre llamadas.
3. `write_links` partido en el cálculo de `dropped` y la escritura; el pase de decisión sólo calcula.
4. El pase de decisión en `spawn_blocking` con indicador de parada.
5. Pruebas: diferenciales contra el pase SQL, intercalado de la cola y ids de la corrida en seco.
6. Un delta de spec si la frase de la transacción descartada aparece en ella.

### No-alcance

- La escritura (`place_rows` por lote, snapshot, embeddings): sigue como está; sus cambios son de otros cambios.
- El índice por `content_hash` y cualquier cambio de esquema (import-linear).
- Una conexión de lectura propia para el daemon (read-connections).
- Cambiar qué filas se embeben o el orden de los embeddings.
- Deshacer una importación.
- Cambios de ranking, de formato de exportación o de informe: el informe no cambia ni un byte.

## Decisiones abiertas

Son del usuario.

- [ ] **Un asiento o dos implementaciones.** Un trait que comparten el SQL y el planificador (recomendado: las reglas son una; el trait tiene dos implementaciones reales) o un planificador aparte con la prueba diferencial como única garantía (menos refactor, y las reglas se pueden separar con el tiempo).
- [ ] **Forma de la carga.** Lotes con `IN` (o valores en fila) sobre el índice por hash de import-linear (recomendado: cada llamada es corta y no depende del tamaño del almacén), o una tabla TEMP de claves con una sola pasada (menos viajes; una llamada tan larga como el almacén).
- [ ] **Tamaño del lote y umbral de salida.** Candidato: ninguna llamada de la carga por encima de ~300 ms (una décima del `HOOK_DEADLINE`) y el pase de decisión del banco de 15 047 filas en menos de la décima parte de los 16,4 s de hoy con el índice. Se fija leyendo la medición.
- [ ] **Parada del planificador.** Indicador atómico leído cada N filas (recomendado) o dejarlo correr hasta el final, con el costo en el apagado.
- [ ] **Qué hacer con un empate de `created_at` en `TWIN_SQL`.** El SQL no desempata (`:501-504`); el planificador toma el menor `id` (el orden del índice). ¿Se agrega `id` al `ORDER BY` de la consulta para que las dos fuentes coincidan por construcción (recomendado)?
- [ ] **Si C queda como respaldo escrito** (con la condición de WAL y `query_only`) o se elimina del texto del diseño.

## Cómo se mide

**RED que fallan hoy** (cada una se escribe antes del código):

1. **Intercalado.** Un almacén en directorio temporal, un archivo de varios miles de filas, una corrida en seco en una tarea y, mientras tanto, sondas `store.conn.call(|c| c.query_row("SELECT 1", …))` cada pocos ms: la mayor espera de una sonda queda bajo un límite fijo. Hoy la sonda espera lo que dura el pase. El archivo se dimensiona para que el pase de hoy dure varias veces el límite.
2. **El hook responde.** En `crates/clawg-cli/tests/binary.rs` (binario real, `CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`): `clawg hook session-start` durante la corrida en seco de un archivo grande contesta con contexto dentro de los 3 s. Hoy imprime nada.
3. **Equivalencia con el pase SQL.** Una tabla de escenarios (nota nueva, presente, reescritura más nueva y más vieja, nota borrada aquí y en el origen, proyecto tomado, gemelo vivo, clave tenida por otro agente y por el mismo, gemelo entre dos filas del archivo, la misma clave dos veces, dos notas con un mismo origen, dos guardados que comparten una fila fusionada y por tanto el `linked` de `write_links`, empate de `created_at`) donde el `Ledger` del planificador es igual al del pase SQL. Hoy el planificador no existe.
4. **Ids de la corrida en seco.** Un almacén al que se purgó la última fila (la secuencia pasa de `MAX(id)`): el informe de la corrida en seco es igual al de la real. Hoy pasa por la transacción revertida; es la guardia contra un planificador que parta de `MAX(id)` (debe fallar con esa implementación ingenua).
5. **Sin escritura.** Después de la corrida en seco, `memory`, `project`, `project_remote`, `memory_fts` y `sqlite_sequence` están intactos (extiende `a_dry_run_reports_without_writing`).
6. **Sin WAL.** Un almacén con diario de reversa: un guardado durante el pase de decisión se confirma (hoy espera la cola; con C fallaría, que es lo que se evita).

**El oráculo de equivalencia** (el informe no cambia ni un byte; ninguna se edita salvo las nuevas): en `import.rs`, `a_round_trip_keeps_rows_states_and_links` (`:1232`), `a_second_import_adds_nothing` (`:1326`), `links_lost_between_transactions_are_filled_by_importing_again` (`:1340`), `a_dry_run_reports_without_writing` (`:1384`), `imported_text_is_masked_and_indexed_once` (`:1408`), `a_deleted_row_comes_back_deleted_without_a_vector` (`:1507`), `an_import_gives_a_colliding_root_its_own_id` (`:1548`), `an_import_into_a_clone_goes_to_the_project_of_its_remote` (`:1564`), `a_live_twin_absorbs_the_imported_row` (`:1588`), `a_taken_topic_key_supersedes_the_imported_row` (`:1605`), `an_import_merges_into_another_agents_live_text_and_key` (`:1622`), `agent_reassigns_every_row_that_has_one` (`:1636`), `invalid_lines_are_reported_and_the_rest_imports` (`:1647`), `links_to_rows_outside_the_file_are_dropped_and_reported` (`:1659`), `an_imported_note_has_the_rows_of_a_save_of_its_text` (`:1843`), `a_key_held_here_supersedes_the_imported_note` (`:1886`), `a_mapped_root_whose_id_is_taken_gets_its_own` (`:1917`), `notes_mapped_to_clones_of_one_remote_share_its_project` (`:1939`), `a_second_import_adds_nothing_even_with_another_map` (`:1975`), `a_moved_note_is_recognized_where_it_is` (`:1994`), `a_note_deleted_here_is_not_brought_back` (`:2014`), `a_note_rewritten_at_the_source_revises_the_stored_one_where_it_is` (`:2034`), `an_older_version_enters_superseded` (`:2089`), `a_note_deleted_at_the_source_changes_nothing` (`:2113`), `a_deleted_note_comes_in_deleted_without_a_vector` (`:2137`), `relations_become_pairs` (`:2159`), `supersedes_over_a_decision_without_the_user_is_escalated` (`:2217`), `what_has_no_place_is_counted` (`:2239`), `the_foreign_report_counts_what_the_import_did` (`:2272`), `a_project_taken_while_importing_is_told_by_its_notes` (`:2352`) y `a_store_from_v16_travels_whole` (`:2381`); en `crates/clawg-cli/tests/api.rs`, `an_import_stores_rows_with_their_state_and_a_dry_run_stores_none` (`:3578`), `an_engram_import_stores_notes_and_a_dry_run_reports_the_same` (`:3646`), `import_refuses_an_unknown_source_or_a_mixed_map` (`:3691`) y `an_imported_long_note_reads_whole_and_changes_for_the_bridge_agent` (`:3740`); en `crates/clawg-cli/tests/binary.rs`, `export_and_import_round_trip_through_the_binary` (`:756`) y `an_engram_export_imports_through_the_binary` (`:794`). Las que comparan corrida en seco y real (`:1384`, `:1843`, `api.rs:3646`) son las que fallan si los ids del planificador no coinciden.

**Medición.** El banco de la entrada (embedder falso, `CLAWG_TEST_HOME` temporal, debug): tiempo del pase de decisión y, por separado, la espera máxima de una sonda en la cola, antes y después, con las mismas semillas; y una importación real en el perfil dev con el banco de sondas (`/health`, listado, búsqueda) que muestre que la fase de decisión ya no agota ningún plazo. Los dos números se anotan en el cierre del cambio; lo que ya se midió (910 s en seco y 3 078 s reales) no está en el repo.

## Riesgos

- **Dos implementaciones de una regla se separan.** Con el asiento se reduce a las lecturas y a la asignación de ids; la prueba diferencial queda permanente y cubre los empates y la visibilidad entre filas del mismo archivo.
- **Ids de la corrida en seco.** Hay que partir de `sqlite_sequence` y no de `MAX(id)`; la guardia (RED 4) lo cubre.
- **Orden de empates.** `TWIN_SQL` no desempata (`:501-504`); el planificador y el SQL deben coincidir en el menor `id`.
- **Memoria del estado cargado.** Del orden de la de `Prepared` en el límite de 256 MiB (estimación, no medida); se mide con un archivo grande antes de fijar el lote.
- **Un guardado entre la carga y la escritura.** No empeora: ya pasa hoy con los minutos de embeddings, y la escritura vuelve a decidir.
- **Carga que se queda larga.** Si un lote supera el umbral de la medición, se acorta el lote; si el costo es la suma, C queda como respaldo.
- **Apagado durante el planificador.** Se cubre con el indicador de parada y una prueba que lo ejerce.
