# eval-golden-expansion — Verificación

**Código:** `main` local, de `e2dda9b` a `c065da5` (17 commits del cambio, más `d3ad95e` chore de `.gitignore` y `c2b096f` docs de la base de tests, ajenos).
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`).
**Estado:** verificado con advertencias. El código cumple R1 a R7 con tests en verde. R8 se midió: C1, C2 y C3 se cumplen; C4 no se cumple con la redacción corregida (valor absoluto, ver WARNING 1) y queda aceptado; C5 no se cumple y queda aceptado por decisión del usuario, sin mover umbrales ni iterar el generador.

## Suite

`cargo test --workspace --no-fail-fast --target-dir target-apply` en Windows, sobre `c065da5`:

| | Pasan | Fallan | Suites |
|---|---|---|---|
| `c065da5` | 1566 | 0 | 11 |

- Contra la base de 1304 de `config.yaml`: +262, de este cambio y de los mergeados antes. Contra los 1540 de la tarea 10.1: +26, los de la revisión (secciones 11 a 18). Coincide con lo reportado en 18.2.
- `clawg-eval` sola: 400 (`golden-mine` 97, `rag-eval` 298, `synth_roundtrip` 5).
- **Una corrida anterior falló 1 test**, fuera de este cambio: `memory::search::tests::disputed_status_does_not_change_score` (`crates/clawg-core/src/memory/search.rs:1571`), con 0,8958237 contra 0,8958239. Pasó en la corrida completa siguiente y en 6 corridas aisladas. Las dos filas fijan `last_accessed_at` con `strftime('%s','now')`, de resolución de un segundo: si los dos inserts caen en segundos distintos, la recencia difiere y el `assert_eq!` sobre `f64` falla. Viene de `46c518c`, anterior a este cambio. Ver WARNING 4.
- **Warnings de compilación:** los mismos de antes. En `clawg-core`, 4 en la librería (`single_shot`, `finish_reason`, `role_to_string`, `Text`) y uno más en sus tests (`insert_row_with_embedding`); en `rag-eval`, 2 (`production_lexical_budget` sin uso en `main.rs:16`, `ceiling` en `follow_up.rs:244`). Los dos de `rag-eval` ya estaban en `e2dda9b~1`.
- **Clippy** (`cargo clippy -p clawg-eval --all-targets`): ningún aviso en los archivos de `golden-mine` ni en lo que agregó el cambio. Los dos de `rag-eval/main.rs` (`:16`, `:271`) son anteriores.
- **Cobertura:** no hay herramienta instalada (`cargo llvm-cov` y `cargo tarpaulin` ausentes). No se mide.

### Atomicidad

Cada commit se extrajo con `git archive` a un árbol limpio (con `docs/eval/` copiado, que está fuera de git) y se corrió `cargo test -p clawg-eval --no-fail-fast`:

| Commit | `clawg-eval` pasan | Fallan |
|---|---|---|
| `e2dda9b` test(eval): make the results-diff test self-contained | 314 | 0 |
| `aac4f37` feat(eval): add an optional subset label to golden cases | 319 | 0 |
| `cb6b51e` feat(eval): carry the subset into each case result | 321 | 0 |
| `f9a2222` feat(eval): report retrieval metrics per case subset | 327 | 0 |
| `6bc700d` feat(golden-mine): add a synth subcommand that preserves the base set | 338 | 0 |
| `ddc6ec8` feat(golden-mine): generate deterministic distractors near real rows | 358 | 0 |
| `32af2da` feat(golden-mine): generate identifier queries with near-identical siblings | 364 | 0 |
| `1b3ebcc` feat(golden-mine): generate multi-reference and high-importance cases | 370 | 0 |
| `834679b` test(eval): run the generated set through rag-eval end to end | 374 | 0 |
| `d3ad95e` chore: ignore the alternate test target directory | 374 | 0 |
| `c2b096f` docs: update the test baseline | 374 | 0 |
| `a423565` feat(eval): add a low-importance subset label | 375 | 0 |
| `67b2c47` feat(golden-mine): derive near-identical identifiers from a seed identifier | 379 | 0 |
| `d01c5d1` feat(golden-mine): mention the queried identifier in passing beside near-identical siblings | 381 | 0 |
| `656bd5c` feat(golden-mine): match distractor length to the rows they imitate | 386 | 0 |
| `2100496` feat(golden-mine): ask about a few aspects of an entity that has many | 389 | 0 |
| `184d084` feat(golden-mine): contest each importance case with rows about the same entity | 391 | 0 |
| `90d07ba` feat(eval): report the most frequent intruder per subset in the audit | 399 | 0 |
| `c065da5` test(eval): cover every generated subset end to end | 400 | 0 |

- Todos compilan y pasan. El conteo nunca baja y los warnings son siempre los mismos (desde `834679b` el test de integración compila además el bin `rag-eval` sin `cfg(test)`, que repite el mismo aviso de `production_lexical_budget`).
- La primera pasada dio conteos planos y no valía: `git archive` conserva el mtime del commit y cargo no recompilaba. Se repitió con `tar -m`; la tabla es de esa segunda pasada.

## Medición `rag-eval --real`

Binario debug de `c065da5`. `--real` sólo leyó el modelo en caché de `~/.clawg-dev/models` (multilingual-e5-small); cada corrida sembró un almacén temporal propio en `%TEMP%\rag-eval-<uuid>`. Un listado de `~/.clawg-dev` antes y después no cambió. No se abrió `memory.db` de ningún perfil ni se tocó `~/.clawg`.

### Generación (re-hecha en esta verificación)

`golden-mine synth --base docs/eval/golden.json --out <scratch>/seed7.json --seed 7`, dos veces, y una con `--seed 8`:

| | sha256 |
|---|---|
| base `golden.json` | `d2d9d7320bce841126eac163790f1052d268521912cfcc6aac4f9af466ed792a` |
| semilla 7, primera | `6312586c2206cc3156ba43f40f2025d176166f29f631118759abc9c3ffdd228c` |
| semilla 7, segunda | `6312586c2206cc3156ba43f40f2025d176166f29f631118759abc9c3ffdd228c` |
| semilla 8 | `4a01beb9e821c091011a064417e5e7b9d42901f2f4fa429f2b478617f54d1e18` |
| `docs/eval/golden-expanded.json` | `6312586c…d228c` (igual a la semilla 7) |

- 1028 filas (48 de la base), 112 casos (46 de la base). El sha256 de `golden.json` es el mismo antes y después de generar.
- Composición: 500 distractores; 30 `lexical`, 12 `multi_ref` (2 o 3 esperadas), 12 `high_importance` (esperada en {8, 9, 10}, competidoras en [3, 5]), 12 `low_importance` (esperada en {3, 4, 5}, competidoras en [8, 10]); 72 filas sintéticas con importancia 8 a 10, 56 con 9 o más.

### Medición previa, reemplazada (tareas 10.2 a 10.4)

Semilla 7, salida `c9bc6317433dd050c9249d318f6072d13485ecb8363c2f5a017ecbd19b5f7396`, 842 filas y 112 casos. No es la baseline: tres subconjuntos saturaban y el criterio de 10.4 no se cumplió (`design.md`, revisión 2026-10-12). Se deja como registro.

| Subconjunto | n | R@1 | R@5 | R@10 | MRR |
|---|---|---|---|---|---|
| global | 112 | 0,5714 | 0,7143 | 0,7679 | 0,6739 |
| natural | 46 | 0,1304 | 0,3043 | 0,4348 | 0,2169 |
| lexical | 30 | 0,9667 | 1,0000 | 1,0000 | 0,9833 |
| multi_ref | 12 | 0,4167 | 1,0000 | — | 1,0000 |
| high_importance | 24 | 1,0000 | 1,0000 | 1,0000 | 1,0000 |

- Ancla (10.2): R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG@10 0,6693.
- Criterio de 10.4 (léxico 10 puntos por debajo del natural): no cumplido, léxico 69,6 puntos por encima. `mean_corpus_distractor_gap` 0,0503; `mean_expected_distractor_gap` 0,0042.

### Baseline vigente (tareas 18.3 a 18.5), confirmada en esta verificación

Todas las cifras de esta sección se re-midieron hoy y coinciden con las de 18.3 y 18.4 hasta el cuarto decimal.

**Ancla** (`rag-eval --real --golden docs/eval/golden.json`), n=46, todo `natural`:

| R@1 | R@5 | R@10 | MRR | nDCG@10 |
|---|---|---|---|---|
| 0,4348 | 0,7826 | 0,8913 | 0,5979 | 0,6693 |

**Set de la semilla 7** (`rag-eval --real --audit --golden seed7.json --sweep-lexical 20 --sweep-lexical 0`):

| Subconjunto | n | R@1 | R@5 | R@10 | MRR | nDCG@10 |
|---|---|---|---|---|---|---|
| global | 112 | 0,2247 | 0,5387 | 0,7679 | 0,3925 | 0,4760 |
| natural | 46 | 0,1304 | 0,3696 | 0,4348 | 0,2194 | 0,2713 |
| lexical | 30 | 0,2333 | 0,4333 | 1,0000 | 0,3825 | 0,5214 |
| multi_ref | 12 | 0,2639 | 0,7778 | 1,0000 | 0,7465 | 0,7686 |
| high_importance | 12 | 0,5833 | 1,0000 | 1,0000 | 0,7292 | 0,7968 |
| low_importance | 12 | 0,1667 | 0,7500 | 1,0000 | 0,3898 | 0,5342 |

- Por idioma: en (56) R@5 0,4911; es (56) R@5 0,5863.
- Audit: USABLE. Saturación R@10 0,7679 (límite 0,90), acuerdo modal 0,0223, solapamiento léxico 0,1143, dominancia de importancia +0,0256. `similarity.mean_corpus_distractor_gap` 0,0572; `mean_expected_distractor_gap` −0,0082.
- `top_intruders`:

| Subconjunto | Fila | Casos | Fracción |
|---|---|---|---|
| high_importance | `s-hi-04-5` | 2 | 16,7 % |
| lexical | `s-lex-11-4` | 2 | 6,7 % |
| low_importance | `s-lo-05-2` | 2 | 16,7 % |
| multi_ref | `s-mr-03-1` | 2 | 16,7 % |
| natural | `s-lex-19-0` | 6 | 13,0 % |

- Canal léxico encendido (`20`) frente a apagado (`0`): **0 de 112** casos con `ranked_refs` distinto, comparando `per_case` de los dos JSON.

**Importancia aplanada** (`--sweep-imp 1.0:1.0`):

| Subconjunto | R@5 | MRR |
|---|---|---|
| natural | 0,3478 | 0,1910 |
| high_importance | 1,0000 | 0,5236 |
| low_importance | 0,9167 | 0,5965 |

MRR(high) − MRR(low): 0,3394 con la importancia por defecto; −0,0729 aplanada.

### Criterio de salida (R8)

| # | Condición | Medido | Resultado |
|---|---|---|---|
| C1 | Ancla R@5 0,7826, MRR 0,5979 | 0,7826 / 0,5979 | Cumplido |
| C2 | `lexical` R@5 <= 0,70 | 0,4333 | Cumplido. `B` de lexical-fusion = 0,4333 |
| C3 | `multi_ref` R@5 y nDCG@10 <= 0,90 | 0,7778 / 0,7686 | Cumplido |
| C4 | MRR high y low <= 0,90; high − low >= 0,15 por defecto y \|high − low\| <= 0,05 aplanada | 0,7292 y 0,3898; 0,3394; −0,0729 | **No cumplido** con la redacción corregida (0,0729 > 0,05). Aceptado, ver WARNING 1 |
| C5 | En `natural`, ningún intruso sintético > 10 % | `s-lex-19-0` 13,0 % (6/46) | **No cumplido.** Aceptado, ver WARNING 2 |
| C6 | Diagnóstico: rankings distintos con léxico 20 frente a 0 | 0/112 | Anotado. Ver SUGGESTION 1 |

## Completitud

`tasks.md`: 49 ítems, 48 marcados (`18.6` con esta verificación). `10.5` queda sin marcar a propósito: está tachada y reemplazada por `18.6`.

## Matriz de cumplimiento

| Requisito | Escenario | Test | Resultado |
|---|---|---|---|
| R1 Generador determinista | misma semilla | `synth/mod.rs > the_same_seed_gives_the_same_set_and_another_seed_a_different_one`; `synth_roundtrip.rs > the_same_seed_gives_identical_bytes_and_another_seed_does_not` | Cumple |
| R1 | otra semilla | los mismos dos | Cumple |
| R2 Composición | conteos mínimos | `the_default_size_adds_at_least_five_hundred_distractors`, `there_are_at_least_thirty_lexical_cases_at_any_size`, `there_are_at_least_ten_multi_reference_cases_at_any_size`, `there_are_enough_high_importance_rows_at_any_size`, `each_high_importance_case_…`, `each_low_importance_case_…` | Cumple |
| R2 | hermano léxico | `the_queried_identifier_appears_in_the_expected_row_and_nowhere_else`, `siblings_carry_identifiers_near_the_queried_one`, `the_expected_row_uses_a_mention_frame_and_its_siblings_the_queried_frame`, `near_identifiers_share_the_prefix_and_keep_the_digits_close` | Cumple |
| R2 | distractor sin copia | `no_distractor_shares_six_words_in_a_row_with_a_base_row`, `a_distractor_reaches_its_groups_median_length_or_chains_three_frames` | Cumple |
| R2 | entidad concurrida | `a_multi_reference_key_names_eight_rows_and_only_some_of_them_answer`, `the_rows_of_a_multi_reference_key_cover_different_aspects`, `check_contested` (high y low) | Cumple |
| R2 | tamaño pequeño | `the_size_only_changes_how_many_distractors_are_added` y los `…_at_any_size` con 0, 10 y 500 | Cumple |
| R3 Etiqueta `subset` | golden actual sin etiqueta | `rag-eval/golden.rs > a_case_without_a_subset_is_natural`; el ancla real sale con un único bloque `natural` | Cumple |
| R3 | etiqueta explícita | `an_explicit_subset_is_parsed` | Cumple |
| R3 | `low_importance` | `every_subset_name_parses_in_snake_case`; `synth/golden.rs > the_low_importance_subset_round_trips_by_its_snake_case_name` | Cumple |
| R3 | clave mal escrita | `a_misspelled_subset_key_fails_to_parse`, `an_unknown_subset_value_fails_to_parse` | Cumple |
| R4 Métricas por subconjunto | dos subconjuntos | `report.rs > aggregate_by_subset_scores_each_subset_on_its_own_cases_only` | Cumple |
| R4 | sólo natural | `with_only_natural_cases_the_single_bucket_equals_the_global_aggregate` | Cumple |
| R4 | resultado previo sin bloque | `a_results_file_without_by_subset_still_reads_and_diffs`, `a_case_result_without_a_subset_reads_back_as_natural` | Cumple |
| R5 Ancla | sin sintéticos | medición `--real` de arriba (no es un test de `cargo`) | Cumple |
| R5 | generador no toca lo natural | `synth_roundtrip.rs > the_base_file_is_left_untouched`, `writing_over_the_base_is_refused`; `golden-mine/main.rs > synth_refuses_to_write_over_its_base`; sha256 de `golden.json` igual tras generar | Cumple |
| R6 Test autocontenido | sin `docs/eval` | `report.rs > a_results_file_without_the_block_still_diffs_against_one_with_it`, que pasa también en un árbol de `c065da5` sin `docs/eval/` | Cumple |
| R7 Intrusos | un intruso repetido | `audit.rs > a_row_in_the_top_five_of_half_the_cases_is_the_subsets_intruder` | Cumple |
| R7 | la esperada no es intrusa | `the_expected_row_is_never_its_own_cases_intruder`, `a_row_expected_by_one_case_still_intrudes_on_another` | Cumple |
| R7 | no cambia el veredicto | `the_audit_carries_the_intruders_without_changing_the_verdict`; `synth_roundtrip.rs > the_audit_names_an_intruder_for_every_generated_subset` | Cumple |
| R8 Criterio de salida | subconjunto saturado / par insensible | criterio evaluado sobre la medición (tabla C1 a C6) | Medido: C4 y C5 fallan, aceptados |

**21 de 21 escenarios de R1 a R7 con test en verde.** R8 es un procedimiento de medición, no un comportamiento del código.

## Coherencia con el diseño

| Decisión | ¿Se siguió? | Nota |
|---|---|---|
| D1 archivo combinado | Sí | `--out` igual a `--base` es error; imprime los dos sha256 (`golden-mine/main.rs:339-340`) |
| D2 vocabulario de la base | Sí | Mezcla dos filas del mismo `(memory_type, lang)` (`synth/mod.rs:88-112`) |
| D3 SplitMix64 | Sí | `synth/rng.rs`, vectores fijados; ninguna dependencia nueva (no cambia ningún `Cargo.toml`) |
| D4 `CaseSubset` enum | Sí | Con `LowImportance` de D9. `known_red` queda fuera porque `aggregate_by_subset` recibe sólo `per_case` activos (`rag-eval/main.rs:436-451`) |
| D5 generador puro | Sí | `run_synth` vuelve antes de `production_base_dir` (`golden-mine/main.rs:195`); `SynthError` con `thiserror`. Prefijo `s-lo-` sumado por D9 |
| D6 test inline | Sí | |
| D7 mención al pasar | Sí | |
| D8 largo y anti-copia | Sí | Tope de 3 marcos y 50 reintentos (`synth/mod.rs:19,22`) |
| D9 entidad concurrida | Sí, con un desvío de texto | D9 dice que la esperada de `low_importance` "tiene 4"; el código sigue la spec (esperada en [3, 5]) y la salida usa 3, 4 y 5. Ver SUGGESTION 3 |
| D10 intrusos y criterio | Sí | `top_intruders` puro, no toca `Verdict` |

## Strict TDD

| Chequeo | Resultado | Detalle |
|---|---|---|
| Evidencia TDD | Parcial | No hay `apply-progress`; la evidencia es la línea RED de cada sección de `tasks.md`. Test y código van en el mismo commit, así que el orden RED → GREEN no se puede comprobar desde git |
| Cada tarea tiene test | Sí | Las secciones 1 a 18 nombran su test y el test existe |
| RED (los tests existen) | Sí | Todos los nombrados están en el código |
| GREEN (pasan) | Sí | 1566/1566 |
| Triangulación | Sí | Los conteos se prueban con 0, 10 y 500 distractores; la intrusión, con casos positivos y negativos |
| Red de seguridad | Sí | La suite completa de `clawg-eval` pasa en cada commit (tabla de atomicidad) |

Capas: unit en `golden-mine` (`synth/*`, `main.rs`) y `rag-eval` (`golden.rs`, `report.rs`, `audit.rs`); integración en `tests/synth_roundtrip.rs` (5 tests, corre los dos binarios con `--fake`). La medición `--real` es manual por diseño (necesita el modelo).

## Reglas de commits

Revisados los 19 commits del rango:

- Ninguno tiene cuerpo, `Co-Authored-By` ni atribución de IA. Todos son conventional commits.
- Ni los mensajes ni las líneas agregadas en `crates/` nombran tareas, fases, planes, specs ni ids `R*`, `D*`, `C*`.
- Ningún `#[allow]` nuevo. Ninguna dependencia nueva. Ningún archivo de `openspec/`, `docs/eval/` ni `docs/migration-plan.md` commiteado (`git diff --stat e2dda9b~1..c065da5`: sólo `crates/clawg-eval/**`, `.gitignore` y `.claude/CLAUDE.md`).
- Sin código muerto nuevo: los warnings de `dead_code` son los anteriores.

## Problemas

**CRITICAL**

Ninguno.

**WARNING**

1. **C4 no se cumple con la redacción corregida, y queda aceptado.** Después de esta verificación la spec y D10 pasaron a pedir `|MRR(high) − MRR(low)| <= 0,05` aplanada. La corrección es posterior a la medición pero va en el sentido más estricto: no convierte un fallo en cumplimiento, sino al revés, así que no es mover el umbral a favor del resultado. Con la redacción original, aplanada, la diferencia MRR(high) − MRR(low) es −0,0729: cumple `<= 0,05` tal como está escrito, pero su valor absoluto supera 0,05 y el signo se invierte. La intención de la spec ("el par tiene que medir la importancia y nada más") pide `|diff| <= 0,05`. Hay dos problemas: la spec tiene un hueco (falta el valor absoluto) y el par tiene un sesgo estructural: sin importancia, `low_importance` rankea mejor que `high_importance` (MRR 0,5965 frente a 0,5236; R@1 0,4167 frente a 0,2500) por algo distinto de la importancia (aspecto elegido, largo o idioma de las filas). La brecha de 0,3394 con la importancia por defecto incluye ese sesgo, así que el efecto propio de la importancia es de unos 0,41 (0,3394 + 0,0729), no 0,34.
2. **C5 no se cumple, y queda aceptado.** `s-lex-19-0` ("El comando falló con `E_AUTH_3252` porque el servicio externo no respondió a tiempo.") aparece en el top-5 de 6 de los 46 casos `natural` (13,0 %, límite 10 %). Por la regla de D10 se registra tal cual; el usuario decidió no iterar más el generador y no se movió el umbral. Consecuencia: el R@5 `natural` de 0,3696 está deprimido en parte por una fila sintética concreta, no sólo por la presión del tamaño del corpus.
3. **`SynthError::DistractorCopiesBase` no tiene test de su camino de error.** El tope de 50 reintentos (`synth/mod.rs:22`, `:93-101`) nunca se ejercita. Las otras tres variantes sí tienen test (`distractors_need_at_least_one_base_row`, `a_duplicate_ref_is_an_error`, `a_duplicate_query_is_an_error`). No es CRITICAL: ningún escenario de la spec lo exige y la invariante anti-copia sí está probada sobre la salida, pero en modo Strict TDD es un camino de error sin RED.
4. **Test intermitente fuera del cambio:** `memory::search::tests::disputed_status_does_not_change_score` (`clawg-core`, de `46c518c`). Compara `f64` con `assert_eq!` sobre dos filas cuyo `last_accessed_at` sale de `strftime('%s','now')`; si los inserts caen en segundos distintos, falla. Hizo fallar 1 de 2 corridas completas. No lo introduce este cambio, pero rompe la regla de "suite en verde" de forma aleatoria.

**SUGGESTION**

1. **Fuera del alcance, para lexical-fusion:** el canal léxico cambia 0 de 112 rankings. Los candidatos que sólo trae el FTS se re-puntúan por coseno y compiten por los mismos `k_final` puestos (`crates/clawg-core/src/memory/search.rs`, ~943-967), así que una fila fuera del KNN no le gana a las 10 primeras. El R@5 `lexical` de 0,4333 es el `B` de una fusión futura (por rango, p. ej. RRF) con los umbrales congelados de su propuesta.
2. ~~Corregir la spec de C4 a `|MRR(high) − MRR(low)| <= 0,05` aplanada~~ (aplicado en spec y D10; C4 queda como fallado, ver WARNING 1). Pendiente: estudiar el sesgo del par antes de usarlo para medir un cambio del factor de importancia.
3. ~~Alinear `design.md` D9 ("la esperada tiene 4") con la spec y el código (esperada en [3, 5])~~ (aplicado).
4. R1 dice que el generador no lee `~/.clawg*` ni la red. Se cumple por construcción (`run_synth` no llama a `production_base_dir` ni a `EvalEmbedder`), pero ningún test lo fija. `synth_roundtrip.rs` podría correr `golden-mine synth` con `HOME`/`USERPROFILE` apuntando a un directorio vacío.
5. En un árbol sin `docs/eval/`, 3 tests de `rag-eval` siguen fallando: `diff::tests::self_diff_is_a_no_op`, `diff_isolates_similarity_from_ranked_movement` y `diff_files_reads_both_paths_and_produces_the_same_result_as_diff_values`. Son anteriores a este cambio y R6 sólo cubría el de `report.rs`, pero el mismo arreglo (fixture inline) los haría pasar en un clon limpio.
6. Un test de `aggregate_by_subset` con un caso `known_red` dejaría explícito que queda afuera; hoy depende de que el llamador filtre (igual que `aggregate_by_lang`).

## Veredicto

**Pasa con advertencias.** El código cumple R1 a R7 con tests en verde, cada commit compila y pasa, y la baseline de 18.3 a 18.5 se reprodujo exacta. El set generado queda como baseline aceptada con C4 y C5 fallados, ambos registrados sin relajar umbrales.

## Diferencias con la spec al consolidar

Al pasar la spec a `specs/eval/spec.md` se contrastó cada afirmación con el código. La spec principal dice lo que hace el código:

- **Dónde escribe el generador:** la spec decía "escriba bajo `docs/eval/`". `synth` escribe donde diga `--out`, que sólo tiene que ser distinto de `--base`, por nombre y por ruta canónica (`crates/clawg-eval/src/bin/golden-mine/main.rs:118` y `:299-304`).
- **Tamaño:** es `--distractors` (500 por defecto), y los conteos fijos son 30 `lexical`, 12 `multi_ref`, 12 `high_importance` y 12 `low_importance` (`synth/mod.rs:17-29`), por encima de los mínimos de la spec.
- **Test autocontenido:** la spec lo ubicaba en `report.rs:851`; hoy es `a_results_file_without_the_block_still_diffs_against_one_with_it`, en `report.rs:1002`.
- **Valores de `subset` desconocidos:** también fallan al parsear (`an_unknown_subset_value_fails_to_parse`). La spec sólo cubría la clave mal escrita.
- **Criterio de salida:** pasa a la spec principal como baseline medida, con C4 y C5 registrados como no cumplidos y aceptados.
