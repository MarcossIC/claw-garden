# disputes — Especificación

Fuente de verdad del comportamiento actual de los pares que encuentra el detector de contradicciones y de las disputas entre memorias: qué pares anota el detector y cuáles le pregunta al usuario, cómo se juzga un par, qué pasa con los pares cuando una de sus filas se borra, se restaura, se reemplaza o se purga, la herramienta que usa el agente, el estado que ve el workspace y la exclusión de pedazos hermanos. El umbral, la banda y los cortes de escalamiento están en «Umbral y banda del detector» y se miden con `conflict-eval` y con los pares de contradicción del golden set ([`specs/eval`](../eval/spec.md)); el filtro de tipo no está acá. Qué agente puede juzgar qué par está en [`specs/visibility`](../visibility/spec.md) (su R3 y R4).

**Origen:**
- [dispute-resolution](../../changes/archive/2026-10-13-dispute-resolution/proposal.md), archivado el 2026-10-13.
- [project-indexing](../../changes/archive/2026-10-15-project-indexing/proposal.md), archivado el 2026-10-15 (sólo "Umbral y banda del detector").
- [conflict-candidates](../../changes/archive/2026-10-18-conflict-candidates/proposal.md), archivado el 2026-10-18 (los pares de `memory_conflict`, el escalamiento, los veredictos, las candidatas en la respuesta del save y el paso de los pares por restore, purge y mantenimiento).
- [topic-revisions](../../changes/archive/2026-10-18-topic-revisions/proposal.md), archivado el 2026-10-18 (sólo R9 y la restauración de una fila con clave en R4).

## Definiciones

- **Fila viva:** fila de `memory` con `deleted_at IS NULL` y `status <> 'superseded'`.
- **Par:** una fila de `memory_conflict` (`crates/clawg-core/migrations/V15__memory_conflict.sql`): `new_id` es la fila más nueva y `old_id` la más vieja, siempre `new_id > old_id`, con la `distance` L2 que midió el detector (`NULL` en un par anterior a la tabla), su `state` y, si está juzgado, su `verdict` y su `reason`. Un par está **pendiente** (`pending`: sólo se le avisó al agente), **escalado** (`escalated`: se le pregunta al usuario) o **juzgado** (`judged`: `verdict` es `supersedes`, `compatible` o `scoped`). No hay claves foráneas, igual que en `memory_piece`: una fila de `memory` sólo se quita con purge, que quita también sus pares.
- **Disputa pendiente entre A y B:** A está `disputed`, su `disputed_against` nombra a B y B está viva (o al revés). Un par escalado es exactamente un id en el `disputed_against` de la fila más nueva; un par pendiente no aparece ahí.
- **Par enlazado:** dos filas que tienen un par en `memory_conflict`, o tales que el `disputed_against` de una nombra a la otra, o que una está `superseded` con `superseded_by` apuntando a la otra (`check_pair`, `crates/clawg-core/src/memory/dispute.rs:140`).
- **Estado de reposo de una fila no reemplazada:** `disputed` si su `disputed_against` nombra al menos una fila viva; `active` si no.
- **Fila protegida:** una `user_preference` o una `architectural_decision`.
- **Pedazos hermanos:** las filas que inserta un mismo evento de ingesta (un mismo save), una por pedazo (`ingest_pieces`, `crates/clawg-core/src/memory/worker.rs:621`).

`disputed_against` significa "disputas pendientes": cerrar una disputa saca el id de la otra fila. `last_disputed_at` no cambia al resolver. `superseded_by` guarda quién reemplazó a quién. Las columnas de `memory` son las de V1 (`crates/clawg-core/migrations/V1__init.sql`); V15 agrega la tabla de pares y no toca `memory`.

## El detector anota candidatas

Cada fila que el detector encuentra en la banda ("Umbral y banda del detector") es una candidata (`Candidate`, `crates/clawg-core/src/memory/contradiction.rs:77`): su id, su distancia, si escala y la primera línea de su texto. El insert MUST escribir un par por candidata en la misma transacción que la fila nueva (`note_candidates`, `crates/clawg-core/src/memory/store_insert.rs:247`), `escalated` si escala y `pending` si no. La fila nueva MUST quedar `disputed`, nombrando en `disputed_against` sólo a las candidatas que escalan (`store_insert.rs:514-526`); con candidatas pendientes nada más, MUST quedar `active`.

