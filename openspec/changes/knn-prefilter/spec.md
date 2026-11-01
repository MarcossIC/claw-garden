# knn-prefilter — Especificación (la puerta de medición)

KP1 a KP8 describen la medición de la [propuesta](proposal.md). Viven en un módulo sólo de tests de `clawg-core` (`crates/clawg-core/src/memory/crowding.rs`), así que no cambian nada de lo que compila un build de producción. Al archivar, entran en [`specs/eval`](../../specs/eval/spec.md) como una sección nueva, «La ventana del KNN contra los vecinos exactos».

## Definiciones

- **Forma:** agentes, proyectos, filas, lectura (abierta o cerrada) y si decide (decisión 2 de la propuesta).
- **Lector:** el agente `a0` con el alcance de la forma: `Reach::AllBut([])` con la lectura abierta, `Reach::AllBut` de los otros agentes con la cerrada.
- **Elegible:** una fila que los predicados de `SEARCH_SQL_MEMORY` dejan pasar para el lector y el proyecto (`crates/clawg-core/src/memory/search.rs:259-276`): viva (`active` o `disputed`, sin `deleted_at`), alcanzable (`Reach::admits`) y del proyecto, sin proyecto o `user_preference`.
- **Distancia del piso:** `sqrt(2 · (1 − 0,79))`, la distancia L2 del piso de coseno de `ScoringConfig::DEFAULT`. Una fila a esa distancia o más puntúa 0.

## KP1. Un corpus determinista con la forma pedida

El generador MUST ser puro: dada una forma y una semilla, MUST devolver las mismas filas, con sus vectores bit a bit, y las mismas consultas y sondas. MUST armar cada vector como la suma normalizada de las componentes global, tema, afirmación y ruido con los pesos de la propuesta (0,75, 0,13, 0,07 y 0,05). MUST dar exactamente las filas de la forma: 15 cadenas de cada largo (5, 10, 20 y 40 versiones `superseded` más la vigente) del lector, 15 afirmaciones repetidas por cada agente en cada proyecto, un 20 % de las filas sin proyecto y, del resto, una de cada 20 `user_preference` y otra de cada 20 borrada y sin vector. Esas dos MUST sortearse fila por fila: un contador vuelve a caer en el mismo agente y el mismo tema cada vez que la vuelta de los agentes por los temas es múltiplo de 20, y deja borradas todas las filas de un agente en un tema.

#### Escenario: la misma semilla
- GIVEN una forma y una semilla
- WHEN el generador corre dos veces
- THEN las dos salidas son iguales, y con otra semilla los vectores difieren

#### Escenario: lo que tiene una forma
- GIVEN la forma F2 (3 agentes, 5 proyectos, 10.000 filas)
- WHEN se genera
- THEN hay 10.000 filas, 2.000 sin proyecto, 60 cadenas con sus 1.125 versiones `superseded`, y 225 copias de afirmaciones repetidas

#### Escenario: las distancias de la geometría
- GIVEN filas de temas distintos, del mismo tema y de la misma afirmación
- WHEN se miden sus distancias L2
- THEN la media de cada grupo cae cerca de 0,71, 0,49 y 0,32, y todo par de la misma afirmación queda dentro de `[0,10, 0,42)`

#### Escenario: el borrado no sigue al agente ni al tema
- GIVEN cinco agentes y cuatro temas de cada clase, de modo que una fila de un agente vuelve al mismo tema cada 20 filas
- WHEN se genera
- THEN ningún grupo de proyecto, tema y agente queda con todas sus filas borradas

**Tests:** `the_same_seed_lays_out_the_same_corpus`, `a_shape_holds_its_chains_copies_and_unowned_rows`, `the_layout_puts_topics_claims_and_unrelated_rows_at_their_distances`, `preferences_and_deletions_fall_apart_from_agents_and_topics`.

## KP2. La elegibilidad pura es la de producción

La medición MUST decidir qué fila es elegible con una función pura sobre lo que el generador sabe de cada fila (agente, proyecto, tipo, estado, borrado), con `Reach::admits` para el agente. Esa función MUST coincidir con los predicados de `SEARCH_SQL_MEMORY`: la referencia en SQL (KP3) y la pura MUST dar las mismas filas sobre el mismo corpus.

#### Escenario: cada predicado
- GIVEN una fila del proyecto del lector, una sin proyecto, una `user_preference` de otro proyecto, un `semantic_fact` de otro proyecto, una de un agente oculto, una `superseded` y una borrada
- WHEN se decide para el lector
- THEN las tres primeras son elegibles y las otras cuatro no

**Tests:** `eligibility_follows_the_production_predicates`, `a_closed_reading_hides_every_other_agent`.

## KP3. La referencia exacta

La referencia de una consulta MUST ser las `L` filas elegibles más cercanas por `vec_distance_l2`, ordenadas por distancia y después por id, entre las que quedan debajo de la distancia del piso. La medición MUST calcularla con una consulta SQL que une `memory` y `memory_vectors` por `rowid` y aplica los predicados de producción, escritos con las mismas macros (`reach_predicate!`, `memory_project_predicate!`), para que no se aparten. Es también la consulta cuyo costo mide KP7. La referencia del detector MUST ser las 5 elegibles más cercanas, filtradas por la banda `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)` y por tipo que sostiene una afirmación, como `detect_contradiction` (`crates/clawg-core/src/memory/contradiction.rs:216-229`).

#### Escenario: SQL contra la referencia pura
- GIVEN un corpus chico con todas las fuentes de crowding
- WHEN se calcula la referencia de cada consulta en SQL y con la elegibilidad pura y distancias en Rust
- THEN las dos dan las mismas filas en el mismo orden

