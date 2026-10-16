# english-recall — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | project-indexing (12) y eval-rigor (33), archivados | ver [`state.yaml`](state.yaml) |

## Problema

Con el pooling mean, R@5 de los casos «en inglés» cae de 0,68 a 0,45. La [exploración](exploration.md) mostró que no es el inglés sino el cruce: separando por pareja consulta–fila, español→español queda en 0,90, inglés→inglés sube de 0,71 a 0,86 y **español→inglés (n = 19) cae de 0,68 a 0,32** en `retrieval` y en coseno puro. En coseno puro pierde 7 casos y no gana ninguno. Esas parejas salen de una lectura a mano: hoy ninguna herramienta las ve. Verificado contra `main` (`49bd269`):

1. **El caso no tiene idioma propio.** `RetrievalCase` no tiene campo de idioma (`crates/clawg-eval/src/bin/rag-eval/golden.rs:58-90`); `case_lang` le asigna el de sus filas esperadas (`crates/clawg-eval/src/bin/rag-eval/main.rs:1212-1229`, usado en `:245`), y eso es `CaseResult.lang` (`report.rs:77-81`). `retrieval.by_lang` (`report.rs:186`, `:535-551`) llama «en» a 15 consultas en español y 7 en inglés.
2. **Ningún bloque desglosa por pareja.** `similarity` no desglosa por idioma (`report.rs:335-368`); agent-view sólo por subconjunto (`agent_view/mod.rs:155-158`) y `CaseView` no lleva idioma (`agent_view/mod.rs:71-89`).
3. **`--diff` no puede probar el criterio.** El McNemar exacto corre sobre todos los casos unidos (`diff.rs:282-301`, `:250-275`), nunca sobre los de una pareja.
4. **Cuatro filas mal etiquetadas:** c01, c08, c09 y c35 dicen `es` y están escritas en inglés. `lang` no entra en la siembra (`seed.rs` no lo lee; `main.rs:208-212` sólo lo pasa a `case_lang`), así que corregirlas no mueve ninguna métrica por caso.
5. **El prefijo nunca se midió con mean.** Los dos roles mandan `"query: "` (`crates/clawg-core/src/memory/embedder.rs:102`, `:123`), y el comentario dice que el `"passage: "` descartado se midió con CLS (`:115-122`).

Correcciones a la exploración: sus citas son de antes de `49bd269` (las de arriba están verificadas), y `--diff` **sí** compara agent-view desde eval-rigor (`diff.rs:196-199`).

**Base.** Después del ancla de eval-rigor (`210c692`, `2026-10-16-real-eval-rigor-tip-anchor.json`) no entró nada que toque el ranking: `search.rs`, `score.rs`, `crates/clawg-cli/src/relevance.rs`, los prefijos y `clawg-eval` no cambiaron. Agent-view sobre el ancla, en cambio, se midió por última vez en `04ab3cd` (`2026-10-15-real-long-save-ranking-tip-agent-view.json`: `search_memory` 36/46 a 764,0 tokens, `recall_context` 39/46 a 1.086,4). Después entraron los lotes de 16 (`063dc51`), la arena de ONNX apagada (`78880f2`), el outbox (36), el enmascarado nuevo (42) y la descarga del modelo ocioso (46). Ninguno toca el ranking, pero agent-view pasa por la ingesta real y nadie lo volvió a medir: el paso 0 lo vuelve a medir.

## Decisiones

Las del 2026-10-16 (exploración) son vinculantes: sólo diagnóstico, brazos de prefijo compilados en un worktree descartable, el criterio McNemar sobre la pareja, el golden cruzado aparte y las etiquetas reales. Esta propuesta cierra las que quedaban:

- **Idioma de la consulta.** `query_lang` opcional en el caso. La pareja de un caso es `(query_lang, lang)`, donde `lang` sigue saliendo de `case_lang`. Un caso al que le falta alguno de los dos no tiene pareja, igual que hoy `by_lang` deja afuera un caso sin idioma unánime (`report.rs:528-534`).
- **Regla de etiquetado.** El idioma de una fila o consulta es el de su prosa; los identificadores y el código no cuentan. c43, c44 y c46 son casi sólo identificadores, y su prosa está en inglés.
- **`golden.json` lleva `query_lang` en sus 46 casos**, además de corregir las cuatro filas (decidido el 2026-10-16). Es una anotación, como la decisión 5: no cambia consultas, filas ni esperadas.
- **McNemar por pareja: el corte va dentro de `--diff`.**

| | A. Corte por pareja en `--diff` (elegida) | B. Un golden con sólo esa pareja |
|---|---|---|
| Código | `diff.rs` agrupa por pareja los casos unidos; es puro y lleva tests | ninguno |
| Corridas por brazo | una (más agent-view), que da el ancla y todas las parejas | una por pareja más el ancla, y otra vez en agent-view |
| Corpus | el mismo store de 48 filas | tiene que copiar las 48 filas: `similarity` rankea contra todo el corpus (`main.rs:1130-1133`), y con menos filas el problema se vuelve más fácil |
| Deriva | las etiquetas viven en un solo archivo | casos y etiquetas copiados que se desincronizan del ancla |
| Riesgo | más filas con p invitan a leerlas todas | ninguno nuevo |

El riesgo de A se acota así: las filas por pareja son informativas, y `primary:` y `--fail-on-regression` no cambian (`diff.rs:46-49`, `:350-354`). Lo que decide este cambio es la fila preregistrada (U1).

- **Bloque del criterio (decidido el 2026-10-16).** U1 se lee en `similarity`: coseno puro, sin piso ni corte. El piso 0,79 está atado a los prefijos con que se midió: cambiar cualquiera de los dos «silently invalidates this floor» (`crates/clawg-core/src/memory/score.rs:174-181`). Leer U1 en `retrieval` con las constantes de hoy mediría el prefijo junto con un piso mal calibrado y dejaría en desventaja a cualquier brazo. `retrieval` y agent-view se miden igual y se anotan: dicen cuánto tendría que recalibrar C, pero no deciden (ver la lectura de (b)).

## Alcance

1. **Código, sólo en `clawg-eval`:**
   - `golden.rs`: `query_lang: Option<String>` en `RetrievalCase`, con `#[serde(default)]`.
   - `report.rs`: `CaseResult.query_lang` (por defecto `None`; no se escribe si falta), y `by_pair` en `RetrievalResults` y en `SimilarityResults`, con `n_cases`, `aggregate` y `hits` (Wilson). El bloque no aparece si ningún caso tiene pareja. La consola suma una tabla por pareja.
   - `agent_view`: `CaseView` suma `query_lang` y `row_lang`, y `by_pair` funciona como `by_subset`.
   - `diff.rs`: un bloque por pareja para `retrieval`, `similarity` y `agent_view`, con `better`, `worse`, `net` y p sobre los casos que tienen la misma pareja en los dos archivos. Un caso cuya pareja difiere entre los dos no entra en ninguna y se informa aparte, con un conteo.
   - El texto de ayuda de `--diff` (`cli.rs:169-177`) y la línea de `.claude/CLAUDE.md` que explica cómo comparar dos corridas.
2. **Datos, fuera de git (`docs/eval/`):** las etiquetas de `golden.json`, y `golden-cross.json`, con las mismas 48 filas, un caso inglés→español por cada fila en español y una segunda consulta en español por cada fila en inglés. Se escribe antes de medir cualquier brazo y su sha queda fijado en el registro.
3. **La medición** de (a) a (f), con los umbrales de abajo.
4. **El registro** en `verify.md`.

## Fuera de alcance

