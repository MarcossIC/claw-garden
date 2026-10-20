# restore — Especificación

Fuente de verdad del comportamiento actual de la vuelta atrás de `delete_memory`: cómo una fila de `memory` borrada vuelve a estar viva con su mismo id, qué se rechaza, qué escribe el restore, y por dónde se lo pide (HTTP, la herramienta `restore_memory` y `clawg undelete`) y cómo se encuentran las borradas. El borrado en sí (`delete_memory_core`) y cómo libera a las filas que disputaban a la borrada están en [`specs/disputes`](../disputes/spec.md) (R4); la clave de tema, en [`specs/ingest`](../ingest/spec.md); los pedazos, en [`specs/pieces`](../pieces/spec.md); las conclusiones, en [`specs/synthesis`](../synthesis/spec.md). El ranking, el esquema y el detector de contradicciones no cambian.

**Origen:** [recoverable-delete](../../changes/archive/2026-10-18-recoverable-delete/proposal.md), archivado el 2026-10-18. Los ids de requisito son los de esa spec; la negativa por clave de tema (R1) y el paso de las disputas (R2) son lo que el código hace hoy y la spec de ese cambio no decía.

## Definiciones

- **Borrada:** una fila de `memory` con `deleted_at` no nulo, todavía en la tabla. Una fila purgada (`clawg purge`, [`specs/secrets`](../secrets/spec.md) R5) ya no está y no es una borrada.
- **Restaurar:** volver a dejar viva una borrada, con el mismo id (`restore_memory_core`, `crates/clawg-core/src/memory/admin.rs:583`).
- **Texto de embedding de una fila:** su `content`, o la cabecera de su vínculo de pedazo, el separador y su `content` cuando el vínculo tiene contexto (`embedding_text`, `crates/clawg-core/src/memory/chunker/mod.rs:55`). Es el mismo que usa el re-embed ([`specs/pieces`](../pieces/spec.md) R7). El contexto sale de `RESTORE_READ` (`admin.rs:434`).
- **Gemela viva:** una fila viva de `memory`, con `status` `active` o `disputed`, del mismo agente, el mismo proyecto (`COALESCE(project_id, '')`) y el mismo `content_hash` no nulo (`RESTORE_TWIN`, `admin.rs:442`). Es la que encontraría el dedup del save (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs`).
- **Titular de la clave:** una fila viva (`active` o `disputed`) del mismo agente y proyecto con la misma `topic_key` que la borrada, que no es un pedazo del save de la borrada (`RESTORE_TOPIC_HOLDER`, `admin.rs:453`). Un save largo con clave se la da a todos sus pedazos, así que los hermanos de un pedazo no la tienen "tomada". Tampoco la tiene una fila que el borrado de la propia fila revivió (`revived_from`), porque el restore la vuelve a reemplazar.

## R1. Qué se restaura

Restaurar `(kind, id)` con un `owner` opcional MUST rechazarse sin escribir nada cuando:

- `kind` no es `memory` (`RestoreError::OnlyMemory`, `admin.rs:382`), con un error que lo dice, distinto de "no existe";
- no hay fila con ese id, está viva, o es de otro agente cuando hay `owner`: los tres MUST dar el mismo "no existe" (`RestoreError::NotFound`, `admin.rs:386`), igual que un id purgado;
- la fila tiene una gemela viva: el error MUST nombrar el id de la gemela (`RestoreError::LiveTwin`, `admin.rs:388`);
- la fila tiene `topic_key` y su alcance (agente y proyecto) ya tiene un titular: el error MUST nombrar la clave y el id del titular (`RestoreError::TopicTaken`, `admin.rs:390`). Restaurarla dejaría dos versiones vivas de un tema que una revisión mantiene en una. Las demás piezas del propio save de la fila, y las filas con esa clave de otro agente, no cuentan;
- `redact` encuentra en su `content` un secreto de formato conocido, con las reglas de hoy y no las de cuando se guardó (`admin.rs:616`): el error MUST nombrar los tipos, nunca el secreto, y MUST nombrar `clawg purge` (`RestoreError::HoldsSecret`, `admin.rs:392`).

El vector MUST calcularse antes de la transacción, con el embedder vigente y el texto de embedding de la fila (`admin.rs:621-628`). Si el embedder falla o no está listo, MUST rechazarse sin escribir nada (`RestoreError::Embed`, `admin.rs:398`). La gemela y el titular se vuelven a mirar dentro de la transacción, junto con que la fila siga borrada y de ese dueño (`admin.rs:642-656`), así que un restore o un save que llegó entre el cálculo del vector y la transacción se ve: el segundo restore de una misma fila da "no existe".

#### Escenario: una gemela viva
- GIVEN una fila A borrada, y después el mismo texto guardado otra vez como B
- WHEN se restaura A
- THEN el error nombra a B y A sigue borrada

#### Escenario: la clave volvió a estar viva
- GIVEN una fila V1 con la clave K, borrada, y otra fila V2 del mismo agente y proyecto, viva, con K
- WHEN se restaura V1
- THEN falla, el error nombra K y V2, y V1 sigue borrada
- AND si V2 se borra, V1 se restaura

#### Escenario: un pedazo de un save con clave
- GIVEN un save de tres pedazos con la clave K, y el segundo borrado
- WHEN se restaura el segundo
- THEN vuelve, junto a sus hermanos vivos, y un save entero borrado se restaura pedazo por pedazo
- AND si otro save del agente tiene K vivo, el pedazo no vuelve

#### Escenario: un secreto
- GIVEN una fila borrada cuyo texto tiene una clave `sk-ant-…` escrita antes del enmascarado
- WHEN se restaura
- THEN el error nombra el tipo de secreto y `clawg purge`, y la fila sigue borrada

**Test:** `restoring_needs_a_deleted_row_of_the_owner`, `only_memory_rows_are_restored`, `a_live_twin_blocks_the_restore`, `a_row_whose_key_is_live_again_is_not_restored`, `a_piece_of_a_keyed_save_is_restored_beside_its_live_siblings`, `a_whole_keyed_save_is_restored_piece_by_piece`, `a_live_row_of_another_save_blocks_the_restore_of_a_piece`, `a_row_holding_a_secret_stays_deleted` y `a_failed_embedding_restores_nothing` en `crates/clawg-core/src/memory/admin.rs`. Ningún test pisa la ventana entre el vector y la transacción: la relectura dentro de la transacción la cubre sólo la lectura del código.

## R2. Qué escribe un restore

Un restore aceptado MUST, en una sola transacción, y sólo si la fila sigue borrada, sin gemela y sin titular al abrirla (`admin.rs:642-687`):

- poner `deleted_at = NULL`, `last_modified_at` en ahora y sumar 1 a `update_count` (`RESTORE_ROW`, `admin.rs:462`);
- borrar cualquier vector que la fila todavía tenga y escribir el nuevo: un delete de una versión anterior, antes de que borrara el vector en línea y de que `sweep_orphaned_vectors` (`crates/clawg-core/src/memory/maintenance.rs:181`) lo barriera, pudo dejar uno, de otro modelo;
- sumar su entrada en `memory_fts` exactamente una vez (`RESTORE_TEXT_INDEX`, `admin.rs:470`): el trigger de borrado del índice (`memory_fts_au_deleted`, `crates/clawg-core/migrations/V8__memory_fts.sql:73`) sólo resta en el paso de viva a borrada y no hay trigger de la vuelta, así que el restore escribe esa mitad, y sólo justo después de que `RESTORE_ROW` dejó viva a la fila;
- llevar a su estado de reposo a la fila (`settle_row`, `crates/clawg-core/src/memory/dispute.rs:207`) y a toda fila viva cuyo `disputed_against` la nombra (`settle_challengers`, `dispute.rs:200`), ambas en `admin.rs:672-673`: el estado de reposo de una fila que no es `superseded` es `disputed` sólo si `disputed_against` nombra una fila viva y no superseded, y `active` si no. El par escalado que el borrado liberó vuelve con ella ([`specs/disputes`](../disputes/spec.md) R4, que lo fija).

MUST NOT cambiar `content`, `content_hash`, `importance`, `tier`, `memory_type`, `access_count`, `last_accessed_at`, `superseded_by`, `memory_derivation` ni `memory_piece`, ni escribir en otra fila fuera de las que `settle_challengers` lleva a su estado de reposo, y de las filas que el restore vuelve a reemplazar por ser lo que su borrado había revivido («Restaurar la borrada deshace la revivida»), y de los pares de la redetección, que son filas nuevas en `memory_conflict`. Una fila `superseded` MUST volver `superseded` y con su `superseded_by`: está viva pero la búsqueda la deja afuera hasta que un veredicto de `resolve_conflict` la conserve.

Devuelve el id, el `status` y el `superseded_by` con que quedó la fila (`RestoredRow`, `admin.rs:409`).

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

#### Escenario: una fila reemplazada
- GIVEN una fila `superseded` por otra, con `tier`, `importance` y `access_count` propios, borrada
- WHEN se restaura
- THEN responde `status = superseded` y el `superseded_by` de antes, y lo demás sigue como estaba salvo `deleted_at`, `last_modified_at` y `update_count`

**Test:** `a_restored_row_is_found_again`, `delete_restore_delete_keeps_the_text_index_whole`, `a_stale_vector_does_not_stop_a_restore`, `a_restored_row_settles_its_own_dispute`, `a_restored_row_brings_its_escalated_pair_back` y `restoring_leaves_the_rest_of_the_row_alone` en `admin.rs`.

## R3. Pedazos y conclusiones

- Un pedazo de un save largo restaurado MUST embeberse con el contexto de su vínculo, y `read_memory` sobre el save MUST dejar de nombrarlo como faltante.
- Un restore MUST usar el embedder vigente: una fila borrada antes de un re-embed con otro modelo vuelve con un vector del modelo de hoy, nunca con uno viejo.
- Una conclusión restaurada MUST volver a cubrir a sus fuentes, y una fuente restaurada MUST quedar pendiente o cubierta según haya una conclusión viva ([`specs/synthesis`](../synthesis/spec.md) R6). No hace falta código: la síntesis mira `deleted_at`, y el restore no toca `memory_derivation`.

#### Escenario: un pedazo
- GIVEN un save de tres pedazos con el segundo borrado
- WHEN se restaura el segundo
- THEN su vector es el de la cabecera más el pedazo, y `read_memory` lo trae sin faltantes

**Test:** `a_restored_piece_is_embedded_behind_its_head_again` y `a_stale_vector_does_not_stop_a_restore` (el vector viejo es de otro modelo) en `admin.rs`; `a_restored_conclusion_covers_its_sources_again` en `crates/clawg-core/src/memory/synthesis.rs`.

## R4. HTTP

`POST /v1/memories/{kind}/{id}/restore` (ruta en `crates/clawg-cli/src/daemon/api.rs:857`, `restore_memory`, `api.rs:1469`) MUST aceptar `agent` en la query, como `DELETE`, y MUST responder:

- 200 con `restored = 1`, `id`, `status` y `superseded_by`;
- 400 si `kind` no es `memory` o no es un kind; el kind se mira antes que el embedder, así que un pedido que no puede salir bien lo dice aunque el modelo siga cargando;
- 404 por "no existe" (R1);
- 409 por gemela viva, con su id en el mensaje, por titular de la clave, con la clave y su id, y por secreto, con los tipos y `clawg purge` (`api.rs:1483`);
- 503 cuando no hay un embedder con que calcular el vector. Con el modelo todavía sin cargar, `ready_embedder` (`api.rs:1223`) responde `embedder_not_ready` (cargando) o `embedder_failed` con la causa y si se reintenta (la carga falló), como la búsqueda. Con el modelo ya cargado una vez, aunque el daemon lo haya soltado por inactivo, el embedder es el `IdleEmbedder` ([`specs/storage`](../storage/spec.md), «Las herramientas funcionan con el modelo suelto»): el restore espera la recarga y responde 200, y sólo si esa recarga falla (o vuelve un modelo de otra identidad) responde `embedder_not_ready` (`embed_failure`, `api.rs:1215`; `crates/clawg-core/src/memory/idle_embedder.rs:128-135`) y la siguiente llamada vuelve a intentar. Cualquier otro error del embedder es un 500.

`GET /v1/memories` MUST aceptar `deleted=true`: lista sólo borradas, de la más recién borrada a la más vieja (`ORDER BY deleted_at DESC, id DESC`, `admin.rs:822`), con los mismos filtros y con `deleted_at` en cada ítem. Sin `deleted`, el listado MUST NOT cambiar. `deleted=true` con `q` MUST ser 400 (`api.rs:220-225`): una borrada no tiene entrada en el índice de texto.

#### Escenario
- GIVEN una memoria guardada y borrada por HTTP
- WHEN se manda `POST .../restore?agent=<otro>`
- THEN responde 404 y sigue borrada
- WHEN se manda `GET /v1/memories?deleted=true`
- THEN aparece con su `deleted_at`
- WHEN se manda `POST .../restore?agent=<autor>`
- THEN responde 200 y una búsqueda la encuentra

**Test:** `restore_brings_a_deleted_memory_back_for_its_owner`, `restore_answers_each_refusal_with_its_status` (404, 409 por gemela, 400 por kind y 503 `embedder_not_ready` sin modelo cargado), `restore_refuses_a_row_whose_topic_is_live_again` (409 por clave) y `listing_deleted_rows_shows_only_them_newest_deletion_first` en `crates/clawg-cli/tests/api.rs`; `deleted_listing_refuses_a_text_query` junto a `list_params_from_query` en `crates/clawg-cli/src/daemon/api.rs`. Ningún test cubre el 409 por secreto por HTTP, el 503 `embedder_failed` ni un restore con el modelo suelto: lo último lo respalda lo que prueban `a_search_with_the_model_unloaded_loads_it_and_answers` y `a_search_whose_model_cannot_load_again_answers_not_ready` para la búsqueda, que usa el mismo embedder.

## R5. Herramienta MCP `restore_memory`

El puente MUST exponer `restore_memory` con `kind` e `id`, y MUST mandar siempre el agente que llama (`restore_memory_impl`, `crates/clawg-cli/src/mcp/server.rs:845`; la herramienta, `server.rs:1135`; `DaemonClient::restore`, `crates/clawg-cli/src/client.rs:996`).

- Sobre una borrada propia, MUST devolver un texto que nombra el id (`restored #N`); si sigue `superseded`, MUST decir por cuál y que la búsqueda no la muestra hasta que `resolve_conflict` la conserve, salvo que la que la reemplaza también esté borrada («Restaurar una fila reemplazada por una borrada») (`format_restore`, `crates/clawg-cli/src/mcp/tools.rs:230`).
- Sobre una de otro agente MUST fallar como si el id no existiera, y la fila MUST NOT cambiar ([`specs/visibility`](../visibility/spec.md) R3).
- Una gemela viva, un titular de la clave o un secreto MUST fallar con el mensaje del daemon. La descripción de la herramienta nombra la gemela, la clave y el secreto; el mensaje del daemon dice cuál de los tres es.
- Contra un daemon que no tiene la ruta, su 404 sin el error JSON del daemon (`404 Not Found`) MUST decir que hay que reiniciar el daemon con `clawg stop`, no que la fila no existe; cualquier otro 404 sigue siendo el "no existe" del daemon (`route_failure_text`, `server.rs:883`).
- La descripción MUST decir que deshace un `delete_memory` del propio agente, con el id y el kind que se borraron, y que lo ajeno es de sólo lectura.

`delete_memory` MUST nombrar `restore_memory` en su respuesta (`deleted N; restore_memory with the same id and kind undoes it`, `format_delete`, `crates/clawg-cli/src/mcp/tools.rs:210`, llamada desde `server.rs:838`) y en su descripción (`server.rs:1119-1124`). `tools/list` MUST listar doce herramientas; la lista completa y el orden están en [`specs/mcp`](../mcp/spec.md) (R3).

**Test:** `restore_memory_undoes_an_own_delete`, `an_agent_cannot_restore_another_agents_memory`, `delete_memory_names_the_way_back` y `restore_memory` en la lista de `tool_descriptions_say_shared_memories_are_read_only` en `crates/clawg-cli/src/mcp/server.rs`; `a_daemon_without_the_restore_route_is_named_as_such` en `server.rs`; `format_restore_says_when_the_row_is_still_superseded` en `crates/clawg-cli/src/mcp/tools.rs`; los conteos de `every_tool_is_registered_under_its_wire_name` y `the_viewer_is_not_exposed_to_the_model` (`server.rs`) y de `a_session_without_initialize_lists_the_tools_in_name_order` (`crates/clawg-cli/src/mcp/handshake.rs`). Ningún test del puente restaura con una gemela, con un titular de la clave ni con un secreto: son los de R1 y R4.

## R6. CLI

- `clawg undelete <ID> [--profile P]` MUST restaurar la fila `<ID>` de `memory` por el daemon, sin scope (reconoce las filas de cualquier agente, como `clawg purge`), e imprimir el resultado (`restored memory #N`, y si sigue reemplazada, por cuál y que la búsqueda no la muestra; `report`, `crates/clawg-cli/src/undelete.rs:9`; `run`, `undelete.rs:36`; `Command::Undelete`, `crates/clawg-cli/src/main.rs:209`). Sin daemon corriendo MUST fallar diciendo cómo arrancarlo, sin abrir la base ni arrancar un daemon: el vector lo calcula el embedder del daemon.
- `clawg memories list --deleted` MUST listar las borradas con su momento de borrado en la columna `DELETED`, de la más recién borrada a la más vieja, y nombrar `clawg undelete` sólo si hay alguna fila de `memory` en la lista (`render_deleted_table`, `crates/clawg-cli/src/memories.rs:114`; el pie, `memories.rs:190`). `clawg memories` sigue sin escribir.

**Test:** `undelete_takes_an_id_and_a_profile` y el `--deleted` de `memories_list_defaults` en `crates/clawg-cli/src/cli.rs`; `undelete_restores_through_the_daemon` y `report_names_the_row_and_a_replacement` en `crates/clawg-cli/src/undelete.rs`; `list_deleted_shows_when_each_row_was_deleted`, `list_args_ask_for_deleted_rows_only_with_the_flag` y `deleted_episodes_alone_are_not_offered_an_undelete` en `crates/clawg-cli/src/memories.rs`. Ningún test corre `clawg undelete` sin daemon.

## R7. Datos de la versión anterior

El restore no tiene migración propia: escribe sobre las columnas, el índice y los vectores que ya existían, y sobre `supersede_cause` y `revived_from` de V17 ([`specs/storage`](../storage/spec.md), «Una base en V16 abre con sus causas de reemplazo»). Filas de `memory` sembradas con SQL como las deja el `delete_memory_core` de la versión anterior (con `deleted_at`, sin vector y con su entrada FTS restada por el trigger), incluido un pedazo con vínculo y contexto, MUST poder restaurarse y MUST quedar alcanzables por la búsqueda por vector y la léxica, con el índice de texto entero.

**Test:** `rows_deleted_by_the_previous_version_can_be_restored` en `admin.rs`.

## Lo que un borrado revive y lo que un restore deshace

**Definiciones:**

- **Revivida:** fila viva que el borrado de otra devolvió a vigente. Su `revived_from` es el id de esa borrada.
- **Escondida:** fila que sigue `superseded` por una borrada porque la reemplazó un veredicto `supersedes` (`supersede_cause = 'verdict'`). Un borrado no la revive.
- **Causa del reemplazo:** `revision` cuando un save con la misma clave de tema reemplazó la fila, `verdict` cuando lo hizo un veredicto. Se escribe al reemplazar y la completa la migración V17 ([`specs/storage`](../storage/spec.md), «Una base en V16 abre con sus causas de reemplazo»).

### Borrar la vigente revive a la anterior

Borrar una fila viva (`delete_memory_core`, `crates/clawg-core/src/memory/admin.rs:316`) MUST aplicar la regla de `plan_revival` (`crates/clawg-core/src/memory/revival.rs:42`), en la misma transacción. Nada revive si la borrada no estaba `active` o `disputed`, o si otra fila viva de su alcance (agente, proyecto, clave) tiene la clave. Si no, revive el save de causa `revision` de id mayor, entero (todas sus piezas), y las demás filas de causa `revision` pasan a estar reemplazadas por la primera fila revivida. Las de causa `verdict` quedan como estaban y se nombran como escondidas.

Revivir MUST dejar la fila `active` (o `disputed` si sus disputas siguen), sin `superseded_by` ni causa, con `revived_from` igual al id borrado (`REVIVE_SQL`, `crates/clawg-core/src/memory/dispute.rs:262`), de vuelta en el KNN y en el índice de texto, y MUST llevar a su estado de reposo a la fila y a las retadoras que la disputaban antes (`settle_row` y `settle_challengers`, `dispute.rs:207` y `dispute.rs:200`). El borrado de una pieza viva de un save largo cuya primera pieza ya está borrada aplica la regla también a esa primera pieza (`revive_after_delete_of_piece`, `revival.rs:137`). Un borrado que no afectó ninguna fila MUST NOT revivir nada, y las filas de otro agente o proyecto MUST NOT revivir. `clawg purge` de una fila viva la marca borrada y aplica la misma regla antes de sacarla, en la misma transacción ([`specs/secrets`](../secrets/spec.md), R5).

La respuesta de `DELETE /v1/memories/{kind}/{id}` (`DeletedBody`, `crates/clawg-cli/src/daemon/api.rs:718`) MUST sumar `revived` (id y primera línea) y `hidden` (ids), omitidos cuando están vacíos, y `delete_memory` MUST nombrarlos (`format_delete`, `crates/clawg-cli/src/mcp/tools.rs:210`).

#### Escenario: la versión nueva se borra
- GIVEN v1 reemplazada por v2 con la clave `storage/engine`
- WHEN se borra v2
- THEN v1 está viva, la búsqueda la encuentra, y la respuesta dice que v1 volvió

#### Escenario: una cadena
- GIVEN v1 ← v2 ← v3 con una clave
- WHEN se borra v3, THEN v2 revive y v1 sigue reemplazada por v2
- WHEN en cambio se borra v2, THEN nada revive: v3 sigue vigente

#### Escenario: un veredicto
- GIVEN L reemplazada por K por un veredicto `supersedes`
- WHEN se borra K
- THEN L sigue reemplazada y la respuesta la nombra en `hidden`

**Test:** `revival_cases` (`revival.rs`); `deleting_the_current_revision_revives_the_one_it_replaced`, `deleting_the_head_of_a_chain_revives_only_the_one_before`, `deleting_a_replaced_revision_revives_nothing`, `deleting_a_verdict_winner_leaves_the_loser_hidden`, `deleting_an_undone_revision_revives_nothing`, `a_revived_save_comes_back_whole_and_the_rest_repoint`, `deleting_every_piece_of_a_long_revision_revives_the_one_before`, `a_revival_stays_in_its_scope` y `a_revived_row_is_disputed_again` en `crates/clawg-core/src/memory/admin.rs`; `deleting_the_current_revision_answers_what_it_revived` en `crates/clawg-cli/tests/api.rs`; `format_delete_names_what_came_back_and_what_stays_hidden` en `crates/clawg-cli/src/mcp/tools.rs`.

### Restaurar la borrada deshace la revivida

Restaurar una borrada D MUST NOT dar `TopicTaken` por las filas que el borrado de D revivió: la consulta de titular (`RESTORE_TOPIC_HOLDER`, `admin.rs:453`) las deja afuera. Si D no vuelve `superseded`, cada fila viva con `revived_from = D` MUST quedar otra vez `superseded` por D con causa `revision` y sin `revived_from` (`take_back_revived`, `admin.rs:558`), y la respuesta MUST listarla en `replaced`. Una fila vigente de la clave que D no revivió sigue dando `TopicTaken`.

#### Escenario
- GIVEN v1 revivida por el borrado de v2, sin otra fila viva con la clave
- WHEN se restaura v2
- THEN v2 vuelve a reemplazar a v1 (`replaced`), y v2 queda viva
- WHEN en cambio, antes de restaurar v2, se guarda v3 con la clave
- THEN v3 reemplaza a v1 (causa `revision`), y restaurar v2 da `TopicTaken`

**Test:** `restoring_the_deleted_revision_replaces_the_revived_one_again` y `a_newer_revision_still_blocks_the_restore` en `crates/clawg-core/src/memory/admin.rs`; `a_restore_answers_its_key_and_its_new_pairs` en `crates/clawg-cli/tests/api.rs`.

### Restaurar una fila reemplazada por una borrada

Restaurar una fila reemplazada por una borrada D MUST aplicar la regla a D (`revive_after_delete` dentro de `restore_memory_core`, `admin.rs:668`): la fila vuelve vigente si su causa es `revision` y la clave no tiene otra vigente. Si sigue reemplazada, la respuesta MUST decir `superseded_by_deleted` (la borrada que la reemplaza) y el texto de `restore_memory` MUST NOT mandar a `resolve_conflict`, porque ese reemplazante no está para juzgarse: restaurar D, o guardar de nuevo el contenido.

**Test:** `a_row_replaced_by_a_deleted_revision_comes_back_current` y `a_row_replaced_by_a_deleted_verdict_winner_says_so` en `admin.rs`; `format_restore_names_key_replacements_and_pairs` en `crates/clawg-cli/src/mcp/tools.rs`.

### Redetección al restaurar

Cuando la fila restaurada queda viva sin estar `superseded`, MUST compararse con el detector de un save (`detect_contradiction`, con sus reglas de banda, tipo y `needs_user`; `restore_candidates`, `admin.rs:484`) contra las filas vivas guardadas después de su `deleted_at` (`note_restored_pairs`, `admin.rs:521`). Cada candidata MUST anotarse como par `(new_id = C, old_id = A)`, en estado `pending` o `escalated`, con `INSERT OR IGNORE`, de modo que un par ya existente con su veredicto no se pisa. Un par escalado MUST sumar A a `disputed_against` de C (`add_dispute_in`, `crates/clawg-core/src/memory/dispute.rs:285`) y dejar a C `disputed`. La respuesta MUST devolver las candidatas en `related` (`RestoredRow.related`, `admin.rs:419`), y `format_restore` MUST marcar las escaladas con «asks the user».

No se anotan: las filas guardadas antes del borrado, las que ya tienen par con A (con o sin veredicto) y las que A vuelve a reemplazar. Si la fila restaurada termina `superseded`, no se escribe ningún par. Un fallo del detector se registra en el log y no devuelve ninguna candidata, como en un save (`contradiction.rs:200-206`); el restore no lo reintenta.

#### Escenario
- GIVEN una fila A borrada, y después C guardada con texto cercano, sin identificador en común
- WHEN se restaura A
- THEN queda el par `(new_id = C, old_id = A)` en `pending`, C aparece en `related` y no cambia de estado
- AND si el par escala (C y A son preferencias cercanas), C queda `disputed` con A en `disputed_against`

**Test:** `a_restored_row_is_paired_with_what_was_saved_while_deleted`, `an_escalated_pair_from_a_restore_disputes_the_newer_row` y `rows_saved_before_the_delete_or_already_paired_are_not_paired_again` en `admin.rs`; `a_restore_answers_its_key_and_its_new_pairs` en `crates/clawg-cli/tests/api.rs`.

### Barrido de mantenimiento

La pasada de mantenimiento (`run_maintenance_once_at`, `crates/clawg-core/src/memory/maintenance.rs:92`, y `revive_hidden_revisions`, `maintenance.rs:235`) MUST aplicar la regla a cada borrada que sigue `active` o `disputed` y a la que todavía apunta una fila viva de causa `revision` (`revive_hidden`, `crates/clawg-core/src/memory/revival.rs:165`). Es idempotente. Lo que revive se registra sólo en el log: `maintenance: brought back N rows a delete of the current version of their topic left hidden` (`maintenance.rs:126`).

**Test:** `maintenance_revives_what_an_earlier_delete_left_hidden` en `crates/clawg-core/src/memory/maintenance.rs`.

### Restaurar en HTTP y en la CLI

`POST /v1/memories/{kind}/{id}/restore` suma a su respuesta `topic_key` (si la fila tiene clave), `replaced`, `superseded_by_deleted` y `related`, omitidos cuando están vacíos o son falsos (`RestoredBody`, `crates/clawg-cli/src/daemon/api.rs:740`). `DaemonClient::restore` los lee con `#[serde(default)]` (`crates/clawg-cli/src/client.rs:996`), así que un daemon anterior no los manda. `clawg undelete` imprime la clave, lo que volvió a reemplazar y cada par nuevo, una línea por candidata, sin juzgarlos (`report`, `crates/clawg-cli/src/undelete.rs:9`).

**Test:** `a_restore_answers_its_key_and_its_new_pairs` en `crates/clawg-cli/tests/api.rs`; `report_names_the_row_and_a_replacement` en `crates/clawg-cli/src/undelete.rs`; `format_restore_names_key_replacements_and_pairs` en `crates/clawg-cli/src/mcp/tools.rs`.

### Filas de una versión anterior a V17

Abrir con V17 una base escrita por una versión anterior, en la que se borró la vigente de un tema, deja la versión anterior `superseded` por una fila borrada, con causa `revision` ([`specs/storage`](../storage/spec.md), «Una base en V16 abre con sus causas de reemplazo»). El primer paso de mantenimiento, `STARTUP_DELAY` después de arrancar el daemon (`crates/clawg-core/src/memory/maintenance.rs:25`), la trae de vuelta con el barrido. Esto se acepta como está diseñado: queda sólo en el log del daemon, `clawg update` no lo informa, y restaurar la borrada la vuelve a reemplazar (`revived_from`).

## Límites conocidos

- Restaurar una fila reemplazada cuyo tema tiene una vigente da `TopicTaken`: la fila sigue borrada. Es lo que impide dos vigentes de una clave.
- Revivir por veredicto no ocurre: una fila que un veredicto `supersedes` reemplazó queda escondida hasta que alguien la juzgue con `resolve_conflict`.
- Un fallo del detector en la redetección no se reintenta: esos pares no se anotan nunca.
