# context-budget — Especificación

Al archivar, R1 a R5 se suman a `specs/retrieval` (el armado de `recall_context`) y R6 y R7 a `specs/mcp` (la petición, la respuesta y la herramienta).

## Definiciones

- **Presupuesto:** el `budget` de la petición, en bytes del prompt (unos 4 por token). Default `8000`, máximo `24000`.
- **Prompt:** el texto `prompt` de `POST /v1/context`, sin lo que el puente le agrega.
- **Cortado:** un hit que el presupuesto dejó afuera; **recortado:** uno cuyo texto se acortó.

## R1. El prompt cumple el presupuesto

El prompt MUST NOT superar el presupuesto, salvo cuando el workspace y el bloque `BUDGET` solos lo superan (R4). Para entrar, MUST quitar primero los hits de LONG-TERM desde el último, después los de RECENT desde el último, y después los pares de conflictos desde el último, que MUST sumarse a `(+N conflictos más sin mostrar)`. El workspace MUST NOT recortarse. Sin recorte, el prompt MUST ser el de hoy.

#### Escenario: muchas filas largas
- GIVEN diez hits de 2.000 bytes y un presupuesto de 8.000
- THEN el prompt ocupa como mucho 8.000 bytes, y los hits que quedan son los primeros en orden de prioridad

#### Escenario: sin recorte
- GIVEN un contexto de 3.000 bytes y el presupuesto default
- THEN el prompt es byte a byte el de antes del cambio

**Tests:** `the_prompt_never_exceeds_the_budget`, `the_cut_starts_at_the_end_of_long_term`, `a_context_that_fits_is_left_as_it_was`, `conflicts_go_after_every_hit` (`prompt_builder.rs`), `a_store_of_long_rows_fits_the_budget` (`prompt_builder.rs`, con el store).

## R2. El primer hit de un bloque se recorta en lugar de saltarse

Cuando el hit a quitar es el único que queda en su bloque, su texto MUST acortarse hasta que el prompt entre, en un límite de carácter UTF-8, y terminar en ` [truncated]`. Sólo si no le queda texto MUST quitarse.

**Tests:** `the_first_hit_of_a_block_is_shortened_not_dropped`, `a_shortened_text_ends_on_a_character_boundary`.

## R3. Lo cortado se nombra por id

Si algo se cortó o recortó, el prompt MUST terminar con un bloque `=== BUDGET ===` que dice el presupuesto, cuántos bytes ocupa el contexto entero, los ids cortados y los recortados (con su kind si no es `memory`) y que `read_memory` los devuelve enteros. Sin recorte, el bloque MUST NOT aparecer. El bloque MUST NOT quitarse para entrar.

#### Escenario: pedir lo que quedó afuera
- GIVEN un contexto que cortó el hit #40
- WHEN el agente llama `read_memory` con 40
- THEN recibe su texto entero

**Tests:** `what_the_budget_left_out_is_named_by_id`, `the_budget_block_stays_with_the_smallest_budget`, `a_hit_cut_by_the_budget_is_readable_whole` (`server.rs`).

## R4. Un workspace más grande que el presupuesto se entrega y se dice

Si el workspace y el bloque `BUDGET` solos superan el presupuesto, el prompt MUST llevar el workspace entero, ningún hit, y el bloque `BUDGET` con cuántos bytes ocupa el contexto entero.

**Test:** `a_workspace_over_the_budget_is_kept_and_said`.

## R5. Un conflicto no cita lo que no se imprimió

Los conflictos MUST pedirse sobre los hits candidatos y el workspace, antes del recorte. Un lado MUST citarse por `#id` sólo si su hit quedó en el prompt; si no, MUST llevar su texto. Los accesos MUST registrarse sólo para los hits que quedaron en el prompt.

**Tests:** `a_conflict_on_a_cut_hit_carries_its_text`, `a_hit_cut_by_the_budget_is_not_counted_as_accessed`.

## R6. `budget` en la petición

`POST /v1/context` MUST aceptar `budget` opcional: ausente, MUST usar 8.000; mayor que 24.000, MUST usar 24.000; `0` o un valor que no es un entero positivo MUST responder `400`. `recall_context` MUST aceptar `budget` y pasarlo; el cliente MUST NOT mandarlo cuando no se le dio.

**Tests:** `context_takes_a_budget_and_clamps_it`, `context_refuses_a_zero_budget` (`tests/api.rs`), `recall_context_passes_its_budget` (`server.rs`).

## R7. El contexto dice si buscó memorias

`POST /v1/context` MUST responder `retrieval` con `state`: `searched`, `embedder_not_ready` (el modelo carga, o una recarga falló), `embedder_failed` (con `detail` y `retryable`) o `search_failed` (con `detail`), leído de la misma fuente que `POST /v1/search`. MUST seguir respondiendo `200` con el workspace. `recall_context` MUST empezar con una frase que dice que las memorias no se buscaron y por qué cuando `state` no es `searched`, haya workspace o no; con el prompt vacío, la frase MUST reemplazar `no context yet`. Sin el campo (un daemon viejo), MUST NOT agregar ninguna frase.

#### Escenario: el modelo carga
- GIVEN un daemon con el embedder en `loading` y un workspace con preferencias
- WHEN el agente llama `recall_context`
- THEN la respuesta empieza diciendo que las memorias no se buscaron porque el modelo carga, y después viene el workspace

**Tests:** `context_says_whether_memories_were_searched` (`tests/api.rs`, los cuatro estados), `a_context_without_retrieval_still_parses` (`client.rs`), `recall_says_when_memories_were_not_searched`, `an_old_daemon_gets_no_invented_notice` (`tools.rs`), `the_retrieval_outcome_names_each_silence` (`prompt_builder.rs`).
