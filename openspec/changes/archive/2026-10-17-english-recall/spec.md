# english-recall — Especificación

Delta sobre [`specs/eval`](../../../specs/eval/spec.md). Suma a `rag-eval` un eje por pareja consulta–fila: el idioma de la consulta en el golden, un bloque `by_pair` en `retrieval`, `similarity` y `agent_view`, y un corte por pareja en `--diff`. No cambia ningún ranking, ninguna métrica que ya existe ni el comportamiento de `clawg-core`. Todo el código está en `clawg-eval`.

Fuera de esta especificación, y en la [propuesta](proposal.md) (sección «Cómo se mide») y en `verify.md`: las etiquetas de `docs/eval/golden.json`, `golden-cross.json` y los goldens de control, el paso 0, las hipótesis (a) a (f), los umbrales U1 a U10 y el registro del diagnóstico. Lo que sigue describe sólo las herramientas con que se mide.

## Definiciones

- **Pareja:** `(query_lang, lang)` de un caso. `query_lang` es la clave opcional del caso; `lang` es lo que da `case_lang` sobre sus filas esperadas (`crates/clawg-eval/src/bin/rag-eval/main.rs:1221`), o sea `CaseResult.lang` (`report.rs:81`). Un caso al que le falta alguno de los dos **no tiene pareja**, igual que hoy `by_lang` deja afuera un caso sin idioma unánime (`report.rs:528-538`); sigue contando en el agregado global y en los demás bloques.
- **Caso con la misma pareja en los dos archivos:** en `--diff`, un caso unido por `query` (`diff.rs:283`, `:289`) cuya pareja es `Some` e igual en el primer y en el segundo archivo.
- **Pareja que difiere:** un caso unido por `query` que tiene pareja en los dos archivos y cuyas parejas no son iguales.
- **Pareja en un solo archivo:** un caso unido por `query` que tiene pareja en uno de los dos archivos y ninguna en el otro (por ejemplo, un archivo viejo contra uno nuevo).
- Un caso sin pareja en ninguno de los dos archivos no entra en ningún bloque ni en ningún conteo. Es la misma regla de `by_lang`, que deja afuera lo que no puede atribuir (`report.rs:528-534`) y no abre un cubo «desconocido».
- **Test RED:** falla hoy. Por **comportamiento** (corre y da otro resultado) o porque **no compila** (usa un campo o una función que no existe). Un test **guarda** pasa hoy y protege algo que no debe cambiar.

## R1. `query_lang` en el golden

`RetrievalCase` (`golden.rs:58-90`) MUST aceptar una clave opcional `query_lang` (texto), con `#[serde(default)]`. Si falta, el caso MUST valer `None`. La clave no se valida más que como texto, igual que `lang` de una fila (`golden.rs:50`): no se detecta ni se compara con un catálogo. Una clave desconocida, también una mal escrita como `query_lnag`, MUST seguir siendo un error de parseo (`deny_unknown_fields`, `golden.rs:59`). `query_lang` MUST NOT entrar en la siembra ni en lo que se embebe: es una anotación.

#### Escenario: caso con idioma de consulta
- GIVEN un caso con `"query_lang": "es"`
- THEN el parseo da `Some("es")` para ese caso
- AND hoy el parseo falla, porque `RetrievalCase` no declara la clave

#### Escenario: golden actual sin la clave
- GIVEN un caso sin `query_lang`
- THEN `query_lang` es `None` y el golden valida igual que antes

#### Escenario: clave mal escrita
- GIVEN un caso con la clave `query_lnag`
- THEN el parseo falla

**Tests** (`golden.rs`): `a_case_may_name_the_language_of_its_query` (**RED**, comportamiento: `deny_unknown_fields` en `golden.rs:59` rechaza la clave), `a_case_without_query_lang_has_none` y `a_misspelled_query_lang_key_fails_to_parse` (**guarda**: pasan hoy; el segundo, por la misma regla que ya cubre `a_misspelled_subset_key_fails_to_parse`).

## R2. La pareja de un caso y su paso a los resultados

`report.rs` MUST dar una función pura `pair_of(query_lang: Option<&str>, lang: Option<&str>) -> Option<Pair>` que devuelve `Some` sólo si los dos están. `Pair` lleva `query_lang` y `lang` y se ordena por `(query_lang, lang)`.

