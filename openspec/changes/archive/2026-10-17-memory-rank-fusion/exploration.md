# memory-rank-fusion — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | eval-rigor (33, planeado: la prueba pareada y los intervalos; las mediciones previas a la compuerta no lo necesitan). Conviene antes de memory-protocol (22) | exploración (borrador) |

**Tesis:** en `search_memory` el bono fijo por identificador y el corte relativo se combinan mal. Una fila con el bono puntúa entre 1 y 2 veces su decaimiento por importancia, y el corte es el 35 % de esa cifra: una fila sin el bono necesita un coseno que ni una sola pareja consulta-fila del golden set alcanza. Resultado medido: las 30 consultas con identificador del set sustituto muestran exactamente 2 filas (el mínimo del corte), contra 9 a 10 en las demás. Además BM25 sólo propone candidatos y nunca ordena, mientras la búsqueda de docs del propio proyecto ya funde sus canales por rango. El primer paso medible es chico (que el corte mire sólo la parte semántica); la fusión por rango completa es un paso aparte, y sólo después.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `score()` es `relevancia × max(piso del tier, e^(−λ·días)) × (0,8333 + imp/60) × estado`, con `relevancia = rescale(cos) + bono` | **Cierta.** `score.rs:264-306`. El reescalado es `clamp((cos − 0,79) / 0,21, 0, 1)` (`score.rs:215-217`, piso `0,79` en `:182`). El bono `lexical_bonus = 1.0` (`:186`) se suma **después** del reescalado y no se acota (`:277`). Pisos de tier `0,5 / 0,2 / 0,05` (`:123`). |
| Sólo reciben el bono las filas que tienen **todos** los identificadores de la consulta | **Cierta.** `is_identifier` (`search.rs:712-717`: letra más dígito, guion bajo o joroba), a lo sumo 8 tokens (`:691`, `:699-710`), `identifier_matches` (`:754-804`). Si más filas que el presupuesto los tienen, nadie lo recibe (`:800-802`); el presupuesto es el léxico, 20 (`SearchScope::all_default`, `:37-46`), con tope de 512 (`:638-645`). |
| El corte relativo corre sobre el score **total** | **Cierta, y corre dos veces.** `RELEVANCE_RATIO = 0,35` y `MIN_SHOWN_HITS = 2` (`crates/clawg-cli/src/relevance.rs:19`, `:22`); `relevant_len` deja las filas con `s >= ratio × mejor` (`:25-34`). El daemon lo aplica si el pedido trae `min_relative_score` (`daemon/api.rs:799-802`), **antes** de `record_hits` (`:803`); el puente lo aplica siempre (`mcp/tools.rs:60-69`) y su cliente manda la constante (`client.rs:383`). |
| `recall_context` no tiene corte | **Cierta.** Busca con el mismo `search_all_memories` y `SearchScope::all_default()` (`prompt_builder.rs:151-157`) y sólo trunca a `K_FINAL = 10` (`:42`, `:179`): recibe el bono y el orden, pero no pierde filas por el corte. |
| «Casi toda fila sin el bono desaparece de `search_memory`» | **Cierta, con un matiz.** `MIN_SHOWN_HITS` hace que la mejor fila **sin** bono se muestre siempre (es la segunda). Las demás sólo si pasan el corte. Medido abajo. |
| BM25 sólo propone y no ordena | **Cierta.** `ORDER BY bm25(...) LIMIT` decide *cuáles* 20 entran (`search.rs:611-618`); los que no vinieron del denso se puntúan por coseno con la misma fórmula (`:890-900`, `:951-1029`). Por debajo del piso, una coincidencia léxica de palabras comunes puntúa 0. El orden final es un único `sort_by(hit_cmp)` por score (`:135-141`, `:1176`), luego `collapse_saves` (`:104-121`) y `truncate(k_final)` (`:1187`). |
| La búsqueda de docs ya funde por rango | **Cierta.** `RRF_K = 60` (`project_docs/mod.rs:42`), `fuse` (`:243-254`), 50 candidatos por canal (`:37`), `search` (`:258-305`). Pero `fuse` recibe **dos** listas fijas y no tiene piso, recencia ni importancia. |
| El detector de contradicciones no usa el canal léxico | **Cierta.** `lexical: None` a propósito (`contradiction.rs:108-130`): este cambio no lo toca. |
| «El arnés no podía ver el canal léxico, por eso se eligió el bono» | **Cierta entonces, no del todo hoy.** Con el set de 46 casos sigue sin verse: las cuatro corridas `--sweep-lexical` dan lo mismo que el ancla (`archive/2026-10-15-project-indexing/verify.md:161`). Pero eval-golden-expansion (fila 5) dejó el set de la semilla 7 con un subconjunto `lexical` de 30 casos (`rag-eval/golden.rs:85-93`), y ése sí lo ve. |
| «Los umbrales con que se aprobó el bono cubren el corte» | **Falsa.** `lexical-fusion` midió el ranking del núcleo y el agent-view sobre el ancla natural, que no tiene consultas con identificador (`archive/2026-10-12-lexical-fusion/verify.md:148-152`, `:156-167`). Ningún umbral miraba cuántas filas llegan al agente. |
| El agent-view cuenta una consulta como acertada si el agente ve lo que buscaba | **Sólo a medias.** `found` es `!found_refs.is_empty()` (`agent_view/runner.rs:174`): basta **una** fila esperada. Un caso que necesita dos filas cuenta como acierto aunque una se haya cortado. Hay que medir con `found_refs`. |

