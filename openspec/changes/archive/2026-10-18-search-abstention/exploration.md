# search-abstention — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | eval-rigor (33), memory-rank-fusion (34) — la parte de «dejar de rellenar» (enfoque C) no depende de ninguno; la puerta calibrada (enfoque A) depende de los dos | exploración (borrador) |

**Tesis:** la búsqueda de memorias no puede decir «no hay nada relevante». Un agente que pregunta por algo que nunca se guardó recibe igual hits: `search_memory` muestra siempre al menos dos, `recall_context` llena hasta diez con lo que haya, incluidas filas con score 0, y la única puerta que existe es una constante absoluta de coseno que se movió de 0,8793 a 0,79 cuando cambió el pooling. Un agente que recibe filas sin relación las toma por contexto. Decir «nada relevante» es una salida que hoy el sistema no tiene, y medirla exige casos sin respuesta que el golden set no tiene.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `search_memory` muestra siempre al menos `MIN_SHOWN_HITS = 2`, sea cual sea su score | **Cierta.** `MIN_SHOWN_HITS = 2` (`crates/clawg-cli/src/relevance.rs:22`) y `relevant_len` conserva los dos primeros sin mirar el score (`relevance.rs:25-34`). Hay un test que lo fija con `[0.10, 0.02, 0.01]` (`relevance.rs:60-62`). Y el motivo está escrito: una consulta del golden set cuya respuesta puntuó 0,04 bajo un mejor de 0,10 (`relevance.rs:56-58`). El mínimo existe para no perder respuestas reales. |
| El corte relativo no es una puerta | **Cierta.** `relevant_len` compara con `ratio × best` (`relevance.rs:32`): es relativo al mejor hit, así que si el mejor es basura, lo que esté a 0,35 de la basura también se muestra. Se aplica dos veces con la misma función: en el daemon cuando la petición trae `min_relative_score` (`daemon/api.rs:799-801`; el puente siempre lo manda, `client.rs:383`) y de nuevo al dar formato (`mcp/tools.rs:60-64`). Un resultado vacío sólo ocurre si la búsqueda no trajo nada: «no memories found» (`tools.rs:82-86`). |
| `recall_context` no tiene corte relativo y las filas de score 0 pueden llenar lugares | **Cierta.** `build_memory_prompt` pide `K_FINAL = 10` más las filas del workspace (`prompt_builder.rs:42,143`), descarta las ya entregadas y corta a `K_FINAL` (`prompt_builder.rs:178-179`) sin mirar el score. Un test lo fija a propósito: «a zero-score hit still fills the prompt» (`prompt_builder.rs:1865-1898`). Lo único que el diseño evita es **contarlas como acceso** (`access.rs:144-157`: «a row at 0 is below the similarity floor and is there to fill a slot»). |
| El piso de coseno sólo pone a cero el componente semántico y el bono de identificador se suma después | **Cierta.** `rescale_cosine` es `((cos − floor)/(1 − floor)).clamp(0,1)` (`score.rs:215-217`); en `score` el bono se suma tras el reescalado (`score.rs:264-277`), «so a match below the floor still scores». El score final multiplica por decaimiento, importancia y estado (`score.rs:279-300`). |
| El piso depende del pooling y del prefijo, y se movió | **Cierta.** El comentario lo dice (`score.rs:161-181`): 0,79 es el percentil 5 de los cosenos consulta-pasaje de la documentación del repo bajo mean, y el 0,8793 de CLS «would now put 91% of the top-10 hits under the floor» (`095f270`). También se movieron, por la misma causa, el umbral de contradicción (de (0,30–0,35] a 0,42, `contradiction.rs:15-25`) y el corte relativo (de 0,6 a 0,35 porque la mediana del mejor score pasó de 0,335 a 0,19, `cb446fa`, `relevance.rs:8-19`). |
| Con este modelo, los cosenos entre textos sin relación ya son altos, así que un valor absoluto es frágil | **Cierta en lo medido.** El propio código lo documenta: la distribución del corpus vivo cae en una banda de 0,86–0,95 bajo CLS (`score.rs:209-214`), y el piso es el **percentil 5** de una distribución de consulta-pasaje (`score.rs:161-164`): por construcción deja pasar el 95 % de esos pares. No es una puerta de relevancia; recorta la cola inferior. No hay un archivo de resultados con scores de consultas sin respuesta, porque no existen esos casos. |
| Hay distribuciones de score por consulta en `docs/eval/results` | **Cierta, con límites.** Los archivos de `--agent-view` traen, por caso, las líneas mostradas con su score a dos decimales (`agent_view.per_case[].search.lines[].score`). Son líneas **después** del corte y redondeadas, y las 46 consultas **tienen** respuesta. Sirven para medir el costo de una puerta, no su beneficio. |

