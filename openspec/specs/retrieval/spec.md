# retrieval — Especificación

Fuente de verdad del comportamiento actual de la recuperación: el corte relativo de `POST /v1/search` y de la herramienta `search_memory`, qué hits cuentan como acceso en `search` y en `recall_context`, el bono que lleva una fila con el identificador exacto de la consulta, y el texto que arma `recall_context` (workspace, último resumen de sesión, hits y conflictos, dentro de un presupuesto de bytes) junto con el parser que lo lee en `rag-eval`. Cómo se pide ese presupuesto, y cómo avisa `recall_context` que no buscó memorias o que ninguna se acercó, está en [`specs/mcp`](../mcp/spec.md). A qué filas de otros agentes llegan la búsqueda y el contexto lo define [`specs/sharing`](../sharing/spec.md); acá está cómo se muestra un hit ajeno. La búsqueda en la documentación y el código de un proyecto es otra cosa y está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [retrieval-hygiene](../../changes/archive/2026-10-12-retrieval-hygiene/proposal.md), archivado el 2026-10-12.
- [lexical-fusion](../../changes/archive/2026-10-12-lexical-fusion/proposal.md), archivado el 2026-10-12.
- [knowledge-synthesis](../../changes/archive/2026-10-13-knowledge-synthesis/proposal.md), archivado el 2026-10-13 (sólo el apartado de las filas que dejó la app de escritorio).
- [context-density](../../changes/archive/2026-10-14-context-density/proposal.md), archivado el 2026-10-14 (desde "Línea de hit en `recall_context`" hasta "Medición del contexto").
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo "Hits de otros agentes" y los cambios de agente en "El bono respeta el scope" y en "Línea de hit en `recall_context`").
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "El piso de coseno", "Indexar un proyecto no cambia la búsqueda de memorias", el valor de `RELEVANCE_RATIO` y la medición con pooling mean en "Medición del contexto").
- [memory-rank-fusion](../../changes/archive/2026-10-17-memory-rank-fusion/proposal.md), archivado el 2026-10-17 (sólo "Definiciones", "Corte relativo en `POST /v1/search`" y "Cliente HTTP y herramienta `search_memory`").
- [context-budget](../../changes/archive/2026-10-17-context-budget/proposal.md), archivado el 2026-10-17 (desde "El prompt cumple el presupuesto" hasta "Un workspace más grande que el presupuesto se entrega y se dice", y lo que esos apartados cambian en "Definiciones", "`recall_context` deja afuera los ceros", los conflictos, "Medición del contexto" e "Invariantes").
- [memory-protocol](../../changes/archive/2026-10-17-memory-protocol/proposal.md), archivado el 2026-10-17 (sólo el texto en inglés de "Una NOTE por bloque de conflictos" y de la línea de omitidos en "Orden y tope de los conflictos").
- [conflict-candidates](../../changes/archive/2026-10-18-conflict-candidates/proposal.md), archivado el 2026-10-18 (sólo el texto de "Una NOTE por bloque de conflictos" y qué es un "Par de conflicto").
- [search-abstention](../../changes/archive/2026-10-18-search-abstention/proposal.md), archivado el 2026-10-18 (sólo la fila de score 0 y la frase de abstención en "Definiciones", y lo que cambian en el corte relativo, en "Corte relativo en `POST /v1/search`", en "`recall_context` deja afuera los ceros", en "Cliente HTTP y herramienta `search_memory`", en el parser de `rag-eval --agent-view`, en "Medición del contexto", en "Invariantes" y en "Fuera de esta spec").
- [agent-lifecycle-hooks](../../changes/archive/2026-10-18-agent-lifecycle-hooks/proposal.md), archivado el 2026-10-18 (sólo «El contexto sin consulta», R1 de esa spec; sus R2 a R5 viven en [`specs/hooks`](../hooks/spec.md)).
- [session-handoff](../../changes/archive/2026-10-19-session-handoff/proposal.md), archivado el 2026-10-19 (sólo «La última sesión», sus R3, R4 y R6, y lo que cambian en «Definiciones», «Hits de otros agentes», «Línea de hit en `recall_context`», «El contexto sin consulta», los apartados del presupuesto, el parser de `rag-eval --agent-view`, «Medición del contexto» e «Invariantes»; el registro de sesiones y el sello en el save viven en [`specs/ingest`](../ingest/spec.md), y el registro desde los hooks en [`specs/hooks`](../hooks/spec.md)).

## Definiciones

- **Hit exacto:** hit cuya fila recibió el bono léxico en esta búsqueda, es decir, `RawHit.lexical_match` (`crates/clawg-core/src/memory/score.rs:28-30`), una fila de `memory` que está en los matches de identificador de la consulta. Viaja en cada hit como `exact`. Un hit de `episodic` o `procedural` nunca es exacto.
- **Fila de score 0:** hit no exacto cuyo score es 0, porque su coseno queda bajo el piso (`rescale_cosine`, `crates/clawg-core/src/memory/score.rs:217`) y no tiene el identificador de la consulta. Vuelve sólo porque el pool tenía lugar. Un hit exacto lleva el bono y nunca vale 0 mientras esté vivo.
- **Frase de abstención:** `no relevant memories for this query: N searched, none close enough to show` (`NO_RELEVANT_MEMORIES` y `nothing_relevant`, `crates/clawg-cli/src/mcp/tools.rs:72-76`), con `N` el número de hits que la búsqueda trajo y no se muestran. Distingue «hay memorias y ninguna se acerca» de `no memories found`, que es un alcance sin nada.
- **Corte relativo con ratio r (`shown_hits`, `crates/clawg-cli/src/relevance.rs:36`):** sobre hits ordenados mejor primero, cada uno con su score y su marca de exacto, pasa todo hit exacto, aunque quede detrás de uno cortado. Una fila de score 0 no pasa nunca y no cuenta para `MIN_SHOWN_HITS` (`relevance.rs:44-46`). Un hit no exacto con score mayor que 0 pasa si está entre los `MIN_SHOWN_HITS` primeros no exactos que puntuaron o si su score es `>= r × mejor score no exacto`. Los que pasan conservan su orden. Sin scores, no queda ninguno. Sin hits exactos ni filas de score 0, el resultado es el de cortar por prefijo: los primeros mientras la posición sea menor que `MIN_SHOWN_HITS` o el score sea `>= r × mejor`.
- **`RELEVANCE_RATIO` (`relevance.rs:19`):** 0,35. El ratio que usa el puente MCP y que pide el cliente HTTP. Medido con `rag-eval --real --agent-view` sobre vectores con pooling mean, cuyo mejor score tiene mediana 0,19: con 0,35 `search_memory` muestra el recuerdo esperado en 36 de 46 consultas con 764 tokens de media; con 0,6 eran 30 con 455 y con 0,2 son 37 con 894. El score del esperado (0,03–0,41) y el del resto (hasta 0,38) se superponen, por eso el corte es relativo.
- **`MIN_SHOWN_HITS` (`relevance.rs:23`):** 2. Hits no exactos que pasan el corte sea cual sea su score, siempre que sea mayor que 0; los exactos no cuentan para este mínimo.
- **Acceso:** un id que llega al access tracker. En el flush suma `access_count`, lleva `last_accessed_at` al máximo entre el actual y el del flush, y recalcula el tier en la misma sentencia (`flush`, `crates/clawg-core/src/memory/access.rs:175`; el `UPDATE` en `:204-209`). El flush corre cada 30 s o al juntar 500 ids (`access.rs:76-78` y `:90`).
- **Decaimiento:** `exp(−λ · max(edad, 0))`, con la edad en días desde `last_accessed_at` y `λ` la del tipo de la fila (`score()`, `crates/clawg-core/src/memory/score.rs:283-287`). Una fecha futura (un reloj atrasado después de guardar, un restore de una máquina adelantada) dice que el reloj se movió, no que la fila es más nueva que una leída recién: MUST valer 1, nunca más. El flush conserva esa fecha (`MAX(last_accessed_at, …)`), así que la fila puntúa como recién leída hasta que el reloj la alcanza. **Test:** `an_access_stamped_in_the_future_scores_like_one_made_now` (`score.rs`).
- **Hit contable:** hit de `kind = memory` con `score > 0` (`AccessTracker::record_hits`, `access.rs:151`). Sólo cuentan los de `memory` porque los ids son por tabla y el flush sólo actualiza `memory`. Un hit con score 0 está bajo el piso de similitud: ninguna herramienta lo muestra, y sólo lo devuelve `POST /v1/search` sin `min_relative_score`.
- **Token:** cada tramo de `char::is_alphanumeric() || '_'` de la consulta, como los parte `fts_tokens` (`crates/clawg-core/src/memory/search.rs:694`).
- **Token identificador (`is_identifier`, `search.rs:838`):** un token que cumple al menos una de:
  - tiene letras y dígitos (`getUser4821`, `v2`);
  - tiene `_` y al menos una letra (`E_AUTH_4821`, `snake_case`);
  - tiene una minúscula seguida inmediatamente de una mayúscula (`getEstadoLabel`).
  Un número solo (`4821`) no es identificador.
