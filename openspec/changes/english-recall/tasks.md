# english-recall — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar; un test que no compila se escribe y se pone en verde dentro del mismo commit. Una tarea, un commit convencional que compila, pasa `cargo test --workspace --target-dir <otro-dir>` y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`, y no deja warnings. Las **guardas** pasan antes y después de cada commit: no son RED y no se tocan para ponerlas en verde.

Restricciones de orden, verificadas contra `HEAD` (`9a78bbd`):

- Con `-D warnings`, un campo o una función sin lector es un error (`dead_code`). Por eso `Pair` y `pair_of` entran con su primer llamador (`aggregate_by_pair`, tarea 2) y `CaseView.query_lang`/`row_lang` con el bloque que los lee (tarea 3). `CaseResult.query_lang` ya tiene lector en la tarea 1, porque `Serialize` lo lee.
- Cada campo nuevo obliga a tocar todos los literales de su tipo en el mismo commit. Los que usan `..otro` no cambian.
- `RunDiff` y `AgentViewDiff` derivan `Clone, Copy, PartialEq, Default` (`diff.rs:74-108`). Un bloque de parejas es un `Vec`, que no es `Copy`: el derive se cambia en la misma tarea que suma el campo. `BlockDiff` y `MetricDiff` siguen siendo `Copy`. `regressed` hace `diff.agent_view.is_some_and(...)` (`diff.rs:48`), que mueve un `Option<AgentViewDiff>` fuera de una referencia: pasa a `diff.agent_view.as_ref().is_some_and(...)` en el mismo commit en que `AgentViewDiff` deja de ser `Copy` (tarea 4b). Los tests de `diff.rs:448` y `:467` arman `AgentViewDiff { ..AgentViewDiff::default() }`, que sigue compilando.
- La tarea 3 depende de la 2 sólo por `Pair`; la 4b depende de la 3. La 4a depende de la 2.

## 1. El idioma de la consulta llega a los resultados (R1, R2)

- [x] Tests primero. `golden.rs`: `a_case_may_name_the_language_of_its_query` (**RED**, comportamiento: `deny_unknown_fields`, `golden.rs:59`); guardas `a_case_without_query_lang_has_none` y `a_misspelled_query_lang_key_fails_to_parse`. `report.rs`: `a_case_result_without_query_lang_neither_writes_nor_requires_it` (**RED**, no compila). `main.rs`: `the_query_language_reaches_the_retrieval_and_similarity_cases` (**RED**, no compila).
- [x] `RetrievalCase.query_lang: Option<String>` con `#[serde(default)]` (`golden.rs:58-90`); `CaseResult.query_lang` con `#[serde(default, skip_serializing_if = "Option::is_none")]` (`report.rs:62-92`); `query_lang` en `EmbeddedCase` (`main.rs:949`) y en `SimCaseSource` (`main.rs:1096`), copiado en `main.rs:240` y `:262`.
- [x] `query_lang: ...` en los literales de `CaseResult`: `main.rs:446` (caso de recuperación, desde `ec.query_lang`), `main.rs:861` (desde `case.query_lang`), `main.rs:1141` (desde `src.query_lang`), `audit.rs:484` y `:664`, y el helper `case` de `report.rs:925`. No se tocan los literales con `..` (`report.rs:943`, `:1021`, `:1035`, `:1061`, `:1069`; `audit.rs:734`).
- Commit: `feat(eval): carry the language of a query from the golden set into each case result`

## 2. `by_pair` en `retrieval` y `similarity` (R2, R3)

