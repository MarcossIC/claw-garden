# revision-eval — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite y el clippy de `.claude/CLAUDE.md` (`-D warnings`). Un módulo del binario entra en el mismo commit que su primer llamador fuera de los tests, para no dejar `dead_code`. Antes del primer test: `cargo clean --workspace --target-dir target-test`. Las tareas marcadas **[modelo real]** o **[prod]** nunca forman parte de la suite ni de CI; se confirman con el usuario justo antes de correrlas.

## 1. `eval_api` (E5)

- [x] Tests `save_stores_under_the_agent_project_and_key_it_is_given`, `a_second_save_under_the_same_key_reports_the_first_as_replaced`, `save_reports_the_candidates_the_detector_paired`, `save_with_an_invalid_key_fails` (`eval_api.rs`, embedder falso): fallan porque `save` no existe.
- [x] `EvalSave`, `EvalSaved`, `EvalCandidate` y `EvalIngest::save` junto a `send` (`:525`), con el armado del `MemoryEvent` en una función privada compartida y `validate_topic_key` antes de encolar; el informe es el `IngestReport` (`worker.rs:81`). `send` no cambia.
- Commit: `feat(core): save a memory with its project and topic key from the eval api`
- [x] Tests `joined_pieces_equal_the_text_read_memory_returns`, `the_band_constants_are_the_detectors` (como `EMBED_MAX_TOKENS`) y uno de `row_distances` contra el KNN.
- [x] `row_distances` (`vec_distance_l2` sobre `memory_vectors`), `join_pieces` e `is_claim_type` (delegan en `pieces.rs:61` y `is_claim_bearing`, `store_insert.rs:59`), y re-export de `NEAR_DUPLICATE_FLOOR` y `CONTRADICTION_THRESHOLD`. Son `pub` de un módulo detrás de `eval`: no hay `dead_code`.
- Commit: `feat(core): expose piece joining, row distances and the detector band to the eval api`

## 2. Esqueleto, `set` y el set de cadenas (E1)

- [ ] Tests `the_shipped_chain_set_meets_every_minimum`, `a_set_below_a_minimum_names_what_is_missing`, `a_query_that_holds_a_versions_value_is_refused`, `a_chain_with_an_invalid_key_or_unknown_field_fails` (`set.rs`): fallan porque ni el módulo ni el set existen.
- [ ] `[[bin]] revision-eval` en `crates/clawg-eval/Cargo.toml`, `src/bin/revision-eval/{main,set}.rs` (`deny_unknown_fields`, mínimos como `Class::minimum`, `validate_topic_key`) y `main.rs` que carga el set e imprime su `version` y sha256.
- [ ] `crates/clawg-eval/data/revision-chains.json`: 40 cadenas escritas para medir (20 `es` y 20 `en`; 24 de 2 versiones y 16 de 3; 20 decisiones, 10 preferencias, 10 hechos; al menos 8 con versiones de más de 480 tokens; `value` por versión y `query` sin ningún `value`; cortas y largas, sin datos de usuario). Es trabajo real, no mecánico: la prueba del set entregado es su compuerta y debe pasar los mínimos de E1.
- Commit: `feat(eval): add the revision-eval binary and its chain set`

## 3. `chains`: brazos y M1 (E2, E5)

- [ ] Tests puros `every_transition_falls_in_exactly_one_class`, `a_far_pair_is_no_mark_above_the_threshold`, `a_close_pair_of_decisions_is_escalated`, `a_row_beyond_the_candidate_pool_is_out_of_pool`, `a_type_that_asserts_nothing_is_not_claim_bearing`, `the_control_arm_stores_only_the_last_version` (`chains.rs`) y `a_rate_carries_its_wilson_interval` (`stats.rs`).
- [ ] `stats.rs` (`wilson`, `Rate` copiados de `conflict-eval/measure.rs:107-135` con sus casos fijados), `chains.rs` (clase y razón, D3, cortes por `change`, `lang` y tipo), `run.rs` (brazos por `EvalIngest::save`, distractores de `openspec/specs`, stores en el temporal, fila viva por el router) y `chains` en `main.rs` con el reporte de M1 y `--fake`.
- [ ] Test `the_router_lists_a_revised_row_superseded_by_the_new_one` y corrida `a_fake_chains_run_stores_every_arm_and_reports_whole` (con limpieza, como `runner.rs:353`).
- Commit: `feat(eval): classify each version transition of a chain without a topic key`

## 4. `chains`: posiciones y M2 (E3)

- [ ] Tests `m2_passes_when_all_three_conditions_hold`, `m2_fails_when_an_old_version_shows`, `m2_fails_when_the_last_drops_two_places`, `m2_needs_thirty_eight_matching_chains`, `an_old_version_above_the_new_one_is_recorded`.
- [ ] Posiciones por versión desde `format_search` y las cabeceras `[fecha #id` del prompt, sesión del router copiada de `agent_view/runner.rs:82-115`, veredicto y lista de cadenas que rompen; entra en el reporte de `run.rs`.
- Commit: `feat(eval): check that a keyed revision hides its old versions`

## 5. `chains`: M4 (E4, E6)

