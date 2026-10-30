# docs-search-quality — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada tarea de código es un commit convencional que compila y pasa la suite (`cargo test --workspace`, o `--target-dir target-test` en Windows) y los dos clippy de `.claude/CLAUDE.md`. Ningún test necesita el modelo real ni un perfil de datos; las mediciones `--real` sí, y se anotan en `verify.md`.

Requisitos en la [spec](spec.md) (DQ1 a DQ9) y decisiones en el [diseño](design.md) (D1 a D5).

## Paso 1: medir bien

### 1. Dejar fuera los docs que citan una pregunta (DQ1, DQ2; D1)

- [x] RED: `a_doc_quotes_a_question_across_lines_and_case`, `quoting_docs_names_only_the_docs_that_quote`, `the_shipped_golden_set_names_docs_this_workspace_has` (ninguno de sus `expected` cita una pregunta), `an_expected_doc_that_quotes_a_question_fails_validation`, `the_report_names_the_docs_it_left_out` (`docs.rs`); `a_doc_left_out_is_neither_indexed_nor_counted` (`project_docs/mod.rs`); `docs_mode_leaves_out_a_doc_that_quotes_a_question` y `docs_mode_refuses_an_expected_doc_that_quotes_a_question` (`tests/docs_mode.rs`).
- [x] GREEN: `refresh` y `scan` con el conjunto `left_out`; `EvalDocIndex::index(root, left_out)`; `quoting_docs`, `check_quotes` y la línea del informe.
- Commit: `feat(eval): leave out of the docs corpus the docs that quote a question`

### 2. El archivo de resultados de docs (DQ3; D2)

- [x] RED: `docs_results_name_their_file_and_refuse_to_overwrite`, `docs_results_carry_ranks_and_no_text` (`docs.rs`).
- [x] GREEN: `DocsResults`, `write_results`; `--real --docs` comprueba el archivo antes de indexar y lo escribe al final; `--fake` sólo imprime.
- Commit: `feat(eval): write a docs run's results file`

### 3. `--diff` de dos corridas de docs (DQ4; D2)

- [x] RED: `a_docs_diff_counts_moves_and_names_the_top_3_changes`, `a_docs_diff_splits_docs_and_code`, `a_docs_diff_warns_when_the_corpus_differs`, `a_docs_file_against_a_memory_file_is_an_error`, `a_docs_hit_at_3_regression_fails_the_run` (`docs.rs`, `main.rs`).
- [x] GREEN: `docs::diff_values`, `render_diff`; `run_diff` elige por `kind`.
- Commit: `feat(eval): diff two docs runs`

### 4. Línea base nueva (necesita el modelo real)

- [ ] `rag-eval --real --docs` sobre el commit anterior a la tarea 1 y sobre la tarea 3: anotar en `verify.md` Hit@1/3/5 y MRR@10 de docs y código, cuántos docs se dejaron fuera y cuáles, y el tiempo de indexado. El primer número no tiene archivo (la tarea 2 no existía); se anota de stdout.

## Golden ampliado (DQ5)

### 5. Preguntas nuevas

- [ ] RED: `the_shipped_golden_set_names_docs_this_workspace_has` pasa a exigir al menos 40 casos, 15 de docs y 15 de código, y que ninguno espere un doc que cite una pregunta.
- [ ] GREEN: las preguntas nuevas, escritas sin las palabras del pasaje que las responde.
- [ ] Línea base del set ampliado con `--real` (necesita el modelo real).
- Commit: `test(eval): widen the docs golden set`

## Paso 2: expresión léxica de docs (DQ6; D3) — puerta: propuesta, decisión 4

### 6. 2a, palabras vacías y tokens cortos

- [ ] RED: tabla de casos de `docs_match_expression` (la pregunta larga, el identificador corto, sólo vacías); `rare_words_beat_stopwords_in_the_lexical_channel` (`project_docs/mod.rs`).
- [ ] GREEN: `DOCS_STOPWORDS`, `docs_match_expression`, `search` la usa; `fts_match_expression` sin cambios.
- [ ] Medición contra la línea base; si no pasa la puerta, se revierte y se anota.

### 7 a 10. 2b orden por rareza, 2c AND con respaldo OR, 2d repetidos, 2e rutas como frase

- [ ] Cada una con su test de tabla y su medición contra la anterior, un commit cada una.

## Paso 3: identificadores partidos (DQ7, DQ8; D4)

### 11. La huella

- [ ] RED: `docs_contract_names_the_lexical_version`, `an_old_fingerprint_reads_as_lexical_version_1`, `an_index_of_the_previous_version_is_rewritten_without_embedding` (con datos sembrados con la huella sin `+lex`), `another_model_still_reembeds_every_passage`.
- [ ] GREEN: `docs_contract`, `vector_part`, `stored_vectors` compara la parte del embedder.

### 12. Las formas partidas

- [ ] RED: `split_identifier` (tabla), `memory_type_memorytype_and_prose_find_each_other`, `the_embedded_and_shown_text_do_not_change`.
- [ ] GREEN y medición.

## Paso 4: prefijo de docs (DQ9; D5)

### 13. Experimento

- [ ] En un worktree descartable con `E5_PASSAGE_PREFIX = "passage: "`, `--real --docs` contra la línea vigente. Se anota y se borra el worktree y su directorio de build. Sólo si pasa la puerta se abre la tarea de productivizarlo.

## Cierre

### 14. Specs y verify

- [ ] Consolidar en `specs/eval` y `specs/project-docs` lo que entró; `verify.md` con la suite, los dos clippy y cada medición.