## Problema (verificado contra el código)

### Cómo se combinan

Sea una fila con bono, de coseno reescalado `c_b` y factor `F_b = decaimiento × importancia`: su score es `(1 + c_b) × F_b`. El corte es `0,35 × (1 + c_b) × F_b`. Una fila sin bono, de coseno reescalado `c_n` y factor `F_n`, sobrevive si `c_n × F_n ≥ 0,35 × (1 + c_b) × F_b`.

| Caso | Coseno que necesita la fila sin bono |
|---|---|
| Fila con bono por debajo del piso (`c_b = 0`) y mismo factor | `c_n ≥ 0,35`, o sea `cos ≥ 0,79 + 0,35 × 0,21 = 0,8635` |
| Fila con bono en `c_b = 0,6` (score cercano a 1,6 con `F_b = 1`) | `c_n ≥ 0,56`, o sea `cos ≥ 0,9076` |
| Igual, con la fila sin bono en importancia 5 (factor `0,8333 + 5/60 = 0,9167`) | `c_n ≥ 0,611`, o sea `cos ≥ 0,918` |

Los cosenos reales de este proyecto no llegan ahí. En la corrida del ancla con pooling mean (`docs/eval/results/2026-10-15-real-project-indexing-tip-anchor.json`, bloque `similarity`, 46 consultas por 48 filas) el percentil 95 es 0,830 y el máximo 0,8847. Ninguna pareja del set alcanza 0,9076, y por encima de 0,8635 queda menos del 5 % de las parejas (el percentil 95 está por debajo). El set es chico, y una base de meses tendrá paráfrasis más cercanas; pero la brecha no es de márgenes: el bono vale 1,0 y la escala útil del coseno reescalado es de 0 a 0,45.

El factor también cuenta. El bono domina cuando la fila con bono es reciente o de tier 1 (`F_b` cerca de 1). Una fila de tier 3 vieja (piso `0,05`) con bono puntúa poco y no fija el corte.

### Lo medido

Agent-view sobre el set de la semilla 7 (`docs/eval/results/2026-10-15-real-long-save-context-tip-seed7-agent-view.json`, 1.040 filas, 124 casos; piso 0,8793, es decir **antes** del pooling mean):

