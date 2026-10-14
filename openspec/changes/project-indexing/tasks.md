# project-indexing — Tareas

Cada tarea es un commit atómico: primero el test que falla, después el código. El suite queda sin fallas nuevas en cada commit.

## 1. Limpieza del chunker de código (D7, R1)

- [ ] Test `a_code_block_save_is_chunked_like_any_other` en `worker.rs` (pasa hoy; protege el borrado).
- [ ] Borrar `chunker/code.rs`, `ChunkerKind::Code`, `Chunk.header`, el parámetro `code_chunker` del worker y del runtime, y las cuatro dependencias `tree-sitter*`.
- Commit: `refactor(core): remove the unused code chunker`

## 2. Migración V11 (D2, R11)

- [ ] Fila de V11 en `published_migration_checksums_are_pinned` (falla) y `a_database_from_the_previous_version_opens_with_an_empty_doc_index` en `db.rs`.
- [ ] `V11__project_docs.sql`; se toca `migrations.rs`.
- Commit: `feat(core): add tables for a per-project index of documentation`

## 3. Corpus (D1, R2)

- [ ] Tests `classify_*` (puros), `the_walk_honours_gitignore_and_keeps_hidden_docs`, `oversized_and_binary_files_are_skipped` en `project_docs/corpus.rs`.
- [ ] Dependencia `ignore`; `classify` y `walk`.
- Commit: `feat(core): find a project's documentation on disk`

## 4. Troceo (D3, R3, R4)

- [ ] Tests de R3 y R4 en `project_docs/chunk.rs`.
- [ ] `chunk_markdown` y `chunk_manifest`.
- Commit: `feat(core): split documentation by heading with its breadcrumb`

## 5. Refresco (D4, R5, R6)

- [ ] Tests `plan_*` (puros), `an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `another_model_reindexes_every_doc`, `a_missing_root_deletes_nothing`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_second_refresh_does_not_wait_for_the_first`, `refreshing_never_touches_memories`, `a_secret_in_a_doc_never_reaches_the_index`.
- [ ] `DocIndex` con `refresh`, `plan_refresh`, y el campo `project_docs` en `MemoryRuntime`.
- Commit: `feat(core): keep a project's documentation index in step with its files`

## 6. Búsqueda en el core (D5, R7, R10)

- [ ] Tests `fuse_*` (puros), `search_finds_a_passage_by_meaning`, `search_finds_an_identifier_lexically`, `search_never_crosses_projects`, `indexing_docs_leaves_memory_search_unchanged`.
- [ ] `DocIndex::search` con KNN particionado, FTS5 y `fuse`; `fts_match_expression` pasa a `pub(crate)`.
- Commit: `feat(core): search a project's documentation by meaning and by term`

## 7. HTTP (D4, D6, R7, R8)

- [ ] Tests `docs_search_indexes_on_first_use_and_answers`, `docs_search_of_an_unregistered_project_is_404`, `docs_search_needs_a_token`, `registering_a_root_indexes_its_docs_in_the_background` en `crates/clawg-cli/tests/api.rs`.
- [ ] `POST /v1/projects/docs/search`; el registro lanza el refresco de fondo; `DaemonClient::search_docs`.
- Commit: `feat(daemon): index registered projects and serve their documentation`

## 8. MCP (D6, R9)

- [ ] Tests `search_project_lists_each_hit_under_its_breadcrumb`, `search_project_warns_while_indexing`, `instructions_say_when_to_search_the_project`; los tests de handshake cuentan 9 herramientas.
- [ ] Herramienta `search_project`, su formato y la oración en `INSTRUCTIONS`.
- Commit: `feat(mcp): add search_project for questions about how a project works`

## 9. Medición (D8, R12)

- [ ] Tests de `docs.rs` y `docs_mode_runs_end_to_end_with_the_fake_embedder`.
- [ ] `rag-eval --docs`, las funciones de `eval_api`, y `crates/clawg-eval/golden/project-docs.json` con 10 preguntas.
- Commit: `feat(eval): measure retrieval over a project's documentation`

## 10. Documentación

- [ ] `CLAUDE.md`: `search_project` en las herramientas MCP, `POST /v1/projects/docs/search` en la API; la línea del backlog de `openspec/README.md` sobre `#[allow(dead_code)]` se va.
- Commit: `docs: describe the project documentation index`

## Verify

- [ ] `cargo test --workspace` en Linux, contra la base: sin fallas nuevas, con el conteo justificado (`verify.md`).
- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows (máquina de Marcos).
- [ ] `rag-eval --real --docs` con los umbrales de la propuesta: Hit@3 ≥ 0,70 y MRR ≥ 0,50 (máquina de Marcos: necesita el modelo real).
- [ ] `rag-eval --real --diff` y `--agent-view --diff` contra el baseline de memorias: diff vacío (máquina de Marcos).
- [ ] Prueba manual con el perfil dev en Claude Code: abrir una sesión en este repo, preguntar algo de los docs, ver que el agente usa `search_project` y que editar un doc cambia el resultado (máquina de Marcos).

## Archive

- [ ] `specs/project-docs/spec.md` con R1–R9 y R11; R10 en `specs/retrieval/spec.md`; R12 en `specs/eval/spec.md`.
- [ ] Carpeta movida a `changes/archive/AAAA-MM-DD-project-indexing/` y `archive: done` en `state.yaml`.
- [ ] Fila 12 de `openspec/README.md` con el enlace archivado.