## Problema (verificado contra el código)

### Cómo llega hoy una consulta sin respuesta al agente

- **`search_memory`.** El daemon busca hasta `limit` (10 por defecto, `api.rs:64-66`), corta con `relevant_len` y el puente vuelve a cortar con la misma función (`api.rs:799-801`, `tools.rs:60-64`). Siempre quedan al menos 2 hits mientras haya 2 en el almacén, con su score impreso (`tools.rs:93-101`) pero sin su escala.
- **`recall_context`.** Los hits van a `RECENT CONTEXT` o `LONG-TERM KNOWLEDGE` (`prompt_builder.rs:211-228`), hasta 10, completos. Las filas de score 0 entran si no hay otras.
- **Los conflictos se piden sobre lo mostrado.** `shown` se arma con los hits (`prompt_builder.rs:211-217`) y `fetch_active_conflicts` consulta pares con un lado en `shown` (`prompt_builder.rs:230`). Una fila de relleno puede traer a la salida un par disputado que no tiene que ver con la pregunta.
- **El agente no puede distinguir.** No hay ninguna frase que diga «nada de esto responde a la consulta». El único texto de vacío es «no memories found», que ocurre sólo si no hay filas en el alcance (`tools.rs:82-86`).

### La puerta que existe es una constante y se movió tres veces

| Constante | Valor anterior | Valor actual | Por qué se movió |
|---|---|---|---|
| Piso de coseno | 0,8793 | 0,79 | El pooling pasó de CLS a mean (`095f270`) |
| Corte relativo | 0,6 | 0,35 | Idem; con 0,6, `search_memory` caía de 41/46 a 30/46 (`verify.md:165`) |
| Umbral de contradicción | 0,35 | 0,42 | Idem (`contradiction.rs:15-25`) |

Las tres se movieron por el mismo cambio de pooling. La regla de `score.rs:176-181` dice que si el prefijo cambia hay que re-medir el piso, y no hay forma de detectarlo desde el código. Cualquier puerta absoluta hereda esa fragilidad, y cualquier cambio de la escala del score (memory-rank-fusion, 34) la mueve otra vez.

### Qué costaría una puerta absoluta, con los datos que hay

Del archivo `docs/eval/results/2026-10-15-real-project-indexing-relcut-agent-view.json` (46 consultas, todas con respuesta; scores de las líneas mostradas de `search_memory`, a dos decimales):

| Medida | Valor |
|---|---|
| Score del primer hit | de 0,03 a 0,41, mediana 0,19 |
| Primer hit, consultas donde la respuesta se ve (36) | de 0,06 a 0,41, mediana 0,23 |
| Primer hit, consultas donde no se ve (10) | de 0,03 a 0,25 |
| Consultas con el primer hit bajo 0,10 | 3, y en 1 de ellas se ve la respuesta |
| Consultas con el primer hit bajo 0,15 | 10, y en 5 de ellas se ve la respuesta |
| Consultas que terminaron con sólo 2 líneas (el mínimo) | 4; en 3 de ellas se ve la respuesta (con primeros hits de 0,06, 0,13 y 0,10) |
| Líneas con score 0 en las 345 mostradas | 1 |