`CaseResult` (`report.rs:62-92`) MUST llevar `query_lang: Option<String>`, con `#[serde(default, skip_serializing_if = "Option::is_none")]` como `abstained` y `fresh_first` (`report.rs:87-91`): un archivo viejo sin la clave se sigue leyendo, y un caso sin `query_lang` no la escribe. `rag-eval` MUST copiar `query_lang` del caso a su `CaseResult` en `retrieval.per_case` (`main.rs:446`, `:861`) y en `similarity.per_case` (`main.rs:1141`), junto a `lang`, para que el bloque `similarity` agrupe por pareja sin otro dato. `EmbeddedCase` (`main.rs:949`) y `SimCaseSource` (`main.rs:1096`) MUST llevarlo.

#### Escenario: pareja completa
- GIVEN `query_lang` `es` y `lang` `en`
- THEN `pair_of` da `Some` con esa pareja

#### Escenario: falta uno
- GIVEN `query_lang` `None`, o `lang` `None`
- THEN `pair_of` da `None`

#### Escenario: el idioma llega a los dos bloques
- GIVEN un caso con `query_lang` `es` cuyas filas esperadas son `en`
- THEN su `CaseResult` en `retrieval.per_case` y en `similarity.per_case` lleva `query_lang` `es` y `lang` `en`

#### Escenario: sin idioma de consulta no se escribe
- GIVEN un `CaseResult` con `query_lang` `None`
- THEN su JSON no tiene la clave `query_lang`
- AND un JSON viejo sin la clave se lee con `query_lang` `None`

**Tests:** `pair_of_needs_both_languages` y `a_case_result_without_query_lang_neither_writes_nor_requires_it` (`report.rs`); `the_query_language_reaches_the_retrieval_and_similarity_cases` (`main.rs`, sobre `build_similarity_results` y la construcción de `CaseResult` de recuperación). Todos **RED, no compilan** (no existen `pair_of`, `Pair` ni `CaseResult.query_lang`).

## R3. `by_pair` en `retrieval` y `similarity`

`RetrievalResults` (`report.rs:176-210`) y `SimilarityResults` (`report.rs:334-368`) MUST incluir `by_pair`: una lista con un bloque por pareja presente, con `query_lang`, `lang`, `n_cases`, el mismo `Aggregate` que el global y sus `hits` (con intervalo de Wilson), en la forma de `by_lang` (`report.rs:99-105`). Se calcula sobre los mismos casos que el agregado de su bloque: `per_case` de recuperación (que ya deja afuera los `known_red` y los `unanswerable`) y `per_case` de `similarity` (que se arma sin ellos, `main.rs:261`). Los bloques van ordenados por `(query_lang, lang)`, para que dos corridas del mismo set den JSON comparable byte a byte (como `aggregate_by_lang`, `report.rs:533-534`).

Un caso sin pareja MUST NOT aparecer en ningún bloque de `by_pair` y MUST seguir contando en `aggregate`, `hits`, `by_lang`, `by_subset` y `by_split`: por eso los `n_cases` de `by_pair` no suman `per_case.len()`. Si ningún caso tiene pareja, `by_pair` MUST NOT aparecer en el JSON (`skip_serializing_if`), y `aggregate`, `per_case` y los demás bloques MUST NOT cambiar. `SimilarityResults` hoy no tiene `by_lang` ni `hits`: `by_pair` los lleva dentro de cada bloque, y nada más se agrega al agregado de `similarity`.

La consola MUST sumar, bajo cada bloque que tenga `by_pair`, una tabla con una fila por pareja (`n`, R@1, R@5, R@10, MRR, nDCG@10), en la forma de `format_subset_table` (`report.rs:581`). Sin parejas no imprime nada.

#### Escenario: dos parejas por el idioma de la consulta
- GIVEN dos casos con las mismas filas esperadas en `en`, uno con `query_lang` `es` y otro con `en`
- THEN hay dos bloques, `es>en` y `en>en`, y cada uno agrega sólo su caso

#### Escenario: sin pareja pero con agregado
- GIVEN un caso sin `query_lang` y otro con filas esperadas sin `lang`
- THEN ninguno aparece en `by_pair`
- AND los dos cuentan en `aggregate`

#### Escenario: ningún caso tiene pareja
- GIVEN un set cuyos casos no tienen `query_lang`
- THEN `by_pair` está vacío en memoria y no existe en el JSON, en `retrieval` ni en `similarity`

#### Escenario: coseno puro por pareja
- GIVEN dos casos de `similarity` en parejas distintas
- THEN cada bloque de `similarity.by_pair` mide sólo sus casos, con `hits`

