# retrieval — Especificación

Fuente de verdad del comportamiento actual de la recuperación: el corte relativo de `POST /v1/search` y de la herramienta `search_memory`, y qué hits cuentan como acceso en `search` y en `recall_context`.

**Origen:** [retrieval-hygiene](../../changes/archive/2026-10-12-retrieval-hygiene/proposal.md), archivado el 2026-10-12.

## Definiciones

- **Corte relativo con ratio r (`relevant_len`, `crates/clawg-cli/src/relevance.rs:20`):** sobre scores ordenados mejor primero, se conservan los primeros mientras la posición sea menor que `MIN_SHOWN_HITS` o el score sea `>= r × mejor`. Sin scores, no queda ninguno.
- **`RELEVANCE_RATIO` (`relevance.rs:14`):** 0,6. El ratio que usa el puente MCP y que pide el cliente HTTP.
- **`MIN_SHOWN_HITS` (`relevance.rs:17`):** 2. Hits que pasan el corte sea cual sea su score.
- **Acceso:** un id que llega al access tracker. En el flush suma `access_count`, lleva `last_accessed_at` al máximo entre el actual y el del flush, y recalcula el tier en la misma sentencia (`flush`, `crates/clawg-core/src/memory/access.rs:175`; el `UPDATE` en `:204-209`). El flush corre cada 30 s o al juntar 500 ids (`access.rs:76-78`).
- **Hit contable:** hit de `kind = memory` con `score > 0` (`AccessTracker::record_hits`, `access.rs:151`). Sólo cuentan los de `memory` porque los ids son por tabla y el flush sólo actualiza `memory`. Un hit con score 0 está bajo el piso de similitud y ocupa un lugar de relleno.

El corte vive en `clawg-cli` y no en el core: `search_all_memories` devuelve siempre la lista completa ordenada, y las métricas de `rag-eval` se miden sobre esa lista (`rag-eval` llama a `store.search` directo).

## Corte relativo en `POST /v1/search`

`POST /v1/search` MUST aceptar el campo opcional `min_relative_score` (`SearchRequest`, `crates/clawg-cli/src/daemon/api.rs:286-297`).

- Ausente: la respuesta MUST traer todos los hits ordenados hasta `limit`, sin corte.
- Presente con un valor en `[0, 1]`: el daemon MUST devolver sólo los hits que pasan el corte relativo con ese ratio, en el mismo orden (`search`, `api.rs:634`; el corte en `:672-676`).
- Presente fuera de `[0, 1]`, o no finito: MUST responder `400` sin buscar ni registrar accesos (`check_relative_score`, `api.rs:262`). La validación corre después de comprobar el embedder: sin embedder listo, la respuesta es la de esa condición (`503`).

#### Escenario: sin campo
- GIVEN una fila igual a la consulta y tres sin relación
- WHEN se busca sin `min_relative_score`
- THEN vuelven las cuatro

#### Escenario: con corte
- GIVEN las mismas filas
- WHEN se busca con `min_relative_score = 0.6`
- THEN vuelven los primeros `relevant_len(scores, 0.6)` hits de la búsqueda sin corte, en el mismo orden, y son menos que cuatro

#### Escenario: fuera de rango
- WHEN se busca con `min_relative_score = 1.5` o `-0.1`
- THEN la respuesta es `400`

**Tests:** `search_without_a_relative_cut_is_unchanged`, `search_applies_the_requested_relative_cut` y `search_rejects_a_relative_score_outside_zero_to_one` (`crates/clawg-cli/tests/api.rs`); `a_relative_score_inside_zero_to_one_passes_through` y `a_relative_score_outside_zero_to_one_is_a_bad_request`, que incluye `NaN` e infinito (`api.rs`, mod tests); `hits_far_below_the_best_match_are_not_shown`, `a_hit_exactly_at_the_ratio_is_shown`, `the_two_best_are_shown_whatever_they_score` y `no_scores_keep_nothing` (`relevance.rs`).

## `search` registra sólo lo que devuelve