Una candidata escala cuando `needs_user` (`contradiction.rs:103`), que es pura, devuelve verdadero: si la distancia no se conoce; si es menor que `ESCALATE_BELOW` y los dos lados son filas protegidas; o si es menor que `IDENTIFIER_ESCALATE_BELOW` y los dos textos comparten un identificador (`shares_identifier`, `contradiction.rs:115`, que usa `identifier_tokens` de `search.rs`). Dos `semantic_fact` cercanos que no llegan a ese corte no escalan, ni un hecho cerca de una preferencia o una decisión: los ve el agente en la respuesta del save y en `memory_status`. El agente no puede reemplazar el lado protegido de un par pendiente sin preguntar (R3), así que una decisión no se pierde porque su par no escaló.

Dentro de un pedazo, el detector MUST quedarse con la candidata más cercana de cada save (`SaveKey`, `contradiction.rs:89`: el save de `memory_piece`, o la fila si no es un pedazo). Un save nuevo MUST anotar a lo sumo un par con cada save anterior: entre sus pedazos, gana el primero que encuentra a ese save (`worker.rs:795` y `:795`). Los pedazos hermanos siguen sin emparejarse (R8).

#### Escenario: hechos cercanos sin identificador común
- GIVEN un `semantic_fact` guardado
- WHEN se guarda otro `semantic_fact` del mismo agente en la banda, sin un identificador compartido
- THEN las dos filas quedan `active` y hay un par `pending` con su distancia

#### Escenario: preferencia cercana
- GIVEN una `user_preference` guardada
- WHEN se guarda otra a distancia menor que `ESCALATE_BELOW`
- THEN la nueva queda `disputed` contra la vieja y el par está `escalated`

#### Escenario: un hecho cerca de una decisión
- GIVEN una `architectural_decision` guardada
- WHEN se guarda un `semantic_fact` a distancia menor que `ESCALATE_BELOW`, sin un identificador compartido a menos de `IDENTIFIER_ESCALATE_BELOW`
- THEN las dos quedan `active` y el par está `pending`

**Tests:** `a_close_fact_is_a_pending_candidate_and_both_stay_active`, `a_close_preference_escalates_and_disputes`, `a_long_save_pairs_once_with_an_earlier_save` y `a_chunk_still_pairs_with_an_older_row` en `worker.rs`; `insert_writes_one_conflict_row_per_candidate` y `pending_candidates_alone_leave_the_row_active` en `store_insert.rs`; `needs_user_cases` (la tabla tipo × distancia × identificador), `escalate_below_pins_the_shipped_value`, `identifier_escalate_below_pins_the_shipped_value` e `identifiers_are_shared_only_when_both_texts_hold_one` en `contradiction.rs`.

## Las candidatas en la respuesta del save

Un save confirmado cuyo detector anotó pares MUST listarlos en `related` (`SaveBody`, `crates/clawg-cli/src/daemon/api.rs:509`; cada uno `RelatedMemory`, `api.rs:561`, con `kind`, `id`, `escalated` y la primera línea de la candidata cortada a `EXCERPT_CHARS` = 80 caracteres, `contradiction.rs:73`), uno por fila vieja. Sin pares, MUST NOT llevar el campo. El texto de `save_memory` MUST sumar una línea `close to earlier memories: #id "<línea>"` (con `, asks the user` en las que escalan) que pide juzgarlas con `resolve_conflict`, sólo cuando hay alguna (`related_text`, `crates/clawg-cli/src/mcp/server.rs:325`). Un puente que no conoce el campo muestra el save igual.

**Tests:** `a_confirmed_save_names_its_candidates` en `worker.rs`; `save_outcome_text_names_related_memories` en `server.rs`; `save_body_leaves_related_out_when_empty` en `api.rs`.

## R1. `supersedes`: una gana

Juzgar un par enlazado `(keep, other)` con `supersedes` (`judge_pair`, `dispute.rs:316`) MUST, en una sola transacción:

- dejar `other` con `status = 'superseded'`, `superseded_by = keep` y `supersede_cause = 'verdict'` (`SUPERSEDE_SQL`, `crates/clawg-core/src/memory/dispute.rs:268`, llamada desde `judge_pair` en `dispute.rs:364`); una revisión por clave escribe `revision` en su lugar (`revise_in`, `crates/clawg-core/src/memory/topic.rs:131`), así que la causa es lo que distingue a las dos ([`specs/restore`](../restore/spec.md), «Borrar la vigente revive a la anterior»);
- sacar a `keep` del `disputed_against` de `other`, y a `other` del de `keep`;
- si `keep` estaba reemplazada por `other`, poner su `superseded_by` en `NULL`;
- llevar a `keep` a su estado de reposo;
- llevar a su estado de reposo a toda fila `disputed` cuyo `disputed_against` nombre a `other` (R4);
- dejar el par `judged` con el veredicto (`RECORD_PAIR_SQL`, `dispute.rs:291`), creando su fila si no la tenía;
- actualizar `last_modified_at` de las filas que cambian.

Ninguna fila MUST borrarse: `other` conserva contenido, vector y FTS, sigue en `list_memories` y en `GET /v1/memories/memory/{id}`, y se recupera juzgando al revés (R3).

#### Escenario: la nueva gana
- GIVEN una fila vieja O activa y una nueva N `disputed` contra O
- WHEN se juzga `(keep = N, other = O)` con `supersedes`
- THEN O queda `superseded` con `superseded_by = N`, N queda `active` con `disputed_against` vacío, y el par queda `judged`

#### Escenario: la vieja gana
- GIVEN las mismas filas
- WHEN se juzga `(keep = O, other = N)` con `supersedes`
- THEN N queda `superseded` con `superseded_by = O`, y O sigue `active`

#### Escenario: quedan otras disputas
- GIVEN N `disputed` contra O1 y O2, las dos vivas
- WHEN se juzga `(keep = N, other = O1)` con `supersedes`
- THEN O1 queda `superseded` y N sigue `disputed`, con `disputed_against` igual a `[O2]`

#### Escenario: la reemplazada no vuelve a buscarse
- GIVEN O reemplazada por N
- WHEN se busca con una consulta igual a O
- THEN O no está entre los hits, y sí está en `list_memories` con `status = 'superseded'` y `superseded_by = N`

**Test:** `the_kept_row_wins_and_the_other_is_superseded`, `the_old_row_can_win`, `a_row_stays_disputed_while_another_dispute_is_open`, `a_superseded_row_leaves_search_but_stays_listed` y `supersedes_keeps_the_superseding_effect_and_judges_the_pair` en `crates/clawg-core/src/memory/dispute.rs`.

## R2. `compatible` y `scoped`: valen las dos

Juzgar un par enlazado con `compatible` o `scoped` MUST sacar a cada fila del `disputed_against` de la otra, poner en `NULL` un `superseded_by` que apunte de una a la otra, llevar a las dos a su estado de reposo y dejar el par `judged` con el veredicto y el motivo. Ninguna MUST quedar `superseded`, y el ranking de las dos no cambia. `scoped` MUST traer un motivo no vacío; sin él, MUST fallar sin cambiar nada (`ResolveError::ReasonRequired`, `dispute.rs:328-330`).

#### Escenario
- GIVEN O activa y N `disputed` contra O
- WHEN se juzga `(keep = N, other = O)` con `compatible`
- THEN las dos quedan `active`, ninguna nombra a la otra, y el par queda `judged` con `compatible`

**Test:** `keeping_both_closes_the_dispute_without_superseding`, `compatible_and_scoped_close_without_touching_the_ranking` y `scoped_without_a_reason_is_refused` en `dispute.rs`.

## `conflicts`: el usuario decide

Juzgar un par enlazado con `conflicts` MUST hacer que la fila más nueva nombre a la más vieja en `disputed_against` y quede `disputed`, y dejar el par `escalated`: pasa a ACTIVE CONFLICTS. Es el veredicto del agente que no está seguro.

**Test:** `conflicts_escalates_a_pending_pair` en `dispute.rs`.

## R3. Deshacer y validar

- Juzgar un par en el que `keep` está reemplazada por `other` MUST revertir esa resolución según el veredicto pedido (R1 o R2). Es la vuelta atrás documentada.
- Un veredicto nuevo sobre un par lo reescribe. Un par juzgado MUST NOT volver a `pending`.
- Juzgar dos veces lo mismo MUST dar el mismo resultado que una sola vez.
- `keep = other` MUST fallar sin cambiar nada (error "misma fila").
- Un id que no existe o está borrado MUST fallar sin cambiar nada (error "no encontrada").
- Un par no enlazado MUST fallar sin cambiar nada (error "no están en disputa"). Lo mismo si alguna de las dos está reemplazada por una tercera fila.
- `supersedes` sobre un par `pending` con una fila protegida MUST fallar sin cambiar nada, con un error que pide juzgarlo `conflicts` y preguntar al usuario (`ResolveError::NeedsUser`, `dispute.rs:349-354`). Sobre un par escalado sí vale: el usuario ya fue consultado.

