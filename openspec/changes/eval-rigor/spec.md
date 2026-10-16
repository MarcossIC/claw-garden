# eval-rigor — Especificación

Delta sobre [`specs/eval`](../../specs/eval/spec.md). Cambia lo que `rag-eval` reporta y compara; no cambia ningún ranking ni ningún número de las métricas que ya existen.

## Definiciones

- **Caso movido:** un caso unido por `query` en los dos archivos de `--diff` cuya métrica es mayor (mejor) o menor (peor) en el segundo. En tokens, menos es mejor.
- **p exacto:** el p de dos colas de la binomial exacta con `p = 0,5` sobre los casos movidos (`mcnemar_exact`).
- **Métrica primaria:** R@5 de `retrieval` y `search_found` de `agent_view`. Las demás son informativas.
- **Acierto estricto en k:** todas las esperadas en el top-k (`recall@k == 1`). **Parcial:** al menos una (`recall@k > 0`).
- **Partición:** `calibration` o `test`, según el sha256 de la consulta ([diseño, D4](design.md#d4-partición-por-hash)).

## R1. Funciones estadísticas

`stats.rs` MUST dar `wilson`, `mcnemar_exact`, `paired_bootstrap_ci` y `min_one_way_significant`, puras y totales.

#### Escenario: valores conocidos
- WHEN se calcula `wilson(36, 46)`
- THEN da 0,644 a 0,877 (±0,001)
- AND `wilson(0, 10)` tiene límite superior 0,278 y `wilson(0, 0)` da 0 a 1
- AND `mcnemar_exact(0, 6)` = 0,03125, `(0, 5)` = 0,0625, `(1, 8)` = 0,0391, `(1, 7)` = 0,0703, `(0, 0)` = 1, y `(b, c)` = `(c, b)`

#### Escenario: bootstrap
- GIVEN la misma lista de diferencias y la misma semilla
- THEN dos llamadas dan el mismo intervalo
- AND diferencias todas 0 dan `(0, 0, 0)`, y diferencias todas positivas dan un límite inferior mayor que 0

**Tests** (`stats.rs`): `wilson_matches_known_intervals`, `wilson_of_no_cases_is_the_whole_range`, `mcnemar_exact_matches_known_p_values`, `mcnemar_exact_is_symmetric_and_never_above_one`, `mcnemar_exact_stays_finite_for_thousands_of_cases`, `bootstrap_is_deterministic_for_a_seed`, `bootstrap_of_zero_differences_is_zero`, `bootstrap_of_positive_differences_excludes_zero`, `six_cases_one_way_is_the_smallest_significant_count`, `splitmix_matches_the_golden_mine_sequence`.

## R2. `--diff` con p e intervalo

Cada métrica de `--diff` MUST informar `better`, `worse`, `net` y el p exacto. MRR, nDCG@10 y los tokens MUST informar además la diferencia media con su intervalo bootstrap. Al final MUST haber una línea por métrica primaria presente que diga si la diferencia es distinguible a p < 0,05 o, si no lo es, que no es distinguible con ese número de casos y cuántos casos en una dirección hacen falta. El texto MUST NOT decir que no hay diferencia.

#### Escenario: archivo contra sí mismo
- GIVEN un archivo de resultados
- WHEN se compara consigo mismo
- THEN cada métrica da 0/0/0 y p = 1

#### Escenario: seis casos mejoran
- GIVEN dos archivos donde R@5 mejora en 6 casos y no empeora en ninguno
- THEN `recall@5` informa p = 0,03125 y la línea primaria dice «distinguible»

#### Escenario: un caso mejora
- GIVEN dos archivos donde R@5 mejora en 1 caso
- THEN p = 1 y la línea primaria dice «no distinguible» y nombra los 6 casos

**Tests** (`diff.rs`): `self_diff_is_a_no_op`, `six_one_way_improvements_are_significant`, `one_improvement_is_not_distinguishable_and_says_how_many_are_needed`, `a_continuous_metric_carries_a_bootstrap_interval`, `the_verdict_never_says_there_is_no_difference`.

## R3. `--diff` ve agent-view, la abstención y la frescura

Cuando los dos archivos tienen `agent_view.per_case`, `--diff` MUST comparar, unidos por `query` y sólo sobre casos activos, `search.found`, `context.found`, los tokens de cada herramienta y la abstención de la búsqueda en los casos `unanswerable`. Cuando los dos tienen `retrieval.unanswerable` o casos con `fresh_first`, MUST comparar la abstención y la frescura. Si a uno de los dos le falta un bloque, ese bloque MUST NOT aparecer y el diff MUST NOT fallar.

#### Escenario: `found` cambia
- GIVEN dos archivos de agent-view iguales salvo un caso con `search.found` de `false` a `true`
- THEN `search_found` informa 1 mejor, 0 peores
- AND hoy el mismo par da 0/0/0

#### Escenario: menos tokens
- GIVEN un caso cuya búsqueda cuesta 100 tokens en el primero y 80 en el segundo
- THEN `search_tokens` lo cuenta como mejor

#### Escenario: un archivo viejo
- GIVEN un archivo sin `agent_view` y otro con el bloque
- THEN el diff no falla y no imprime el bloque

**Tests** (`diff.rs`): `a_found_flip_in_agent_view_shows_up`, `fewer_tokens_count_as_better`, `agent_view_joins_on_query_and_skips_known_red`, `a_file_without_agent_view_diffs_without_the_block`, `abstention_and_freshness_are_diffed_when_both_files_have_them`.

## R4. `--fail-on-regression`

Con `--diff` y `--fail-on-regression`, `rag-eval` MUST salir con 6 si una métrica primaria presente empeora (`worse > better`) con p < 0,05, y con 0 si no. Sin el flag MUST salir con 0 como hoy. El flag MUST NOT aceptarse sin `--diff`.

#### Escenario: regresión significativa
- GIVEN 6 casos que pierden R@5 y ninguno que gana
- THEN la salida es 6

#### Escenario: regresión chica
- GIVEN 5 casos que pierden R@5
- THEN la salida es 0

**Tests:** `a_significant_primary_regression_fails`, `five_lost_cases_do_not_fail`, `an_informative_regression_never_fails` (`diff.rs`); `fail_on_regression_parses_on_either_side_of_diff`, `fail_on_regression_without_diff_is_an_error` (`cli.rs`); `diff_exits_six_on_a_significant_regression_only_with_the_flag` (`main.rs`).

## R5. Aciertos estrictos y parciales con intervalo

El reporte de recuperación MUST incluir `hits` (aciertos estrictos y parciales en 1, 5 y 10, cada uno con su intervalo de Wilson) en el bloque global y en cada bloque de `by_lang`, `by_subset` y `by_split`. `aggregate` MUST NOT cambiar. La salida de consola MUST mostrar R@5 estricto y parcial con su intervalo.

#### Escenario: dos esperadas, una encontrada
- GIVEN un caso con dos esperadas y una sola en el top 5
- THEN cuenta 0 como acierto estricto en 5 y 1 como parcial, y su R@5 sigue siendo 0,5

**Tests** (`report.rs`): `a_case_with_one_of_two_expected_rows_is_a_partial_but_not_a_strict_hit`, `hits_carry_the_wilson_interval_of_their_count`, `every_split_block_carries_its_hits`.

## R6. Procedencia con el tamaño de efecto mínimo

`GoldenSetProvenance` MUST incluir `min_one_way_cases` y su nota MUST decir cuántos casos en una dirección hacen falta y cuántos puntos son sobre el número de casos del set. `provisional` MUST NOT cambiar.

#### Escenario: el set de 46
- GIVEN un golden de 46 casos y 28 pares
- THEN `min_one_way_cases` es 6 y la nota dice «13.0 points of 46»

**Test** (`report.rs`): `the_note_says_how_many_one_way_cases_a_conclusion_needs`.

## R7. Partición por hash

Cada caso MUST pertenecer a `calibration` o `test` según `split_of(query)`, y el reporte MUST incluir `by_split` con un bloque por partición presente. La asignación de un caso MUST NOT depender de los otros casos ni de su orden.

#### Escenario: agregar casos
- GIVEN 40 consultas y su partición
- WHEN se agregan 20 más
- THEN ninguna de las 40 cambia de lado

#### Escenario: proporción
- GIVEN 1.000 consultas distintas
- THEN entre el 65 % y el 75 % son `calibration`

**Tests** (`report.rs`): `split_of_is_stable_when_cases_are_added`, `about_seventy_percent_of_queries_calibrate`, `aggregate_by_split_scores_each_split_on_its_own_cases`.

## R8. Casos `unanswerable`

Un caso con `subset: "unanswerable"` MUST tener `expected_refs` vacía, y un caso de otra clase MUST NOT tenerla vacía. Su resultado MUST ir a `retrieval.unanswerable` con `abstained`, MUST NOT entrar en `per_case`, en R@k, MRR, nDCG, `similarity` ni `--follow-up`, y `retrieval.abstention` MUST dar la tasa con su intervalo. En agent-view MUST quedar fuera de `hit_rate` y llevar `abstained` (la búsqueda no imprimió líneas), con `agent_view.abstention`. Sin casos `unanswerable`, ninguno de esos bloques MUST aparecer.

#### Escenario: abstención correcta
- GIVEN un caso `unanswerable` cuya búsqueda devuelve una lista vacía
- THEN cuenta como abstención correcta y no aparece en R@5

#### Escenario: no se abstiene
- GIVEN un caso `unanswerable` cuya búsqueda devuelve filas
- THEN `abstained` es falso y R@5 global no cambia

#### Escenario: golden mal escrito
- GIVEN un caso `unanswerable` con una esperada, o un caso `natural` sin esperadas
- THEN la validación falla

**Tests:** `an_unanswerable_case_must_expect_nothing`, `only_an_unanswerable_case_may_expect_nothing` (`golden.rs`); `an_unanswerable_case_stays_out_of_the_ranked_metrics_and_reports_abstention` (`main.rs`); `abstention_is_absent_without_unanswerable_cases` (`report.rs`); `an_unanswerable_case_stays_out_of_hit_rate_and_reports_whether_search_abstained` (`agent_view/mod.rs`).

## R9. Casos `update`

Un caso con `subset: "update"` MUST tener exactamente una esperada y `stale_refs` no vacía, dentro de `corpus_refs` y sin la esperada; la esperada MUST tener `age_days` menor que cada vieja. `stale_refs` MUST NOT aparecer en otra clase. `age_days` MUST ser no negativa y, al sembrar, MUST restarse de `created_at`. El caso MUST seguir en `per_case` y en R@k, con `fresh_first`, y `retrieval.freshness` MUST dar la tasa con su intervalo.

#### Escenario: la nueva primero
- GIVEN un caso `update` cuya esperada queda en el puesto 2 y la vieja en el 4
- THEN `fresh_first` es verdadero

#### Escenario: la vieja primero
- GIVEN la vieja en el puesto 1 y la esperada en el 3
- THEN `fresh_first` es falso

#### Escenario: fechas al revés
- GIVEN un caso `update` cuya esperada tiene `age_days` 30 y la vieja 1
- THEN la validación falla

**Tests:** `an_update_case_needs_one_expected_and_older_stale_rows`, `stale_refs_outside_an_update_case_fail`, `a_negative_age_fails` (`golden.rs`); `fresh_first_needs_the_expected_row_before_every_stale_one` (`metrics.rs`); `seed_backdates_a_row_by_its_age` (`seed.rs`); `eval_insert_backdates_created_at` (`eval_api.rs`).

## R10. Un golden sin clases nuevas mide igual

Un golden sin `unanswerable`, `update`, `stale_refs` ni `age_days` MUST producir las mismas métricas, el mismo `content_sha256` y ningún bloque `unanswerable`, `abstention` ni `freshness`.

**Test** (`main.rs`): `a_golden_without_the_new_classes_measures_exactly_as_before`, que compara `aggregate` y `per_case` contra los valores fijados de un golden de fixture con `--fake`.

## R11. Brazo sólo léxico

`--lexical-only` MUST ordenar cada caso por `bm25` con la consulta FTS del canal léxico de producción, sin consultar vectores, y escribir un resultado con el tag `lexical-only` que `--diff` puede comparar con cualquier otro. MUST NOT combinarse con `--agent-view`, `--follow-up`, `--docs`, los barridos ni `--fts-tokenizer`.

#### Escenario: palabra exacta
- GIVEN dos filas, una con la palabra `zanzibar` y otra sin ella pero con un vector más cercano
- WHEN se busca `zanzibar` con `--lexical-only`
- THEN sólo aparece la fila con la palabra

**Tests:** `lexical_search_ranks_by_bm25_without_vectors` (`eval_api.rs`); `lexical_only_conflicts_with_the_other_arms` (`cli.rs`); `a_lexical_only_run_writes_a_tagged_file_that_diffs` (`main.rs`).

## R12. Los tests de `--diff` no leen archivos ignorados

Ningún test de `diff.rs` MUST leer un archivo bajo `docs/eval/`.

**Tests:** `self_diff_is_a_no_op`, `diff_isolates_similarity_from_ranked_movement` y `diff_files_reads_both_paths_and_produces_the_same_result_as_diff_values`, con un fixture en el código; pasan en una máquina sin `docs/eval/`.
