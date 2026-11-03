# search-arm-timing — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (transversal) | S | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); verificado otra vez contra `develop`:

- `search_all_memories_inner` corre los tres KNN y los candidatos léxicos con `tokio::try_join!` (`crates/clawg-core/src/memory/search.rs:1304-1359`). Cada brazo toma su reloj antes de su primera espera (`:1308`, `:1320`, `:1332`, `:1351`) y lo cierra al volver (`:1311`, `:1323`, `:1335`, `:1353`); el léxico le suma el puntaje de sus candidatos (`:1393-1402`).
- Las cuatro llamadas van a la misma cola: `knn_one` y `lexical_candidates` llaman a `store.conn.call` (`search.rs:565-566`, `:861-862`), y `tokio-rusqlite` las corre de a una, en orden de llegada, en un solo hilo (`tokio-rusqlite-0.7.0/src/lib.rs:376-377`, `event_loop` en `:412`). `try_join!` las encola juntas y espera la primera.
- Por eso `knn_episodic` incluye la ejecución de `knn_memory`, `knn_skill` la de los dos y `lexical` la de los tres, y cada brazo suma además lo que otras llamadas tuvieran delante en la cola.
- `docs/reference/data-profiles.md:18` y R14 de [`specs/metrics`](../../../specs/metrics/spec.md) lo presentan como la duración de cada brazo.
- Los tests fijan qué brazos se registran (`search.rs:5424`, `:5432`) y el total (`crates/clawg-cli/tests/api.rs:1584`); nada fija qué mide cada brazo.

## Objetivo que sirve

Medición, transversal. read-connections (57) decide en su puerta si una lectura espera en la cola, y lee la búsqueda por brazo para saber en cuál. Con las series de hoy esa lectura atribuye a cada brazo el costo de los anteriores. knn-prefilter (37) no decide con ellas ([su verify](../../knn-prefilter/verify.md), después de la tabla de la segunda pasada).

## Decisiones

1. **Enfoque A de la exploración.** Cada brazo toma su reloj dentro de la clausura que corre en el hilo de la conexión, así que mide su ejecución sin la espera en la cola. B (esperas en orden con el reloj donde está) dejaría en cada brazo lo que otras llamadas tuvieran en la cola, que depende de la carga. C (todos los brazos en una sola llamada) sostiene el hilo más tiempo seguido: es una decisión de read-connections, no de medición.
2. **Qué mide cada brazo.** El tiempo que sus llamadas corrieron en el hilo de la conexión:
   - `identifiers`: la consulta de coincidencias (`identifier_matches`). El filtro de `holds_identifier`, que corre después en la tarea sobre las filas que volvieron, queda fuera.
   - `knn_memory`, `knn_episodic`, `knn_skill`: la llamada de `knn_one` entera: las pasadas, la cuenta de elegibles, los enlaces de las piezas, el puntaje, el orden y el recorte, que ya corren todos en esa clausura (`search.rs:566-680`).
   - `lexical`: la llamada de candidatos más la de puntaje de los que el denso no trajo, si corrió.
   - Un brazo que corrió sin nada que pedirle a la conexión (una consulta sin identificadores para `identifiers`, sin tokens para `lexical`) registra 0; uno que no corrió (el alcance lo apagó, o `identifiers` y `lexical` sin texto de consulta) no registra nada, como hoy.
   - Una consulta léxica o de identificadores que falla sigue sin hacer fallar la búsqueda y cuenta lo que corrió antes de fallar.
   - `total` no cambia: de punta a punta, con las esperas. Quien quiera lo que costó la cola lee `total` y `db.probe_wait`.
