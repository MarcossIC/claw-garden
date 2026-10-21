# revision-eval — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite y el clippy de `.claude/CLAUDE.md` (`-D warnings`). Un módulo del binario entra en el mismo commit que su primer llamador fuera de los tests, para no dejar `dead_code`. Antes del primer test: `cargo clean --workspace --target-dir target-test`. Las tareas marcadas **[modelo real]** o **[prod]** nunca forman parte de la suite ni de CI; se confirman con el usuario justo antes de correrlas.

## Dónde quedó (2026-10-21)

Hechos los grupos 1 a 7 y 9.1, la compuerta de `rag-eval` y `chains --real`. Falta M5 (grupo 8), y con él `verify.md` (9), el archivo y la versión (10).

**Bloqueo de M5.** `clawg export` existe en el código desde antes de este cambio, pero ninguna release lo trae: la última publicada es `0.34.0` (`clawg update --check` la da por actual) y no tiene ese subcomando. Una build de debug rechaza el perfil `prod` por diseño, así que no sirve para leerlo. Para seguir hace falta una release con `export` (la `0.43.0` que sale de llevar `develop` a `main`) instalada con `clawg update`; recién ahí se exporta el store real ya cargado y corre el grupo 8.

**Cómo retomar**, con la release instalada:

1. Crear un directorio `revision-eval-<algo>` directamente bajo el temporal del sistema (`work_dir` rechaza cualquier otro lugar) y exportar ahí: `clawg export --profile prod --include-deleted --out <temp>/revision-eval-<algo>/export.jsonl`.
2. `cargo run -p clawg-eval --bin revision-eval --target-dir target-test -- store --real --export <temp>/revision-eval-<algo>/export.jsonl`, con el modelo del perfil dev.
3. Un sub-agente Haiku (lo eligió el dueño del store, 2026-10-20) llena `proposed` en `labels.jsonl`; el dueño decide cada `label`.
4. `revision-eval store --labels <temp>/revision-eval-<algo>/labels.jsonl` y comprobar que el directorio ya no existe.
5. Grupos 9 y 10.

Fijado antes de medir: la línea de aviso de D4 y que la población de «los sin clave» de M5 son los saves sin clave de un tipo que afirma (D5).

## 1. `eval_api` (E5)

- [x] Tests `save_stores_under_the_agent_project_and_key_it_is_given`, `a_second_save_under_the_same_key_reports_the_first_as_replaced`, `save_reports_the_candidates_the_detector_paired`, `save_with_an_invalid_key_fails` (`eval_api.rs`, embedder falso): fallan porque `save` no existe.
- [x] `EvalSave`, `EvalSaved`, `EvalCandidate` y `EvalIngest::save` junto a `send` (`:525`), con el armado del `MemoryEvent` en una función privada compartida y `validate_topic_key` antes de encolar; el informe es el `IngestReport` (`worker.rs:81`). `send` no cambia.
- Commit: `feat(core): save a memory with its project and topic key from the eval api`
- [x] Tests `joined_pieces_equal_the_text_read_memory_returns`, `the_band_constants_are_the_detectors` (como `EMBED_MAX_TOKENS`) y uno de `row_distances` contra el KNN.
- [x] `row_distances` (`vec_distance_l2` sobre `memory_vectors`), `join_pieces` e `is_claim_type` (delegan en `pieces.rs:61` y `is_claim_bearing`, `store_insert.rs:59`), y re-export de `NEAR_DUPLICATE_FLOOR` y `CONTRADICTION_THRESHOLD`. Son `pub` de un módulo detrás de `eval`: no hay `dead_code`.
- Commit: `feat(core): expose piece joining, row distances and the detector band to the eval api`

## 2. Esqueleto, `set` y el set de cadenas (E1)

- [x] Tests `the_shipped_chain_set_meets_every_minimum`, `a_set_below_a_minimum_names_what_is_missing`, `a_query_that_holds_a_versions_value_is_refused`, `a_chain_with_an_invalid_key_or_unknown_field_fails` (`set.rs`): fallan porque ni el módulo ni el set existen.
- [x] `[[bin]] revision-eval` en `crates/clawg-eval/Cargo.toml`, `src/bin/revision-eval/{main,set}.rs` (`deny_unknown_fields`, mínimos como `Class::minimum`, `validate_topic_key`) y `main.rs` que carga el set e imprime su `version` y sha256. El sha256 se toma con finales de línea LF (`the_set_hash_is_the_same_with_either_line_ending`, `e311cea`): el set está en git y un checkout de Windows con `core.autocrlf` lo trae en CRLF.
- [x] `crates/clawg-eval/data/revision-chains.json`: 40 cadenas escritas para medir (20 `es` y 20 `en`; 24 de 2 versiones y 16 de 3; 20 decisiones, 10 preferencias, 10 hechos; al menos 8 con versiones de más de 480 tokens; `value` por versión y `query` sin ningún `value`; cortas y largas, sin datos de usuario). Es trabajo real, no mecánico: la prueba del set entregado es su compuerta y debe pasar los mínimos de E1.
- Commit: `feat(eval): add the revision-eval binary and its chain set`

