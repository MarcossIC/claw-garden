# knn-prefilter — Verificación (la puerta de medición)

**La puerta abre.** F3, la lectura cerrada, pierde el 20,7 % del pool de 30 contra la referencia exacta (U2, umbral 0,05). U1 y U3 quedan debajo en las cuatro formas que deciden, y F1, F2 y F4 no pasan ningún umbral. La consulta exacta, tal como la escribe la exploración, cuesta de 1,01 a 18,33 veces la segunda pasada que reemplazaría; con 10.000 filas elegibles, 2,69 veces. Por K1, A vuelve a la mesa. Qué paso sigue, B1, una variante o A, no lo decide esta medición (abajo, «Lo que sigue»).

Los umbrales y las formas son los de la [propuesta](proposal.md), commiteada en `c4ef39b` antes de escribir la medición.

## Cómo se corrió

| | |
|---|---|
| Código | `93be98b` (`crates/clawg-core/src/memory/crowding.rs`) |
| Build | debug, `cargo test -p clawg-core --lib`; release no se corrió, porque pide autorización |
| Máquina | Linux 6.18 x86_64, Intel Xeon @ 2,10 GHz, 4 CPU |
| Semilla | 37, la misma para todas las formas |
| Store | en disco, en un directorio de `test_dir()`, cerrado al terminar; no queda nada en el temporal |
| Modelo y perfiles | ninguno |
| Duración | la pérdida, 672 s; el costo, 237 s |

```
cargo test -p clawg-core --lib -- --ignored what_the_window_loses --nocapture
cargo test -p clawg-core --lib -- --ignored what_the_exact_query_costs --nocapture
```

### Primera corrida (descartada)

La primera corrida, sobre `4431f98`, decidía «una de cada 20» con un contador. En E1, con 24 temas por clase y 5 agentes, el contador volvía al mismo agente y tema cada 120 filas, múltiplo de 20, y dejaba temas con todas las filas del lector borradas: 2 de 100 búsquedas de tema propio no tenían referencia. En F1 a F4 esa vuelta es de 29, 51 y 291 filas, que no son múltiplos de 20, así que no los tocaba. `93be98b` sortea cada fila, con el test `preferences_and_deletions_fall_apart_from_agents_and_topics`, que falló con el contador.

Las dos corridas usan corpus distintos (el sorteo cambia todos los vectores que siguen) y leen la puerta igual. En F3, U2 dio 0,2131 en la primera y 0,2072 en la segunda; las demás formas que deciden quedaron debajo de los tres umbrales en las dos. Los números de abajo son de la segunda.

## Los umbrales

Sobre las búsquedas de tema y las de cadenas de hasta 20 versiones, y sobre las sondas de esas cadenas y de las afirmaciones repetidas (decisión 6 de la propuesta):

| Forma | Decide | U1 `loss@10` (> 0,02) | U2 `loss@30` (> 0,05) | U3 detector (> 0,10) | Lectura |
|---|---|---|---|---|---|
| F1: 1 agente, 3 proyectos, 10.000 | sí | 0,0000 | 0,0000 | 0,0111 | no abre |
| F2: 3, 5, 10.000, abierta | sí | 0,0000 | 0,0014 | 0,0037 | no abre |
| F3: 3, 5, 10.000, cerrada | sí | 0,0028 | **0,2072** | 0,0083 | **abre (U2)** |
| F4: 3, 5, 50.000, abierta | sí | 0,0000 | 0,0000 | 0,0037 | no abre |
| E1: 5, 20, 50.000, cerrada | no | 0,0865 | 0,4482 | 0,5167 | informa |

## Lo que pierde la búsqueda

`loss@10` y `loss@30` son la media por tipo de consulta; entre paréntesis, el p90 y el máximo cuando no son 0. Todas las consultas tuvieron referencia (filas elegibles bajo la distancia del piso).