Las respuestas de las consultas con primer hit bajo son reales: una puerta en 0,15 sobre el score final haría que 5 de 46 consultas con respuesta dejaran de mostrarla, y una en 0,10, 1 de 46. Cada caso son 2,2 puntos, y con 46 casos una diferencia tan chica no se distingue del ruido (eval-rigor, 33); aun así, es lo que cuesta. Y las consultas sin respuesta, que serían las que justifican la puerta, no están en el set: **el beneficio es hoy inmedible.**

El score final mezcla además tres factores que no son relevancia: decaimiento por antigüedad, importancia (hasta 1,8× de rango, `score.rs:211-214`) y estado (`score.rs:279-300`). Una memoria muy relevante, vieja y poco importante puntúa menos que una irrelevante de importancia alta y reciente. Una puerta sobre el score final decide por esa mezcla; una puerta sobre la relevancia (la distancia de la fila, `RankedHit.distance`, `search.rs:84`) la evita pero queda ligada al embedding.

### Por qué un criterio relativo por consulta tampoco alcanza con los datos actuales

Con las mismas líneas, el hueco entre el primero y el segundo hit es mediana 0,03; en 17 de 46 consultas es menor a 0,02 y en 13 de ellas la respuesta se ve. Es normal que varios candidatos compitan cuando la pregunta tiene respuesta. Un criterio basado en el hueco o en el z-score de la propia distribución de la consulta rechazaría muchas consultas con respuesta. Se mide sobre líneas posteriores al corte y redondeadas, así que es una cota gruesa; la comparación definitiva necesita los scores crudos.

## Lo que ya existe y se reutiliza

- `relevant_len` (`relevance.rs:25-34`): función pura, con tests; el lugar natural de una puerta nueva. Su cabecera explica por qué vive fuera del core: «the store's ranking, and every metric `rag-eval` reads from it, stays the full ranked list» (`relevance.rs:1-5`).
- `min_relative_score` como campo opcional y validado de la petición (`api.rs:269-302,336`): el patrón para sumar un parámetro de puerta sin romper clientes viejos.
- `RankedHit` con `score` y `distance` (`search.rs:76-92`): la entrada de cualquier criterio, sea sobre el score final o sobre la relevancia.
- La regla «lo no mostrado no se cuenta como acceso» (`access.rs:144-157`, `api.rs:796-802`): una puerta que oculta filas las excluye del registro de accesos con el mismo mecanismo.
- `rag-eval --agent-view` y sus líneas con score (`agent_view/mod.rs:70-84`): ya mide lo que recibe el agente.
- `ready_embedder` y los textos de estado (`api.rs:756-763`, `server.rs:415-431`): ya distinguen «el modelo no está listo»; falta distinguir «nada relevante».
- `format_context` («no context yet», `tools.rs:285-291`): el lugar donde `recall_context` dice que no hay nada.

## Enfoques posibles

### A. Puerta calibrada en casos sin respuesta (condicionada a 33 y 34)

Una función pura `abstain(scores_o_distancias, puerta) -> bool` (o el número de hits que pasan), con la puerta calibrada sobre las consultas sin respuesta de la partición de calibración que provee eval-rigor, y reportada sobre la de prueba. Se expone como parámetro opcional de la petición (patrón de `min_relative_score`) y se aplica en `search_memory`, `recall_context` y el conteo de conflictos.

- Variante A1: sobre el score final. Simple, pero mezcla decaimiento e importancia.
- Variante A2: sobre la relevancia (`distance` o el coseno reescalado antes de multiplicar). Independiente de la antigüedad y la importancia, pero atada al embedding y obligada a re-medirse con él.
- A favor: es la única opción que puede producir un «nada relevante» de verdad.
- En contra: sin casos `unanswerable` (33) no se puede calibrar; hasta que 34 fije la escala del score cualquier valor se invalida (si 34 reemplaza el bono de identificador y el corte relativo por fusión de rangos, cambia el significado de «score»). Si la puerta toca el mínimo de 2 hits, se pierden respuestas reales (tabla de arriba).