El pool del detector (`CANDIDATE_POOL`, `contradiction.rs:59`) es privado, así que la medición lo repite y un test los ata: con 7 filas elegibles dentro de la banda, la sonda devuelve tantas como la referencia, 5.

**Tests:** `the_exact_query_returns_the_pure_reference`, `the_detector_reference_keeps_the_pool_of_the_detector`.

## KP4. La pérdida

La pérdida a `L` de una consulta MUST ser `|R \ P| / |R|`, con `R` la referencia a `L` y `P` las primeras `L` filas de `memory` del resultado de producción. Una consulta con `R` vacía MUST quedar fuera del promedio y contarse aparte. La pérdida del detector MUST ser la suma de los candidatos de la referencia que `detect_contradiction` no devolvió sobre la suma de los candidatos de la referencia. Los percentiles MUST ser de rango más cercano.

#### Escenario: pérdida parcial
- GIVEN una referencia de 10 filas y un resultado que tiene 7 de ellas y 3 que no están en la referencia
- WHEN se mide
- THEN la pérdida es 0,3

#### Escenario: sin referencia
- GIVEN una referencia vacía
- WHEN se mide
- THEN no hay pérdida que promediar

**Tests:** `loss_counts_the_reference_rows_the_result_left_out`, `a_quantile_is_the_nearest_rank`.

## KP5. La medición ve la pérdida cuando la hay, y no la inventa cuando no

Sobre un corpus chico que corre con la suite, la medición MUST dar pérdida 0 y una sola pasada en cada búsqueda cuando no hay ninguna fila que el lector no pueda leer, y MUST dar pérdida mayor que 0 cuando más filas ajenas que la ventana ensanchada (120) están más cerca que las del lector. MUST leer si la búsqueda corrió la segunda pasada del gauge `search.knn_retry` del recorder del store, y la latencia del histograma `search.duration`, sin cambiar `search.rs`.

#### Escenario: sin filas ajenas
- GIVEN un corpus de un agente y un proyecto, sin revisiones
- WHEN corre cada consulta de tema
- THEN la pérdida a 10 y a 30 es 0 y ninguna búsqueda corrió la segunda pasada

#### Escenario: una multitud más allá de la ventana
- GIVEN 200 filas de otro proyecto más cerca de la consulta que las 40 del lector
- WHEN corre la consulta
- THEN corrió la segunda pasada y la pérdida a 30 es mayor que 0

#### Escenario: la versión vigente detrás de sus revisiones
- GIVEN una cadena de 10 versiones `superseded` y la vigente, del lector, entre 40 filas del tema
- WHEN corren su búsqueda y su sonda
- THEN las dos corrieron la segunda pasada, la vigente sale primera en la búsqueda y la sonda la devuelve sola

**Tests:** `without_foreign_rows_nothing_is_lost_in_one_pass`, `rows_past_the_widened_window_are_counted_lost`, `the_live_version_behind_its_revisions_is_found_first`.

## KP6. El informe de la pérdida

`what_the_window_loses_against_the_exact_neighbours`, `#[ignore]`, MUST correr cada forma de la propuesta (F1 a F4 y E1) con su semilla y MUST imprimir, por forma: la semilla, cuántas consultas y sondas corrieron, `loss@10` y `loss@30` medios y su p90 y máximo por tipo de consulta (tema propio, tema compartido, cadena de cada largo), la pérdida del detector por cadena de cada largo y en las afirmaciones repetidas, el puesto de la versión vigente por largo, la parte de búsquedas y sondas con segunda pasada, y p50 y p99 de la latencia total y del brazo `knn_memory`. MUST imprimir también, para las formas que deciden, cada umbral (U1 a U3) con su valor sobre lo que decide (temas y cadenas de hasta 20) y si pasa. No necesita el modelo ni un perfil de datos.

La lectura de los umbrales MUST ser pura: deja fuera las cadenas de más de 20 versiones, y una forma abre si pasa uno.

**Tests:** `the_thresholds_read_only_what_decides`; la corrida, `what_the_window_loses_against_the_exact_neighbours` (`cargo test -p clawg-core --lib -- --ignored what_the_window_loses --nocapture`).

## KP7. El costo de la consulta exacta

`what_the_exact_query_costs_against_the_window`, `#[ignore]`, MUST medir sobre tablas de 1.000, 10.000 y 100.000 filas, con el 100 %, el 10 % y el 1 % elegible para el lector, la consulta exacta de KP3 con `L` = 30 y las dos pasadas de vec0 de `SEARCH_SQL_MEMORY` (`k` = 30 y `k` = 120), 30 veces cada una y alternadas. MUST imprimir p50 y máximo de cada una, el cociente de la exacta con la pasada de `k` = 120, el build (debug o release) y el plan de la exacta (`EXPLAIN QUERY PLAN`). MUST comprobar que la exacta y la de `k` = 120 dan las mismas filas cuando todas son elegibles.

**Test:** `what_the_exact_query_costs_against_the_window` (`cargo test -p clawg-core --lib -- --ignored what_the_exact_query_costs --nocapture`).

## KP8. No queda nada

Toda corrida, de la suite o ignorada, MUST abrir su store en un directorio de `crate::test_dir::test_dir()` y MUST cerrarlo antes de terminar, de modo que no quede nada en el temporal del sistema (`.claude/CLAUDE.md`, Testing).

**Tests:** los de KP5, KP6 y KP7, que fallan con `TestDir` si el store queda abierto.

## Invariantes

- Nada de esto compila en un build de producción: el módulo está bajo `#[cfg(test)]`.
- `search.rs`, `contradiction.rs` y el esquema no cambian. La medición llama a `search_all_memories` y `detect_contradiction` tal como los llama el daemon.
- Lo que decide (generador, elegibilidad, pérdida, percentiles, lectura de umbrales) es lógica pura, separada de la base.
