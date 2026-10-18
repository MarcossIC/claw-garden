# topic-revisions — Especificación

Requisitos nuevos sobre [`specs/ingest`](../../../specs/ingest/spec.md), [`specs/pieces`](../../../specs/pieces/spec.md), [`specs/update`](../../../specs/update/spec.md) y [`specs/mcp`](../../../specs/mcp/spec.md). Lo que no se nombra aquí sigue como está. Términos: la **clave** de un save es su `topic_key`; una fila **viva** no está borrada y su `status` es `active` o `disputed`; el **alcance** de una clave es `(agent_id, COALESCE(project_id, ''), topic_key)`.

## T1. La columna

La migración V16 MUST agregar `memory.topic_key` (TEXT, nullable) y un índice parcial sobre `(agent_id, COALESCE(project_id, ''), topic_key)` para las filas no borradas con clave. MUST NOT cambiar ninguna fila existente.

#### Escenario: base de la versión anterior
- GIVEN una base en V15 con filas, piezas y pares
- WHEN se abre con la versión nueva
- THEN cada fila queda igual y con `topic_key = NULL`, y la tabla de pares y las piezas siguen iguales

**Test:** `a_database_from_before_topic_keys_opens_unchanged` en `db.rs`; el checksum de V16 en `published_migration_checksums_are_pinned`.

## T2. Formato de la clave

`validate_topic_key` MUST ser pura y aceptar sólo segmentos no vacíos de `[a-z0-9._-]` separados por `/`, de 1 a 80 caracteres en total. El error MUST nombrar `topic_key`.

**Test:** `topic_key_cases` en `topic.rs`.

## T3. La clave entra con el save

- `POST /v1/memories` MUST aceptar `topic_key` opcional, rechazar una inválida con `400` antes de escribir la outbox, y devolverla en el cuerpo de la respuesta (`201` o `202`).
- `MemoryEvent.topic_key` MUST ser `#[serde(default)]`: un evento de la outbox escrito sin el campo se lee como un save sin clave.
- Sólo un save explícito (`MemorySource::Explicit`) MUST llevar la clave a sus filas.

**Test:** `a_save_refuses_an_invalid_topic_key` y `a_save_echoes_its_topic_key` en `crates/clawg-cli/tests/api.rs`; `an_event_without_a_topic_key_still_parses` en `events.rs`.

## T4. Revisión

Un save explícito con clave cuyas piezas se guardaron todas MUST, en la transacción que termina el save:

1. dejar `superseded`, con `superseded_by` = la primera fila del save, cada fila viva del mismo alcance que no es pieza del save (salvo T6);
2. liberar las filas que disputaban a las reemplazadas (`release_challengers`);
3. dar la clave a las filas del save que no tenían ninguna;
4. llevar a las filas que insertó el save el menor tier y el mayor `access_count` entre ellas y las reemplazadas.

El informe del save MUST listar las reemplazadas en `replaced` (id y primera línea). Un save con clave sin filas anteriores MUST comportarse como uno sin clave, salvo que sus filas la llevan. Un save que falla antes de terminar MUST NOT reemplazar nada.

#### Escenario: una decisión revisada
- GIVEN una `architectural_decision` guardada con la clave `storage/engine`
- WHEN el mismo agente guarda otro texto con esa clave en el mismo proyecto
- THEN hay una sola fila viva con la clave, la nueva; la vieja queda `superseded` por la nueva, y ni KNN ni el índice de texto la devuelven

#### Escenario: mismo texto
- GIVEN una fila viva con la clave K
- WHEN se guarda el mismo texto con K
- THEN la fila se refuerza y no se reemplaza nada

#### Escenario: volver al texto anterior
- GIVEN la versión 1 de K reemplazada por la versión 2
- WHEN se guarda otra vez el texto de la versión 1 con K
- THEN se inserta una fila nueva que reemplaza a la versión 2; la versión 1 sigue `superseded`

**Test:** `a_keyed_save_replaces_the_live_row_of_its_key`, `the_same_text_under_its_key_only_reinforces`, `saving_an_earlier_text_again_is_another_revision`, `a_revision_inherits_tier_and_accesses`, `a_revision_releases_the_rows_disputing_the_old_one`, `a_failed_revision_replaces_nothing` y `a_reinforced_row_without_a_key_takes_it`, `a_save_without_a_key_replaces_nothing` y `a_revision_is_finished_with_its_outbox_row` en `worker.rs`.

## T5. Tipo e importancia

Si el save con clave no trae `memory_type` o `importance`, las filas nuevas MUST tomar los de la fila viva más antigua del alcance.

**Test:** `a_revision_keeps_the_type_and_importance_it_does_not_name` en `worker.rs`.

## T6. Piezas

Un save largo con clave MUST reemplazar todas las filas vivas del alcance que no son piezas suyas. Una fila del alcance que `memory_piece` enlaza a un save cuya primera pieza es una fila viva sin esa clave (un save que la dedup le sumó a otro) MUST quedar viva, sin clave.

**Test:** `a_long_revision_replaces_every_piece_of_the_old_save` y `a_piece_shared_with_another_save_is_detached` en `worker.rs`.

## T7. Detector

El detector MUST NOT emparejar una pieza nueva con una fila viva de su mismo alcance. Las demás filas se siguen emparejando.

**Test:** `a_revision_does_not_pair_with_the_row_it_replaces` en `worker.rs`.

## T8. Alcance y orden

- Otro agente u otro proyecto con la misma clave MUST NOT reemplazar ni heredar nada.
- Dos saves con la misma clave en cola MUST aplicarse en orden: la fila viva final es la del segundo.

**Test:** `a_key_is_scoped_to_its_agent_and_project` y `two_queued_revisions_apply_in_order` en `worker.rs`.

## T9. Restaurar

`restore` de una fila borrada con clave MUST fallar sin cambiar nada cuando el alcance ya tiene otra fila viva, y nombrarla; la API MUST responder `409`.

**Test:** `a_row_whose_key_is_live_again_is_not_restored` en `admin.rs`.

## T10. MCP y listado

- `save_memory` MUST tomar `topic_key` y su schema MUST describir el formato. El texto del save MUST nombrar cada fila reemplazada con su primera línea. Si el daemon no devuelve `topic_key` cuando se mandó, MUST decir que se guardó sin reemplazar y que se reinicie el daemon.
- `list_memories` MUST mostrar `topic=K` en una fila con clave y `by=#N` en una `superseded`.
- `protocol.md` MUST decir cuándo usar la clave. La apertura MUST seguir en 2048 caracteres o menos.

**Test:** `save_outcome_text_names_the_replaced_memories`, `a_key_sent_to_an_older_daemon_is_reported` en `server.rs`; `rows_show_their_key_and_who_replaced_them` en `tools.rs`; `the_agent_contract_matches_its_file`.
