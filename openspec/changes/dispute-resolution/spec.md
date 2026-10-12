# dispute-resolution — Especificación

**Dominio destino:** `specs/disputes/spec.md` (nuevo; se crea al archivar).

**Alcance:** cómo sale una fila de `disputed`, qué pasa con las disputas cuando una de sus filas se borra o se reemplaza, la herramienta que usa el agente, el estado que ve el workspace y la exclusión de pedazos hermanos en el detector. El umbral, la banda y el filtro de tipo del detector no cambian. El esquema tampoco.

## Definiciones

- **Fila viva:** fila de `memory` con `deleted_at IS NULL` y `status <> 'superseded'`.
- **Disputa pendiente entre A y B:** A está `disputed` y su `disputed_against` nombra a B, y B está viva (o al revés).
- **Par enlazado:** dos filas con una disputa pendiente entre ellas, o tales que una tiene `superseded_by` apuntando a la otra.
- **Estado de reposo de una fila no reemplazada:** `disputed` si su `disputed_against` nombra al menos una fila viva; `active` si no.
- **Pedazos hermanos:** las filas que inserta un mismo evento de ingesta (un mismo save), una por pedazo (`crates/clawg-core/src/memory/worker.rs:508-546`).

## R1. Resolver con una ganadora

Resolver un par enlazado `(keep, other)` con la salida "una gana" MUST, en una sola transacción:

- dejar `other` con `status = 'superseded'` y `superseded_by = keep`;
- sacar a `keep` del `disputed_against` de `other`, y a `other` del de `keep`;
- si `keep` estaba reemplazada por `other`, poner su `superseded_by` en `NULL`;
- llevar a `keep` a su estado de reposo;
- llevar a su estado de reposo a toda fila `disputed` cuyo `disputed_against` nombre a `other` (R4);
- actualizar `last_modified_at` de las filas que cambian.

Ninguna fila MUST borrarse: `other` conserva contenido, vector y FTS.

#### Escenario: la nueva gana
- GIVEN una fila vieja O activa y una nueva N `disputed` contra O
- WHEN se resuelve `(keep = N, other = O)` con "una gana"
- THEN O queda `superseded` con `superseded_by = N`, y N queda `active` con `disputed_against` vacío

#### Escenario: la vieja gana
- GIVEN las mismas filas
- WHEN se resuelve `(keep = O, other = N)` con "una gana"
- THEN N queda `superseded` con `superseded_by = O`, y O sigue `active`

#### Escenario: quedan otras disputas
- GIVEN N `disputed` contra O1 y O2, las dos vivas
- WHEN se resuelve `(keep = N, other = O1)`
- THEN O1 queda `superseded` y N sigue `disputed`, con `disputed_against` igual a `[O2]`

#### Escenario: la reemplazada no vuelve a buscarse
- GIVEN O reemplazada por N
- WHEN se busca con una consulta igual a O
- THEN O no está entre los hits, y sí está en `list_memories` con `status = 'superseded'` y `superseded_by = N`

**Test:** `the_kept_row_wins_and_the_other_is_superseded`, `the_old_row_can_win`, `a_row_stays_disputed_while_another_dispute_is_open` y `a_superseded_row_leaves_search_but_stays_listed` en `crates/clawg-core/src/memory/dispute.rs`. Fallan hoy: la operación no existe.

## R2. Resolver con las dos válidas

Resolver un par enlazado con la salida "valen las dos" MUST sacar a cada fila del `disputed_against` de la otra, poner en `NULL` un `superseded_by` que apunte de una a la otra, y llevar a las dos a su estado de reposo. Ninguna MUST quedar `superseded` por esta operación.

#### Escenario
- GIVEN O activa y N `disputed` contra O
- WHEN se resuelve `(keep = N, other = O)` con "valen las dos"
- THEN las dos quedan `active`, y ninguna nombra a la otra

**Test:** `keeping_both_closes_the_dispute_without_superseding`.

## R3. Deshacer y validar

- Resolver un par en el que `keep` está reemplazada por `other` MUST revertir esa resolución según la salida pedida (R1 o R2). Es la vuelta atrás documentada.
- Resolver dos veces lo mismo MUST dar el mismo resultado que una sola vez.
- `keep = other` MUST fallar sin cambiar nada (error "misma fila").
- Un id que no existe o está borrado MUST fallar sin cambiar nada (error "no encontrada").
- Un par no enlazado MUST fallar sin cambiar nada (error "no están en disputa"). Lo mismo si alguna de las dos está reemplazada por una tercera fila.

#### Escenario: deshacer
- GIVEN O reemplazada por N
- WHEN se resuelve `(keep = O, other = N)` con "una gana"
- THEN O queda `active` con `superseded_by = NULL`, y N queda `superseded` con `superseded_by = O`

#### Escenario: par sin disputa
- GIVEN dos filas activas sin relación
- WHEN se resuelven entre sí
- THEN falla con "no están en disputa" y las dos siguen iguales

**Test:** `resolving_backwards_undoes_a_resolution`, `resolving_twice_is_the_same_as_once`, `a_row_cannot_be_resolved_against_itself`, `a_missing_or_deleted_row_is_not_found` y `rows_without_a_dispute_are_refused` en `dispute.rs`.