## 3. `chains`: brazos y M1 (E2, E5)

- [x] Tests puros `every_transition_falls_in_exactly_one_class`, `a_far_pair_is_no_mark_above_the_threshold`, `a_close_pair_of_decisions_is_escalated`, `a_row_beyond_the_candidate_pool_is_out_of_pool`, `a_type_that_asserts_nothing_is_not_claim_bearing`, `the_control_arm_stores_only_the_last_version` (`chains.rs`) y `a_rate_carries_its_wilson_interval` (`stats.rs`). Además: el orden por rondas de los brazos, el par sólo entre las dos versiones, y las filas compartidas de D3 (`a_row_two_versions_share_is_the_newer_ones`, `the_rows_a_transition_compares_leave_out_the_ones_both_hold`, `an_older_version_the_newer_holds_whole_is_below_the_floor`).
- [x] `stats.rs` (`wilson`, `Rate` copiados de `conflict-eval/measure.rs:107-135` con sus casos fijados), `chains.rs` (clase y razón, D3, cortes por `change`, `lang` y tipo), `run.rs` (brazos por `EvalIngest::save`, distractores de `openspec/specs`, stores en el temporal, fila viva por el router) y `chains` en `main.rs` con el reporte de M1 y `--fake`.
- [x] Test `the_router_lists_a_revised_row_superseded_by_the_new_one` y corrida `a_fake_chains_run_stores_every_arm_and_reports_whole` (con limpieza, como `runner.rs:353`). `chains --fake` sobre el set entregado: 40 cadenas, 56 transiciones, 683 distractores, una versión viva por cadena en el brazo con clave y en el control.
- Commit: `feat(eval): classify each version transition of a chain without a topic key`

## 4. `chains`: posiciones y M2 (E3)

- [x] Tests `m2_passes_when_all_three_conditions_hold`, `m2_fails_when_an_old_version_shows`, `m2_fails_when_the_last_drops_two_places`, `m2_needs_thirty_eight_matching_chains`, `an_old_version_above_the_new_one_is_recorded`. Además, la lectura de ids de las dos salidas y la posición por fila propia (`the_ids_are_read_in_the_order_each_output_prints_them`, `a_version_is_placed_by_the_first_hit_of_a_row_of_its_own`).
- [x] Posiciones por versión desde `format_search` y las cabeceras `[fecha #id` del prompt, sesión del router copiada de `agent_view/runner.rs:82-115` (ya en el grupo 3), veredicto y lista de cadenas que rompen; entra en el reporte de `run.rs`, con las posiciones por brazo y cadena. La corrida `--fake` del set chico lee la última versión en las dos salidas por un identificador de la consulta; la del set entregado sale entera (su M2 sobre el doble prueba la tubería, no la clave).
- Commit: `feat(eval): check that a keyed revision hides its old versions`

## 5. `chains`: M4 (E4, E6)

- [x] Tests `the_rewrite_adds_the_key_only_to_keyed_hits`, `the_byte_delta_is_the_sum_of_the_added_keys`, `a_prompt_over_budget_after_the_rewrite_is_counted`, `m4_reports_the_fixed_cost_of_the_key_from_the_contract_and_the_protocol`, `the_production_context_still_omits_the_key` (este pasa hoy), `the_rewrite_finds_every_keyed_header_of_a_real_prompt`. Además, `the_key_of_a_search_hit_shows_for_the_save_project_of_its_row` (formatea con el proyecto de la fila: con otro el delta sería 0 sin error) y `m4_sums_the_queries_and_prices_the_warning_line`.
- [x] `cost.rs` (reescritura de D4, cota de la clave más larga, costo fijo desde `agent_contract.json` y `protocol.md`, línea de aviso constante), usado por `run.rs` con `EvalTokenCounter`. `header_id` sale de `context_ids` para que la reescritura y la lectura de posiciones reconozcan la misma cabecera. `--fake` sobre el set entregado: búsqueda +16 tokens (442 bytes) sobre 40 consultas, contexto +17 tokens (473 bytes), cota de arriba +91 y +115, ningún prompt sobre 8.000 bytes, costo fijo 50 y 101 tokens y línea de aviso de 24 (todos con el contador falso de palabras).
- Commit: `feat(eval): measure what showing a topic key costs in tokens`

