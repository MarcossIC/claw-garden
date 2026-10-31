# docs-index-cost — Verificación

Verifica los grupos 1 a 9 de [tasks.md](tasks.md) (commits `21ca2b9..15ce920`) contra la [spec](spec.md) (IC1 a IC8) y el [diseño](design.md) (D1 a D14). El grupo 10, las corridas D y R con el modelo real, no se corrió: necesita autorización (ver «Corridas reales»).

## Windows (2026-10-31)

- `target-test` se reusó como lo dejó la aplicación, sin `cargo clean`.
- Sobre el árbol de `15ce920` (el `HEAD`, `b327fe8`, sólo marca tareas en `tasks.md`), los tres comandos salen con 0:
  - `cargo test --workspace --no-fail-fast --target-dir target-test`, con `tests/binary.rs`. Pasa entero en la primera corrida. `memory::search::tests::disputed_status_does_not_change_score`, que a veces falla con la máquina cargada, pasó en las cuatro corridas de la suite (la limpia y las tres de mutaciones);
  - `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`;
  - `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`, lo que compila el release, sin `testing` ni `eval`.
- Cada prueba que nombran la spec y las tareas aparece `ok` en la corrida, con su nombre exacto.
- Las tres pruebas `#[ignore]` de la suite son las de antes (`import_memory.rs` y `transport_bar.rs`).
- `docs/eval/cache/` no existe después de las cuatro corridas de la suite: ninguna prueba escribe en la carpeta real de la memoria. En el temporal del sistema no quedó ningún `rag-eval-test-*`. Sí hay un `rag-eval-docs-3ae147e5-…` de 59 MB, creado el 2026-10-30 a las 14:32 y escrito hasta las 15:13. Por el nombre lo hizo `docs::run`, no una prueba (las pruebas usan `TempPath`), y el `memory.lock` que conserva dice que el proceso murió antes de `remove_scratch`. No es de este cambio ni de esta verificación, y no se tocó.
- Al terminar, `git status` está limpio y `git diff 15ce920 -- crates Cargo.toml Cargo.lock` no da nada. Las mutaciones se revirtieron con `git restore` de los archivos mutados, que estaban limpios.

## Qué se vio fallar antes de cada cambio

No hay un registro de la aplicación con el RED de cada prueba (en modo openspec no hay `apply-progress`). `tasks.md` marca RED y GREEN por grupo. Leyendo cada commit, cada prueba nueva usa un tipo o una función que el mismo commit crea, así que sin el código no compila:

| Commit | Lo que las pruebas nombran y el commit crea |
|---|---|
| `21ca2b9` | `RefreshRun`, `RefreshTiming`, `DocTiming`, `DocPhases` |
| `50ae323` | `refresh_timing_line` |
| `079f473` | `EvalEmbedder::of`, `passage_tokens_ran` |
| `1da1230` | `EvalCpu`, `eval_api::session_threads` |
| `36b9de2` | el módulo `vector_memo` entero |
| `7f2561b` | `EvalIndexRun`, `IndexBlock`, `Phases`, el campo `index` |
| `8c03a1d` | `passage_lengths`, `lengths_by_doc`, `Tokens`, `padding_warning` |
| `e21b4a6` | `area_of`, `ByArea` |
| `15ce920` | `memo_dir`, `default_cache_dir`, `CliError::NoCacheWithoutDocs`, `Cli::cache_dir`, `RealRun` |

Hay dos excepciones, que son guardas y pasan sin código nuevo: `production_never_names_the_archive` (`e21b4a6`) y `docs_mode_with_the_fake_embedder_writes_no_file` (`15ce920`). `the_docs_log_line_shows_the_reused_passages` ya existía y no cambia.

Que una prueba no compile sin el código no dice que muerda la lógica. Eso lo miden las mutaciones de abajo.

## Las mutaciones

Se aplicaron en tres tandas, cada mutación a una pieza distinta. Cada tanda se corrió con `cargo test --workspace --no-fail-fast --target-dir target-test` y se revirtió antes de la siguiente. Las fallas de cada tanda se atribuyeron por la prueba y por el mensaje.