- **Identificadores de la consulta (`identifier_tokens`, `search.rs:825`):** los tokens identificadores distintos de toda la consulta, en orden de aparición, hasta 8 (`MAX_IDENTIFIER_TOKENS`, `search.rs:817`).
- **Match de identificador (`identifier_matches`, `search.rs:895`):** fila de `memory` visible para el llamador (mismos predicados de agente, proyecto, estado y `deleted_at` que el canal léxico) cuyo contenido tiene **todos** los identificadores de la consulta, según FTS5 (`"t1" AND "t2" …`). FTS5 separa en `_`, así que `"user_id"` es una frase que también matchea `user id` y `user-id`: para los identificadores con `_` el contenido MUST tenerlos escritos tal cual, sin distinguir mayúsculas y sin pegarse a otro nombre (`holds_identifier`, `search.rs:858`). Las filas que no los tienen así no reciben bono ni cuentan para el presupuesto.
- **Presupuesto léxico:** `SearchScope.lexical`, 20 por defecto (`search.rs:43`).
- **Candidato léxico:** fila de `memory` que trae el canal léxico: matchea la consulta en `memory_fts`, mejor primero por bm25, y cumple antes del `LIMIT` del presupuesto léxico los mismos predicados de estado (`active` o `disputed`), `deleted_at`, agente y proyecto que el canal denso (`LEXICAL_CANDIDATE_SQL`, `search.rs:737`). `memory_fts` pierde una fila sólo cuando se borra, así que sin el filtro de estado las versiones `superseded` de un tema, casi el texto de la vigente, gastarían el presupuesto y la dejarían afuera. La consulta MUST recorrer los matches de `memory_fts` y leer `memory` por rowid, no al revés. **Tests:** `superseded_versions_do_not_spend_the_lexical_budget` y `the_lexical_candidate_query_is_driven_by_the_match` (`search.rs`).
- **Bono léxico:** `ScoringConfig.lexical_bonus`, 1,0 por defecto (`crates/clawg-core/src/memory/score.rs:188`).
- **Presupuesto del contexto:** el `budget` de `POST /v1/context`, en bytes del prompt (unos 4 por token): `DEFAULT_BUDGET` 8.000 y `MAX_BUDGET` 24.000 (`crates/clawg-core/src/memory/prompt_builder.rs:54-56`). Cómo llega y se valida está en [`specs/mcp`](../mcp/spec.md).
- **Hit candidato:** hit que el prompt de `recall_context` llevaría sin presupuesto: los de la búsqueda sin las filas del workspace ni las filas de score 0, cortados en `K_FINAL = 10` (`build_memory_prompt`, `prompt_builder.rs:156`; las filas del workspace salen en `:217`, las de score 0 en `:222` y el corte en `:224`).
- **Hit mostrado:** hit candidato de `kind = memory` que el presupuesto dejó en RECENT o LONG-TERM, entero o recortado. Los ids son por tabla, así que un hit episodic o procedural nunca cuenta. Sin recorte, los hits mostrados son todos los candidatos de `memory`. El resumen de LAST SESSION ([«La última sesión»](#la-última-sesión)) no es hit candidato ni mostrado.
- **Cortado / recortado:** un hit candidato que el presupuesto dejó afuera / uno cuyo texto acortó.
- **Fila del workspace:** id en `EmittedIds` (`crates/clawg-core/src/memory/workspace.rs:201`), que se suma a los candidatos de `memory` en `shown` (`assemble`, `prompt_builder.rs:315-320`). Su bullet no tiene id. **No** es un hit mostrado.
- **Posición de un hit:** índice 0-based en el orden impreso: RECENT y luego LONG-TERM (`positions`, `prompt_builder.rs:494`). LAST SESSION no cuenta. Las posiciones se toman antes del presupuesto; el presupuesto puede quitar un hit del medio, pero los que quedan conservan su orden.
- **Par de conflicto:** un par escalado: fila nueva (`disputed`) contra una fila vieja que nombra en `disputed_against` ([`specs/disputes`](../disputes/spec.md)). Los pares pendientes no entran al bloque; los ve el agente en la respuesta del save y en `memory_status`. `fetch_active_conflicts` (`prompt_builder.rs:953`) sólo trae pares con al menos un lado en `shown`, es decir, hit candidato de `memory` o fila del workspace, antes del presupuesto; el otro lado puede estar fuera del prompt.

El corte vive en `clawg-cli` y no en el core: `search_all_memories` devuelve siempre la lista completa ordenada, y las métricas de `rag-eval` se miden sobre esa lista (`rag-eval` llama a `store.search` directo).

## Corte relativo en `POST /v1/search`

`POST /v1/search` MUST aceptar el campo opcional `min_relative_score` (`SearchRequest`, `crates/clawg-cli/src/daemon/api.rs:382-393`).

- Ausente: la respuesta MUST traer todos los hits ordenados hasta `limit`, sin corte, las filas de score 0 incluidas.
- Presente con un valor en `[0, 1]`: el daemon MUST devolver sólo los hits que pasan el corte relativo con ese ratio, en el mismo orden (`search`, `api.rs:1153`; el corte en `:1188-1191`). Cada hit de la respuesta MUST llevar `exact`.
- Presente fuera de `[0, 1]`, o no finito: MUST responder `400` sin buscar ni registrar accesos (`check_relative_score`, `api.rs:335`). La validación corre después de comprobar el embedder: sin embedder listo, la respuesta es la de esa condición (`503`).

La respuesta MUST llevar `withheld`: cuántos hits de la búsqueda dejó afuera el corte, `0` sin `min_relative_score` (`SearchBody`, `api.rs:605-610`; la cuenta en `:1187` y `:1194`). Con `hits` vacío, es lo que separa «nada se acercó» de «no hay nada guardado».

#### Escenario: sin campo
- GIVEN una fila igual a la consulta y tres sin relación
- WHEN se busca sin `min_relative_score`
- THEN vuelven las cuatro

#### Escenario: nada relevante
- GIVEN cuatro filas ortogonales a la consulta
- WHEN se busca con `min_relative_score = 0.35`
- THEN la respuesta trae `hits: []` y `withheld: 4`; sin el campo, vuelven las cuatro y `withheld: 0`

#### Escenario: con corte
- GIVEN las mismas filas
- WHEN se busca con `min_relative_score = 0.6`
- THEN vuelven los hits que pasa `shown_hits` con ratio 0,6 sobre la búsqueda sin corte, en el mismo orden, y son menos que cuatro

#### Escenario: exacto detrás de uno cortado
- GIVEN una fila con el identificador de la consulta, con coseno 0,85, y tres sin él con 0,88, 0,87 y 0,86, del mismo tipo, importancia y edad
- WHEN se busca con `min_relative_score = 0.35`
- THEN vuelven las cuatro, con `exact = true` sólo en la primera

#### Escenario: fuera de rango
- WHEN se busca con `min_relative_score = 1.5` o `-0.1`
- THEN la respuesta es `400`

**Tests:** `search_without_a_relative_cut_is_unchanged`, `search_applies_the_requested_relative_cut`, `search_rejects_a_relative_score_outside_zero_to_one` y `search_withholds_hits_that_did_not_score` (`crates/clawg-cli/tests/api.rs`); `a_relative_score_inside_zero_to_one_passes_through` y `a_relative_score_outside_zero_to_one_is_a_bad_request`, que incluye `NaN` e infinito (`api.rs`, mod tests); `relevance_ratio_pins_the_shipped_value`, `hits_far_below_the_best_match_are_not_shown`, `a_hit_exactly_at_the_ratio_is_shown`, `the_two_best_are_shown_whatever_they_score` y `no_scores_keep_nothing` (`relevance.rs`); `exact_hits_do_not_set_the_bar_for_the_others`, `an_exact_hit_is_shown_behind_one_the_cut_drops` y `the_two_best_inexact_hits_are_shown_whatever_they_score` (`relevance.rs`); `a_hit_that_did_not_score_is_never_shown`, `hits_that_did_not_score_do_not_fill_the_minimum` y `an_exact_hit_is_shown_among_hits_that_did_not_score` (`relevance.rs`); `search_shows_the_rows_an_identifier_match_used_to_hide` y `an_identifier_match_from_another_agent_is_exact_and_attributed` (`api.rs`).

## `search` registra sólo lo que devuelve

El handler `search` MUST registrar como acceso exactamente los hits contables de la respuesta que envía, después del corte (`api.rs:1192`). Un hit cortado, o con score 0, MUST NOT cambiar `access_count` ni `last_accessed_at`.

#### Escenario: hit cortado
- GIVEN una fila A con score alto y una fila B bajo el corte de 0,6 en posición ≥ 2
- WHEN se busca con `min_relative_score = 0.6` y se hace flush del tracker
- THEN `access_count` de A sube en 1, y el de B y su `last_accessed_at` no cambian

#### Escenario: cero sin corte
- GIVEN una fila C que vuelve con score 0
- WHEN se busca sin `min_relative_score` y se hace flush
- THEN `access_count` de C no cambia

**Tests:** `record_hits_keeps_only_memory_hits_that_scored` y `record_hits_with_nothing_countable_sends_nothing` (`access.rs`, mod tests) fijan qué hits cuentan. `search_applies_the_requested_relative_cut` fija que la respuesta, que es lo que se registra, no trae los cortados. El flush corre cada 30 s y un test HTTP no lo puede adelantar, por eso la prueba va en esas dos piezas.

## `recall_context` deja afuera los ceros

`build_memory_prompt` MUST sacar las filas de score 0 después de sacar las filas del workspace y antes de cortar en `K_FINAL = 10` (`crates/clawg-core/src/memory/prompt_builder.rs:217`, `:222`, `:224`; `K_FINAL` en `:47`). Una fila sacada MUST NOT aparecer en el prompt ni entrar en las filas sobre las que se piden los conflictos (`shown`, `:315-320`), así que un par escalado cuyos lados sólo llegan como filas de score 0 no trae el bloque ACTIVE CONFLICTS. `Retrieval::Searched` (`:102`) MUST decir cuántos hits usó el prompt antes del presupuesto (`hits`), contando el save que [LAST SESSION](#la-última-sesión) saca de los hits si era uno de ellos, y cuántas filas de score 0 sacó (`withheld`) (`:223` y `:226`).

Como acceso, `build_memory_prompt` MUST registrar sólo los hits contables entre los que quedan en el prompt después del presupuesto (`:255-257`). Un hit cortado MUST NOT contar; uno recortado cuenta.

#### Escenario: cero de relleno
- GIVEN una fila igual a la consulta y otra ortogonal (score 0)
- WHEN se arma el prompt y se hace flush
- THEN sólo la primera aparece en el prompt y sube `access_count`, y `Retrieval::Searched` dice `hits: 1` y `withheld: 1`

#### Escenario: un conflicto ajeno
- GIVEN un par escalado cuyos dos lados son ortogonales a la consulta
- WHEN se arma el prompt
- THEN el prompt no tiene el bloque ACTIVE CONFLICTS

#### Escenario: otra tabla
- GIVEN un hit procedural con el mismo id que una fila `memory` que no es hit
- WHEN se arma el prompt y se hace flush
- THEN la fila `memory` no cambia

#### Escenario: cortado por el presupuesto
- GIVEN dos hits contables y un presupuesto que sólo deja entrar uno
- WHEN se arma el prompt y se hace flush
- THEN sólo el que quedó en el prompt sube `access_count`

**Tests:** `recall_leaves_out_hits_that_did_not_score`, `a_hit_that_did_not_score_brings_no_conflict`, `a_memory_hit_is_recorded_by_the_access_tracker`, `a_hit_from_another_table_does_not_touch_the_memory_row_with_the_same_id` y `a_hit_cut_by_the_budget_is_not_counted_as_accessed` (`prompt_builder.rs`, mod tests).

## Cliente HTTP y herramienta `search_memory`

`DaemonClient::search` MUST mandar `min_relative_score = RELEVANCE_RATIO` junto con los campos de `SearchArgs` (`search_request`, `crates/clawg-cli/src/client.rs:544`; uso en `:715`), y devolver los hits con `withheld` (`Found`, `client.rs:151-159`), leído con `#[serde(default)]`: un daemon anterior no lo manda y vale 0. El puente MUST volver a aplicar el corte al renderizar `search_memory` (`relevant_hits` y `format_search`, `crates/clawg-cli/src/mcp/tools.rs:65` y `:79`), con el mismo `shown_hits` que el daemon, así que un hit exacto se muestra aunque el corte deje fuera a otro. `Hit` del cliente MUST leer `exact` con `#[serde(default)]`: un hit sin el campo vale `false`. Aplicar el corte dos veces da el mismo resultado, y contra un daemon que ignora el campo el agente ve lo mismo.

Cuando no queda ninguna línea, `search_memory` MUST responder la frase de abstención si algún hit se ocultó, en el daemon o en el puente, con `N` los hits que devolvió el daemon más su `withheld` (`format_search`, `tools.rs:82-84`), y `no memories found` si no se ocultó ninguno (`format_hits`, `tools.rs:98-102`).

#### Escenario: cuerpo del pedido
- WHEN el cliente arma el cuerpo de una búsqueda
- THEN el cuerpo contiene `"min_relative_score": 0.35` además de los campos de `SearchArgs`

#### Escenario: render
- GIVEN hits con scores 0,50, 0,10 y 0,05
- THEN `search_memory` muestra los dos primeros y no el tercero

#### Escenario: nada relevante en la búsqueda
- GIVEN un daemon que respondió `hits: []` y `withheld: 3`
- THEN `search_memory` dice `no relevant memories for this query: 3 searched, none close enough to show`

#### Escenario: alcance vacío
- GIVEN `hits: []` y `withheld: 0`
- THEN `search_memory` dice `no memories found`

**Tests:** `search_request_carries_the_relevance_ratio` y `a_search_answer_without_withheld_still_parses` (`client.rs`, mod tests); `relevant_hits_keeps_the_ones_the_cut_lets_through_in_order`, `the_search_output_shows_only_the_relevant_hits`, `search_says_when_nothing_was_relevant` y `nothing_stored_is_not_called_irrelevant` (`tools.rs`, mod tests); `search_memory_tells_nothing_relevant_from_nothing_stored` y `search_with_nothing_stored_says_so_without_failing` (`crates/clawg-cli/src/mcp/server.rs`).

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

Para un hit marcado como match de identificador (`RawHit.lexical_match`, `score.rs:30`), `score()` (`score.rs:262`) MUST valer `(cos' + lexical_bonus) × max(piso_tier, decaimiento) × importancia × estado`, donde `cos'` es el coseno ya re-escalado contra el piso (la suma en `score.rs:275`). Para un hit sin match, MUST valer lo mismo que sin el bono.

#### Escenario: el bono se suma después del piso
- GIVEN un hit match con coseno bajo el piso (cos' = 0), tier 1, edad 0, importancia 1 y estado `active`
- THEN el score es `lexical_bonus × importancia`, mayor que 0

#### Escenario: sin match, sin cambio
- GIVEN el mismo hit sin match
- THEN el score es 0

**Test:** `a_lexical_match_adds_the_bonus_after_the_floor`, `without_a_match_the_score_is_unchanged` y `a_match_outranks_a_closer_row_with_the_same_decay_and_importance` en `score.rs`; el literal, en `scoring_config_default_pins_the_shipped_ranking_literals`.

## El piso de coseno

El coseno de cada hit MUST re-escalarse contra `ScoringConfig::DEFAULT.cosine_floor`, que vale 0,79 (`score.rs:185`): `cos' = clamp((cos − piso) / (1 − piso), 0, 1)` (`rescale_cosine`, `score.rs:217`; aplicado en `score()`, `score.rs:269`). Un coseno bajo el piso vale 0 y el piso es el cero del término de similitud; sin la re-escala, todos los cosenos caerían en una banda angosta y la similitud casi no pesaría frente a la importancia.

El valor está atado a la distribución de cosenos del embedder: al pooling mean del modelo (`multilingual-e5-small/mean`, [`specs/storage`](../storage/spec.md)) y a los prefijos de consulta y pasaje que el daemon manda hoy. Cambiar cualquiera de los dos mueve toda la distribución y obliga a medirlo de nuevo.

**Cómo se midió.** Sobre el golden set de memorias (`docs/eval/golden.json`, 46 casos), con pooling mean, el 2026-10-15: un barrido del piso de 0,70 a 0,83 da el mejor MRR (0,5524), nDCG@10 (0,6215) y R@10 (0,8478) en 0,79, así que 0,79 queda.

**Tests:** `scoring_config_default_pins_the_shipped_ranking_literals` (fija el literal 0,79; `score.rs`), y `rescale_cosine_maps_the_floor_to_zero`, `rescale_cosine_maps_one_to_one_regardless_of_floor`, `rescale_cosine_clamps_a_cosine_below_the_floor_to_zero` y `rescale_cosine_is_monotone_non_decreasing_for_any_floor` (`score.rs`).

## La búsqueda premia el identificador exacto

Con el canal léxico encendido y texto de consulta, `search_all_memories` MUST marcar como match de identificador cada fila que lo sea, venga del canal denso (`knn_one`, `search.rs:486`) o del léxico (`score_lexical_candidates`, `search.rs:1089`), y MUST sumar los matches a los candidatos léxicos aunque estén fuera del top denso o pasado el token 16. Los matches se calculan antes del `try_join!` de los canales (`search.rs:1221`).

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

Un match de identificador MUST cumplir los mismos predicados que el candidato léxico: agente (el llamador y los autores de su alcance, [`specs/sharing`](../sharing/spec.md) R2), proyecto (con la excepción de `user_preference`), estado `active`/`disputed` y `deleted_at IS NULL` (`IDENTIFIER_MATCH_SQL`, `search.rs:864`). Una fila de un autor fuera de su alcance, borrada o de otro proyecto MUST NOT recibir el bono ni contar para el presupuesto.

#### Escenario: fila de otro agente
- GIVEN 21 filas de otro agente que el llamador no lee y una del llamador con `getUser42`
- WHEN el llamador busca `getUser42` con presupuesto 20
- THEN su fila lleva el bono

**Test:** `rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget` en `search.rs`.

#### Escenario: palabras partidas como el identificador
- GIVEN 25 filas con `snake case` o `snake-case` y una con `snake_case`, y presupuesto 20
- WHEN se busca `snake_case`
- THEN sólo la fila con `snake_case` es match

**Test:** `an_underscore_identifier_is_held_only_as_written` y `words_split_like_an_underscore_identifier_neither_match_nor_use_the_budget` en `search.rs`.

## La búsqueda no falla por el canal de identificadores

Un error de la consulta de matches (FTS ausente, MATCH rechazado, conexión cerrada) MUST dejar la búsqueda como si no hubiera matches, igual que los candidatos léxicos (`search.rs:918-921`, junto a `:808-811`).

**Test:** `a_failing_identifier_query_leaves_the_search_unboosted` en `search.rs`.

## Las filas que dejó la app de escritorio siguen alcanzables

Ningún código de producción escribe `episodic_trajectory` ni `procedural_skill`, pero una base puede traer filas de la app de escritorio. Esas filas MUST seguir alcanzables: la búsqueda MUST consultarlas (KNN de 5 + 5), el workspace MUST mostrar los skills del proyecto, y `list_memories`, `GET`, `PATCH` y `DELETE` MUST aceptarlas. El re-embed de `procedural_skill` MUST embeber `"{trigger} {skill_name}"`.

#### Escenario: filas como las escribía la app de escritorio
- GIVEN una trayectoria y un skill escritos con los mismos escritores que usaba la app (que quedan sólo para tests), y sus vectores
- WHEN se busca con el vector del skill, se regenera el workspace del proyecto, se lista y se re-embebe
- THEN el skill está entre los hits y en el workspace, las dos filas están en el listado, y el re-embed del skill embebe `"when login fails Fix login flow"` para `trigger = "when login fails"` y `skill_name = "Fix login flow"`

**Test:** `episodic_trajectory_retrievable_by_same_agent_id` y `knn_procedural_returns_skill_rows` (`search.rs`), `regenerate_reports_the_ids_it_rendered` y `workspace_top_skills_exclude_foreign_project` (`workspace.rs`), `list_memories_kind_none_merges_all_tables_sorted_desc` (`admin.rs`) y `reembed_reinfer_uses_passage_role_and_shared_skill_embedding_text` (`maintenance.rs`, fija el texto literal). La apertura de una base en V9 con esas filas está en [`specs/synthesis`](../synthesis/spec.md) (su R9).

## Hits de otros agentes

La búsqueda y el contexto traen, además de lo propio, las filas de los autores que la política de [`specs/sharing`](../sharing/spec.md) deja leer al llamador: `POST /v1/search` y `POST /v1/context` resuelven el alcance del agente del pedido en cada llamada (`reach`, `crates/clawg-cli/src/daemon/api.rs:1203`; usos en `api.rs:1171`, `:1245` y `:1254`). Sobre eso:

- Cada hit de `POST /v1/search` MUST llevar su autor como `agent_id` (`RankedHit.agent_id`, `search.rs:119`); el cliente lo lee con `#[serde(default)]` y un hit sin autor cuenta como propio (`client.rs:175`).
- El workspace, ACTIVE CONFLICTS, RECENT del contexto sin consulta, el listado, la síntesis y el detector de contradicciones al guardar MUST seguir viendo sólo lo del agente: sólo los hits de búsqueda y el resumen de [LAST SESSION](#la-última-sesión) llegan a filas ajenas.
- Un hit ajeno MUST contar como acceso igual que uno propio (`api.rs:1192`).

**Test:** `a_hit_carries_its_author` (`search.rs`), `search_returns_the_author_of_each_hit` y `search_and_context_follow_the_sharing_policy_from_the_next_call` (`crates/clawg-cli/tests/api.rs`), `the_workspace_and_conflicts_stay_own` (`prompt_builder.rs`) y `contradiction_probe_ignores_shared_authors` (`contradiction.rs`).

## Línea de hit en `recall_context`

Cada hit, y el resumen de LAST SESSION, MUST imprimirse como un encabezado de una línea seguido del texto en la misma línea (`format_hit_line`, `prompt_builder.rs:628`):

- `kind = memory`: `[YYYY-MM-DD #id tipo] texto`
- otro kind: `[YYYY-MM-DD #id kind/tipo] texto`, con `kind` en minúsculas (`episodic`, `procedural`). Para esos kinds la búsqueda pone el tipo fijo `episode` o `procedure`.
- un hit de otro agente lleva ` by <autor>` antes del `]` (`[YYYY-MM-DD #id tipo by <autor>] texto`), y si hay alguno, contando el resumen de LAST SESSION, el prompt termina con un bloque `=== SHARED ===` con una nota (`render`, `prompt_builder.rs:537-543`): [`specs/sharing`](../sharing/spec.md) R4. Sin hits ajenos, el prompt es el de siempre.

Si el texto tiene varias líneas, las siguientes MUST seguir tal cual (líneas en blanco y código incluidos) hasta el próximo encabezado de hit o separador `=== `. No se imprime `SOURCE:`, `TYPE:` ni `CONTENT:`. El `#id` es el que aceptan `delete_memory` y `update_memory`, con el kind de la línea (`memory` si no tiene).

#### Escenario: memory
- GIVEN un hit memory id 57, tipo `semantic_fact`, creado el 2026-10-10, texto `usar sqlite`
- THEN el prompt contiene la línea `[2026-10-10 #57 semantic_fact] usar sqlite`

#### Escenario: otro kind
- GIVEN un hit procedural id 3, tipo `procedure`
- THEN su línea empieza con `[<fecha> #3 procedural/procedure] `

#### Escenario: texto multilínea
- GIVEN un hit con texto `fn main() {\n    run();\n\n    stop();\n}` seguido de otro hit
- THEN las 5 líneas, la vacía incluida, siguen al encabezado y el segundo encabezado las cierra

**Test:** `hit_line_is_one_header_with_id_and_text` y `format_hit_line_drops_the_old_labels` en `prompt_builder.rs`; para el hit ajeno, `a_shared_hit_names_its_author` y `the_shared_note_appears_only_with_shared_hits` (`prompt_builder.rs`).

## Conflictos por referencia

Cada par MUST imprimir los ids de sus dos lados como `#id`: son los valores que `resolve_conflict` recibe como `keep` y `other` (`ResolveConflictArgs`, `crates/clawg-cli/src/mcp/server.rs:347`), y los dos son ids de `memory`.

Un lado que es hit mostrado MUST imprimirse sólo como `#id`. Un lado que no es hit mostrado (fila del workspace, o fila fuera del prompt) MUST imprimirse con su texto completo bajo el encabezado `[YYYY-MM-DD #id]` la primera vez que aparece en el bloque, y sólo como `#id` las siguientes (`format_conflict_block`, `prompt_builder.rs:671`). Así el texto de una fila aparece a lo sumo una vez en el bloque aunque esté en varios pares: en una cadena de disputas, la fila del medio es nueva en un par y vieja en otro. Un par con ambos lados mostrados MUST NOT imprimir ningún texto. `importance`, `tier` y `status` no se imprimen. Un lado cuyo hit cortó el presupuesto no es hit mostrado: MUST llevar su texto, nunca un `#id` que el prompt no imprime (el bloque cita por las posiciones de los hits que quedaron, `render`, `prompt_builder.rs:535`).

Formato de un grupo:

```
NEW #N                      | NEW [YYYY-MM-DD #N disputed] texto
OLD #M                      | OLD [YYYY-MM-DD #M] texto
```

#### Escenario: lado mostrado
- GIVEN el par (old 1, new 2), con 2 como hit en RECENT y 1 sin mostrar
- THEN el bloque contiene `NEW #2` y `OLD [<fecha> #1] <texto de 1>`, y el texto de 2 aparece una sola vez en todo el prompt

#### Escenario: ambos mostrados
- GIVEN old 1 y new 2 como hits
- THEN el bloque tiene `NEW #2` y `OLD #1` y ninguno de los dos textos se repite

#### Escenario: workspace
- GIVEN old 1 es una fila del workspace y new 2 un hit
- THEN `OLD [<fecha> #1] <texto>` lleva el texto, aunque el bullet del workspace ya lo muestre: el bullet no tiene id

#### Escenario: cadena
- GIVEN los pares (old 2, new 3), (old 1, new 3) y (old 1, new 2), ninguno con hits
- THEN el bloque es `NEW [<fecha> #3 disputed] <texto de 3>`, `OLD [<fecha> #2] <texto de 2>`, `OLD [<fecha> #1] <texto de 1>`, `NEW #2`, `OLD #1`

#### Escenario: lado cortado por el presupuesto
- GIVEN el par (old 2, new 1), con 1 como hit mostrado y 2 como hit candidato que el presupuesto dejó afuera
- THEN el bloque contiene `NEW #1` y `OLD [<fecha> #2] <texto de 2>`, no `OLD #2`

**Test:** `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest`, `a_row_in_several_pairs_prints_its_text_once` y `e2e_full_4_block_output` (la fila mostrada aparece una vez y el bloque la cita como `OLD #id`) en `prompt_builder.rs`; el lado cortado, `a_conflict_on_a_cut_hit_carries_its_text` (`prompt_builder.rs`).

## Conflictos agrupados por fila nueva

Los pares que comparten `new_id` MUST formar un grupo: una línea `NEW`, y una línea `OLD` por cada vieja. El texto de la nueva MUST aparecer a lo sumo una vez en el bloque.

#### Escenario
- GIVEN new 9 disputa a las viejas 1, 2 y 3, y 9 no es hit
- THEN `[<fecha> #9 disputed] <texto>` aparece una vez, seguido de tres líneas `OLD`

**Test:** `a_new_row_disputing_several_old_rows_prints_its_text_once` y `a_row_in_several_pairs_prints_its_text_once` en `prompt_builder.rs`.

## Una NOTE por bloque de conflictos

La NOTE MUST imprimirse una sola vez por bloque, justo bajo el separador y antes de los grupos (`CONFLICT_NOTE`, `prompt_builder.rs:643-646`). MUST estar en inglés, como todo lo que el agente lee de clawg, y empezar con `NOTE:`. MUST NOT afirmar que las memorias se contradicen: el detector mide cercanía ([`specs/disputes`](../disputes/spec.md)). MUST pedir que se pregunte al usuario y nombrar `resolve_conflict` con sus veredictos y el significado de `keep`. Se imprime en cada bloque, así que MUST NOT pasar de los 228 bytes de la NOTE que no nombraba veredictos:

```
NOTE: these memories may conflict; the user has not said which holds.
Before a decision that depends on them, ask the user.
Then call resolve_conflict: verdict supersedes with keep = the one that holds, or compatible or scoped.
```

#### Escenario
- GIVEN 3 grupos
- THEN `NOTE:` aparece exactamente una vez, y `resolve_conflict` también

**Test:** `conflict_block_has_one_note`, `the_conflict_note_names_the_tool`, `the_conflict_note_is_in_english`, `the_conflict_note_does_not_claim_a_contradiction` y `the_conflict_note_is_no_longer_than_the_one_without_verdicts` en `prompt_builder.rs`.

## Orden y tope de los conflictos

Una función pura sobre los pares leídos (`rank_conflicts`, `prompt_builder.rs:652`) MUST ordenarlos por clave `(rank, new_id desc, old_id asc)`:

- `rank` = menor posición de hit entre sus lados que son hits candidatos de `memory` (las posiciones se toman antes del presupuesto, `prompt_builder.rs:324` y `:333`);
- los pares sin ningún hit candidato (sólo tocan una fila del workspace) MUST ir al final, con `rank` = infinito.

El resultado MUST NOT depender del orden de entrada. Los grupos salen en orden de su primer par.

Tras ordenar, el bloque MUST conservar los primeros 5 pares (`MAX_CONFLICT_PAIRS`, `prompt_builder.rs:49`; el tope cuenta pares, antes de agrupar). Si quedan N > 0 afuera, la última línea del bloque MUST ser `(+N more conflicts not shown)` (`prompt_builder.rs:700-702`). Con N = 0 MUST NOT haber esa línea. Los pares que el presupuesto quita después ([El prompt cumple el presupuesto](#el-prompt-cumple-el-presupuesto)) se suman a N.

#### Escenario: posición
- GIVEN un par cuyo hit está en posición 0 y otro en posición 4
- THEN el primero va antes, aunque su `new_id` sea menor

#### Escenario: sólo workspace
- GIVEN un par sólo de workspace con `new_id` 99 y otro con un hit en posición 9
- THEN el de workspace va último

#### Escenario: estable
- GIVEN los mismos pares en dos permutaciones de entrada
- THEN la salida es idéntica

#### Escenario: tope
- GIVEN 8 pares
- THEN hay 5 pares impresos y la línea `(+3 more conflicts not shown)`; con 5 pares no hay línea de omitidos

**Test:** `conflict_pairs_rank_by_best_hit_position_workspace_only_last`, `conflict_pair_order_ignores_input_order` y `conflict_block_keeps_five_pairs_and_counts_the_rest` en `prompt_builder.rs`.

## Workspace en el prompt

El workspace entregado al modelo (`workspace_for_prompt`, `prompt_builder.rs:905`) MUST NOT contener `<!-- `, ni sección USER vacía, ni un encabezado `## ` de la sección AUTO sin bullets debajo. El contenido no vacío de USER MUST conservarse tal cual lo escribió el usuario, encabezados incluidos, sin marcadores y sin líneas en blanco al principio ni al final: la poda de encabezados sólo mira la sección AUTO, que es la que genera `render_rows` (`workspace.rs:448`). Un archivo sin `AUTO_START` MUST entregarse entero; uno con AUTO pero sin `AUTO-END` se toma como AUTO hasta el final, y el texto antes de `AUTO-START` se entrega. Si no queda nada, no se emite la sección WORKSPACE. El archivo en disco MUST ser byte a byte idéntico antes y después de un recall.

#### Escenario: sin marcadores
- GIVEN un archivo con AUTO/USER, un bullet bajo `## User Preferences` y los otros dos encabezados vacíos
- THEN el prompt contiene el bullet y su encabezado, y ni `<!--` ni los otros encabezados

#### Escenario: USER con contenido
- GIVEN texto del usuario entre USER-START y USER-END
- THEN aparece en el prompt sin marcadores

#### Escenario: archivo plano
- GIVEN un archivo sin marcadores
- THEN se entrega tal cual

**Test:** `workspace_in_prompt_has_no_markers_or_empty_headings`, `workspace_user_section_content_is_kept_without_markers` y `workspace_file_bytes_are_unchanged_by_rendering_the_prompt` en `prompt_builder.rs`; el archivo plano, `embedder_none_emits_workspace_only_if_present`.

## El contexto sin consulta

Un `POST /v1/context` sin `query`, o con una que sólo tiene espacios, MUST NOT embeber ni buscar, así que no necesita el modelo (`ContextRequest.query` es opcional, `crates/clawg-cli/src/daemon/api.rs:396-400`; el handler decide con `filter(|q| !q.trim().is_empty())`, `api.rs:1237`, y llama a `build_session_prompt`, `crates/clawg-core/src/memory/prompt_builder.rs:272`, en `api.rs:1255`). Es el contexto con el que empieza una sesión, cuando todavía no hay una pregunta. Su prompt MUST llevar el workspace, el resumen de [LAST SESSION](#la-última-sesión) y, bajo RECENT, las filas vivas (`active` o `disputed`, sin `deleted_at`) del agente que pide, creadas en los últimos siete días, con el predicado de proyecto de la búsqueda: las del proyecto pedido, las que no tienen proyecto y las `user_preference` de cualquier proyecto (`RECENT_ROWS_SQL`, `prompt_builder.rs:804-817`, el mismo predicado que `crates/clawg-core/src/memory/search.rs:241`). Quedan afuera las filas que el workspace ya imprimió (`EmittedIds`) y el save del resumen de LAST SESSION. Va de la más nueva a la más vieja (`created_at DESC, id ASC`), hasta `K_FINAL` saves (diez, `prompt_builder.rs:47`), una línea por save.

Cada save largo MUST contarse una vez, por su pedazo de menor posición, que es el que nombra el tema (`one_row_per_save`, `prompt_builder.rs:857`). Como los pedazos son filas de `memory`, la consulta trae como mucho `RECENT_ROW_POOL` (200, `prompt_builder.rs:819`) filas y las junta después. Los pedazos de un save comparten `created_at` y se insertan en orden, así que `id ASC` dentro de un mismo instante hace que el corte del pool nunca deje a un save sin su apertura. No lleva LONG-TERM: sin consulta no hay nada que sea relevante, sólo nuevo. La última sesión, los conflictos activos y el presupuesto MUST ser los de un contexto con consulta (`with_last_session` y `assemble`, `prompt_builder.rs:288-289`).

`retrieval` MUST ser `{"state": "recent", "hits": N}`, con `N` los saves que usó el prompt antes de sacar el del resumen (`Retrieval::Recent`, `prompt_builder.rs:287`; `RetrievalState::Recent`, `api.rs:632`). MUST NOT registrar accesos: esas filas se eligieron por ser nuevas y no por ser relevantes, y contarlas en cada inicio de sesión promovería una fila por su edad (`build_session_prompt` no recibe un `AccessTracker`). Cómo lee el puente el estado `recent` está en [`specs/mcp`](../mcp/spec.md), y el hook que pide este contexto al iniciar la sesión, en [`specs/hooks`](../hooks/spec.md).

Queda abierto que RECENT muestre menos de diez saves cuando los diez más nuevos suman más de 200 filas: el pool corta por filas y no por saves. Un arreglo posible es juntar los pedazos en SQL, por `memory_piece`, antes del `LIMIT` ([verify](../../changes/archive/2026-10-18-agent-lifecycle-hooks/verify.md#lo-que-encontró-la-verificación), W2).

#### Escenario: la sesión empieza
- GIVEN un agente con un save de ayer y otro de hace un mes en el proyecto, y un daemon sin embedder
- WHEN pide el contexto sin consulta
- THEN el prompt trae el de ayer bajo RECENT, no trae el de hace un mes, y `retrieval.state` es `recent`

#### Escenario: un save largo
- GIVEN un save de más de una pieza y el pool de 200 filas cortado dentro de sus pedazos
- WHEN se pide el contexto sin consulta
- THEN el save sale una vez, por su apertura

#### Escenario: una consulta en blanco
- GIVEN `recall_context` con `query` igual a `"   "`
- THEN el daemon arma el contexto sin consulta, sin embeber y sin registrar accesos

**Tests:** `a_context_without_a_query_lists_the_recent_saves_by_date`, `a_context_without_a_query_shows_a_long_save_once_by_its_opening`, `a_long_save_the_row_pool_cuts_is_still_shown_by_its_opening`, `a_context_without_a_query_reads_only_its_own_agent` y `a_context_without_a_query_skips_what_the_workspace_showed_and_the_dead` (`prompt_builder.rs`); `context_without_a_query_does_not_need_the_embedder` (`crates/clawg-cli/tests/api.rs`). Ningún test fija el tope de diez saves, que una consulta en blanco tome este camino, el predicado de proyecto con una `user_preference` de otro proyecto ni que no se registren accesos (esto último lo garantiza la firma de `build_session_prompt`).

## La última sesión

Todo prompt de `POST /v1/context`, con consulta o sin ella, MUST llevar después del workspace y antes de RECENT un bloque `=== LAST SESSION ===` (`SEP_LAST_SESSION`, `prompt_builder.rs:25`; el orden de los bloques en `render`, `:515`) con el último resumen visible, si existe: el `session_summary` vivo (`active` o `disputed`, sin `deleted_at`) más nuevo del proyecto pedido, o de las filas sin proyecto si no se pide ninguno, entre los que el lector puede leer: propios, de los autores que su alcance admite o sin autor ([`specs/sharing`](../sharing/spec.md)). La consulta los trae del más nuevo al más viejo (`LAST_SESSION_SQL`, `prompt_builder.rs:709-717`, `ORDER BY created_at DESC, id ASC`) y `fetch_last_session` (`:759`) se queda con el primero cuyo autor pasa `Reach::admits` (`:773`; `crates/clawg-core/src/memory/sharing.rs:55`), el mismo criterio que la búsqueda. Los pedazos de un save comparten `created_at`, así que de un save largo la consulta trae primero su primer pedazo; el bloque MUST mostrar el save entero, sus pedazos visibles unidos como los devuelve `GET /v1/memories/{kind}/{id}/save` (`read_save_core`, `crates/clawg-core/src/memory/pieces.rs:98`; en `with_last_session`, `prompt_builder.rs:745-756`), sin etiqueta `piece=`, y con el id del pedazo elegido. Un resumen sigue el protocolo y termina en «Next steps» y «Relevant files», justo lo que su primer pedazo dejaría afuera. Si el save no se puede leer, el bloque MUST mostrar ese pedazo con su etiqueta `piece=1/N`. Lo que no entra lo acorta el presupuesto ([«El prompt cumple el presupuesto»](#el-prompt-cumple-el-presupuesto)).

- **Proyecto exacto:** un contexto con proyecto MUST NOT traer un resumen sin proyecto, que no es la sesión anterior de ese proyecto; uno sin proyecto trae sólo los que no lo tienen.
- **Sin consulta ni modelo ni umbral de antigüedad:** el bloque no depende de la pregunta, así que llega también al contexto sin consulta y cuando las memorias no se buscaron. La línea lleva su fecha y el agente la juzga: un umbral escondería el único resumen de un proyecto retomado después de meses.
- **La línea es la de un hit** (`format_hit_line`, `prompt_builder.rs:628`), con ` by <autor>` cuando el resumen es de otro agente; entonces el prompt suma la nota `SHARED` como con cualquier hit ajeno (`render`, `:537`). Lo que más sirve es justamente el resumen de la sesión anterior aunque la haya hecho otro agente en el mismo proyecto.
- **Sin accesos:** el bloque MUST NOT registrar accesos (`Fitted.shown`, `prompt_builder.rs:380`, no lo incluye): se eligió por ser el último y no por relevante, y contarlo en cada contexto lo promovería de tier por su edad.
- **Sin repetirse:** si el resumen, u otro pedazo de su save, es también un hit, MUST NOT aparecer en RECENT ni en LONG-TERM (`with_last_session`, `prompt_builder.rs:723`; el filtro, `:738-744`). El workspace no hace falta mirarlo: nunca imprime un `session_summary`.
- **Fuera de los conflictos:** no se suma a las filas cuyos pares se buscan ni a las posiciones de los hits (`assemble`, `:305`; `positions`, `:494`).

Sin resumen visible MUST NOT haber bloque. Si la consulta falla, el prompt sale sin el bloque y la falla sólo queda en el log (`prompt_builder.rs:730-736`).

#### Escenario: la sesión siguiente
- GIVEN dos `session_summary` de `a` en `p`, de ayer y de hace un mes
- WHEN `a` pide el contexto de `p` con una consulta que no se les parece
- THEN el prompt trae el de ayer bajo LAST SESSION y no el de hace un mes

#### Escenario: otro agente
- GIVEN el resumen más nuevo de `p` guardado por `b`
- WHEN `a` pide el contexto y `sharing.json` no oculta `b` a `a`
- THEN el bloque lo trae marcado `by b`; si lo oculta, el bloque trae el de `a` o no aparece

#### Escenario: un resumen largo
- GIVEN un `session_summary` de dos pedazos, el segundo con «Next steps» y «Relevant files»
- WHEN se pide el contexto
- THEN el bloque trae los dos pedazos unidos bajo el id del primero, y el segundo no aparece como hit

**Tests:** `the_last_session_block_shows_the_newest_summary` (el de otro proyecto, el sin proyecto, el borrado y el más viejo quedan fuera; el bloque va antes de RECENT), `the_last_session_block_shows_a_long_summary_whole`, `the_last_session_block_respects_sharing`, `the_last_session_block_is_not_repeated_as_a_hit`, `a_context_without_a_query_opens_with_the_last_session` (también con un resumen de hace un mes) en `prompt_builder.rs`; `a_reach_admits_the_reader_and_whoever_is_not_hidden` en `sharing.rs`. Con Claude Code sobre el perfil dev, cada sesión nueva abrió con el resumen de la anterior, y `recall_context` con una consulta que no se le parecía lo trajo igual ([verify](../../changes/archive/2026-10-19-session-handoff/verify.md#claude-code-sobre-el-perfil-dev-2026-10-19)).

## El prompt cumple el presupuesto

`build_memory_prompt` y `build_session_prompt` reciben el presupuesto del contexto y arman el texto con `fit_to_budget` (`prompt_builder.rs:404`), una función pura sobre el workspace, el último resumen, los hits candidatos y los pares ya ordenados. Si el texto entero entra, MUST ser byte a byte el que se arma sin presupuesto. Si no entra, MUST quitar de a uno, midiendo otra vez el texto entero después de cada paso, primero los hits de RECENT y LONG-TERM, después el resumen de LAST SESSION y después los pares de conflictos desde el último, hasta que entre (`:417-421`). Entre los hits va primero el de menor score, sea del bloque que sea, y un hit exacto sólo cuando no queda ningún otro (`next_to_cut`, `prompt_builder.rs:467`); un empate se resuelve desde el final de LONG-TERM y después desde el final de RECENT, así que un contexto sin consulta, cuyos hits valen 0, se sigue cortando desde su save más viejo. Los bloques dicen la edad de un hit y no su relevancia: cortar un bloque entero primero quitaba un hit exacto de la semana pasada antes que uno de RECENT que apenas pasó el piso, y la búsqueda nunca corta un exacto ([«Definiciones»](#definiciones), corte relativo). El resumen queda por encima de los hits porque ninguna consulta lo trae de vuelta, y cede ante los conflictos porque esos piden una pregunta al usuario. El texto MUST NOT superar el presupuesto salvo en el caso de [un workspace más grande que el presupuesto](#un-workspace-más-grande-que-el-presupuesto-se-entrega-y-se-dice).

El workspace MUST NOT recortarse: sus preferencias y su sección USER son el acuerdo con el usuario, y cada fila que imprimió es una que la búsqueda salteó (`EmittedIds`), así que cortarla la dejaría sin ningún canal. Medir el texto entero en cada paso cuenta la nota `SHARED`, el bloque `BUDGET` y el texto que gana un conflicto cuando se va su hit.

#### Escenario: muchas filas largas
- GIVEN diez hits de 2.000 bytes y un presupuesto de 8.000
- THEN el prompt ocupa como mucho 8.000 bytes, y los hits que quedan son los primeros en orden de prioridad

#### Escenario: sin recorte
- GIVEN un contexto de 3.000 bytes y el presupuesto default
- THEN el prompt es byte a byte el que se arma sin presupuesto, sin bloque `BUDGET`

#### Escenario: un exacto de la semana pasada
- GIVEN un hit exacto en LONG-TERM con score 0,01, dos hits de RECENT y un presupuesto que deja dos hits
- THEN se corta un hit de RECENT y el exacto queda entero; con un presupuesto que deja uno solo, el que queda es el exacto

**Tests:** `the_prompt_never_exceeds_the_budget`, `the_cut_starts_at_the_end_of_long_term` (el empate), `the_cut_takes_the_lowest_score_first_across_recent_and_long_term`, `an_exact_hit_is_cut_only_when_no_other_hit_is_left`, `a_context_that_fits_is_left_as_it_was`, `conflicts_go_after_every_hit`, `the_last_session_block_gives_way_after_the_hits` (que además comprueba que el resumen no entra en `Fitted.shown`) y, con el store, `a_store_of_long_rows_fits_the_budget` (`prompt_builder.rs`). Que sin recorte el prompt sea el de antes lo siguen probando `e2e_full_4_block_output` y los tests de bloques, que corren con el default.

## El primer hit de un bloque se recorta en lugar de saltarse

Cuando el hit a quitar es el único que queda en su bloque, su texto MUST acortarse en lo que sobra, en un límite de carácter UTF-8, sin espacio al final y terminado en ` [truncated]` (`shorten`, `prompt_builder.rs:479`). LAST SESSION tiene un solo hit, así que el resumen siempre se acorta antes de irse. Si volver a medir lo deja todavía por encima, se acorta otra vez sin repetir la marca. Sólo si no le queda texto MUST quitarse. Un hit recortado sigue siendo hit mostrado: un hit acortado y su id dicen más que un bloque vacío.

**Tests:** `the_first_hit_of_a_block_is_shortened_not_dropped` y `a_shortened_text_ends_on_a_character_boundary` (`prompt_builder.rs`).

## Lo cortado se nombra por id

Si algo se cortó o recortó, el prompt MUST terminar con un bloque `=== BUDGET ===` (`render`, `prompt_builder.rs:544-552`; su texto en `format_cut`, `:559`) que dice el presupuesto, cuántos bytes ocupa el contexto entero, los ids cortados en el orden en que se imprimirían y los recortados, el resumen de LAST SESSION entre ellos como un hit más, con su kind delante cuando no es `memory` (`episodic #3`), y que `read_memory` los devuelve enteros:

```
=== BUDGET ===
Cut to fit 8000 bytes; the full context is 12040 bytes. Left out: #40, #41. Shortened: #12. read_memory returns any of them whole.
```

Sin recorte el bloque MUST NOT aparecer. El bloque MUST NOT quitarse para entrar.

#### Escenario: pedir lo que quedó afuera
- GIVEN un contexto que cortó el hit #40
- WHEN el agente llama `read_memory` con 40
- THEN recibe su texto entero

**Tests:** `what_the_budget_left_out_is_named_by_id` y `the_budget_block_stays_with_the_smallest_budget` (`prompt_builder.rs`); `a_hit_cut_by_the_budget_is_readable_whole` (`crates/clawg-cli/src/mcp/server.rs`).

## Un workspace más grande que el presupuesto se entrega y se dice

Si sin ningún hit, resumen ni par el texto sigue por encima del presupuesto, el prompt MUST llevar el workspace entero, ningún hit y el bloque `BUDGET` con cuántos bytes ocupa el contexto entero y la frase `The workspace alone is over the budget and is shown whole.` (`prompt_builder.rs:423-426` y `:576-578`). Es el único caso en que el prompt supera el presupuesto.

**Test:** `a_workspace_over_the_budget_is_kept_and_said` (`prompt_builder.rs`).

## El parser de `rag-eval --agent-view`

`context_segments` (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs:84`) MUST segmentar por sección (`context_sections`, `parse.rs:67`): los segmentos de hit salen sólo de LAST SESSION, RECENT CONTEXT y LONG-TERM KNOWLEDGE (`parse.rs:89`), así que un `session_summary` esperado que pasa a LAST SESSION sigue contando como encontrado; los bullets, sólo de WORKSPACE; el texto de ACTIVE CONFLICTS, SHARED y BUDGET MUST NOT contar para `found`. Encabezado de hit = línea que cumple `[` + `YYYY-MM-DD` + ` #` + dígitos + (` ` + palabra sin espacios)? + `]` + ` ` (`hit_text`, `parse.rs:137`); el texto es lo que sigue en la misma línea, y el segmento corre hasta el próximo encabezado o separador. Un marcador `<!--` cierra un bullet.

La salida de `search_memory` MUST leerse como cero líneas cuando es `no memories found` o empieza con la frase de abstención (`parse_search_output`, `parse.rs:33-36`), así que un caso `unanswerable` cuya búsqueda la dice cuenta como `abstained`.

#### Escenario: multilínea
- GIVEN dos hits, el primero con código y línea en blanco
- THEN hay 2 segmentos, el primero completo

#### Escenario: conflictos
- GIVEN una sección ACTIVE CONFLICTS con `NEW [..#2 disputed] x` y `OLD [..#1] y`
- THEN no se genera ningún segmento

**Test:** `a_hit_line_is_one_segment`, `a_hit_of_another_kind_is_one_segment`, `a_multi_line_hit_runs_until_the_next_header`, `a_hit_ends_at_the_next_section_separator`, `a_hit_may_open_with_a_dash`, `conflict_text_yields_no_segments`, `workspace_markers_end_a_bullet`, `the_abstention_sentence_parses_to_no_lines` y `the_last_session_block_counts_as_shown` en `parse.rs`; `read_context_finds_rows_in_hit_lines_and_workspace_bullets` en `agent_view/mod.rs`. El punta a punta del agent-view (`runner.rs`) arma un prompt real y lo lee con este parser.

## Los ids de `recall_context` en `delete_memory` y `update_memory`

La doc de `DeleteMemoryArgs.id` (`server.rs:283-286`), la de `UpdateMemoryArgs.id` (`server.rs:306-309`) y la descripción de `delete_memory` (`server.rs:1106-1111`) MUST nombrar `recall_context` además de `search_memory` y `list_memories`, y decir que un hit sin kind explícito es `memory`. `kind` sigue siendo obligatorio en las dos.

**Test:** `memory_id_docs_name_recall_context_and_the_default_kind` en `server.rs`.

## Medición del contexto

`rag-eval --real --agent-view` sobre `docs/eval/golden.json` (sha256 `d2d9d732…ed792a`, n = 46), medido en Windows el 2026-10-14 sobre `d175348`. Es la referencia contra la que un cambio que toque el prompt compara con `--diff` y por sección:

| Métrica | Antes de context-density (`d28fe90`) | Desde context-density |
|---|---|---|
| `found` | 41/46 | 41/46 |
| `search_memory`, media / máximo | 851,5 / 1.239 | 851,5 / 1.239 |
| ACTIVE CONFLICTS, media cuando aparece (45 casos) | 2.273,8 | 403,7 |
| `recall_context`, media / máximo | 3.355,0 / 4.900 | 1.405,0 / 1.738 |
| RECENT CONTEXT, media | 1.129,6 | 1.009,0 |

Detalle de las corridas en el [`verify.md`](../../changes/archive/2026-10-14-context-density/verify.md#r11-rag-eval) de context-density.

Esas cifras son con pooling CLS. Desde project-indexing (pooling mean, `RELEVANCE_RATIO` 0,35), medido en Windows el 2026-10-15 con el mismo golden set: `search_memory` encuentra 36/46 con 764,0 / 1.206 tokens (media / máximo) y `recall_context` 39/46 con 1.086,4 / 1.408. Los casos que se pierden contra CLS (c02, c09, c11, c18, c26 y c41) no vuelven aunque el corte baje a 0,2: la pérdida es del pooling, sobre todo en las consultas en inglés. Detalle en el [`verify.md`](../../changes/archive/2026-10-15-project-indexing/verify.md) de project-indexing.

Desde context-budget, con el presupuesto default de 8.000 bytes, medido en Windows el 2026-10-17 sobre `f3c44ef` con el golden de english-recall (sha256 `363d3e08…a1fb`, n = 46): `recall_context` 39/46 con 1.086,4 / 1.408 tokens y `search_memory` 36/46 con 764,0 / 1.206, sin ningún prompt cortado (ninguno lleva `=== BUDGET ===`). Contra `2026-10-17-real-rank-fusion-tip-agent-view.json` (`31420de`), `rag-eval --diff` no mueve ningún caso en ninguna métrica. Detalle en el [`verify.md`](../../changes/archive/2026-10-17-context-budget/verify.md) de context-budget.

Desde search-abstention, sin filas de score 0, medido en Windows el 2026-10-18 sobre `1307019` con el mismo golden: `search_memory` 36/46 con 761,4 / 1.206 tokens y `recall_context` 37/46 con 987,7 / 1.405. Contra `2026-10-18-real-topic-revisions-final-agent-view.json` (`78b62ea`), `rag-eval --diff` no mueve ningún caso del ranking ni de `search.found`. `recall_context` deja de encontrar c09 y c10, que sólo llegaban al prompt como filas de score 0 ordenadas por id, y gasta menos tokens en 7 casos (−95,9 de media). Detalle en el [`verify.md`](../../changes/archive/2026-10-18-search-abstention/verify.md) de search-abstention.

Desde session-handoff, con LAST SESSION, medido en Windows el 2026-10-19 sobre `1f18565` contra `bca0d0a` (el `develop` anterior al cambio, que da lo mismo que la corrida de search-abstention) con el mismo golden: `recall_context` 37/46 con 1.036,5 / 1.489 tokens y `search_memory` 36/46 con 761,4 / 1.206. `rag-eval --diff` no mueve ningún caso del ranking, de `search.found` ni de `context.found`; `recall_context` gasta más en los 46 casos (+48,7 de media, intervalo [+36,6, +60,9]), menos que los 64,2 tokens de un `session_summary` medio del corpus. Como el eval siembra sin proyecto, cada prompt abre con el mismo resumen, el más nuevo de los seis (87 tokens con el encabezado): en 22 casos ya estaba en RECENT y se mudó (+7, el encabezado), y en 24 no estaba (+87). Detalle en el [`verify.md`](../../changes/archive/2026-10-19-session-handoff/verify.md) de session-handoff.

## Indexar un proyecto no cambia la búsqueda de memorias

Con un proyecto indexado ([`specs/project-docs`](../project-docs/spec.md)), `search_all_memories`, `POST /v1/search`, `POST /v1/context`, `search_memory` y `recall_context` MUST devolver exactamente lo mismo que sin índice: el índice vive en tablas propias (`project_doc_*`) que ninguna de esas rutas consulta, y el refresco no escribe `memory` ni sus vectores, su FTS o el access tracker.

#### Escenario
- GIVEN un agente con memorias en un proyecto
- WHEN se busca antes y después de indexar docs del mismo proyecto con textos casi iguales a esas memorias
- THEN los resultados son idénticos

**Tests:** `indexing_docs_leaves_memory_search_unchanged` (`search_all_memories`, con un canal léxico y un proyecto) y `refreshing_never_touches_memories` (filas, vectores y FTS de `memory` byte a byte) en `crates/clawg-core/src/memory/project_docs/mod.rs`. Los otros caminos de la lista no tienen un test propio: no leen las tablas del índice.

## Invariantes

- El ranking del core (`search_all_memories`, `hit_cmp`) no depende del corte ni de que las herramientas dejen afuera las filas de score 0: los dos sólo deciden cuánto de la lista recibe quien llama.
- Las corridas de eval no registran accesos (`AccessTracker::inert`, `crates/clawg-core/src/eval_api.rs:503`).
- Con `lexical_bonus = 0` el ranking es bit a bit el de antes del bono: es la forma de apagarlo sin tocar el canal léxico.
- Los contadores ya guardados no se reescriben. Los accesos que se contaron de más antes de este comportamiento quedan como estaban.
- El formato del prompt de `recall_context` no toca la búsqueda: la salida de `search_memory` es la misma, y el orden y los separadores de las secciones (`prompt_builder.rs:24-30`) y `K_FINAL` (`:47`) no dependen de él. El separador `=== ACTIVE CONFLICTS (review needed) ===` lo nombran las instrucciones del servidor y la doc de `resolve_conflict`.
- El presupuesto no toca la búsqueda ni el ranking: corta lo que se imprime, después de `K_FINAL`, y la salida de `search_memory` es la misma.
- Un prompt vacío se entrega como `no context yet` (`format_context`, `crates/clawg-cli/src/mcp/tools.rs:508`), salvo cuando las memorias no se buscaron, o se buscaron y todas las filas eran de score 0: entonces lo reemplaza el aviso de [`specs/mcp`](../mcp/spec.md) o la frase de abstención. Ese aviso va antes del prompt y el de `synthesize` después (`format_recall`, `tools.rs:489`), los dos fuera de lo que mide `rag-eval`.
- Armar el prompt no escribe: `fetch_active_conflicts` y `fetch_last_session` son un `SELECT`, y el archivo del workspace no cambia.
- Un contexto sin consulta tampoco registra accesos: `build_session_prompt` no recibe un `AccessTracker` (`prompt_builder.rs:272`), a diferencia de `build_memory_prompt`. El resumen de LAST SESSION no lo registra en ninguno de los dos.

## Fuera de esta spec

- El orden entre hits con score 0: `hit_cmp` desempata por `(kind, id)` ascendente (`crates/clawg-core/src/memory/search.rs:196`). Desde lexical-fusion, un match de identificador ya no depende de ese desempate para llegar al top 10: el bono lo deja con score > 0 aunque su coseno quede bajo el piso. Ninguna herramienta muestra esas filas, pero las métricas de ranking de `rag-eval` miden la lista entera: un `recall@10` puede contar una fila de score 0 que llegó por su id (c09 y c10 del golden set).
- Una puerta de abstención calibrada, que deje de mostrar también filas con score mayor que 0 en los casos sin respuesta. El piso es el percentil 5 de los cosenos consulta-pasaje, así que casi todo texto sin relación puntúa más que 0 y se sigue mostrando.
- Un bono parcial (la fila tiene sólo algunos identificadores de la consulta), números solos como identificadores y FTS para `episodic` y `procedural`.
- Qué pares se disputan y cómo se resuelven: [`specs/disputes`](../disputes/spec.md). Acá sólo está cómo se imprimen.
- Un id en los bullets del workspace. Hoy no lo tienen, y por eso una fila del workspace que está en un conflicto lleva su texto también en el bloque.
