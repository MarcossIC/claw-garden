# knn-prefilter — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2, 4 | M | — (el corpus multiagente y multiproyecto con el que se mide lo construye este cambio: eval-rigor (33), archivado, lo dejó fuera) | exploración (borrador) |

**Tesis:** en la búsqueda de memorias, vec0 corta a sus `k` vecinos más cercanos de **toda** la tabla y recién después se aplican el agente, el proyecto y el estado. Cuantas más filas tengan los otros agentes y los otros proyectos, menos de las propias sobreviven a ese corte, y el único remedio es un reintento con una ventana cuatro veces mayor. Las revisiones de un tema suman otra fuente: una fila `superseded` conserva su vector y es casi el mismo texto que la versión viva, así que ocupa la ventana justo en las consultas sobre ese tema. El mismo KNN filtrado después alimenta la detección de contradicciones, con un pool de 5. Hoy nadie midió cuánto se pierde con el código real, y el arnés de evaluación no puede verlo porque su corpus es de un solo agente, sin proyecto y sin revisiones. Este cambio empieza midiendo, y sólo si la pérdida existe elige entre tocar el esquema de vec0 o calcular la distancia exacta sobre las filas que el SQL ya admite.

Es la parte D, pendiente, de `agent-sharing` (fila 16 del mapa): [parte D](../archive/2026-10-14-agent-sharing/exploration.md#parte-d-prefiltro-en-vec0).

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| Las tablas vec0 de memoria no tienen partition key ni columnas de metadata | **Cierta.** `memory_vectors`, `episodic_vectors` y `procedural_vectors` son `vec0(embedding float[384])` (`crates/clawg-core/migrations/V2__vectors.sql:6-13`). |
| Los filtros de estado, agente y proyecto corren después del corte de vec0 | **Cierta, y el código lo dice.** `knn.k = ?2` se pasa como restricción de la tabla virtual y todo lo del `WHERE` se aplica a filas que vec0 "ALREADY cut down to k" (`crates/clawg-core/src/memory/search.rs:196-205`). Los predicados son `status IN ('active','disputed')`, `deleted_at IS NULL`, el JSON de agente y ocultos, y `project_id = ?4 OR NULL OR memory_type = 'user_preference'` (`search.rs:217-221`). |
| Hay un solo reintento y no es un bucle | **Cierta.** `knn_one` (`search.rs:439`): si vuelven menos de `k` cuenta las filas elegibles (`ELIGIBLE_SQL_MEMORY`, `search.rs:269-278`, con `LIMIT k`) y, si faltan, repite **una vez** con `retry_window(k) = min(4k, 4096)` (`search.rs:374-376`, `VEC0_MAX_K` en `:305`; la segunda pasada está en `:537`). El comentario dice que es una rama única a propósito: "iterating would burn scans" (`search.rs:434`). |
| Los candidatos por defecto son 30, 5, 5 y 20 | **Cierta** (`SearchScope::all_default`, `search.rs:37-46`). **Matiz:** episódica y procedimental no se escriben nunca, así que el pool efectivo es el de `memory` (README, "Ideas evaluadas": "dos sentencias vacías por tabla"). El tamaño final lo fija `limit`: 10 por defecto, 50 como máximo (`crates/clawg-cli/src/daemon/api.rs:69`, `:71`, `:273-282`). |
| La detección de contradicciones usa el mismo KNN filtrado después | **Cierta.** `CANDIDATE_POOL = 5` (`crates/clawg-core/src/memory/contradiction.rs:58`), más un lugar por hermano (`:197`); el alcance es `Reach::Own` (`:177`) y llama a `search_all_memories` (`:199`). Con el reintento, la ventana real es 4 × (5 + hermanos) = 20 para un save sin hermanos. |
| El estado casi no cuenta como fuente de crowding porque ningún código escribe `superseded` | **Falsa desde conflict-candidates (24) y topic-revisions (27).** `SUPERSEDE_SQL` (`crates/clawg-core/src/memory/dispute.rs:268`) lo escriben la revisión de un tema (`crates/clawg-core/src/memory/topic.rs:131`), el restore que vuelve a reemplazar lo que su borrado revivió (`crates/clawg-core/src/memory/admin.rs:569`) y un veredicto `supersedes` (`dispute.rs:364`). La fila reemplazada conserva su vector: sólo el borrado (`DELETE_VEC_MEMORY`, `admin.rs:286`) y `purge` (`crates/clawg-core/src/memory/secrets.rs:104`) lo quitan. Las revisiones de un tema son textos casi iguales a la versión viva, así que son los vecinos más cercanos de una consulta sobre ese tema: el peor caso para la ventana. |
| Ya existe distancia exacta | **Cierta, pero sólo para ids ya elegidos.** `vec_distance_l2(v.embedding, ?1)` sobre las filas `m.id IN (...)` del canal léxico (`search.rs:961-968`). No ordena ni corta: calcula una distancia por id nombrado. |
| `project_doc_vectors` ya usa partition key | **Cierta** (`crates/clawg-core/migrations/V12__project_docs.sql:33-39`). **Matiz que cambia la comparación:** ahí el filtro es una igualdad con un solo proyecto. En memoria el predicado es una disyunción de tres ramas (este proyecto, sin proyecto, preferencia de usuario de cualquier proyecto), que una sola partición no expresa. |
| sqlite-vec 0.1.9 admite partition keys y columnas de metadata | **Cierta en la versión** (`Cargo.lock:2566-2567`), y verificada contra el fuente de la dependencia (`sqlite-vec-0.1.9/sqlite-vec.c` del registro de cargo). Una metadata INTEGER o TEXT **sí admite `IN`**: `VEC0_METADATA_OPERATOR_IN` (`:5446`, `:5711`) y `struct Vec0MetadataIn`, «TEXT or INTEGER only for now» (`:6122-6123`); sobre otro tipo es un error (`:5702`). Una metadata booleana sólo admite `=` y `!=` (`:5755`). La metadata no acepta NULL: el insert exige el tipo declarado (`:8193-8217`). El `UPDATE` de una metadata está soportado («handle metadata column updates», `:9212`); el de una partition key, no («UPDATE on partition key columns are not supported yet», `:9192`). Un texto de metadata no tiene tope de largo: guarda 12 bytes en la vista del chunk y el resto aparte (`:3489-3490`, `:4157-4163`), así que los ids de agente de largo libre caben. La página de documentación de sqlite-vec no lista el `IN`: si se usa, un test lo fija, porque otra versión podría cambiarlo. |
| El crowding no está medido | **Cierta, y peor:** el arnés no puede medirlo. `rag-eval` arma el `SearchScope` con `project_id: None` y `Reach::Own` ("One agent's corpus", `crates/clawg-core/src/eval_api.rs:100-112`) e inserta las filas sin proyecto (`eval_api.rs:289`); el generador de distractores no menciona `agent_id` ni `project_id` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs`, sin coincidencias). |
| `eval-rigor` (33) agrega ese corpus | **Falsa.** Se archivó dejando la clase multiagente y multiproyecto fuera del alcance (`openspec/changes/archive/2026-10-15-eval-rigor/proposal.md:30`, `:53`). La medición de este cambio construye su propio corpus. |
| El DDL de vec0 funciona dentro de la migración | **Parcial.** `CREATE VIRTUAL TABLE` de vec0 sí corre en la transacción agrupada: lo cubre el test 7 con las migraciones reales (`openspec/changes/archive/2026-10-12-data-preservation/proposal.md:87`, `tasks.md:16`; runner agrupado en `crates/clawg-core/src/memory/db.rs:379-392`). `DROP` y renombrar quedan **sin verificar** (`openspec/changes/model-reembed/exploration.md:62`). |

## Problema (verificado contra el código)

### Qué se pierde y cuándo

Tres cosas deciden si una fila propia llega al resultado:

1. Su rango global entre **todas** las filas de la tabla, de cualquier agente y proyecto, tiene que caer dentro de la ventana de vec0 (30 en la primera pasada, 120 en la segunda).
2. Pasada la segunda ventana no hay tercera: `knn_one` no itera (`search.rs:432-438`).
3. El tope duro es `VEC0_MAX_K = 4096` (`search.rs:302-305`), que es el límite de vec0 y no una decisión de política.

**Estimación (no medida), con una hipótesis explícita.** Sea `s` la fracción de los vecinos cercanos a la consulta que es elegible para quien pregunta. Si las filas elegibles se reparten parejo entre los vecinos, la primera pasada devuelve cerca de `k·s` filas, y la fila propia de rango propio `r` tiene rango global esperado `r/s`. Entonces:

| Caso | Ventana | Condición para ver las `n` propias que necesita | Con `s = 1/3` | Con `s = 1/10` |
|---|---|---|---|---|
| Búsqueda de memoria, pool 30 | 30, luego 120 | `n/s ≤ 120` | llena el pool de 30 (30/s = 90 ≤ 120) | llena sólo 12 filas de las 30; las 10 finales sí entran (10/s = 100 ≤ 120) |
| Búsqueda con `limit = 50` | 120 | `50/s ≤ 120` | **no** entra (50/s = 150 > 120): quedan unas 40 | unas 12 |
| Contradicciones, pool 5 | 5, luego 20 | `5/s ≤ 20`, es decir `s ≥ 1/4` | se ven las 5 | se ven 2 |

Traducido: con 4 agentes que guardan sobre los mismos temas, `s ≈ 1/4` y el detector queda en el borde; con 5 o más, empieza a no ver filas propias dentro de la banda de disputa. La hipótesis de reparto parejo es **débil en los dos sentidos**: los temas se agrupan por proyecto y por agente, lo que empeora el crowding para las consultas de un tema que otro proyecto también trabaja, y lo alivia para las demás. Por eso la tabla orienta y no decide: lo decide la medición.

Un caso concreto para contradicciones. El detector sólo mira filas propias (`Reach::Own`, `contradiction.rs:175-177`) y la banda es `[0.10, 0.42)` de distancia (`contradiction.rs:37`, `:45`, `:214-215`). Si otro agente guardó el mismo hecho con otras palabras, esas filas son vecinas muy cercanas de la consulta y ocupan la ventana de 5 y de 20 antes que la fila propia que habría que marcar como disputada. El save de ese agente se acepta sin que se detecte la contradicción.

Una medición sintética fuera del repo, con el SQLite empaquetado (3.50.2), sqlite-vec 0.1.9, las migraciones reales y el SQL de `SEARCH_SQL_MEMORY` copiado tal cual, coincide con la estimación: con 3 agentes, 5 proyectos y la lectura compartida cerrada (el lector puede leer cerca del 7 % de los vectores), quedaron 2 filas después del `JOIN` con `k = 30` y 8 con `k = 120`, igual con 10.000 que con 50.000 filas. Los vectores eran aleatorios, el reparto parejo que la tabla supone, así que sirve como orden de magnitud y no reemplaza la puerta de medición. Lo que muestra es que la pérdida crece con la fracción ajena (más proyectos, más agentes con la lectura cerrada) y no con el tamaño del store.

### Las revisiones de un tema

Una fila `superseded` conserva su vector (ver premisas) y el `WHERE` la descarta después del corte (`search.rs:217`). Las versiones anteriores de un tema dicen casi lo mismo que la vigente, así que en una consulta sobre ese tema ocupan los primeros lugares de la ventana. En la misma medición sintética, con 40 revisiones `superseded` junto a la versión viva en un store de 5.000 filas, no quedó ninguna fila con `k = 5`, quedó una con `k = 30` y seis con `k = 120`. El detector de contradicciones (pool 5, reintento 20) es el más expuesto: un save sin clave sobre un tema con muchas revisiones puede no ver la versión viva, que es justo la fila contra la que habría que anotar el par. Esto no depende de cuántos agentes ni proyectos haya: alcanza con un agente que revisa un tema muchas veces, y la historia no se borra nunca.

### Quién paga el reintento

Cuando el reintento corre, vec0 vuelve a recorrer sus vectores (el README ya describe a vec0 como fuerza bruta: "Índice HNSW … Descartado. vec0 es fuerza bruta", `openspec/README.md`, tabla de mejoras). No es un costo de ventana sino de pasada: quien está crowded paga una segunda búsqueda **en cada** consulta y puede igual volver corto (`search.rs:408-411`). Medido en el corpus de evaluación, una búsqueda tarda 13,0 ms (p50) y 16,4 ms (p99), y `recall_context` 15,2 y 20,7 (`docs/eval/results/2026-10-15-real-project-indexing-relcut-agent-view.json:65-66,75-76`). Ese corpus es de un solo agente: no dice qué cuesta una base crowded.

### Qué sostiene el reintento hoy

- El conteo de elegibles sólo es correcto si cada fila viva de `memory` tiene su vector (`search.rs:415-430`). Nada en el esquema lo impone; lo sostiene que todo insert escriba fila y vector en una transacción y que un borrado quite el vector después del borrado lógico (`crates/clawg-core/src/memory/admin.rs:345-346`, `crates/clawg-core/src/memory/secrets.rs:104`).
- El estado también se filtra después del corte (`search.rs:217-218`). El vector de una fila borrada se quita, así que las lápidas no ocupan la ventana; el de una fila `superseded` no, así que las revisiones sí (ver «Las revisiones de un tema»). El crowding viene del agente, del proyecto y de la historia de los temas.

## Lo que ya existe y se reutiliza

- `KnnOutcome { hits, passes }` y su contador de pasadas, que ya existe justamente para que "el número de consultas" sea una propiedad medida y no una afirmación (`search.rs:352-369`, `:435-436`).
- `ELIGIBLE_SQL_MEMORY`, que ya aplica los mismos predicados sin pasar por vec0 (`search.rs:269-278`) y los índices parciales de `memory` por agente y por proyecto (`crates/clawg-core/migrations/V1__init.sql:37-43`).
- `vec_distance_l2` y la garantía de que devuelve valores idénticos bit a bit a la columna `distance` de vec0, fijada por un test existente (`lexical_scoring_distance_is_bit_identical_to_the_dense_knn_distance`, `search.rs:3903`; el comentario en `:939-941`). Es lo que permite mezclar una ruta exacta con la de vec0 sin cambiar el puntaje.
- La técnica de la consulta del canal léxico: `JOIN memory m NOT INDEXED ON m.id = v.rowid` para que el plan sea búsqueda por rowid, con un test que afirma el plan (`search.rs:892-924`, `:961-968`). Ojo: ese `NOT INDEXED` existe porque, sin él, el planificador recorre todas las filas vivas del agente por `idx_memory_agent_active` (`search.rs:894-899`); la consulta exacta de B, en cambio, sí quiere recorrer las elegibles, así que su plan es otro y se afirma aparte. Su comentario también documenta el límite de la otra vía: un `rowid IN (...)` sobre un KNN de vec0 **se evalúa como filtro posterior** y descarta filas fuera de `k` (`search.rs:928-938`). Esto descarta la idea de "filtrar mejor dentro de la misma consulta".
- El arnés de distractores de `golden-mine synth` (500 por defecto) y `rag-eval --agent-view`, que ya mide latencia p50 y p99 (`openspec/specs/eval/spec.md:52-54`).
- El runner de migraciones agrupado, con `pre-migration` automático si hay migraciones pendientes (`db.rs:379-392`; CLAUDE.md del repo).
- El patrón de partition key ya probado en `project_doc_vectors` (`V12__project_docs.sql:36-39`).

## Enfoques posibles

### A. Recrear `memory_vectors` con partición y metadata

Una migración nueva crea `memory_vectors` con `project_id` como partition key (con `''` en lugar de NULL) y, como columnas de metadata, un booleano `live` (activa o disputada; una fila borrada ya no tiene vector), `agent_id` y `memory_type`. Copia los vectores desde la tabla vieja uniéndolos con `memory`, sin volver a embeber, y reemplaza la tabla.

- A favor: el filtro entra al escaneo, así que `k` vecinos son `k` vecinos elegibles, también en un tema con muchas revisiones. Una partición restringe el escaneo a sus propios chunks, así que además baja la latencia: una medición sintética fuera del repo, a 50.000 filas, dio del orden de 19 ms sin filtro contra 2 ms con partition key y metadata. Es la solución estructural.
- En contra, todo medido contra el repo:
  - **El predicado de proyecto no cabe en una partición.** Hacen falta hasta tres KNN (este proyecto; sin proyecto con el centinela, porque la metadata no acepta NULL y una partición NULL no se puede pedir por igualdad; y preferencias de otros proyectos) y fusionar en Rust.
  - **El agente sí cabe, calculado en cada llamada.** El predicado es `reader = agent OR (hidden IS NOT NULL AND agent NOT IN hidden)` (`search.rs:219`), y la lista de ocultos sale de `sharing.json` en cada llamada, así que no se hornea en el dato. Pero vec0 0.1.9 admite `agent_id IN (...)` sobre una metadata TEXT (ver premisas), y la llamada puede armar la lista de agentes que el lector puede leer en ese momento. vec0 no tiene `NOT IN`, así que la lista es de los admitidos y no de los ocultos.
  - **El estado hay que mantenerlo a mano.** Cada transacción que cambia `status` tiene que actualizar `live` en la misma transacción: la revisión (`topic.rs:131`), el veredicto (`dispute.rs:364`), el restore que vuelve a reemplazar (`admin.rs:569`) y la revivida (`REVIVE_SQL`, `crates/clawg-core/src/memory/revival.rs:113`). Un olvido deja una fila invisible o visible de más, sin error. Hace falta un test de invariante que compare `memory` con la metadata, del tipo de los que suma doctor-checks (31).
  - **Los valores de partición envejecen.** El re-key de proyecto actualiza `project_id` de las tablas de datos (`crates/clawg-core/src/memory/project.rs:125`) y no toca los vectores. vec0 no admite `UPDATE` de una partition key (premisas), así que cada re-key (hoy el de identidad; mañana el de `project-clone-identity`) tendría que borrar y volver a insertar el vector. Un `PATCH` de `memory_type` y el re-embebido de `model-reembed` también tienen que escribir la metadata.
  - **Una partición con pocos vectores fragmenta el escaneo:** la documentación de sqlite-vec pide del orden de cientos de vectores por valor. Un usuario con muchos proyectos chicos podría escanear más chunks que hoy.
  - **Hay que recrear la tabla** (vec0 no se altera) y la parte `DROP` o renombrar dentro de la transacción **no está verificada**.
  - Suma estados que mantener sincronizados con `memory` sin que el esquema los ate, justo la fragilidad que `search.rs:415-430` ya describe para el conteo.
- Requiere: la próxima migración libre (V18 o la que siga; 56, 54 y 51 también pueden pedir una), con su snapshot `pre-migration`, un test que siembra la versión anterior con vectores (CLAUDE.md del repo y `config.yaml`: "tests que siembran la versión anterior") y `rag-eval --diff`.

### B. Distancia exacta sobre las filas que el SQL admite

No se migra nada. Cuando la primera pasada de vec0 queda corta y el conteo dice que hay más filas elegibles, **la segunda pasada deja de ser una ventana de 4k y pasa a ser una consulta exacta**:

```
SELECT ... vec_distance_l2(v.embedding, ?1) AS distance ...
FROM memory m JOIN memory_vectors v ON v.rowid = m.id
WHERE <los mismos predicados> ORDER BY distance LIMIT ?k
```

- **B1 (recomendado, condicionado a la puerta de medición): exacta sólo como reintento.** El camino rápido de vec0 no cambia, de modo que un llamador sin crowding paga lo de hoy (`passes == 1`) y el diff de `rag-eval` sobre el corpus actual tiene que dar vacío. Quien hoy recibe un resultado corto o una ventana de 120 insuficiente recibe el resultado correcto, sin el tope de 4096.
- **B2: siempre exacta.** Un solo camino, pero cambia la latencia de todos los llamadores y mueve las métricas.
- A favor de B: no toca el esquema, no hay migración, no tiene estados duplicados, devuelve el resultado **correcto** (los `k` más cercanos entre las elegibles) y no una ventana más grande. Reutiliza predicados, distancia y la forma de la consulta léxica. La distancia es idéntica a la de vec0, así que el puntaje no cambia (`search.rs:939-941`).
- En contra:
  - **Costo lineal en las filas elegibles.** Son 384 multiplicaciones y sumas por fila (unas 770 operaciones de punto flotante) y 1.536 bytes leídos (384 × 4); con 100.000 filas elegibles, unos 154 MB leídos por consulta. Como vec0 ya recorre **todos** los vectores en cada búsqueda, la exacta lee como mucho lo mismo, salvo por el costo de entrar a vec0 por rowid en vez de recorrerlo en orden: el propio código mide que, a 5.000 filas y 20 candidatos, la consulta léxica por rowid tarda unos 330 µs contra 730 µs si el planificador la conduce desde un índice, y que escribir mal la unión (`v.rowid IN`) cuesta decenas de milisegundos a ese tamaño (`search.rs:899-905`). **A qué N deja de ser despreciable es justo lo que hay que medir**; no tengo un número.
  - Una consulta con `ORDER BY` sobre una expresión: el plan hay que afirmarlo con un test, como el del canal léxico, porque dos formas equivalentes pueden diferir en órdenes de magnitud.
  - Depende del conteo de elegibles, con la misma invariante fila ↔ vector (`search.rs:415-430`).

### C. Reintento adaptativo que crece hasta llenar el pool

Un bucle que duplica `k` hasta tener suficientes elegibles o llegar a 4096.

- A favor: poco código nuevo.
- En contra: cada pasada es un recorrido completo de vec0, así que el costo se multiplica; el código actual rechaza esto por escrito (`search.rs:432-438`); y termina en el mismo techo de 4096, con lo cual una base grande sigue sin resolverse. **No recomendado.**

### D. No hacer nada hasta medir

No es un destino sino la **puerta**: se corre la medición de más abajo y se registra el resultado. Si la pérdida medida es despreciable en las formas de base realistas (ver decisiones), el cambio se cierra sin tocar el código de búsqueda y deja el corpus y el criterio escritos. Es la única opción que no puede salir mal.

**Recomendación:** D como paso obligatorio primero; si la medición muestra pérdida, B1. A sólo si B1 resulta demasiado lento en una base del tamaño que importe, con la medición en la mano.

## Descartado

| Opción | Razón técnica |
|---|---|
| Subir `memory: Some(30)` o `CANDIDATE_POOL` | Mueve las métricas de todos los llamadores y no resuelve nada: la ventana sigue siendo un múltiplo fijo de `k`, y el crowding crece con la base. Toca `all_default` (`search.rs:37-46`) sin evidencia. |
| Filtrar con `rowid IN (...)` dentro de la consulta de vec0 | Se evalúa después del corte; el código lo midió y lo rechazó (`search.rs:928-938`). |
| Una base o una tabla vec0 por agente | Rompe el cerebro compartido (objetivo 4) y la política de lectura en `sharing.json`, que se lee en cada búsqueda. |
| Mover la lista de ocultos a la metadata | `sharing.json` cambia sin que cambie la base (CLAUDE.md del repo); el dato quedaría desfasado hasta el próximo re-embebido. |
| Cambiar el motor a uno con índice aproximado | Ya descartado: rompe la transacción única memoria + vector + FTS (README, tabla de mejoras). |
| Contar la ventana en `memory_vectors` en vez de en las elegibles | Es la lectura que `ELIGIBLE_SQL_MEMORY` evita a propósito (`search.rs:256-268`). |

## Interacciones

- **eval-rigor (33, archivado):** su comparación pareada con intervalo es la que permite afirmar que un diff no pierde casos. Dejó fuera la clase multiagente y multiproyecto, así que el corpus de la puerta es la primera tarea de este cambio.
- **topic-revisions (27) y revision-aware-delete (49), archivados:** escriben las filas `superseded` que ocupan la ventana, y la revivida las vuelve a poner vivas. Con A, cada una de esas transacciones escribe también `live`; con B no se enteran.
- **save-bounds (54):** un save largo corre un KNN del detector por pieza; este cambio abarata cada uno, aquel acota cuántos hay.
- **read-connections (57):** espera a este cambio, que baja el costo de cada escaneo en el único hilo de SQLite, para medir lo que quede.
- **doctor-checks (31):** si se elige A, la coincidencia entre `memory` y la metadata de `memory_vectors` es una invariante más que puede comprobar.
- **conflict-candidates (24):** el detector pasa a anotar pares candidatos; si cambia a qué filas mira, el pool de 5 de este cambio hay que reconciliarlo con el suyo.
- **model-reembed (17) y recoverable-delete (18):** ambos escriben vectores (re-embebido y restauración). Con A, cada uno tendría que escribir también la partición y la metadata; con B no se enteran.
- **project-clone-identity (30):** re-key de proyecto. Con A obliga a reescribir vectores; con B no.
- **agent-sharing / `sharing.json`:** B respeta la lista de ocultos tal como está, porque reusa el mismo predicado.
- **knowledge-synthesis y `search_project`:** la búsqueda de documentos no se toca; ya tiene partición (`V12__project_docs.sql:36-39`).

## Alcance candidato

1. **Puerta de medición:** corpus con `A` agentes y `P` proyectos y una función exacta de referencia, y una métrica de pérdida por crowding para búsqueda y para contradicciones (ver "Cómo se mide").
2. Si la puerta lo justifica, la decisión pura `needs_exact(len_primera_pasada, k, elegibles)` y la consulta exacta (B1) para `memory`, con su test de plan.
3. Aplicarlo a la detección de contradicciones sin cambiar su alcance (`Reach::Own`) ni su banda.
4. Un campo en el registro de la búsqueda que diga qué camino se usó (vec0, ventana ampliada, exacta), en la línea que ya reporta `pool_req` y `pool_ret` (`format_pool_line`, `search.rs:587`).
5. Reportar `rag-eval --diff` y la latencia de `--agent-view` antes y después.
6. CLAUDE.md del repo: sólo si cambia algo visible para el usuario (no se espera).

### No-alcance

- Cambiar el modelo, los pesos del puntaje, la ventana de recencia o los umbrales de la banda de disputa.
- Las tablas episódica y procedimental (siguen vacías; si se vuelven a escribir se retoma).
- Cambiar el esquema (salvo que la medición lleve a A, y entonces es un cambio nuevo con su propia propuesta).
- La búsqueda de documentos de proyecto.
- Políticas de visibilidad (lo que cada agente puede leer): este cambio sólo asegura que lo que ya puede leer llegue.

## Decisiones abiertas

Son del usuario.

- [ ] **Qué formas de base cuentan como realistas** para decidir si hay pérdida: por ejemplo 3 agentes × 5 proyectos y 10.000 filas. Sin esa forma la puerta no tiene criterio.
- [ ] **Umbral de la puerta.** Propuesta: se actúa si el recall propio @10 contra la referencia exacta baja más de 0,02 en alguna forma realista, o si el detector deja pasar más de un par de cada diez en el caso de contradicciones. Los números son propuestos y hay que fijarlos antes de medir.
- [ ] **B1 o B2.** B1 conserva el camino rápido; B2 simplifica el código a costa de la latencia de todos.
- [ ] **Cómo se anuncia la ruta exacta.** ¿Sólo en el log de búsqueda, o también en `memory_status`?
- [ ] **Si la exacta tiene techo.** Si las filas elegibles superan un N medido, ¿se cae a la ventana ampliada de hoy y se avisa, o se permite la consulta cara?
- [ ] **Cómo es el corpus propio.** Cuántos agentes, proyectos y filas, y cuántas revisiones por tema: sin cadenas de revisiones la puerta no ve la fuente de crowding que ya existe con un solo agente.
- [ ] **Alcance en contradicciones.** ¿Se aplica el mismo mecanismo o el detector se queda con la ventana ampliada por su pool chico?

## Cómo se mide

### Puerta previa (antes de elegir)

Con un corpus sintético multiagente y multiproyecto, con cadenas de revisiones (lo construye este cambio), sobre las formas de base de la decisión de arriba:

1. **Pérdida por crowding en búsqueda:** para cada consulta, la fracción de los `limit` primeros de la **referencia exacta** (distancia `vec_distance_l2` sobre las filas elegibles, calculada en el test) que **no** aparece en el resultado de producción. Se reporta por forma de base.
2. **Pérdida en contradicciones:** de los pares sembrados dentro de la banda, cuántos devuelve `detect_contradiction` y cuántos se pierden porque filas de otro agente, o revisiones `superseded`, ocupan la ventana.
3. **Costo:** `passes` y latencia p50 y p99 de producción, por forma de base.
4. **Costo de la exacta:** la consulta exacta con 1.000, 10.000 y 100.000 filas elegibles. Es el número que falta para decidir B contra A.

Si la pérdida es menor que el umbral, se cierra con D y queda registrado.

### Tests RED primero (si la puerta lo justifica)

1. `needs_exact` es pura: sin pasada corta no; con pasada corta y elegibles de sobra, sí; con pasada corta y todo lo elegible ya devuelto, no.
2. **Crowding por agente:** el llamador tiene 5 filas relevantes; 3 agentes ajenos tienen 200 filas más cercanas que ellas. Hoy el resultado es corto (la ventana de 120 no llega); con el cambio vuelven las 5 en orden de distancia.
3. **Crowding por proyecto:** lo mismo con 9 proyectos ajenos que rodean al propio, incluyendo una `user_preference` de otro proyecto que debe seguir apareciendo.
4. **Contradicciones:** otro agente tiene más de 20 filas más cercanas que la propia, que está dentro de la banda. Hoy `detect_contradiction` devuelve `[]`; con el cambio devuelve el id propio, y no marca nunca una fila ajena.
5. **Revisiones:** un tema con 40 versiones `superseded` junto a la vigente; una búsqueda sobre el tema devuelve la vigente y un save sin clave cercano la ve como candidata. Hoy no.
6. **Sin regresión de costo:** un llamador sin crowding sigue en `passes == 1` y su resultado es idéntico bit a bit.
7. **Paridad:** la distancia de la ruta exacta es idéntica a la de vec0 (el test existente `lexical_scoring_distance_is_bit_identical_to_the_dense_knn_distance`, `search.rs:3903`, extendido a la consulta nueva).
8. **Plan:** `EXPLAIN QUERY PLAN` de la consulta exacta, con las dos líneas (`memory` y `memory_vectors`), como lo hace `the_scoring_statement_reaches_its_rows_by_id_at_every_candidate_count` para el canal léxico (`search.rs:3667`).
9. **Visibilidad:** la ruta exacta respeta `hidden` de `sharing.json`, excluye `deleted_at` y los estados fuera de `active` y `disputed`.

Si se elige A (cambio nuevo): test sembrado en la versión anterior con vectores y snapshot previo, test de `DROP` y renombrar vec0 dentro de la transacción, test de re-key que borra y vuelve a insertar el vector en su partición, test de invariante entre `status` y `live` después de cada transición (revisión, veredicto, restore, revivida), y un test que fija el `IN` sobre la metadata TEXT.

### rag-eval

**Obligatorio** (`config.yaml`: todo cambio de recuperación se valida con `rag-eval --diff`). Sobre el corpus actual el diff tiene que ser **vacío**: un solo agente y sin proyecto nunca dispara el reintento, porque la primera pasada llena el pool o trae todo lo elegible. Las mejoras se muestran sobre el corpus multi-agente nuevo, con el ancla actual (R@5 0,6522, `config.yaml`) sin moverse. `rag-eval --diff` compara también dos corridas `--agent-view` (`search.found` es métrica primaria; `crates/clawg-eval/src/bin/rag-eval/diff.rs:48`), pero no la latencia: esa se compara a mano.

## Riesgos

- **Medir contra una forma de base que nadie tiene.** Un corpus sintético con reparto parejo puede sobre o subestimar el crowding real, que depende de qué tan agrupados estén los temas por proyecto. Por eso las formas se eligen a propósito y el resultado se reporta por forma.
- **Latencia de la ruta exacta en bases grandes.** Es lineal en las filas elegibles y no tengo el número. Con B1 sólo la paga quien hoy ya paga el reintento.
- **La invariante fila ↔ vector** de la que depende el conteo de elegibles no está en el esquema (`search.rs:415-430`). La ruta exacta la hereda; una ruta de inserción que la rompa hace que la exacta corra siempre.
- **Elegir A sin necesitarlo.** Es la opción más cara y deja metadata que se desfasa con cada re-key, `PATCH` y re-embebido. La puerta existe para no llegar ahí por intuición.
- **El corpus es trabajo propio.** eval-rigor no lo trajo; si este cambio no lo construye, la puerta no tiene con qué medir y queda en D por falta de medición, no por falta de pérdida.
- **Con A, una transición que olvida `live`** deja una fila invisible o visible de más sin ningún error; sólo un test de invariante la ve.
