# topic-key-warning — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. La candidata y la regla (R1, R2)

- [x] Tests primero: `an_unkeyed_save_revises_its_closest_candidate_of_its_own_scope` (`topic.rs`), `an_unkeyed_save_names_the_closest_candidate_its_key_could_revise` y `a_save_under_a_key_or_without_a_candidate_of_its_scope_names_none` (`worker.rs`).
- [x] `Candidate.same_scope`, `unkeyed_revision` y `IngestReport.revision_candidate`, que `process_tracked` llena al terminar el save.

## 2. La respuesta y el aviso (R2, R3)

- [x] Tests primero: `a_confirmed_save_names_its_revision_candidate_only_when_it_has_one` (`daemon/api.rs`), `an_unkeyed_save_names_the_row_a_key_would_revise` (`tests/api.rs`), `an_unkeyed_save_close_to_its_own_row_is_told_to_take_a_key` (`server.rs`) y `save_memory_ships_the_warning_line_that_was_measured` (`cost.rs`).
- [x] `SaveBody.revision_candidate`, `SaveOutcome::Saved.revision_candidate`, `topic_key_warning` en `tools.rs` y la línea en `save_outcome_text`.

## 3. Docs

- [x] `.claude/CLAUDE.md`, el mapa de `openspec/README.md` y la tarea 3 de [revision-recheck](../revision-recheck/tasks.md), cuyo criterio, protocolo y umbral fija la [propuesta](proposal.md#criterio-fijado-antes-de-medir).

## 4. Verificación

- [x] Suite y clippy de `.claude/CLAUDE.md` en Linux.
- [ ] El CI en verde en Linux y Windows.
- [ ] **[casos reales]** `revision-eval store`: un modo que lista, de un export, sólo los saves en que disparó el aviso (un save sin clave, de un tipo que afirma, con una línea `conflict` hacia la fila más cercana de su agente y proyecto) con su par, para etiquetarlos. Tests primero, con un export armado a mano.
- [ ] **[casos reales]** Un export de un store que haya usado un clawg con el aviso. Etiquetar los pares con el protocolo de la propuesta (dos pasadas ciegas, kappa con su intervalo, tercera pasada para los desacuerdos) y medir la precisión contra el 40 %, también por tamaño del store. El texto queda fuera del repositorio y el resultado, fechado en `docs/eval/results/`, lleva sólo conteos y tasas. Si no pasa, el aviso se quita o se ajusta su regla en un cambio propio.
- [ ] `verify.md`.

## 5. Archive

- [ ] `specs/mcp` con R1-R3; `git mv` a `changes/archive/AAAA-MM-DD-topic-key-warning`; `state.yaml` archivado; fila 68 y el orden de ejecución del `README.md` de openspec; subir el minor.
