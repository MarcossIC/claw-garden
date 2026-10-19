# conclusion-upkeep — Especificación

Requisitos nuevos sobre [`specs/synthesis`](../../../specs/synthesis/spec.md) y [`specs/secrets`](../../../specs/secrets/spec.md). Lo que no se nombra acá sigue como está. Términos: una fila **viva** no está borrada y su `status` no es `superseded`; una fuente está **perdida** cuando su fila no existe (purgada), está borrada o está `superseded`; una conclusión **a revisar** es una conclusión viva con al menos una fuente perdida.

## C1. La regla

El estado de una fuente MUST decidirse por una función pura: `purged` si la fila no existe, `deleted` si tiene `deleted_at`, `superseded` si su `status` lo es, y `live` en cualquier otro caso (`active` o `disputed`). Una conclusión MUST quedar a revisar desde una sola fuente perdida.

**Test:** `source_state_cases` y `review_needs_one_lost_source` (`synthesis.rs`).

## C2. `synthesize` lista las conclusiones a revisar

`POST /v1/synthesis` MUST devolver `to_review`, las conclusiones a revisar del alcance (mismo agente y proyecto que las pendientes), de la más nueva a la más vieja y hasta 10, cada una con `conclusion` (la fila, con su `derived_from`), `live` (cuántas fuentes siguen vivas) y `lost` (cada fuente perdida con su `id`, su `state` y, si está `superseded`, su `superseded_by`); y `to_review_total`, cuántas hay. Restaurar la fuente perdida MUST sacar la conclusión de la lista. Una conclusión de otro agente MUST NOT aparecer.

#### Escenario: una fuente borrada
- GIVEN una conclusión C con `derived_from = [A, B]`
- WHEN se borra A y se sintetiza
- THEN `to_review` es `[C]` con `live = 1` y `lost = [A deleted]`
- WHEN se restaura A
- THEN `to_review` está vacía

#### Escenario: una fuente reemplazada
- GIVEN una conclusión C con `derived_from = [A]` y A reemplazada por A2
- THEN `to_review` tiene C con `lost = [A superseded by A2]`

**Test:** `a_conclusion_with_a_deleted_source_is_to_review`, `a_conclusion_with_every_source_live_is_not`, `restoring_the_source_takes_the_conclusion_off_the_list`, `a_replaced_source_names_its_replacement` y `another_agents_conclusion_is_not_to_review` (`synthesis.rs`); `synthesis_over_http` (`crates/clawg-cli/tests/api.rs`).

## C3. El texto de `synthesize`

`synthesize` MUST mostrar las conclusiones a revisar en su propio bloque, cada una con `live=K/N` y sus fuentes perdidas (`#4 deleted`, `#5 replaced by #9`, `#6 purged`), con la guía de qué hacer: borrar la que ya no vale, borrar y volver a guardar corregida la que vale en parte o ahora descansa en las que reemplazan, y dejar la que sigue valiendo. MUST mostrarlo aunque no haya pendientes, y MUST NOT repetir esas conclusiones en «Conclusions already drawn».

**Test:** `the_synthesis_names_what_each_conclusion_lost` y `conclusions_to_review_show_with_nothing_pending` (`tools.rs`).

## C4. El aviso

`POST /v1/context` MUST devolver `conclusions_to_review`: las conclusiones a revisar del alcance pedido y del agente del pedido con una fuente perdida después del `reviewed_at` del alcance (por su `deleted_at`, o por su `last_modified_at` si está `superseded`), o todas si el alcance nunca se revisó. Una fuente purgada MUST contar sólo si el alcance nunca se revisó: no queda cuándo se perdió. `recall_context` y el hook `session-start` MUST agregar al final una línea que nombra `synthesize` y la cantidad cuando es 1 o más. La línea MUST NOT ir dentro del prompt de `build_memory_prompt`. El cliente MUST leer el campo con `#[serde(default)]`.

#### Escenario
- GIVEN una conclusión C con `derived_from = [A, B]`
- WHEN se borra A, THEN `conclusions_to_review` es 1
- WHEN se sintetiza el alcance, THEN es 0, y C sigue en `to_review`
- WHEN se borra B, THEN vuelve a ser 1

**Test:** `the_review_count_follows_losses_after_the_last_review` y `a_purged_source_counts_until_the_first_review` (`synthesis.rs`); `the_review_nudge_appears_from_one` (`tools.rs`); `session_start_carries_the_review_nudge` (`hook.rs`); `context_reports_conclusions_to_review` (`crates/clawg-cli/tests/api.rs`).

## C5. `derived_from` rechaza lo reemplazado

El daemon MUST rechazar con 400, sin encolar nada, un save cuyo `derived_from` nombre una fila propia, no borrada, con `status = 'superseded'`. El mensaje MUST nombrar cada id rechazado con su reemplazante (`5 (by 9)`). Una fuente `disputed` MUST seguir aceptándose.

**Test:** `replaced_ids_name_their_replacement` (`synthesis.rs`); `a_save_from_a_replaced_source_is_refused` (`crates/clawg-cli/tests/api.rs`).

## C6. `purge` conserva el vínculo de la fuente

`clawg purge` de una fila MUST borrar los vínculos de `memory_derivation` en los que es conclusión (`memory_id`) y MUST NOT borrar aquellos en los que es fuente (`source_id`). La conclusión MUST seguir siendo conclusión y quedar a revisar con la fuente `purged`.

**Test:** `purging_keeps_the_links_that_name_the_row_as_a_source` (`secrets.rs`); `a_purged_source_leaves_the_conclusion_to_review` (`synthesis.rs`).
