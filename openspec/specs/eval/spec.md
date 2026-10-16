# eval — Especificación

Fuente de verdad del comportamiento actual de la evaluación de recuperación: el generador sintético de `golden-mine synth`, la etiqueta de subconjunto del golden set y sus casos sin respuesta y de versión nueva, el reporte de `rag-eval` (por subconjunto, aciertos estrictos y parciales con intervalo, partición calibración/prueba), la comparación de dos corridas con `--diff`, el brazo sólo léxico, los intrusos del audit, el barrido de contradicciones, `rag-eval --docs` (la medición del índice de la documentación y el código de un proyecto, [`specs/project-docs`](../project-docs/spec.md)) y la baseline medida.

**Origen:**
- [eval-golden-expansion](../../changes/archive/2026-10-12-eval-golden-expansion/proposal.md), archivado el 2026-10-12.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Barrido de contradicciones" y "`rag-eval --docs`").
- [eval-rigor](../../changes/archive/2026-10-15-eval-rigor/proposal.md), archivado el 2026-10-15 (casos `unanswerable` y `update`, aciertos, procedencia, partición, `--diff`, brazo sólo léxico y el piso léxico de la baseline).

## Definiciones

- **Golden set:** el JSON que lee `rag-eval` (`docs/eval/golden.json`, fuera de git): `corpus`, `retrieval_cases` y pares. El parseo usa `deny_unknown_fields` en todos sus tipos (`crates/clawg-eval/src/bin/rag-eval/golden.rs:31`).
- **Subconjunto:** `CaseSubset` (`golden.rs:94`): `natural`, `lexical`, `multi_ref`, `high_importance`, `low_importance`, `long`, `update` o `unanswerable`. Los casos del golden set original son `natural`.
- **Set generado:** la salida de `golden-mine synth`: todas las filas, casos y pares de la base sin cambios, seguidos de lo que agrega el generador (`synth::generate`, `crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:69`).
- **Distractor:** fila sintética que ningún caso espera.
- **Caso activo:** caso que entra a las métricas ordenadas; los `known_red` y los `unanswerable` no entran.
- **Intruso:** la fila no esperada que aparece en el top-5 (`INTRUDER_DEPTH`, `crates/clawg-eval/src/bin/rag-eval/audit.rs:52`) de más casos activos de un subconjunto.
- **Caso movido:** en `--diff`, un caso unido por `query` en los dos archivos cuya métrica es mejor o peor en el segundo. En las métricas de recuperación, mejor es mayor; en tokens, menor.
- **p exacto:** el p de dos colas de la binomial exacta con p = 0,5 sobre los casos movidos, que en datos pareados es el test exacto de McNemar (`mcnemar_exact`, `crates/clawg-eval/src/bin/rag-eval/stats.rs:13`).
- **Métrica primaria:** R@5 de `retrieval` y `search.found` de `agent_view`. Las demás son informativas y nunca hacen fallar un diff.
- **Acierto estricto en k:** todas las esperadas en el top-k (`recall@k` = 1). **Parcial:** al menos una (`recall@k` > 0).
- **Partición:** `calibration` o `test`, según el sha256 de la consulta. Una constante nueva se elige leyendo `calibration` y se reporta sobre `test`.

## Generador sintético (`golden-mine synth`)

```
golden-mine synth --base <path> --out <path> --seed <N> [--distractors N]
```

### Determinismo

Con la misma base, la misma semilla y el mismo código, `synth` MUST producir un archivo idéntico byte a byte; con otra semilla MUST producir otro. El generador es puro: SplitMix64 propio (`synth/rng.rs`), sin dependencias nuevas, y `run_synth` (`golden-mine/main.rs:297`) vuelve antes de `production_base_dir`, así que MUST NOT leer `~/.clawg*` ni la red.

#### Escenario: misma semilla
- GIVEN dos corridas con semilla 7 y el mismo tamaño
- WHEN se comparan los archivos generados
- THEN son idénticos byte a byte

#### Escenario: otra semilla
- GIVEN corridas con semillas 7 y 8
- THEN los archivos difieren

**Tests:** `the_same_seed_gives_the_same_set_and_another_seed_a_different_one` (`synth/mod.rs`) y `the_same_seed_gives_identical_bytes_and_another_seed_does_not` (`crates/clawg-eval/tests/synth_roundtrip.rs`).

### La base no se toca

`--out` MUST ser distinto de `--base`, comparado por nombre (`parse_synth_args`, `golden-mine/main.rs:118`) y por ruta canónica (`run_synth`). La base sólo se lee. Al terminar, `synth` imprime el sha256 de la base y el de la salida.

#### Escenario: generador no toca lo natural
- GIVEN un `golden.json` existente
- WHEN se ejecuta el generador
- THEN el contenido de `golden.json` queda intacto

**Tests:** `the_base_file_is_left_untouched` y `writing_over_the_base_is_refused` (`synth_roundtrip.rs`), `synth_refuses_to_write_over_its_base` (`golden-mine/main.rs`).

### Composición

El tamaño es `--distractors` (`DEFAULT_DISTRACTORS` = 500, `synth/mod.rs:17`); es lo único que escala (`Sizes`, `synth/mod.rs:56`). La salida SHALL incluir:

- **Distractores** (500 por defecto). Cada uno mezcla términos de dos filas base del mismo `(memory_type, lang)`. Su largo MUST llegar a la mediana de largo de las filas base de su grupo, salvo que ya encadene 3 marcos (`MAX_CHAINED_FRAMES`). Ningún distractor MUST compartir una secuencia de 6 palabras seguidas con una fila base (`COPY_RUN_WORDS`, hasta `MAX_COPY_RETRIES` = 50 reintentos).
- **30 casos `lexical`** (`LEXICAL_FAMILIES`), en familias de 8 filas:
  - la esperada menciona el identificador al pasar, dentro de un marco distinto del que parafrasea la consulta;
  - los 7 hermanos usan ese marco, con identificadores del mismo prefijo cuyos dígitos son una permutación de los de la esperada o difieren en ±1 en el último dígito (`near_identifiers`, `synth/mod.rs:251`);
  - el identificador consultado MUST aparecer en una sola fila de todo el corpus de salida.
