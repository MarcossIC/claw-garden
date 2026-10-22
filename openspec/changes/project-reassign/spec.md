# project-reassign — Especificación

Suma requisitos a [`specs/update`](../../specs/update/spec.md), [`specs/project`](../../specs/project/spec.md), [`specs/mcp`](../../specs/mcp/spec.md), [`specs/visibility`](../../specs/visibility/spec.md) y [`specs/storage`](../../specs/storage/spec.md). Lo que no se nombra aquí sigue como está.

## Definiciones

- **Movimiento:** cambiar el `project_id` de filas de `memory` a un **destino**, que es un id de proyecto o «sin proyecto».
- **Unidad:** el cierre de una fila: las filas de todo save de `memory_piece` que la contiene, las que reemplazó con causa `revision` y la que la reemplazó con esa causa, las que un borrado suyo revivió (`revived_from`) y la que revivió por el suyo, repetido hasta que no entra ninguna más. Borradas y `superseded` incluidas. Una fila sin vínculos es su propia unidad.
- **Fila viva:** `deleted_at IS NULL` y `status IN ('active', 'disputed')`.
- **Fila protegida:** la de [`specs/update`](../../specs/update/spec.md): `memory_type` `user_preference` o `architectural_decision`.
- **Lector:** el de [`specs/visibility`](../../specs/visibility/spec.md); sin lector, la llamada es del usuario y alcanza toda fila.
- **`scope` de destino:** el que el CHECK de `memory` exige: una fila con agente conserva el suyo; una sin agente queda `project` con un destino y `global` sin él.

## PR1. Mover cambia el proyecto de una unidad entera y nada más

Un movimiento MUST cambiar, en una sola transacción, `project_id` al destino y `scope` al de destino de cada fila de cada unidad aceptada, y MUST NOT cambiar ninguna otra columna de `memory` (ni `last_modified_at`, ni `update_count`, ni `status`), ni `memory_vectors`, ni `memory_fts`, ni `memory_conflict`, ni `memory_derivation`, ni `memory_piece`. No MUST necesitar el embedder.

#### Escenario: sin proyecto a uno
- GIVEN una fila sin proyecto que se encuentra desde A y desde B
- WHEN se mueve a B
- THEN se encuentra desde B, por vector, por texto y por identificador, y no desde A

#### Escenario: de un proyecto a ninguno
- WHEN una fila de A pasa a sin proyecto
- THEN se encuentra desde A, desde B y sin proyecto

#### Escenario: la unidad entera
- GIVEN un save de tres piezas, un segundo save que comparte una pieza con el primero, y una clave con dos versiones
- WHEN se mueve una pieza o la versión vieja
- THEN se mueven las filas de los dos saves, o las dos versiones, y las no nombradas figuran en `pulled_in`

#### Escenario: una fila sin autor
- GIVEN una fila sin agente con `scope = 'project'`
- WHEN pasa a sin proyecto, y después vuelve a un proyecto
- THEN queda `global`, y después `project`

**Test:** `a_row_without_a_project_moves_into_one`, `moving_to_no_project_shows_the_row_from_every_project`, `a_row_without_an_agent_keeps_the_scope_check`, `the_pieces_of_a_save_move_together`, `two_saves_that_share_a_row_move_together`, `a_move_leaves_vectors_and_the_text_index_alone` (`relocate.rs`); `scope_after_a_move_follows_the_check` (`relocate.rs`, puro).

## PR2. Las reglas que dependen del proyecto siguen valiendo después

Después de mover, la cadena de una clave, la dedup, el revivir, LAST SESSION y la revisión de conclusiones MUST comportarse en el destino como si las filas se hubieran guardado ahí.

#### Escenario: la cadena se mueve con su vigente
- GIVEN una clave con su vigente y la versión que reemplazó, en A
- WHEN se mueven a B y después se borra la vigente
- THEN la versión anterior vuelve viva en B

#### Escenario: un save posterior en el destino
- WHEN después de mover se guarda en B el mismo texto, o un save bajo la clave
- THEN refuerza la fila movida, o la reemplaza

#### Escenario: un resumen de sesión
- WHEN una `session_summary` sin proyecto pasa a B y es la más nueva viva de B
- THEN es la LAST SESSION de B

#### Escenario: una fuente reemplazada
- WHEN se mueve una fila `superseded` que es fuente de una conclusión
- THEN `conclusions_to_review` da lo mismo antes y después

