# retrieval — Especificación

Fuente de verdad del comportamiento actual de la recuperación: el corte relativo de `POST /v1/search` y de la herramienta `search_memory`, qué hits cuentan como acceso en `search` y en `recall_context`, el bono que lleva una fila con el identificador exacto de la consulta, y el texto que arma `recall_context` (hits, conflictos y workspace) junto con el parser que lo lee en `rag-eval`. A qué filas de otros agentes llegan la búsqueda y el contexto lo define [`specs/sharing`](../sharing/spec.md); acá está cómo se muestra un hit ajeno. La búsqueda en la documentación y el código de un proyecto es otra cosa y está en [`specs/project-docs`](../project-docs/spec.md).

**Origen:**
- [retrieval-hygiene](../../changes/archive/2026-10-12-retrieval-hygiene/proposal.md), archivado el 2026-10-12.
- [lexical-fusion](../../changes/archive/2026-10-12-lexical-fusion/proposal.md), archivado el 2026-10-12.
- [knowledge-synthesis](../../changes/archive/2026-10-13-knowledge-synthesis/proposal.md), archivado el 2026-10-13 (sólo el apartado de las filas que dejó la app de escritorio).
- [context-density](../../changes/archive/2026-10-14-context-density/proposal.md), archivado el 2026-10-14 (desde "Línea de hit en `recall_context`" hasta "Medición del contexto").
- [agent-sharing](../../changes/archive/2026-10-14-agent-sharing/proposal.md), archivado el 2026-10-14 (sólo "Hits de otros agentes" y los cambios de agente en "El bono respeta el scope" y en "Línea de hit en `recall_context`").
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "El piso de coseno", "Indexar un proyecto no cambia la búsqueda de memorias", el valor de `RELEVANCE_RATIO` y la medición con pooling mean en "Medición del contexto").
- [memory-rank-fusion](../../changes/archive/2026-10-17-memory-rank-fusion/proposal.md), archivado el 2026-10-17 (sólo "Definiciones", "Corte relativo en `POST /v1/search`" y "Cliente HTTP y herramienta `search_memory`").

## Definiciones

- **Hit exacto:** hit cuya fila recibió el bono léxico en esta búsqueda, es decir, `RawHit.lexical_match` (`crates/clawg-core/src/memory/score.rs:28-30`), una fila de `memory` que está en los matches de identificador de la consulta. Viaja en cada hit como `exact`. Un hit de `episodic` o `procedural` nunca es exacto.
- **Corte relativo con ratio r (`shown_hits`, `crates/clawg-cli/src/relevance.rs:31`):** sobre hits ordenados mejor primero, cada uno con su score y su marca de exacto, pasa todo hit exacto, aunque quede detrás de uno cortado. Un hit no exacto pasa si está entre los `MIN_SHOWN_HITS` primeros no exactos o si su score es `>= r × mejor score no exacto`. Los que pasan conservan su orden. Sin scores, no queda ninguno. Sin hits exactos, el resultado es el de cortar por prefijo: los primeros mientras la posición sea menor que `MIN_SHOWN_HITS` o el score sea `>= r × mejor`.
- **`RELEVANCE_RATIO` (`relevance.rs:19`):** 0,35. El ratio que usa el puente MCP y que pide el cliente HTTP. Medido con `rag-eval --real --agent-view` sobre vectores con pooling mean, cuyo mejor score tiene mediana 0,19: con 0,35 `search_memory` muestra el recuerdo esperado en 36 de 46 consultas con 764 tokens de media; con 0,6 eran 30 con 455 y con 0,2 son 37 con 894. El score del esperado (0,03–0,41) y el del resto (hasta 0,38) se superponen, por eso el corte es relativo.
- **`MIN_SHOWN_HITS` (`relevance.rs:22`):** 2. Hits no exactos que pasan el corte sea cual sea su score; los exactos no cuentan para este mínimo.
- **Acceso:** un id que llega al access tracker. En el flush suma `access_count`, lleva `last_accessed_at` al máximo entre el actual y el del flush, y recalcula el tier en la misma sentencia (`flush`, `crates/clawg-core/src/memory/access.rs:175`; el `UPDATE` en `:204-209`). El flush corre cada 30 s o al juntar 500 ids (`access.rs:76-78` y `:90`).
- **Hit contable:** hit de `kind = memory` con `score > 0` (`AccessTracker::record_hits`, `access.rs:151`). Sólo cuentan los de `memory` porque los ids son por tabla y el flush sólo actualiza `memory`. Un hit con score 0 está bajo el piso de similitud y ocupa un lugar de relleno.
- **Token:** cada tramo de `char::is_alphanumeric() || '_'` de la consulta, como los parte `fts_tokens` (`crates/clawg-core/src/memory/search.rs:576`).
- **Token identificador (`is_identifier`, `search.rs:712`):** un token que cumple al menos una de:
  - tiene letras y dígitos (`getUser4821`, `v2`);
  - tiene `_` y al menos una letra (`E_AUTH_4821`, `snake_case`);
  - tiene una minúscula seguida inmediatamente de una mayúscula (`getEstadoLabel`).
  Un número solo (`4821`) no es identificador.