- **12 casos `multi_ref`** (`MULTI_REF_TOPICS`). Cada clave tiene 8 filas de aspectos que la nombran; la consulta pide 2 o 3 aspectos y sólo esas filas son `expected_refs`.
- **12 casos `high_importance` y 12 `low_importance`** (`CONTESTED_CASES`), cada uno sobre una clave con 6 filas de aspectos (`CONTESTED_POOL`) y una consulta por un aspecto:
  - en `high_importance`, la esperada tiene importancia en [8, 10] y las otras 5, en [3, 5];
  - en `low_importance`, la esperada tiene importancia en [3, 5] y las otras 5, en [8, 10].
- En total, al menos 20 filas con `importance` en [8, 10], de ellas al menos 10 con `importance >= 9`.

Toda `expected_ref` MUST existir en el corpus del caso. Los mínimos de casos y de filas de importancia se mantienen a cualquier tamaño, también con 0 distractores.

#### Escenario: conteos mínimos
- GIVEN una corrida con valores por defecto
- THEN hay >= 500 distractores, >= 30 casos léxicos, >= 10 multi-referencia, >= 10 `high_importance`, >= 10 `low_importance`, >= 20 filas con importancia 8-10 y >= 10 con >= 9

#### Escenario: hermano léxico
- GIVEN un caso léxico
- THEN el identificador consultado aparece sólo en la esperada
- AND cada hermano tiene un identificador del mismo prefijo, distinto del consultado, cuyos dígitos son una permutación o un ±1 en el último dígito
- AND el marco de la esperada no es el de los hermanos

#### Escenario: distractor sin copia
- GIVEN cualquier distractor
- THEN no comparte ninguna secuencia de 6 palabras seguidas con ninguna fila base
- AND su largo es >= la mediana de su grupo, o encadena 3 marcos

#### Escenario: entidad concurrida
- GIVEN un caso `multi_ref`, `high_importance` o `low_importance`
- THEN las filas que nombran su clave son más que las esperadas (8 en `multi_ref`, 6 en los de importancia), y las no esperadas tratan otro aspecto
- AND las importancias de la esperada y de las competidoras caen en los rangos de su subconjunto

#### Escenario: tamaño pequeño
- GIVEN `--distractors` menor que 500
- THEN sólo cambia el número de distractores

**Tests** (`synth/mod.rs`, mod tests): `the_default_size_adds_at_least_five_hundred_distractors`, `the_size_only_changes_how_many_distractors_are_added`, `there_are_at_least_thirty_lexical_cases_at_any_size`, `the_queried_identifier_appears_in_the_expected_row_and_nowhere_else`, `the_expected_row_uses_a_mention_frame_and_its_siblings_the_queried_frame`, `siblings_carry_identifiers_near_the_queried_one`, `near_identifiers_share_the_prefix_and_keep_the_digits_close`, `a_distractor_reaches_its_groups_median_length_or_chains_three_frames`, `no_distractor_shares_six_words_in_a_row_with_a_base_row`, `there_are_at_least_ten_multi_reference_cases_at_any_size`, `a_multi_reference_key_names_eight_rows_and_only_some_of_them_answer`, `the_rows_of_a_multi_reference_key_cover_different_aspects`, `there_are_enough_high_importance_rows_at_any_size`, `each_high_importance_case_expects_a_high_row_among_low_ones_on_the_same_entity` y `each_low_importance_case_expects_a_low_row_among_high_ones_on_the_same_entity`.

## Golden set de `rag-eval`

### Etiqueta de subconjunto

`RetrievalCase` MUST aceptar una clave opcional `subset` (`golden.rs:80`) con los ocho valores de `CaseSubset`, en snake_case. Si falta, el caso MUST valer `natural`. Una clave desconocida, también una mal escrita como `subsett`, y un valor desconocido MUST seguir siendo un error de parseo.

#### Escenario: golden actual sin etiqueta
- GIVEN el `golden.json` actual, sin `subset`
- THEN todos los casos son `natural`

#### Escenario: etiqueta explícita
- GIVEN un caso con `"subset": "lexical"` (o `"low_importance"`)
- THEN el caso es de ese subconjunto

#### Escenario: clave mal escrita
- GIVEN un caso con la clave `subsett` o el valor `lexcial`
- THEN el parseo falla

**Tests** (`golden.rs`, mod tests): `a_case_without_a_subset_is_natural`, `an_explicit_subset_is_parsed`, `every_subset_name_parses_in_snake_case` (lista los seis primeros valores; `update` y `unanswerable` se parsean en los tests de validación de abajo), `a_misspelled_subset_key_fails_to_parse` y `an_unknown_subset_value_fails_to_parse`.

### Casos `unanswerable`

Un caso `unanswerable` no tiene respuesta en el corpus: lo que se mide es si la búsqueda no devuelve nada.

- Un caso con `subset: "unanswerable"` MUST tener `expected_refs` vacía, y un caso de cualquier otra clase MUST NOT tenerla vacía, sea cual sea su estado (`golden.rs:343-347`). Si no, la validación falla (salida 2).
- En recuperación, el caso lleva `abstained` = el ranking quedó vacío (`crates/clawg-eval/src/bin/rag-eval/main.rs:443`) y va a `retrieval.unanswerable` (`report.rs:202`) antes de mirar su estado, así que también entra uno `known_red` (`main.rs:462`). MUST NOT entrar en `per_case`, R@k, MRR, nDCG, `by_subset`, `by_split`, `similarity` (`main.rs:261`) ni `--follow-up` (`main.rs:755`). `retrieval.abstention` (`report.rs:206`) da la tasa de abstención con su intervalo de Wilson.
- En agent-view, el caso lleva `abstained` = `search_memory` no imprimió líneas (`agent_view/runner.rs:193`), queda fuera de `hit_rate`, de los resúmenes de tokens y de `by_subset`, y `agent_view.abstention` (`agent_view/mod.rs:162`) da la tasa sobre los casos activos.
- Sin casos `unanswerable`, `retrieval.unanswerable`, `retrieval.abstention` y `agent_view.abstention` MUST NOT aparecer.

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

### Casos `update`

Un caso `update` tiene una fila nueva que repite un dato que dicen filas más viejas: la nueva debería salir primero.