#### Escenario (guarda): un golden sin `query_lang` mide igual
- GIVEN el golden de fixture de `a_golden_without_the_new_classes_measures_exactly_as_before` (`main.rs:1361`), sin `query_lang`
- WHEN se corre `--fake`
- THEN los `per_case` y el `aggregate` son los mismos que antes, no aparece `by_pair` ni `query_lang` en el JSON y el resto del JSON es igual byte a byte
- AND su `content_sha256` sale de los bytes crudos del archivo (`golden.rs:278`), así que sólo cambia si cambian los bytes

**Tests:** `two_cases_differing_only_in_query_lang_fall_in_different_pairs`, `a_case_missing_either_language_has_no_pair_but_counts_in_the_aggregate`, `by_pair_is_absent_from_the_json_when_no_case_has_a_pair` y `the_console_lists_one_row_per_pair` (`report.rs`); `similarity_by_pair_scores_each_pair_on_its_own_cases` (`main.rs`, sobre `build_similarity_results`). Todos **RED, no compilan** (no existe `by_pair`). La guarda es `a_golden_without_query_lang_measures_exactly_as_before` (`main.rs`): pasa hoy porque los campos no existen, y falla si el cambio deja escapar una clave nueva o mueve una métrica; es el análogo de R10 de eval-rigor y vive junto al RED `by_pair_is_absent_from_the_json_when_no_case_has_a_pair`.

## R4. `agent_view` por pareja

`CaseView` (`agent_view/mod.rs:70-89`) MUST llevar `query_lang` y `row_lang` (`Option<String>`, `#[serde(default, skip_serializing_if = "Option::is_none")]`), copiados del caso y de `case_lang` al armarlo (`agent_view/runner.rs:164`). `AgentViewResults` (`agent_view/mod.rs:142-164`) MUST incluir `by_pair`, un mapa ordenado por la clave `"{query_lang}>{row_lang}"` (por ejemplo `es>en`), con el mismo valor que `by_subset` (`SubsetSummary`) y calculado sobre los mismos casos que `by_subset`: activos y que no son `unanswerable` (`build_results`, `agent_view/mod.rs:331`, `:348-349`). Los `known_red` y los `unanswerable` MUST NOT entrar. Un caso sin alguno de los dos idiomas no entra en ninguna pareja y sigue en `search.active`, `context.active` y `by_subset`. Sin parejas, `by_pair` MUST NOT aparecer. Un resultado viejo, sin `query_lang` ni `row_lang`, MUST seguir leyéndose.

#### Escenario: casos activos por pareja
- GIVEN tres casos activos, dos `es>en` y uno `en>en`, un `known_red` `es>en` y un `unanswerable` `es>en`
- THEN `by_pair` tiene `es>en` con `n_cases` 2 y `en>en` con 1

#### Escenario: sin idiomas
- GIVEN casos sin `query_lang` ni `row_lang`
- THEN `by_pair` no aparece y `by_subset` es el de siempre

**Tests** (`agent_view/mod.rs`): `build_results_splits_active_cases_by_pair`, `agent_view_pairs_leave_out_known_red_and_unanswerable` y `a_case_view_without_languages_reads_back_and_writes_none`. Todos **RED, no compilan** (no existen los campos).

## R5. Corte por pareja en `--diff`

`--diff` MUST sumar, para `retrieval`, `similarity` y `agent_view`, un bloque por pareja con `better`, `worse`, `net` y el p exacto (`MetricDiff`, `diff.rs:26-42`) calculados sólo sobre los casos con la misma pareja en los dos archivos. La pareja de cada archivo sale del `query_lang` y del `lang` (o `row_lang`) de su propio `per_case` de ese bloque. En `retrieval` y `similarity` las métricas son las cinco de `diff_block` (R@1, R@5, R@10, MRR, nDCG@10; `diff.rs:282-301`); en `agent_view`, `search.found` y `context.found`, sobre casos activos como `diff_agent_view` (`diff.rs:250-275`). Los bloques van ordenados por pareja.

Dos clases de caso MUST NOT entrar en ningún bloque de pareja y MUST informarse aparte, cada una con su propio conteo por bloque (`retrieval`, `similarity`, `agent_view`), impreso sólo si es mayor que 0:
- **pareja que difiere** (pareja en los dos archivos, distinta);
- **pareja en un solo archivo** (pareja en uno y ninguna en el otro).

