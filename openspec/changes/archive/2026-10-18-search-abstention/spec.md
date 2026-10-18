# search-abstention — Especificación

## Definiciones

- **Fila de score 0:** un hit sin coincidencia exacta (`exact = false`) cuyo score es 0, porque su coseno queda bajo el piso (`crates/clawg-core/src/memory/score.rs:215-217`). Una fila exacta lleva el bono y nunca vale 0 mientras esté viva.
- **Frase de abstención:** `no relevant memories for this query: N searched, none close enough to show`, con `N` el número de hits que se ocultaron.

## R1. El corte no muestra filas de score 0

`shown_hits` (`crates/clawg-cli/src/relevance.rs:31`) MUST NOT mostrar una fila inexacta de score 0, y esa fila MUST NOT contar para `MIN_SHOWN_HITS`. Una fila exacta MUST seguir mostrándose siempre. Con las filas que puntuaron, el corte MUST ser el de hoy.

#### Escenario: relleno tras una respuesta débil
- GIVEN scores `[0.10, 0.0, 0.0]`, todos inexactos
- THEN se muestra sólo el primero

#### Escenario: nada puntuó
- GIVEN scores `[0.0, 0.0]`, inexactos
- THEN no se muestra ninguno

#### Escenario: una respuesta débil sigue viéndose
- GIVEN scores `[0.10, 0.02, 0.01]`
- THEN se muestran los dos primeros, como hoy

**Tests:** `a_hit_that_did_not_score_is_never_shown`, `hits_that_did_not_score_do_not_fill_the_minimum`, `an_exact_hit_is_shown_among_hits_that_did_not_score` (`relevance.rs`); los tests que ya existen en `relevance.rs` fijan que el resto no cambia.

## R2. `POST /v1/search` dice cuánto ocultó

La respuesta MUST llevar `withheld`: cuántos hits de la búsqueda dejó afuera el corte de `min_relative_score`, y `0` sin el campo. Sin `min_relative_score`, la respuesta MUST traer todos los hits, los de score 0 incluidos. `DaemonClient::search` MUST devolver los hits y `withheld`, leído con `#[serde(default)]`.

#### Escenario: nada relevante
- GIVEN tres filas ortogonales a la consulta
- WHEN se busca con `min_relative_score = 0.35`
- THEN la respuesta trae `hits: []` y `withheld: 3`

#### Escenario: sin corte
- GIVEN las mismas filas
- WHEN se busca sin `min_relative_score`
- THEN vuelven las tres y `withheld: 0`

**Tests:** `search_withholds_hits_that_did_not_score` (`crates/clawg-cli/tests/api.rs`); `a_search_answer_without_withheld_still_parses` (`client.rs`).

## R3. `recall_context` no rellena

`build_memory_prompt` MUST sacar los hits de score 0 después de sacar las filas del workspace y antes de cortar en `K_FINAL`. Un hit sacado MUST NOT aparecer en el prompt ni entrar en las filas sobre las que se piden los conflictos. `Retrieval::Searched` MUST decir cuántos hits usó el prompt antes del presupuesto (`hits`) y cuántos sacó (`withheld`).

#### Escenario: cero de relleno
- GIVEN una fila igual a la consulta y otra ortogonal
- WHEN se arma el prompt
- THEN sólo aparece la primera, el prompt usó 1 hit y sacó 1

#### Escenario: un conflicto ajeno
- GIVEN un par escalado cuyos dos lados son ortogonales a la consulta
- WHEN se arma el prompt
- THEN el prompt no tiene el bloque ACTIVE CONFLICTS

**Tests:** `recall_leaves_out_hits_that_did_not_score` (reemplaza a `recall_does_not_count_zero_score_hits_as_accesses`, cuya aserción de acceso pasa a él), `a_hit_that_did_not_score_brings_no_conflict` (`prompt_builder.rs`).

## R4. `POST /v1/context` dice cuánto ocultó

Con `retrieval.state = "searched"`, la respuesta MUST llevar `hits` y `withheld` de `Retrieval::Searched`. `Context` del cliente MUST leerlos con `#[serde(default)]` (0 si faltan). Los otros estados MUST NOT cambiar.

**Tests:** `context_says_how_many_hits_it_withheld` y `context_says_whether_memories_were_searched` (`crates/clawg-cli/tests/api.rs`); `a_searched_context_without_counts_still_parses` (`client.rs`).

## R5. Las herramientas dicen la frase de abstención

`search_memory` MUST responder la frase de abstención cuando no queda ninguna línea y algún hit se ocultó (en el daemon o en el puente), y `no memories found` cuando no se ocultó ninguno. `recall_context` MUST empezar con la frase cuando `retrieval` es `searched` con `hits = 0` y `withheld > 0`, seguida de `; only the workspace is shown below.` si hay prompt o de `; the workspace is empty.` si no lo hay; en ese último caso reemplaza a `no context yet`. Con otro estado MUST decir lo de hoy (`Memories were not searched: …`), nunca la frase. Sin los campos (un daemon anterior) MUST NOT agregar ninguna frase.

#### Escenario: nada relevante en la búsqueda
- GIVEN un daemon que respondió `hits: []` y `withheld: 3`
- THEN `search_memory` dice `no relevant memories for this query: 3 searched, none close enough to show`

#### Escenario: alcance vacío
- GIVEN `hits: []` y `withheld: 0`
- THEN `search_memory` dice `no memories found`

#### Escenario: el modelo carga
- GIVEN `retrieval.state = "embedder_not_ready"`
- THEN `recall_context` dice que las memorias no se buscaron, y no la frase

**Tests:** `search_says_when_nothing_was_relevant`, `nothing_stored_is_not_called_irrelevant`, `recall_says_when_nothing_was_relevant`, `an_old_daemon_gets_no_invented_notice` (que suma `searched` sin conteos) y `recall_says_when_memories_were_not_searched` (`tools.rs`); `search_memory_tells_nothing_relevant_from_nothing_stored` y `search_with_nothing_stored_says_so_without_failing` (`server.rs`).

## R6. agent-view lee la frase como cero líneas

El parser de `rag-eval --agent-view` MUST leer la frase de abstención como una salida sin líneas, igual que `no memories found`, de modo que un caso `unanswerable` cuya búsqueda la dice cuenta como `abstained`.

**Test:** `the_abstention_sentence_parses_to_no_lines` (`agent_view/parse.rs`).

## Invariantes

- El ranking del core y lo que `rag-eval` mide sobre él no cambian.
- `agent_contract.json` no cambia.
- Ningún hit de score 0 se registra como acceso (ya era así, `crates/clawg-core/src/memory/access.rs:147-155`).
