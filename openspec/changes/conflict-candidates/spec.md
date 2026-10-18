# conflict-candidates — Especificación

Requisitos nuevos o cambiados sobre [`specs/disputes`](../../specs/disputes/spec.md). Lo que no se nombra aquí sigue como está. Términos: un **par** es una fila de `memory_conflict`; un par está **pendiente**, **escalado** o **juzgado** según su `state`.

## C1. El detector anota candidatas

Cuando el detector encuentra filas en la banda, el insert MUST escribir un par por candidata, en la misma transacción que la fila nueva, con `new_id` = la fila nueva, `old_id` = la candidata y su `distance`. La fila nueva MUST quedar `disputed` y nombrar a la vieja en `disputed_against` sólo si el par escala (C2). Si no escala, MUST quedar `active` con el par `pending`.

#### Escenario: hechos cercanos sin identificador común
- GIVEN un `semantic_fact` guardado
- WHEN se guarda otro `semantic_fact` del mismo agente en la banda, sin un identificador compartido
- THEN las dos filas quedan `active` y hay un par `pending` con su distancia

#### Escenario: preferencia cercana
- GIVEN una `user_preference` guardada
- WHEN se guarda otra a distancia menor que `ESCALATE_BELOW`
- THEN la nueva queda `disputed` contra la vieja y el par está `escalated`

**Test:** `a_close_fact_is_a_pending_candidate_and_both_stay_active` y `a_close_preference_escalates_and_disputes` en `worker.rs`; `insert_writes_one_conflict_row_per_candidate` en `store_insert.rs`.

## C2. Escalamiento

`needs_user(new_type, old_type, distance, shares_identifier)` MUST ser pura y devolver verdadero si la distancia no se conoce, o si es menor que `ESCALATE_BELOW` (0,40) y algún lado es `user_preference` o `architectural_decision`, o los dos textos comparten un identificador (`identifier_tokens`).

**Test:** `needs_user_cases` en `contradiction.rs`, con la tabla tipo × distancia × identificador.

## C3. Un par por pareja de saves

Dentro de un pedazo, el detector MUST quedarse con la candidata más cercana de cada save (`memory_piece.save_id`). Un save nuevo MUST anotar a lo sumo un par con cada save anterior. Los pedazos hermanos siguen sin emparejarse (R8).

**Test:** `a_long_save_pairs_once_with_an_earlier_save` y `a_chunk_still_pairs_with_an_older_row` en `worker.rs`.

## C4. Veredictos

`judge_pair(keep, other, verdict, reason, owner)` sobre un par enlazado (R3 de `specs/disputes`, o con un par en la tabla) MUST:

| Veredicto | Efecto |
|---|---|
| `supersedes` | R1: `other` queda `superseded` por `keep`; el par queda `judged` |
| `compatible` | R2: las dos en reposo; el par queda `judged` |
| `scoped` | Igual que `compatible`; MUST traer un motivo no vacío, si no falla sin cambiar nada |
| `conflicts` | La más nueva nombra a la más vieja en `disputed_against` y queda `disputed`; el par queda `escalated` |

Un `supersedes` sobre un par `pending` en el que algún lado es `user_preference` o `architectural_decision` MUST fallar sin cambiar nada. Un par juzgado MUST NOT volver a `pending`. Un veredicto nuevo sobre el mismo par lo reescribe, y así se deshace (R3).

**Test:** `supersedes_keeps_the_superseding_effect_and_judges_the_pair`, `compatible_and_scoped_close_without_touching_the_ranking`, `scoped_without_a_reason_is_refused`, `conflicts_escalates_a_pending_pair`, `a_protected_pending_pair_is_not_superseded_without_the_user` y `a_pending_pair_links_the_rows` en `dispute.rs`.

## C5. HTTP y MCP

