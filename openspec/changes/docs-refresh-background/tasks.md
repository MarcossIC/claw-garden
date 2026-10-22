# docs-refresh-background — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Un doc malo no frena a los demás (R3, D3)

- [x] Tests primero: `an_unusable_vector_skips_its_doc_and_the_rest_go_on` y `a_short_batch_still_aborts_the_refresh` (`project_docs/mod.rs`); `a_doc_the_index_could_not_embed_fails_the_measurement` (`eval_api.rs`).
- [x] `RefreshReport::failed` y `FailedDoc`; `refresh` saltea un `DocIndexError::Vector` y sigue; `DocIndexError::VectorCount`; `EvalDocIndex::index` falla con un doc en `failed`; el daemon deja cada doc salteado en el log.
- Commit: `fix(docs): skip a doc whose vector is unusable and index the rest`

## 2. El error del último refresco llega al modelo (R2, D2)

- [x] Tests primero: `the_last_error_lasts_until_a_refresh_succeeds` (`project_docs/mod.rs`); `a_background_refresh_that_failed_warns_the_next_search` (`tests/api.rs`).
- [x] `RefreshGuard::run`, `Progress::last_error`, `DocIndex::last_error`; `search_docs` toma `warning` de ahí.
- Commit: `feat(daemon): name the last failed docs refresh in every docs search`

## 3. La búsqueda no espera el refresco más que un plazo (R1, D1)

- [x] Tests primero: `begin_if_due_claims_only_a_due_idle_project` (`project_docs/mod.rs`); `a_search_answers_from_the_index_while_a_long_refresh_runs`, `a_small_edit_is_found_by_the_search_that_refreshes` y `the_refresh_outlives_the_search_that_started_it` (`tests/api.rs`).
- [x] `DocIndex::begin_if_due`; `refresh_docs_in_background` y `refresh_docs`, extraídas de `spawn_doc_refresh`; `REFRESH_GRACE`; `search_docs` lanza el refresco vencido y espera a lo sumo el plazo.
- Commit: `fix(daemon): refresh due docs in the background and wait for them at most 3 s`

## 4. Docs

- [x] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say a docs search waits a refresh for at most 3 s`

## 5. Verificación

- [ ] Suite y clippy en Linux sobre el último commit, y cada commit de código por separado ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 6. Archive

- [ ] `specs/project-docs` con R1, R2 y R3, las citas realineadas; `git mv` a `changes/archive/AAAA-MM-DD-docs-refresh-background`; `state.yaml` archivado; fila 64 y el orden de ejecución del `README.md` de openspec; subir el minor.