| # | Mutación | Prueba que falla | Resultado |
|---|---|---|---|
| A1 | el decorador busca también en lo que embebió en la misma corrida (`crates/clawg-core/src/eval_api/vector_memo.rs:312`) | `what_a_run_adds_is_not_used_until_the_next_run` (`vector_memo.rs:963`): el embedder recibe `["same"]` y no `["same", "same"]` | muerde |
| A2 | `put_text` sin el largo delante (`vector_memo.rs:41`) | `the_memo_key_is_stable` (`:711`), `the_memo_key_separates_neighbouring_components` (`:706`, sólo el segundo par; ver SUGGESTION 2) | muerde |
| A3 | `passage_tokens_ran` suma también `role=query` (`crates/clawg-core/src/eval_api.rs:626`) | `padded_ran_sums_only_passage_batches` (`eval_api.rs:2008`): 60 en vez de 40 | muerde |
| A4 | `usable` no mira si es finito (`vector_memo.rs:249`) | `a_vector_the_embedder_returns_unusable_is_not_stored` (`:1008`): 2 vectores guardados en vez de 1 | muerde |
| A5 | el refresco no anota la entrada del doc que no se leyó (`crates/clawg-core/src/memory/project_docs/mod.rs:524`) | ninguna | **no muerde** (WARNING 1: sin costura no se puede provocar la carrera entre el recorrido y la lectura) |
| A6 | el refresco no anota la entrada del doc cuyo vector no sirve (`project_docs/mod.rs:547`) | `a_refresh_times_a_doc_whose_vector_is_unusable` (`5f44ffc`): falta `("a.md", 0)` | muerde |
| B1 | `hash_contract` sin el SHA del modelo (`vector_memo.rs:54`) | `the_memo_key_changes_with_each_component_of_the_contract` («model sha»), `the_memo_key_is_stable`, `a_memo_made_under_another_contract_is_not_used` («model sha»: `(true, 3, 0)`) | muerde |
| B2 | `--no-cache` lee y escribe igual: `memo_dir` no mira `no_cache` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:329`) | `the_memo_is_used_only_by_a_real_docs_run_that_did_not_ask_for_none` (`cli.rs:1130`), `no_cache_leaves_the_folder_byte_for_byte` (`docs.rs:1276`, «a folder that was not there is not created») | muerde |
| B3 | `area_of` compara `openspec/changes/archive` sin la barra (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:173`) | `area_of_a_path_is_decided_by_its_prefix_and_kind` (`index_cost.rs:472`): `archived/x.md` da `Archive` | muerde |
| B4 | `finish` sólo si el índice terminó bien (`crates/clawg-eval/src/bin/rag-eval/docs.rs:772`) | ninguna en `docs.rs` | **no muerde** en `docs.rs` (WARNING 3: `rag-eval` sólo construye `EvalEmbedder::fake()`, que no falla); la conducta de `finish` tras un índice fallido la sostiene `what_a_run_embedded_before_the_index_failed_is_kept` (`b0623f8`) |
| C1 | `finish` reescribe el archivo aunque la corrida no agregó nada (`vector_memo.rs:461`) | `a_run_that_adds_nothing_does_not_touch_the_file` (`b0623f8`): el `modified()` cambia | muerde |
| C2 | un solo nombre de archivo para todos los contratos (`vector_memo.rs:108`) | `a_memo_made_under_another_contract_is_not_used` (`b0623f8`): el primer contrato pierde su memoria | muerde |
| C3 | el decorador manda lo que falta de a un texto por llamada (`vector_memo.rs:316-317`) | `an_empty_memo_embeds_in_the_batches_production_does` (`:925`), `a_changed_doc_embeds_only_its_new_texts` (`:948`) | muerde |
| C4 | `RefreshTiming::phases` no suma `remove` a `write` (`project_docs/mod.rs:144`) | `the_docs_timing_line_names_every_phase` (`crates/clawg-cli/src/daemon/api.rs:3211`: `write 0.5`), `a_doc_that_was_not_read_still_counts_its_read_time` (`project_docs/mod.rs:1151`) | muerde |
| C5 | `lengths_by_doc` cuenta sin la ventana: `encode_batch(…, true)` en vez de `window_encodings` (`eval_api.rs:1399`) | `a_passage_longer_than_the_window_counts_the_window` (`891e28d`): 700 en vez de 512 | muerde |
| C6 | `EvalEmbedder::real` vuelve a pasarle `Arc::default()` a `from_parts` (`eval_api.rs:581`) | ninguna | no muerde, y se esperaba: necesita el modelo (SUGGESTION 4) |

