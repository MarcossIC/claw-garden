# knowledge-synthesis — Especificación

**Dominio destino:** `specs/synthesis/spec.md` (nuevo; se crea al archivar). R1 se suma a `specs/retrieval/spec.md`.

**Alcance:** qué código sin llamador se va y qué lectura de datos se queda, cómo se guarda la procedencia de una conclusión, qué devuelve la síntesis y cuándo `recall_context` la recuerda. El ranking, los tiers y el umbral del detector no cambian.

## Definiciones

- **Fila viva:** fila de `memory` con `deleted_at IS NULL` y `status <> 'superseded'` (la misma definición que usa dispute-resolution).
- **Conclusión:** fila de `memory` con al menos una fila en `memory_derivation` que la nombra como `memory_id`.
- **Fuentes de una conclusión:** los `source_id` de sus filas en `memory_derivation`.
- **Alcance:** un agente y un `project_id`, o todo el store cuando se pide sin proyecto. Como en la búsqueda y en visibility-policy, un agente sólo ve sus filas; sin agente el pedido es del dueño de la base (el visor, `curl`) y ve las de todos. Su clave en `synthesis_review` es `(agent_id, scope_key)`: el agente o `''` para el dueño, y el `project_id` o `''` para todo el store.
- **Memoria pendiente de un alcance:** fila viva del alcance que no es conclusión y que no es fuente de ninguna conclusión viva.
- **Memoria sin revisar de un alcance:** memoria pendiente con `created_at` mayor que el `reviewed_at` del alcance, o cualquier pendiente si el alcance nunca se revisó.

## R1. Las filas que dejó la app siguen alcanzables

Borrar el pipeline LLM MUST NOT cambiar qué se recupera de `episodic_trajectory` y `procedural_skill`: la búsqueda MUST seguir consultándolas (KNN de 5 + 5), el workspace MUST seguir mostrando los skills del proyecto, y `list_memories`, `GET`, `PATCH` y `DELETE` MUST seguir aceptándolas. El re-embed de `procedural_skill` MUST seguir embebiendo `"{trigger} {skill_name}"`.

#### Escenario: filas como las escribía la app de escritorio
- GIVEN una trayectoria y un skill escritos con los mismos escritores que usaba la app (que quedan sólo para tests), y sus vectores
- WHEN se busca con el vector del skill, se regenera el workspace del proyecto, se lista y se re-embebe
- THEN el skill está entre los hits y en el workspace, las dos filas están en el listado, y el re-embed del skill embebe `"when login fails Fix login flow"` para `trigger = "when login fails"` y `skill_name = "Fix login flow"`

**Test:** los que ya existen y siguen pasando después del borrado: `episodic_trajectory_retrievable_by_same_agent_id` y `knn_procedural_returns_skill_rows` (`search.rs`), `regenerate_reports_the_ids_it_rendered` y `workspace_top_skills_exclude_foreign_project` (`workspace.rs`), `list_memories_kind_none_merges_all_tables_sorted_desc` (`admin.rs`) y `reembed_reinfer_uses_passage_role_and_shared_skill_embedding_text` (`maintenance.rs`), que ahora fija el texto literal en vez de llamar a la función que se mudó. No fallan hoy: protegen el borrado. La apertura de una base en V9 con esas filas es R9.

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

**Test:** `a_conclusion_is_linked_to_its_sources`, `every_chunk_of_a_conclusion_is_linked`, `a_reinforced_conclusion_gains_the_new_sources` y `a_row_is_never_its_own_source` en `crates/clawg-core/src/memory/worker.rs`. Fallan hoy: el campo no existe.

## R3. Fuentes inválidas

El daemon MUST rechazar con 400, sin encolar nada, un save cuyo `derived_from` nombre un id que no existe en `memory`, que está borrado o que es de otro agente, o que tenga más de 50 ids distintos. El mensaje MUST nombrar los ids rechazados, y MUST NOT distinguir una fila ajena de una que no existe (la misma regla de visibility-policy).

#### Escenario
- GIVEN una memoria viva A y una borrada D
- WHEN se guarda C con `derived_from = [A, D, 999999]`
- THEN la respuesta es 400 y nombra `D` y `999999`, y no se guarda nada

#### Escenario: fuente de otro agente
- GIVEN una memoria M del agente A y una T del agente B
- WHEN A guarda C con `derived_from = [M, T]`
- THEN la respuesta es 400 y nombra `T`