- [x] Tests primero (`report.rs`), todos **RED, no compilan**: `pair_of_needs_both_languages`, `two_cases_differing_only_in_query_lang_fall_in_different_pairs`, `a_case_missing_either_language_has_no_pair_but_counts_in_the_aggregate`, `by_pair_is_absent_from_the_json_when_no_case_has_a_pair`, `the_console_lists_one_row_per_pair`, `a_results_file_without_by_pair_still_reads_and_diffs`. `main.rs`: `similarity_by_pair_scores_each_pair_on_its_own_cases` (**RED**, no compila, sobre `build_similarity_results`). Guarda, escrita en este commit y en verde antes y después: `a_golden_without_query_lang_measures_exactly_as_before` (`main.rs`, junto a `a_golden_without_the_new_classes_measures_exactly_as_before`, `main.rs:1361`).
- [x] `Pair` (derive `Ord`, ordenada por `(query_lang, lang)`), `pair_of`, `PairAggregate { query_lang, lang, n_cases, aggregate, hits }` y `aggregate_by_pair` en `report.rs`, en la forma de `aggregate_by_lang` (`report.rs:535`). `by_pair` en `RetrievalResults` (`from_cases`, `report.rs:433`) y en `SimilarityResults` con `skip_serializing_if = "Vec::is_empty"`; tabla de consola por pareja junto a `format_subset_table` (`report.rs:581`) y en `print_stdout` (`report.rs:760`).
- [x] Literales de `SimilarityResults` con el campo nuevo: `main.rs:1191` y `report.rs:1266`.
- Commit: `feat(eval): report retrieval and similarity by the language pair of query and row`

## 3. `agent_view` por pareja (R4)

- [x] Tests primero (`agent_view/mod.rs`), todos **RED, no compilan**: `build_results_splits_active_cases_by_pair`, `agent_view_pairs_leave_out_known_red_and_unanswerable`, `a_case_view_without_languages_reads_back_and_writes_none`.
- [x] `CaseView.query_lang` y `row_lang` con `default` y `skip_serializing_if` (`agent_view/mod.rs:70-89`); `AgentViewResults.by_pair: BTreeMap<String, SubsetSummary>` con clave `"{query_lang}>{row_lang}"`, calculado en `build_results` sobre los mismos casos que `by_subset` (`mod.rs:331`, `:348-349`) y omitido si está vacío.
- [x] `runner.rs:164`: `query_lang` desde el caso y `row_lang` con `crate::case_lang` (función privada de `main.rs`, visible desde sus módulos hijos, sin cambiar visibilidad), con un mapa `ref → lang` armado desde `input.golden_set.corpus` (`runner.rs:52-53`): `lang_by_ref` de `main.rs:208` se arma después de la ramificación a agent-view (`main.rs:190`), así que el runner no lo recibe.
- [x] Literales de `CaseView`: `runner.rs:164`, `agent_view/mod.rs:529` y `:613`. Literales de `AgentViewResults`: `agent_view/mod.rs:379` y `report.rs:1251`.
- Commit: `feat(eval): report agent-view hit rates by the language pair of query and row`

## 4a. Corte por pareja en `--diff`: retrieval y similarity (R5, R6, R7)

- [x] Tests primero (`diff.rs`), todos **RED, no compilan**: `a_pair_block_joins_only_the_cases_with_that_pair`, `six_improvements_in_one_pair_are_significant_there_only`, `a_case_whose_pair_differs_between_the_files_is_reported_apart`, `a_case_with_a_pair_in_one_file_only_is_reported_apart`, `a_case_without_a_pair_in_either_file_is_counted_nowhere`, `similarity_has_its_own_pair_blocks`, `the_global_rows_do_not_change_when_pairs_are_present`, `a_file_without_pairs_diffs_against_one_with_them`, `two_files_without_pairs_have_no_pair_block`, `a_regression_inside_a_pair_never_fails`, `the_primary_lines_ignore_the_pair_blocks`. `main.rs`: `diff_exits_zero_when_only_a_pair_regresses` (**RED**, comportamiento). Guardas ya existentes que siguen en verde: `self_diff_is_a_no_op`, `a_significant_primary_regression_fails`, `five_lost_cases_do_not_fail`, `an_informative_regression_never_fails`, `diff_exits_six_on_a_significant_regression_only_with_the_flag`.
- [x] `PairDiff { pair, joined, block: BlockDiff }` y dos conteos (`pair_differs`, `pair_in_one_file_only`) por bloque, en `RunDiff` (`diff.rs:99`) con `Vec`, de modo que `RunDiff` deja de derivar `Copy` (conserva `Clone, PartialEq, Default`). La agrupación es pura y reusa `diff_block` y `accumulate` (`diff.rs:282`, `:313`) sobre los casos de cada pareja; la pareja sale de `pair_of` sobre el `query_lang` y el `lang` del `per_case` de cada archivo. `print_stdout` (`diff.rs:327`) imprime `{bloque} by pair ...` y los dos conteos sólo si son mayores que 0. `regressed` y las líneas `primary:` no se tocan.
- Commit: `feat(eval): cut retrieval and similarity diffs by the language pair of the cases`