### B. Criterio relativo por consulta, sin constante absoluta

Hueco entre el primero y el k-ésimo, o z-score del primero contra la distribución de la propia consulta.

- A favor: no depende de la escala absoluta.
- En contra: con los datos del repo rechazaría consultas con respuesta (17 de 46 con hueco menor a 0,02). Si 34 pasa a fusión de rangos, el score ya no tiene una distribución sobre la que calcular un hueco. **Descartado por ahora; se reabre con 34 y con scores crudos.**

### C. Dejar de rellenar y decirlo explícito (recomendado como primera parte; no depende de 33 ni de 34)

Sin ninguna constante nueva:

- `recall_context` no llena lugares con filas de score 0: se descartan de `hits` antes de cortar a `K_FINAL`. Hoy afecta a 1 de 345 líneas del eval (cero cambio esperado en `found`).
- `MIN_SHOWN_HITS` se aplica sólo a hits con score mayor que 0. El piso de coseno ya decide ese cero; no se agrega un valor.
- Cuando no queda nada, las dos herramientas dicen una frase explícita («no relevant memories for this query») distinta de «no memories found» (almacén vacío en el alcance), de «memories are not searchable yet» (cambio 23) y de «no context yet» (`tools.rs:287`).
- Se revierte el test que fija el comportamiento actual (`prompt_builder.rs:1865-1898`): ahora afirma lo contrario.

- A favor: pequeño, puro, sin constantes; quita un relleno que el propio código ya reconocía como «only fills a slot».
- En contra, y hay que decirlo: casi no cambia nada hoy. El piso es el percentil 5, así que casi todo texto sin relación tiene score mayor que 0 y pasa. **C por sí solo no produce abstención; produce la frase y quita el relleno literal.** La abstención real es A.

### D. Dejar que el agente lo decida

El agente ya ve el `score=` de cada línea (`tools.rs:93-101`).

- A favor: cero código.
- En contra: recibe el número sin la escala, que se mueve (mediana del mejor score 0,19 hoy y 0,335 antes) y varía por idioma; una respuesta puede tener 0,04 (`relevance.rs:56-58`). Descartado.

**Recomendación:** C primero (cambio chico y sin constantes) y A como segunda parte, sólo cuando 33 aporte los casos y 34 fije la escala. B queda descartado hasta tener scores crudos.

## Por qué se mide después de memory-rank-fusion y con los casos de eval-rigor

- **34 cambia la escala.** El score final de hoy es `relevancia × decaimiento × importancia × estado`, con el bono de identificador sumado a la relevancia (`score.rs:277`): una fila con coincidencia exacta tiene `relevancia ≥ 1,0` y una sólo semántica cae en `[0, 1]`. Si 34 reemplaza el bono fijo y el corte relativo por fusión de rangos, el score deja de ser un coseno reescalado y toda puerta calibrada antes queda sin sentido. Calibrar antes sería repetir lo que le pasó al piso.
- **33 aporta lo que falta para medir.** Sin casos sin respuesta no hay beneficio medible, y sin la partición calibración y prueba la puerta se elegiría y se informaría con los mismos datos. Con 46 casos, además, un caso son 2,2 puntos: el costo de la puerta (consultas con respuesta que dejan de verse) se mide con la prueba pareada de 33, no a ojo.

## Interacciones