**Test:** `a_revision_chain_moves_with_its_head_and_deleting_the_head_still_revives`, `after_a_move_a_save_of_the_same_text_reinforces_the_moved_row`, `after_a_move_a_save_under_its_key_revises_it`, `a_session_summary_moved_into_a_project_is_its_last_session`, `moving_a_superseded_source_does_not_make_its_conclusions_reappear_for_review` (`relocate.rs`).

## PR3. Lo que rechaza una unidad

Una unidad MUST rechazarse, sin escribir ninguna de sus filas, cuando:

- una fila viva del destino al alcance del lector, fuera de las que se mueven, tiene el `content_hash` de una fila viva de la unidad (`live_twin`, nombrando las dos);
- una fila viva del destino al alcance del lector, fuera de las que se mueven, tiene la `topic_key` de una fila viva de la unidad (`topic_taken`, nombrando la clave y su titular);
- una unidad aceptada antes en el mismo movimiento lleva al destino ese hash o esa clave (los mismos motivos, nombrando la fila de esa unidad): se mueve la primera, en orden de id;
- sus filas están en más de un proyecto (`spans_projects`);
- tiene una fila fuera del alcance del lector (`out_of_reach`, sin nombrarla).

Una unidad que ya está en el destino MUST quedar `unchanged` y no registrarse.

#### Escenario: un choque no cambia nada
- GIVEN una fila de A y otra viva de B con el mismo texto, o con la misma clave
- WHEN se mueve la de A a B
- THEN el movimiento se rechaza nombrando la de B, y ninguna fila ni el registro cambian

#### Escenario: dos del mismo lote
- GIVEN dos filas sin proyecto con el mismo texto
- WHEN un lote las mueve a B
- THEN se mueve la de id menor y la otra se omite nombrándola

**Test:** `a_live_row_of_the_same_text_in_the_target_refuses_the_move`, `a_live_holder_of_the_topic_key_in_the_target_refuses_the_move`, `two_selected_saves_that_would_collide_in_the_target_move_only_the_first`, `a_unit_across_projects_is_refused`, `a_hidden_row_in_the_unit_refuses_it_without_naming_it`, `a_clash_is_judged_within_the_readers_reach` (`relocate.rs`); `decide_*` (`relocate.rs`, puros).

## PR4. Filas borradas y pares

Una fila borrada MUST moverse con su unidad sin chocar, y un `restore` posterior MUST validar en el proyecto nuevo. Los pares abiertos (`pending`, `escalated`) entre una fila movida y una que queda en otro proyecto MUST nombrarse en el reporte (`split_pairs`) y quedar como están; `disputed` se conserva.

**Test:** `a_deleted_row_moves_with_its_save_and_is_restored_into_the_new_project`, `a_pair_left_across_projects_is_reported` (`relocate.rs`).

## PR5. El destino

El daemon MUST rechazar con 404, sin escribir, un destino que no está en `project`. «Sin proyecto» siempre es un destino válido. El puente y `clawg projects move --to PATH` MUST registrar la raíz de la ruta antes (sin `index_docs`), y MUST rechazar una ruta cuya raíz no es un directorio; una raíz cuyo id es de otra MUST ser el 409 `project_root_collision` de siempre.

**Test:** `an_unregistered_target_is_refused` (`relocate.rs`); `a_move_to_an_unregistered_project_is_404` (`tests/api.rs`); `update_memory_moves_a_save_to_the_project_of_a_path` y `update_memory_refuses_a_path_that_is_not_a_directory` (`server.rs`).

## PR6. El registro y la vuelta atrás

Cada fila que un movimiento cambia, por cualquier puerta, MUST dejar en `memory_move` el número del movimiento, la fila, el proyecto y el `scope` de origen y de destino y el momento, en la misma transacción. Deshacer el movimiento N MUST mover de vuelta, con las reglas de PR1 a PR4 y sin exigir que el origen esté registrado, las filas de N que siguen en el destino que N anotó, y MUST ser a su vez un movimiento registrado. Una fila de N que se movió otra vez después MUST quedar donde está. Un N que no existe MUST ser un 404.

**Test:** `every_moved_row_is_logged_and_the_log_undoes_it`, `undoing_leaves_a_row_that_moved_again` (`relocate.rs`); `a_database_from_the_previous_version_opens_with_an_empty_move_log` (`db.rs`).

## PR7. Un ensayo dice lo mismo que el movimiento

Con `dry_run` el movimiento MUST responder el mismo reporte que daría sin él, salvo `move_id` y `snapshot`, y MUST NOT escribir nada ni tomar un snapshot.