## 4b. Corte por pareja en `--diff`: agent-view (R5)

- [x] Tests primero (`diff.rs`), **RED, no compilan**: `agent_view_pairs_use_only_active_cases`. Guarda que cubre el cambio de `regressed`: `a_significant_primary_regression_fails` (`diff.rs:442`), que arma `AgentViewDiff` y comprueba `regressed`.
- [x] Bloque de parejas en `AgentViewDiff` sólo con `search.found` y `context.found` (sin tokens, decisión de R5), calculado en `diff_agent_view` (`diff.rs:250`) sobre casos activos; `AgentViewDiff` deja de derivar `Copy`; `regressed` pasa a `diff.agent_view.as_ref().is_some_and(...)` (`diff.rs:48`). Impresión dentro del bloque `agent_view` de `print_stdout`.
- Commit: `feat(eval): cut agent-view diffs by the language pair of the cases`

## 5. Texto de ayuda de `--diff` (R8)

- [x] Test primero (`cli.rs`): `the_diff_help_describes_the_pair_cut` (**RED**, comportamiento: `HELP_TEXT`, `cli.rs:169-177`, no habla de parejas), con el patrón de `HELP_TEXT.contains(...)` de `cli.rs:866`.
- [x] El texto de `--diff` dice que, si los archivos traen `query_lang`, también corta por pareja consulta–fila, y que ese corte no entra en `primary:` ni en `--fail-on-regression`.
- Commit: `docs(eval): describe the language-pair cut in the --diff help`

## 6. Línea de `.claude/CLAUDE.md` sobre cómo comparar dos corridas (R8)

- [x] No es testable (ningún test lee ese archivo): tarea de documentación, verificada por lectura. En la línea «A ranking change is compared with the baseline by …» (línea 109 al escribir esto; buscarla por texto), mencionar el corte por pareja y que un cambio que toque el cruce de idiomas se lee ahí.
- [x] **Cuidado.** Otra sesión tiene ediciones sin commitear en `.claude/CLAUDE.md` (aparece como `M` en `git status`). Se agrega a la tarea con `git add -p` y sólo el hunk de esta línea; nunca `git add .claude/CLAUDE.md` entero, `git add -A`, `git add .` ni `git add -u`. Antes de commitear, `git diff --cached` debe mostrar sólo ese hunk.
- Commit: `docs: describe the language-pair cut when comparing two runs`

## 7. Verificación del código