## 6. Resultado y verificación de `chains` (E9, D6, D7)

- [x] Test `a_result_header_names_the_model_contract_and_the_set_hash` (más `the_result_file_is_named_by_date_and_tag` y `a_result_is_never_written_over_an_existing_file`); `main.rs` escribe `docs/eval/results/{fecha}-real-revision-eval-chains[-{tag}].json` sin pisar uno existente (`--fake` sólo imprime). Con `--tag SLUG` para dos corridas del mismo día; la existencia del archivo se comprueba antes de correr.
- Commit: `feat(eval): write the dated result of the chains run`
- [x] **[modelo real]** `rag-eval --real --agent-view` y `--real` con `--diff` contra `2026-10-20-real-untrusted-recall-fix-agent-view.json` y `2026-10-18-real-search-abstention.json`: ningún caso movido, y las filas `search.tokens` y `context.tokens` del diff (`crates/clawg-eval/src/bin/rag-eval/diff.rs:120-121`) con cambio medio 0. Perfil dev, golden de `docs/eval/`. Corrido el 2026-10-21 con `--tag revision-eval` (`2026-10-21-real-revision-eval-agent-view.json` y `2026-10-21-real-revision-eval.json`): 0 casos movidos en recuperación, similitud y vista del agente, y en cada par de idiomas; `search.tokens` y `context.tokens` con cambio medio +0,0000.
- [x] **[modelo real]** `revision-eval chains --real`: `docs/eval/results/2026-10-21-real-revision-eval-chains.json` (`multilingual-e5-small/mean`, set v1 `fcc21149…`, 40 cadenas, 56 transiciones, 684 distractores). `docs/eval/` dejó de versionarse (`ed2ddff`), así que el archivo queda local y sus números se asientan acá. Lectura preliminar; el veredicto contra los umbrales va en `verify.md`:
  - **Brazos:** con clave y control, una versión viva en las 40 cadenas; sin clave, 96 versiones vivas y ninguna cadena con una sola.
  - **M1** (transiciones sin clave): `escalated` 28/56 (50 %, Wilson 37,3–62,7 %), `pending` 14/56 (25 %), `no_mark` 14/56 (25 %, 15,5–37,7 %: 6 `below_floor`, 7 `above_threshold`, 1 `out_of_pool`, 0 `not_claim_bearing`). La vieja queda arriba de la nueva en 35/56 (62,5 %) y `unmarked_or_above` es 40/56 (71,4 %, 58,5–81,6 %).
  - **M2: no pasa.** (a) y (b) se cumplen; (c) no: coinciden con el control 38 de 40, el mínimo, pero en `en-repo-layout` la última versión baja dos puestos en `recall_context` (7 en el control, 9 con clave; no aparece en `search_memory` en ningún brazo). `en-test-names` baja uno (2 → 3), que se admite. Según E3, no se arregla en este cambio.
  - **M4** (tokenizador real): `search_memory` con las claves que hoy muestra, +45,075 tokens de media por consulta (1.803 en 40); con todas las filas con clave, +85,05. `recall_context` reescrito, +54,45 (2.178); con todas las filas con clave, +111,0. Ningún prompt pasa de 8.000 bytes en ninguna medida. Costo fijo: 84 tokens de la descripción en el schema y 152 del párrafo de `protocol.md`. Línea de aviso: 34 tokens.
  - **Contra los umbrales:** history-import no se cumple porque M2 no pasa, aunque su otra mitad sí (71,4 % ≥ 25 %). En «mostrar la clave en `recall_context`» el lado de M4 no se cumple (+111,0 > 60 de media con todas las filas con clave) y M3 sigue pendiente. En el aviso, los 34 tokens quedan bajo el tope de 40 donde aparece; la media por save y las tasas esperan M5.
- Commit: `docs(openspec): record the revision chains measurement` (el resultado no se versiona: `docs/eval/` está en `.gitignore`)

## 7. `store` puro (E7, E8)