Un caso sin pareja en ninguno de los dos archivos no se cuenta en ninguna parte. Los bloques de pareja MUST NOT aparecer si ningún caso unido tiene la misma pareja en los dos archivos, y los conteos se imprimen igual si son mayores que 0. Las filas globales de cada bloque MUST NOT cambiar: el corte no quita ni suma casos a `diff_block` ni a `diff_agent_view`.

La salida imprime, tras cada bloque global que tenga parejas, `{bloque} by pair {query_lang}>{lang} ({n} joined):` seguido de las mismas filas que el bloque global (`print_metric`, `diff.rs:367`). Además imprime, cuando son mayores que 0, `  pair differs between the files: {k} case(s), counted in no pair` y `  pair in one file only: {k} case(s), counted in no pair`.

**Decisiones.**
- En `agent_view` cada pareja lleva sólo `search.found` y `context.found`, sin tokens: lo que el criterio pregunta de los tokens (U9) es el cambio global con su intervalo, y repartir 46 casos en tres parejas deja un intervalo bootstrap que no dice nada.
- Un caso sin pareja en ninguno de los dos archivos no se cuenta en ninguna parte, por la misma regla que `by_lang` (`report.rs:528-534`): un cubo o un conteo «sin pareja» mezclaría lo que no se sabe atribuir con lo que se compara. El caso sigue en las filas globales.

#### Escenario: seis casos mejoran en una pareja
- GIVEN dos archivos con 19 casos `es>en` donde R@5 mejora en 6 y no empeora en ninguno
- THEN el bloque `es>en` de `retrieval` informa `recall@5` 6 mejores, 0 peores y p = 0,03125

#### Escenario: sólo cuentan los casos de la pareja
- GIVEN 19 casos `es>en` y 20 `es>es`, y R@5 que mejora sólo en `es>es`
- THEN el bloque `es>en` da 0/0/0 con p = 1 y el de `es>es` da las mejoras

#### Escenario: la pareja cambió entre archivos
- GIVEN un caso `es>en` en el primer archivo y `en>en` en el segundo
- THEN no entra ni en `es>en` ni en `en>en`
- AND el conteo «pair differs» da 1 y el de «pair in one file only» da 0

#### Escenario: pareja en un solo archivo
- GIVEN un caso `es>en` en el segundo archivo y sin `query_lang` en el primero
- THEN no entra en ningún bloque de pareja
- AND el conteo «pair in one file only» da 1 y el de «pair differs» da 0

#### Escenario: sin pareja en ninguno
- GIVEN un caso sin pareja en los dos archivos
- THEN no entra en ningún bloque ni en ningún conteo

#### Escenario: `similarity` y `agent_view`
- GIVEN dos archivos con parejas en `similarity.per_case` y en `agent_view.per_case`
- THEN cada uno tiene su corte por pareja, y `agent_view` lo calcula sólo sobre casos activos

**Tests** (`diff.rs`): `a_pair_block_joins_only_the_cases_with_that_pair`, `six_improvements_in_one_pair_are_significant_there_only`, `a_case_whose_pair_differs_between_the_files_is_reported_apart`, `a_case_with_a_pair_in_one_file_only_is_reported_apart`, `a_case_without_a_pair_in_either_file_is_counted_nowhere`, `similarity_has_its_own_pair_blocks`, `agent_view_pairs_use_only_active_cases` y `the_global_rows_do_not_change_when_pairs_are_present`. Todos **RED, no compilan** (`RunDiff` y `AgentViewDiff` no llevan bloques de pareja).

## R6. Archivos de resultados viejos

Un archivo sin `query_lang` en sus casos y sin `by_pair` MUST seguir leyéndose y comparándose con `--diff` contra uno que los tiene, en cualquier orden. Los casos sin pareja en el archivo viejo y con pareja en el nuevo son casos con pareja en un solo archivo (R5): se cuentan aparte, no entran en ningún bloque de pareja y no hacen fallar el diff. Dos archivos viejos entre sí MUST dar lo mismo que hoy y ningún bloque de pareja. El diff MUST NOT leer `by_pair` de los archivos: lo reconstruye de `per_case`.

#### Escenario: viejo contra nuevo
- GIVEN un archivo sin parejas y otro, del mismo set, con `query_lang` en sus 46 casos
- THEN el diff no falla, las filas globales dan 0/0/0 y p = 1
- AND el conteo «pair in one file only» da 46 en `retrieval`, el de «pair differs» da 0 y no hay bloques de pareja (ningún caso tiene la misma pareja en los dos archivos)

