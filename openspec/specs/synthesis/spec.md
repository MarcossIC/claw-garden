# synthesis — Especificación

Fuente de verdad del comportamiento actual de la síntesis dirigida por el agente: cómo se guarda una conclusión con las memorias de las que sale, cómo se valida y se ve esa procedencia, qué devuelve `synthesize`, cuándo `recall_context` lo recuerda y cómo se revisa una conclusión que perdió una fuente. El ranking, los tiers y el umbral del detector no están acá: no cambiaron. Las filas `episodic_trajectory` y `procedural_skill` que dejó la app de escritorio siguen alcanzables por búsqueda, workspace y listado: eso está en [`specs/retrieval`](../retrieval/spec.md) (su apartado de las filas de la app). Qué agente ve y cita qué filas está en [`specs/visibility`](../visibility/spec.md).

**Origen:**
- [knowledge-synthesis](../../changes/archive/2026-10-13-knowledge-synthesis/proposal.md), archivado el 2026-10-13. Los ids de requisito son los de esa spec (R1 vive en `specs/retrieval`).
- [conclusion-upkeep](../../changes/archive/2026-10-19-conclusion-upkeep/proposal.md), archivado el 2026-10-19: «Conclusiones a revisar» y su aviso, el rechazo de una fuente reemplazada (R3) y el enlace que una purga conserva (R6).

## Definiciones

- **Fila viva:** fila de `memory` con `deleted_at IS NULL` y `status <> 'superseded'` (la misma definición que usa [`specs/disputes`](../disputes/spec.md)).
- **Conclusión:** fila de `memory` con al menos una fila en `memory_derivation` que la nombra como `memory_id`.
- **Fuentes de una conclusión:** los `source_id` de sus filas en `memory_derivation`.
- **Alcance:** un agente y un `project_id`, o todo el store cuando se pide sin proyecto. Como en visibility (y a diferencia de la búsqueda, que alcanza lo que [`specs/sharing`](../sharing/spec.md) comparte), un agente sólo ve sus filas; sin agente el pedido es del dueño de la base (el visor, `curl`) y ve las de todos. Su clave en `synthesis_review` es `(agent_id, scope_key)`: el agente o `''` para el dueño, y el `project_id` o `''` para todo el store.
- **Memoria pendiente de un alcance:** fila viva del alcance que no es conclusión y que no es fuente de ninguna conclusión viva **del mismo agente** (`PENDING`, `crates/clawg-core/src/memory/synthesis.rs`). La conclusión de otro agente no cubre la memoria, porque este agente no la ve.
- **Memoria sin revisar de un alcance:** memoria pendiente con `created_at` mayor que el `reviewed_at` del alcance, o cualquier pendiente si el alcance nunca se revisó.
- **Fuente perdida:** fuente que no es una fila viva: su fila no existe (purgada), está borrada o está `superseded`. Una fuente `disputed` no está perdida: la disputa ya sale en ACTIVE CONFLICTS y no dice que la fuente sea falsa.
- **Conclusión a revisar:** conclusión viva con al menos una fuente perdida.

## R2. Guardar una conclusión con sus fuentes

`POST /v1/memories` y `save_memory` MUST aceptar `derived_from`, una lista de ids de `memory`. Cada fila que el save inserta o refuerza MUST quedar enlazada a cada fuente en `memory_derivation`, en la misma transacción que la inserción o el refuerzo. Un id repetido MUST contarse una vez, y una fila MUST NOT enlazarse consigo misma (un save que refuerza a su propia fuente no crea el enlace). Una lista vacía MUST ser lo mismo que no mandarla.

#### Escenario: conclusión de dos memorias
- GIVEN dos memorias vivas A y B
- WHEN se guarda C con `derived_from = [A, B]`
- THEN `memory_derivation` tiene `(C, A)` y `(C, B)`, y la respuesta del save es la de siempre

#### Escenario: conclusión larga
- GIVEN dos memorias vivas A y B
- WHEN se guarda un texto que se parte en dos pedazos C1 y C2 con `derived_from = [A, B]`
- THEN los dos pedazos quedan enlazados a A y a B

