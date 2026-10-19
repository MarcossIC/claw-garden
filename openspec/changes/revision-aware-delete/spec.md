# revision-aware-delete — Especificación

Requisitos nuevos sobre [`specs/restore`](../../specs/restore/spec.md), [`specs/disputes`](../../specs/disputes/spec.md), [`specs/storage`](../../specs/storage/spec.md), [`specs/ingest`](../../specs/ingest/spec.md) y [`specs/mcp`](../../specs/mcp/spec.md). Lo que no se nombra acá sigue como está. Términos: una fila **viva** no está borrada y su `status` es `active` o `disputed`; la **vigente** de una clave es la fila viva de su alcance `(agent_id, COALESCE(project_id, ''), topic_key)`; una fila **reemplazada por D** está `superseded` con `superseded_by = D`.

## R1. Las columnas

V17 MUST agregar `memory.supersede_cause` (`revision` | `verdict`, nullable) y `memory.revived_from` (INTEGER, nullable) con un índice parcial sobre las no nulas, y MUST completar `supersede_cause` de cada fila `superseded` como dice el [diseño](design.md#d2-la-causa-de-las-filas-que-ya-existen). MUST NOT cambiar ninguna otra columna.

#### Escenario: base de la versión anterior
- GIVEN una base en V16 con una revisión por clave, un veredicto `supersedes` entre dos filas sin clave, una revisión deshecha por veredicto (misma clave, par `supersedes` juzgado), piezas, pares e índice de texto
- WHEN se abre con la versión nueva
- THEN la revisión queda `revision`, los dos veredictos `verdict`, las demás filas sin causa ni `revived_from`, y cada fila, par, vínculo y posting del índice de texto igual

**Test:** `a_database_from_before_revivals_opens_with_its_supersede_causes` (`db.rs`); el checksum de V17 en `published_migration_checksums_are_pinned`.

## R2. La causa se escribe al reemplazar

Una revisión por clave MUST dejar `supersede_cause = 'revision'`; un veredicto `supersedes`, `verdict`. Una fila revivida (por un veredicto o por un borrado) MUST quedar sin causa.

**Test:** `a_revision_records_why_it_replaced` (`worker.rs`); `a_verdict_records_why_it_replaced` (`dispute.rs`).

## R3. La regla

`plan_revival` MUST ser pura y decidir, para una fila borrada D y las filas reemplazadas por D: no revivir nada si D no estaba viva o si la clave tiene otra vigente; si no, revivir el save de causa `revision` de id mayor entero, y reapuntar las demás de causa `revision` a su primera fila. Las de causa `verdict` MUST quedar como están y nombrarse como escondidas.

**Test:** `revival_cases` (`revival.rs`).

## R4. Borrar la vigente revive a la anterior

En la transacción del borrado de una fila `memory` viva, MUST aplicarse R3. Cada fila revivida MUST quedar `active` (o `disputed` si sus disputas siguen), sin `superseded_by` ni causa, con `revived_from = D`, de vuelta en el KNN y el índice de texto, y sus retadoras MUST volver a disputarla. La respuesta MUST nombrar las revividas (`revived`, con su primera línea) y las escondidas (`hidden`). Un borrado que no afectó ninguna fila MUST NOT revivir nada. Borrar la última pieza viva de un save largo cuya primera pieza ya está borrada MUST aplicar R3 a esa primera pieza. Las filas de otro agente o proyecto MUST NOT revivir.

#### Escenario: la versión nueva se borra
- GIVEN v1 reemplazada por v2 con la clave `storage/engine`
- WHEN se borra v2
- THEN v1 está viva, la búsqueda la encuentra, y la respuesta dice que v1 volvió

#### Escenario: una cadena
- GIVEN v1 ← v2 ← v3 con una clave
- WHEN se borra v3, THEN v2 revive y v1 sigue reemplazada por v2
- WHEN en cambio se borra v2, THEN nada revive: v3 sigue vigente

#### Escenario: un veredicto
- GIVEN L reemplazada por K por un veredicto `supersedes`, o v2 reemplazada por v1 al deshacer una revisión
- WHEN se borra el ganador
- THEN la perdedora sigue reemplazada y la respuesta la nombra en `hidden`

**Test:** `deleting_the_current_revision_revives_the_one_it_replaced`, `deleting_the_head_of_a_chain_revives_only_the_one_before`, `deleting_a_replaced_revision_revives_nothing`, `deleting_a_verdict_winner_leaves_the_loser_hidden`, `deleting_an_undone_revision_revives_nothing`, `a_revived_save_comes_back_whole_and_the_rest_repoint`, `deleting_every_piece_of_a_long_revision_revives_the_one_before`, `a_revival_stays_in_its_scope` y `a_revived_row_is_disputed_again` (`admin.rs`).

## R5. Restaurar la borrada deshace la revivida

Restaurar D MUST NOT dar `TopicTaken` por una fila que el borrado de D revivió. Si D no vuelve `superseded`, cada fila viva con `revived_from = D` MUST quedar otra vez `superseded` por D con causa `revision` y sin `revived_from`, y la respuesta MUST listarla en `replaced`. Una fila vigente de la clave que no revivió el borrado de D MUST seguir dando `TopicTaken`.

**Test:** `restoring_the_deleted_revision_replaces_the_revived_one_again` y `a_newer_revision_still_blocks_the_restore` (`admin.rs`).

## R6. Restaurar una fila reemplazada por una borrada

Restaurar una fila reemplazada por una D borrada MUST aplicar R3 a D: por revisión y sin vigente, la fila vuelve vigente. Si sigue reemplazada, la respuesta MUST decir `superseded_by_deleted`, y el texto MUST NOT mandar a `resolve_conflict`.

**Test:** `a_row_replaced_by_a_deleted_revision_comes_back_current` y `a_row_replaced_by_a_deleted_verdict_winner_says_so` (`admin.rs`).

## R7. Redetección al restaurar

Restaurar A sin que vuelva `superseded` MUST comparar A con las filas vivas guardadas después de su `deleted_at`, con el detector de un save (`detect_contradiction`, sus reglas de banda, tipo y `needs_user`). Cada candidata MUST anotarse como par `(new_id = C, old_id = A)`, `pending` o `escalated`, y la respuesta MUST devolverla en `related`. Un par escalado MUST dejar a C `disputed` con A en su `disputed_against`. Una fila guardada antes del borrado, una que ya tiene par con A (con o sin veredicto) y una que A vuelve a reemplazar (R5) MUST NOT anotarse.

**Test:** `a_restored_row_is_paired_with_what_was_saved_while_deleted`, `an_escalated_pair_from_a_restore_disputes_the_newer_row` y `rows_saved_before_the_delete_or_already_paired_are_not_paired_again` (`admin.rs`).

## R8. Barrido

El paso de mantenimiento MUST aplicar R3 a cada fila borrada que sigue `active` o `disputed` y a la que apunta una fila viva de causa `revision`, y MUST ser idempotente.

**Test:** `maintenance_revives_what_an_earlier_delete_left_hidden` (`maintenance.rs`).

## R9. HTTP, MCP y CLI

- `DELETE /v1/memories/{kind}/{id}` MUST sumar `revived` y `hidden`, omitidos vacíos; `POST …/restore` MUST sumar `topic_key`, `replaced`, `superseded_by_deleted` y `related`, omitidos vacíos o falsos. El cliente MUST leerlos con `#[serde(default)]`.
- `delete_memory` MUST nombrar las revividas y las escondidas; `restore_memory` MUST mostrar `topic=K`, lo que volvió a reemplazar, el reemplazante borrado y cada par nuevo; `clawg undelete` lo mismo.
- La descripción de `delete_memory` MUST decir que borrar la vigente de una clave trae la versión anterior; la de `restore_memory` MUST nombrar el rechazo por clave.

**Test:** `deleting_the_current_revision_answers_what_it_revived` y `a_restore_answers_its_key_and_its_new_pairs` (`crates/clawg-cli/tests/api.rs`); `format_delete_names_what_came_back_and_what_stays_hidden` y `format_restore_names_key_replacements_and_pairs` (`tools.rs`); `report_names_the_row_and_a_replacement` (`undelete.rs`); `the_agent_contract_matches_its_file`.