## R4. Borrar o reemplazar libera las disputas

Cuando una fila X se borra (`delete_memory_core`) o pasa a `superseded`, toda fila `disputed` cuyo `disputed_against` nombra a X MUST, en la misma transacción, volver a su estado de reposo. `disputed_against` MAY seguir nombrando a X; X ya no cuenta porque no está viva.

#### Escenario: borrar la vieja
- GIVEN O activa y N `disputed` sólo contra O
- WHEN se borra O
- THEN N queda `active`

#### Escenario: borrar una de dos
- GIVEN N `disputed` contra O1 y O2
- WHEN se borra O1
- THEN N sigue `disputed`

**Test:** `deleting_the_disputed_row_releases_its_challenger` y `deleting_one_of_two_keeps_the_dispute` en `crates/clawg-core/src/memory/admin.rs`. Fallan hoy: N queda `disputed`.

## R5. El mantenimiento arregla las huérfanas existentes

La pasada de mantenimiento MUST llevar a su estado de reposo toda fila `disputed` que no tenga ninguna fila viva en `disputed_against`, y MUST NOT tocar las demás. Una segunda pasada MUST NOT cambiar nada.

#### Escenario
- GIVEN una fila N `disputed` contra O, y O borrada como la borra la versión anterior (sólo `deleted_at`)
- WHEN corre la pasada de mantenimiento
- THEN N queda `active`, con el mismo contenido y alcanzable por búsqueda; una fila `disputed` contra una fila viva sigue igual

**Test:** `maintenance_releases_disputes_left_by_an_earlier_delete` y `maintenance_release_is_idempotent` en `crates/clawg-core/src/memory/maintenance.rs`.

## R6. HTTP y MCP

`POST /v1/disputes/resolve`, detrás del token, MUST aceptar `{"keep": <id>, "other": <id>, "keep_both": <bool, opcional, false por defecto>}` sobre filas de `memory`, y responder:

| Caso | Respuesta |
|---|---|
| Resuelto | `200` con el `id`, `status` y `superseded_by` de las dos filas |
| `keep = other` | `400` |
| Una fila no existe o está borrada | `404` |
| Par no enlazado, o una reemplazada por una tercera | `409` |

El puente MCP MUST exponer `resolve_conflict` con los mismos argumentos, y devolver el resultado o el error como texto. El bloque de conflictos de `recall_context` y las instrucciones del servidor MUST nombrar `resolve_conflict` como el paso que sigue a la respuesta del usuario.

#### Escenario
- GIVEN O activa y N `disputed` contra O
- WHEN se llama `resolve_conflict` con `keep = N`, `other = O`
- THEN el texto dice que N queda y O queda reemplazada por N, y `list_memories` lo muestra

**Test:** `resolve_supersedes_the_other_row`, `resolve_rejects_the_same_row`, `resolve_unknown_row_is_not_found` y `resolve_rows_without_a_dispute_conflict` en `crates/clawg-cli/tests/api.rs`; `resolve_conflict_reports_the_outcome` y el conteo de herramientas en `crates/clawg-cli/src/mcp/server.rs`; `the_conflict_note_names_the_tool` en `prompt_builder.rs`.

## R7. El workspace no muestra filas reemplazadas

Las consultas a `memory` del workspace MUST excluir `status = 'superseded'`. Las filas `disputed` MUST seguir apareciendo como hoy.

#### Escenario
- GIVEN dos preferencias de importancia 9 en tier 1, una reemplazada por la otra, y una tercera `disputed`
- WHEN se regenera el workspace
- THEN aparecen la ganadora y la `disputed`, y la reemplazada no

**Test:** `a_superseded_row_is_not_in_the_workspace` en `crates/clawg-core/src/memory/workspace.rs`. Falla hoy.

## R8. Los pedazos hermanos no se disputan

El detector MUST NOT devolver como disputa una fila insertada por el mismo evento que el pedazo que se compara. Para que los hermanos no ocupen el presupuesto de candidatos, la búsqueda MUST pedir tantos candidatos extra como hermanos ya insertados.

#### Escenario
- GIVEN un save explícito `semantic_fact` que se parte en dos pedazos cuyos vectores están en la banda del detector
- WHEN se ingiere
- THEN los dos pedazos quedan `active`

#### Escenario: una disputa real sigue detectándose
- GIVEN una fila vieja en la banda del segundo pedazo de un save de dos pedazos
- WHEN se ingiere
- THEN el segundo pedazo queda `disputed` contra la fila vieja, y no contra su hermano

**Test:** `sibling_chunks_of_one_save_do_not_dispute_each_other` y `a_chunk_still_disputes_an_older_row` en `crates/clawg-core/src/memory/worker.rs`. El primero falla hoy.

## Datos de la versión anterior

**Test:** `disputes_written_by_the_previous_version_resolve_and_release` en `dispute.rs`: siembra con SQL directo filas como las deja hoy el detector (`status = 'disputed'`, `disputed_against` JSON, `last_disputed_at`), con la vieja viva y con la vieja borrada sólo por `deleted_at`. Prueba que siguen alcanzables por búsqueda, que la resolución funciona sobre ellas y que el barrido libera la huérfana.