- [x] Tests `the_counts_that_need_no_replay_come_from_the_parsed_export`, `the_nearest_earlier_save_is_looked_up_within_agent_and_project`, `a_candidate_of_another_project_does_not_fire`, `every_firing_save_is_labelled_plus_a_seeded_sample`, `the_same_seed_picks_the_same_sample`, `the_labels_file_does_not_say_which_pair_fired`, `a_labels_file_with_an_empty_label_is_refused`, `the_score_reads_only_the_owners_label_and_reports_the_agreement`, `an_unknown_export_format_writes_nothing`; exports escritos en el test (como `EXAMPLE`, `format.rs:289`). Además: `the_pieces_of_a_save_are_joined_and_a_missing_one_cuts_the_text`, `a_save_a_key_replaced_is_no_longer_an_earlier_save`, `the_warning_fires_on_an_unkeyed_claim_with_a_candidate_of_its_own_scope`, `the_prevalence_bound_counts_the_firing_as_a_census_and_the_rest_as_a_sample`, `the_thresholds_read_the_proposals_table`, `a_shuffle_keeps_every_item_and_repeats_for_a_seed` y `the_arguments_of_store_parse`.
- [x] `store.rs` (saves desde `format::parse`, disparo, cota de prevalencia, `labels.jsonl` con `proposed` y `label`), `SplitMix64` copiado de `rag-eval/stats.rs:94` (con `shuffle`), y los dos pasos en `main.rs` y `run.rs` (`--export … --fake|--real`, `--labels`), directorio de trabajo bajo `temp_dir()` borrado también ante un error. No hizo falta nada nuevo en `eval_api`: `format` ya es público.
- [x] Tests `a_fake_store_run_counts_and_labels_a_small_export`, `the_results_file_of_store_holds_no_stored_text`, `the_working_files_are_removed_even_when_the_run_fails`. Además: `a_labels_file_the_owner_has_not_finished_keeps_the_directory` y `a_working_directory_outside_the_temp_dir_is_refused_and_left_alone`.
- Commit: `feat(eval): replay an exported store and score its revisions`

## 8. `store --real` (M5)

Bloqueado hasta que haya una release con `clawg export` instalada (ver «Dónde quedó»). El dueño del store ya autorizó el export, la reproducción y el etiquetado con Haiku (2026-10-20).

- [ ] **[prod]** `clawg export --profile prod --include-deleted --out <temp>/revision-eval-X/export.jsonl` con el `clawg` instalado, en un `revision-eval-*` directamente bajo el temporal. Sin tocar `~/.clawg*` más que esa lectura.
- [ ] **[modelo real]** `revision-eval store --real --export …`; un agente Haiku llena `proposed`; el dueño decide cada `label`.
- [ ] `revision-eval store --labels …`: borra el directorio de trabajo; el resultado, sólo conteos, tasas y Wilson, va a `docs/eval/results/` (local); comprobar que el directorio temporal no existe.
- Commit: `docs(openspec): record the store revision measurement`

## 9. Verificación y docs

- [x] `.claude/CLAUDE.md`: `revision-eval` en la fila `clawg-eval` de Workspace y, si hace falta, una frase en Testing como las de los otros binarios.
- Commit: `docs: describe the revision-eval binary`
- [ ] `cargo test --workspace --target-dir target-test` y el clippy de `.claude/CLAUDE.md`; `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`; sin otro directorio de build que `target` y `target-test`.
- [ ] `verify.md`: la suite y los clippy pasan (sin contar tests); aviso «pasá `topic_key`» (M5, M4) y mapeo de history-import (M1, M2) contra los umbrales, citando los campos del resultado; búsqueda por clave, clave en `recall_context` y quitar reglas quedan pendientes de M3, con lo medido por M1, M2 y M4.
- Commit: `docs(openspec): record the verification of revision-eval`

## 10. Archive y versión

- [ ] Sección «`revision-eval`: revisiones por clave» en `specs/eval`; `git mv` al archivo con la fecha; `state.yaml` en `archived` y la fila 48 de `openspec/README.md` como archivada.
- Commit: `docs(openspec): archive revision-eval`
- [ ] Subir el minor `0.43.0` → `0.44.0` en `[workspace.package]` del `Cargo.toml` raíz y `cargo update --workspace`; ambos archivos juntos (el `0.43.0` lo subió el archivo de preference-topic-scope, `a0746cf`).
- Commit: `chore: bump version to 0.44.0`