| Consulta | n por forma (F1 / F2–F3 / F4–E1) | F1 | F2 | F3 | F4 | E1 |
|---|---|---|---|---|---|---|
| tema propio, `loss@10` | 60 / 85 / 100 | 0 | 0 | 0 | 0 | 0,011 (0 / 0,300) |
| tema propio, `loss@30` | | 0 | 0 | 0,035 (0,167 / 0,400) | 0 | 0,201 (0,600 / 0,733) |
| tema compartido, `loss@10` | 60 / 85 / 100 | 0 | 0 | 0,007 (0 / 0,600) | 0 | 0,160 (0,500 / 0,600) |
| tema compartido, `loss@30` | | 0 | 0,004 (0 / 0,300) | 0,376 (0,533 / 0,867) | 0 | 0,691 (0,833 / 0,867) |
| cadena de 5, `loss@30` | 15 | 0 | 0 | 0,133 (0,400 / 0,400) | 0 | 0,280 (0,533 / 0,600) |
| cadena de 10, `loss@30` | 15 | 0 | 0 | 0,330 (0,600 / 0,633) | 0 | 0,571 (0,833 / 0,867) |
| cadena de 20, `loss@30` | 15 | 0 | 0 | 0,178 (0,500 / 0,533) | 0 | 0,522 (0,800 / 0,833) |
| cadena de 40, `loss@30` (informa) | 15 | 0 | 0,013 (0,100 / 0,100) | 0,388 (0,633 / 0,733) | 0,020 (0,067 / 0,167) | 0,358 (0,667 / 0,667) |

`loss@10` de las cadenas es 0 en F1 a F4 salvo la de 40 en F3 (0,013, máximo 0,200). En E1 es 0, 0,167, 0,107 y 0,020 para 5, 10, 20 y 40.

- **La pérdida está en el pool, no en el top 10.** En F1 a F4 la ventana ensanchada (120) alcanza a las 10 primeras elegibles de cada consulta. En F3 no alcanza a las 30: el lector ve un tercio de cada bloque, y en un tema compartido la parte elegible de los vecinos queda cerca de 1/6, bajo el 1/4 que pide un pool de 30 (premisas de la propuesta). Coincide con la estimación de la exploración.
- **F3 no abre sólo por los temas compartidos.** Los temas propios quedan en 0,035, bajo U2. Las cadenas de 5 a 20 pierden entre 0,133 y 0,330, y alternan temas propios y compartidos, así que mezclan las dos causas: las versiones `superseded` y el tercio que se ve de cada bloque.
- **La versión vigente sale primera en todas las búsquedas de afirmación**, de todo largo y en todas las formas (15 de 15 en cada celda). Las revisiones no la tapan en la búsqueda: lo que pierden es lo que viene detrás de ella.
- **El tamaño no cambia la pérdida.** F4 (50.000 filas) no pierde más que F2 (10.000), como esperaba la exploración: decide la fracción ajena.

## Lo que pierde el detector

Candidatos de la referencia que `detect_contradiction` no devolvió, sobre los candidatos de la referencia:

| Sonda | F1 | F2 | F3 | F4 | E1 |
|---|---|---|---|---|---|
| cadena de 5 | 0/15 | 0/15 | 0/15 | 0/15 | 0/15 |
| cadena de 10 | 0/15 | 0/15 | 0/15 | 0/15 | 0/15 |
| cadena de 20 | 1/15 | 1/15 | 1/15 | 1/15 | 2/15 |
| cadena de 40 (informa) | 7/15 | 8/15 | 8/15 | 5/15 | 8/15 |
| afirmación repetida | 0/45 | 0/225 | 0/75 | 0/225 | 60/75 |

- **Las cadenas largas tapan al detector con un solo agente.** Con 40 versiones el detector no ve la vigente en 5 a 8 de 15 sondas, también en F1. Su ventana ensanchada es de 20, y la vigente está entre 41 filas igual de cerca. No decide, porque la propuesta dejó las cadenas de 40 como estrés, pero no depende ni de agentes ni de proyectos.
- **Las afirmaciones repetidas sólo se pierden en E1**, con 100 copias (5 agentes × 20 proyectos) y una sola elegible: la ventana de 20 la encuentra 15 de 75 veces.

## Lo que cuesta hoy

Parte de las búsquedas y sondas que corrieron la segunda pasada (`search.knn_retry`), y la latencia en debug:

| | F1 | F2 | F3 | F4 | E1 |
|---|---|---|---|---|---|
| tema propio, segunda pasada | 21/60 | 30/85 | 85/85 | 20/100 | 100/100 |
| tema compartido, segunda pasada | 60/60 | 85/85 | 85/85 | 100/100 | 100/100 |
| cadenas y sondas, segunda pasada | todas | todas | todas | todas | todas |
| búsqueda, p50 / p99 | 40,6 / 62,6 ms | 40,1 / 55,7 ms | 44,5 / 55,5 ms | 173,6 / 233,4 ms | 180,0 / 229,7 ms |
| su brazo `knn_memory`, p50 / p99 | 39,3 / 61,1 ms | 38,9 / 54,0 ms | 43,2 / 53,9 ms | 172,4 / 231,1 ms | 178,9 / 228,1 ms |
| sonda, p50 / p99 | 36,0 / 46,3 ms | 36,2 / 45,8 ms | 38,4 / 53,4 ms | 158,4 / 202,4 ms | 160,0 / 200,4 ms |

Con la lectura abierta nada se pierde, pero cada búsqueda de un tema compartido paga dos pasadas. Es lo que la exploración llamaba «quién paga el reintento»: casi todo el tiempo de una búsqueda es el brazo de vec0.

[search-arm-timing](../archive/2026-11-03-search-arm-timing/exploration.md) (101) muestra que cada brazo de `search.duration` suma lo que tardaron los anteriores en la cola de la conexión. Estas cifras no cambian por eso:

- `knn_memory` es el primer brazo que entra en la cola, y cada búsqueda medida corre sola y sin canal léxico.
- `total` mide la búsqueda de punta a punta.
- K1 no lee esas series: cronometra cada consulta dentro del hilo de la conexión (`timed` del módulo).
- La puerta se lee por pérdidas, no por duraciones.

## El costo de la consulta exacta (K1)

La consulta de B como la escribe la exploración (`EXACT_SQL` del módulo: `memory` unida a `memory_vectors` por `rowid`, los predicados de producción, `ORDER BY vec_distance_l2`, `LIMIT 30`) contra las dos pasadas de vec0 sobre la tabla entera. p50 / máximo de 30 corridas, en ms, en debug:

| Filas | Elegibles | Exacta | vec0 `k` = 30 | vec0 `k` = 120 | Exacta / `k` = 120 | Brazo `knn_memory` de la búsqueda (segundas pasadas) |
|---|---|---|---|---|---|---|
| 1.000 | 100 % | 30,66 / 39,15 | 1,69 / 2,65 | 1,84 / 3,41 | 16,70 | 2,44 / 4,68 (0/30) |
| 1.000 | 10 % | 3,83 / 4,03 | 1,43 / 1,55 | 1,67 / 1,78 | 2,29 | 4,28 / 5,63 (30/30) |
| 1.000 | 1 % | 2,13 / 2,32 | 1,84 / 2,01 | 2,11 / 2,28 | 1,01 | 5,81 / 7,51 (30/30) |
| 10.000 | 100 % | 337,42 / 406,34 | 18,08 / 22,03 | 19,66 / 25,95 | 17,16 | 18,74 / 22,75 (0/30) |
| 10.000 | 10 % | 47,20 / 66,60 | 17,68 / 22,93 | 19,60 / 25,45 | 2,41 | 39,32 / 50,31 (30/30) |
| 10.000 | 1 % | 20,81 / 31,46 | 17,65 / 22,37 | 20,22 / 25,02 | 1,03 | 40,48 / 63,93 (30/30) |
| 100.000 | 100 % | 3.465,28 / 4.288,01 | 172,20 / 208,06 | 189,04 / 236,87 | 18,33 | 163,33 / 198,71 (0/30) |
| 100.000 | 10 % | 504,74 / 664,98 | 165,70 / 199,08 | 187,58 / 220,95 | 2,69 | 345,04 / 448,65 (26/30) |
| 100.000 | 1 % | 208,23 / 312,58 | 164,65 / 200,92 | 183,13 / 226,51 | 1,14 | 344,28 / 435,45 (29/30) |

Con todas las filas elegibles, la exacta devolvió las mismas 30 filas que la pasada de `k` = 120 en cada corrida.

El plan (`EXPLAIN QUERY PLAN`, igual en los tres tamaños):

```
SCAN v VIRTUAL TABLE INDEX 0:1 | SEARCH m USING INTEGER PRIMARY KEY (rowid=?) | LIST SUBQUERY 1
| SCAN json_each VIRTUAL TABLE INDEX 3: | CREATE BLOOM FILTER | USE TEMP B-TREE FOR ORDER BY
```