- [ ] Tests `the_rewrite_adds_the_key_only_to_keyed_hits`, `the_byte_delta_is_the_sum_of_the_added_keys`, `a_prompt_over_budget_after_the_rewrite_is_counted`, `m4_reports_the_fixed_cost_of_the_key_from_the_contract_and_the_protocol`, `the_production_context_still_omits_the_key` (este pasa hoy), `the_rewrite_finds_every_keyed_header_of_a_real_prompt`.
- [ ] `cost.rs` (reescritura de D4, cota de la clave más larga, costo fijo desde `agent_contract.json` y `protocol.md`, línea de aviso constante), usado por `run.rs` con `EvalTokenCounter`.
- Commit: `feat(eval): measure what showing a topic key costs in tokens`

## 6. Resultado y verificación de `chains` (E9, D6, D7)

- [ ] Test `a_result_header_names_the_model_contract_and_the_set_hash`; `main.rs` escribe `docs/eval/results/{fecha}-real-revision-eval-chains[-{tag}].json` sin pisar uno existente (`--fake` sólo imprime).
- Commit: `feat(eval): write the dated result of the chains run`
- [ ] **[modelo real]** `rag-eval --real --agent-view` y `--real` con `--diff` contra `2026-10-20-real-untrusted-recall-fix-agent-view.json` y `2026-10-18-real-search-abstention.json`: ningún caso movido, y las filas `search.tokens` y `context.tokens` del diff (`crates/clawg-eval/src/bin/rag-eval/diff.rs:120-121`) con cambio medio 0. Perfil dev, golden de `docs/eval/`.
- [ ] **[modelo real]** `revision-eval chains --real`; el resultado se versiona en `docs/eval/results/`.
- Commit: `docs(eval): record the revision chains measurement`

## 7. `store` puro (E7, E8)

- [ ] Tests `the_counts_that_need_no_replay_come_from_the_parsed_export`, `the_nearest_earlier_save_is_looked_up_within_agent_and_project`, `a_candidate_of_another_project_does_not_fire`, `every_firing_save_is_labelled_plus_a_seeded_sample`, `the_same_seed_picks_the_same_sample`, `the_labels_file_does_not_say_which_pair_fired`, `a_labels_file_with_an_empty_label_is_refused`, `the_score_reads_only_the_owners_label_and_reports_the_agreement`, `an_unknown_export_format_writes_nothing`; exports escritos en el test (como `EXAMPLE`, `format.rs:289`).
- [ ] `store.rs` (saves desde `format::parse`, disparo, cota de prevalencia, `labels.jsonl` con `proposed` y `label`), `SplitMix64` copiado de `rag-eval/stats.rs:94`, y los dos pasos en `main.rs` y `run.rs` (`--export … --fake`, `--labels`), directorio de trabajo bajo `temp_dir()` borrado también ante un error.
- [ ] Tests `a_fake_store_run_counts_and_labels_a_small_export`, `the_results_file_of_store_holds_no_stored_text`, `the_working_files_are_removed_even_when_the_run_fails`.
- Commit: `feat(eval): replay an exported store and score its revisions`

## 8. `store --real` (M5)

- [ ] **[prod]** Confirmar con el usuario; `clawg export --include-deleted --out <temp>/revision-eval-X/export.jsonl` con el `clawg` instalado. Sin tocar `~/.clawg*` más que esa lectura.
- [ ] **[modelo real]** `revision-eval store --real --export …`; el agente de costo bajo (Sonnet o Haiku) llena `proposed` en una sesión que autoriza el dueño del store; el dueño decide cada `label`.
- [ ] `revision-eval store --labels …`: borra el directorio de trabajo; el resultado, sólo conteos, tasas y Wilson, va a `docs/eval/results/`; comprobar que el directorio temporal no existe.
- Commit: `docs(eval): record the store revision measurement`

## 9. Verificación y docs

- [ ] `.claude/CLAUDE.md`: `revision-eval` en la fila `clawg-eval` de Workspace y, si hace falta, una frase en Testing como las de los otros binarios.
- Commit: `docs: describe the revision-eval binary`
- [ ] `cargo test --workspace --target-dir target-test` y el clippy de `.claude/CLAUDE.md`; `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`; sin otro directorio de build que `target` y `target-test`.
- [ ] `verify.md`: la suite y los clippy pasan (sin contar tests); aviso «pasá `topic_key`» (M5, M4) y mapeo de history-import (M1, M2) contra los umbrales, citando los campos del resultado; búsqueda por clave, clave en `recall_context` y quitar reglas quedan pendientes de M3, con lo medido por M1, M2 y M4.
- Commit: `docs(openspec): record the verification of revision-eval`

## 10. Archive y versión

- [ ] Sección «`revision-eval`: revisiones por clave» en `specs/eval`; `git mv` al archivo con la fecha; `state.yaml` en `archived` y la fila 48 de `openspec/README.md` como archivada.
- Commit: `docs(openspec): archive revision-eval`
- [ ] Subir el minor `0.41.0` → `0.42.0` en `[workspace.package]` del `Cargo.toml` raíz y `cargo update --workspace`; ambos archivos juntos.
- Commit: `chore: bump version to 0.42.0`