En B2, la carpeta que ya tenía la memoria quedó con los mismos bytes porque la segunda corrida no agregaba nada. La prueba falla por la carpeta que no existía. Eso es lo esperable, y explica también por qué C1 no muerde: `encode` escribe las entradas en orden de clave, así que reescribir la misma memoria da los mismos bytes, y una comparación de bytes no puede ver una reescritura.

## Cumplimiento de la spec

Cada escenario contra la prueba que lo sostiene, corrida en verde en esta verificación. ✅ cumple; ⚠️ parcial.

| Requisito | Escenario | Prueba | |
|---|---|---|---|
| IC1 | los tiempos no tocan el reporte | `timing_does_not_change_what_a_refresh_writes` (`project_docs/mod.rs:1099`): `RefreshReport`, pasajes y blobs de vectores iguales | ✅ |
| IC1 | una fase por doc | `a_refresh_times_each_phase_of_every_doc` (`:1070`), `an_index_run_carries_the_timings_of_the_refresh` (`eval_api.rs:2824`) | ✅ |
| IC1 | nada cambió | `an_unchanged_refresh_has_no_doc_timings` (`:1120`): sin docs, `embed` en cero, `scan > 0` | ✅ |
| IC1 | un doc que no se leyó | `a_doc_that_was_not_read_still_counts_its_read_time` (`:1136`) prueba la suma de `RefreshTiming::phases`, no que el refresco anote la entrada (A5, A6) | ⚠️ |
| IC2 | la línea | `the_docs_timing_line_names_every_phase` (`api.rs:3177`), con los literales de `seconds` (`read 0.2`, `write 0.8`) | ✅ |
| IC2 | lo demás no cambia | `the_docs_log_line_shows_the_reused_passages` (`api.rs:3218`) y las series de `crates/clawg-cli/tests/api.rs:3175-3180`, sin tocar | ✅ |
| IC3 | otro prefijo, otro modelo, otra dimensión, otro SHA | `a_memo_made_under_another_contract_is_not_used` (`vector_memo.rs:1026`), con `from_memo = 0` y el embedder recibiendo los tres textos | ✅ |
| IC3 | otro rol, otro texto (un carácter, un espacio) | `the_memo_key_changes_with_each_component_of_the_contract` (`:668`) | ✅ |
| IC3 | otra máquina, de a un campo y los hilos | la misma tabla: `os`, `arch`, `avx2`, `avx512f`, `fma`, `threads`. `from_memo = 0` sale de la clave, que el decorador busca tal cual | ✅ |
| IC3 | lo mismo da la misma clave | `the_memo_key_is_stable` (`:709`), un literal de SHA-256, así que vale entre procesos; `the_memo_key_separates_neighbouring_components` (`:695`) | ✅ |
| IC4 | la segunda corrida no embebe nada | `a_full_memo_embeds_nothing_and_stores_the_same_vector_bytes` (`:881`) | ✅ |
| IC4 | la memoria vacía embebe como producción | `an_empty_memo_embeds_in_the_batches_production_does` (`:908`) | ✅ |
| IC4 | sólo se embeben los textos nuevos | `a_changed_doc_embeds_only_its_new_texts` (`:926`) | ✅ |
| IC4 | lo nuevo no se usa en la misma corrida | `what_a_run_adds_is_not_used_until_the_next_run` (`:952`) | ✅ |
| IC4 | las preguntas no pasan por ella | `questions_never_go_through_the_memo` (`:969`) | ✅ |
| IC4 | `--no-cache` | `no_cache_leaves_the_folder_byte_for_byte` (`docs.rs:1264`), `the_memo_is_used_only_by_a_real_docs_run_that_did_not_ask_for_none` (`cli.rs:1118`); `index.memo.used = false`, en `the_index_block_reports_the_memo_and_the_phases` | ✅ |
| IC4 | `--fake` | `docs_mode_with_the_fake_embedder_never_touches_the_memo` (`docs.rs:1280`), `no_cache_without_docs_is_an_error` (`cli.rs:1099`) | ✅ |
| IC4 | la ruta no depende del directorio de trabajo | `the_memo_folder_sits_next_to_the_results_folder` (`cli.rs:1109`) | ✅ |
| IC4 | sólo claves y vectores | `the_memo_file_holds_no_text` (`vector_memo.rs:1012`) | ✅ |
| IC5 | truncada | `a_truncated_memo_is_ignored_with_a_warning` (`:532`) | ✅ |
| IC5 | cabecera desconocida o ilegible | `a_memo_with_an_unknown_header_is_ignored` (`:548`: marca, versión, contrato, dimensión, un byte del medio), `an_unreadable_memo_is_ignored` (`:588`) | ✅ |
| IC5 | la escritura se corta | `a_failed_write_leaves_the_previous_memo` (`:639`), `a_write_that_fails_is_a_warning_and_not_an_error` (`:1073`); el reemplazo de una ilegible, `a_memo_that_cannot_be_read_is_replaced_by_what_the_run_embedded` (`:1055`) | ✅ |
| IC6 | relleno de una tanda | `padding_of_a_batch_is_rows_times_the_longest` (`index_cost.rs:341`) | ✅ |
| IC6 | más de una tanda | `a_doc_over_embed_batch_runs_several_batches` (`:348`) | ✅ |
| IC6 | ordenar puede rellenar más | `sorting_can_pad_more_when_the_last_batch_is_short` (`:368`), `one_batch_pads_the_same_in_any_order` (`:355`), `sorting_keeps_ties_in_their_order` (`:378`) | ✅ |
| IC6 | ordenar entre docs | `sorting_across_docs_batches_the_whole_run` (`:384`) | ✅ |
| IC6 | ventana de 512 | ninguna prueba del conteo pasa por un pasaje más largo que la ventana (C5). `window_encodings` sí está probada en el embedder (`a_long_text_is_cut_to_the_window_and_keeps_its_closing_token`, `crates/clawg-core/src/memory/embedder.rs:1087`) | ⚠️ |
| IC6 | sin pasajes | `an_empty_corpus_has_no_tokens` (`:396`) | ✅ |
| IC6 | el aviso | `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` (`:413`); que el archivo los lleve, `the_index_block_carries_the_tokens_of_a_real_run` (`docs.rs:1169`) | ✅ |
| IC6 | el recorder | `padded_ran_sums_only_passage_batches` (`eval_api.rs:1999`), `the_real_embedder_keeps_its_recorder` (`:2012`, por `EvalEmbedder::of`, como permite la spec) | ✅ |
| IC7 | la ruta decide, código bajo archive, prefijos parecidos | `area_of_a_path_is_decided_by_its_prefix_and_kind` (`index_cost.rs:460`) | ✅ |
| IC7 | las áreas suman el total | `areas_add_up_to_the_totals` (`:477`), con `remove = 0` (`:440`); con un `remove` mayor que cero, `write` no suma (WARNING 2) | ⚠️ |
| IC7 | una tanda múltiple | `only_docs_over_a_batch_count_as_multi_batch` (`:508`) | ✅ |
| IC7 | producción no nombra el archivo | `production_never_names_the_archive` (`project_docs/mod.rs:812`) | ✅ |
| IC8 | el bloque | `the_index_block_reports_the_memo_and_the_phases` (`docs.rs:1074`) | ✅ |
| IC8 | sin memoria | la misma, con `None`: `{used: false, from_memo: 0, embedded: 100, seconds: 0.0}` | ✅ |
| IC8 | sin texto ni rutas | `the_index_block_holds_no_text_and_no_path` (`:1096`), `the_areas_hold_no_path` (`index_cost.rs:523`) | ✅ |
| IC8 | lo de antes no se mueve | `index_seconds_and_corpus_keep_their_shape` (`docs.rs:1105`) | ✅ |
| IC8 | `--diff` de un archivo viejo contra uno nuevo | `a_results_file_without_an_index_block_still_diffs` (`:1121`), `a_results_file_with_an_index_block_still_diffs_against_an_old_one` (`:1134`) | ✅ |
| IC8 | el informe de stdout | `the_docs_report_names_the_memo_file` (`:1292`), `a_fake_run_reports_no_tokens_and_no_memo` (`:1310`), `the_report_shows_the_phases_after_its_first_line` (`:1146`), `the_report_prints_one_line_per_area_of_a_real_run` (`:1190`), `docs_mode_with_the_fake_embedder_writes_no_file` (`crates/clawg-eval/tests/docs_mode.rs:121`) | ✅ |

