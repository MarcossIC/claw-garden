# agent-authorship — Especificación

Cambia requisitos de [`specs/visibility`](../../../specs/visibility/spec.md), [`specs/sharing`](../../../specs/sharing/spec.md), [`specs/disputes`](../../../specs/disputes/spec.md), [`specs/ingest`](../../../specs/ingest/spec.md), [`specs/restore`](../../../specs/restore/spec.md), [`specs/update`](../../../specs/update/spec.md), [`specs/pieces`](../../../specs/pieces/spec.md), [`specs/synthesis`](../../../specs/synthesis/spec.md), [`specs/mcp`](../../../specs/mcp/spec.md), [`specs/retrieval`](../../../specs/retrieval/spec.md), [`specs/doctor`](../../../specs/doctor/spec.md), [`specs/portability`](../../../specs/portability/spec.md) y [`specs/storage`](../../../specs/storage/spec.md). Lo que no se nombra aquí sigue como está.

## Definiciones

- **Lector:** el agente que hace una llamada, con lo que `sharing.json` le deja alcanzar (`Reader { agent_id, reach }`). Una fila **está al alcance** del lector cuando la escribió él, cuando no la escribió ningún agente (`agent_id` NULL) o cuando su autor no está oculto para él (`Reach::admits`). Una llamada **sin lector** es del usuario y alcanza toda fila.
- **El que guarda:** el lector de un save, con el alcance que la política le da cuando el worker lo procesa.
- **Por proyecto:** para la dedup y la clave, igualdad de `COALESCE(project_id, '')`; para el detector, el predicado de proyecto de la búsqueda (el proyecto, las filas sin proyecto y toda `user_preference`).

## AA1. Un solo criterio para leer y modificar

`read_save_core`, `update_memory_core`, `delete_memory_core`, `restore_memory_core`, `judge_pair`, `resolve_dispute_core`, `existing_memory_ids` y `replaced_memory_ids` MUST recibir el lector (`Option<&Reader>`) en lugar de un dueño, y MUST tratar como inexistente, sin escribir nada, una fila que no está al alcance del lector. Una fila de otro agente o sin autor que está al alcance MUST tratarse como una propia. Sin lector MUST alcanzar toda fila.

`read_save_core` MUST juntar cada pieza del save que está al alcance; una pieza fuera del alcance MUST figurar como faltante.

#### Escenario: otro agente trabaja con lo que encuentra
- GIVEN un save largo de A y una fila sin autor
- WHEN B lee entero el save, cambia la importancia de una fila de A, la borra, la restaura y juzga un par entre dos filas de A
- THEN cada operación queda hecha y A la ve

#### Escenario: lo que la política oculta
- GIVEN `sharing.json` con `hide B A`
- WHEN B lee, cambia, borra, restaura o juzga una fila de A
- THEN la respuesta es «no existe» y nada cambia, y A sigue pudiendo con lo de B

**Test:** `a_reader_admits_what_its_reach_admits` (`sharing.rs`); `the_reach_predicate_agrees_with_admits` (`search.rs`, también `reader_predicate!`); `read_save_lists_missing_pieces_and_another_agent_reads_it_whole`, `a_hidden_authors_piece_reads_as_missing` (`pieces.rs`); `another_agent_updates_a_row_it_reaches_but_not_a_hidden_one`, `a_hidden_authors_row_is_not_found_and_stays`, `restoring_needs_a_deleted_row_in_reach` (`admin.rs`); `another_agent_judges_a_pair_it_reaches`, `a_pair_with_a_hidden_side_is_not_found` (`dispute.rs`); `existing_ids_follow_the_readers_reach` (`synthesis.rs`); `another_agent_deletes_a_memory_unless_hidden`, `another_agent_restores_a_memory_unless_hidden`, `another_agent_reads_a_save_unless_hidden`, `another_agent_updates_a_memory_unless_hidden`, `another_agent_resolves_a_conflict_unless_hidden`, `a_shared_memory_is_changed_by_the_agent_that_found_it` (`server.rs`); `get_save_of_another_agent_reads_whole_unless_hidden`, `delete_with_a_hidden_author_is_404_and_another_agent_deletes_otherwise`, `patch_with_a_hidden_author_is_404_and_another_agent_patches_otherwise`, `resolve_with_a_hidden_author_is_404_and_another_agent_resolves_otherwise`, `restore_brings_a_deleted_memory_back_for_any_agent_that_reaches_it` (`crates/clawg-cli/tests/api.rs`).

## AA2. El daemon nombra al lector