- **El planificador recorre vec0 entero** y busca cada fila en `memory` por su id. La exacta no lee sólo las elegibles: recorre la tabla de vectores, como vec0, y además calcula la distancia de cada fila elegible. Eso explica el piso de alrededor de 1× con el 1 % elegible.
- **Cada fila elegible cuesta unos 34 µs**: 3.465 ms / 100.000 y 337 ms / 10.000. Una fila de la pasada de vec0 cuesta unos 1,9 µs. La exacta es lineal en las elegibles, como preveía la exploración, pero con una pendiente unas 18 veces la de vec0.
- **K1, la lectura:** ninguna celda queda en un cociente ≤ 1; la más baja es 1,01, con 10 filas elegibles. Así que no hay techo candidato entre lo medido. Con 10.000 elegibles el cociente es 2,69 (tabla de 100.000) y 17,16 (tabla de 10.000), por encima de 1: **A vuelve a la mesa, con estos números.**
- **Debug.** Los valores absolutos son de un build debug, y el C de SQLite y sqlite-vec corre con la optimización del perfil. Decide el cociente, que compara dos consultas del mismo C (decisión 7).

## Lo que sigue (no lo decide este paso)

La propuesta dice que, si la puerta abre, el paso siguiente es B1, con su propuesta y sus tests. La medición deja tres datos para ese paso:

1. **B1 tal como está escrito costaría a quien hoy no pierde nada.** B1 corre la exacta cuando la primera ventana vuelve corta, y en F2 y F4 eso pasa en todas las búsquedas de temas compartidos, sin pérdida. Con el lector de F2, que alcanza más de un tercio de las filas, la tabla de costo pone la exacta varias veces por encima de la pasada que reemplaza. Es una estimación hecha con la tabla, no una medición.
2. **Una variante que la medición sugiere:** correr la exacta sólo si la segunda ventana también vuelve con menos filas elegibles que el pool y el conteo dice que hay más. Sale de lo que `knn_one` ya cuenta, sin otra consulta. En F2 y F4 no correría nunca; en F3 sí, y ahí se pierde el 20,7 % del pool. Estimación con la tabla, para un lector de F3 (≈ 12 % de las filas elegibles, según el corpus): el brazo de vec0 de una búsqueda tapada pasaría de unos 39 ms a unos 86 ms en debug con 10.000 filas, sumando la exacta a las dos pasadas de hoy.
3. **A**, la partición y la metadata en vec0, es la que la regla K1 vuelve a poner sobre la mesa. Una partición reduce el recorrido al proyecto (exploración, «Enfoques posibles»). Sus costos (re-key, `live`, migración) no cambiaron.

Quedan abiertas las decisiones de la exploración que este paso no tomó: B1 o B2, cómo se anuncia la ruta exacta, si tiene techo y el alcance en contradicciones. A eso se suma si F3 sigue siendo una forma que decide, ahora que es la única que abre.

**Recomendación para ese paso:** la variante del punto 2. Cubre la única forma que abre sin cobrarles a F2 y F4, mientras que A paga re-key, `live` y una migración por esa misma forma. Lo que la decide es si F3, la lectura cerrada, sigue siendo una forma que importa: si no, no hace falta ninguna de las dos.

## El KNN en uso real (2026-11-03)

Otra pregunta y otra fuente que la puerta: `clawg metrics report --by version`, con una ventana de 30 días, sobre un perfil de producción en Windows con más de 54.000 filas importadas. La pregunta era si la etapa `store` del worker pesa, y qué la explica.

`ingest.stage`, con `--label stage=<etapa>`. Una toma es una pasada del worker por un guardado:

| Versión | Etapa | Tomas | p50 | Media | p95 | Máx |
|---|---|---:|---:|---:|---:|---:|
| 0.71.0 | `embed` | 26 | 94 ms | 450 ms | 1.280 ms | 1.420 ms |
| 0.71.0 | `store` | 26 | 119 ms | 132 ms | 314 ms | 326 ms |
| 0.72.2 | `embed` | 49 | 73 ms | 302 ms | 1.161 ms | 1.322 ms |
| 0.72.2 | `store` | 49 | 115 ms | 186 ms | 1.105 ms | 1.316 ms |

- `store` supera a `embed` en la mediana de las dos versiones, y en 0.72.2 su p95 llega al 95 % del de `embed`. Los p50 son interpolados, pero caen en buckets distintos: la dirección es firme aunque el valor exacto no.
- `embed` sigue costando más en total: en 0.72.2, la media de `store` es el 62 % de la de `embed`.
- La cola creció: en 0.72.2, 3 de 49 tomas de `store` pasan de 1 s; en 0.71.0 ninguna pasa de 326 ms.
- No urge: las 49 tomas de `store` suman unos 9 s.

