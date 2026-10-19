# session-handoff — Especificación

## Definiciones

- **Id de sesión:** el `session_id` que el host manda por stdin a cada hook.
- **Registro:** lo que el daemon recuerda, en memoria, del último id de sesión de cada par (agente, proyecto).
- **Resumen visible:** un `session_summary` vivo (`active` o `disputed`, no borrado) del proyecto pedido (o sin proyecto, si no se pide ninguno) cuyo autor es el agente que pide o uno cuyas memorias `sharing.json` le deja leer.

## R1. Registro de sesiones

`POST /v1/sessions` con `{agent_id, project_id?, session_id}` MUST recordar `session_id` para `(agent_id, project_id)` y responder `204`. Un registro nuevo para el mismo par MUST reemplazar al anterior; otro agente u otro proyecto MUST NOT verse afectados. Un `session_id` vacío, de más de 128 caracteres o con caracteres fuera del ASCII visible MUST ser un `400`. El registro MUST NOT escribirse en el store y MAY perderse al reiniciar el daemon.

#### Escenario: dos sesiones seguidas
- GIVEN el registro `s1` para (`a`, `p`) y luego `s2` para el mismo par
- WHEN `a` guarda en `p`
- THEN la fila lleva `s2`; un save de `b` en `p` o de `a` en `q` lleva `NULL`

**Tests:** `a_session_registry_keeps_the_latest_per_agent_and_project` (`daemon/sessions.rs`), `a_session_id_is_checked` (`daemon/sessions.rs`), `a_save_carries_the_registered_session` (`tests/api.rs`).

## R2. El save sella la sesión

Al aceptar un save, el daemon MUST poner en el evento el id registrado para su `(agent_id, project_id)`, o ninguno si no hay registro, antes de escribirlo en el outbox. Cada fila que el save inserta, todos sus pedazos, MUST llevar ese id en `source_session_id`. Un evento sin el campo (un outbox anterior) MUST guardarse con `NULL`. `save_memory` MUST NOT tener un argumento de sesión.

#### Escenario: sin hooks
- GIVEN un daemon sin registro
- WHEN llega un save
- THEN la fila tiene `source_session_id` `NULL` y se lee, busca y muestra como antes

**Tests:** `an_event_without_a_session_reads_as_none` (`events.rs`), `a_save_writes_its_session_on_every_row` (`worker.rs`), `a_save_carries_the_registered_session`, `a_queued_save_keeps_its_session_in_the_outbox`, `rows_without_a_session_stay_reachable` (`tests/api.rs`), `the_agent_contract_matches_its_file` (sin cambios).

## R3. El bloque LAST SESSION

Todo prompt de `POST /v1/context`, con consulta o sin ella, MUST llevar, si existe, el resumen visible más nuevo (por `created_at`, un save largo por su pedazo de menor posición) bajo `=== LAST SESSION ===`, después del workspace y antes de RECENT, en el formato de línea de los hits, con ` by <author>` cuando es de otro agente. No depende de la consulta ni del modelo. MUST NOT registrar accesos. Si el resumen más nuevo es también un hit, MUST NOT repetirse en RECENT ni en LONG-TERM. Sin resumen visible MUST NOT haber bloque.

#### Escenario: la sesión siguiente
- GIVEN dos `session_summary` de `a` en `p`, de ayer y de hace un mes
- WHEN `a` pide el contexto de `p` con una consulta que no se les parece
- THEN el prompt trae el de ayer bajo LAST SESSION y no el de hace un mes

#### Escenario: otro agente
- GIVEN el resumen más nuevo de `p` guardado por `b`
- WHEN `a` pide el contexto y `sharing.json` no oculta `b` a `a`
- THEN el bloque lo trae marcado `by b`; si lo oculta, el bloque trae el de `a` o no aparece

**Tests:** `the_last_session_block_shows_the_newest_summary`, `the_last_session_block_respects_sharing`, `the_last_session_block_is_not_repeated_as_a_hit`, `a_context_without_a_query_opens_with_the_last_session` (`prompt_builder.rs`), `a_reach_admits_the_reader_and_whoever_is_not_hidden` (`sharing.rs`).

## R4. El presupuesto

Mientras el prompt pasa del presupuesto, el bloque MUST cortarse después de LONG-TERM y de RECENT y antes que los pares de conflicto: primero se acorta con ` [truncated]` y se va cuando no queda texto, y lo cortado MUST nombrarse en `=== BUDGET ===` como los demás hits.

**Tests:** `the_last_session_block_gives_way_after_the_hits` (`prompt_builder.rs`).

## R5. Los hooks registran la sesión

`clawg hook session-start` y `clawg hook user-prompt-submit` MUST, cuando la entrada trae `session_id`, registrarlo para su `--agent` y el proyecto de `cwd`. Una falla del registro MUST NOT cambiar la salida ni el código de salida del hook.

#### Escenario: la sesión empieza
- GIVEN un daemon real
- WHEN corre `clawg hook session-start --agent a` con `session_id` `s1` y el `cwd` del proyecto, y después `a` guarda en ese proyecto
- THEN la fila lleva `s1`

**Tests:** `the_hook_input_tolerates_unknown_and_missing_fields` (`hook.rs`, ahora con `session_id`), `a_hook_registers_the_session_of_the_saves` (`tests/binary.rs`).

## R6. La evaluación lee el bloque

`rag-eval --agent-view` MUST contar como mostrado un hit de LAST SESSION, igual que uno de RECENT o LONG-TERM.

**Tests:** `the_last_session_block_counts_as_shown` (`agent_view/parse.rs`).