`?agent=` de `PATCH`, `DELETE`, `GET …/save` y `POST …/restore`, y `agent_id` de `POST /v1/disputes/judge`, `/v1/disputes/resolve`, `POST /v1/synthesis` y `GET /v1/disputes` MUST seguir aceptándose y MUST nombrar al lector, cuyo alcance el daemon lee de `sharing.json` en cada pedido. Un `sharing.json` que no se puede leer MUST dejar al lector con sus filas y las sin autor. `derived_from` MUST comprobar sus fuentes con el alcance de `agent_id`, y una fuente fuera del alcance MUST ser «no existe».

**Test:** las de AA1 en `tests/api.rs`, que mandan `?agent=` y `agent_id` como un puente anterior; `an_unreadable_policy_leaves_an_agent_its_own_rows_and_the_authorless` y `a_conclusion_may_cite_another_agents_memory_unless_hidden` (`tests/api.rs`).

## AA3. La dedup es del proyecto

Un save MUST reforzar la fila viva más antigua del proyecto con su `content_hash` que está al alcance del que guarda, sea de quien sea. `RESTORE_TWIN` MUST buscar la gemela viva igual, con el alcance del lector del restore. `idx_memory_dedup` MUST servir la búsqueda sin `TEMP B-TREE`.

#### Escenario
- GIVEN una fila de A en el proyecto P
- WHEN B guarda el mismo texto en P
- THEN el save responde `reinforced` con el id de la fila de A; en otro proyecto, o con `hide B A`, se inserta otra fila

**Test:** `reinforce_if_duplicate_reinforces_another_agents_row`, `reinforce_if_duplicate_leaves_a_hidden_authors_row` y `reinforce_lookup_uses_dedup_index` (`store_insert.rs`); `the_same_text_from_another_agent_reinforces_the_row_already_there` (`worker.rs`); `a_restore_finds_another_agents_twin` (`admin.rs`).

## AA4. El detector compara con el proyecto al alcance

`detect_contradiction` MUST buscar candidatas con el alcance del que guarda (al restaurar, el del lector; sin lector, toda fila), las sin autor incluidas. `Reach::OwnOnly` MUST desaparecer. `revision_candidate` MAY nombrar la fila de otro agente.

#### Escenario
- GIVEN una `architectural_decision` de A y otra sin autor en el proyecto
- WHEN B guarda una decisión dentro de la banda de las dos
- THEN el save de B tiene un par con cada una y las nombra en `related`; con `hide B A`, sólo con la sin autor

**Test:** `contradiction_probe_reaches_other_authors`, `a_save_is_paired_with_another_agents_row_and_a_row_no_agent_saved` y `a_hidden_author_is_never_a_candidate` (`contradiction.rs`).

## AA5. La clave de tema es del proyecto

El alcance de una clave MUST ser `(COALESCE(project_id, ''), topic_key)` dentro del alcance del que guarda: un save con clave MUST reemplazar a todo titular vivo del proyecto que alcanza, de cualquier autor, y listarlo en `replaced`. El titular que comprueba un restore (`RESTORE_TOPIC_HOLDER`) MUST buscarse igual con el alcance del lector. El revivir al borrar MUST considerar revisión a toda fila que la clave reemplazó en el proyecto, sin mirar el autor, y MUST tomar como titular a cualquier fila viva del proyecto con la clave. `idx_memory_topic` MUST servir la lectura del alcance.

#### Escenario
- GIVEN un titular de A con la clave K en el proyecto P
- WHEN B guarda con K en P
- THEN la fila de A queda `superseded` por la de B con causa `revision`; borrar la de B revive la de A; en otro proyecto, o con `hide B A`, nada se reemplaza

**Test:** `a_key_is_scoped_to_its_project`, `a_keyed_save_replaces_every_holder_of_its_project`, `a_key_leaves_a_hidden_authors_holder` (`worker.rs`); `deleting_another_agents_revision_revives_the_earlier_one`, `a_revival_stays_in_its_project` y `a_row_whose_key_another_agent_holds_is_not_restored` (`admin.rs`); `the_scope_lookup_uses_the_topic_index` (`topic.rs`).

## AA6. La síntesis es del proyecto

Una memoria MUST estar pendiente cuando está viva, no es una conclusión y ninguna conclusión viva, de cualquier autor, se sacó de ella. El alcance de `synthesize`, `unreviewed` y `conclusions_to_review` MUST ser el proyecto (sin proyecto, todo el store) dentro del alcance del lector. La revisión MUST guardarse por proyecto (`synthesis_review.agent_id = ''`) y MUST apagar el aviso para todo agente del proyecto. Las filas por agente de `synthesis_review` MUST quedar como están.

**Test:** `another_agents_conclusion_covers_a_memory`, `another_agents_conclusion_is_to_review_unless_hidden`, `synthesis_reaches_every_author_of_the_project`, `a_review_is_the_projects` (`synthesis.rs`); `an_agent_reviews_another_agents_memories_unless_hidden` y `context_reports_unreviewed_memories` (`tests/api.rs`); `synthesize_lists_what_no_conclusion_covers_and_recall_stops_reminding` (`server.rs`).