#### Escenario: la conclusión ya existía
- GIVEN una conclusión C ya guardada con `derived_from = [A]`
- WHEN se guarda el mismo texto con `derived_from = [B]`
- THEN C se refuerza (no hay fila nueva) y queda enlazada a A y a B

**Test:** `a_conclusion_is_linked_to_its_sources`, `every_chunk_of_a_conclusion_is_linked`, `a_reinforced_conclusion_gains_the_new_sources` y `a_row_is_never_its_own_source` en `crates/clawg-core/src/memory/worker.rs`.

## R3. Fuentes inválidas

El daemon MUST rechazar con 400, sin encolar nada, un save cuyo `derived_from` nombre un id que no existe en `memory`, que está borrado o que es de otro agente, o que tenga más de 50 ids distintos. El mensaje MUST nombrar los ids rechazados, y MUST NOT distinguir una fila ajena de una que no existe (la misma regla de [`specs/visibility`](../visibility/spec.md)).

#### Escenario
- GIVEN una memoria viva A y una borrada D
- WHEN se guarda C con `derived_from = [A, D, 999999]`
- THEN la respuesta es 400 y nombra `D` y `999999`, y no se guarda nada

#### Escenario: fuente de otro agente
- GIVEN una memoria M del agente A y una T del agente B
- WHEN A guarda C con `derived_from = [M, T]`
- THEN la respuesta es 400 y nombra `T`

Una conclusión descansa sólo en filas vivas. El daemon MUST rechazar también con 400, sin encolar nada, un save cuyo `derived_from` nombre una fila propia, no borrada y `superseded`, con un mensaje que nombra cada id rechazado con la fila que lo reemplazó y pide sacar la conclusión de las vigentes: `derived_from names memories that were replaced: 5 (by 9); draw the conclusion from the current ones` (`replaced_memory_ids`, `crates/clawg-core/src/memory/synthesis.rs:330`; el rechazo en `crates/clawg-cli/src/daemon/api.rs:1049-1057`; `replaced_list`, `api.rs:316`). Se mira después de las que faltan, así que un id ausente, borrado o ajeno responde como arriba. Una fuente `disputed` MUST seguir aceptándose. Como el resto de `derived_from`, se valida antes de encolar: una fila de la cola escrita antes enlaza lo que nombra.

#### Escenario: fuente reemplazada
- GIVEN una memoria A reemplazada por A2 (un save con la misma `topic_key`)
- WHEN se guarda C con `derived_from = [A]`
- THEN la respuesta es 400 y nombra `A (by A2)`, y no se guarda nada; con `derived_from = [A2]`, C se guarda