#### Escenario: viejo contra viejo
- GIVEN dos archivos sin parejas
- THEN la salida no tiene bloques de pareja ni conteo

**Tests** (`diff.rs`): `a_file_without_pairs_diffs_against_one_with_them` y `two_files_without_pairs_have_no_pair_block`. **RED, no compilan** (el conteo y el campo de pareja no existen). `a_results_file_without_by_pair_still_reads_and_diffs` (`report.rs`, como `a_results_file_without_by_subset_still_reads_and_diffs`, `report.rs:1129`) es **RED, no compila** por la misma razón, y fija además que el archivo nuevo escribe `by_pair` como arreglo.

## R7. El veredicto y la salida de error no cambian

Las líneas `primary:` (`diff.rs:350-354`) MUST ser las mismas con o sin bloques de pareja, y `regressed` (`diff.rs:46-49`) MUST seguir mirando sólo `retrieval.recall_5` y `agent_view.search_found` globales. Una regresión dentro de una pareja, por grande que sea, MUST NOT hacer salir con 6 (`EXIT_REGRESSION`, `main.rs:58`) y MUST NOT agregar líneas a `primary:`. Las filas por pareja son informativas, igual que las demás métricas no primarias.

#### Escenario: regresión sólo en una pareja
- GIVEN 20 casos `es>en` que pierden R@5 y 20 `es>es` que lo ganan
- THEN el bloque `es>en` informa 0 mejores, 20 peores y p < 0,05
- AND `regressed` es falso y `--fail-on-regression` sale con 0
- AND `primary:` queda como sin parejas

**Tests:** `a_regression_inside_a_pair_never_fails` y `the_primary_lines_ignore_the_pair_blocks` (`diff.rs`); `diff_exits_zero_when_only_a_pair_regresses` (`main.rs`, junto a `diff_exits_six_on_a_significant_regression_only_with_the_flag`). **RED, no compilan** los de `diff.rs` (construyen un `RunDiff` con bloques de pareja); el de `main.rs` es **RED por comportamiento**: escribe dos archivos con `query_lang`, y hoy el diff no distingue parejas, así que no puede afirmar que el bloque `es>en` informa 20 peores.

## R8. Texto de ayuda y documentación

El texto de ayuda de `--diff` (`HELP_TEXT`, `cli.rs:169-177`) MUST decir que, cuando los archivos traen `query_lang`, también corta por pareja consulta–fila y que ese corte no entra en `primary:` ni en `--fail-on-regression`. Esto sí es testable, con el mismo patrón que `HELP_TEXT.contains("--agent-view")` (`cli.rs:866`).

La línea de `.claude/CLAUDE.md` que explica cómo comparar dos corridas («A ranking change is compared with the baseline by …», línea 109 de ese archivo) MUST mencionar el corte por pareja y que un cambio de ranking que toque el cruce de idiomas se lee ahí. No es un requisito testable: ningún test lee ese archivo, y `crates/clawg-cli/tests/conventions.rs` no lo cubre. Es **una tarea de documentación para sdd-tasks**, verificada por lectura en `verify`. La línea no tiene copia en `.cursor/rules/cursor.mdc` ni en `.gemini/GEMINI.md` (se buscó la frase exacta).

**Test** (`cli.rs`): `the_diff_help_describes_the_pair_cut` (**RED, comportamiento**: `HELP_TEXT` no menciona parejas hoy).

## Tabla de tests

