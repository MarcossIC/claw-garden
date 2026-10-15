# eval — Especificación

Fuente de verdad del comportamiento actual de la evaluación de recuperación: el generador sintético de `golden-mine synth`, la etiqueta de subconjunto del golden set, el reporte por subconjunto de `rag-eval`, los intrusos del audit y la baseline medida con el set generado.

**Origen:** [eval-golden-expansion](../../changes/archive/2026-10-12-eval-golden-expansion/proposal.md), archivado el 2026-10-12.

## Definiciones

- **Golden set:** el JSON que lee `rag-eval` (`docs/eval/golden.json`, fuera de git): `corpus`, `retrieval_cases` y pares. El parseo usa `deny_unknown_fields` en todos sus tipos (`crates/clawg-eval/src/bin/rag-eval/golden.rs:31`).
- **Subconjunto:** `CaseSubset` (`golden.rs:85`): `natural`, `lexical`, `multi_ref`, `high_importance`, `low_importance` o `long`. Los casos del golden set original son `natural`.
- **Set generado:** la salida de `golden-mine synth`: todas las filas, casos y pares de la base sin cambios, seguidos de lo que agrega el generador (`synth::generate`, `crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:59`).
- **Distractor:** fila sintética que ningún caso espera.
- **Caso activo:** caso que entra a las métricas; los `known_red` no entran.
- **Intruso:** la fila no esperada que aparece en el top-5 (`INTRUDER_DEPTH`, `crates/clawg-eval/src/bin/rag-eval/audit.rs:52`) de más casos activos de un subconjunto.

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

El tamaño es `--distractors` (`DEFAULT_DISTRACTORS` = 500, `synth/mod.rs:17`); es lo único que escala (`Sizes`, `synth/mod.rs:46`). La salida SHALL incluir:

- **Distractores** (500 por defecto). Cada uno mezcla términos de dos filas base del mismo `(memory_type, lang)`. Su largo MUST llegar a la mediana de largo de las filas base de su grupo, salvo que ya encadene 3 marcos (`MAX_CHAINED_FRAMES`). Ningún distractor MUST compartir una secuencia de 6 palabras seguidas con una fila base (`COPY_RUN_WORDS`, hasta `MAX_COPY_RETRIES` = 50 reintentos).
- **30 casos `lexical`** (`LEXICAL_FAMILIES`), en familias de 8 filas:
  - la esperada menciona el identificador al pasar, dentro de un marco distinto del que parafrasea la consulta;
  - los 7 hermanos usan ese marco, con identificadores del mismo prefijo cuyos dígitos son una permutación de los de la esperada o difieren en ±1 en el último dígito (`near_identifiers`, `synth/mod.rs:235`);
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

`RetrievalCase` MUST aceptar una clave opcional `subset` (`golden.rs:75`) con los seis valores de `CaseSubset`, en snake_case. Si falta, el caso MUST valer `natural`. Una clave desconocida, también una mal escrita como `subsett`, y un valor desconocido MUST seguir siendo un error de parseo.

#### Escenario: golden actual sin etiqueta
- GIVEN el `golden.json` actual, sin `subset`
- THEN todos los casos son `natural`

#### Escenario: etiqueta explícita
- GIVEN un caso con `"subset": "lexical"` (o `"low_importance"`)
- THEN el caso es de ese subconjunto

#### Escenario: clave mal escrita
- GIVEN un caso con la clave `subsett` o el valor `lexcial`
- THEN el parseo falla

**Tests** (`golden.rs`, mod tests): `a_case_without_a_subset_is_natural`, `an_explicit_subset_is_parsed`, `every_subset_name_parses_in_snake_case`, `a_misspelled_subset_key_fails_to_parse` y `an_unknown_subset_value_fails_to_parse`.

## Reporte de `rag-eval`

### Métricas por subconjunto

El reporte de recuperación MUST incluir `by_subset` (`report.rs:119`): un bloque por subconjunto presente, con `n_cases` y el mismo `Aggregate` que el global, junto a `by_lang`. Se calcula sólo sobre los casos activos (`aggregate_by_subset`, `crates/clawg-eval/src/bin/rag-eval/report.rs:385`, que `main.rs` llama con `per_case`), así que los `known_red` MUST NOT entrar. Los bloques van ordenados por el nombre serializado, para que dos corridas del mismo set den JSON comparable byte a byte.

Un resultado previo sin `by_subset`, o con casos sin `subset`, MUST seguir leyéndose (los casos sin etiqueta vuelven como `natural`) y comparándose con `--diff`.

#### Escenario: dos subconjuntos
- GIVEN casos `natural` y `lexical`
- THEN hay un bloque por cada uno, con R@5 y MRR calculados sólo sobre sus casos

#### Escenario: sólo natural
- GIVEN un golden sin casos etiquetados
- THEN hay un único bloque `natural` con las mismas cifras que el agregado global

