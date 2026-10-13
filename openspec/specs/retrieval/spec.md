# retrieval — Especificación

Fuente de verdad del comportamiento actual de la recuperación: el corte relativo de `POST /v1/search` y de la herramienta `search_memory`, qué hits cuentan como acceso en `search` y en `recall_context`, y el bono que lleva una fila con el identificador exacto de la consulta.

**Origen:**
- [retrieval-hygiene](../../changes/archive/2026-10-12-retrieval-hygiene/proposal.md), archivado el 2026-10-12.
- [lexical-fusion](../../changes/archive/2026-10-12-lexical-fusion/proposal.md), archivado el 2026-10-12.
- [knowledge-synthesis](../../changes/archive/2026-10-13-knowledge-synthesis/proposal.md), archivado el 2026-10-13 (sólo el apartado de las filas que dejó la app de escritorio).

## Definiciones

- **Corte relativo con ratio r (`relevant_len`, `crates/clawg-cli/src/relevance.rs:20`):** sobre scores ordenados mejor primero, se conservan los primeros mientras la posición sea menor que `MIN_SHOWN_HITS` o el score sea `>= r × mejor`. Sin scores, no queda ninguno.
- **`RELEVANCE_RATIO` (`relevance.rs:14`):** 0,6. El ratio que usa el puente MCP y que pide el cliente HTTP.
- **`MIN_SHOWN_HITS` (`relevance.rs:17`):** 2. Hits que pasan el corte sea cual sea su score.
- **Acceso:** un id que llega al access tracker. En el flush suma `access_count`, lleva `last_accessed_at` al máximo entre el actual y el del flush, y recalcula el tier en la misma sentencia (`flush`, `crates/clawg-core/src/memory/access.rs:175`; el `UPDATE` en `:204-209`). El flush corre cada 30 s o al juntar 500 ids (`access.rs:76-78`).
- **Hit contable:** hit de `kind = memory` con `score > 0` (`AccessTracker::record_hits`, `access.rs:151`). Sólo cuentan los de `memory` porque los ids son por tabla y el flush sólo actualiza `memory`. Un hit con score 0 está bajo el piso de similitud y ocupa un lugar de relleno.
- **Token:** cada tramo de `char::is_alphanumeric() || '_'` de la consulta, como los parte `fts_tokens` (`crates/clawg-core/src/memory/search.rs:520`).
- **Token identificador (`is_identifier`, `search.rs:656`):** un token que cumple al menos una de:
  - tiene letras y dígitos (`getUser4821`, `v2`);
  - tiene `_` y al menos una letra (`E_AUTH_4821`, `snake_case`);
  - tiene una minúscula seguida inmediatamente de una mayúscula (`getEstadoLabel`).
  Un número solo (`4821`) no es identificador.
- **Identificadores de la consulta (`identifier_tokens`, `search.rs:643`):** los tokens identificadores distintos de toda la consulta, en orden de aparición, hasta 8 (`MAX_IDENTIFIER_TOKENS`, `search.rs:635`).
- **Match de identificador (`identifier_matches`, `search.rs:698`):** fila de `memory` visible para el llamador (mismos predicados de agente, proyecto, estado y `deleted_at` que el canal léxico) cuyo contenido tiene **todos** los identificadores de la consulta, según FTS5 (`"t1" AND "t2" …`). FTS5 separa en `_`, así que `"user_id"` es una frase que también matchea `user id` y `user-id`: para los identificadores con `_` el contenido MUST tenerlos escritos tal cual, sin distinguir mayúsculas y sin pegarse a otro nombre (`holds_identifier`, `search.rs:667`). Las filas que no los tienen así no reciben bono ni cuentan para el presupuesto.
- **Presupuesto léxico:** `SearchScope.lexical`, 20 por defecto (`search.rs:38`).
- **Bono léxico:** `ScoringConfig.lexical_bonus`, 1,0 por defecto (`crates/clawg-core/src/memory/score.rs:183`).

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

## Identificadores de la consulta

`identifier_tokens(query)` MUST devolver los identificadores de la consulta según la definición, de **toda** la consulta, sin repetir y como mucho 8. Es una función pura.