**Test:** `a_save_from_unknown_sources_is_refused` en `crates/clawg-cli/tests/api.rs` y los tests puros de la validación en `crates/clawg-cli/src/daemon/api.rs` (`sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `missing_sources_are_the_ones_that_were_not_found`). Fallan hoy.

## R4. Una conclusión no disputa a sus fuentes

El detector de contradicciones MUST excluir de sus candidatos las fuentes del save que está guardando, como ya excluye a los pedazos hermanos. MUST seguir comparando la conclusión contra cualquier otra fila.

#### Escenario
- GIVEN una memoria A y otra O, las dos `user_preference` y dentro de la banda del detector respecto de C
- WHEN se guarda C con `derived_from = [A]`
- THEN C queda `disputed` contra O y no contra A

**Test:** `a_conclusion_does_not_dispute_its_sources` en `worker.rs`. Falla hoy.

## R5. La procedencia se ve

`GET /v1/memories/memory/{id}` y `GET /v1/memories` MUST devolver `derived_from` (los ids de las fuentes, en orden ascendente) en cada fila de `memory` que sea conclusión, y omitirlo en las demás. `list_memories` en MCP MUST mostrarlo en la línea de la fila.

**Test:** `the_listing_names_the_sources_of_a_conclusion` en `crates/clawg-core/src/memory/admin.rs` y `rows_show_where_a_conclusion_came_from` en `crates/clawg-cli/src/mcp/tools.rs`. Fallan hoy.

## R6. Borrar no rompe la procedencia

Borrar una conclusión MUST NOT tocar sus fuentes ni sus filas de `memory_derivation`. Borrar una fuente MUST NOT tocar la conclusión. Una conclusión borrada o reemplazada MUST dejar de contar para decidir qué memorias están pendientes.

**Test:** cubierto por R7 (`deleting_a_conclusion_makes_its_sources_pending_again`).

## R7. Síntesis

`POST /v1/synthesis` (y la herramienta MCP `synthesize`) MUST, para un alcance:

- devolver las memorias pendientes del alcance, de la más nueva a la más vieja, hasta `limit` (30 por defecto, entre 1 y 100), y cuántas pendientes hay en total;
- devolver las conclusiones vivas del alcance (hasta 20, de la más nueva a la más vieja) con su `derived_from`;
- con agente, mirar sólo sus filas; la herramienta MCP siempre manda el agente que llama;
- registrar `reviewed_at = ahora` para el alcance en `synthesis_review`; la revisión de un agente no apaga el recordatorio de otro;
- MUST NOT escribir nada más: no crea, no borra ni cambia memorias, y no registra accesos.

`synthesize` MUST devolver el texto con las instrucciones: qué es una conclusión, que generaliza dos o más memorias, que se guarda con `save_memory`, el tipo que corresponde y `derived_from`, que una conclusión vieja que quedó mal se corrige guardando la nueva y resolviendo el conflicto, y que si nada generaliza no se guarda nada.

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

**Test:** `synthesis_lists_what_no_conclusion_covers`, `deleting_a_conclusion_makes_its_sources_pending_again`, `synthesis_stays_in_its_project`, `synthesis_stays_with_its_agent`, `synthesis_records_the_review_and_nothing_else`, `existing_ids_leave_out_unknown_and_deleted_rows` y `existing_ids_leave_out_another_agents_rows` en `crates/clawg-core/src/memory/synthesis.rs`; `synthesis_over_http`, `an_agent_neither_reviews_nor_cites_another_agents_memories` y `a_misspelt_synthesis_field_is_refused` en `crates/clawg-cli/tests/api.rs`; `the_synthesis_text_says_what_to_save`, `a_synthesis_with_nothing_pending_asks_for_nothing` en `tools.rs`; `synthesize_lists_what_no_conclusion_covers_and_recall_stops_reminding` en `server.rs`. Fallan hoy.

No hay filtro de texto: la síntesis le da al modelo lo que no revisó, no un buscador. Buscar por texto es `search_memory`, y el filtro del listado es del visor, que no se le expone al modelo.

## R8. Recordatorio en `recall_context`

`POST /v1/context` MUST devolver `unreviewed`: cuántas memorias sin revisar tiene el alcance pedido, para el agente del pedido. `recall_context` MUST agregar al final del contexto una línea que nombra `synthesize` y la cantidad cuando `unreviewed` es 20 o más, y no agregar nada debajo de eso. La línea MUST NOT ir dentro del prompt que arma `build_memory_prompt`, para que `rag-eval --agent-view` no cambie.

Las instrucciones del servidor MCP MUST nombrar `synthesize`.

#### Escenario
- GIVEN 25 memorias nuevas en P y ninguna síntesis
- WHEN se pide `recall_context` en P
- THEN el texto termina con la línea que nombra `synthesize` y 25
- WHEN se sintetiza P y se vuelve a pedir `recall_context`
- THEN la línea no está

**Test:** `unreviewed_counts_what_arrived_after_the_last_review` en `synthesis.rs`; `the_nudge_appears_from_the_threshold` en `tools.rs`; `context_reports_unreviewed_memories` en `crates/clawg-cli/tests/api.rs`; el test de instrucciones en `server.rs`. Fallan hoy.

## R9. Datos de la versión anterior

Una base en V9 MUST abrirse con esta versión aplicando V10 sin cambiar ninguna fila existente, y todas sus memorias MUST quedar pendientes y sin revisar. Un volcado de la cola escrito sin `derived_from` MUST leerse con `derived_from` vacío.

**Test:** `a_store_from_the_previous_version_opens_with_every_row_untouched` en `crates/clawg-core/src/memory/db.rs` (siembra en V9 una memoria, una trayectoria y un skill), `a_spill_without_sources_still_parses` en `crates/clawg-core/src/memory/spill.rs` y la fila de V10 en `published_migration_checksums_are_pinned`. Fallan hoy.