**Tests** (`report.rs`, mod tests): `aggregate_by_subset_scores_each_subset_on_its_own_cases_only`, `with_only_natural_cases_the_single_bucket_equals_the_global_aggregate`, `a_results_file_without_by_subset_still_reads_and_diffs` y `a_case_result_without_a_subset_reads_back_as_natural`.

### Test de resultados autocontenido

El test que compara un resultado sin el bloque `agent_view` contra uno con el bloque MUST pasar sin que exista nada bajo `docs/eval/`: usa un fixture inline.

**Test:** `a_results_file_without_the_block_still_diffs_against_one_with_it` (`report.rs:1002`).

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

## Subconjunto `long`

Mide si la respuesta que cae dentro de un save largo llega al agente. Viene de [long-save-context](../../changes/archive/2026-10-14-long-save-context/proposal.md), R9 de esa spec.

### Generador del subconjunto `long`

`golden-mine synth` MUST agregar, después de todo lo que agrega hoy y sin cambiar nada de eso (`long_saves`, `crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:417`, llamado al final de `generate`), 12 casos `long` (6 en inglés y 6 en español):

- cada caso tiene una fila propia de al menos 3 pedazos con el chunker de producción, también en modo sin tokenizer;
- la fila nombra la clave del caso sólo en su primera oración, y el dato que responde la consulta (`answer`) aparece una sola vez, después del primer tercio del texto;
- el relleno (`templates::long_filler`, `synth/templates.rs:284`) no nombra la clave y no repite una secuencia de 6 palabras de una fila de la base. Entre dos filas `long` lo que se exige es que no compartan ningún pedazo: el dedup los absorbería y el caso mediría otra cosa. Las filas `long` salen de las mismas plantillas, así que entre ellas sí pueden repetirse frases cortas;
- el caso lleva `answer`: el dato tal como aparece en la fila.

Con la misma base y la misma semilla, las filas y casos que el generador agrega hoy MUST salir idénticos. Como el archivo crece, el set de la semilla 7 pasa de 1.028 filas y 112 casos a 1.040 filas y 124 casos, y su sha256 cambia.

`RetrievalCase` MUST aceptar `answer` opcional (`golden.rs:80`), y `CaseSubset` el valor `long` (`golden.rs:92`). Un golden sin esas claves MUST parsear igual que antes.

#### Escenario: el dato después del corte
- GIVEN un caso `long` con clave `amber-falcon` y `answer` `"retry budget of 7 attempts"`
- THEN su fila nombra `amber-falcon` sólo en la primera oración, `retry budget of 7 attempts` aparece una vez después del primer tercio, y la fila se parte en 3 pedazos o más

**Tests:** `there_are_twelve_long_cases_at_any_size`, `a_long_row_names_its_key_only_in_its_first_sentence`, `the_answer_appears_once_after_the_first_third`, `a_long_row_splits_into_at_least_three_pieces` y `adding_long_cases_leaves_the_rest_of_the_set_unchanged` (`synth/mod.rs`); `a_long_case_writes_its_subset_and_answer` (`synth/golden.rs`); `an_answer_and_the_long_subset_parse` (`rag-eval/golden.rs`).

### Métricas de agent-view para `answer`

`rag-eval --agent-view` MUST reportar, para los casos con `answer` (`crates/clawg-eval/src/bin/rag-eval/agent_view/mod.rs`, `AnswerStats`):

- **respuesta visible:** el texto que devolvió la herramienta contiene `answer`, comparado con los espacios normalizados (`normalize`, `agent_view/matcher.rs:22`);
- **respuesta alcanzable** (sólo `search_memory`): visible, o visible en el save que devuelve `GET /v1/memories/memory/{id}/save` para la primera línea que carga la fila esperada. Contra un daemon sin esa ruta, alcanzable es igual a visible;
- **pedazos repetidos:** cuántas respuestas de `search_memory` muestran dos líneas del mismo save (dos líneas con el mismo id, o con filas del mismo caso);
- **por subconjunto:** las métricas de agent-view separadas por `subset` (`by_subset`, `SubsetSummary`), así `long` y los otros cinco se comparan por separado.

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

## Baseline medida

Estas cifras no son tests de `cargo`: se miden a mano con `rag-eval --real`, que necesita el modelo. La medición completa está en el [`verify.md` archivado](../../changes/archive/2026-10-12-eval-golden-expansion/verify.md#medición-rag-eval---real). Las cifras de la semilla 7 de abajo son de antes de [long-save-context](../../changes/archive/2026-10-14-long-save-context/proposal.md); el set actual tiene 1.040 filas y 124 casos.

### Ancla de regresión

Medir con `rag-eval --real` el `golden.json` actual, sin filas sintéticas, MUST reproducir R@5 0,7826 y MRR 0,5979 (n = 46, todo `natural`).

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

- El generador y el reporte son lógica pura, separada de la E/S: se prueban sin modelo, sin red y sin `docs/eval/`.
- Producción nunca enlaza esto: `clawg-eval` usa `clawg-core` a través de la feature `eval`.
