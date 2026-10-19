# recoverable-delete — Especificación

**Dominio destino:** `specs/restore/spec.md` (nuevo; se crea al archivar). Al archivar, el conteo de herramientas de `specs/mcp` (R2 y R3: "once") pasa a doce; `specs/visibility` R3 y R4 suman `restore_memory` y su ruta; `specs/disputes` R4 cambia "un undelete futuro" por un enlace a R4 de acá.

**Alcance:** cómo vuelve a estar viva una fila de `memory` borrada con `delete_memory`, qué se rechaza, y cómo se encuentran las borradas. El ranking, el esquema y el detector de contradicciones no cambian.

## Definiciones

- **Borrada:** una fila de `memory` con `deleted_at` no nulo, todavía en la tabla. Una fila purgada (`clawg purge`) ya no está y no es una borrada.
- **Restaurar:** volver a dejar viva una borrada, con el mismo id.
- **Texto de embedding de una fila:** su `content`, o la cabecera de su vínculo de pedazo, el separador y su `content` cuando el vínculo tiene contexto (`embedding_text`, `crates/clawg-core/src/memory/chunker/mod.rs:48`). Es el mismo que usa el re-embed ([`specs/pieces`](../../../specs/pieces/spec.md) R7).
- **Gemela viva:** una fila viva de `memory`, con `status` `active` o `disputed`, del mismo agente, el mismo proyecto (`COALESCE(project_id, '')`) y el mismo `content_hash` no nulo. Es la que encontraría el dedup del save (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs:236-243`).

## R1. Qué se restaura

Restaurar `(kind, id)` con un `owner` opcional MUST rechazarse sin escribir nada cuando:

- `kind` no es `memory` (un error que lo dice, distinto de "no existe");
- no hay fila con ese id, está viva, o es de otro agente cuando hay `owner`: los tres MUST dar el mismo "no existe", igual que un id purgado;
- la fila tiene una gemela viva: el error MUST nombrar el id de la gemela;
- `redact` encuentra en su `content` un secreto de formato conocido: el error MUST nombrar los tipos, nunca el secreto, y MUST nombrar `clawg purge`.

El vector MUST calcularse antes de la transacción, con el embedder vigente y el texto de embedding de la fila. Si el embedder falla o no está listo, MUST rechazarse sin escribir nada.

#### Escenario: una gemela viva
- GIVEN una fila A borrada, y después el mismo texto guardado otra vez como B
- WHEN se restaura A
- THEN el error nombra a B y A sigue borrada

#### Escenario: un secreto
- GIVEN una fila borrada cuyo texto tiene una clave `sk-ant-…` escrita antes del enmascarado
- WHEN se restaura
- THEN el error nombra el tipo de secreto y `clawg purge`, y la fila sigue borrada

**Test:** `restoring_needs_a_deleted_row_of_the_owner`, `a_live_twin_blocks_the_restore`, `a_row_holding_a_secret_stays_deleted`, `only_memory_rows_are_restored` y `a_failed_embedding_restores_nothing` en `crates/clawg-core/src/memory/admin.rs`. Fallan hoy: no existe el restore.

## R2. Qué escribe un restore

Un restore aceptado MUST, en una sola transacción, y sólo si la fila sigue borrada y sin gemela al abrirla:

- poner `deleted_at = NULL`, `last_modified_at` en ahora y sumar 1 a `update_count`;
- borrar cualquier vector que la fila todavía tenga y escribir el nuevo (un delete de antes de la poda en línea puede haber dejado uno, `sweep_orphaned_vectors`, `crates/clawg-core/src/memory/maintenance.rs:170`);
- sumar su entrada en `memory_fts` exactamente una vez;
- recalcular su estado de reposo si no es `superseded`: `disputed` sólo si `disputed_against` nombra una fila viva, `active` si no ([`specs/disputes`](../../../specs/disputes/spec.md) R4).

MUST NOT cambiar `content`, `content_hash`, `importance`, `tier`, `memory_type`, `access_count`, `last_accessed_at`, `superseded_by`, `memory_derivation` ni `memory_piece`, ni ninguna otra fila. MUST NOT volver a disputar las filas que se liberaron al borrarla.

Devuelve el id, el `status` y el `superseded_by` con que quedó la fila.

#### Escenario: vuelve a la búsqueda
- GIVEN una fila con la palabra "zircon" en su texto, borrada
- WHEN se restaura
- THEN la búsqueda por vector la encuentra, la búsqueda léxica por "zircon" la encuentra, y `update_count` subió en 1

#### Escenario: borrar, restaurar, borrar
- GIVEN una fila viva
- WHEN se borra, se restaura y se vuelve a borrar
- THEN el índice FTS queda igual que con un solo borrado, y su chequeo de integridad pasa

#### Escenario: una disputa que ya no está
- GIVEN una fila `disputed` contra otra, y las dos borradas
- WHEN se restaura la primera
- THEN queda `active`, y la otra sigue borrada

**Test:** `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_row_settles_its_own_dispute`, `restoring_leaves_the_rest_of_the_row_alone` en `admin.rs`. Fallan hoy.

## R3. Pedazos y conclusiones

- Un pedazo de un save largo restaurado MUST embeberse con el contexto de su vínculo, y `read_memory` sobre el save MUST dejar de nombrarlo como faltante.
- Un restore MUST usar el embedder vigente: una fila borrada antes de un re-embed con otro modelo vuelve con un vector del modelo de hoy, nunca con uno viejo.
- Una conclusión restaurada MUST volver a cubrir a sus fuentes, y una fuente restaurada MUST quedar pendiente o cubierta según haya una conclusión viva ([`specs/synthesis`](../../../specs/synthesis/spec.md) R6). No hace falta código: la síntesis mira `deleted_at`.

#### Escenario: un pedazo
- GIVEN un save de tres pedazos con el segundo borrado
- WHEN se restaura el segundo
- THEN su vector es el de la cabecera más el pedazo, y `read_memory` lo trae sin faltantes

**Test:** `a_restored_piece_is_embedded_behind_its_head_again` y `a_stale_vector_does_not_stop_a_restore` (el vector viejo es de otro modelo) en `admin.rs`; `a_restored_conclusion_covers_its_sources_again` en `crates/clawg-core/src/memory/synthesis.rs`. El primero falla hoy; el segundo también, porque no hay restore que llamar.

## R4. HTTP

`POST /v1/memories/{kind}/{id}/restore` MUST aceptar `agent` en la query, como `DELETE`, y MUST responder:

- 200 con `restored = 1`, `id`, `status` y `superseded_by`;
- 400 si `kind` no es `memory` o no es un kind;
- 404 por "no existe" (R1);
- 409 por gemela viva, con su id en el mensaje, y por secreto, con los tipos y `clawg purge`;
- 503 si el embedder no está listo, con el mismo cuerpo que la búsqueda.

`GET /v1/memories` MUST aceptar `deleted=true`: lista sólo borradas, de la más recién borrada a la más vieja, con los mismos filtros. Sin `deleted`, el listado MUST NOT cambiar. `deleted=true` con `q` MUST ser 400: una borrada no tiene entrada en el índice de texto.

#### Escenario
- GIVEN una memoria guardada y borrada por HTTP
- WHEN se manda `POST .../restore?agent=<otro>`
- THEN responde 404 y sigue borrada
- WHEN se manda `GET /v1/memories?deleted=true`
- THEN aparece con su `deleted_at`
- WHEN se manda `POST .../restore?agent=<autor>`
- THEN responde 200 y una búsqueda la encuentra

**Test:** `restore_brings_a_deleted_memory_back_for_its_owner`, `restore_answers_each_refusal_with_its_status` y `listing_deleted_rows_shows_only_them_newest_deletion_first` en `crates/clawg-cli/tests/api.rs`; `deleted_listing_refuses_a_text_query` junto a `list_params_from_query` en `crates/clawg-cli/src/daemon/api.rs`. Fallan hoy.

## R5. Herramienta MCP `restore_memory`

El puente MUST exponer `restore_memory` con `kind` e `id`, y MUST mandar siempre el agente que llama.

- Sobre una borrada propia, MUST devolver un texto que nombra el id; si sigue `superseded`, MUST decir por cuál y que la búsqueda no la muestra.
- Sobre una de otro agente MUST fallar como si el id no existiera, y la fila MUST NOT cambiar.
- Una gemela viva o un secreto MUST fallar con el mensaje del daemon.
- Contra un daemon que no tiene la ruta, su 404 sin el error JSON del daemon MUST decir que hay que reiniciar el daemon, no que la fila no existe.
- La descripción MUST decir que deshace un `delete_memory` del propio agente, con el id y el kind que se borraron.

`delete_memory` MUST nombrar `restore_memory` en su respuesta y en su descripción. `tools/list` MUST listar doce herramientas.

**Test:** `restore_memory_undoes_an_own_delete`, `an_agent_cannot_restore_another_agents_memory`, `delete_memory_names_the_way_back` y `restore_memory` en `tool_descriptions_say_shared_memories_are_read_only` en `crates/clawg-cli/src/mcp/server.rs`; `a_daemon_without_the_restore_route_is_named_as_such` en `server.rs`; `format_restore_says_when_the_row_is_still_superseded` en `crates/clawg-cli/src/mcp/tools.rs`; los conteos de `every_tool_is_registered_under_its_wire_name`, `the_viewer_is_not_exposed_to_the_model` (`server.rs`) y `a_session_without_initialize_lists_the_tools_in_name_order` (`handshake.rs`). Fallan hoy.

## R6. CLI

- `clawg undelete <ID> [--profile P]` MUST restaurar la fila `<ID>` de `memory` por el daemon, sin scope, e imprimir el resultado. Sin daemon corriendo MUST fallar diciendo cómo arrancarlo, sin abrir la base: el vector lo calcula el embedder del daemon.
- `clawg memories list --deleted` MUST listar las borradas con su momento de borrado, y nombrar `clawg undelete` sólo si hay alguna fila de `memory` en la lista. `clawg memories` sigue sin escribir.

**Test:** `undelete_takes_an_id_and_a_profile` y el `--deleted` de `memories_list_defaults` en `crates/clawg-cli/src/cli.rs`; `undelete_restores_through_the_daemon` y `report_names_the_row_and_a_replacement` en `crates/clawg-cli/src/undelete.rs`; `list_deleted_shows_when_each_row_was_deleted` y `list_args_ask_for_deleted_rows_only_with_the_flag` y `deleted_episodes_alone_are_not_offered_an_undelete` en `crates/clawg-cli/src/memories.rs`. Fallan hoy.

## R7. Datos de la versión anterior

Filas de `memory` sembradas con SQL como las deja el `delete_memory_core` de la versión anterior (con `deleted_at`, sin vector y con su entrada FTS restada por el trigger), incluido un pedazo con vínculo y contexto, MUST poder restaurarse y MUST quedar alcanzables por la búsqueda por vector y la léxica.

**Test:** `rows_deleted_by_the_previous_version_can_be_restored` en `admin.rs`. Falla hoy.
