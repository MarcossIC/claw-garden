# embedder-idle-unload — Especificación

Al archivar, estos requisitos se suman a `specs/storage` (ciclo de vida del embedder) y `specs/mcp` (el campo de `/health`).

## Definiciones

- **Tiempo de ocio:** los minutos de `CLAWG_EMBEDDER_IDLE_MINUTES`, 10 si falta o está vacía.
- **Residente:** el modelo está en memoria y una llamada no necesita cargarlo.
- **Uso:** una llamada de embedding, desde que pide el modelo (incluida su carga) hasta que vuelve.

## R1. Soltar el modelo ocioso

El daemon MUST soltar el modelo cuando pasó el tiempo de ocio desde el último uso y no hay ningún uso en curso. Un uso MUST reiniciar la cuenta. Con `CLAWG_EMBEDDER_IDLE_MINUTES=0` MUST NOT soltarlo nunca. Un valor que no es un número entero de minutos SHOULD avisar en el log y MUST usar 10, sin impedir el arranque.

#### Escenario: se suelta tras el ocio
- GIVEN un modelo residente con un tiempo de ocio de 10 minutos
- WHEN pasan 10 minutos sin usos
- THEN deja de ser residente

#### Escenario: un uso largo lo retiene
- GIVEN una llamada de embedding que dura más que el tiempo de ocio
- WHEN se cumple el tiempo de ocio durante la llamada
- THEN el modelo sigue residente, y la cuenta empieza cuando la llamada vuelve

**Tests:** `a_model_goes_only_after_the_idle_time_with_nothing_in_flight`, `an_unused_model_is_unloaded_after_the_idle_time`, `each_use_restarts_the_idle_time`, `a_call_in_flight_keeps_the_model_past_the_idle_time`, `the_idle_time_defaults_to_ten_minutes_and_zero_keeps_the_model`, `an_idle_time_that_is_not_whole_minutes_is_refused` (`idle_embedder.rs`).

## R2. Recargar a demanda, una sola vez

Una llamada con el modelo suelto MUST cargarlo con los mismos archivos verificados y la misma carga del arranque, y MUST devolver el mismo vector que antes de soltarlo. Las llamadas que llegan durante una carga MUST compartirla. `model_id`, `dimensions` y los prefijos MUST NOT cargar el modelo.

#### Escenario: varias llamadas, una carga
- GIVEN el modelo suelto
- WHEN llegan 8 llamadas a la vez
- THEN el modelo se carga una vez y las 8 responden

**Tests:** `a_call_after_an_unload_loads_the_model_again_and_gets_the_same_vector`, `calls_arriving_while_the_model_is_away_share_one_load`, `identity_and_prefixes_never_load_the_model` (`idle_embedder.rs`), `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` (`bootstrap.rs`).

## R3. Una recarga que falla no deja nada roto

Una recarga que falla MUST responder «no listo» (`EmbedError::NotReady`) y la llamada siguiente MUST intentar otra vez. Una recarga que trae otro `model_id` u otra dimensión MUST rechazarse igual: los vectores nuevos no pueden caer en otro espacio que los guardados. El estado del embedder MUST NOT pasar a `failed` por una recarga.

#### Escenario: los archivos desaparecieron
- GIVEN el modelo suelto y una recarga que falla
- WHEN llega una búsqueda
- THEN responde `503 embedder_not_ready`, y la búsqueda siguiente vuelve a intentar la carga

**Tests:** `a_failed_load_is_not_ready_and_the_next_call_tries_again`, `a_model_that_comes_back_as_another_model_is_refused` (`idle_embedder.rs`), `a_search_whose_model_cannot_load_again_answers_not_ready` (`tests/api.rs`).

## R4. Las herramientas funcionan con el modelo suelto

Buscar, guardar, restaurar y `recall_context` con el modelo suelto MUST esperar la recarga y responder como con el modelo residente. `embedder` en `/health` MUST seguir en `ready`.

**Tests:** `a_search_with_the_model_unloaded_loads_it_and_answers`, `a_save_with_the_model_unloaded_is_stored` (`tests/api.rs`).

## R5. `/health` dice si el modelo está en memoria

`GET /health` MUST incluir `embedder_resident`: `true` con el modelo residente, `false` con el modelo suelto y `null` antes de la primera carga. El cliente MUST leer un `/health` sin el campo como `None`.

**Tests:** `health_says_whether_the_model_is_in_memory` (`tests/api.rs`), `a_health_without_residency_still_parses` (`client.rs`).

## R6. Registrar un proyecto despierta el modelo

`POST /v1/projects` MUST empezar a cargar un modelo suelto y MUST responder sin esperar esa carga. Con el modelo residente MUST NOT cargar nada. Un modelo despertado MUST contar como recién usado.

#### Escenario: el puente empieza una sesión
- GIVEN el modelo suelto
- WHEN el puente registra su raíz
- THEN la respuesta llega enseguida y el modelo queda residente poco después, sin que nadie haya buscado

**Tests:** `wake_loads_the_model_without_a_caller_waiting`, `a_woken_model_gets_a_full_idle_time` (`idle_embedder.rs`), `registering_a_project_wakes_an_unloaded_model_without_waiting_for_it` (`tests/api.rs`).

## R7. La memoria vuelve al sistema

Al soltar el modelo, la memoria privada del proceso SHOULD volver a menos de 300 MB y MUST NOT crecer de una recarga a la siguiente. En Linux con glibc el hilo dueño del modelo MUST devolver al sistema las páginas libres al terminar.

**Medición:** paso 0 en Linux ([diseño, D1](design.md#d1-la-memoria-vuelve-medición)); en Windows, [verify](verify.md).