**41 de 44 escenarios cumplen; 3 cumplen en parte** (el doc que no se leyó, la ventana de 512 y la suma de las áreas). Ningún escenario queda sin prueba.

Hay tres reglas del texto de los requisitos, fuera de los escenarios, que tampoco tienen una prueba que muerda: IC4 «haya terminado bien o no» (B4), IC4 «una corrida que no agregó nada MUST NOT reescribir el archivo» (C1) y el archivo por contrato de D7 (C2).

Que `memo.seconds` quede fuera de `index_seconds` (IC8) es estructural, y ninguna prueba lo mide. La memoria se lee antes de `EvalDocIndex::open` (`docs.rs:691`), `started` se toma después (`:767`), y `finish` corre recién después de `took` (`:769-773`).

## La producción

El daemon no cambia de conducta más allá de la línea nueva:

- `RefreshReport` no gana campos. `try_refresh` mapea `.report` (`project_docs/mod.rs:355`). `refresh` escribe lo mismo que antes: el diff sólo agrega `lap` y los `push` de `timing`, y `written` se toma antes de mover `passages`.
- `record_refresh` (`api.rs:3036`) tiene la misma firma y las mismas series, y recibe `refreshed.as_ref().ok().map(|run| &run.report)`. La línea `docs refresh of …` sale sólo en la rama `Ok`, después de `refresh_line` (`api.rs:3025`), sin rutas.
- `embedder::session_threads` devuelve el mismo `EmbedError::LoadFailed` que `session_builder` armaba en línea. `longest_first` pasa a `pub(crate)` y `timing::seconds` a `pub`. Ninguna otra visibilidad de producción cambia.
- El clippy de lo que publica el release sale limpio. `vector_memo` y todo lo nuevo de `eval_api` compilan sólo con `eval`.

