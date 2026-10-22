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

## 4. La marca del disparo (R4)

Sin la marca, un export reconoce sólo los avisos que el agente ignoró ([Cómo cambió el criterio](proposal.md#cómo-cambió-el-criterio)). Tiene que llegar en el mismo release que el aviso: los casos de un release con el aviso y sin la marca no cuentan.

- [ ] Tests primero:
  - `an_unkeyed_save_marks_the_pair_with_its_revision_candidate` y `a_save_under_a_key_marks_no_pair` (`worker.rs`);
  - `following_the_warning_keeps_the_pair_marked` (`worker.rs`): el mismo texto bajo una clave, la fila toma la clave y el par sigue marcado;
  - la migración: V19 deja sin marca los pares que ya existen;
  - `the_export_carries_the_warning_mark_and_the_import_keeps_it` y `a_conflict_line_without_the_mark_reads_unmarked` (`portability`).
- [ ] `V19`, con `memory_conflict.topic_key_warned`. El worker calcula `unkeyed_revision` antes de `outbox_finish` o `finish_save`, y esa transacción marca el par. `ConflictLine.topic_key_warned` va en el export y en la importación.
- [ ] Docs: `.claude/CLAUDE.md` (la marca, V19 y la línea `conflict` del export).

## 5. Verificación

- [x] Suite y clippy de `.claude/CLAUDE.md` en Linux, con R1-R3.
- [ ] Suite y clippy de `.claude/CLAUDE.md` con R4.
- [ ] El CI en verde en Linux y Windows.
- [x] Fijar, antes del primer caso real, `N_MAX` y la decisión por defecto de la regla de tres salidas ([criterio](proposal.md#criterio-fijado-antes-de-medir)): 200 pares marcados y, si al llegar sigue sin definirse, el aviso se queda.
- [ ] **[casos reales]** `revision-eval store`: un modo que lista, de un export, los pares marcados (`topic_key_warned`) con su save, y una muestra al azar de los saves sin clave en que no disparó (`rest`), mezclados para etiquetarlos a ciegas. Por cada par marcado dice también si la fila avisada tiene clave, si el par tiene `supersedes` y si un save posterior reusó la clave. Tests primero, con un export armado a mano.
- [ ] **[casos reales]** Un export de un store que haya usado un clawg con la marca.
  - Etiquetar los pares con el protocolo de la propuesta: dos pasadas ciegas, kappa con su intervalo, tercera pasada para los desacuerdos.
  - Aplicar la regla de decisión: la precisión contra `p_min` de la prevalencia medida, con la cota de Wilson y sus tres salidas, y la precisión por distancia antes de quitar o ajustar.
  - Informar lo que hizo el agente y la tasa de disparo, también por tamaño del store.

  El texto queda fuera del repositorio. El resultado, fechado en `docs/eval/results/`, lleva sólo conteos y tasas. Si el aviso no pasa, se quita o se ajusta su regla en un cambio propio.
- [ ] `verify.md`.

## 6. Archive

- [ ] `specs/mcp` con R1-R4; `git mv` a `changes/archive/AAAA-MM-DD-topic-key-warning`; `state.yaml` archivado; fila 68 y el orden de ejecución del `README.md` de openspec; subir el minor.
