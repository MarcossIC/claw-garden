# eval-rigor — Verificación

**Estado:** verificado en Linux, commit por commit, y en Windows sobre `main`, con la suite y `rag-eval --real` ([Windows](#windows), [rag-eval](#rag-eval---real-en-windows)). En Windows falla un test, que es de project-root-guard y no de este cambio.

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama, en un worktree aparte.

| Commit | Pasan | Fallan | Warnings |
|---|---|---|---|
| Base (`main`, f8a361e) | 1.902 | 10 | 0 |
| `docs(openspec): propose paired tests, intervals and new case classes for rag-eval` | 1.902 | 10 | 0 |
| `test(eval): diff results from an inline fixture instead of an ignored file` | 1.905 | 7 | 0 |
| `feat(eval): report an exact paired p-value for every metric in --diff` | 1.912 | 7 | 0 |
| `feat(eval): add a paired bootstrap interval to the continuous metrics in --diff` | 1.917 | 7 | 0 |
| `feat(eval): compare what the agent received in --diff` | 1.921 | 7 | 0 |
| `feat(eval): fail a diff on a significant regression of a primary metric` | 1.927 | 7 | 0 |
| `feat(eval): report strict and partial hits with Wilson intervals` | 1.932 | 7 | 0 |
| `feat(eval): split cases into calibration and test by a hash of the query` | 1.936 | 7 | 0 |
| `feat(eval): measure abstention on cases that have no answer` | 1.943 | 7 | 0 |
| `feat(eval): measure whether a newer row outranks the one it replaces` | 1.952 | 7 | 0 |
| `feat(eval): rank by bm25 alone as a floor for the full ranking` | 1.955 | 7 | 0 |
| `docs: describe how rag-eval compares two runs` | 1.955 | 7 | 0 |

Todos los commits compilan. Las 7 fallas que quedan son las de `doctor::tests` de `main`, que modelan un PATH de Windows. Las 3 de `diff::tests` que leían `docs/eval/results/` (ignorado por git) pasan desde el segundo commit (R12).

**Conteo:** 50 tests nuevos, ninguno borrado. En Windows se espera la base más 50, sin fallas.

`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina con 0 en la punta de la rama.

### Windows

`cargo test --workspace --no-fail-fast --target-dir target-test`:

| Commit | Pasan | Fallan | Ignorados | Warnings del workspace |
|---|---|---|---|---|
| `main` en `210c692` (este cambio y project-root-guard ya mergeados) | 1.983 | 1 | 0 | 0 |

1.984 es la base de 1.918, más los 50 de este cambio y los 16 de project-root-guard. Los 7 de `doctor::tests` que fallan en Linux pasan en Windows, y todos los de este cambio también.

La única falla es `projects::tests::adding_a_project_lets_its_docs_be_searched` (`crates/clawg-cli/src/projects.rs:57`). Viene de project-root-guard (`b72d0a9`) y el error está en el test, no en `add`:

- el test registra la etiqueta con `std::fs::canonicalize` (`projects.rs:47`), que en Windows deja el prefijo `\\?\`;
- `add` resuelve la raíz con `ProjectIdentity::resolve`, que lo saca (`strip_verbatim`, `crates/clawg-core/src/memory/identity.rs:133`);
- las dos formas de escribir la misma carpeta dan ids distintos (`.tmp6qjxhe-d2c5` y `.tmp6qjxhe-5a2e`).

En Linux no hay prefijo, y por eso pasa. Queda para la verificación de project-root-guard en Windows.

`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` sobre `210c692`: **código 0**, sin ningún warning.

## Lo que muestra el diff nuevo

Dos archivos sintéticos con la forma del caso de project-indexing: 46 casos, R@5 baja en 3 y `search.found` de agent-view baja de 41 a 30, con 100 tokens menos por búsqueda. Antes, `--diff` daba 0/0/0 en agent-view. Ahora:

```
  agent_view (fewer tokens is better; the mean is second run minus first):
    metric          better  worse     net       p
    search.found         0     11     -11  0.0010
    context.found        0      0       0  1.0000
    search.tokens       46      0      46  0.0000  mean -100.0000 [-100.0000, -100.0000]
    context.tokens       0      0       0  1.0000  mean +0.0000 [+0.0000, +0.0000]
  primary:
    retrieval recall@5: not distinguishable with 3 case(s) moving (p = 0.2500); a conclusion needs at least 6 cases moving one way
    agent_view search.found: distinguishable, worse (p = 0.0010, 0 better / 11 worse)
a primary metric got worse with p < 0.05
```

Con `--fail-on-regression` sale con 6.

## Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1 | `stats.rs`: `wilson_*`, `mcnemar_exact_*`, `bootstrap_*`, `six_cases_one_way_is_the_smallest_significant_count`, `splitmix_matches_the_golden_mine_sequence` |
| R2, R3 | `diff.rs`: `six_one_way_improvements_are_significant`, `one_improvement_is_not_distinguishable_and_says_how_many_are_needed`, `the_verdict_never_says_there_is_no_difference`, `a_continuous_metric_carries_a_bootstrap_interval`, `a_found_flip_in_agent_view_shows_up`, `fewer_tokens_count_as_better`, `agent_view_joins_on_query_and_skips_known_red`, `a_file_without_agent_view_diffs_without_the_block`, `abstention_is_diffed_when_both_files_have_it`, `freshness_is_diffed_when_both_files_have_update_cases` |
| R4 | `diff.rs`, `cli.rs` y `main.rs` (`diff_exits_six_on_a_significant_regression_only_with_the_flag`) |
| R5, R6, R7 | `report.rs` |
| R8, R9, R10 | `golden.rs`, `metrics.rs`, `seed.rs`, `report.rs`, `agent_view/mod.rs`, `eval_api.rs` y los de punta a punta de `main.rs` |
| R11 | `lexical_search_ranks_by_bm25_without_vectors` (`eval_api.rs`), `lexical_only_conflicts_with_the_other_arms`, `a_lexical_only_run_writes_a_tagged_file_that_diffs` |
| R12 | los tres de `diff.rs`, en verde en Linux sin `docs/eval/` |

## rag-eval --real en Windows

Corrido con el binario de `main` en `210c692` y el modelo cacheado del perfil dev (`multilingual-e5-small/mean`), sobre un store temporal. `rag-eval` nombra los archivos con la fecha UTC, y por eso son del 2026-10-16.

1. **Ancla.** `rag-eval --real --tag eval-rigor-tip-anchor` sobre `docs/eval/golden.json` da R@1 0,4348, R@5 0,6522, R@10 0,8478, MRR 0,5524 y nDCG@10 0,6215. El `content_sha256` (`d2d9d732…`) es el de `2026-10-15-real-project-indexing-tip-anchor.json`. El `--diff` contra ese archivo da 0 mejores y 0 peores en todas las métricas de `retrieval` y `similarity`, p = 1 e intervalos [0, 0], así que R10 se cumple con el set real. El reporte nuevo trae:
   - `hits`: estricto y parcial en 5, 30/46 = 0,6522 [0,508, 0,773]. Coinciden porque cada caso de este set espera una sola fila;
   - `by_split`: calibración 33 casos con R@5 0,6667, y prueba 13 con R@5 0,6154;
   - la nota «at least 6 cases moving one way (13.0 points of 46)».

   Archivo: `2026-10-16-real-eval-rigor-tip-anchor.json`.
2. **`relsweep` de agent-view.**

   | Comparación | `search.found` | p | Veredicto | `search.tokens` (media) |
   |---|---|---|---|---|
   | 0,60 contra 0,35 | 6 mejores, 0 peores | 0,0313 | «distinguishable, better» | 0 mejores, 35 peores, +309,4 [+239,6, +379,7] |
   | 0,35 contra 0,20 | 1 mejor, 0 peores | 1 | «not distinguishable» | 0 mejores, 20 peores, +130,3 [+74,9, +195,3] |

   Es lo que se había anticipado en Linux. `--diff` ve ahora lo que antes daba 0/0/0: bajar el corte de 0,60 a 0,35 hace que la búsqueda le muestre al agente 6 casos más, a cambio de ~309 tokens por búsqueda.
3. **Piso léxico.** `rag-eval --real --lexical-only` da R@1 0,3478, R@5 0,5000, R@10 0,5000, MRR 0,4040 y nDCG@10 0,4279. En inglés R@5 es 0,2727 y en español 0,7083. El `--diff` del ancla contra el piso:

   | Métrica | Mejores | Peores | p | Media |
   |---|---|---|---|---|
   | R@5 | 3 | 10 | 0,0923 | — |
   | R@10 | 0 | 16 | < 0,0001 | — |
   | MRR | 6 | 21 | 0,0059 | −0,1485 [−0,2643, −0,0333] |

   El ranking completo le gana al piso de BM25 en R@10 y en MRR. En R@5, 7 casos netos no alcanzan para distinguir con 46 casos. Archivo: `2026-10-16-real-lexical-only.json`.

## Diferencias entre la spec del cambio y el código

Encontradas al consolidar en [`specs/eval`](../../../specs/eval/spec.md), que describe el código. Todos los tests que nombra la spec existen con ese nombre. Las rutas son relativas a `crates/clawg-eval/src/bin/rag-eval/`.

1. **R2, intervalo en `similarity`:** el bloque `similarity` también lleva intervalo en MRR y nDCG@10, porque se arma con el mismo `diff_block` (`diff.rs:202-203`). Con cero casos unidos, el intervalo se imprime igual: `mean +0.0000 [+0.0000, +0.0000]`.
2. **R2, la línea primaria de R@5:** la spec pide una línea «por métrica primaria presente», pero la de R@5 se imprime siempre (`diff.rs:351`). Entre dos archivos de agent-view, cuyo `retrieval.per_case` está vacío, dice «not distinguishable with 0 case(s) moving».
3. **R2, cuántos casos hacen falta:** la línea dice siempre «at least 6 cases moving one way» (`diff.rs:63-65`). Ese es el mínimo cuando ningún caso se mueve en la otra dirección. Con movimiento mixto hacen falta más, y la línea no lo dice. En el diff del ancla contra el piso léxico se mueven 13 casos de R@5 (3 mejores y 10 peores, p = 0,0923), y la línea repite el 6.
4. **R3, tokens y abstención:**
   - las filas de tokens del diff incluyen los casos `unanswerable` (`diff.rs:265-270`), y el resumen de tokens de agent-view los excluye (`agent_view/mod.rs:331-334`);
   - un caso cuenta como `unanswerable` si lo es en cualquiera de los dos archivos (`diff.rs:258`);
   - la fila `search.abstain` se imprime sólo si algún caso se movió (`diff.rs:338`).
5. **R4, el orden de los rechazos:**
   - `rag-eval --fail-on-regression` sin modo ni `--diff` falla primero por falta de modo (`cli.rs:367`), no con `FailOnRegressionWithoutDiff`. Igual sale con 1;
   - después de `--diff a b`, todo lo que no sea el flag se ignora sin aviso (`cli.rs:294`).
6. **R8, abstención y `known_red`:**
   - en recuperación, un caso `unanswerable` va a `unanswerable` antes de que se mire si es `known_red` (`main.rs:462`), así que la abstención de recuperación lo cuenta;
   - la de agent-view cuenta sólo los activos (`agent_view/mod.rs:350-356`);
   - un caso `known_red` sin esperadas, que en `f8a361e` validaba, ahora falla, porque la regla no mira el estado (`golden.rs:343-347`).
7. **R9, fuera de `retrieval`:**
   - agent-view siembra por la ruta real de guardado y no lee `age_days`: nada en `agent_view/` lo nombra. Ahí, las filas de un caso `update` tienen todas la misma edad;
   - `--follow-up` deja `fresh_first` en `None` (`main.rs:874`);
   - un caso `update` que además es `known_red` no entra en `freshness`.
8. **R10, el test:** `a_golden_without_the_new_classes_measures_exactly_as_before` no compara `content_sha256`. Ese hash sale de los bytes crudos del archivo (`golden.rs:278`), así que no puede cambiar. Con el set real, la corrida de Windows dio el mismo `d2d9d732…`.
9. **R11, lo que la spec no dice:**
   - `--audit` se acepta junto a `--lexical-only` (`cli.rs:403-413`);
   - si FTS falla, da una lista vacía sin avisar (`crates/clawg-core/src/memory/search.rs:699-704`), y la corrida sale entera en cero;
   - se busca con la consulta tal cual, sin la historia (`main.rs:418-419`);
   - `--tag X` da `X-lexical-only` (`main.rs:1082-1088`).
10. **`golden-mine`:**
    - su esquema espejo (`golden-mine/synth/golden.rs:1-4`) no conoce `age_days`, `stale_refs`, `update` ni `unanswerable`, y rechaza las claves que no conoce;
    - por eso `synth --base` sobre un golden que los use falla al leerlo, y es a propósito.
11. **Nombres de subconjunto:** `every_subset_name_parses_in_snake_case` (`golden.rs:506`) lista sólo los seis valores de antes. `update` y `unanswerable` se parsean en los tests de validación.

Ninguna de estas diferencias cambia un número de los que se midieron arriba. La 3 puede hacer que se lea mal un diff con movimiento mixto, y la 6 ya no deja escribir un golden que antes valía. Las dos quedan anotadas en el backlog.