## El diseño y los desvíos declarados

D1 a D14 se siguen. Sobre los nueve desvíos que anotó la aplicación:

1. **`eval_api::session_threads() -> Result<usize, EvalError>`** (`eval_api.rs:670`). No es un desvío: D5 dice eso mismo. La que devuelve `EmbedError` es `embedder::session_threads`, `pub(crate)`. ✅
2. **`EvalMemoReport.file` es sólo el nombre, y `warnings` incluye el aviso de lectura.** `docs.rs` imprime el de lectura antes de indexar (`docs.rs:692`) y después sólo los que no son ese (`:774`). La carpeta es siempre `docs/eval/cache/`, así que el nombre basta para «nombrar el archivo» de IC8. ✅ (ver SUGGESTION 3 para el caso sin hilos)
3. **`IndexBlock.memo` es un `MemoBlock { used, from_memo, embedded, seconds }`** (`index_cost.rs:58`). Es exactamente la forma de IC8, y es mejor que un `Option<EvalMemoReport>`, porque el bloque no puede llevar el nombre del archivo ni un aviso. `the_index_block_holds_no_text_and_no_path` busca `memo.bin` y `warning`. ✅
4. **`passage_lengths()` es `async` y devuelve `Result<Option<…>, EvalError>`** (`eval_api.rs:1447`). Leer el store puede fallar, y el `None` sin tokenizer sigue como D11. Un fallo corta la corrida con `EXIT_RUNTIME_FAILURE` (`docs.rs:727-730`). ✅
5. **`render` recibe `Option<RealRun { block, memo }>`** (`docs.rs:238`). Es lo que pide D13: las fases siempre, y el bloque sólo en una corrida real. ✅
6. **Los tokens por área son `Option`, y el `write` por área no incluye `remove`.** Lo primero es coherente con `IndexBlock.tokens: Option` de la tarea 7. Lo segundo rompe la suma que IC7 pide (WARNING 2). ⚠️
7. **`production_never_names_the_archive` pasa sin código nuevo.** Es una guarda: D12 dice que hoy no aparece. IC7 tiene otras tres pruebas que fallan sin el código, así que la regla de `openspec/config.yaml` («each requirement maps to at least one test that fails before the change») se cumple. ✅
8. **Las pruebas de tiempos afirman `scan > 0`.** `scan` incluye un `spawn_blocking` y el recorrido del árbol, y `Instant` tiene 100 ns de resolución en Windows. Pasó en las cuatro corridas. ✅
9. **El fixture `secret/area/guide.md` pasó a `secret/folder/guide.md`** (`docs.rs:1018`), porque la aguja `area` coincidía con la clave `by_area`. La prueba no pierde fuerza: `folder` no es el nombre de ningún campo. ✅

## `.claude/CLAUDE.md` y las reglas