- **Identificadores de la consulta (`identifier_tokens`, `search.rs:699`):** los tokens identificadores distintos de toda la consulta, en orden de aparición, hasta 8 (`MAX_IDENTIFIER_TOKENS`, `search.rs:691`).
- **Match de identificador (`identifier_matches`, `search.rs:754`):** fila de `memory` visible para el llamador (mismos predicados de agente, proyecto, estado y `deleted_at` que el canal léxico) cuyo contenido tiene **todos** los identificadores de la consulta, según FTS5 (`"t1" AND "t2" …`). FTS5 separa en `_`, así que `"user_id"` es una frase que también matchea `user id` y `user-id`: para los identificadores con `_` el contenido MUST tenerlos escritos tal cual, sin distinguir mayúsculas y sin pegarse a otro nombre (`holds_identifier`, `search.rs:723`). Las filas que no los tienen así no reciben bono ni cuentan para el presupuesto.
- **Presupuesto léxico:** `SearchScope.lexical`, 20 por defecto (`search.rs:43`).
- **Bono léxico:** `ScoringConfig.lexical_bonus`, 1,0 por defecto (`crates/clawg-core/src/memory/score.rs:185`).
- **Hit mostrado:** hit de `kind = memory` presente en RECENT o LONG-TERM del prompt de `recall_context` (`build_memory_prompt`, `crates/clawg-core/src/memory/prompt_builder.rs:110`). Los ids son por tabla (`shown`, `prompt_builder.rs:211`, sólo toma `TableKind::Memory`), así que un hit episodic o procedural nunca cuenta.
- **Fila del workspace:** id en `EmittedIds` (`crates/clawg-core/src/memory/workspace.rs:201`), que también se suma a `shown`. Su bullet no tiene id. **No** es un hit mostrado.
- **Posición de un hit:** índice 0-based en el orden impreso: RECENT y luego LONG-TERM.
- **Par de conflicto:** fila nueva (`disputed`) contra una fila vieja. `fetch_active_conflicts` (`prompt_builder.rs:478`) sólo trae pares con al menos un lado que es hit mostrado o fila del workspace; el otro lado puede estar fuera del prompt.

El corte vive en `clawg-cli` y no en el core: `search_all_memories` devuelve siempre la lista completa ordenada, y las métricas de `rag-eval` se miden sobre esa lista (`rag-eval` llama a `store.search` directo).

## Corte relativo en `POST /v1/search`

`POST /v1/search` MUST aceptar el campo opcional `min_relative_score` (`SearchRequest`, `crates/clawg-cli/src/daemon/api.rs:340-350`).

- Ausente: la respuesta MUST traer todos los hits ordenados hasta `limit`, sin corte.
- Presente con un valor en `[0, 1]`: el daemon MUST devolver sólo los hits que pasan el corte relativo con ese ratio, en el mismo orden (`search`, `api.rs:877`; el corte en `:799-802`). Cada hit de la respuesta MUST llevar `exact`.
- Presente fuera de `[0, 1]`, o no finito: MUST responder `400` sin buscar ni registrar accesos (`check_relative_score`, `api.rs:308`). La validación corre después de comprobar el embedder: sin embedder listo, la respuesta es la de esa condición (`503`).