**Test:** `a_dry_run_reports_what_a_move_does_and_writes_nothing` (`relocate.rs`).

## PR8. `PATCH` mueve un save

`PATCH /v1/memories/memory/{id}` MUST aceptar `project_id` (un destino) o `no_project: true`, no los dos (400), solos o con `importance` y `memory_type`, todo en una transacción: un rechazo de cualquiera no escribe nada. En `episodic` y `procedural` un destino MUST ser un 400. Una unidad rechazada por PR3 MUST ser un 409 cuyo mensaje nombra el motivo y la fila, y un destino no registrado un 404. Una unidad con una fila protegida viva MUST pedir `user_answered`, como sacarle el tipo (409 `NeedsUser` sin él). La respuesta MUST sumar el `project_id` en que quedó la fila (`null` sin proyecto), y, cuando movió, `move_id` y las otras filas que se movieron con ella (`pulled_in`). `?agent=` nombra al lector como en todo `PATCH`.

**Test:** `a_move_and_a_change_of_importance_are_one_write`, `a_protected_row_leaves_its_project_only_on_the_users_word`, `a_patch_needs_a_change` (`admin.rs`); `patch_moves_a_whole_save`, `patch_with_a_clash_is_409_and_changes_nothing`, `patch_with_both_destinations_is_400`, `patch_moves_while_the_embedder_loads` (`tests/api.rs`).

## PR9. El lote

`POST /v1/memories/move` MUST tomar `dry_run` y o bien una selección y un destino (`to_project` o `to_none`, uno solo) o bien `undo` con el número de un movimiento. La selección MUST tener al menos uno de `ids`, `from_project`, `from_none`, `agent`, `memory_type`, `source`, `session_id` o `q`; sin `include_deleted` toma filas no borradas, y `q` (la búsqueda del visor) con `include_deleted` es un 400. Antes de escribir, cuando hay algo que mover, MUST tomar un snapshot `pre-move` y nombrarlo. El lote es del usuario: no tiene lector ni pide `user_answered`, y el reporte cuenta en `protected` las filas protegidas vivas que llegaron al destino. Una unidad rechazada se informa en `skipped` y el resto se mueve.

`clawg projects move` MUST mandar ese pedido al daemon que corre (sin arrancar uno), con `--id`, `--from-project`, `--from-none`, `--here`, `--agent`, `--type`, `--source`, `--session`, `--text`, `--include-deleted`, un destino `--to PATH`, `--to-project ID` o `--to-none`, `--dry-run` y `--undo ID`, y MUST imprimir el reporte. El análisis de argumentos es puro.

**Test:** `move_takes_a_selection_and_one_target_or_an_undo` (`cli.rs`); `the_batch_moves_reports_and_snapshots`, `a_batch_without_a_selection_is_400` (`tests/api.rs`); `projects_move_moves_and_undoes_a_seeded_store` (`tests/binary.rs`).

## PR10. Listar lo que no tiene proyecto

`GET /v1/memories` MUST aceptar `no_project=true`, que lista sólo las filas sin proyecto; con `project` es un 400. `clawg memories list` y `search` MUST tomar `--no-project`.

**Test:** `a_listing_of_rows_without_a_project`, `no_project_and_project_together_are_refused` (`admin.rs`, `api.rs`); `list_and_search_take_no_project` (`cli.rs`).

## PR11. La herramienta de los agentes

`update_memory` MUST tomar `to_project_path` (una ruta cuyo proyecto es el destino) o `clear_project` (destino sin proyecto), no los dos; ausentes, el proyecto no cambia. Su descripción MUST decir para qué sirve y que mueve el save entero, y `protocol.md` MUST decir cuándo usarla. La respuesta MUST nombrar el proyecto en que quedó la fila y las otras que se movieron. `agent_contract.json` cambia sólo por eso.

**Test:** `update_memory_moves_a_save_to_the_project_of_a_path`, `update_memory_clears_the_project`, `update_memory_refuses_both_destinations` (`server.rs`); `format_update_names_where_the_row_ended` (`tools.rs`); `the_agent_contract_matches_its_file`.

## PR12. V21

V21 MUST crear `memory_move` sin tocar ninguna fila. Una base en V20 MUST abrir con el registro vacío y cada fila igual.

**Test:** `a_database_from_the_previous_version_opens_with_an_empty_move_log` (`db.rs`); `published_migration_checksums_are_pinned` (`migrations.rs`).