| Subconjunto | n | Filas mostradas (media) | Mejor score (mediana) | Segundo (mediana) |
|---|---|---|---|---|
| `lexical` | 30 | **2,00** (30 de 30 casos con exactamente 2) | 1,40 | 0,54 |
| `natural` | 46 | 10,00 | 0,41 | 0,39 |
| `multi_ref` | 12 | 9,67 | 0,575 | 0,525 |
| `high_importance` | 12 | 8,67 | 0,55 | 0,54 |
| `low_importance` | 12 | 9,25 | 0,54 | 0,51 |

Las consultas con identificador quedan en el mínimo del corte, y la segunda fila sólo está por `MIN_SHOWN_HITS`. Con el pooling mean la brecha crece, no se achica: la mediana del mejor score de las filas sin bono baja de 0,335 a 0,19 (`archive/2026-10-15-project-indexing/verify.md:165`) mientras el score de una fila con bono casi no depende de la escala (es `1 + c_b`). Esa corrida con mean no existe para el set de la semilla 7: es la primera medición de la compuerta.

Dos efectos que acompañan: el corte va antes de `record_hits` (`api.rs:796-803`), así que una fila oculta no suma accesos y no sube de tier; y el hit rate del arnés no ve nada de esto, porque en esos 30 casos la fila esperada es la que tiene el identificador (hit rate 1,0).

## Por qué se eligió el bono entonces, y qué cambió

- **Entonces.** El arnés no veía el canal léxico: todas las corridas `lex*` daban métricas idénticas y 0 de 112 casos cambiaban de ranking (`archive/2026-10-12-lexical-fusion/proposal.md:15-18`; fila «Migrar a RRF» de `openspec/README.md`). `design.md:7-12` descartó el RRF porque reemplaza `cos'` por rangos en todas las consultas (cambia las 46 naturales sin que ninguna lo necesite), pierde la magnitud con la que se calibraron el piso y el rango de importancia, y con 0 a 3 resultados léxicos cada uno se lleva el `1/61` completo. El bono midió `lexical` R@5 de 0,4333 a 1,0 y MRR a 1,0, 0 casos naturales peores y el ancla sin cambios (`verify.md:156-167`).
- **Desde entonces.** (1) Existe el set de la semilla 7 y un subconjunto léxico, así que una fusión por rango se puede medir. (2) El pooling mean movió la escala: la mediana del mejor score pasó de 0,335 a 0,19 y `RELEVANCE_RATIO` se recalibró de 0,6 a 0,35 con consultas naturales solamente (`project-indexing/verify.md:163-184`). (3) memory-protocol (22) va a pedirle a los agentes que escriban identificadores exactos en los saves; cada identificador raro es una oportunidad de bono. (4) El proyecto ya usa `fuse` por rango en `search_project`.
- **Una advertencia que viene del propio repo.** En la búsqueda de docs, con el canal léxico sin filtrar palabras vacías, la fusión dio Hit@3 0,20 contra 0,30 de sólo vectores con el mismo pooling, y 0,40 al filtrar las palabras de menos de 4 letras (`project-indexing/verify.md:54-66`, 10 preguntas). El canal léxico de las memorias tampoco filtra nada (`search.rs:563-580`, tokenizer `unicode61` en `V8__memory_fts.sql:43`). Una fusión por rango sin esa compuerta repite el problema. La filtración de palabras vacías es la fila 20 (docs-search-quality).

## Lo que ya existe y se reutiliza