#### Escenario: deshacer
- GIVEN O reemplazada por N
- WHEN se juzga `(keep = O, other = N)` con `supersedes`
- THEN O queda `active` con `superseded_by = NULL`, y N queda `superseded` con `superseded_by = O`

#### Escenario: par sin disputa
- GIVEN dos filas activas sin relación
- WHEN se juzgan entre sí
- THEN falla con "no están en disputa" y las dos siguen iguales

**Test:** `resolving_backwards_undoes_a_resolution`, `resolving_twice_is_the_same_as_once`, `a_row_cannot_be_resolved_against_itself`, `a_missing_or_deleted_row_is_not_found`, `rows_without_a_dispute_are_refused`, `a_pending_pair_links_the_rows`, `a_protected_pending_pair_is_not_superseded_without_the_user`, `the_old_resolve_records_its_verdict` y `every_verdict_parses_back_to_itself` en `dispute.rs`.

## R4. Borrar, reemplazar, restaurar y purgar

Cuando una fila X se borra (`delete_memory_core`, `crates/clawg-core/src/memory/admin.rs:316`) o pasa a `superseded`, toda fila `disputed` cuyo `disputed_against` nombra a X MUST volver a su estado de reposo en la misma transacción (`release_challengers`, `dispute.rs:191`). Pasa a `superseded` por un juicio (R1) o porque un save con la misma `topic_key` la reemplaza ([`specs/ingest`](../ingest/spec.md), «Un save con clave reemplaza la fila viva de su clave»): en los dos casos se liberan sus retadoras. `disputed_against` MAY seguir nombrando a X; X ya no cuenta porque no está viva. Borrar MUST NOT tocar los pares de X.

Restaurar X (`restore_memory_core`, `admin.rs:583`; el resto del restore está en [`specs/restore`](../restore/spec.md)) MUST llevar a su estado de reposo a X y a toda fila cuyo `disputed_against` la nombra (`settle_row` y `settle_challengers`, `dispute.rs:207` y `dispute.rs:200`, llamadas en `admin.rs:672-673`): el par escalado vuelve con ella.

Restaurar una fila borrada que tenía `topic_key` MUST fallar sin cambiar nada cuando el alcance de su clave ya tiene una fila viva de otro save (las demás piezas de su propio save no cuentan) (`RESTORE_TOPIC_HOLDER`, `admin.rs:453`; `RestoreError::TopicTaken`, `admin.rs:391`). El error MUST nombrar la clave y la fila que la tiene, y la fila restaurada se queda borrada: restaurarla dejaría dos versiones vivas de un tema que una revisión mantiene en una. `POST /v1/memories/memory/{id}/restore` MUST responder `409` (`restore_memory`, `crates/clawg-cli/src/daemon/api.rs:1390`, el brazo del `409` en `api.rs:1404`), como con un gemelo vivo o un secreto.

Purgar X (`purge`, `crates/clawg-core/src/memory/secrets.rs:181`) MUST borrar los pares que la nombran (`secrets.rs:232`). El `reason` de un veredicto se guarda enmascarado ([`specs/secrets`](../secrets/spec.md) R7).

#### Escenario: borrar la vieja
- GIVEN O activa y N `disputed` sólo contra O
- WHEN se borra O
- THEN N queda `active`

#### Escenario: borrar una de dos
- GIVEN N `disputed` contra O1 y O2
- WHEN se borra O1
- THEN N sigue `disputed`

#### Escenario: restaurar la vieja
- GIVEN O borrada y N `disputed` sólo contra O, liberada al borrar
- WHEN se restaura O
- THEN N vuelve a `disputed` y el par vuelve a ACTIVE CONFLICTS

#### Escenario: restaurar una fila cuya clave volvió a estar viva
- GIVEN una fila V1 con la clave K, borrada, y otra fila V2 viva con K
- WHEN se restaura V1
- THEN falla, el error nombra K y V2, y V1 sigue borrada
- AND el endpoint responde `409`