- Cualquier cambio de comportamiento en `clawg-core`: prefijos, pooling, modelo, piso (`score.rs:182`), `RELEVANCE_RATIO` (`crates/clawg-cli/src/relevance.rs:19`), la banda `[0,10, 0,42)` (`contradiction.rs:58`, `:66`), el canal léxico y el esquema.
- Un constructor o un flag de prefijo sólo para evaluar.
- Detectar el idioma, en el store o en `rag-eval`.
- Reescribir consultas de `golden.json` o sumarle casos.
- Recomendar un idioma para los saves: va a memory-protocol (22).
- Un `--fail-on-regression` por pareja, y mezclar este trabajo con memory-rank-fusion (34).
- **El cambio de prefijo (C).** Si (b) gana, es un cambio aparte. Arrastra model-reembed (17): `is_pooling_upgrade` exige los mismos prefijos (`bootstrap.rs:380-385`), y otro desajuste deja la memoria apagada. También el test que fija los prefijos (`embedder.rs:858-866`), el índice de docs, que sólo reindexa cuando cambia `model_id` (`project_docs/mod.rs:108-114`), y volver a medir el piso, el ratio y la banda.

## Cómo se mide

Todo corre en local y necesita dos cosas que no están en git: el golden set (`docs/eval/golden.json`, que cada uno arma con `golden-mine` a partir de sus memorias) y el modelo en el caché del perfil. `rag-eval` siembra un store temporal con ellos y nunca abre un store real.

### Paso 0: la base

Sobre el commit de `main` que trae el código (el que sigue a `49bd269`), con `golden.json` ya etiquetado:

1. `rag-eval --real --tag english-recall-base`: su `--diff` contra `2026-10-16-real-eval-rigor-tip-anchor.json` tiene que dar 0 casos movidos en todas las métricas de `retrieval` y de `similarity`. Si algo se mueve, se para y se busca la causa antes de cualquier brazo. El `content_sha256` cambia sólo por las etiquetas.
2. `rag-eval --real --agent-view --tag english-recall-base`, y su `--diff` contra el archivo de `04ab3cd`. Si algo se mueve, el archivo nuevo pasa a ser la base y el movimiento se anota con su p.
3. Los mismos dos con `--golden docs/eval/golden-cross.json`, que dan la base de U6.

### Hipótesis

| | Medición | Qué significa |
|---|---|---|
| a. El piso | `--sweep-cos-floor` 0,70, 0,75, 0,79, 0,81 y 0,83 sobre la base, leído por pareja | El piso sólo existe en `retrieval`, así que se lee la prueba de U1 en ese bloque. Si ningún piso la cumple contra 0,79, queda descartada, como ya sugiere la exploración. Si alguno lo da, se reabre como un cambio propio que elige el piso leyendo `by_split.calibration` |
| b. El prefijo | Brazos A1 `"query: "`/`"passage: "` y A2 `""`/`""` contra A0, la base. Un worktree por brazo, desde el commit del paso 0, cambiando sólo `embedder.rs:102` y `:123`, con su propio `--target-dir` y `--tag english-recall-b-a1` o `-a2`, sobre los dos goldens, con y sin `--agent-view`. `similarity.query_prefix`/`passage_prefix` de cada archivo prueban qué brazo corrió (`main.rs:1203-1208`) | Ver la tabla de lectura de abajo |
| c. Composición | Regla fijada ahora: una fila «con código» tiene al menos un tramo entre comillas invertidas. Se cuentan los fallos de español→inglés con código y sin código en A0 y en cada brazo. Control: `golden-c-prosa.json`, con esos tramos quitados de las filas esperadas de español→inglés, medido en A0 | Es informativa: con unas 6 filas con código, sólo un 6 a 0 llega a p < 0,05. Si el control recupera los casos con código, es una pista para (c) y no una conclusión |
| d. Lo léxico | `--lexical-only` sobre la base, leído por pareja | Una consulta en español no comparte palabras con una fila en inglés, así que se espera español→inglés cerca de 0. Si es así, el canal léxico no rescata el cruce y no se toca. Si acierta, se anota qué identificadores comparten |
| e. Traducción | `golden-e-consulta-en.json` (las 19 consultas en inglés, a mano) y `golden-e-fila-es.json` (las 19 filas en español, a mano), medidos en A0 y en el brazo ganador si lo hay. El segundo se une por `query`; el primero se aparea a mano por la fila esperada, porque cada caso espera una fila distinta (46 casos, 46 filas) | Si alguna de las dos variantes cumple U1 contra la base, el cruce explica la brecha: la fila se encuentra en su propio idioma. Si ninguna, pesa más el contenido (c). Si es parcial, se anota como mixto |
| f. Etiquetas y n | El código y las etiquetas. El `by_pair` del paso 0 tiene que reproducir la tabla de la exploración (n 20/7/19; R@5 0,90/0,86/0,32 en los dos bloques), con su intervalo de Wilson | Si no la reproduce, el registro usa las cifras del paso 0 y dice por qué |