3. **Cómo vuelve la duración.** `KnnOutcome`, que ya lleva `passes`, gana el campo `ran`. Los canales que no fallan (`identifier_matches`, `lexical_candidates`, `score_lexical_candidates`) devuelven su duración junto con su resultado, por una función que corre la consulta con `call_raw` y la cronometra dentro: así un error de SQLite vuelve con lo que la consulta corrió, y una llamada que nunca llegó al hilo (la conexión cerrada) vuelve con 0.
4. **Esperas en orden, con el mismo orden.** `try_join!` pasa a esperar cada brazo antes de mandar el siguiente: `identifiers` (ya iba primero), `knn_memory`, `knn_episodic`, `knn_skill`, los candidatos léxicos y su puntaje, y después `save_links`. Es el orden en que entraban a la cola. Un error de un KNN sigue haciendo fallar la búsqueda y deja sin correr lo que viene después, como `try_join!` soltaba los otros.
   - **Lo que cambia en la cola.** Antes las cuatro llamadas entraban juntas y nada corría entre ellas. Ahora una llamada que llega durante la búsqueda puede correr entre dos brazos. Con la cola ocupada, `total` puede crecer, porque cada brazo vuelve a hacer fila; una escritura deja de esperar los cuatro brazos juntos. No cambia ninguna promesa de coherencia: cada brazo ya era su propia llamada, sin una transacción compartida, y los identificadores, el puntaje léxico y `save_links` ya eran llamadas aparte. Si la búsqueda debe sostener el hilo en una sola llamada (C) lo decide read-connections con su medición.
5. **La serie conserva su nombre**, `search.duration` con `arm`:
   - La unidad y las etiquetas no cambian, y `total` sigue diciendo lo mismo: partir la serie dejaría `total` en una y los brazos en otra, o lo duplicaría.
   - Ninguna fila de `metrics.db` mezcla los dos significados: cada minuto y cada hora plegada son de una ejecución (R4 de `specs/metrics`), y cada ejecución guarda su versión. `clawg metrics report --by version`, el agrupamiento por defecto, separa el antes y el después.
   - Sólo `--by day`, `--by hour` o `--by minute` sobre una ventana que cruza la actualización junta ejecuciones de las dos versiones. Los docs lo dicen: se lee por versión o con un `--since` posterior.
   - Quienes leen la serie siguen andando: `crowding.rs:499` lee `knn_memory` y `total` de búsquedas solas y sin canal léxico, donde el brazo sólo pierde el salto al hilo; `crates/clawg-cli/tests/api.rs:1584` lee `total`. Un nombre nuevo obligaría a juntar dos series para ver la historia.

## Alcance

1. El test en rojo de «Cómo se mide» y la medición de cada brazo en el hilo de la conexión.
2. Las esperas en orden, con un test de que una llamada que llega durante la búsqueda corre entre sus brazos.
3. `docs/reference/data-profiles.md` y R14 de `specs/metrics`: qué mide un brazo y desde qué versión.
4. La fila 101 de `openspec/README.md` y `state.yaml`.

## Fuera de alcance

- Cambiar qué consultas corren o cuántas llamadas hace una búsqueda: son las mismas, en el mismo orden.
- El ranking: el orden de los resultados sigue saliendo de `hit_cmp` (`search.rs:225`).
- C y registrar aparte la espera de cada búsqueda: los decide read-connections.
- Subir la versión y archivar.
- Lo que ve el modelo: `agent_contract.json` no cambia.

## Cómo se mide el éxito

Tests que fallan hoy, sin modelo real, red ni perfil (ver [spec](spec.md)):

- Con la conexión retenida por una llamada que espera una señal, una búsqueda con canal léxico y una consulta sin identificadores (`identifier_matches` vuelve sin llamar a la conexión, `search.rs:960-963`) arranca y queda en la cola. A los 200 ms se suelta la conexión: `total` queda en 150 ms o más y cada brazo por debajo de 150 ms. Hoy los brazos incluyen la retención.
- Con la conexión retenida, la búsqueda en la cola y detrás de ella una llamada de 200 ms, `total` pasa de 250 ms: el segundo brazo esperó esa llamada. Hoy los brazos entran juntos delante de ella y la búsqueda termina en unos 100 ms.

El ranking no se mueve: `rag-eval --fake` antes y después sobre el mismo golden, con `--diff` sin ningún caso movido. El golden de 46 casos (`docs/eval/golden.json`) está fuera de git; sin él sirve uno sintético de `golden-mine synth`.

## Datos y vuelta atrás

`memory.db` no cambia, ni `metrics.db`: ni esquema, ni nombres de series, ni etiquetas. Volver al binario anterior vuelve a sumar la espera en cada brazo; las filas de las dos versiones quedan separadas por la versión de su ejecución.
