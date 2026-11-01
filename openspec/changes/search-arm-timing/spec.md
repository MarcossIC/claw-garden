# search-arm-timing — Especificación

Delta sobre R14 de [`specs/metrics`](../../specs/metrics/spec.md). La serie sigue siendo `search.duration` (`us`) con `arm` en `knn_memory`, `knn_episodic`, `knn_skill`, `lexical`, `identifiers` y `total`; cambia qué mide cada brazo y cómo esperan los brazos su turno en la conexión. `total`, `search.knn_retry`, los resultados y su orden no cambian.

## A1. Cada brazo mide su ejecución en el hilo de la conexión

Cada brazo que corrió (el alcance lo prendió y, para `identifiers` y `lexical`, la búsqueda trae texto) MUST registrar el tiempo que sus llamadas corrieron en el hilo de la conexión del store, sin la espera en su cola:

- `identifiers`: la consulta de coincidencias; lo que se filtra después fuera del hilo MUST NOT contarse;
- `knn_memory`, `knn_episodic`, `knn_skill`: la llamada del KNN de su tabla, con sus pasadas, la cuenta de elegibles y el puntaje;
- `lexical`: la llamada de candidatos más la de puntaje, si corrió.

Un brazo que corrió sin nada que pedirle a la conexión (una consulta sin identificadores para `identifiers`, sin tokens para `lexical`) MUST registrar 0. Un brazo que no corrió MUST NOT registrar nada. Una consulta léxica o de identificadores que falla MUST NOT hacer fallar la búsqueda, y su brazo MUST registrar lo que corrió. `total` MUST seguir midiendo la búsqueda de punta a punta, esperas incluidas.

#### Escenario: una búsqueda que espera detrás de una llamada en curso
- GIVEN la conexión retenida por una llamada que espera una señal, y una búsqueda con el KNN de `memory` y el canal léxico, con una consulta sin identificadores, encolada detrás
- WHEN la llamada suelta la conexión 200 ms después de que la búsqueda arrancó
- THEN `total` registra 150 ms o más
- AND cada brazo registra menos de 150 ms

**Tests:** `an_arm_measures_its_run_on_the_connection_not_its_wait` (`crates/clawg-core/src/memory/search.rs`).

## A2. Los brazos esperan su turno uno detrás de otro

La búsqueda MUST mandar cada llamada a la conexión después de que volvió la anterior, en este orden: `identifiers`, `knn_memory`, `knn_episodic`, `knn_skill`, los candidatos léxicos y su puntaje. Una llamada que otro encola durante la búsqueda MAY correr entre dos brazos. Un error de un KNN MUST hacer fallar la búsqueda sin mandar los brazos que siguen.

#### Escenario: una llamada que llega durante la búsqueda
- GIVEN la conexión retenida, una búsqueda con el KNN de `memory` y el canal léxico encolada detrás, y después una llamada que ocupa la conexión 200 ms
- WHEN se suelta la conexión
- THEN el canal léxico, que se manda cuando vuelve el KNN de `memory`, espera esa llamada
- AND `total` registra 250 ms o más

**Tests:** `a_call_that_arrives_during_a_search_runs_between_its_arms` (`search.rs`).

## A3. El nombre y lo que se registra, como antes

Las mismas búsquedas MUST registrar los mismos brazos bajo `search.duration`, y `search.knn_retry` igual que antes. Los docs MUST decir qué mide un brazo y que hasta 0.73.0 cada brazo incluía la espera en la cola, la de sus hermanos incluida, y que `clawg metrics report --by version` separa las dos.

**Tests:** `a_search_records_how_long_each_arm_took`, `a_widened_window_is_counted_by_its_table` (`search.rs`); `a_search_without_a_language_has_no_count`, que cuenta las búsquedas por `total` (`crates/clawg-cli/tests/api.rs`). Pasan hoy y después: fijan lo que no cambia.

## Invariantes

- Los resultados de una búsqueda y su orden no cambian: `rag-eval --fake --diff` sobre el mismo golden no mueve ningún caso.
- Ninguna consulta se agrega ni se quita.
- `memory.db`, `metrics.db` y lo que ve el modelo no cambian.