- `relevant_len` es pura y está testeada (`relevance.rs:25-68`); el corte tiene un test de API (`crates/clawg-cli/tests/api.rs:796-818`).
- `identifier_matches` ya decide *qué* filas son coincidencia exacta; marcar esa condición en el resultado no cuesta una consulta nueva (`RawHit.lexical_match`, `score.rs:28-30`, ya viaja por `compute_score`).
- `Hit` del cliente ya tolera campos que un daemon viejo no manda (`#[serde(default)]`, `client.rs:151-158`): el patrón de `agent_id` y `piece` sirve para un campo nuevo.
- `rag-eval --real --agent-view` corre el daemon real en el mismo proceso y formatea con `format_search` (`agent_view/runner.rs:10-12`, `:135`): ve el corte. El ancla llama al núcleo y no lo ve (`relevance.rs:1-6`).
- `fuse` y `RRF_K` para la opción B (`project_docs/mod.rs:42`, `:243-254`).
- `--sweep-cos-floor`, `--sweep-lexical`, `--sweep-imp`, `--audit`, `--diff` (`rag-eval/cli.rs:304-334`). `golden-mine synth` arma el set sustituto.

## Enfoques posibles

### A. El corte mira sólo la parte semántica (recomendado como primer paso)

El mejor score de referencia se calcula **sin** las filas con bono. Las filas con bono se muestran siempre (están acotadas: a lo sumo el presupuesto de 20, y el límite del pedido); las demás se cortan contra la mejor fila sin bono. En la práctica la lista se parece a la de una consulta sin identificador, con las coincidencias exactas arriba.

- Forma mínima: la fila de resultado lleva una marca booleana (por ejemplo `exact`, a nombrar), `relevant_len` pasa a recibir pares `(score, marca)`, y los dos puntos de corte (`api.rs:799-802`, `tools.rs:60-64`) la usan. La marca es opcional y por defecto falsa: un daemon viejo no la manda y el puente cae en el comportamiento actual; un puente viejo la ignora.
- A favor: no cambia el orden ni los scores, así que el ancla y el ranking del núcleo quedan **idénticos por construcción** (`--diff` vacío). No depende de la escala del coseno, que english-recall (21) puede volver a mover. Cambia sólo `search_memory`.
- En contra: toca el cliente y el daemon (los dos cortan), y suma filas y tokens a las consultas con identificador. No resuelve que BM25 no ordene.
- Variante A2, restar el bono del score y cortar contra `score − bono × F`: exige conocer `F` fuera del núcleo. Se descarta salvo que A1 no alcance.

### B. Fusión por rango (RRF) sobre tres canales: denso, BM25, identificador

Una lista por canal; `fuse` generalizado a N listas; recencia, importancia y estado quedan como factores suaves que multiplican el score fundido; el piso de coseno queda como compuerta de la lista densa (la abstención es search-abstention, 35).

- A favor: BM25 pasa a ordenar; el canal de identificadores se vuelve una lista y no un salto de 1,0; reutiliza el código de docs.
- En contra, con números de este proyecto:
  - **El rango comprime el score.** Con `RRF_K = 60`, el rango 1 vale `1/61 = 0,0164` y el 30 (el pool denso, `search.rs:39`) vale `1/90 = 0,0111`: la peor fila de la lista es el 68 % de la mejor. El corte de 0,35 dejaría de cortar nada, así que B exige otra regla de recorte (ver C) en el mismo cambio.
  - **La importancia pasa a mandar.** Entre el rango 1 y el 10 hay una caída de `1/61` a `1/70`, un 13 %; la importancia mueve el score entre 0,85 y 1,0, un 15 %. El rango de 0,85 a 1,0 se angostó justamente porque la importancia pisaba a la relevancia (`score.rs:135-143`). Hay que recalibrarlo, y eso es otro factor más en el mismo cambio.
  - **La garantía del bono se vuelve estadística.** Hoy una fila con el identificador gana a un hermano más cercano con el mismo factor, por construcción (`score.rs:183-185`). Con rangos dependería de los pesos de tres listas.
  - **Hay que filtrar el canal léxico** o se repite el 0,20 de arriba.
  - **Cambia varios factores a la vez** (formato de score, corte, importancia y bono). Cada diferencia medida dejaría de tener una sola causa, y la regla de `config.yaml` pide validar todo cambio de ranking con `rag-eval --diff` contra la base.