- **eval-rigor (33).** Provee los casos `unanswerable`, la partición, el intervalo de Wilson y el diff pareado de agent-view. Sin él, este cambio sólo puede entregar C.
- **memory-rank-fusion (34).** Ver arriba. Si 34 quita el corte relativo, `relevant_len` y `MIN_SHOWN_HITS` cambian de dueño y esta exploración se reescribe sobre el nuevo.
- **context-budget (23).** Menos filas sin relación dentro de un presupuesto fijo; y son tres estados distintos que deben tener frases distintas: el embedder no está listo (23), nada relevante (35) y almacén vacío. Conviene que compartan el mismo lugar del texto.
- **agent-lifecycle-hooks (25).** Un contexto inyectado al iniciar la sesión no tiene una pregunta concreta del usuario y es donde el relleno cuesta más: hasta 10 filas completas en cada sesión (`prompt_builder.rs:179`, hasta 480 tokens por fila). Es una inferencia sobre el uso planeado, no una medición. Lo que 25 inyecte hereda lo que 35 decida.
- **conflict-candidates (24).** Los conflictos se piden sobre lo mostrado (`prompt_builder.rs:211-230`); menos relleno es menos pares ajenos a la pregunta.
- **english-recall (21).** El subconjunto en inglés tiene menos recall (R@5 de 0,68 a 0,45 con mean, `verify.md:129`). Una puerta calibrada sobre el set mezclado puede abstenerse de más en inglés; hay que reportarla por idioma (`by_lang`).
- **recoverable-delete (18), topic-revisions (27).** Sin relación directa.

## Alcance candidato

1. Función pura de puerta junto a `relevant_len`, con su tabla de casos (parte C: `score > 0`; parte A: la puerta calibrada).
2. `recall_context` descarta las filas de score 0 antes de `K_FINAL` y dice «no relevant memories» cuando no queda nada; `search_memory` aplica `MIN_SHOWN_HITS` sólo sobre hits con score mayor que 0.
3. La frase de abstención en las dos herramientas, distinta de las otras tres de vacío o de estado.
4. Si se hace A: parámetro opcional de la petición (`/v1/search`, `/v1/context`), validado como `min_relative_score`; el puente y el daemon usan el mismo valor; la constante vive en un lugar, no dos.
5. Mover (o pasar como argumento) la función al lugar donde `prompt_builder` pueda usarla: `relevance.rs` está en `clawg-cli` y `build_memory_prompt` en `clawg-core`, que no puede depender de aquél.
6. Actualizar la descripción de las herramientas y `CLAUDE.md` del repo (nueva frase, parámetro nuevo), y `specs/retrieval`.
7. Reportar `rag-eval --agent-view` antes y después, con el diff pareado de 33.

### No-alcance

- Cambiar el piso de coseno, `RELEVANCE_RATIO` o el ranking: lo hace memory-rank-fusion (34).
- La búsqueda en documentos (`search_project`).
- Que el servidor decida por el agente qué hacer con una consulta sin respuesta (guardar, preguntar): sólo informa.
- Generar los casos sin respuesta y la partición: es de 33.
- Un modelo, un reranker o un clasificador aparte.

## Decisiones abiertas