**Test:** `a_save_from_unknown_sources_is_refused` y `a_save_from_a_replaced_source_is_refused` en `crates/clawg-cli/tests/api.rs`; los tests puros de la validación en `crates/clawg-cli/src/daemon/api.rs` (`sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `missing_sources_are_the_ones_that_were_not_found` y `replaced_sources_name_what_replaced_them`); `replaced_ids_name_their_replacement` en `synthesis.rs`.

## R4. Una conclusión no disputa a sus fuentes

El detector de contradicciones MUST excluir de sus candidatos las fuentes del save que está guardando, como excluye a los pedazos hermanos (R8 de [`specs/disputes`](../disputes/spec.md)). MUST seguir comparando la conclusión contra cualquier otra fila.

#### Escenario
- GIVEN una memoria A y otra O, las dos `user_preference` y dentro de la banda del detector respecto de C
- WHEN se guarda C con `derived_from = [A]`
- THEN C queda `disputed` contra O y no contra A

**Test:** `a_conclusion_does_not_dispute_its_sources` en `worker.rs`.

## R5. La procedencia se ve

`GET /v1/memories/memory/{id}` y `GET /v1/memories` MUST devolver `derived_from` (los ids de las fuentes, en orden ascendente) en cada fila de `memory` que sea conclusión, y omitirlo en las demás. `list_memories` en MCP MUST mostrarlo en la línea de la fila.

**Test:** `the_listing_names_the_sources_of_a_conclusion` en `crates/clawg-core/src/memory/admin.rs` y `rows_show_where_a_conclusion_came_from` en `crates/clawg-cli/src/mcp/tools.rs`.

## R6. Borrar no rompe la procedencia

Borrar una conclusión MUST NOT tocar sus fuentes ni sus filas de `memory_derivation`. Borrar una fuente MUST NOT tocar la conclusión. Una conclusión borrada o reemplazada MUST dejar de contar para decidir qué memorias están pendientes.

Purgar una fila (`clawg purge`) sí la borra de verdad: MUST borrar en la misma transacción sus filas de `memory_derivation` en las que es conclusión (`memory_id`), y MUST NOT borrar las que la nombran como fuente (`source_id`) (`crates/clawg-core/src/memory/secrets.rs:110`). Una conclusión purgada no descansa en nada, y la tabla no tiene claves foráneas (`crates/clawg-core/migrations/V10__synthesis.sql:8-13`), así que nada más borra sus enlaces. El enlace de una fuente purgada guarda un id, nunca texto, y es lo único que le dice a la conclusión que perdió esa fuente: la conclusión sigue siendo conclusión y queda a revisar con la fuente `purged` («Conclusiones a revisar»). Ese id no lo vuelve a tomar otra fila: `memory.id` es `AUTOINCREMENT` (`crates/clawg-core/migrations/V1__init.sql:5`), y `VACUUM` conserva `sqlite_sequence`.

#### Escenario: purgar una fuente
- GIVEN una conclusión C con `derived_from = [A, B]`
- WHEN se purga A
- THEN `(C, A)` sigue en `memory_derivation`, C sigue siendo conclusión y `synthesize` la lista a revisar con `lost = [A purged]`; purgar C borra sus dos enlaces

**Test:** `deleting_a_conclusion_makes_its_sources_pending_again` (R7) y `purging_keeps_the_links_that_name_the_row_as_a_source` en `crates/clawg-core/src/memory/secrets.rs`; `a_purged_source_leaves_the_conclusion_to_review` en `synthesis.rs`.

## R7. Síntesis

`POST /v1/synthesis` (y la herramienta MCP `synthesize`) MUST, para un alcance:

- devolver las memorias pendientes del alcance, de la más nueva a la más vieja, hasta `limit` (30 por defecto, entre 1 y 100), y cuántas pendientes hay en total;
- devolver las conclusiones vivas del alcance (hasta 20, de la más nueva a la más vieja) con su `derived_from`;
- devolver las conclusiones a revisar del alcance y cuántas son (`to_review` y `to_review_total`, «Conclusiones a revisar»);
- en `synthesize`, marcar el origen de cada memoria y conclusión que lo tiene (`origin=inferred`, `origin=conclusion`; P5 de [`specs/provenance`](../provenance/spec.md)). Una conclusión se guarda sellada `synthesis` / `agent`, y `user_stated` con `derived_from` es un 400 (P1 y P2);
- con agente, mirar sólo sus filas; la herramienta MCP siempre manda el agente que llama;
- registrar `reviewed_at = ahora` para el alcance en `synthesis_review`; la revisión de un agente no apaga el recordatorio de otro;
- MUST NOT escribir nada más: no crea, no borra ni cambia memorias, y no registra accesos.

`synthesize` MUST devolver el texto con las instrucciones: qué es una conclusión, que generaliza dos o más memorias, que se guarda con `save_memory`, el tipo que corresponde y `derived_from`, que una conclusión vieja que quedó mal se corrige guardando la nueva y resolviendo el conflicto, y que si nada generaliza no se guarda nada.

Sin memorias pendientes, `synthesize` MUST decir que ninguna espera una conclusión, MUST seguir mostrando las conclusiones ya sacadas (siguen respondiendo qué aprendió el agente) y las que hay que revisar, y MUST NOT incluir las instrucciones de cómo guardar una: no hay nada que guardar.

#### Escenario: sólo lo que falta
- GIVEN memorias A, B y E en el proyecto P, y una conclusión C viva con `derived_from = [A, B]`
- WHEN se sintetiza P
- THEN las pendientes son `[E]`, el total es 1 y las conclusiones son `[C]` con `derived_from = [A, B]`

#### Escenario: borrar la conclusión devuelve sus fuentes
- GIVEN el mismo estado
- WHEN se borra C y se sintetiza P
- THEN las pendientes son E, B y A

#### Escenario: otro proyecto no se mezcla
- GIVEN una memoria F en el proyecto Q
- WHEN se sintetiza P
- THEN F no está; y sintetizando sin proyecto, sí

#### Escenario: otro agente no se mezcla
- GIVEN una memoria del agente A y otra del agente B en P
- WHEN A sintetiza P
- THEN sólo está la de A, y B sigue teniendo su memoria sin revisar

#### Escenario: la conclusión de otro agente no cubre
- GIVEN una memoria M del agente A, y una conclusión del agente B con `derived_from = [M]`
- WHEN A sintetiza
- THEN M sigue pendiente

#### Escenario: sin pendientes
- GIVEN una conclusión C viva y ninguna memoria pendiente
- WHEN el agente llama `synthesize`
- THEN el texto dice que ninguna memoria espera una conclusión, lista C con sus fuentes y no trae las instrucciones de `save_memory`

**Test:** `synthesis_lists_what_no_conclusion_covers`, `deleting_a_conclusion_makes_its_sources_pending_again`, `synthesis_stays_in_its_project`, `synthesis_stays_with_its_agent`, `another_agents_conclusion_does_not_cover_a_memory`, `synthesis_records_the_review_and_nothing_else`, `existing_ids_leave_out_unknown_and_deleted_rows` y `existing_ids_leave_out_another_agents_rows` en `crates/clawg-core/src/memory/synthesis.rs`; `synthesis_over_http`, `an_agent_neither_reviews_nor_cites_another_agents_memories` y `a_misspelt_synthesis_field_is_refused` en `crates/clawg-cli/tests/api.rs`; `the_synthesis_text_says_what_to_save`, `a_synthesis_with_nothing_pending_asks_for_nothing` y `the_conclusions_show_even_with_nothing_pending` en `tools.rs`; `synthesize_lists_what_no_conclusion_covers_and_recall_stops_reminding` en `server.rs`.

No hay filtro de texto: la síntesis le da al modelo lo que no revisó, no un buscador. Buscar por texto es `search_memory`, y el filtro del listado es del visor, que no se le expone al modelo.

## Conclusiones a revisar

Una conclusión MUST quedar a revisar desde una sola fuente perdida: borrar o reemplazar una memoria dice algo sobre ella, y sólo quien lee la conclusión puede decir si lo que queda la sigue sosteniendo. El estado de cada fuente lo decide una función pura (`SourceState::of`, `crates/clawg-core/src/memory/synthesis.rs:72`): `purged` si la fila no existe, `deleted` si tiene `deleted_at`, `superseded` si su `status` lo es, y `live` en cualquier otro caso (`active` o `disputed`); `needs_review` (`synthesis.rs:85`) pide una que no sea `live`. En SQL la condición es una sola constante (`LOST_SOURCE`, `synthesis.rs:43`), y la regla pura tiene la última palabra sobre cada conclusión que la consulta elige (`reviews`, `synthesis.rs:182`), así que las dos no pueden listar distinto. La lista se calcula al leer, sin columna ni tabla: restaurar una fuente o revivir una versión saca la conclusión de la lista sin que nadie limpie estado.

`POST /v1/synthesis` MUST devolver, en la misma transacción que lo demás de R7 (`synthesis_core`, `synthesis.rs:236`):

- `to_review`: las conclusiones a revisar del alcance (mismo agente y proyecto que las pendientes), de la más nueva a la más vieja y hasta 10 (`REVIEW_LIMIT`, `synthesis.rs:48`), cada una con `conclusion` (la fila, con su `derived_from`), `live` (cuántas fuentes siguen vivas) y `lost` (cada fuente perdida con su `id`, su `state` y, si está `superseded`, su `superseded_by`, que se omite cuando no lo tiene) (`LostSource` y `ConclusionReview`, `synthesis.rs:89-104`);
- `to_review_total`: cuántas hay.

Una conclusión de otro agente MUST NOT aparecer. El tope es la mitad del de las conclusiones vivas: cada línea a revisar es más larga, y diez ya son una sesión de trabajo. El cliente MUST leer los dos campos con `#[serde(default)]` (`Synthesis`, `crates/clawg-cli/src/client.rs:302-314`): un daemon anterior no los manda.

`synthesize` MUST mostrarlas en su propio bloque, `Conclusions to review (K of N, newest first).`, seguido de la guía (`REVIEW_GUIDE`, `crates/clawg-cli/src/mcp/tools.rs:331-336`): borrar con `delete_memory` la que ya no vale; borrar y volver a guardar corregida, con `derived_from` en las memorias en que descansa ahora, la que vale en parte o ahora descansa en las que reemplazaron a sus fuentes; dejar la que sigue valiendo. Cada línea MUST llevar `live=K/N` y sus fuentes perdidas (`#4 deleted`, `#5 replaced by #9`, `#6 purged`; `lost_source` y `review_line`, `tools.rs:357-372`). MUST mostrarlo aunque no haya pendientes, y MUST NOT repetir esas conclusiones en «Conclusions already drawn» (`format_synthesis`, `tools.rs:377`). Las instrucciones de `protocol.md` dicen lo mismo en su sección `## Conclusions` (`crates/clawg-cli/src/mcp/protocol.md:130-138`), y la descripción de `synthesize` pide usarla cuando `recall_context` dice que hay conclusiones para revisar (`crates/clawg-cli/src/mcp/server.rs:1172-1182`).

#### Escenario: una fuente borrada
- GIVEN una conclusión C con `derived_from = [A, B]`
- WHEN se borra A y se sintetiza
- THEN `to_review` es `[C]` con `live = 1` y `lost = [A deleted]`
- WHEN se restaura A
- THEN `to_review` está vacía

#### Escenario: una fuente reemplazada
- GIVEN una conclusión C con `derived_from = [A]` y A reemplazada por A2
- THEN `to_review` tiene C con `lost = [A superseded by A2]`, y `synthesize` la muestra con `lost=#A replaced by #A2`

**Test:** `source_state_cases`, `review_needs_one_lost_source`, `a_conclusion_with_a_deleted_source_is_to_review`, `a_conclusion_with_every_source_live_is_not`, `restoring_the_source_takes_the_conclusion_off_the_list`, `a_replaced_source_names_its_replacement`, `a_purged_source_leaves_the_conclusion_to_review`, `the_review_list_is_capped_newest_first` y `another_agents_conclusion_is_not_to_review` en `synthesis.rs`; `synthesis_over_http` en `crates/clawg-cli/tests/api.rs`; `the_synthesis_names_what_each_conclusion_lost` y `conclusions_to_review_show_with_nothing_pending` en `tools.rs`.

### El aviso de revisión

`POST /v1/context` MUST devolver `conclusions_to_review` (`ContextBody`, `crates/clawg-cli/src/daemon/api.rs:618-620`; contado en `api.rs:1266-1270`): las conclusiones a revisar del alcance pedido y del agente del pedido que perdieron una fuente después del `reviewed_at` del alcance, o todas si el alcance nunca se revisó (`conclusions_to_review_count`, `synthesis.rs:294`). Una fuente borrada se perdió en su `deleted_at`; una `superseded`, en su `last_modified_at`, que el reemplazo escribe (`SUPERSEDE_SQL`, `crates/clawg-core/src/memory/dispute.rs:268`). Una fuente purgada MUST contar sólo si el alcance nunca se revisó: no queda cuándo se purgó, y `purge` es una operación a mano del dueño de la base. Así, llamar a `synthesize` apaga el aviso aunque la conclusión siga en la lista, y una pérdida posterior lo vuelve a encender; un aviso que contara todas las conclusiones a revisar no se apagaría nunca si el agente decide que una sigue valiendo. Un conteo que falla se registra y da 0, como `unreviewed`. El cliente MUST leer el campo con `#[serde(default)]` (`Context`, `crates/clawg-cli/src/client.rs:272-275`).

`recall_context` y el hook `session-start` MUST agregar al final, después del aviso de R8, una línea desde una conclusión a revisar: cuántas conclusiones descansan en memorias borradas o reemplazadas desde la última revisión, y que al terminar la tarea se llame a `synthesize` para revisarlas (`review_nudge`, `crates/clawg-cli/src/mcp/tools.rs:451`; `format_recall`, `tools.rs:489`; `session_start_text`, `crates/clawg-cli/src/hook.rs:86`). El umbral es 1, no el 20 de R8: una conclusión que perdió su base es más rara que una memoria sin revisar, y cuesta más si se deja. La línea MUST NOT ir dentro del prompt de `build_memory_prompt`, así que `rag-eval --agent-view` no cambia.

#### Escenario
- GIVEN una conclusión C con `derived_from = [A, B]`
- WHEN se borra A, THEN `conclusions_to_review` es 1
- WHEN se sintetiza el alcance, THEN es 0, y C sigue en `to_review`
- WHEN se borra B, THEN vuelve a ser 1

**Test:** `the_review_count_follows_losses_after_the_last_review` y `a_purged_source_counts_until_the_first_review` en `synthesis.rs`; `the_review_nudge_appears_from_one` en `tools.rs`; `session_start_carries_the_review_nudge` en `crates/clawg-cli/src/hook.rs`; `answers_without_conclusions_to_review_still_parse` en `crates/clawg-cli/src/client.rs`; `context_reports_conclusions_to_review` en `crates/clawg-cli/tests/api.rs`.

## R8. Recordatorio en `recall_context`

`POST /v1/context` MUST devolver `unreviewed`: cuántas memorias sin revisar tiene el alcance pedido, para el agente del pedido. `recall_context` MUST agregar al final del contexto una línea que nombra `synthesize` y la cantidad cuando `unreviewed` es 20 o más, y no agregar nada debajo de eso. La línea MUST NOT ir dentro del prompt que arma `build_memory_prompt`, para que `rag-eval --agent-view` no cambie. Un contexto vacío (`no context yet`) lleva la línea igual.

La misma línea llega al inicio de la sesión por el hook `session-start` ([`specs/hooks`](../hooks/spec.md), R4).

Las instrucciones del servidor MCP MUST nombrar `synthesize`.

#### Escenario
- GIVEN 25 memorias nuevas en P y ninguna síntesis
- WHEN se pide `recall_context` en P
- THEN el texto termina con la línea que nombra `synthesize` y 25
- WHEN se sintetiza P y se vuelve a pedir `recall_context`
- THEN la línea no está

**Test:** `unreviewed_counts_what_arrived_after_the_last_review` en `synthesis.rs`; `the_nudge_appears_from_the_threshold` y `the_nudge_follows_an_empty_context_too` en `tools.rs`; `context_reports_unreviewed_memories` en `crates/clawg-cli/tests/api.rs`; el test de instrucciones en `server.rs`.

## R9. Datos de la versión anterior

Una base en V9 MUST abrirse con esta versión aplicando V10 (`memory_derivation` y `synthesis_review`, dos tablas nuevas) sin cambiar ninguna fila existente, y todas sus memorias MUST quedar pendientes y sin revisar. Un volcado de la cola que una versión anterior escribió sin `derived_from` MUST leerse con `derived_from` vacío.

Las conclusiones a revisar no cambian el esquema. Un enlace a una fuente purgada que un binario anterior encuentra lo lee como cualquier otro: un `derived_from` que nombra un id que ya no existe, lo mismo que ya veía de una fuente borrada. Volver a ese binario no pide nada; si se quieren sacar esos enlaces, es a mano: `DELETE FROM memory_derivation WHERE source_id NOT IN (SELECT id FROM memory);`.

**Test:** `a_store_from_the_previous_version_opens_with_every_row_untouched` en `crates/clawg-core/src/memory/db.rs` (siembra en V9 una memoria, una trayectoria y un skill), `a_spill_without_sources_still_parses` en `crates/clawg-core/src/memory/spill.rs` y la fila de V10 en `published_migration_checksums_are_pinned`.