- Se vuelve a evaluar sólo con A medido y el paso previo (palabras vacías) hecho.

### C. Reemplazar el corte relativo por un presupuesto de tokens en `search_memory`

Mostrar filas hasta llenar un tope de tokens, sin mirar la escala del score.

- A favor: costo predecible; no depende de la distribución de scores, que cambió dos veces (CLS a mean, y el ratio).
- En contra: corta por tamaño, no por relevancia: una fila larga y buena puede quedar afuera. Pisa a context-budget (23), que dejó explícitamente fuera «presupuestar `search_memory` o `search_project`». Hoy el corte casi no es el que acota el tamaño: el máximo medido es 1.206 tokens (`2026-10-15-real-project-indexing-relcut-agent-view.json`). Se retoma si B se hace.

### D. Acotar o escalar el bono

Con un bono `w` en lugar de 1,0, y mismos factores, una fila sin bono de coseno reescalado `c_n` sobrevive si `w ≤ c_n / 0,35 − c_b`. Con `c_n = 0,3` (coseno 0,853) y `c_b = 0,3`, `w ≤ 0,557`.

- A favor: un solo número. Se mide con el subconjunto `lexical` (umbral actual `R@5 ≥ 0,90`).
- En contra: sigue ciego (el corte depende de `w` y de la escala del coseno), y pierde la garantía de que el match exacto le gana a un hermano a igual factor, que es lo que justificó `w = 1,0` (`lexical-fusion/design.md:12`). No hay eje de sweep del bono en `rag-eval` (`proposal.md:48` de lexical-fusion lo dejó fuera): habría que agregarlo. Si A deja un resto, D es el ajuste fino.

**Primer paso medible: A.** B y C quedan como la pregunta siguiente, con los resultados de A y de 20 y 21 en la mano.

## Descartado

| Opción | Razón técnica |
|---|---|
| Bajar `RELEVANCE_RATIO` para que sobrevivan más filas | Ya está en el borde del plateau: 0,35 es el ratio más alto con 36/46, y 0,2 gana un caso y cuesta 130 tokens (`project-indexing/verify.md:171-182`). Con una fila en 1,6 no hay ratio razonable que sirva a las dos poblaciones. |
| Quitar `MIN_SHOWN_HITS` | Es lo que hoy deja ver la segunda fila de un caso con bono. Está justificado por consultas donde todos los scores son bajos (`relevance.rs:56-62`). |
| Cortar antes de puntuar, por tipo de consulta | Dos caminos de corte para un mismo score; A es el mismo efecto sin bifurcar. |
| Aplicar el corte sólo en el puente y quitarlo del daemon | Cambia el contrato de `POST /v1/search` y el orden de `record_hits`. Se puede unificar después; no es parte del arreglo. |

## Interacciones

- **memory-protocol (22).** Al pedir identificadores exactos en los saves, más consultas traen filas con bono. Por eso A conviene **antes** de 22 (el orden de `openspec/README.md` pone a 22 en el cuarto lugar). Si 22 aterriza antes, hay que medir `search_memory` en vista de agente antes y después.
- **english-recall (21).** Si el prefijo cambia, se mueven el piso, el corte y todos los umbrales absolutos. A es relativo y puede ir con o sin 21; B y D no. **No se mezclan 34 y un cambio de prefijo en una medición:** un solo factor por vez.
- **eval-rigor (33).** Aporta la prueba pareada y los intervalos. Sin ella la compuerta usa conteos de casos que mejoran y empeoran.
- **docs-search-quality (20).** El filtro de palabras vacías es requisito de B.
- **search-abstention (35).** Decide qué hace la búsqueda cuando nada pasa el piso. B conserva el piso como compuerta de la lista densa y no la cambia.
- **context-budget (23).** No presupuesta `search_memory`; C sí lo haría.
- **conflict-candidates (24) y agent-lifecycle-hooks (25).** `recall_context` no tiene corte, así que el inyector de sesión no cambia con A.
- **Compartir entre agentes.** `identifier_matches` ya ve las filas de otros agentes que el lector puede leer (`search.rs:744`): una coincidencia exacta de otro agente también recibe el bono. A no cambia eso; la fila se sigue marcando `by <autor>`.