**Test:** `deleting_the_disputed_row_releases_its_challenger`, `deleting_one_of_two_keeps_the_dispute`, `a_restored_row_brings_its_escalated_pair_back` y `a_row_whose_key_is_live_again_is_not_restored` en `crates/clawg-core/src/memory/admin.rs`; `restore_refuses_a_row_whose_topic_is_live_again` en `crates/clawg-cli/tests/api.rs`; `purging_drops_the_pairs_of_the_row` en `secrets.rs`.

### Un veredicto y el borrado de su ganador

Borrar la fila que un veredicto `supersedes` dejó como ganadora (K) MUST NOT revivir a la perdedora (L): L sigue `superseded` por K, porque una decisión juzgada no se deshace por un borrado. La respuesta del borrado MUST nombrar a L en `hidden` ([`specs/restore`](../restore/spec.md), «Borrar la vigente revive a la anterior»).

#### Escenario
- GIVEN L reemplazada por K por un veredicto `supersedes`
- WHEN se borra K
- THEN L sigue `superseded` por K, la búsqueda no la muestra, y la respuesta la nombra en `hidden`

**Test:** `deleting_a_verdict_winner_leaves_the_loser_hidden` en `crates/clawg-core/src/memory/admin.rs`; `a_verdict_records_why_it_replaced` en `crates/clawg-core/src/memory/dispute.rs`.

### Redetección al restaurar

Restaurar una fila que sigue viva y no reemplazada anota los pares con las filas guardadas mientras estuvo borrada, con las reglas de `needs_user` de este spec. El par es `(new_id = C, old_id = A)`, donde C es la guardada después (la nueva) y A la restaurada: C es la que carga la disputa si el par escala, y `disputed_against` de C nombra a A. Un par ya existente con su veredicto no se pisa. El detalle y los casos que no se anotan están en [`specs/restore`](../restore/spec.md), «Redetección al restaurar».

**Test:** `a_restored_row_is_paired_with_what_was_saved_while_deleted` y `an_escalated_pair_from_a_restore_disputes_the_newer_row` en `crates/clawg-core/src/memory/admin.rs`.

## R5. El mantenimiento libera las huérfanas

La pasada de mantenimiento (`release_orphaned_disputes`, `crates/clawg-core/src/memory/maintenance.rs:222`) MUST llevar a su estado de reposo toda fila `disputed` que no tenga ninguna fila viva en `disputed_against`, y MUST NOT tocar las demás. Una segunda pasada MUST NOT cambiar nada.

Cubre las huérfanas que dejaron las versiones que borraban sin liberar. La misma pasada MUST borrar los pares que nombran una fila que ya no existe (`drop_orphaned_pairs`, `maintenance.rs:248`), que sólo deja un purge de una versión anterior a la tabla.

#### Escenario
- GIVEN una fila N `disputed` contra O, y O borrada como la borraba la versión anterior (sólo `deleted_at`)
- WHEN corre la pasada de mantenimiento
- THEN N queda `active`, con el mismo contenido y alcanzable por búsqueda; una fila `disputed` contra una fila viva sigue igual

**Test:** `maintenance_releases_disputes_left_by_an_earlier_delete`, `maintenance_release_is_idempotent` y `maintenance_drops_pairs_of_missing_rows` en `maintenance.rs`.

## R6. HTTP y MCP

Detrás del token, sobre filas de `memory` (rutas en `crates/clawg-cli/src/daemon/api.rs:804-806`):

- `POST /v1/disputes/judge` MUST aceptar `{"keep": <id>, "other": <id>, "verdict": "supersedes" | "compatible" | "scoped" | "conflicts", "reason": <opcional>, "agent_id": <opcional>}` (`judge_dispute`, `api.rs:1435`).
- `POST /v1/disputes/resolve` MUST seguir aceptando `{"keep", "other", "keep_both": <bool, opcional, false por defecto>, "agent_id"}` para los puentes anteriores, y guardar `compatible` (con `keep_both`) o `supersedes` (`resolve_dispute_core`, `dispute.rs:299`).
- `GET /v1/disputes?agent_id=` MUST devolver `{pending, escalated, pairs}`: los conteos de pares abiertos entre filas vivas del agente y hasta 5 pares pendientes (`OPEN_PAIRS_SHOWN`, `api.rs:784`), los más viejos primero (`open_pairs`, `dispute.rs:435`), cada lado con la marca de su origen (P4 de [`specs/provenance`](../provenance/spec.md)).

Las dos rutas de veredicto responden igual (`resolve_error`, `api.rs:1413`):