Las variantes de (c) y (e) son controles fuera de git: se escriben y se fijan por sha antes de medir, y nunca entran en `golden.json`.

### Umbrales, fijados antes de medir

La base es el paso 0; entre paréntesis, la referencia de hoy. «Sin bajar» quiere decir que el valor del brazo es mayor o igual al de la base. U1 a U6 deciden y se leen en `similarity`, que no tiene piso ni corte. U7 a U10 dependen de constantes calibradas para el prefijo de hoy: se anotan y no deciden.

| # | Métrica | Bloque | Base | Umbral |
|---|---|---|---|---|
| U1 | español→inglés, R@5, los 19 casos del ancla | `similarity` | (6/19 = 0,32) | McNemar exacto p < 0,05, con más ganados que perdidos. Con 0 perdidos hacen falta 6 ganados (p = 0,031, ≥ 0,63); con 1, 8 (p = 0,039); con 2, 10 (p = 0,039) |
| U2 | español→inglés, R@10 | `similarity` | (0,58) | sin bajar |
| U3 | español→español, R@5 y R@10 | `similarity` | (0,90 / 0,95) | sin bajar |
| U4 | inglés→inglés, R@5 y R@10 | `similarity` | (0,86 / 0,86) | sin bajar |
| U5 | Global: R@1, R@5, R@10, MRR y nDCG@10 | `similarity` | paso 0 | ninguna baja |
| U6 | `golden-cross.json`: R@5 de español→inglés y de inglés→español | `similarity` | paso 0 | sin bajar |
| U7 | Ancla global: R@1, R@5, R@10, MRR y nDCG@10 | `retrieval` | (0,4348 / 0,6522 / 0,8478 / 0,5524 / 0,6215) | se anota con su p |
| U8 | Agent-view: `search.found` y `context.found` | agent-view | (36/46 y 39/46) | se anota con su p |
| U9 | Agent-view: tokens de `search` y de `context` | agent-view | (764,0 y 1.086,4) | se anota con el intervalo bootstrap del cambio medio |
| U10 | Curva de contradicciones (28 pares) | — | paso 0 | se anota; la re-mide C |

Lectura de (b):

| U1 a U6 (`similarity`) | U7 a U9 (constantes de hoy) | Lectura |
|---|---|---|
| se cumplen | ninguna empeora con p < 0,05 | (b) explica la brecha, y el brazo ya mejora lo que recibe el agente sin recalibrar. Se propone C como cambio aparte, que igual vuelve a medir el piso, el ratio y la banda |
| se cumplen | alguna empeora con p < 0,05 | (b) explica la brecha, pero el piso y el ratio calibrados para el prefijo de hoy no dejan verlo. Se propone C como cambio aparte con la recalibración adentro; U7 a U9 dicen cuánto hay que mover |
| no se cumplen | U7 u U8 mejoran con p < 0,05 | Es una anomalía (la geometría no mejora y el ranking sí): se investiga antes de concluir |
| no se cumplen | el resto | (b) no explica la brecha: siguen (c) y (e), y si tampoco la explican, D (aceptarla) |

Si los dos brazos cumplen, se anotan los dos y C elige. Un umbral que falla se anota tal cual y no se mueve después de medir.