## Alcance candidato

1. **Compuerta previa** (abajo): base con mean sobre la semilla 7, un subconjunto nuevo y una medida estricta.
2. Marca de coincidencia exacta en el resultado de búsqueda y en el `Hit` del cliente, opcional en el protocolo.
3. `relevant_len` pura con la marca; el daemon y el puente la usan igual.
4. `CLAUDE.md` del repo: el campo nuevo en `POST /v1/search`.
5. Reporte de `rag-eval --real --agent-view` y del ancla antes y después.

### No-alcance

- La fusión por rango (B) y el presupuesto de tokens (C).
- Cambiar el bono, `RELEVANCE_RATIO`, `MIN_SHOWN_HITS`, el piso o el rango de importancia.
- Filtrar palabras vacías del canal léxico (20).
- Cambiar el prefijo del embedder o el modelo (21, 17).
- El detector de contradicciones y `recall_context`.
- Comparar el agent-view en `rag-eval --diff` (hoy no lo hace: `openspec/README.md`, backlog); se compara a mano o lo cubre 33.

## Decisiones abiertas

Son del usuario.

- [ ] **Alcance.** ¿Sólo A, o A seguido de la evaluación de B en este mismo cambio? Recomendado: sólo A, con B como cambio aparte.
- [ ] **Qué ve el usuario de las filas con bono.** ¿Se muestran todas (hasta el presupuesto de 20) o sólo las primeras N? Con el límite del pedido (10 por defecto, `api.rs:64-66`) ya hay un tope.
- [ ] **`MIN_SHOWN_HITS` con A.** ¿Las 2 filas mínimas se cuentan entre las filas sin bono (más tokens) o entre todas (menos tokens, pero la fila con bono ocupa una)?
- [ ] **Nombre y forma del campo** (`exact`, `identifier_match`) y si se publica en la API HTTP o sólo hacia el puente.
- [ ] **Umbrales de la compuerta** (abajo): propuestos, no fijados.
- [ ] **Qué fixture vale como «fila relacionada».** Propuesta: una fila del mismo tema sin el identificador, con coseno cercano a 0,88.
- [ ] **¿Se unifica el corte en un solo lugar?** Hoy lo aplican el daemon y el puente.

## Cómo se mide

### Compuerta previa (antes de la propuesta)

1. **Base con mean.** Correr `rag-eval --real --agent-view --golden docs/eval/golden-expanded.json` con las constantes actuales (piso 0,79, ratio 0,35). Las únicas cifras de la semilla 7 son del piso 0,8793. Sin esa base no hay antes y después.
2. **Un subconjunto que mida lo oculto.** Hoy ningún caso espera a la vez la fila con el identificador y una fila relacionada sin él (`multi_ref` muestra 9,67 filas de media, así que sus consultas no activan el bono). Agregar a `golden-mine synth` y a `CaseSubset` (`golden.rs:85-93`) un subconjunto con `expected_refs` de al menos dos filas: la del identificador y otra del mismo tema con coseno cercano a 0,88. Mínimo propuesto: 30 casos.
3. **Una medida estricta.** Derivar del `found_refs` (`agent_view/mod.rs:162-207`) «todas las esperadas se muestran», porque `found` acepta una sola (`runner.rs:174`).

### Umbrales propuestos (a fijar antes de medir)