| Caso | Respuesta |
|---|---|
| Juzgado | `200` con el `id`, `status` y `superseded_by` de las dos filas, y el `pair` en que quedó (`judged` o `escalated`) |
| `keep = other`, un veredicto desconocido (el error lista los cuatro) o `scoped` sin motivo | `400` |
| Una fila no existe o está borrada (o, con `agent_id`, es de otro agente) | `404` |
| Par no enlazado, una reemplazada por una tercera, o `supersedes` sobre un par pendiente con una fila protegida | `409` |

El puente MCP MUST exponer `resolve_conflict` con `keep`, `other`, `verdict` (obligatorio, con el `enum` de los cuatro valores en el schema, `verdict_enum`, `server.rs:341`) y `reason` (`ResolveConflictArgs`, `server.rs:347`); el agente es el del puente. Contra un daemon que contesta un 404 sin cuerpo en `/v1/disputes/judge`, MUST decir que se reinicie el daemon con `clawg stop` (`resolve_conflict_impl`, `server.rs:904`). `memory_status` MUST sumar una línea con los pares pendientes y escalados del agente, y los pendientes listados, citados por id, con cómo juzgarlos (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:154`); contra un daemon sin la ruta, omite la línea. En cada par va primero el lado sin marca, y la marca va junto al id del otro; ACTIVE CONFLICTS marca cada lado sin reordenar (P4 de [`specs/provenance`](../provenance/spec.md)). El origen no cambia el estado de ningún par.

La NOTE de ACTIVE CONFLICTS (una vez por bloque; [`specs/retrieval`](../retrieval/spec.md)) MUST NOT afirmar que las memorias se contradicen, y MUST nombrar `resolve_conflict` y sus veredictos. La apertura del protocolo ([`specs/mcp`](../mcp/spec.md)) MUST pedir que se juzguen en la sesión las candidatas que nombra `save_memory` y que se pregunte al usuario por lo que lista ACTIVE CONFLICTS. Cada par del bloque imprime los ids de sus dos lados, que son los que recibe `resolve_conflict`.

#### Escenario
- GIVEN O activa y N `disputed` contra O
- WHEN se llama `resolve_conflict` con `keep = N`, `other = O` y `verdict = supersedes`
- THEN el texto dice que N queda y O queda reemplazada por N, y `list_memories` lo muestra

**Test:** `resolve_supersedes_the_other_row`, `resolve_rejects_the_same_row`, `resolve_unknown_row_is_not_found`, `resolve_rows_without_a_dispute_conflict`, `judge_records_a_verdict`, `judge_refuses_scoped_without_reason`, `resolve_with_keep_both_still_works` y `disputes_lists_pending_pairs_of_the_agent` en `crates/clawg-cli/tests/api.rs`; `resolve_conflict_reports_the_outcome`, `resolve_conflict_against_an_older_daemon_asks_for_a_restart`, `memory_status_counts_pending_candidates`, `the_instructions_carry_the_saving_habits` y el conteo de herramientas en `crates/clawg-cli/src/mcp/server.rs`; `the_conflict_note_names_the_tool` y `the_conflict_note_does_not_claim_a_contradiction` en `crates/clawg-core/src/memory/prompt_builder.rs`.

## R7. El workspace no muestra filas reemplazadas

Las consultas a `memory` del workspace MUST excluir `status = 'superseded'`. Las filas `disputed` MUST seguir apareciendo.

#### Escenario
- GIVEN dos preferencias de importancia 9 en tier 1, una reemplazada por la otra, y una tercera `disputed`
- WHEN se regenera el workspace
- THEN aparecen la ganadora y la `disputed`, y la reemplazada no

**Test:** `a_superseded_row_is_not_in_the_workspace` en `crates/clawg-core/src/memory/workspace.rs`.

## R8. Los pedazos hermanos no se disputan

El detector (`detect_contradiction`, `contradiction.rs:182`) MUST NOT devolver como candidata una fila insertada por el mismo evento que el pedazo que se compara. Para que los hermanos no ocupen el presupuesto de candidatos, la búsqueda MUST pedir tantos candidatos extra como hermanos ya insertados (`contradiction.rs:198`). Las filas de las que se sacó una conclusión entran como hermanas: la conclusión las reformula, no las contradice.

Con el modelo real, antes de esta regla 220 de 263 pedazos de 40 saves largos quedaban `disputed` contra sus hermanos; con ella, ninguno ([verify](../../changes/archive/2026-10-13-dispute-resolution/verify.md#hipótesis-de-los-pedazos-hermanos-con-el-modelo-real)).

Desde [long-save-context](../../changes/archive/2026-10-14-long-save-context/proposal.md), los pedazos hermanos también quedan vinculados en `memory_piece`, al final del evento, con su posición en el save ([`specs/pieces`](../pieces/spec.md) R3). La exclusión no lee ese vínculo: sigue siendo la de las filas que inserta el mismo evento. Un save es una sola afirmación de su autor, y por eso la búsqueda, `recall_context` y el detector ven un pedazo por save ([`specs/pieces`](../pieces/spec.md) R4).

#### Escenario
- GIVEN un save explícito `semantic_fact` que se parte en dos pedazos cuyos vectores están en la banda del detector
- WHEN se ingiere
- THEN los dos pedazos quedan `active` y no hay par entre ellos

#### Escenario: una fila vieja sigue emparejándose
- GIVEN una fila vieja en la banda del segundo pedazo de un save de dos pedazos, los dos `semantic_fact` sin identificador en común
- WHEN se ingiere
- THEN hay un solo par, `pending`, entre el save nuevo y la fila vieja, y ninguno entre los hermanos

**Test:** `sibling_chunks_of_one_save_do_not_dispute_each_other`, `a_chunk_still_pairs_with_an_older_row` y `a_conclusion_does_not_dispute_its_sources` en `crates/clawg-core/src/memory/worker.rs`.

## R9. Una revisión no se empareja con lo que reemplaza

El detector MUST NOT emparejar un pedazo nuevo con una fila viva del alcance de la `topic_key` del save: son las filas que ese mismo save va a dejar `superseded` al terminar ([`specs/ingest`](../ingest/spec.md), «Un save con clave reemplaza la fila viva de su clave»). Un par entre una versión y la siguiente nacería `pending` y quedaría muerto en la misma transacción que reemplaza a la vieja. Las demás filas se siguen emparejando, también las de otra clave.

El worker suma las filas del alcance a los hermanos que ya pasa al detector (`siblings`, `worker.rs:790-792`), que `detect_contradiction` deja fuera como a los pedazos de R8 y a las fuentes de una conclusión.

#### Escenario
- GIVEN un `semantic_fact` guardado con la clave K, y otro `semantic_fact` cercano sin clave
- WHEN se guarda una versión nueva con K, a una distancia de los dos dentro de la banda
- THEN hay un par con el que no tiene clave y ninguno con la versión que reemplaza

**Test:** `a_revision_does_not_pair_with_the_row_it_replaces` en `crates/clawg-core/src/memory/worker.rs`.

## Umbral y banda del detector

El detector (`detect_contradiction`, `contradiction.rs:182`) MUST devolver como candidata una fila de `memory`, del mismo agente y de un tipo que afirma algo, cuya distancia L2 al vector nuevo (ambos unitarios) cae en la banda `[NEAR_DUPLICATE_FLOOR, CONTRADICTION_THRESHOLD)` (`contradiction.rs:215-216`):

| Constante | Valor | Qué separa |
|---|---|---|
| `CONTRADICTION_THRESHOLD` (`contradiction.rs:37`) | 0,42 | Por debajo, dos textos dicen casi lo mismo y pueden contradecirse; por encima, son compatibles |
| `NEAR_DUPLICATE_FLOOR` (`contradiction.rs:45`) | 0,10 | Por debajo, son la misma afirmación (mayúsculas, espacios interiores, una edición mínima) que el hash de contenido no atrapó: no se anota |
| `ESCALATE_BELOW` (`contradiction.rs:63`) | 0,40 | Dentro de la banda, por debajo un par de dos filas protegidas escala al usuario |
| `IDENTIFIER_ESCALATE_BELOW` (`contradiction.rs:70`) | 0,30 | Por debajo, una candidata cuyos dos textos comparten un identificador escala al usuario |

Los cuatro valores son de L2 sobre vectores del pooling mean del modelo ([`specs/storage`](../storage/spec.md)); cambiar el pooling los mueve. Una cota en compilación exige `NEAR_DUPLICATE_FLOOR < CONTRADICTION_THRESHOLD` (`contradiction.rs:50`).

**Cómo se midieron.** Con el golden set de memorias (`docs/eval/golden.json`, 46 casos, 28 pares de contradicción), con pooling mean, el 2026-10-15: la curva del barrido de [`specs/eval`](../eval/spec.md) da 0 falsos positivos sobre los pares no contradictorios hasta L2 0,50, recall 0,8 en 0,40 y 0,9 en 0,45. Los pares escritos a mano como proxy (diez que se contradicen y diez compatibles) quedan en (0,350, 0,469]: la última contradicción en 0,350 y el primer par compatible en 0,469, así que 0,42 queda con margen por debajo de él y `ESCALATE_BELOW` = 0,40 entre los dos. `NEAR_DUPLICATE_FLOOR` sale del mismo proxy: el par de una oración y ella misma con un doble espacio queda a L2 0,097, y el piso lo deja justo por encima. Esa calibración mide la distancia sola; el detector entero (el filtro de tipo, el piso, el colapso por save y el escalamiento) lo mide `conflict-eval` ([`specs/eval`](../eval/spec.md)). Los dos cortes de escalamiento salen de ahí, el 2026-10-18: con un solo corte de 0,40 para todo, el 23,4 % de los saves de un flujo de hechos del mismo proyecto escalaba por compartir identificadores (`agent_id`, `memory_type`), con esos pares entre 0,34 y 0,40, y en un store real un hecho cerca de una nota de estado guardada como decisión escalaba igual que dos decisiones. Con `IDENTIFIER_ESCALATE_BELOW` = 0,30 y la regla de los dos lados protegidos, el flujo baja al 0,7 % y las contradicciones protegidas del set etiquetado siguen escalando 9 de 9.

El detector sigue viendo sólo las filas del que guarda (`Reach::OwnOnly`, `contradiction.rs:178`, [`specs/sharing`](../sharing/spec.md)) y no consulta el canal léxico (`lexical: None`).

**Tests:** `contradiction_threshold_pins_the_shipped_value`, `near_duplicate_floor_pins_the_shipped_value`, `escalate_below_pins_the_shipped_value` e `identifier_escalate_below_pins_the_shipped_value` (fijan los literales), `hit_below_near_duplicate_floor_is_excluded`, `detect_contradiction_near_vector_returns_stored_id`, `detect_contradiction_far_vectors_returns_empty` y `user_preference_contradiction_detected_at_in_band_distance` en `crates/clawg-core/src/memory/contradiction.rs`.

## Datos de la versión anterior

Las filas `disputed` que escribió una versión anterior (`status = 'disputed'`, `disputed_against` JSON, `last_disputed_at`) MUST seguir alcanzables por búsqueda, juzgarse con R1 a R3 y liberarse con R5.

Una base en V14 MUST abrir en V15 con un par `escalated` por cada id de cada `disputed_against`, con `distance = NULL` y `created_at` igual al `last_disputed_at` de la fila (`V15__memory_conflict.sql:30-34`), y con las filas iguales (estado, `disputed_against`, contenido) y alcanzables por búsqueda: lo que ya estaba en ACTIVE CONFLICTS sigue ahí, porque una distancia que no se guardó siempre escala. Un `disputed_against` que no es JSON válido no nombra nada y MUST NOT impedir la migración. Una disputa que una versión anterior cerró ya no está en `disputed_against`, así que no tiene par y no se puede volver a juzgar.

**Test:** `disputes_written_by_the_previous_version_resolve_and_release` en `dispute.rs`, que siembra esas filas con SQL directo, con la vieja viva y con la vieja borrada sólo por `deleted_at`; `a_database_with_disputes_opens_with_escalated_pairs` en `crates/clawg-core/src/memory/db.rs`, que siembra V14 con una fila `disputed` contra una viva, otra contra una borrada sólo por `deleted_at` y una `superseded`.

## Rollback

Un binario anterior a V15 MUST NOT abrir una base en V15: la rechaza sin tocarla con `SchemaNewer`, que nombra `clawg restore` ([`specs/storage`](../storage/spec.md), «Verificación del esquema antes de migrar»). Para volver a él hay dos caminos: restaurar la copia `pre-migration` que se escribió antes de V15 (se pierde lo guardado después), o borrar la tabla con `DROP TABLE memory_conflict; DELETE FROM refinery_schema_history WHERE version = 15;`. Con lo segundo, el binario anterior ve los pares escalados como filas `disputed` y los pendientes como filas `active`, igual que si el detector no los hubiera visto.

Para recuperar una fila reemplazada sin el puente basta `UPDATE memory SET status='active', superseded_by=NULL WHERE id=?`.