#### Escenario: sin campo
- GIVEN una fila igual a la consulta y tres sin relación
- WHEN se busca sin `min_relative_score`
- THEN vuelven las cuatro

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

**Tests:** `search_without_a_relative_cut_is_unchanged`, `search_applies_the_requested_relative_cut` y `search_rejects_a_relative_score_outside_zero_to_one` (`crates/clawg-cli/tests/api.rs`); `a_relative_score_inside_zero_to_one_passes_through` y `a_relative_score_outside_zero_to_one_is_a_bad_request`, que incluye `NaN` e infinito (`api.rs`, mod tests); `relevance_ratio_pins_the_shipped_value`, `hits_far_below_the_best_match_are_not_shown`, `a_hit_exactly_at_the_ratio_is_shown`, `the_two_best_are_shown_whatever_they_score` y `no_scores_keep_nothing` (`relevance.rs`); `exact_hits_do_not_set_the_bar_for_the_others`, `an_exact_hit_is_shown_behind_one_the_cut_drops` y `the_two_best_inexact_hits_are_shown_whatever_they_score` (`relevance.rs`); `search_shows_the_rows_an_identifier_match_used_to_hide` y `an_identifier_match_from_another_agent_is_exact_and_attributed` (`api.rs`).

## `search` registra sólo lo que devuelve

El handler `search` MUST registrar como acceso exactamente los hits contables de la respuesta que envía, después del corte (`api.rs:915`). Un hit cortado, o con score 0, MUST NOT cambiar `access_count` ni `last_accessed_at`.

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

`build_memory_prompt` MUST registrar sólo los hits contables entre los que entran al prompt, después de sacar las filas del workspace y de cortar en `K_FINAL = 10` (`crates/clawg-core/src/memory/prompt_builder.rs:42`, `:178-179`, `:187`). Los hits con score 0 MUST seguir apareciendo en el prompt.

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

`DaemonClient::search` MUST mandar `min_relative_score = RELEVANCE_RATIO` junto con los campos de `SearchArgs` (`search_request`, `crates/clawg-cli/src/client.rs:403`; uso en `:538`). El puente MUST volver a aplicar el corte al renderizar `search_memory` (`relevant_hits` y `format_search`, `crates/clawg-cli/src/mcp/tools.rs:60` y `:67`), con el mismo `shown_hits` que el daemon, así que un hit exacto se muestra aunque el corte deje fuera a otro. `Hit` del cliente MUST leer `exact` con `#[serde(default)]`: un hit sin el campo vale `false`. Aplicar el corte dos veces da el mismo resultado, y contra un daemon que ignora el campo el agente ve lo mismo.

#### Escenario: cuerpo del pedido
- WHEN el cliente arma el cuerpo de una búsqueda
- THEN el cuerpo contiene `"min_relative_score": 0.35` además de los campos de `SearchArgs`

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

Para un hit marcado como match de identificador (`RawHit.lexical_match`, `score.rs:30`), `score()` (`score.rs:263`) MUST valer `(cos' + lexical_bonus) × max(piso_tier, decaimiento) × importancia × estado`, donde `cos'` es el coseno ya re-escalado contra el piso (la suma en `score.rs:276`). Para un hit sin match, MUST valer lo mismo que sin el bono.

