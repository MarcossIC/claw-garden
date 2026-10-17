# memory-rank-fusion — Especificación

Delta sobre [`specs/retrieval`](../../../specs/retrieval/spec.md) (el corte relativo y el bono léxico) y [`specs/eval`](../../../specs/eval/spec.md) (agent-view y `golden-mine synth`). No cambia ningún score, el orden de la búsqueda ni `recall_context`.

## Definiciones

- **Hit exacto:** hit cuya fila recibió el bono léxico en esta búsqueda, es decir, una fila de `memory` que está en los matches de identificador de la consulta (`identifier_matches`, `crates/clawg-core/src/memory/search.rs:754`). Es exactamente la condición `RawHit.lexical_match` (`crates/clawg-core/src/memory/score.rs:28-30`).
- **Mejor score semántico:** el score más alto entre los hits que no son exactos. Sin hits no exactos, no hay.
- **Corte relativo con ratio r**, sobre hits ordenados mejor primero, cada uno con su score y su marca:
  - un hit exacto MUST pasar siempre;
  - un hit no exacto MUST pasar si está entre los `MIN_SHOWN_HITS` primeros **hits no exactos**, o si su score es `>= r × mejor score semántico`.
  Los que pasan conservan su orden. Sin ninguna marca, el resultado MUST ser el mismo que el corte de hoy (los primeros `relevant_len(scores, r)` hits).
- **Test RED:** falla hoy, por comportamiento o porque no compila. **Guarda:** pasa hoy y protege algo que no debe cambiar.

## R1. La marca llega al resultado de la búsqueda

`RankedHit` MUST llevar `exact: bool`, igual a `lexical_match` de la fila que lo produjo, en los dos lugares que arman hits (la búsqueda densa y los candidatos léxicos). El score y el orden MUST NOT cambiar. Un hit de `episodic` o `procedural` MUST valer `exact = false` (el bono sólo existe en `memory`).

#### Escenario: la fila con el identificador viene marcada
- GIVEN una fila con `parseLedger4821` y otra sin él
- WHEN se busca `qué hace parseLedger4821`
- THEN el hit de la primera tiene `exact = true` y el de la segunda `exact = false`

**Tests** (`search.rs`): `a_row_holding_the_query_identifier_comes_back_exact` (**RED**, no compila); guarda: los tests del bono léxico existentes no cambian.

## R2. El corte relativo usa la marca

`relevant_len` (`crates/clawg-cli/src/relevance.rs:25`) MUST pasar a decidir sobre `(score, exact)` con la regla de las definiciones, y devolver qué hits pasan, no un largo: un hit exacto puede quedar detrás de uno cortado.

#### Escenario: la tesis, en puro
- GIVEN un hit exacto en 1,6 y hits no exactos en 0,30, 0,25, 0,20 y 0,05
- WHEN se corta con 0,35
- THEN pasan el exacto y los de 0,30, 0,25 y 0,20 (hoy, sólo los dos primeros)

#### Escenario: un exacto bajo, detrás de uno cortado
- GIVEN hits no exactos en 0,40, 0,30 y 0,05, y un exacto en 0,02
- WHEN se corta con 0,35
- THEN pasan 0,40, 0,30 y el exacto, y el de 0,05 no

#### Escenario: sin marcas, como hoy
- GIVEN cualquier lista de los tests actuales de `relevance.rs`, sin marcas
- THEN pasan los mismos hits que hoy

La función se llama `shown_hits(&[(f64, bool)], ratio) -> Vec<bool>`.

**Tests** (`relevance.rs`): `exact_hits_do_not_set_the_bar_for_the_others`, `an_exact_hit_is_shown_behind_one_the_cut_drops` y `the_two_best_inexact_hits_are_shown_whatever_they_score` (**RED**, no compilan); los cinco tests actuales pasan a la firma nueva sin marcas y son guardas.

## R3. El daemon y el puente cortan igual

`POST /v1/search` con `min_relative_score` MUST aplicar el corte de R2 y MUST registrar como acceso exactamente los hits que envía (`api.rs:915`). La respuesta MUST llevar `exact` en cada hit. El puente (`relevant_hits`, `crates/clawg-cli/src/mcp/tools.rs:60`) MUST aplicar el mismo corte con `RELEVANCE_RATIO`. `Hit` del cliente MUST leer `exact` con `#[serde(default)]`: un hit sin el campo vale `false`.