Tomadas en la [propuesta](proposal.md#decisiones): sólo C, sin constante nueva; la frase de abstención sin sugerencia; la puerta calibrada (A) queda para un cambio aparte, y con ella las preguntas sobre la puerta.

- [x] **¿Se entrega C solo y A después, o se espera a 33 y 34 para todo?** C ahora.
- [ ] **Sobre qué se pone la puerta de A:** el score final (A1) o la relevancia antes de decaimiento e importancia (A2).
- [ ] **Cómo se calibra:** valor fijo con partición calibración y prueba, o un porcentil de las consultas sin respuesta; y qué margen se acepta respecto del mínimo observado.
- [ ] **Pre-compromiso de aceptación.** Propuesta a discutir: abstención correcta de al menos 16 de 20 sin respuesta (cota inferior de Wilson publicada) y `found` de `search_memory` con respuesta de al menos 34/46 (hoy 36/46, con dos casos de tolerancia, no distinguibles del ruido según 33).
- [ ] **¿La puerta puede bajar el mínimo de 2 hits?** Cada respuesta perdida cuesta 2,2 puntos; la tabla muestra 3 de 4 consultas con sólo 2 líneas que tienen la respuesta.
- [x] **Texto exacto de la frase de abstención** (inglés, como el resto de las herramientas) y si lleva sugerencia («guardá lo que aprendas»).
- [ ] **Dónde se mide:** en el ranking del core (scores crudos, para calibrar) o en agent-view (lo que ve el agente, para aceptar). Se inclina a las dos, con cada una en su papel.
- [ ] **Dónde vive la función:** moverla a `clawg-core` o pasar el valor como argumento a `build_memory_prompt`.
- [x] **¿Se aplica también a las filas del workspace?** Hoy el workspace se entrega entero y no depende de la consulta; este cambio no lo toca.

## Cómo se mide

### Compuerta previa (antes de la propuesta de A)

Depende de 33. Con los casos `unanswerable` (mínimo propuesto 20, con 0 errores de 20 la cota de Wilson es 0,161) y las consultas con respuesta del set:

- Distribución del primer score y del score de la relación (distancia) en consultas con y sin respuesta, por idioma, con los scores crudos de la búsqueda, no los redondeados de agent-view.
- Curva de la puerta: abstención correcta en la calibración y respuestas perdidas, y el valor elegido se reporta sobre la partición de prueba.

### Tests RED primero

1. La función pura: `relevant_len` con `[0.10, 0.0, 0.0]` y el nuevo comportamiento devuelve 1 (hoy 2); con `[0.0, 0.0]` devuelve 0.
2. `recall_context` sobre un almacén con filas todas de score 0: hoy las incluye (`prompt_builder.rs:1898`); con el cambio no, y dice la frase de abstención.
3. Una fila de score 0 no entra en `shown` y por tanto no trae conflictos ajenos.
4. `search_memory` sin nada relevante devuelve la frase de abstención y no «no memories found»; con almacén vacío devuelve «no memories found».
5. Con el embedder en `loading` o `failed` no se dice «nada relevante» (cambio 23 domina).
6. Un ratio y una puerta ausentes en la petición (cliente viejo) se comportan como hoy.
7. Con un hit de score mayor que 0, el mínimo de 2 hits se conserva (la consulta con respuesta en 0,04 bajo 0,10 de `relevance.rs:56-58` sigue viéndose).
8. Parte A: la puerta, con scores sintéticos, abstiene sobre un caso `unanswerable` y no sobre un caso con respuesta de score bajo.

### `rag-eval`

**Obligatorio:** se toca lo que recibe el agente. Correr `rag-eval --real --agent-view` con el diff pareado de agent-view de 33 antes y después, y reportar `found` y tokens de `search_memory` y `recall_context` (hoy 36/46 a 764,0 y 39/46 a 1.086,4; `docs/eval/results/2026-10-15-real-project-indexing-relcut-agent-view.json`). Resultado esperado de C: `found` sin cambios y tokens sin aumento. Para A: abstención correcta sobre los casos sin respuesta y sin caída de `found` más allá del pre-compromiso.

## Riesgos

- **Perder una respuesta real.** Tres de las cuatro consultas que terminaron con sólo 2 líneas tienen la respuesta; una puerta que corte ahí las pierde. Es el costo principal y lo mide 33.
- **Una puerta que se invalida sola.** Si 34 cambia la escala o el modelo cambia, el valor calibrado deja de valer. Por eso A va después de 34 y su valor se documenta con la medición, como el piso.
- **Falsa tranquilidad de C.** Quita el relleno literal y escribe la frase, pero casi no abstiene (1 de 345 líneas hoy). Hay que decirlo para que nadie crea que el problema está resuelto.
- **Mezclar estados.** «Nada relevante», «modelo no listo» y «almacén vacío» tienen consecuencias distintas para el agente; si comparten texto, concluye lo equivocado.
- **Sesgo por idioma.** El subconjunto en inglés puntúa distinto; una puerta única puede abstenerse de más en ese idioma.
- **Cambio de contrato en las herramientas.** Una frase nueva y un parámetro nuevo cambian lo que ve el agente en cada sesión; quality-guards (32) fija ese contrato y el diff tiene que ser a propósito.
- **Estado del repo:** el test de `prompt_builder.rs:1865-1898` afirma el comportamiento actual de las filas de score 0 y tiene que cambiar con la decisión; `CLAUDE.md` describe `recall_context` y `search_memory`.
