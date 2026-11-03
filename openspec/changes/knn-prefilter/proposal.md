# knn-prefilter — Propuesta (la puerta de medición)

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2, 4 | S–M (sólo la puerta; B1, si la puerta abre, es otro paso con su propia propuesta) | — | ver [`state.yaml`](state.yaml) |

Esta propuesta cubre sólo la [puerta previa](exploration.md#puerta-previa-antes-de-elegir) de la exploración: construir el corpus multiagente, multiproyecto y con cadenas de revisiones, medir la pérdida contra la referencia exacta y el costo de la consulta exacta, y dejar el resultado escrito. Las formas de base y los umbrales se fijan acá, antes de medir, y no se mueven después de ver los números.

## Problema

vec0 corta sus `k` vecinos más cercanos de **toda** `memory_vectors`, y el estado, el agente y el proyecto se aplican después de ese corte (`crates/clawg-core/src/memory/search.rs:244-276`, `knn.k = ?2` en `:266`). Si la primera ventana vuelve corta y el conteo de elegibles (`ELIGIBLE_SQL_MEMORY`, `:331-346`) dice que hay más, `knn_one` repite **una vez** con `retry_window(k) = min(4k, 4096)` (`:483-485`, segunda pasada en `:649`); no hay tercera (`:541-547`). Una fila `superseded` conserva su vector, así que las revisiones de un tema ocupan la ventana justo en las consultas sobre ese tema (exploración, «Las revisiones de un tema»).

Lo que hay medido no alcanza para decidir:

- **A escala real hay ventanas tapadas.** Sobre el historial importado (~52.000 filas), 26 y 13 de 89 casos de ranking-at-scale (86) tuvieron el pool `crowded`: el pool de 30 volvió, después del reintento, con menos de 30 saves aunque el alcance tenía más ([verify de ranking-at-scale](../archive/2026-10-29-ranking-at-scale/verify.md), línea 61). Eso dice que la ventana se tapa, no qué filas se perdieron contra el resultado exacto ni si alguna habría llegado al top 10.
- **La estimación de la exploración supone reparto parejo** (tabla de «Qué se pierde y cuándo») y una medición sintética fuera del repo con vectores aleatorios. Ninguna pasó por `search_all_memories` ni por `detect_contradiction` con las filas armadas para cada fuente de crowding.
- **`rag-eval` no tiene referencia exacta.** Desde ranking-at-scale busca con el alcance del daemon y el proyecto del caso (`crates/clawg-core/src/eval_api.rs:370-388`), pero su corpus es de un agente, sin proyecto y sin revisiones, y su fondo real pide el perfil dev y autorización. Ninguna corrida compara lo devuelto con los vecinos exactos.
- **Nadie midió la consulta exacta.** El costo de B («Enfoques posibles») es «justo lo que hay que medir»: la exploración no tiene un número.

### Premisas que cambiaron desde la exploración (2026-10-19)

| Premisa | Hoy |
|---|---|
| El detector de contradicciones busca con `Reach::OwnOnly` (líneas 19 y 48) | **Ya no.** Desde agent-authorship (76) busca con el alcance de quien guarda: `reach: saver.reach.clone()` (`crates/clawg-core/src/memory/contradiction.rs:173-185`). Las filas de otros agentes son candidatas, no crowding, salvo que `sharing.json` las oculte. El crowding del detector viene de otros proyectos, de agentes ocultos y de las revisiones `superseded`. El test 4 de la exploración («no marca nunca una fila ajena») deja de valer, como anotó la [exploración de agent-authorship](../archive/2026-10-22-agent-authorship/exploration.md), línea 213. |
| Una búsqueda con `limit = 50` necesita `50/s ≤ 120` (tabla de la línea 43) | **El pool es 30.** `knn_one` recorta a `k` saves (`trim_to_saves`, `search.rs:678`) y `limit` sólo corta después (`:1434`), así que la ventana densa no le da a ninguna llamada más de 30 saves (`SearchScope::all_default`, `:40`). Si las 30 filas elegibles son 30 saves distintos, la condición de una llamada con `limit ≥ 30` es `30/s ≤ 120`, es decir `s ≥ 1/4`. La ventana cuenta filas y el pool cuenta saves (`:646`, `:678`): si un save aporta varias piezas, las 120 filas rinden menos saves, y con `r` piezas por save la condición pasa a `s ≥ r/4`. En un store de producción con saves de varias piezas, 1/4 es la cota más favorable. |
| `recall_context` es otra medición | **Comparte la ventana densa, no el resultado final.** Busca con el mismo `SearchScope::all_default()` y el mismo alcance (`crates/clawg-core/src/memory/prompt_builder.rs:227-233`), con `K_FINAL` = 10 más lo ya emitido (`:57`, `:217`), y saca lo ya emitido antes de su corte final (`:254`, `:261`). Esta medición corre sin texto de consulta (decisión 1), mientras las dos herramientas le pasan texto al canal léxico (`crates/clawg-cli/src/daemon/api.rs:1503`, `prompt_builder.rs:230`), que puede recuperar lo que la ventana densa perdió (`search.rs:1421`). Lo medido es la pérdida dentro de esa ventana, que comparten; no es la pérdida final de ninguna de las dos. |
| `rag-eval` usa `Reach::Own` y `project_id: None` (línea 24) | Usa el alcance del daemon y el proyecto del caso (`eval_api.rs:384-388`), como se dijo arriba. Le sigue faltando el corpus y la referencia. |
| Citas | `search.rs:224-233` → `:244-258`; `:246-253` → `:267-273`; `:311-326` → `:331-346`; `:361` → `:381`; `:430-432` → `:483-485`; `:495` → `:548`; `:593` → `:649`; `:620-621` → `:677-678`; `:37-46` → `:38-47`; `contradiction.rs:58` → `:59`; `:178` → `:183`; `:200` → `:206`; `api.rs:75`, `:77` → `crates/clawg-cli/src/daemon/api.rs:94`, `:96`; `:289-298` → `:319-323`. Lo demás sigue donde la exploración lo cita. |

## Objetivo

2 y 4: que lo que un agente puede leer le llegue, tenga lo que tenga el resto de la base. Este paso sólo mide y deja escrito qué se pierde, dónde y cuánto cuesta la alternativa exacta. No toca la búsqueda.

## Decisiones (fijadas antes de medir)

Cierran tres [decisiones abiertas](exploration.md#decisiones-abiertas) de la exploración: las formas de base, el umbral y cómo es el corpus. Las otras (B1 o B2, cómo se anuncia la ruta exacta, si tiene techo, el alcance en contradicciones) se toman en el paso que siga, si la puerta abre, con estos números.

### 1. El corpus: vectores armados, sin texto ni modelo

Cada vector es la suma normalizada de cuatro direcciones unitarias al azar de 384 dimensiones, con pesos fijos:

| Componente | Peso² | Qué representa |
|---|---|---|
| global `g`, la misma para todas las filas | 0,75 | la anisotropía del modelo: dos textos sin relación tienen coseno alto |
| tema `t` | 0,13 | de qué trata la fila |
| afirmación `c` | 0,07 | qué dice; la comparten las versiones de una cadena y las copias de una misma afirmación |
| ruido propio | 0,05 | lo que distingue a la fila |

Las distancias que resultan, y por qué esos valores:

| Par | Coseno | Distancia L2 | Referencia en el código |
|---|---|---|---|
| temas distintos | 0,75 | ≈ 0,71 | debajo del piso de coseno 0,79 (`crates/clawg-core/src/memory/score.rs:187`), es decir de la distancia 0,648: puntúa 0 |
| mismo tema | 0,88 | ≈ 0,49 | encima del piso; cerca del primer par compatible de la calibración del detector (0,469, `contradiction.rs:25-32`) |
| misma afirmación | 0,95 | ≈ 0,32 | dentro de la banda de disputa `[0,10, 0,42)` (`contradiction.rs:38`, `:46`) |

Una consulta de tema lleva `g` y `t` con sus pesos y un ruido propio de 0,12; una de afirmación lleva además `c`. La consulta **no** lleva nada del proyecto: dentro de un tema que varios proyectos comparten, las filas del proyecto propio y las ajenas quedan intercaladas al azar. Es el supuesto neutro, y para el crowding por proyecto el pesimista: una consulta real a veces nombra algo del proyecto y acerca sus filas.

Cómo se reparten las filas de una forma de `N` filas, `A` agentes y `P` proyectos:

- **Cadenas de revisiones:** 15 de cada largo, 5, 10, 20 y 40 versiones `superseded` más la vigente (1.185 filas). Son del agente lector `a0`, repartidas por turno entre los proyectos y alternando un tema propio y uno compartido. Todas las versiones de una cadena comparten su afirmación.
- **Afirmaciones repetidas:** 15, cada una guardada por cada agente en cada proyecto (15·A·P filas), sobre temas compartidos. Es el hecho que un usuario repite en todos sus proyectos («los tests corren con `cargo test`»).
- **El resto:** un 20 % de `N` sin proyecto y lo que queda repartido por igual entre los proyectos. Cada proyecto pone la mitad de sus filas en temas compartidos (los mismos en todos los proyectos, donde también caen todas las filas sin proyecto) y la otra mitad en temas propios, en bloques de unas 40 filas por tema y proyecto. Dentro de un bloque, las filas van por turno a cada agente. Una de cada 20 es `user_preference` y el resto `semantic_fact`; otra de cada 20 está borrada y sin vector, como la deja un borrado (exploración, premisas).
- **Lo que no varía:** toda fila viva está `active`, con importancia 5, tier 3 y `created_at` y `last_accessed_at` un día después del reloj de la corrida. Con un sello adelantado el decaimiento vale exactamente 1 (`score.rs:287-289`), así que el puntaje ordena por distancia y nada más. Sin texto, la búsqueda corre sin canal léxico (`query_text: None`): se mide la ventana densa, que es lo que cambiaría B1.

### 2. Las formas de base

| Forma | Agentes | Proyectos | Filas | Lectura | Decide |
|---|---|---|---|---|---|
| F1 | 1 | 3 | 10.000 | — | sí |
| F2 | 3 | 5 | 10.000 | abierta: sin `sharing.json`, cada agente lee a todos | sí |
| F3 | 3 | 5 | 10.000 | cerrada: cada agente oculta a los otros dos | sí |
| F4 | 3 | 5 | 50.000 | abierta | sí |
| E1 | 5 | 20 | 50.000 | cerrada | no: informa |

- **F1** es un solo cliente con unos pocos proyectos, el uso de hoy: el crowding viene de los otros proyectos y de las revisiones.
- **F2** es la forma de la exploración (3 × 5 × 10.000) con la lectura por defecto. Los otros agentes son elegibles, así que sólo cuentan los proyectos.
- **F3** es la misma con la lectura cerrada, que `sharing.json` permite (objetivo 4): de cada bloque, el lector ve un tercio.
- **F4** es F2 al tamaño de una importación real (54.436 filas en prod, `openspec/README.md`, orden de ejecución). La exploración espera que la pérdida dependa de la fracción ajena y no del tamaño; F4 lo comprueba y da el costo a esa escala.
- **E1** es un estrés: muchos agentes ocultos y muchos proyectos. Se informa y no decide.
- **Las cadenas de 40** están en todas las formas y se informan aparte, sin decidir. Ningún store medido mostró cadenas así de largas: el set de revision-recheck (70) tenía una o dos versiones viejas por tema, y las claves existen desde topic-revisions (27), archivado el 2026-10-18. Las de 5, 10 y 20 deciden.

### 3. Lector, consultas y sondas

- El lector es `a0`, con la lectura de la forma. Busca desde cada proyecto, hasta 5 (en E1, los 5 primeros de 20).
- **Consultas de tema:** por proyecto, hasta 20 temas propios y 20 compartidos.
- **Consultas de afirmación:** una por cadena, desde su proyecto.
- **Sondas del detector:** una por cadena, que debe encontrar la versión vigente, y una por afirmación repetida y proyecto, que debe encontrar las copias que el lector alcanza en ese proyecto. La sonda es un `semantic_fact` sin hermanos, como un save sin clave.
- Cada consulta corre por `search_all_memories` con `SearchScope::all_default()`, el proyecto y el alcance del lector, como el daemon (`api.rs:1496-1507`), y pide 30. Cada sonda corre por `detect_contradiction` (`contradiction.rs:187-249`).

### 4. La referencia exacta

- **Búsqueda:** las `L` filas elegibles más cercanas por `vec_distance_l2`, con los predicados de producción (estado, borrado, alcance y proyecto), entre las que quedan debajo de la distancia del piso de coseno: una fila que puntúa 0 no es algo que la búsqueda pierda. `L` = 10 es el `limit` por defecto y el `K_FINAL` de `recall_context`. `L` = 30 es el pool entero, lo que la puntuación reordena en un store real.
- **Detector:** las 5 filas elegibles más cercanas (el pool sin la ventana), filtradas por la banda `[0,10, 0,42)` y por tipo, como `detect_contradiction`.
- Con el puntaje constante, la referencia es lo que producción devolvería con una ventana que no se tapa. Hace el papel del control de `revision-eval chains`: en una consulta de afirmación, la versión vigente es la primera de la referencia.

### 5. Qué se mide

| Medida | Definición | Por forma |
|---|---|---|
| `loss@10`, `loss@30` | `\|referencia \ resultado\| / \|referencia\|`, promedio sobre las consultas con referencia no vacía | por tipo de consulta: tema propio, tema compartido, cadena por largo |
| pérdida del detector | candidatos de la referencia que `detect_contradiction` no devuelve, sobre los candidatos de la referencia | por cadena según largo, y afirmaciones repetidas |
| la versión vigente | su puesto en el resultado de cada consulta de afirmación | por largo |
| `passes` | parte de las búsquedas y de las sondas que corrieron la segunda pasada (`search.knn_retry`, `search.rs:1360-1364`) | |
| latencia | p50 y p99 del total de la búsqueda y del brazo `knn_memory` (`search.duration`, `:1311`) | |

### 6. Los umbrales

La puerta **abre** si, en alguna forma que decide (F1 a F4), sobre las consultas de tema y las de cadenas de 5, 10 y 20 (y las sondas de esas cadenas y de las afirmaciones repetidas):

| # | Umbral | Lo mide |
|---|---|---|
| U1 | `loss@10` medio > 0,02 | la parte densa de lo que ve una búsqueda por defecto y `recall_context` |
| U2 | `loss@30` medio > 0,05 | el pool que reordena la puntuación |
| U3 | el detector pierde más de 1 de cada 10 candidatos de la referencia (> 0,10) | lo que deja pasar sin par |

- **U1 y U3** son los que propuso la exploración.
- **U2 es nuevo.** En el corpus todas las filas puntúan igual salvo por la distancia, así que el top 10 es el de la distancia. En un store real, el decaimiento, la importancia y el tier reordenan el pool, y una fila 25ª por distancia puede quedar entre las 10 primeras. 0,05 es, en promedio, una fila y media de 30.
- **Si ninguna forma que decide pasa un umbral,** el cambio cierra con D, sin tocar la búsqueda, y deja escritos el corpus, la medición y este criterio.
- **Si alguno pasa,** el paso siguiente es B1 (exploración, «Enfoques posibles»), con su propuesta y sus tests, que relee las premisas de arriba.

### 7. El costo de la consulta exacta

- **Qué se mide.** La consulta de B como la escribe la exploración: `memory` unida a `memory_vectors` por `rowid`, con los predicados de producción, `ORDER BY vec_distance_l2` y `LIMIT 30`. Se compara con las dos pasadas de vec0 sobre la misma tabla: `k` = 30 y `k` = 120, que es la segunda pasada que B1 reemplazaría.
- **Dónde.** Tablas de 1.000, 10.000 y 100.000 filas, con el 100 %, el 10 % y el 1 % elegible para el lector. Para cada celda se dan p50 y máximo de 30 corridas y el plan (`EXPLAIN QUERY PLAN`).
- **K1, criterio para B contra A.** No abre ni cierra la puerta. B1 es viable a un tamaño si la exacta, en p50, cuesta a lo sumo lo que la segunda pasada de vec0 a ese tamaño (cociente ≤ 1). Si pasa de 1, el mayor número de elegibles medido con cociente ≤ 1 es el techo candidato de la decisión «si la exacta tiene techo». Si ya pasa de 1 con 10.000 elegibles, A vuelve a la mesa con el número en la mano.
- **Build.** Se mide en un build debug: un build release pide autorización. Decide el cociente y no el valor absoluto, porque las dos consultas corren el mismo C de SQLite y sqlite-vec con la misma optimización.

## Alcance

- **El corpus y su medición**, en un módulo sólo de tests de `clawg-core` (`crates/clawg-core/src/memory/crowding.rs`, `#[cfg(test)]`). Necesita `detect_contradiction`, los predicados de alcance y proyecto y los umbrales del detector, que son `pub(crate)`, y no agrega visibilidad ni toca `search.rs` ni `contradiction.rs`.
  - Lo puro, con sus tests en la suite: el generador determinista con semilla, la elegibilidad según los predicados de producción, la pérdida y los percentiles.
  - Una corrida chica en la suite que prueba la medición misma:
    - sin filas ajenas, la pérdida es 0 y no hay segunda pasada;
    - con más filas ajenas que la ventana ensanchada, la pérdida es mayor que 0;
    - la consulta exacta en SQL devuelve las mismas filas que la referencia pura;
    - lee las pasadas que cuenta la búsqueda.
  - Dos tests `#[ignore]` que imprimen el informe: la pérdida por forma y el costo de la exacta. No necesitan el modelo ni un perfil de datos. Abren el store en un directorio de `test_dir()` y lo cierran.
- **El resultado**, en [`verify.md`](verify.md) de este cambio: cada medida por forma, cada umbral con su lectura y la decisión de la puerta.
- **[`state.yaml`](state.yaml) y la fila 37 de `openspec/README.md`.**

### No-alcance

- Cambiar `search.rs`, el esquema, `memory_vectors` o el detector de contradicciones.
- B1, B2 o A: los decide el paso siguiente, si la puerta abre.
- `rag-eval` y su golden, y el modelo real.
- Subir la versión y archivar.

## Cómo se mide el éxito

- `verify.md` lee cada umbral (U1 a U3, y K1) con los números de la corrida, contra los valores de esta propuesta.
- Tests que fallan hoy, porque el módulo no existe: los del generador, la elegibilidad, la pérdida y los percentiles, y la corrida chica que prueba la medición.
- La suite y los dos clippy pasan.

## Migración y vuelta atrás

No cambia nada guardado ni nada que compile un build de producción: el módulo está bajo `#[cfg(test)]`. Volver atrás es revertir sus commits.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| La geometría sintética sobrestima o subestima el crowding real | Se fija acá, antes de medir; el informe separa tema propio de compartido, que es donde el supuesto pesa |
| La consulta sin nada del proyecto exagera el crowding por proyecto | Es el supuesto neutro, y B1 sólo cuesta a quien hoy ya paga la segunda pasada. Si la puerta abre sólo por temas compartidos, `verify.md` lo dice |
| El puntaje constante oculta lo que reordena un store real | U2 mide el pool entero y no sólo el top 10 |
| Un resultado atado a una semilla | La semilla está fija y en el informe. Las consultas y sondas se cuentan por forma, para que se vea cuántas sostienen cada número |
| El costo en debug no es el de release | Decide el cociente con vec0, que corre el mismo C (decisión 7) |
| Mover un umbral después de ver el número | Los umbrales están en esta propuesta, que se commitea antes de medir |