El handler `search` MUST registrar como acceso exactamente los hits contables de la respuesta que envía, después del corte (`api.rs:677`). Un hit cortado, o con score 0, MUST NOT cambiar `access_count` ni `last_accessed_at`.

#### Escenario: hit cortado
- GIVEN una fila A con score alto y una fila B bajo el corte de 0,6 en posición ≥ 2
- WHEN se busca con `min_relative_score = 0.6` y se hace flush del tracker
- THEN `access_count` de A sube en 1, y el de B y su `last_accessed_at` no cambian

#### Escenario: cero sin corte
- GIVEN una fila C que vuelve con score 0
- WHEN se busca sin `min_relative_score` y se hace flush
- THEN `access_count` de C no cambia

**Tests:** `record_hits_keeps_only_memory_hits_that_scored` y `record_hits_with_nothing_countable_sends_nothing` (`access.rs`, mod tests) fijan qué hits cuentan. `search_applies_the_requested_relative_cut` fija que la respuesta, que es lo que se registra, no trae los cortados. El flush corre cada 30 s y un test HTTP no lo puede adelantar, por eso la prueba va en esas dos piezas.

## `recall_context` no cuenta los ceros

`build_memory_prompt` MUST registrar sólo los hits contables entre los que entran al prompt, después de sacar las filas del workspace y de cortar en `K_FINAL = 10` (`crates/clawg-core/src/memory/prompt_builder.rs:32`, `:156-157`, `:165`). Los hits con score 0 MUST seguir apareciendo en el prompt.

#### Escenario: cero de relleno
- GIVEN una fila igual a la consulta y otra ortogonal (score 0)
- WHEN se arma el prompt y se hace flush
- THEN las dos aparecen en el prompt, y sólo la primera sube `access_count`

#### Escenario: otra tabla
- GIVEN un hit procedural con el mismo id que una fila `memory` que no es hit
- WHEN se arma el prompt y se hace flush
- THEN la fila `memory` no cambia

**Tests:** `recall_does_not_count_zero_score_hits_as_accesses`, `a_memory_hit_is_recorded_by_the_access_tracker` y `a_hit_from_another_table_does_not_touch_the_memory_row_with_the_same_id` (`prompt_builder.rs`, mod tests).

## Cliente HTTP y herramienta `search_memory`

`DaemonClient::search` MUST mandar `min_relative_score = RELEVANCE_RATIO` junto con los campos de `SearchArgs` (`search_request`, `crates/clawg-cli/src/client.rs:288`; uso en `:436`). El puente MUST volver a aplicar el corte al renderizar `search_memory` (`relevant_hits` y `format_search`, `crates/clawg-cli/src/mcp/tools.rs:55` y `:62`). Aplicar el corte dos veces da el mismo resultado, y contra un daemon que ignora el campo el agente ve lo mismo.

#### Escenario: cuerpo del pedido
- WHEN el cliente arma el cuerpo de una búsqueda
- THEN el cuerpo contiene `"min_relative_score": 0.6` además de los campos de `SearchArgs`

#### Escenario: render
- GIVEN hits con scores 0,50, 0,10 y 0,05
- THEN `search_memory` muestra los dos primeros y no el tercero

**Tests:** `search_request_carries_the_relevance_ratio` (`client.rs`, mod tests); `relevant_hits_keeps_the_ones_the_cut_lets_through_in_order` y `the_search_output_shows_only_the_relevant_hits` (`tools.rs`, mod tests).

## Invariantes

- El ranking del core (`search_all_memories`, `hit_cmp`) no depende del corte: el corte sólo decide cuánto de la lista recibe quien llama.
- Las corridas de eval no registran accesos (`AccessTracker::inert`, `crates/clawg-core/src/eval_api.rs:461`).
- Los contadores ya guardados no se reescriben. Los accesos que se contaron de más antes de este comportamiento quedan como estaban.

## Fuera de esta spec

- El orden entre hits con score 0, y si se muestran, se decide en lexical-fusion. Hoy `hit_cmp` desempata por `(kind, id)` ascendente (`crates/clawg-core/src/memory/search.rs:78`), y ese orden es lo único que deja llegar al top 10 a un match léxico bajo el piso.