- **Nada prohibido en el rango.** No hay `#[allow]`, código comentado ni `TODO`. Los cuatro comentarios `//` nuevos explican un porqué (`docs.rs:690`, `:770-771`, `:787-788`). Ningún nombre de prueba, comentario, mensaje de error o commit nombra un plan, un grupo, una tarea, el cambio ni los números IC o D. Los nueve commits son convencionales y no tienen atribución.
- **Errores.** `MemoUnreadable` usa `thiserror` (`vector_memo.rs:88`). Lo que cruza a `rag-eval` es `EvalError`.
- **Puro e I/O.** `memo_key`, `contract_digest`, `decode`, `split_hits`, `merge`, `padded*`, `Tokens::new`, `area_of`, `ByArea::of`, `IndexBlock::new`, `memo_dir` y `refresh_timing_line` son puras. El I/O vive en `load_memo`, `replace_file`, `EvalMemo::open` y `finish`, y `encode` escribe sobre cualquier `Write`.
- **Sin dependencias nuevas.** No cambia ningún `Cargo.toml` ni `Cargo.lock`. Lo nuevo de visibilidad tiene uso: los tipos de `RefreshRun` los lee el daemon, y lo `pub` de `eval_api` lo lee `rag-eval` o `e5_golden`.
- **Temporales.** `clawg-core` usa `crate::test_dir::test_dir()`, `rag-eval` usa `TempPath`, y `docs_mode.rs` usa `tempfile::tempdir()` como el resto de ese archivo. Ninguna prueba apunta la memoria a `docs/eval/cache` real.
- **Sin ruta ni texto en el bloque `index`**: sólo números, booleanos, `debug`/`release`, `os` y `arch`.

## Lo que encontró la verificación

**CRITICAL:** ninguno.

**WARNING:**

1. **El refresco no tiene prueba de que anote la entrada del doc que no se leyó ni la del que quedó como estaba** (`crates/clawg-core/src/memory/project_docs/mod.rs:524`, `:547`). Si se borran los dos `push`, no falla ninguna prueba (A5, A6), y el escenario de IC1 «un doc que no se leyó» queda sostenido sólo por la suma pura de `a_doc_that_was_not_read_still_counts_its_read_time` (`:1136`). **Arreglo:** una prueba que llame `refresh` con el `Unusable` de `:1242` y afirme que `a.md` está en `timing.docs` con `passages = 0` y `embed > 0`. La rama del doc que desaparece entre el recorrido y la lectura no se puede provocar sin una costura. Se puede aceptar así, o probar con la misma forma de entrada si el `push` pasa a una función pura.
   **Cerrado en `5f44ffc` (parcial):** `a_refresh_times_a_doc_whose_vector_is_unusable` mata la mutación A6. A5 sigue sin prueba: el doc que no se lee entre el recorrido y la lectura necesita una costura que no existe y no se agregó una.