| Métrica | Base | Umbral |
|---|---|---|
| Subconjunto nuevo: todas las esperadas se muestran | (a medir; se espera cerca de 0) | `>= 0,90` |
| `lexical`: la fila con el identificador sigue primera | 30 de 30 | 30 de 30 |
| `lexical`: tokens medios | (a medir) | no más que los del subconjunto `natural` de la misma corrida |
| `natural`, `multi_ref`, `high_importance`, `low_importance`: salida del `search_memory` | — | idéntica caso por caso (sin marca no cambia nada) |
| Ancla (`--diff`) | R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524, nDCG@10 0,6215 | idéntica |
| Agent-view del ancla | `search_memory` 36/46 a 764,0 tokens; `recall_context` 39/46 a 1.086,4 | idéntico |

Si un umbral falla se anota tal cual y no se mueve después de medir.

### Tests RED primero

1. `relevant_len` con marca, pura: una fila con bono en 1,6 y filas en 0,30 / 0,25 / 0,20 / 0,05. Hoy deja 2; con A deja la del bono y las que superan `0,35 × 0,30`.
2. Sin ninguna marca, `relevant_len` da lo mismo que hoy en todos los casos de `relevance.rs:36-68`.
3. Un `Hit` sin el campo nuevo se deserializa (daemon viejo) y un daemon nuevo con un puente viejo responde igual que hoy.
4. **El fixture de la tesis:** una fila con el identificador y dos filas relacionadas sin él, con cosenos cercanos a 0,88 y 0,87, más ruido. Hoy el `search_memory` completo muestra la fila con bono y **una** relacionada (la segunda por `MIN_SHOWN_HITS`) y oculta la otra; con A muestra las dos. Hacen falta dos relacionadas: con una sola, el mínimo la muestra y el test no falla hoy. Para que el corte la oculte, la fila con bono necesita un coseno de 0,837 o más: una relacionada de coseno 0,88 (`c_n = 0,43`) sobrevive sólo si `c_b ≤ 0,22`.
5. Daemon y puente coinciden sobre la lista mostrada (hoy es un solo corte aplicado dos veces).
6. Una fila de otro agente con el identificador recibe la marca y se sigue mostrando `by <autor>`.
7. La marca no cambia `record_hits` más allá de las filas que ahora se muestran.

`rag-eval --diff`: el ancla y el ranking de la semilla 7 quedan **vacíos** (A no toca el núcleo). El agent-view se compara a mano o con lo que agregue 33: `--diff` hoy compara sólo `retrieval.per_case` y `similarity.per_case`.

Si más adelante se evalúa B: umbrales de `lexical-fusion` como piso (`lexical` R@5 `>= 0,90` y MRR `>= 0,80`), 0 casos naturales peores, el ancla sin bajar, `importance_rank_dominance` por debajo de la base de mean, y la prueba pareada de 33.

## Riesgos

- **Más tokens en las consultas con identificador.** Hoy cuestan unos 100 tokens porque muestran 2 filas; con A se parecen a una consulta natural (cientos). Es el costo de no esconder, y por eso hay un umbral de tokens.
- **Una medición que no ve lo que importa.** El hit rate no baja cuando se corta una fila relacionada. Sin el subconjunto nuevo y la medida estricta, A aprobaría o fallaría sin razón.
- **Fixtures sustitutos.** El set de la semilla 7 es sintético (hermanos con identificadores casi iguales); la proporción de consultas con identificador en el uso real no está medida.
- **Medir con otra escala que la de producción.** La base de la semilla 7 es del piso 0,8793. La compuerta 1 existe para eso.
- **Dos puntos de corte.** Si sólo se cambia uno, el otro revierte el efecto en silencio. El test 5 lo cubre.
- **Orden de cambios.** Si 21 cambia el prefijo antes, los números absolutos de arriba (0,8635, 0,9076) se mueven; el argumento relativo de A no.
- **Alcance de B.** Es la parte que más se parece a un rediseño del ranking. Mezclarla con A rompe la regla de un solo factor por cambio.