- [x] `cargo test --workspace --target-dir <otro-dir>` y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` en cada commit de las tareas 1 a 6 (los procesos `clawg.exe` vivos bloquean `target\debug\clawg.exe`; no se matan). Criterio: ambos salen con 0 en cada commit.
- [x] Las tres guardas de R1 y R3 y las existentes de `diff.rs` listadas arriba, en verde en cada commit.

## 8. Datos, fuera de git (necesitan `docs/eval/`, ignorado por git, y el golden set de 46 casos)

Ninguna de estas tareas toca el historial de git; sólo `verify.md` (tarea 12) se commitea. Dónde se corre: en cualquier máquina que tenga el golden set; las que miden necesitan además el modelo real en el caché del perfil.

- [x] **8.1 Etiquetas.** Copiar `docs/eval/golden.json` a `docs/eval/golden.pre-english-recall.json` antes de tocarlo. Después, en `golden.json`: `query_lang` en los 46 casos según la regla de etiquetado de la propuesta (idioma de la prosa; los identificadores no cuentan) y corregir `lang` de las cuatro filas c01, c08, c09 y c35. Hecho cuando: los 46 casos tienen `query_lang`, ninguna consulta, fila ni esperada cambió (diff de la copia previa contra el nuevo, sólo claves nuevas y esas cuatro filas) y los dos `sha256` quedan anotados. Necesita el código de las tareas 1 a 3 (sin él el golden se rechaza).
- [x] **8.2 Goldens de control, antes de medir cualquier brazo.** Escribir `docs/eval/golden-cross.json` (las mismas 48 filas, un caso inglés→español por cada fila en español y una segunda consulta en español por cada fila en inglés, con una regla fija) y los controles de (c) y (e): `golden-c-prosa.json`, `golden-e-consulta-en.json` y `golden-e-fila-es.json`. Hecho cuando: cada archivo valida con `rag-eval --fake --golden <archivo>` y su `sha256` queda anotado antes de la primera corrida de un brazo. Ninguno entra en `golden.json` ni en git.

## 9. Paso 0: la base (necesita el modelo real y `docs/eval/`)

- [ ] **9.1** Sobre el commit con las tareas 1 a 6: `rag-eval --real --tag english-recall-base`; su `--diff` contra `2026-10-16-real-eval-rigor-tip-anchor.json` da 0 casos movidos en `retrieval` y en `similarity`. Si algo se mueve, se para y se busca la causa. El `content_sha256` cambia sólo por las etiquetas (8.1). Hecho cuando el diff está anotado con su salida.
- [ ] **9.2** `rag-eval --real --agent-view --tag english-recall-base` y su `--diff` contra `2026-10-15-real-long-save-ranking-tip-agent-view.json`; si algo se mueve, el archivo nuevo es la base y el movimiento se anota con su p.
- [ ] **9.3** Los mismos dos con `--golden docs/eval/golden-cross.json`: la base de U6.
- [ ] **9.4** Hipótesis (f): el `by_pair` del paso 0 reproduce la tabla de la exploración (n 20/7/19; R@5 0,90/0,86/0,32 en los dos bloques); si no, el registro usa las cifras del paso 0 y dice por qué.

## 10. Hipótesis (a) a (e) (necesitan el modelo real; (b) además un worktree descartable)

- [ ] **(a)** `--sweep-cos-floor` 0,70, 0,75, 0,79, 0,81 y 0,83 sobre la base, leído por pareja.
- [ ] **(b)** Brazos A1 (`"query: "`/`"passage: "`) y A2 (`""`/`""`) contra A0 (la base). Un worktree por brazo, desde el commit del paso 0, cambiando sólo `embedder.rs:102` y `:123`, cada uno con su `--target-dir` y `--tag english-recall-b-a1` o `-a2`, sobre los dos goldens, con y sin `--agent-view`. Hecho cuando `similarity.query_prefix` y `passage_prefix` de cada archivo prueban qué brazo corrió. **Los worktrees no se mergean ni se commitean y se borran al terminar**; la rama de `main` no recibe ningún cambio de prefijo.
- [ ] **(c)** Contar los fallos de español→inglés con código (al menos un tramo entre comillas invertidas) y sin código, en A0 y en cada brazo; control `golden-c-prosa.json` medido en A0. Informativa.
- [ ] **(d)** `--lexical-only` sobre la base, leído por pareja.
- [ ] **(e)** `golden-e-consulta-en.json` y `golden-e-fila-es.json` medidos en A0 y en el brazo ganador, si lo hay.
- Hecho cuando: cada corrida tiene archivo en `docs/eval/results/`, su `--tag`, el `sha` del golden y su `--diff` contra la base, con los umbrales U1 a U10 de la propuesta sin moverse.

## 11. Borrar los worktrees descartables

- [ ] `git worktree remove` de cada brazo de (b) y borrar sus `--target-dir`. Hecho cuando `git worktree list` no muestra ninguno y `git status` no tiene cambios de prefijo.

## 12. El registro (`verify.md`)

- [ ] `openspec/changes/english-recall/verify.md`: un apartado por paso y por hipótesis, con commit, comando, `--tag` y archivo de resultados, `sha` del golden, cifras por pareja con su intervalo, filas del `--diff` con su p y el veredicto según las tablas de la propuesta; un apartado para memory-protocol (22) con las cifras por pareja; y si sigue C, D o las dos. Incluye el conteo de la suite y el resultado de `clippy` de la tarea 7. Sin números locales ni rutas personales: dice qué necesita cada paso, no en qué máquina corrió.
- Commit: `docs(openspec): record the language-pair diagnosis`

## 13. Archive (fuera de este cambio hasta que se cierre)

- [ ] La base por pareja pasa a `specs/eval` y al contexto de `openspec/config.yaml`; la conclusión, a la fila 21 de `openspec/README.md`; el subir de versión pertenece al archivado y no es una tarea de este plan.