2. **Las áreas no suman `index.phases.write`: el `write` por área deja afuera `remove`** (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:242-281`, frente a `Phases::of`, `:33`). IC7 dice que la suma de las áreas «MUST dar los totales de `index.phases` (menos `scan`)», e IC1 cuenta `remove_docs` dentro de `write`. `areas_add_up_to_the_totals` (`:477`) lo esconde con `remove = 0` (`run_of`, `:440`). En `rag-eval` el store es nuevo y `remove` es casi cero, así que el error es chico, pero el invariante que afirma la spec es falso. **Arreglo:** al archivar, que IC7 diga «menos `scan` y la remoción», y que la prueba use un `remove` mayor que cero y afirme `Σ write + remove == phases.write`. Otra opción es llevar `remove` aparte en `index.phases`.
   **Cerrado en `891e28d`:** `the_removal_of_docs_is_in_the_total_write_and_in_no_area` afirma `Σ write por área + remove == phases.write` con `remove > 0` (mata quitar `remove` de `Phases::of`). Queda para el archivado la redacción de IC7: «menos `scan` y la remoción».
3. **IC4 «haya terminado bien o no» no tiene prueba** (`crates/clawg-eval/src/bin/rag-eval/docs.rs:772`). Con `finish` sólo en una corrida que terminó bien, la suite pasa entera (B4). `rag-eval` sólo puede construir `EvalEmbedder::fake()`, que nunca falla, así que la prueba natural va en `vector_memo`. **Arreglo:** indexar por `memo.embedder()` con un `Recording::faulty(Fault::NotFinite)` y un doc con `BROKEN`, de modo que `index` falle, llamar `finish` y comprobar que el archivo tiene los vectores del doc sano. El cableado de `docs.rs` (el `finish` antes del `match`) seguiría sin prueba, salvo que esa decisión pase a una función que `measure_with` llame.
   **Cerrado en `b0623f8` (parcial):** `what_a_run_embedded_before_the_index_failed_is_kept` prueba que `finish` conserva lo de un índice que falló. El cableado de `docs.rs` (B4) sigue sin prueba: `rag-eval` sólo puede construir `EvalEmbedder::fake()`, que nunca hace fallar el índice.
4. **IC4 «una corrida que no agregó nada MUST NOT reescribir el archivo» no tiene prueba que muerda** (`crates/clawg-core/src/eval_api/vector_memo.rs:461`). Con el `fresh.is_empty()` quitado, la suite pasa (C1). `a_full_memo_embeds_nothing_and_stores_the_same_vector_bytes` (`:904`), `questions_never_go_through_the_memo` (`:986`) y `a_real_docs_run_keeps_its_vectors_in_the_cache_folder` (`docs.rs:1249`) comparan bytes, y la codificación es determinista, así que una reescritura da los mismos bytes. **Arreglo:** afirmar que el `modified()` del archivo no cambia, o dejarlo de solo lectura y afirmar que `finish` no avisa.
   **Cerrado en `b0623f8`:** `a_run_that_adds_nothing_does_not_touch_the_file` mata la mutación C1 mirando el `modified()` tras fijarlo en una fecha vieja.
5. **«Un archivo por contrato» (D7 a) no tiene prueba** (`vector_memo.rs:107-109`). Con un nombre fijo para todos los contratos, la suite pasa (C2). La conducta que se perdería sin que nadie lo vea: una corrida de otro contrato lee el archivo, lo toma por ilegible (`OtherContract`) y lo reemplaza, así que alternar dos contratos (el experimento del prefijo, por ejemplo) vacía la memoria cada vez. **Arreglo:** al final de `a_memo_made_under_another_contract_is_not_used` (`:1026`), volver a abrir con el primer contrato y afirmar `from_memo = 3`, o afirmar que la carpeta tiene un archivo por contrato.
   **Cerrado en `b0623f8`:** `a_memo_made_under_another_contract_is_not_used` mata la mutación C2: un archivo por contrato y el primero sigue entero.
6. **El escenario «ventana de 512» de IC6 no tiene prueba del conteo** (`crates/clawg-core/src/eval_api.rs:1399`). Con `encode_batch` sin ventana en lugar de `window_encodings`, la suite pasa (C5): todos los textos de las pruebas son cortos. **Arreglo:** un caso de `lengths_are_counted_in_batches_of_the_embedder` (`:2906`) con un texto de 700 palabras sobre `word_tokenizer`, que tiene que contar `EMBED_MAX_TOKENS`.
   **Cerrado en `891e28d`:** `a_passage_longer_than_the_window_counts_the_window` mata la mutación C5.

**SUGGESTION:**

1. **Cadenas con saltos de línea crudos en el código nuevo.** Están en `project_docs/mod.rs:1072-1081`, `:1101-1107` y `:1122-1124`, en `eval_api.rs:2827-2837`, en `docs.rs:1199-1204` y en el `format!` de producción de `AreaBlock::line` (`index_cost.rs:216-217`). El SQL de `vector_blobs` (`project_docs/mod.rs:1054`) tiene corridas de espacios donde iba una continuación `\`. Funcionan igual, porque Rust normaliza CRLF en los literales y el repo usa `core.autocrlf=true`. Pero el resto de esos archivos usa `\n` y `\` + salto, y estas parecen un `\n` que una herramienta convirtió. Conviene pasarlas a escapes.
2. **El primer par de `the_memo_key_separates_neighbouring_components`** (`vector_memo.rs:696-700`, `model_id` frente a `query_prefix`) no es vecino en el hash, porque `dimensions` va en medio. Sin los largos (A2) ese par sigue distinto, y sólo falla el segundo (`passage_prefix` frente a `model_sha256`, `:706`). Cambiar el primero por `query_prefix` frente a `passage_prefix` haría que las dos aserciones prueben lo que dicen.
3. **Una memoria sin hilos imprime un nombre vacío.** Si `session_threads` falla, `EvalMemoReport.file` es `""` (`vector_memo.rs:470`) y el informe dice `vector memo : 0 from the memo, …` (`index_cost.rs:79-81`). Una línea propia («not used: …») sería más clara. Es un caso que en la práctica no ocurre.
4. **Que `EvalEmbedder::real` conserve su recorder** (`eval_api.rs:580-583`) no tiene prueba (C6), como admite IC6: sólo se puede ver con el modelo. Lo cierra la puerta `tokens.padded == tokens.padded_ran` de las corridas D y R: con el recorder perdido, `padded_ran` sería 0 y `padding_warning` avisaría.
5. **`production_never_names_the_archive` lee sólo `project_docs/`** (`project_docs/mod.rs:812-827`), como dice D12, y no `crates/clawg-core/src`, como dice IC7. Hoy ninguna fuente de `clawg-core` ni de `clawg-cli` contiene `archive/`. Al archivar se puede alinear el texto de IC7 con D12.
6. **`a_vector_the_embedder_returns_unusable_is_not_stored`** (`vector_memo.rs:990`) mira la salida del decorador y no el error del índice con y sin memoria. Con la memoria vacía la salida es idéntica a la del embedder, así que el riesgo es bajo. Comparar el `Err` de `EvalDocIndex::index` en los dos casos sostendría «falla igual que sin memoria» tal como lo escribe IC4.
7. **`an_unchanged_refresh_has_no_doc_timings`** (`project_docs/mod.rs:1120`) no mira `remove`, que el escenario describe como «casi cero, no cero». Afirmarlo arriesga una prueba frágil, así que se puede dejar como está.

## Resultado de la suite y de clippy

- `cargo test --workspace --no-fail-fast --target-dir target-test`: pasa (salida 0) sobre `15ce920`, a la primera.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: salida 0.
- `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`: salida 0.
- Ninguna métrica de `rag-eval` cambia en este rango: sin el modelo no hay corrida que medir, y las corridas D y R (abajo) dan los números.

## Veredicto

**Actualización tras el cierre de huecos (`5f44ffc`, `b0623f8`, `891e28d`):** muerden A6, C1, C2 y C5, y W2 queda cubierto (la redacción de IC7 se corrige al archivar). Siguen abiertos sólo A5 (carrera sin costura) y B4 en `docs.rs` (`rag-eval` no puede construir un embedder que falle); ninguno es un defecto del código. El veredicto sigue en PASS WITH WARNINGS por esos dos.

**PASS WITH WARNINGS.** La suite y los dos clippy pasan en Windows. Los ocho requisitos tienen prueba, 41 de 44 escenarios cumplen enteros y ninguno queda sin prueba. La producción sólo gana la línea del log. De las 16 mutaciones, nueve muerden. Muerden en todo lo de más riesgo: la clave, el decorador, `--no-cache`, `padded_ran`, el área, los valores no finitos y la remoción dentro de `write`. Siete no muerden. Una se esperaba, porque necesita el modelo (C6). Las otras seis son los WARNING 1 (A5 y A6), 3, 4, 5 y 6: la entrada de un doc fallado en los tiempos, `finish` en una corrida que falla, no reescribir sin novedades, un archivo por contrato y la ventana de 512. El WARNING 2 es un invariante de IC7 que el código no cumple con `remove > 0`. Ninguno cambia un número que lean las puertas de `docs-index-speed`, salvo el WARNING 2, por la remoción, que en `rag-eval` es casi cero.

## Corridas reales

**Pendiente.** Faltan las corridas D (build debug, memoria vacía) y R (build release en `target-test`, `--no-cache`, con `CLAWG_PROFILE=dev`) del grupo 10 de [tasks.md](tasks.md). Necesitan el modelo real del perfil dev y una autorización explícita, y en esta verificación no se corrió ninguna. Las completa otro paso con:

- de D y de R: `index_seconds`, `index.build`, `index.machine`, `memo.*`, `phases.*`, `tokens.*` y `by_area.*`;
- los cocientes que leen las puertas de `docs-index-speed`;
- las puertas: la suma de `index.phases` dentro del 5 % de `index_seconds`, `tokens.padded == tokens.padded_ran`, `memo.embedded == corpus.passages` en D, `memo.used = false` en R y el `--diff` de D contra R sin preguntas movidas.
