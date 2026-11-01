# search-arm-timing — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (transversal) | S | — | exploración cerrada; sigue en la [propuesta](proposal.md) |

**Tesis:** `search.duration` por brazo no mide lo que dice. Los tres KNN y el canal léxico arrancan su reloj casi juntos y se encolan en la misma conexión, así que cada brazo suma lo que tardaron los anteriores. read-connections decide con mediciones de esta búsqueda, por eso este cambio va antes. knn-prefilter no: su puerta se lee por pérdidas, y la latencia que informa es la de `total` y la de `knn_memory`, el primer brazo de la cola, en búsquedas que corren solas y sin canal léxico ([su verify](../knn-prefilter/verify.md), después de la tabla de la segunda pasada).

## Problema (verificado contra el código)

- `search_all_memories_inner` corre los brazos con `tokio::try_join!` (`crates/clawg-core/src/memory/search.rs:1304-1359`). Cada uno toma `started` antes de su primera espera (`:1308`, `:1320`, `:1332`, `:1351`) y lo cierra al volver (`:1311`, `:1323`, `:1335`, `:1353`).
- Todos van a la misma cola: `knn_one` llama a `store.conn.call` (`search.rs:564-566`), y `tokio-rusqlite` atiende en orden con un solo hilo (ver [read-connections](../read-connections/exploration.md), primera tabla de premisas).
- Por eso `knn_episodic` incluye la ejecución de `knn_memory`, `knn_skill` la de los dos, y `lexical` la de los tres, más el puntaje de sus candidatos (`:1401`). Cada brazo suma además lo que otras llamadas tuvieran en la cola.
- `try_join!` no da paralelismo. El comentario de `:1288-1290` ya lo dice de otro brazo: «All arms share one connection, so nothing ran in parallel to lose».
- `docs/reference/data-profiles.md:18` y `openspec/specs/metrics/spec.md:164` lo presentan como la duración de cada brazo, sin esa salvedad.
- Los tests sólo fijan qué brazos se registran (`search.rs:5424`, `:5432`) y el total (`crates/clawg-cli/tests/api.rs:1584`). Nada fija qué mide cada uno.

Sin medir: cuánto se desvía en una base real. La conclusión sale de leer el código; el primer test en rojo la comprueba.

## Lo que ya existe y se reutiliza

- `db.probe_wait` mide aparte la espera en la cola (`MemoryStore::probe_wait`, `crates/clawg-core/src/memory/conn_probe.rs`), así que un brazo no necesita incluirla para que se vea una cola sostenida. **Límite:** es una muestra cada 10 s (`PROBE_INTERVAL`, `crates/clawg-cli/src/daemon/store_probe.rs:14`), y una cola breve puede no caer en ninguna, aunque se refleje en `total`. Antes de usar esta serie para decidir si hacen falta conexiones de lectura hay que contar con ese límite, o registrar aparte la espera de las búsquedas.
- Retener la conexión desde un test ya está hecho (`crates/clawg-cli/src/daemon/api.rs:3644-3648`).
- `metrics.db` guarda la versión de cada corrida, así que un minuto anterior y uno posterior se pueden separar.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Reloj dentro de la clausura y esperas en orden** | Cada brazo mide su ejecución, sin hermanos ni cola. Saca una concurrencia que no existe | El brazo deja de incluir la cola: quien quiera el tiempo de punta a punta lee `total` |
| B. Sólo esperas en orden, con el reloj donde está | El cambio más chico | El brazo sigue sumando lo que otras llamadas tengan en la cola, y eso depende de la carga |
| C. Todos los brazos en una sola llamada a la conexión | Una vuelta en vez de cinco y una lectura coherente | Sostiene el hilo más tiempo seguido, sin dejar pasar una escritura entre brazos. Es una decisión de read-connections, no de medición |

**Recomendado: A.** `total` y `db.probe_wait` siguen diciendo el resto. C se evalúa en read-connections, si su medición lo pide.

## Alcance candidato

1. Medir cada brazo en el hilo de la conexión, sin contar la espera.
2. Reemplazar `try_join!` por esperas en orden, conservando el mismo orden de brazos.
3. Decir en `docs/reference/data-profiles.md` y en `specs/metrics` qué mide un brazo y desde qué versión.

### No-alcance

- Cambiar qué consultas corren o cuántas vueltas da una búsqueda.
- Cualquier cambio de ranking: el orden de los resultados sigue saliendo de `hit_cmp` (`search.rs:225`).

## Decisiones abiertas

1. El detalle de cómo vuelve la duración desde la clausura: un campo de `KnnOutcome`, que ya lleva `passes`, parece el lugar natural.
2. Si el nombre de la serie se conserva o se crea otra, ya que el significado cambia. Conservarlo mezcla semánticas en los histogramas de una misma serie; la versión de cada corrida permite separarlas.

## Cómo se mide

- **Test en rojo:** una búsqueda con una consulta sin identificadores: `identifier_matches` vuelve sin pedir la conexión (`search.rs:960-963`), así que la espera no cae antes de que arranquen los relojes de los brazos KNN. El test toma la conexión con una llamada que espera una señal para soltarla, confirma que ya empezó (como `the_probe_waits_behind_a_call_in_progress`, `crates/clawg-core/src/memory/conn_probe.rs:67`), lanza la búsqueda, deja pasar 200 ms y recién ahí la suelta. Así `total` cubre toda la retención y queda por encima de 150 ms, y los brazos KNN quedan por debajo. Hoy los brazos la incluyen.
- `rag-eval --diff` con el embedder falso sin ningún caso movido: el cambio no toca el ranking.

## Interacciones

- **read-connections:** lee estas series en su puerta de medición. Con los números de hoy, `knn_skill` parece caro por lo que costó `knn_memory`.
- **knn-prefilter:** no decide con estas series. Su puerta se lee por pérdidas, K1 cronometra cada consulta dentro del hilo de la conexión, y sus latencias son las de `total` y `knn_memory`, el primer brazo de la cola, en búsquedas que corren solas y sin canal léxico ([su verify](../knn-prefilter/verify.md), después de la tabla de la segunda pasada).
- **retire-legacy-kinds:** saca dos de los brazos. El test no debería depender de ellos.