- `CorpusRow` MUST aceptar `age_days` opcional (`golden.rs:54-55`, 0 si falta), y ninguna fila MUST tenerla negativa (`golden.rs:311-315`).
- `RetrievalCase` MUST aceptar `stale_refs` opcional (`golden.rs:88-89`). Sólo un caso `update` la puede tener. Ese caso MUST tener exactamente una esperada y `stale_refs` no vacía, dentro de `corpus_refs` y sin la esperada, y cada fila vieja MUST tener `age_days` mayor que la esperada; con edades iguales falla (`validate_update`, `golden.rs:361-384`).
- Al sembrar, `age_days` se pasa a milisegundos redondeados (`DAY_MS`, `seed.rs:89-94`) y se resta de `created_at` (`EvalStore::insert`, `crates/clawg-core/src/eval_api.rs:271-285`). Agent-view siembra por la ruta real de guardado y no lee `age_days`.
- El caso sigue en `per_case` y en R@k, y lleva `fresh_first`: la esperada está en el ranking y ninguna vieja sale antes (`metrics.rs:42-45`). `retrieval.freshness` (`report.rs:209`) da la tasa con su intervalo de Wilson, sobre los casos activos; `--follow-up` no la calcula.

Errores de validación: `E_NO_EXPECTED`, `E_UNANSWERABLE_EXPECTS`, `E_NEGATIVE_AGE`, `E_STALE_OUTSIDE_UPDATE`, `E_UPDATE_SHAPE` y `E_UPDATE_NOT_NEWER` (`golden.rs:233-251`).

#### Escenario: la nueva primero
- GIVEN un caso `update` cuya esperada queda en el puesto 2 y la vieja en el 4
- THEN `fresh_first` es verdadero

#### Escenario: la vieja primero
- GIVEN la vieja en el puesto 1 y la esperada en el 3
- THEN `fresh_first` es falso

#### Escenario: fechas al revés
- GIVEN un caso `update` cuya esperada tiene `age_days` 30 y la vieja 1
- THEN la validación falla

**Tests:** `an_update_case_needs_one_expected_and_older_stale_rows`, `stale_refs_outside_an_update_case_fail`, `a_negative_age_fails` (`golden.rs`); `fresh_first_needs_the_expected_row_before_every_stale_one` (`metrics.rs`); `freshness_counts_only_update_cases` (`report.rs`); `age_days_converts_to_whole_milliseconds` (`seed.rs`); `eval_insert_backdates_created_at` (`eval_api.rs`); `an_update_case_is_scored_and_reports_freshness` (`main.rs`).

### Un golden sin clases nuevas mide igual

Un golden sin `unanswerable`, `update`, `stale_refs` ni `age_days` MUST producir las mismas métricas y ningún bloque `unanswerable`, `abstention` ni `freshness`. Su `content_sha256` sale de los bytes crudos del archivo (`golden.rs:278`), así que no cambia.