| Test | Archivo | Estado | Por qué |
|---|---|---|---|
| `a_case_may_name_the_language_of_its_query` | `golden.rs` | **RED** (comportamiento) | `deny_unknown_fields` (`golden.rs:59`) rechaza `query_lang` |
| `a_case_without_query_lang_has_none` | `golden.rs` | guarda | hoy parsea sin la clave |
| `a_misspelled_query_lang_key_fails_to_parse` | `golden.rs` | guarda | hoy toda clave desconocida falla |
| `pair_of_needs_both_languages` | `report.rs` | **RED** (no compila) | no existe `pair_of` |
| `a_case_result_without_query_lang_neither_writes_nor_requires_it` | `report.rs` | **RED** (no compila) | `CaseResult` no tiene el campo |
| `the_query_language_reaches_the_retrieval_and_similarity_cases` | `main.rs` | **RED** (no compila) | `EmbeddedCase`/`SimCaseSource`/`CaseResult` no lo llevan |
| `two_cases_differing_only_in_query_lang_fall_in_different_pairs` | `report.rs` | **RED** (no compila) | no existe `by_pair` |
| `a_case_missing_either_language_has_no_pair_but_counts_in_the_aggregate` | `report.rs` | **RED** (no compila) | ídem |
| `by_pair_is_absent_from_the_json_when_no_case_has_a_pair` | `report.rs` | **RED** (no compila) | ídem |
| `the_console_lists_one_row_per_pair` | `report.rs` | **RED** (no compila) | no existe la tabla |
| `similarity_by_pair_scores_each_pair_on_its_own_cases` | `main.rs` | **RED** (no compila) | `SimilarityResults` no tiene `by_pair` |
| `a_golden_without_query_lang_measures_exactly_as_before` | `main.rs` | guarda | hoy no hay claves nuevas ni métricas distintas |
| `build_results_splits_active_cases_by_pair` | `agent_view/mod.rs` | **RED** (no compila) | `CaseView` y `AgentViewResults` no llevan idiomas |
| `agent_view_pairs_leave_out_known_red_and_unanswerable` | `agent_view/mod.rs` | **RED** (no compila) | ídem |
| `a_case_view_without_languages_reads_back_and_writes_none` | `agent_view/mod.rs` | **RED** (no compila) | ídem |
| `a_pair_block_joins_only_the_cases_with_that_pair` | `diff.rs` | **RED** (no compila) | `RunDiff` no tiene bloques de pareja |
| `six_improvements_in_one_pair_are_significant_there_only` | `diff.rs` | **RED** (no compila) | ídem |
| `a_case_whose_pair_differs_between_the_files_is_reported_apart` | `diff.rs` | **RED** (no compila) | ídem |
| `a_case_with_a_pair_in_one_file_only_is_reported_apart` | `diff.rs` | **RED** (no compila) | ídem |
| `a_case_without_a_pair_in_either_file_is_counted_nowhere` | `diff.rs` | **RED** (no compila) | ídem |
| `similarity_has_its_own_pair_blocks` | `diff.rs` | **RED** (no compila) | ídem |
| `agent_view_pairs_use_only_active_cases` | `diff.rs` | **RED** (no compila) | `AgentViewDiff` no tiene bloques de pareja |
| `the_global_rows_do_not_change_when_pairs_are_present` | `diff.rs` | **RED** (no compila) | ídem |
| `a_file_without_pairs_diffs_against_one_with_them` | `diff.rs` | **RED** (no compila) | no existe el conteo |
| `two_files_without_pairs_have_no_pair_block` | `diff.rs` | **RED** (no compila) | no existe el campo que se afirma vacío |
| `a_results_file_without_by_pair_still_reads_and_diffs` | `report.rs` | **RED** (no compila) | no existe `by_pair` |
| `a_regression_inside_a_pair_never_fails` | `diff.rs` | **RED** (no compila) | construye un `RunDiff` con bloques de pareja |
| `the_primary_lines_ignore_the_pair_blocks` | `diff.rs` | **RED** (no compila) | ídem |
| `diff_exits_zero_when_only_a_pair_regresses` | `main.rs` | **RED** (comportamiento) | hoy no se informa el bloque de pareja que el test afirma |
| `the_diff_help_describes_the_pair_cut` | `cli.rs` | **RED** (comportamiento) | `HELP_TEXT` (`cli.rs:169-177`) no menciona parejas |

Cada requisito tiene al menos un test RED. Las guardas (`a_case_without_query_lang_has_none`, `a_misspelled_query_lang_key_fails_to_parse` y `a_golden_without_query_lang_measures_exactly_as_before`) pasan hoy y MUST seguir en verde en cada commit; cada una convive con el RED de su requisito (R1 y R3).

## Invariantes

- La pareja, `by_pair` y el corte de `--diff` son lógica pura sobre `CaseResult`/`CaseView`, separada de la E/S: se prueban sin modelo, sin red y sin `docs/eval/` (R12 de eval-rigor sigue valiendo para los tests nuevos de `diff.rs` y `report.rs`).
- `--diff` sigue siendo determinista: los bloques van ordenados por pareja y el bootstrap usa la semilla fija.
- Un resultado viejo se sigue leyendo y comparando; un `rag-eval` anterior no lee un golden con `query_lang` (`deny_unknown_fields`), y eso es aceptable dentro del repo.
- No se agrega dependencia ni visibilidad: `Pair` y `pair_of` viven en `report.rs`, que `diff.rs` y `agent_view` ya importan (`diff.rs:17`, `agent_view/mod.rs:20`).