#### Escenario: tipos de identificador
- GIVEN la consulta `qué hace getEstadoLabel con E_AUTH_4821 y getUser4821 en 2026`
- THEN devuelve `getEstadoLabel`, `E_AUTH_4821`, `getUser4821`, en ese orden
- AND no devuelve `qué`, `hace`, `con`, `y`, `en` ni `2026`

#### Escenario: identificador al final de una consulta larga
- GIVEN una consulta de 20 palabras comunes seguidas de `parseLedger7731`
- THEN devuelve `parseLedger7731`

#### Escenario: sin identificadores
- GIVEN `how do we deploy the importer`
- THEN devuelve una lista vacía

**Test:** `identifier_tokens_*` en `search.rs`, mod tests.

## El score de un match suma el bono

Para un hit marcado como match de identificador (`RawHit.lexical_match`, `score.rs:30`), `score()` (`score.rs:261`) MUST valer `(cos' + lexical_bonus) × max(piso_tier, decaimiento) × importancia × estado`, donde `cos'` es el coseno ya re-escalado contra el piso (la suma en `score.rs:274`). Para un hit sin match, MUST valer lo mismo que sin el bono.

#### Escenario: el bono se suma después del piso
- GIVEN un hit match con coseno bajo el piso (cos' = 0), tier 1, edad 0, importancia 1 y estado `active`
- THEN el score es `lexical_bonus × importancia`, mayor que 0

#### Escenario: sin match, sin cambio
- GIVEN el mismo hit sin match
- THEN el score es 0

**Test:** `a_lexical_match_adds_the_bonus_after_the_floor`, `without_a_match_the_score_is_unchanged` y `a_match_outranks_a_closer_row_with_the_same_decay_and_importance` en `score.rs`; el literal, en `scoring_config_default_pins_the_shipped_ranking_literals`.

## La búsqueda premia el identificador exacto

Con el canal léxico encendido y texto de consulta, `search_all_memories` MUST marcar como match de identificador cada fila que lo sea, venga del canal denso (`knn_one`, `search.rs:338`) o del léxico (`score_lexical_candidates`, `search.rs:895`), y MUST sumar los matches a los candidatos léxicos aunque estén fuera del top denso o pasado el token 16. Los matches se calculan antes del `try_join!` de los canales (`search.rs:1026`).

#### Escenario: el identificador le gana a los hermanos
- GIVEN 8 filas: 7 hermanos de coseno alto con `resolveLedger4812`, `resolveLedger4821`… y una fila de coseno más bajo que menciona `resolveLedger4182`
- WHEN se busca `what does resolveLedger4182 do`
- THEN la fila con `resolveLedger4182` sale primera

#### Escenario: identificador fuera del top denso
- GIVEN una fila con `parseLedger7731` lejos del vector de la consulta (fuera del pool denso) y la consulta con `parseLedger7731` después del token 16
- WHEN se busca
- THEN la fila vuelve con score > 0

#### Escenario: canal apagado
- GIVEN las mismas filas y `SearchScope.lexical = None`, o sin texto de consulta
- THEN no hay bono

**Test:** `an_exact_identifier_outranks_its_closer_siblings`, `an_identifier_past_the_sixteenth_token_still_reaches_its_row` y `no_bonus_without_the_lexical_channel_or_query_text` en `search.rs`.

## Un identificador que no discrimina no da bono

Si las filas visibles que matchean los identificadores de la consulta son más que el presupuesto léxico, ninguna fila MUST recibir el bono (la consulta trae `presupuesto + 1` filas como mucho). Si la consulta no tiene identificadores, MUST NOT correr la consulta de matches.

#### Escenario: identificador repetido
- GIVEN 21 filas que mencionan `getUser42` y un presupuesto de 20
- WHEN se busca `getUser42`
- THEN ningún hit lleva bono: los scores son los mismos que con `lexical_bonus = 0`

#### Escenario: consulta natural
- GIVEN una consulta sin tokens identificadores
- THEN los scores son bit a bit los de `lexical_bonus = 0`

**Test:** `an_identifier_shared_by_more_rows_than_the_budget_gives_no_bonus` y `a_query_without_identifiers_scores_exactly_as_without_the_bonus` en `search.rs`.

## El bono respeta el scope

Un match de identificador MUST cumplir los mismos predicados que el candidato léxico: agente, proyecto (con la excepción de `user_preference`), estado `active`/`disputed` y `deleted_at IS NULL` (`IDENTIFIER_MATCH_SQL`, `search.rs:682`). Una fila de otro agente, borrada o de otro proyecto MUST NOT recibir el bono ni contar para el presupuesto.

#### Escenario: fila de otro agente
- GIVEN 21 filas de otro agente y una del llamador con `getUser42`
- WHEN el llamador busca `getUser42` con presupuesto 20
- THEN su fila lleva el bono

**Test:** `rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget` en `search.rs`.

#### Escenario: palabras partidas como el identificador
- GIVEN 25 filas con `snake case` o `snake-case` y una con `snake_case`, y presupuesto 20
- WHEN se busca `snake_case`
- THEN sólo la fila con `snake_case` es match

**Test:** `an_underscore_identifier_is_held_only_as_written` y `words_split_like_an_underscore_identifier_neither_match_nor_use_the_budget` en `search.rs`.

## La búsqueda no falla por el canal de identificadores

Un error de la consulta de matches (FTS ausente, MATCH rechazado, conexión cerrada) MUST dejar la búsqueda como si no hubiera matches, igual que los candidatos léxicos (`search.rs:731`, junto a `:627`).

**Test:** `a_failing_identifier_query_leaves_the_search_unboosted` en `search.rs`.

## Las filas que dejó la app de escritorio siguen alcanzables

Ningún código de producción escribe `episodic_trajectory` ni `procedural_skill`, pero una base puede traer filas de la app de escritorio. Esas filas MUST seguir alcanzables: la búsqueda MUST consultarlas (KNN de 5 + 5), el workspace MUST mostrar los skills del proyecto, y `list_memories`, `GET`, `PATCH` y `DELETE` MUST aceptarlas. El re-embed de `procedural_skill` MUST embeber `"{trigger} {skill_name}"`.

#### Escenario: filas como las escribía la app de escritorio
- GIVEN una trayectoria y un skill escritos con los mismos escritores que usaba la app (que quedan sólo para tests), y sus vectores
- WHEN se busca con el vector del skill, se regenera el workspace del proyecto, se lista y se re-embebe
- THEN el skill está entre los hits y en el workspace, las dos filas están en el listado, y el re-embed del skill embebe `"when login fails Fix login flow"` para `trigger = "when login fails"` y `skill_name = "Fix login flow"`

**Test:** `episodic_trajectory_retrievable_by_same_agent_id` y `knn_procedural_returns_skill_rows` (`search.rs`), `regenerate_reports_the_ids_it_rendered` y `workspace_top_skills_exclude_foreign_project` (`workspace.rs`), `list_memories_kind_none_merges_all_tables_sorted_desc` (`admin.rs`) y `reembed_reinfer_uses_passage_role_and_shared_skill_embedding_text` (`maintenance.rs`, fija el texto literal). La apertura de una base en V9 con esas filas está en [`specs/synthesis`](../synthesis/spec.md) (su R9).

## Invariantes

- El ranking del core (`search_all_memories`, `hit_cmp`) no depende del corte: el corte sólo decide cuánto de la lista recibe quien llama.
- Las corridas de eval no registran accesos (`AccessTracker::inert`, `crates/clawg-core/src/eval_api.rs:461`).
- Con `lexical_bonus = 0` el ranking es bit a bit el de antes del bono: es la forma de apagarlo sin tocar el canal léxico.
- Los contadores ya guardados no se reescriben. Los accesos que se contaron de más antes de este comportamiento quedan como estaban.

## Fuera de esta spec

- El orden entre hits con score 0: `hit_cmp` desempata por `(kind, id)` ascendente (`crates/clawg-core/src/memory/search.rs:78`). Desde lexical-fusion, un match de identificador ya no depende de ese desempate para llegar al top 10: el bono lo deja con score > 0 aunque su coseno quede bajo el piso.
- Un bono parcial (la fila tiene sólo algunos identificadores de la consulta), números solos como identificadores y FTS para `episodic` y `procedural`.