**Tests** (`main.rs`): `a_golden_without_the_new_classes_measures_exactly_as_before`, y `an_unanswerable_case_stays_out_of_the_ranked_metrics_and_reports_abstention`, que comprueba que sumar un caso `unanswerable` deja `per_case` y `aggregate` idénticos. Con el set real, el ancla de eval-rigor reprodujo el `content_sha256` y todas las métricas de la anterior ([Ancla de regresión](#ancla-de-regresión)).

## Reporte de `rag-eval`

### Métricas por subconjunto

El reporte de recuperación MUST incluir `by_subset` (`report.rs:189`): un bloque por subconjunto presente, con `n_cases`, el mismo `Aggregate` que el global y sus `hits`, junto a `by_lang`. Se calcula sólo sobre los casos activos (`aggregate_by_subset`, `crates/clawg-eval/src/bin/rag-eval/report.rs:556`, que `main.rs` llama con `per_case`), así que los `known_red` y los `unanswerable` MUST NOT entrar. Los bloques van ordenados por el nombre serializado, para que dos corridas del mismo set den JSON comparable byte a byte.

Un resultado previo sin `by_subset`, o con casos sin `subset`, MUST seguir leyéndose (los casos sin etiqueta vuelven como `natural`) y comparándose con `--diff`.

#### Escenario: dos subconjuntos
- GIVEN casos `natural` y `lexical`
- THEN hay un bloque por cada uno, con R@5 y MRR calculados sólo sobre sus casos

#### Escenario: sólo natural
- GIVEN un golden sin casos etiquetados
- THEN hay un único bloque `natural` con las mismas cifras que el agregado global

**Tests** (`report.rs`, mod tests): `aggregate_by_subset_scores_each_subset_on_its_own_cases_only`, `with_only_natural_cases_the_single_bucket_equals_the_global_aggregate`, `a_results_file_without_by_subset_still_reads_and_diffs` y `a_case_result_without_a_subset_reads_back_as_natural`.

### Aciertos estrictos y parciales

El reporte de recuperación MUST incluir `hits` (`report.rs:182`): para k = 1, 5 y 10, los aciertos estrictos y los parciales, cada uno como `Rate` (`report.rs:457`: `hits`, `n_cases`, `rate`, `lo`, `hi`) con su intervalo de Wilson. Va en el bloque global y en cada bloque de `by_lang`, `by_subset` y `by_split`. `aggregate` MUST NOT cambiar: R@k sigue siendo la fracción de esperadas encontradas. La consola MUST mostrar `hits@5 strict k/n = r [lo, hi]  partial k/n = r [lo, hi]` (`format_hit`, `report.rs:576`).

#### Escenario: dos esperadas, una encontrada
- GIVEN un caso con dos esperadas y una sola en el top 5
- THEN cuenta 0 como acierto estricto en 5 y 1 como parcial, y su R@5 sigue siendo 0,5

**Tests** (`report.rs`): `a_case_with_one_of_two_expected_rows_is_a_partial_but_not_a_strict_hit`, `hits_carry_the_wilson_interval_of_their_count`, `every_split_block_carries_its_hits`.

### Tamaño de efecto mínimo en la procedencia

`GoldenSetProvenance` MUST incluir `min_one_way_cases` (`report.rs:258`), y su nota MUST terminar con `a paired comparison needs at least {m} cases moving one way ({p} points of {n})` (`report.rs:284-286`), donde `n` es el total de casos del golden (también los `known_red` y los `unanswerable`) y `p` = 100·m/n. Con `n` = 0 la nota omite el paréntesis. `provisional` MUST NOT cambiar.

#### Escenario: el set de 46
- GIVEN un golden de 46 casos y 28 pares
- THEN `min_one_way_cases` es 6 y la nota dice «13.0 points of 46»

**Test** (`report.rs`): `the_note_says_how_many_one_way_cases_a_conclusion_needs`.

### Partición calibración/prueba

Cada caso MUST pertenecer a `calibration` o a `test` según `split_of(query)` (`report.rs:142`): los primeros 16 dígitos hex del sha256 de la consulta, leídos como `u64`, módulo 10, son `calibration` si dan menos de `CALIBRATION_BUCKETS` = 7 (`report.rs:137`). La asignación MUST NOT depender de los otros casos ni de su orden, y no usa semilla. El reporte MUST incluir `by_split` (`report.rs:193`), un bloque por partición presente con `n_cases`, `aggregate` y `hits`, calibración primero, calculado sobre `per_case`. La consola lo encabeza con `retrieval by split (choose on calibration, report test):`. `similarity` y `agent_view` no tienen `by_split`.

#### Escenario: agregar casos
- GIVEN 40 consultas y su partición
- WHEN se agregan 20 más
- THEN ninguna de las 40 cambia de lado

#### Escenario: proporción
- GIVEN 1.000 consultas distintas
- THEN entre el 65 % y el 75 % son `calibration`

**Tests** (`report.rs`): `split_of_is_stable_when_cases_are_added`, `about_seventy_percent_of_queries_calibrate`, `aggregate_by_split_scores_each_split_on_its_own_cases`.

### Tests de resultados autocontenidos

Ningún test de `report.rs` ni de `diff.rs` MUST leer un archivo bajo `docs/eval/`: usan fixtures en el código (`reference_value`, `diff.rs:382`) o un directorio temporal.

**Tests:** `a_results_file_without_the_block_still_diffs_against_one_with_it` (`report.rs:1313`); `self_diff_is_a_no_op`, `diff_isolates_similarity_from_ranked_movement` y `diff_files_reads_both_paths_and_produces_the_same_result_as_diff_values` (`diff.rs`).

### Intrusos por subconjunto en el audit

Con `--audit`, el reporte MUST incluir `top_intruders` (`audit.rs:101`): para cada subconjunto, el intruso y la fracción de casos activos del subconjunto en que aparece (`top_intruders`, `audit.rs:326`). Una fila esperada de un caso MUST NOT contar como intrusa en ese caso, aunque sí puede serlo en otro. Un subconjunto sin intrusos no aparece. La métrica se reporta y MUST NOT cambiar el veredicto del audit.

#### Escenario: un intruso repetido
- GIVEN 4 casos `natural` y una fila `s-dis-0001` en el top-5 de 2 de ellos sin ser esperada
- WHEN se audita
- THEN el bloque `natural` nombra `s-dis-0001` con fracción 0,5

#### Escenario: la esperada no es intrusa
- GIVEN un caso cuya fila esperada está en el puesto 1
- THEN esa fila no cuenta como intrusa en ese caso

**Tests:** `a_row_in_the_top_five_of_half_the_cases_is_the_subsets_intruder`, `the_expected_row_is_never_its_own_cases_intruder`, `a_row_expected_by_one_case_still_intrudes_on_another` y `the_audit_carries_the_intruders_without_changing_the_verdict` (`audit.rs`, mod tests); `the_audit_names_an_intruder_for_every_generated_subset` (`synth_roundtrip.rs`).

## Comparar dos corridas (`--diff`)

```
rag-eval --diff <a.json> <b.json> [--fail-on-regression]
```

### Estadística

`stats.rs` MUST dar funciones puras y totales:

- `mcnemar_exact(better, worse)` (`stats.rs:13`): el p exacto, sumado en logaritmos para que miles de casos no lo lleven a 0, y nunca mayor que 1. Es simétrica, y `(0, 0)` da 1.
- `min_one_way_significant()` (`stats.rs:53`): la menor cantidad de casos que, moviéndose todos en una dirección, llega a p < `ALPHA` = 0,05 (`stats.rs:49`). No depende del tamaño del set, porque sólo cuentan los casos que se movieron, y da 6: `(0, 6)` da 0,03125 y `(0, 5)`, 0,0625.
- `wilson(k, n)` (`stats.rs:35`): el intervalo de Wilson al 95 % (`Z_95` = 1,959963985, `stats.rs:29`). `wilson(0, 0)` da 0 a 1.
- `paired_bootstrap_ci(deltas, resamples, seed)` (`stats.rs:74`): la media de las diferencias por caso y su intervalo percentil al 95 %, con `BOOTSTRAP_RESAMPLES` = 10.000 remuestreos con reposición (`stats.rs:65`) y la semilla fija `BOOTSTRAP_SEED` (`stats.rs:68`) sobre el mismo SplitMix64 de `golden-mine` (`stats.rs:94-110`), así que los mismos dos archivos dan siempre el mismo intervalo. Sin diferencias da `(0, 0, 0)`.

#### Escenario: valores conocidos
- WHEN se calcula `wilson(36, 46)`
- THEN da 0,644 a 0,877 (±0,001)
- AND `wilson(0, 10)` tiene límite superior 0,278
- AND `mcnemar_exact(0, 6)` = 0,03125, `(0, 5)` = 0,0625, `(1, 8)` = 0,0391 y `(1, 7)` = 0,0703

#### Escenario: bootstrap
- GIVEN la misma lista de diferencias y la misma semilla
- THEN dos llamadas dan el mismo intervalo
- AND diferencias todas 0 dan `(0, 0, 0)`, y diferencias todas positivas dan un límite inferior mayor que 0

**Tests** (`stats.rs`): `wilson_matches_known_intervals`, `wilson_of_no_cases_is_the_whole_range`, `mcnemar_exact_matches_known_p_values`, `mcnemar_exact_is_symmetric_and_never_above_one`, `mcnemar_exact_stays_finite_for_thousands_of_cases`, `bootstrap_is_deterministic_for_a_seed`, `bootstrap_of_zero_differences_is_zero`, `bootstrap_of_positive_differences_excludes_zero`, `six_cases_one_way_is_the_smallest_significant_count`, `splitmix_matches_the_golden_mine_sequence`.

### Qué compara

`--diff` une los casos de los dos archivos por `query` y MUST informar, por cada métrica, `better`, `worse`, `net` y el p exacto (`MetricDiff`, `diff.rs:26`; `p_value`, `diff.rs:39`):

- **`retrieval` y `similarity`:** R@1, R@5, R@10, MRR y nDCG@10. MRR y nDCG@10 llevan además la diferencia media (segundo menos primero) con su intervalo bootstrap, en los dos bloques (`diff_block`, `diff.rs:282`).
- **`agent_view`**, cuando los dos archivos tienen `agent_view.per_case`, sólo sobre casos activos (`diff_agent_view`, `diff.rs:250`): `search.found`, `context.found`, `search.tokens` y `context.tokens`. En tokens, menos es mejor, pero la media del intervalo es el cambio crudo, así que una media negativa es una mejora. Los tokens incluyen los casos `unanswerable`. Un caso cuenta como `unanswerable` si lo es en cualquiera de los dos archivos, y la fila `search.abstain` se imprime sólo si alguno se movió.
- **Abstención**, cuando los dos tienen `retrieval.unanswerable` (`diff_abstention`, `diff.rs:237`): abstenerse en el segundo es mejor.
- **Frescura**, cuando algún caso unido tiene `fresh_first` en los dos (`diff_freshness`, `diff.rs:210`).

Si a uno de los dos archivos le falta un bloque, ese bloque MUST NOT aparecer y el diff MUST NOT fallar.

Una fila se imprime como `{métrica}  {better}  {worse}  {net}  {p}`, seguida de `mean {media} [{lo}, {hi}]` en las que llevan intervalo (`print_metric`, `diff.rs:367-372`). Con cero casos unidos, el intervalo se imprime igual, en cero.

### Veredicto

Al final, bajo `primary:`, MUST imprimirse siempre la línea de `retrieval recall@5`, haya o no casos de recuperación en los archivos, y la de `agent_view search.found` cuando existe el bloque (`diff.rs:350-354`). El texto sale de `verdict` (`diff.rs:55`):

- con p < 0,05: `{nombre}: distinguishable, better|worse (p = {p}, {b} better / {w} worse)`;
- si no: `{nombre}: not distinguishable with {movidos} case(s) moving (p = {p}); a conclusion needs at least 6 cases moving one way`. El 6 es el mínimo cuando ningún caso se mueve en la otra dirección; con movimiento mixto hacen falta más, y la línea no lo dice.

El texto MUST NOT decir que no hay diferencia, ni que las corridas son iguales.

#### Escenario: archivo contra sí mismo
- GIVEN un archivo de resultados
- WHEN se compara consigo mismo
- THEN cada métrica da 0/0/0 y p = 1

#### Escenario: seis casos mejoran
- GIVEN dos archivos donde R@5 mejora en 6 casos y no empeora en ninguno
- THEN `recall@5` informa p = 0,03125 y la línea primaria dice «distinguishable»

#### Escenario: un caso mejora
- GIVEN dos archivos donde R@5 mejora en 1 caso
- THEN p = 1 y la línea primaria dice «not distinguishable» y nombra los 6 casos

#### Escenario: `found` cambia
- GIVEN dos archivos de agent-view iguales salvo un caso con `search.found` de `false` a `true`
- THEN `search.found` informa 1 mejor, 0 peores

#### Escenario: menos tokens
- GIVEN un caso cuya búsqueda cuesta 100 tokens en el primero y 80 en el segundo
- THEN `search.tokens` lo cuenta como mejor

#### Escenario: un archivo viejo
- GIVEN un archivo sin `agent_view` y otro con el bloque
- THEN el diff no falla y no imprime el bloque

**Tests** (`diff.rs`): `self_diff_is_a_no_op`, `six_one_way_improvements_are_significant`, `one_improvement_is_not_distinguishable_and_says_how_many_are_needed`, `a_continuous_metric_carries_a_bootstrap_interval`, `the_verdict_never_says_there_is_no_difference`, `a_found_flip_in_agent_view_shows_up`, `fewer_tokens_count_as_better`, `agent_view_joins_on_query_and_skips_known_red`, `a_file_without_agent_view_diffs_without_the_block`, `abstention_is_diffed_when_both_files_have_it` y `freshness_is_diffed_when_both_files_have_update_cases`.

### `--fail-on-regression`

Con `--diff` y `--fail-on-regression`, antes o después de `--diff`, `rag-eval` MUST salir con 6 (`EXIT_REGRESSION`, `main.rs:58`) si una métrica primaria presente empeora (`worse > better`) con p < 0,05 (`regressed`, `diff.rs:46-49`), e imprimir por stderr `a primary metric got worse with p < 0.05`. Si no, sale con 0. Sin el flag MUST salir con 0. Las métricas informativas nunca cuentan: probarlas todas haría probable una falsa alarma.

El flag MUST NOT aceptarse sin `--diff`: es un error de uso, que sale con 1. Si además falta el modo, el error que se informa es el de falta de modo (`cli.rs:367`). Lo que venga después de `--diff a b`, salvo el flag, se ignora.

#### Escenario: regresión significativa
- GIVEN 6 casos que pierden R@5 y ninguno que gana
- THEN la salida es 6

#### Escenario: regresión chica
- GIVEN 5 casos que pierden R@5
- THEN la salida es 0

**Tests:** `a_significant_primary_regression_fails`, `five_lost_cases_do_not_fail`, `an_informative_regression_never_fails` (`diff.rs`); `fail_on_regression_parses_on_either_side_of_diff`, `fail_on_regression_without_diff_is_an_error` (`cli.rs`); `diff_exits_six_on_a_significant_regression_only_with_the_flag` (`main.rs`).

## Brazo sólo léxico (`--lexical-only`)

```
rag-eval (--fake | --real) --lexical-only [--tag <slug>]
```

`--lexical-only` MUST ordenar cada caso por el canal léxico solo: la misma consulta FTS5 y el mismo orden `bm25` que la búsqueda completa (`lexical_candidates`, `crates/clawg-core/src/memory/search.rs:705`), con la consulta tal cual, sin vectores ni puntaje, y hasta 10 filas (`EvalStore::lexical_search`, `crates/clawg-core/src/eval_api.rs:345`). Si FTS falla, el caso queda sin filas, sin aviso.

El resultado MUST llevar el tag `lexical-only` (`LEXICAL_ONLY_SLUG`, `main.rs:46`), y con `--tag X`, `X-lexical-only`, en un archivo que `--diff` compara con cualquier otro. `similarity` se sigue calculando. El flag MUST NOT combinarse con `--agent-view`, `--follow-up`, `--docs`, los barridos ni `--fts-tokenizer` (`cli.rs:403-413`). Con `--audit` sí se puede.

#### Escenario: palabra exacta
- GIVEN dos filas, una con la palabra `zanzibar` y otra sin ella pero con un vector más cercano
- WHEN se busca `zanzibar` con `--lexical-only`
- THEN sólo aparece la fila con la palabra

**Tests:** `lexical_search_ranks_by_bm25_without_vectors` (`eval_api.rs`); `lexical_only_conflicts_with_the_other_arms` (`cli.rs`); `a_lexical_only_run_writes_a_tagged_file_that_diffs` (`main.rs`).

## Subconjunto `long`

Mide si la respuesta que cae dentro de un save largo llega al agente. Viene de [long-save-context](../../changes/archive/2026-10-14-long-save-context/proposal.md), R9 de esa spec.

### Generador del subconjunto `long`

`golden-mine synth` MUST agregar, después de todo lo que agrega hoy y sin cambiar nada de eso (`long_saves`, `crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:421`, llamado al final de `generate`), 12 casos `long` (6 en inglés y 6 en español):

- cada caso tiene una fila propia de al menos 3 pedazos, medidos sin tokenizer contra el tope con el que se definió el subconjunto y no contra el de producción: el generador alarga la fila hasta `LONG_MIN_PIECES` = 4 pedazos de `LONG_PIECE_TOKENS` = 424 tokens (`synth/mod.rs:35-39`), cortada con el solapamiento de un chunk (64) por `eval_api::fallback_pieces(text, max_tokens)` (`crates/clawg-core/src/eval_api.rs:69`). Así el tope de producción ([`specs/pieces`](../pieces/spec.md), R2) se puede mover sin mover el set que lo mide: base y punta miden las mismas filas, y el hecho queda a dos tercios del texto;
- la fila nombra la clave del caso sólo en su primera oración, y el dato que responde la consulta (`answer`) aparece una sola vez, después del primer tercio del texto;
- el relleno (`templates::long_filler`, `synth/templates.rs:284`) no nombra la clave y no repite una secuencia de 6 palabras de una fila de la base. Entre dos filas `long` lo que se exige es que no compartan ningún pedazo: el dedup los absorbería y el caso mediría otra cosa. Las filas `long` salen de las mismas plantillas, así que entre ellas sí pueden repetirse frases cortas;
- el caso lleva `answer`: el dato tal como aparece en la fila.

Con la misma base y la misma semilla, las filas y casos que el generador agrega hoy MUST salir idénticos. Como el archivo crece, el set de la semilla 7 pasa de 1.028 filas y 112 casos a 1.040 filas y 124 casos, y su sha256 cambia.

`RetrievalCase` MUST aceptar `answer` opcional (`golden.rs:85`), y `CaseSubset` el valor `long` (`golden.rs:101`). Un golden sin esas claves MUST parsear igual que antes.

#### Escenario: el dato después del corte
- GIVEN un caso `long` con clave `amber-falcon` y `answer` `"retry budget of 7 attempts"`
- THEN su fila nombra `amber-falcon` sólo en la primera oración, `retry budget of 7 attempts` aparece una vez después del primer tercio, y la fila se parte en 3 pedazos o más

**Tests:** `there_are_twelve_long_cases_at_any_size`, `a_long_row_names_its_key_only_in_its_first_sentence`, `the_answer_appears_once_after_the_first_third`, `a_long_row_splits_into_at_least_three_pieces`, `adding_long_cases_leaves_the_rest_of_the_set_unchanged` y `long_rows_do_not_depend_on_the_piece_cap` (fija el digest de las 12 filas `long` de la base de los tests con `--seed 7`) (`synth/mod.rs`); `a_long_case_writes_its_subset_and_answer` (`synth/golden.rs`); `an_answer_and_the_long_subset_parse` (`rag-eval/golden.rs`).

### Métricas de agent-view para `answer`

`rag-eval --agent-view` MUST reportar, para los casos con `answer` (`crates/clawg-eval/src/bin/rag-eval/agent_view/mod.rs`, `AnswerStats`):

- **respuesta visible:** el texto que devolvió la herramienta contiene `answer`, comparado con los espacios normalizados (`normalize`, `agent_view/matcher.rs:22`);
- **respuesta alcanzable** (sólo `search_memory`): visible, o visible en el save que devuelve `GET /v1/memories/memory/{id}/save` para la primera línea que carga la fila esperada. Contra un daemon sin esa ruta, alcanzable es igual a visible;
- **pedazos repetidos:** cuántas respuestas de `search_memory` muestran dos líneas del mismo save (dos líneas con el mismo id, o con filas del mismo caso);
- **por subconjunto:** las métricas de agent-view separadas por `subset` (`by_subset`, `SubsetSummary`), así cada subconjunto se compara por separado.

`SearchLine` (`agent_view/parse.rs:11`) lleva el `id` de la línea que imprimió la herramienta.

**Tests:** `the_answer_counts_only_when_its_text_is_visible`, `a_read_save_makes_a_hidden_answer_reachable`, `a_line_repeating_an_earlier_lines_row_is_counted` y `build_results_splits_active_cases_by_subset_with_their_answers` (`agent_view/mod.rs`); `a_case_that_names_its_answer_reports_whether_the_answer_reached_the_agent` (`agent_view/runner.rs`).

### Criterio de la medición `long`

Medido con `rag-eval --real --agent-view` sobre el set de la semilla 7 generado por la punta (base `db0c6a1`, punta `f060593`). Detalle en el [`verify.md` archivado](../../changes/archive/2026-10-14-long-save-context/verify.md#u2-u3-y-u4).

| # | Condición | Resultado medido |
|---|---|---|
| U1 | `rag-eval --diff` del ancla sin cambios en retrieval | Cumplido (0 mejores, 0 peores) |
| U2 | `long`: `answers.search_reachable` >= 0,75 y al menos 0,25 por encima de la base | **No cumplido**: 0,333 contra 0,000 de la base. Aceptado al archivar |
| U3 | `long`: `repeated_rows` = 0 | Cumplido (0; `natural` baja de 3 a 0) |
| U4 | los otros cinco: ninguna métrica de agent-view baja más de 0,02 | Cumplido |

U2 no se cumple, y se registra tal cual; el umbral no se movió. La respuesta alcanzable sube de 0,000 a 0,333: en 4 de 12 casos el save aparece en `search_memory` (puestos 7 a 9 de 10) y en los 8 restantes queda fuera del top 10. Las filas cortas del set, densas en las palabras del tema, le ganan a un pedazo de hasta 424 tokens. Cuando el save aparece, `read_memory` hace su parte: la respuesta es alcanzable. El límite está en el ranking, que queda como `long-save-ranking` en el [mapa de openspec](../../README.md).

[long-save-ranking](../../changes/archive/2026-10-15-long-save-ranking/proposal.md) lo cierra con pedazos de 96 tokens y un pozo de KNN que cuenta saves ([`specs/pieces`](../pieces/spec.md), R2 y R9). Medido sobre el mismo set de la semilla 7 (sha `98ea356e…`, idéntico con los dos binarios), base `9aa1012` (con el pooling mean y el corte de relevancia de 0,35) y punta `04ab3cd`. Detalle en su [`verify.md` archivado](../../changes/archive/2026-10-15-long-save-ranking/verify.md#u2-y-u3).

| # | Condición | Resultado medido |
|---|---|---|
| U0 | registrar `long` de la base | `search_reachable` 0,833, `search_visible` 0,000, `context_visible` 0,500 |
| U1 | el ancla no empeora | Cumplido (`--diff` vacío; agent-view sin diferencias, caso por caso) |
| U2 | `long`: `search_reachable` ≥ 0,917, `search_visible` ≥ 0,5 y `context_visible` ≥ 0,75 | Cumplido: 1,000, 0,917 y 0,917 |
| U3 | los otros cinco: ninguna métrica de agent-view baja más de 0,02; `repeated_rows` = 0 | Cumplido (ningún hit rate cambia; 0 filas repetidas) |

Queda un caso: `s-long-06` sale primero en `search_memory`, pero con un pedazo que no tiene el hecho. Se alcanza con `read_memory`.

## Barrido de contradicciones

El reporte de `rag-eval` MUST incluir la curva del umbral de contradicción: para cada umbral L2 de `SWEEP` (`crates/clawg-eval/src/bin/rag-eval/metrics.rs:101`: 0,20, 0,25, 0,30, 0,35, 0,40, 0,45 y 0,50), la fracción de pares de contradicción a esa distancia o menos (recall), la de pares no contradictorios (`fpr_no_contradiction`) y la de pares idénticos (`fpr_identical`). Un grupo sin pares MUST dar 0 y no `NaN` (`threshold_curve`). El rango cubre el umbral del detector (`CONTRADICTION_THRESHOLD`, [`specs/disputes`](../disputes/spec.md)) con margen a los dos lados.

**Tests:** `threshold_curve_hand_picked_buckets`, `threshold_curve_empty_buckets_are_zero_not_nan` y `threshold_curve_uses_sweep_constants_in_order` en `crates/clawg-eval/src/bin/rag-eval/metrics.rs`.

## `rag-eval --docs`: medir el índice de documentación

```
rag-eval (--fake | --real) --docs [--root <dir>] [--golden <path>]
```

Mide docs y código juntos ([`specs/project-docs`](../project-docs/spec.md)) y MUST:

- indexar la raíz (por defecto, el workspace de `clawg-eval`: `default_docs_root`, `cli.rs:235`) en un store temporal con el embedder elegido, que se borra al terminar. No toca el store de memorias de ningún perfil; con `--real` el modelo se toma del directorio de datos que resuelve `production_base_dir` (`main.rs:74`), el mismo caché que usa el resto de `rag-eval --real`;
- leer el golden set de docs (por defecto `crates/clawg-eval/golden/project-docs.json`, `default_docs_golden_path`, `cli.rs:230`; `deny_unknown_fields`): una lista de `{ "id", "question", "expected": [{ "path", "heading"? }] }`, con ids únicos, preguntas no vacías y al menos un `expected` por pregunta;
- contar un acierto cuando un hit tiene la ruta esperada y, si se da `heading`, el último tramo de su breadcrumb igual a ese encabezado (`matches`, `crates/clawg-eval/src/bin/rag-eval/docs.rs:86`); la posición de una pregunta es la del primer hit que acierta con cualquiera de sus `expected`, entre los 10 primeros;
- reportar por stdout Hit@1, Hit@3, Hit@5 y MRR@10 para todas las preguntas y, aparte, para las de docs y las de código (según la primera ruta esperada), el tiempo de indexado, la cantidad de docs, de pasajes y de salteados, y el detalle de cada pregunta (para una que no acierta primera, qué esperaba y qué salió primero). No escribe archivo de resultados, así que no hay `--diff` de docs;
- rechazar `--docs` junto a `--audit`, `--follow-up`, `--agent-view`, `--lexical-only` o los flags de sweep (`--sweep-imp`, `--sweep-cos-floor`, `--sweep-lexical`, `--fts-tokenizer`), y `--root` sin `--docs`; el rechazo es un error de uso (salida 1);
- salir con 2 si el golden set no se puede leer o no valida, o si un `expected` nombra un doc que el corpus no tiene (`check_corpus`, `docs.rs:76`): así un doc renombrado rompe la medición en vez de bajarla en silencio.

El golden set de docs se versiona porque pregunta sobre los docs públicos de este repo, a diferencia de `docs/eval/golden.json`. Sus preguntas evitan las palabras del pasaje que las responde, para medir sentido y no coincidencia léxica; cuando el mismo contenido vive en dos archivos, cualquiera cuenta.

**Tests:** `the_docs_golden_set_parses_and_rejects_unknown_fields`, `hits_count_when_path_and_heading_match`, `an_expected_doc_missing_from_the_corpus_fails_validation`, `the_report_scores_docs_and_code_apart` y `the_shipped_golden_set_names_docs_this_workspace_has` en `crates/clawg-eval/src/bin/rag-eval/docs.rs`; `docs_reads_its_own_golden_set_and_this_workspace_by_default`, `docs_takes_a_root_and_a_golden_set`, `root_without_docs_is_an_error` y `docs_with_another_mode_is_an_error` en `crates/clawg-eval/src/bin/rag-eval/cli.rs`; `a_doc_index_answers_from_the_root_it_indexed` en `crates/clawg-core/src/eval_api.rs`; `docs_mode_runs_end_to_end_with_the_fake_embedder` y `docs_mode_refuses_a_golden_set_naming_a_doc_the_root_lacks` en `crates/clawg-eval/tests/docs_mode.rs`.

## Baseline medida

Estas cifras no son tests de `cargo`: se miden a mano con `rag-eval --real`, que necesita el modelo. La medición completa está en el [`verify.md` archivado](../../changes/archive/2026-10-12-eval-golden-expansion/verify.md#medición-rag-eval---real). Las cifras de la semilla 7 de abajo son de antes de [long-save-context](../../changes/archive/2026-10-14-long-save-context/proposal.md); el set actual tiene 1.040 filas y 124 casos. Todas son, además, de antes del pooling mean ([`specs/storage`](../storage/spec.md)): los vectores eran CLS y el piso de coseno 0,8793, así que no se reproducen con el embedder de hoy.

### Ancla de regresión

Medir con `rag-eval --real` el `golden.json` actual, sin filas sintéticas, MUST reproducir R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524 y nDCG@10 0,6215 (n = 46, todo `natural`), con el pooling mean y el piso de coseno 0,79 que se entregan hoy ([`specs/retrieval`](../retrieval/spec.md), «El piso de coseno»). Medido en Windows el 2026-10-15 sobre `fc0606d`, en `docs/eval/results/2026-10-15-real-project-indexing-tip-anchor.json`. Un cambio que toque el ranking compara contra ese archivo con `--diff`.

Reproducido el 2026-10-15 sobre `210c692`, al archivar eval-rigor, en `2026-10-16-real-eval-rigor-tip-anchor.json` (`rag-eval` fecha los archivos en UTC). El `content_sha256` es el mismo (`d2d9d732…`), y el `--diff` contra el anterior no mueve ningún caso en ninguna métrica. El archivo nuevo trae además lo que el anterior no tiene:

- `hits`: hits@5 estricto 30/46 = 0,6522 [0,508, 0,773], igual al parcial porque cada caso espera una sola fila;
- `by_split`: calibración, 33 casos con R@5 0,6667; prueba, 13 con R@5 0,6154.

Los dos archivos sirven de ancla, pero para leer `by_split` hace falta el nuevo.

### Piso léxico

`rag-eval --real --lexical-only` sobre el mismo `golden.json`, el 2026-10-15 y sobre `210c692` (`2026-10-16-real-lexical-only.json`), da R@1 0,3478, R@5 0,5000, R@10 0,5000, MRR 0,4040 y nDCG@10 0,4279. En inglés R@5 es 0,2727 y en español 0,7083. Contra el ancla:

| Métrica | Mejores con el piso | Peores con el piso | p |
|---|---|---|---|
| R@10 | 0 | 16 | < 0,0001 |
| MRR | 6 | 21 | 0,0059 (media −0,1485 [−0,2643, −0,0333]) |
| R@5 | 3 | 10 | 0,0923 |

El ranking completo es mejor en R@10 y en MRR. En R@5 la diferencia no se distingue con 46 casos. El número absoluto de un cambio de ranking se lee contra este piso.

La referencia anterior, R@5 0,7826 y MRR 0,5979, era con vectores CLS y el piso 0,8793, y el código de hoy ya no la puede reproducir. La baja la trajo el pooling mean y es toda de las consultas en inglés: en español R@5 pasa de 0,875 a 0,833 y MRR de 0,7153 a 0,7167; en inglés R@5 cae de 0,6818 a 0,4545 y MRR de 0,4698 a 0,3733. Se aceptó al archivar project-indexing ([`verify.md`](../../changes/archive/2026-10-15-project-indexing/verify.md)).

### Set de la semilla 7

`golden-mine synth --seed 7` con valores por defecto da 1028 filas y 112 casos (sha256 `6312586c…d228c`). Es la baseline de los cambios que dependen de este, como lexical-fusion.

| Subconjunto | n | R@5 | MRR | nDCG@10 |
|---|---|---|---|---|
| global | 112 | 0,5387 | 0,3925 | 0,4760 |
| natural | 46 | 0,3696 | 0,2194 | 0,2713 |
| lexical | 30 | 0,4333 | 0,3825 | 0,5214 |
| multi_ref | 12 | 0,7778 | 0,7465 | 0,7686 |
| high_importance | 12 | 1,0000 | 0,7292 | 0,7968 |
| low_importance | 12 | 0,7500 | 0,3898 | 0,5342 |

### Criterio de salida

| # | Condición | Resultado medido |
|---|---|---|
| C1 | El ancla se reproduce | Cumplido |
| C2 | `lexical`: R@5 <= 0,70 | Cumplido (0,4333; es `B` de lexical-fusion) |
| C3 | `multi_ref`: R@5 y nDCG@10 <= 0,90 | Cumplido (0,7778 / 0,7686) |
| C4 | MRR de `high_importance` y de `low_importance` <= 0,90; high − low >= 0,15 por defecto, y \|high − low\| <= 0,05 con `--sweep-imp 1.0:1.0` | No cumplido: 0,3394 por defecto, −0,0729 aplanada. Aceptado |
| C5 | En `natural`, ningún intruso sintético en más del 10 % de los casos | No cumplido: `s-lex-19-0` en el 13,0 %. Aceptado |

Además se anota, sin umbral: con el canal léxico en 20 frente a 0, **0 de 112** casos cambian de ranking.

Si una condición falla, el resultado MUST registrarse tal cual; los umbrales MUST NOT ajustarse después de medir. C4 y C5 quedaron aceptados por decisión del usuario, sin mover umbrales ni iterar el generador.

## Invariantes

- El generador, el reporte, la estadística y `--diff` son lógica pura, separada de la E/S: se prueban sin modelo, sin red y sin `docs/eval/`.
- `--diff` es determinista: el bootstrap usa una semilla fija, y los mismos dos archivos dan siempre la misma salida.
- Producción nunca enlaza esto: `clawg-eval` usa `clawg-core` a través de la feature `eval`.