### Tests RED

Strict TDD; cada uno falla hoy:

1. Un golden sin `query_lang` parsea igual que antes; una clave mal escrita (`query_lnag`) falla.
2. Un golden sin `query_lang` mide igual, de punta a punta: los mismos `per_case` y `aggregate`, ningún `by_pair` y el resto del JSON igual byte a byte.
3. Dos casos que sólo difieren en `query_lang` caen en parejas distintas, y cada pareja se agrega sólo sobre sus casos, en `retrieval` y en `similarity`.
4. Un caso sin `query_lang` o sin idioma de fila unánime no tiene pareja, pero sigue contando en el agregado.
5. Agent-view separa los casos activos por pareja; los `known_red` y los `unanswerable` quedan afuera.
6. Un archivo de resultados sin `query_lang` ni `by_pair` se lee y se compara con `--diff` contra uno que los tiene.
7. En `--diff`, 6 casos de español→inglés que mejoran (y ninguno que empeora) dan p = 0,03125 en esa pareja, y las líneas `primary:` quedan iguales.
8. Un caso cuya pareja difiere entre los dos archivos no cuenta en ninguna y se informa aparte; una regresión dentro de una pareja nunca hace salir con 6.

### Registro del diagnóstico

Va en `openspec/changes/english-recall/verify.md`, un apartado por paso y por hipótesis: commit, comando, `--tag` y archivo de `docs/eval/results/`, sha del golden, cifras por pareja con su intervalo, filas del `--diff` con su p y el veredicto según las tablas de arriba. Lleva además un apartado para memory-protocol (22) con las cifras por pareja. Al archivar, la base por pareja pasa a `specs/eval` y al contexto de `config.yaml`, la conclusión a la fila 21 de `openspec/README.md`, y, si (b) gana, el mapa suma C, que entra por 17.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Un n chico (19, 7) y varias comparaciones (2 brazos, varias parejas, 2 bloques) dan un falso ganador | Media | Una sola métrica decide, preregistrada (U1); U2 a U6 son guardas y U7 a U10 sólo se anotan. `golden-cross.json`, escrito antes de medir, sirve de réplica (U6). Con 19 casos, la parte `test` de `by_split` (unos 6) no alcanza para una prueba, así que no reemplaza a esa réplica |
| Los casos nuevos se escriben sabiendo qué falla | Media | Una consulta por fila, con una regla fija, escrita antes de cualquier brazo y fijada por sha |
| Las etiquetas son lecturas a mano | Baja | Hay una regla escrita; las parejas del paso 0 se contrastan con las de la exploración |
| La base de agent-view se movió sin que nadie lo midiera | Media | El paso 0 la vuelve a medir y, si se movió, la reemplaza con el movimiento anotado |
| Un brazo compila un prefijo equivocado | Baja | `similarity.query_prefix`/`passage_prefix` de cada archivo, más un `--tag` por brazo. El worktree no se mergea y se borra al terminar |
| `golden.json` no tiene historia en git | Media | Antes de etiquetar se copia a `docs/eval/golden.pre-english-recall.json` |

## Migración y vuelta atrás

No toca `memory.db` ni ningún dato guardado: `rag-eval` siembra un store temporal. Los archivos de resultados viejos se siguen leyendo, porque cada campo nuevo es opcional al leer y cada bloque nuevo falta si no hay parejas. Un `rag-eval` anterior no lee un golden con `query_lang` (`deny_unknown_fields`, `golden.rs:31`); eso es aceptable dentro del repo. Para volver atrás, se revierten los commits y se restaura `golden.pre-english-recall.json`.

## Éxito

- La suite pasa, y el test 2 prueba que un golden sin `query_lang` mide igual.
- El paso 0 reproduce el ancla con 0 casos movidos en `retrieval` y `similarity`.
- `verify.md` da un veredicto para cada una de (a) a (f), con los umbrales de arriba sin mover, y dice si sigue C, D o las dos cosas a la vez.