Lo que quedó descartado como causa:

- **La espera en la conexión:** `db.probe_wait` da, en 0.72.2, p50 de 50 µs y p99 de 240 µs en 13.273 sondeos. No lo descarta del todo para un guardado puntual, porque el sondeo corre cada 10 s y no coincide con los guardados.
- **Las piezas, como causa de que creciera la cola:** `ingest.pieces` tiene casi la misma forma en las dos versiones (media 1,54 y 1,55, máximo 8), pero el máximo de `store` pasó de 326 a 1.316 ms.
- **El código:** entre v0.71.0 y v0.72.2 no cambió ninguna dependencia, y el único commit que toca los archivos de la etapa `store` es de tests (`c7d91b0`).

`search.duration`, con `--label arm=knn_memory`. El detector pasa por este brazo (`contradiction_search_scope` sólo pide `memory`), junto con las búsquedas normales, y la serie no los separa:

| Versión | Tomas | Mín | p50 | Media | p95 | Máx |
|---|---:|---:|---:|---:|---:|---:|
| 0.71.0 | 25 | 57 ms | 183 ms | 163 ms | 344 ms | 357 ms |
| 0.72.2 | 93 | 57 ms | 164 ms | 171 ms | 438 ms | 612 ms |
| 0.73.0 | 19 | 75 ms | 175 ms | 248 ms | 874 ms | 874 ms |

Sólo 0.73.0 tiene el reloj de R14 de [`specs/metrics`](../../specs/metrics/spec.md): hasta 0.72.2 cada brazo suma las esperas, así que esas filas no se comparan con la última.

**Lectura.** El criterio se fijó antes de leer: un p50 cerca de 100 ms quiere decir que el KNN explica `store` y no hacen falta timers; muy por debajo, el costo está en el insert o el commit, y valen dos timers en `store_pieces`, alrededor de `detect_contradiction` e `insert_unless_twin`. Dio 175 ms, así que el KNN explica la mayor parte de `store` y los timers no hacen falta. También encaja con la cola: al menos 28 de los 49 guardados de 0.72.2 son de una pieza, y a 115 ms por pieza, 8 piezas dan 0,9 s, cerca de 1,3 s. Un tipo que no afirma nada, como `session_summary`, saltea el KNN (`is_claim_bearing`, `crates/clawg-core/src/memory/contradiction.rs:197`), lo que podría explicar que el guardado de 8 piezas de 0.71.0 no pasara de 326 ms.

Es una dirección, no una medida del detector:

- El p50 del brazo supera al de `store` entero (115 ms), que lo contiene. Son poblaciones distintas: la serie mezcla búsquedas normales y no dice cuántas tomas son de guardados. El dato más firme es el mínimo: ninguna toma de 0.73.0 bajó de 75 ms.
- Son 19 tomas de una sola ejecución, con el p95 igual al máximo.
- El `store` comparado es de 0.72.2: cuando se leyó, `ingest.stage` no tenía grupo 0.73.0. Con más uso, repetir la lectura de `stage=store` compara la misma versión.

**Para el paso siguiente:** esta pasada la paga cada búsqueda y cada guardado que afirma algo, no sólo una búsqueda tapada. B1 y la variante no la bajan, porque no la tocan. A podría, porque reduce el recorrido al proyecto, pero no está medido cuánto. No cambia la recomendación de arriba, porque este costo no urge; sí es un argumento a favor de A que la puerta no tenía.

## Lo que no se midió

- **El canal léxico.** La búsqueda corrió sin texto (decisión 1). Una fila que la ventana perdió puede volver por FTS5 si comparte palabras con la consulta.
- **Una consulta que nombra algo del proyecto.** El supuesto neutro intercala las filas propias y ajenas de un tema compartido. Con una consulta que acerque las propias, F3 perdería menos.
- **Otras formas de la consulta exacta.** Por ejemplo, una que lea los vectores elegibles sin recorrer vec0 entero. Sólo se midió la de la exploración.
- **Release y el modelo real.** No se corrieron.
- **Otra semilla.** La semilla es una. Las dos corridas, con corpus distintos, leyeron la puerta igual («Primera corrida»).

## Suite y clippy

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` pasan, en Linux, sobre el último commit.