#### Escenario: la tesis, de punta a punta
- GIVEN una fila con el identificador de la consulta, con coseno 0,85, y tres filas sin él con cosenos 0,88, 0,87 y 0,86, todas del mismo tipo, importancia y edad, más ruido bajo el piso
- WHEN se busca con `min_relative_score = 0.35`
- THEN vuelven las cuatro, con `exact = true` sólo en la primera (hoy vuelven dos)
- AND el puente, sobre esa respuesta, muestra las cuatro

#### Escenario: daemon viejo
- GIVEN un hit JSON sin `exact`
- THEN el cliente lo lee con `exact = false`

#### Escenario: otro agente
- GIVEN la fila con el identificador guardada por otro agente que el lector puede leer
- WHEN el lector busca
- THEN el hit llega con `exact = true` y `agent_id` del autor, y `search_memory` lo muestra `by <autor>`

**Tests:** `search_shows_the_rows_an_identifier_match_used_to_hide` y `an_identifier_match_from_another_agent_is_exact_and_attributed` (`crates/clawg-cli/tests/api.rs`, **RED**: con la regla de hoy el daemon devuelve dos hits, comprobado), con un embedder de prueba que fija los vectores por texto; `a_hit_without_exact_reads_as_inexact` (`client.rs`, guarda que compila recién con el campo); `relevant_hits_keeps_exact_hits_and_cuts_against_the_rest` y `an_exact_hit_from_another_agent_is_shown_by_its_author` (`tools.rs`, **RED**). Guardas: `search_without_a_relative_cut_is_unchanged`, `search_applies_the_requested_relative_cut`. Qué se registra como acceso no tiene test propio, como hoy ([`specs/retrieval`](../../../specs/retrieval/spec.md#search-registra-sólo-lo-que-devuelve)): el handler registra la misma lista que responde (`api.rs:915`), y el flush corre cada 30 s, así que un test HTTP no lo puede adelantar.

## R4. Agent-view mide si se mostraron todas las esperadas

Cada `ToolStats` de agent-view MUST llevar `all_hit_rate`: la parte de los casos cuyos `found_refs` son todas sus `expected_refs`. Un archivo sin el campo MUST seguir leyéndose (`#[serde(default)]`). La consola lo imprime junto al `hit rate` por subconjunto.

#### Escenario: un caso con dos esperadas y una vista
- GIVEN un caso con `expected_refs = [a, b]` y `found_refs = [a]`
- THEN cuenta para `hit_rate` y no para `all_hit_rate`

**Tests** (`agent_view/stats.rs`, `agent_view/mod.rs`): `all_hit_rate_counts_only_cases_that_showed_every_expected_row` y `summarize_reports_hit_rate_token_mean_and_max` (**RED**, no compilan); `a_summary_without_all_hit_rate_still_reads` (guarda que compila recién con el campo).

## R5. El subconjunto `lexical_related`

`golden-mine synth` MUST agregar, después de todo lo que genera hoy, 30 casos `lexical_related`. Cada uno:

- una consulta que nombra un identificador (función o código de error, alternados, en inglés y en español) y un tema identificado por un nombre clave;
- `expected_refs`: la fila que tiene el identificador y tres filas del mismo tema y nombre clave que no lo tienen;
- `corpus_refs`: esas cuatro y cuatro hermanas con identificadores vecinos (los mismos dígitos en otro orden o el último un paso al lado) que no hablan del tema;
- `query_lang` igual al idioma de sus filas.

Para una misma semilla, las filas y los casos de los demás subconjuntos MUST NOT cambiar. Los identificadores MUST NOT repetirse con los de `lexical` ni aparecer en la base. `rag-eval` MUST aceptar `"subset": "lexical_related"`. `synth` MUST aceptar `query_lang` en los casos de la base y conservarlo.

**Tests** (`golden-mine/synth/mod.rs`): `there_are_thirty_lexical_related_cases`, `a_lexical_related_case_expects_the_identifier_row_and_three_related_rows`, `only_the_identifier_row_of_a_related_case_holds_its_identifier`, `lexical_related_identifiers_are_unique_and_new` (**RED**); guarda: `the_other_subsets_do_not_change_when_lexical_related_is_added`, que fija el sha256 del set de la semilla 7 sin las filas y casos nuevos al que daba el generador antes de este cambio, y `adding_long_cases_leaves_the_rest_of_the_set_unchanged`, que ahora quita los dos. `templates.rs`: `a_related_topic_names_the_identifier_only_in_its_exact_row_and_query`. `golden.rs` de `synth`: `a_base_case_keeps_its_query_lang` (**RED**). `rag-eval/golden.rs`: `every_subset_name_parses_in_snake_case` suma `lexical_related` (**RED**). `tests/synth_roundtrip.rs` lista el subconjunto nuevo en `by_subset` y en los intrusos de la auditoría.