## AA7. Las vistas

- ACTIVE CONFLICTS MUST mostrar los pares con un lado entre las filas del prompt cuyas dos filas están al alcance del lector, de cualquier autor.
- LAST SESSION MUST leer entero el resumen con el alcance del lector.
- `open_pairs` MUST contar los pares cuya fila nueva cumple el predicado de proyecto y cuyos dos lados están al alcance; `GET /v1/disputes` MUST aceptar `project_id`, y el puente MUST mandar el de su proyecto.
- `GET /v1/memories` MUST aceptar `reader`, que acota el listado al alcance de ese agente, junto a `agent`, que filtra por autor. `list_memories` MUST listar lo que alcanza el agente, con ` by <autor>` en cada fila ajena, y MUST aceptar un `agent` opcional que filtra por autor.
- `search_memory` y `read_memory` MUST mostrar `topic=K` en toda fila cuyo proyecto es donde iría el save, de cualquier autor.
- La línea `shared:` de `memory_status` MUST decir que el agente lee y cambia lo que alcanza.
- Siguen por agente, sin cambios, el workspace, RECENT, la sesión registrada y el aviso del hook.

**Test:** `the_workspace_stays_own_and_conflicts_are_shared`, `a_hidden_side_keeps_its_pair_out` (`prompt_builder.rs`); `open_pairs_count_by_project_and_reach` (`dispute.rs`); `disputes_lists_the_pending_pairs_an_agent_reaches_in_a_project` (`tests/api.rs`); `memory_status_counts_pending_candidates` y `memory_status_reports_the_sharing_of_the_agent` (`server.rs`); `a_list_reaches_what_its_reader_reaches` (`admin.rs`); `list_memories_names_other_authors_and_filters_by_agent` y `the_viewer_is_not_exposed_to_the_model` (`server.rs`); `a_row_of_another_agent_names_it` y `another_agents_hit_shows_its_topic_key` (`tools.rs`).

## AA8. Lo que ve el modelo

Las descripciones de `read_memory`, `update_memory`, `delete_memory`, `restore_memory`, `resolve_conflict` y `list_memories`, `SHARED_NOTE` y `protocol.md` MUST NOT decir que lo ajeno es de sólo lectura. `protocol.md` MUST decir que una clave nombra un tema del proyecto, y que guardar bajo ella reemplaza la versión de otro agente. Las instrucciones MUST seguir en 2.048 caracteres o menos.

**Test:** `tool_descriptions_say_shared_memories_can_be_changed` (`server.rs`), `the_agent_contract_matches_its_file` bendecido, y los tests de `protocol.md`.

## AA9. El doctor

`memory.topic` y `memory.content_hash` MUST agrupar por proyecto, con los mismos `check_id` y `reason_code`, y nombrar en `evidence.agents` a los autores implicados (`null` por una fila sin autor). `fallback_agent` MUST decir que esas filas se leen y se cambian, pero que el workspace y RECENT del agente real no las incluyen.

**Test:** `two_live_saves_of_one_topic_are_found` y `live_rows_with_one_hash_are_found` (`inspect.rs`), y el de las invariantes de `checks.rs`, que lee `evidence.agents`.

## AA10. V20

La migración V20 MUST reemplazar `idx_memory_dedup` por `(COALESCE(project_id, ''), content_hash, created_at)` e `idx_memory_topic` por `(COALESCE(project_id, ''), topic_key)`, con los mismos `WHERE`, y MUST NOT cambiar ninguna fila. El import MUST buscar la gemela viva y los titulares de la clave por proyecto.

#### Escenario: base de la versión anterior
- GIVEN una base en V19 con dos agentes con la misma clave viva en un proyecto, dos filas vivas con un mismo hash, `synthesis_review` por agente, una fila `disputed` con su par escalado, una conclusión y una fila sin autor
- WHEN se abre con la versión nueva
- THEN cada fila queda igual y alcanzable desde el otro agente, los índices nuevos existen y los planes de la dedup y de la clave los usan, y el primer save con la clave reemplaza a los dos titulares

**Test:** `a_store_from_before_project_scoped_rules_keeps_its_rows` (`db.rs`: filas iguales, `synthesis_review` intacta, los dos índices nuevos y el snapshot `pre-migration`); el checksum de V20 en `published_migration_checksums_are_pinned` (`migrations.rs`); los planes en `reinforce_lookup_uses_dedup_index` y `the_scope_lookup_uses_the_topic_index`; el primer save con la clave en `a_keyed_save_replaces_every_holder_of_its_project` (`worker.rs`); `an_import_merges_into_another_agents_live_text_and_key` (`import.rs`).
