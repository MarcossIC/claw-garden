# embed-throughput — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. La medición (ET3)

- [ ] Test primero (`import.rs`): `padding_counts_each_run_of_the_embedder_apart`, que no compila sin `padding`.
- [ ] `padding` y `the_embedding_phase_of_a_real_export` (`#[ignore]`), con `rows_to_embed` sacado de `write` para que el banco embeba las mismas filas que la importación.
- [ ] Correrlo sobre el export real: M1 sobre todas las filas, M2 a M4 sobre las primeras 5.000 con el modelo (resultados en la [propuesta](proposal.md#problema)).

## 2. El orden (ET1, ET2)

- [ ] Tests primero: `longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order`, `an_import_of_another_tool_embeds_its_rows_longest_first`, `a_clawg_import_embeds_its_rows_longest_first` (`import.rs`) y `counting_together_counts_each_text_as_alone` (`token_splitter.rs`), que fallan por no compilar o porque hoy los textos llegan en el orden del archivo; y `each_imported_row_keeps_the_vector_of_its_own_text`, que pasa hoy.
- [ ] `TokenSplitter::count_all` y `ChatChunker::count_all`.
- [ ] `longest_first` y `embedding_order` en `import.rs`; `write` embebe en ese orden dentro de la fase `embed`.
- [ ] `import` recibe el chunker; `import_memories` arma uno y se lo pasa a `import` y a `import_foreign`.
- [ ] El banco mide además el orden de la importación.
- [ ] Las pruebas de import que ya existen pasan sin cambiar ninguna aserción.

## 3. Docs

- [ ] `.claude/CLAUDE.md` (cómo embebe una importación) y la fila 85 de `openspec/README.md`.

## 4. Verificación

- [ ] Suite y los dos clippy de `.claude/CLAUDE.md` en Windows, en `target-test`.
- [ ] **[perfil dev aislado, modelo real]** La importación real del export completo con el build de la cabeza anterior y con el del cambio: `embed` al menos 15 % menos, el mismo reporte, y la memoria privada de `embed` y su pico por minuto a no más de un 5 % de la base.
- [ ] `verify.md`.

## 5. Archive

- [ ] ET1 a ET3 en `specs/portability`; `git mv` a `changes/archive/2026-10-24-embed-throughput`; `state.yaml` archivado; la fila 85 y el orden de ejecución del `README.md` de openspec; subir el minor.