#### Escenario: el bono se suma después del piso
- GIVEN un hit match con coseno bajo el piso (cos' = 0), tier 1, edad 0, importancia 1 y estado `active`
- THEN el score es `lexical_bonus × importancia`, mayor que 0

#### Escenario: sin match, sin cambio
- GIVEN el mismo hit sin match
- THEN el score es 0

**Test:** `a_lexical_match_adds_the_bonus_after_the_floor`, `without_a_match_the_score_is_unchanged` y `a_match_outranks_a_closer_row_with_the_same_decay_and_importance` en `score.rs`; el literal, en `scoring_config_default_pins_the_shipped_ranking_literals`.

## El piso de coseno

El coseno de cada hit MUST re-escalarse contra `ScoringConfig::DEFAULT.cosine_floor`, que vale 0,79 (`score.rs:181`): `cos' = clamp((cos − piso) / (1 − piso), 0, 1)` (`rescale_cosine`, `score.rs:214`; aplicado en `score()`, `score.rs:270`). Un coseno bajo el piso vale 0 y el piso es el cero del término de similitud; sin la re-escala, todos los cosenos caerían en una banda angosta y la similitud casi no pesaría frente a la importancia.

El valor está atado a la distribución de cosenos del embedder: al pooling mean del modelo (`multilingual-e5-small/mean`, [`specs/storage`](../storage/spec.md)) y a los prefijos de consulta y pasaje que el daemon manda hoy. Cambiar cualquiera de los dos mueve toda la distribución y obliga a medirlo de nuevo.

**Cómo se midió.** Sobre el golden set de memorias (`docs/eval/golden.json`, 46 casos), con pooling mean, el 2026-10-15: un barrido del piso de 0,70 a 0,83 da el mejor MRR (0,5524), nDCG@10 (0,6215) y R@10 (0,8478) en 0,79, así que 0,79 queda.

**Tests:** `scoring_config_default_pins_the_shipped_ranking_literals` (fija el literal 0,79; `score.rs`), y `rescale_cosine_maps_the_floor_to_zero`, `rescale_cosine_maps_one_to_one_regardless_of_floor`, `rescale_cosine_clamps_a_cosine_below_the_floor_to_zero` y `rescale_cosine_is_monotone_non_decreasing_for_any_floor` (`score.rs`).

## La búsqueda premia el identificador exacto

Con el canal léxico encendido y texto de consulta, `search_all_memories` MUST marcar como match de identificador cada fila que lo sea, venga del canal denso (`knn_one`, `search.rs:391`) o del léxico (`score_lexical_candidates`, `search.rs:951`), y MUST sumar los matches a los candidatos léxicos aunque estén fuera del top denso o pasado el token 16. Los matches se calculan antes del `try_join!` de los canales (`search.rs:1087`).

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

Un match de identificador MUST cumplir los mismos predicados que el candidato léxico: agente (el llamador y los autores de su alcance, [`specs/sharing`](../sharing/spec.md) R2), proyecto (con la excepción de `user_preference`), estado `active`/`disputed` y `deleted_at IS NULL` (`IDENTIFIER_MATCH_SQL`, `search.rs:738`). Una fila de un autor fuera de su alcance, borrada o de otro proyecto MUST NOT recibir el bono ni contar para el presupuesto.

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

Un error de la consulta de matches (FTS ausente, MATCH rechazado, conexión cerrada) MUST dejar la búsqueda como si no hubiera matches, igual que los candidatos léxicos (`search.rs:787`, junto a `:683`).

**Test:** `a_failing_identifier_query_leaves_the_search_unboosted` en `search.rs`.

## Las filas que dejó la app de escritorio siguen alcanzables

Ningún código de producción escribe `episodic_trajectory` ni `procedural_skill`, pero una base puede traer filas de la app de escritorio. Esas filas MUST seguir alcanzables: la búsqueda MUST consultarlas (KNN de 5 + 5), el workspace MUST mostrar los skills del proyecto, y `list_memories`, `GET`, `PATCH` y `DELETE` MUST aceptarlas. El re-embed de `procedural_skill` MUST embeber `"{trigger} {skill_name}"`.

#### Escenario: filas como las escribía la app de escritorio
- GIVEN una trayectoria y un skill escritos con los mismos escritores que usaba la app (que quedan sólo para tests), y sus vectores
- WHEN se busca con el vector del skill, se regenera el workspace del proyecto, se lista y se re-embebe
- THEN el skill está entre los hits y en el workspace, las dos filas están en el listado, y el re-embed del skill embebe `"when login fails Fix login flow"` para `trigger = "when login fails"` y `skill_name = "Fix login flow"`

**Test:** `episodic_trajectory_retrievable_by_same_agent_id` y `knn_procedural_returns_skill_rows` (`search.rs`), `regenerate_reports_the_ids_it_rendered` y `workspace_top_skills_exclude_foreign_project` (`workspace.rs`), `list_memories_kind_none_merges_all_tables_sorted_desc` (`admin.rs`) y `reembed_reinfer_uses_passage_role_and_shared_skill_embedding_text` (`maintenance.rs`, fija el texto literal). La apertura de una base en V9 con esas filas está en [`specs/synthesis`](../synthesis/spec.md) (su R9).

## Hits de otros agentes

La búsqueda y el contexto traen, además de lo propio, las filas de los autores que la política de [`specs/sharing`](../sharing/spec.md) deja leer al llamador: `POST /v1/search` y `POST /v1/context` resuelven el alcance del agente del pedido en cada llamada (`reach`, `api.rs:925`; usos en `api.rs:895` y `:964`). Sobre eso:

- Cada hit de `POST /v1/search` MUST llevar su autor como `agent_id` (`RankedHit.agent_id`, `search.rs:88`); el cliente lo lee con `#[serde(default)]` y un hit sin autor cuenta como propio (`client.rs:165`).
- El workspace, ACTIVE CONFLICTS, el listado, la síntesis y el detector de contradicciones al guardar MUST seguir viendo sólo lo del agente: sólo los hits de búsqueda llegan a filas ajenas.
- Un hit ajeno MUST contar como acceso igual que uno propio (`api.rs:915`).

**Test:** `a_hit_carries_its_author` (`search.rs`), `search_returns_the_author_of_each_hit` y `search_and_context_follow_the_sharing_policy_from_the_next_call` (`crates/clawg-cli/tests/api.rs`), `the_workspace_and_conflicts_stay_own` (`prompt_builder.rs`) y `contradiction_probe_ignores_shared_authors` (`contradiction.rs`).

## Línea de hit en `recall_context`

Cada hit MUST imprimirse como un encabezado de una línea seguido del texto en la misma línea (`format_hit_line`, `prompt_builder.rs:333`):

- `kind = memory`: `[YYYY-MM-DD #id tipo] texto`
- otro kind: `[YYYY-MM-DD #id kind/tipo] texto`, con `kind` en minúsculas (`episodic`, `procedural`). Para esos kinds la búsqueda pone el tipo fijo `episode` o `procedure`.
- un hit de otro agente lleva ` by <autor>` antes del `]` (`[YYYY-MM-DD #id tipo by <autor>] texto`), y si hay alguno el prompt termina con un bloque `=== SHARED ===` con una nota (`prompt_builder.rs:276-282`): [`specs/sharing`](../sharing/spec.md) R4. Sin hits ajenos, el prompt es el de siempre.

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

Cada par MUST imprimir los ids de sus dos lados como `#id`: son los valores que `resolve_conflict` recibe como `keep` y `other` (`ResolveConflictArgs`, `crates/clawg-cli/src/mcp/server.rs:268`), y los dos son ids de `memory`.

Un lado que es hit mostrado MUST imprimirse sólo como `#id`. Un lado que no es hit mostrado (fila del workspace, o fila fuera del prompt) MUST imprimirse con su texto completo bajo el encabezado `[YYYY-MM-DD #id]` la primera vez que aparece en el bloque, y sólo como `#id` las siguientes (`format_conflict_block`, `prompt_builder.rs:376`). Así el texto de una fila aparece a lo sumo una vez en el bloque aunque esté en varios pares: en una cadena de disputas, la fila del medio es nueva en un par y vieja en otro. Un par con ambos lados mostrados MUST NOT imprimir ningún texto. `importance`, `tier` y `status` no se imprimen.

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

**Test:** `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest`, `a_row_in_several_pairs_prints_its_text_once` y `e2e_full_4_block_output` (la fila mostrada aparece una vez y el bloque la cita como `OLD #id`) en `prompt_builder.rs`.

## Conflictos agrupados por fila nueva

Los pares que comparten `new_id` MUST formar un grupo: una línea `NEW`, y una línea `OLD` por cada vieja. El texto de la nueva MUST aparecer a lo sumo una vez en el bloque.

#### Escenario
- GIVEN new 9 disputa a las viejas 1, 2 y 3, y 9 no es hit
- THEN `[<fecha> #9 disputed] <texto>` aparece una vez, seguido de tres líneas `OLD`

**Test:** `a_new_row_disputing_several_old_rows_prints_its_text_once` y `a_row_in_several_pairs_prints_its_text_once` en `prompt_builder.rs`.

## Una NOTE por bloque de conflictos

La NOTE MUST imprimirse una sola vez por bloque, justo bajo el separador y antes de los grupos (`CONFLICT_NOTE`, `prompt_builder.rs:348`):

```
NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
Con su respuesta, llamá a resolve_conflict (keep = la que vale, other = la otra).
```

#### Escenario
- GIVEN 3 grupos
- THEN `NOTE:` aparece exactamente una vez, y `resolve_conflict` también

**Test:** `conflict_block_has_one_note` y `the_conflict_note_names_the_tool` en `prompt_builder.rs`.

## Orden y tope de los conflictos

Una función pura sobre los pares leídos (`rank_conflicts`, `prompt_builder.rs:357`) MUST ordenarlos por clave `(rank, new_id desc, old_id asc)`:

- `rank` = menor posición de hit entre sus lados que son hits mostrados;
- los pares sin ningún hit mostrado (sólo tocan una fila del workspace) MUST ir al final, con `rank` = infinito.

El resultado MUST NOT depender del orden de entrada. Los grupos salen en orden de su primer par.

Tras ordenar, el bloque MUST conservar los primeros 5 pares (`MAX_CONFLICT_PAIRS`, `prompt_builder.rs:44`; el tope cuenta pares, antes de agrupar). Si quedan N > 0 afuera, la última línea del bloque MUST ser `(+N conflictos más sin mostrar)`. Con N = 0 MUST NOT haber esa línea.

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
- THEN hay 5 pares impresos y la línea `(+3 conflictos más sin mostrar)`; con 5 pares no hay línea de omitidos

**Test:** `conflict_pairs_rank_by_best_hit_position_workspace_only_last`, `conflict_pair_order_ignores_input_order` y `conflict_block_keeps_five_pairs_and_counts_the_rest` en `prompt_builder.rs`.

## Workspace en el prompt

El workspace entregado al modelo (`workspace_for_prompt`, `prompt_builder.rs:430`) MUST NOT contener `<!-- `, ni sección USER vacía, ni un encabezado `## ` de la sección AUTO sin bullets debajo. El contenido no vacío de USER MUST conservarse tal cual lo escribió el usuario, encabezados incluidos, sin marcadores y sin líneas en blanco al principio ni al final: la poda de encabezados sólo mira la sección AUTO, que es la que genera `render_rows` (`workspace.rs:445`). Un archivo sin `AUTO_START` MUST entregarse entero; uno con AUTO pero sin `AUTO-END` se toma como AUTO hasta el final, y el texto antes de `AUTO-START` se entrega. Si no queda nada, no se emite la sección WORKSPACE. El archivo en disco MUST ser byte a byte idéntico antes y después de un recall.

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

## El parser de `rag-eval --agent-view`

`context_segments` (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs:84`) MUST segmentar por sección (`context_sections`, `parse.rs:67`): los segmentos de hit salen sólo de RECENT CONTEXT y LONG-TERM KNOWLEDGE; los bullets, sólo de WORKSPACE; el texto de ACTIVE CONFLICTS MUST NOT contar para `found`. Encabezado de hit = línea que cumple `[` + `YYYY-MM-DD` + ` #` + dígitos + (` ` + palabra sin espacios)? + `]` + ` ` (`hit_text`, `parse.rs:137`); el texto es lo que sigue en la misma línea, y el segmento corre hasta el próximo encabezado o separador. Un marcador `<!--` cierra un bullet.

#### Escenario: multilínea
- GIVEN dos hits, el primero con código y línea en blanco
- THEN hay 2 segmentos, el primero completo

#### Escenario: conflictos
- GIVEN una sección ACTIVE CONFLICTS con `NEW [..#2 disputed] x` y `OLD [..#1] y`
- THEN no se genera ningún segmento

**Test:** `a_hit_line_is_one_segment`, `a_hit_of_another_kind_is_one_segment`, `a_multi_line_hit_runs_until_the_next_header`, `a_hit_ends_at_the_next_section_separator`, `a_hit_may_open_with_a_dash`, `conflict_text_yields_no_segments` y `workspace_markers_end_a_bullet` en `parse.rs`; `read_context_finds_rows_in_hit_lines_and_workspace_bullets` en `agent_view/mod.rs`. El punta a punta del agent-view (`runner.rs`) arma un prompt real y lo lee con este parser.

## Los ids de `recall_context` en `delete_memory` y `update_memory`

La doc de `DeleteMemoryArgs.id` (`server.rs:228`), la de `UpdateMemoryArgs.id` (`server.rs:251`) y la descripción de `delete_memory` (`server.rs:873`) MUST nombrar `recall_context` además de `search_memory` y `list_memories`, y decir que un hit sin kind explícito es `memory`. `kind` sigue siendo obligatorio en las dos.

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

## Indexar un proyecto no cambia la búsqueda de memorias

Con un proyecto indexado ([`specs/project-docs`](../project-docs/spec.md)), `search_all_memories`, `POST /v1/search`, `POST /v1/context`, `search_memory` y `recall_context` MUST devolver exactamente lo mismo que sin índice: el índice vive en tablas propias (`project_doc_*`) que ninguna de esas rutas consulta, y el refresco no escribe `memory` ni sus vectores, su FTS o el access tracker.

#### Escenario
- GIVEN un agente con memorias en un proyecto
- WHEN se busca antes y después de indexar docs del mismo proyecto con textos casi iguales a esas memorias
- THEN los resultados son idénticos

**Tests:** `indexing_docs_leaves_memory_search_unchanged` (`search_all_memories`, con un canal léxico y un proyecto) y `refreshing_never_touches_memories` (filas, vectores y FTS de `memory` byte a byte) en `crates/clawg-core/src/memory/project_docs/mod.rs`. Los otros caminos de la lista no tienen un test propio: no leen las tablas del índice.

## Invariantes

- El ranking del core (`search_all_memories`, `hit_cmp`) no depende del corte: el corte sólo decide cuánto de la lista recibe quien llama.
- Las corridas de eval no registran accesos (`AccessTracker::inert`, `crates/clawg-core/src/eval_api.rs:478`).
- Con `lexical_bonus = 0` el ranking es bit a bit el de antes del bono: es la forma de apagarlo sin tocar el canal léxico.
- Los contadores ya guardados no se reescriben. Los accesos que se contaron de más antes de este comportamiento quedan como estaban.
- El formato del prompt de `recall_context` no toca la búsqueda: la salida de `search_memory` es la misma, y el orden y los separadores de las secciones (`prompt_builder.rs:21-24`) y `K_FINAL` (`:42`) no dependen de él. El separador `=== ACTIVE CONFLICTS (review needed) ===` lo nombran las instrucciones del servidor y la doc de `resolve_conflict`.
- Un prompt vacío se entrega como `no context yet` (`format_context`, `crates/clawg-cli/src/mcp/tools.rs:298`), y el aviso de `synthesize` va después del prompt (`format_recall`, `tools.rs:285`), fuera de lo que mide `rag-eval`.
- Armar el prompt no escribe: `fetch_active_conflicts` es un `SELECT`, y el archivo del workspace no cambia.

## Fuera de esta spec

- El orden entre hits con score 0: `hit_cmp` desempata por `(kind, id)` ascendente (`crates/clawg-core/src/memory/search.rs:135`). Desde lexical-fusion, un match de identificador ya no depende de ese desempate para llegar al top 10: el bono lo deja con score > 0 aunque su coseno quede bajo el piso.
- Un bono parcial (la fila tiene sólo algunos identificadores de la consulta), números solos como identificadores y FTS para `episodic` y `procedural`.
- Qué pares se disputan y cómo se resuelven: [`specs/disputes`](../disputes/spec.md). Acá sólo está cómo se imprimen.
- Un id en los bullets del workspace. Hoy no lo tienen, y por eso una fila del workspace que está en un conflicto lleva su texto también en el bloque.
