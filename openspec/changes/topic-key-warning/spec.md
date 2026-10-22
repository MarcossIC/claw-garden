# topic-key-warning — Especificación

## R1. La candidata dice si una clave del save la alcanza

Cada candidata del detector (`Candidate`) MUST llevar `same_scope`: verdadero cuando la fila es del proyecto del save, o cuando ni la fila ni el save tienen proyecto, que es el alcance de una clave (`SCOPE_ROWS_SQL`). Una fila sin proyecto, o una `user_preference` de otro proyecto, que el detector también alcanza, MUST llevarlo falso. El campo no cambia qué pares se escriben ni su estado.

## R2. El save sin clave nombra la fila que más probablemente revisa

Un save confirmado sin `topic_key` MUST nombrar en `revision_candidate` la candidata más cercana con `same_scope`, y de dos igual de cercanas la primera que encontró el detector. Un save con clave, o uno sin candidatas de su alcance, MUST NOT nombrar ninguna, y la respuesta de `POST /v1/memories` MUST omitir el campo. Un save que queda en cola (`202`) no lo nombra. La decisión MUST ser una función pura de la clave y las candidatas (`unkeyed_revision`).

#### Escenario: la más cercana de su proyecto
- GIVEN tres filas del agente: una del proyecto `p-1` a distancia 0,3 del save, otra sin proyecto a 0,15 y otra de `p-1` a 0,2
- WHEN el agente guarda sin clave en `p-1`
- THEN las tres son candidatas, la sin proyecto con `same_scope` falso
- AND `revision_candidate` es la de `p-1` a 0,2, no la más cercana de todas

#### Escenario: con clave
- GIVEN una fila del agente en `p`
- WHEN el agente guarda bajo `build/tool` un texto cercano en `p`
- THEN la respuesta nombra la fila en `related` y no lleva `revision_candidate`

#### Escenario: un save sin proyecto
- GIVEN filas del agente sólo en proyectos
- WHEN guarda sin proyecto ni clave un texto cercano
- THEN ninguna candidata tiene `same_scope` y no hay `revision_candidate`

**Tests:** `an_unkeyed_save_revises_its_closest_candidate_of_its_own_scope` (`topic.rs`); `an_unkeyed_save_names_the_closest_candidate_its_key_could_revise` y `a_save_under_a_key_or_without_a_candidate_of_its_scope_names_none` (`worker.rs`); `a_confirmed_save_names_its_revision_candidate_only_when_it_has_one` (`daemon/api.rs`); `an_unkeyed_save_names_the_row_a_key_would_revise` (`tests/api.rs`).

## R3. `save_memory` avisa con la línea medida

Cuando la respuesta trae `revision_candidate`, `save_memory` MUST agregar, en una línea propia y después de la de `close to earlier memories`, `this save has no topic_key: if it revises #N, save it again under a topic_key naming their subject, so the next revision replaces it`, con `N` la fila nombrada. Sin el campo (un save con clave, sin candidata de su alcance, o un daemon anterior) MUST NOT agregarla. La línea MUST ser la que midió `revision-eval` (`WARNING_LINE`, `cost.rs`) con el id de la fila.

#### Escenario: la línea
- GIVEN un save confirmado como `#57` con `related` `#412 "Build tool"` y `revision_candidate` 412
- THEN el texto es `saved #57`, la línea `close to earlier memories: #412 "Build tool"…` y, en la última línea, el aviso con `#412`

**Tests:** `an_unkeyed_save_close_to_its_own_row_is_told_to_take_a_key` (`server.rs`); `save_memory_ships_the_warning_line_that_was_measured` (`crates/clawg-eval/src/bin/revision-eval/cost.rs`).