- `POST /v1/disputes/judge` MUST aceptar `{keep, other, verdict, reason?, agent_id?}` y responder como `/v1/disputes/resolve` (R6), con `400` para un veredicto desconocido o un `scoped` sin motivo y `409` para C4 rechazado.
- `POST /v1/disputes/resolve` MUST seguir aceptando `keep_both`, y guardar `compatible` o `supersedes`.
- `GET /v1/disputes?agent_id=` MUST devolver `{pending, escalated, pairs}`, con hasta 5 pares pendientes del agente, los más viejos primero, sin filas borradas ni reemplazadas.
- `resolve_conflict` MUST tomar `keep`, `other`, `verdict` y `reason`. Contra un daemon que contesta 404 en la ruta nueva, MUST decir que se reinicie el daemon.
- `memory_status` MUST contar los pares pendientes y nombrar los listados.

**Test:** `judge_records_a_verdict`, `judge_refuses_scoped_without_reason`, `resolve_with_keep_both_still_works` y `disputes_lists_pending_pairs_of_the_agent` en `crates/clawg-cli/tests/api.rs`; `resolve_conflict_reports_the_outcome`, `resolve_conflict_against_an_older_daemon_asks_for_a_restart` y `memory_status_counts_pending_candidates` en `server.rs`.

## C6. Candidatas en la respuesta del save

Un save confirmado cuyo detector anotó pares MUST listarlos en `related` (`id`, `escalated`, y la primera línea de la candidata recortada a 80 caracteres), uno por fila vieja. Sin pares, MUST NOT llevar el campo. El texto de `save_memory` MUST sumar una línea con los ids sólo cuando hay alguno. Un puente que no conoce el campo MUST mostrar el save igual.

**Test:** `a_confirmed_save_names_its_candidates` en `worker.rs`; `save_outcome_text_names_related_memories` en `server.rs`; `save_body_leaves_related_out_when_empty` en `api.rs`.

## C7. Textos

`CONFLICT_NOTE` MUST NOT afirmar que las memorias se contradicen, y MUST nombrar `resolve_conflict` y sus veredictos. La apertura del protocolo MUST pedir que se pregunte al usuario por lo que lista ACTIVE CONFLICTS, y que se juzguen solas las candidatas de la respuesta del save.

**Test:** `the_conflict_note_does_not_claim_a_contradiction` en `prompt_builder.rs`; `the_instructions_carry_the_saving_habits` en `server.rs`.

## C8. Borrar, restaurar, purgar, mantener

- Borrar una fila MUST NOT tocar sus pares (R4 sigue igual para el estado).
- Restaurar una fila MUST llevar a su estado de reposo las filas cuyo `disputed_against` la nombra: el par escalado vuelve. Esto reemplaza la última oración de R4.
- `purge` MUST borrar los pares que nombran la fila.
- La pasada de mantenimiento MUST borrar los pares que nombran una fila que no existe.

**Test:** `a_restored_row_brings_its_escalated_pair_back` en `admin.rs`; `purging_drops_the_pairs_of_the_row` en `secrets.rs`; `maintenance_drops_pairs_of_missing_rows` en `maintenance.rs`.

## C9. Datos de la versión anterior

Una base en V14 con filas `disputed` (una contra una fila viva, otra contra una fila borrada sólo por `deleted_at`) y una fila `superseded` MUST abrir en V15 con un par `escalated` por id de cada `disputed_against`, `distance = NULL`, y con las filas iguales (estado, `disputed_against`, contenido) y alcanzables por búsqueda. Un `disputed_against` que no es JSON válido MUST NOT impedir la migración.

**Test:** `a_database_with_disputes_opens_with_escalated_pairs` en `db.rs`; la fijación del checksum en `migrations.rs`.

## C10. Medición

`conflict-eval` MUST ingerir con el worker de producción (`EvalIngest`) cada par etiquetado en una base propia, y un flujo de hechos en una sola base. MUST informar, por clase, la fracción con candidata y la fracción escalada, con intervalo de Wilson, y para el flujo las candidatas y los escalados por save. Con `--fake` corre sin el modelo.

**Test:** `wilson_interval_cases` y `labelled_pairs_parse_and_cover_every_class` en `crates/clawg-eval/src/bin/conflict-eval`; `conflict_pairs_reads_the_table` en `eval_api.rs`.
