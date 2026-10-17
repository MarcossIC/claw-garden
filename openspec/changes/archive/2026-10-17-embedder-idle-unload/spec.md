# embedder-idle-unload — Especificación

Al archivar, estos requisitos se suman a `specs/storage` (ciclo de vida del embedder) y `specs/mcp` (el campo de `/health`). R8 y R9 también van a `specs/storage`, y el requisito de pooling de `specs/storage` que hoy nombra `FastEmbedLocal` y fastembed pasa a nombrar `E5Embedder`; su test `the_e5_model_is_pooled_by_mean`, que miraba la configuración de fastembed, ya no existe y lo reemplaza `mean_pooling_matches_fastembeds_formula_bit_for_bit`.

## Definiciones

- **Tiempo de ocio:** los minutos de `CLAWG_EMBEDDER_IDLE_MINUTES`, 10 si falta o está vacía.
- **Residente:** el modelo está en memoria y una llamada no necesita cargarlo.
- **Uso:** una llamada de embedding, desde que pide el modelo (incluida su carga) hasta que vuelve.
- **Línea de base:** la memoria privada del mismo build con el embedder falso (`CLAWG_TEST_EMBEDDER=fake`) y sólo `tokenizer.json` en su directorio de datos.

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

**Tests:** `a_model_goes_only_after_the_idle_time_with_nothing_in_flight`, `an_unused_model_is_unloaded_after_the_idle_time`, `each_use_restarts_the_idle_time`, `a_call_in_flight_keeps_the_model_past_the_idle_time`, `the_idle_time_defaults_to_ten_minutes_and_zero_keeps_the_model`, `an_idle_time_that_is_not_whole_minutes_is_refused`, `the_sweeper_ends_with_the_embedder` (`idle_embedder.rs`).

## R2. Recargar a demanda, una sola vez

Una llamada con el modelo suelto MUST cargarlo con los mismos archivos verificados y la misma carga del arranque, y MUST devolver el mismo vector que antes de soltarlo. Una recarga MUST NOT volver a parsear el tokenizer ([R8](#r8-un-solo-tokenizer-por-proceso)). Las llamadas que llegan durante una carga MUST compartirla. `model_id`, `dimensions` y los prefijos MUST NOT cargar el modelo.

#### Escenario: varias llamadas, una carga
- GIVEN el modelo suelto
- WHEN llegan 8 llamadas a la vez
- THEN el modelo se carga una vez y las 8 responden

**Tests:** `a_call_after_an_unload_loads_the_model_again_and_gets_the_same_vector`, `calls_arriving_while_the_model_is_away_share_one_load`, `identity_and_prefixes_never_load_the_model` (`idle_embedder.rs`), `a_reload_does_not_parse_the_tokenizer_again` (`bootstrap.rs`), `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` (`bootstrap.rs`).

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

Con el modelo suelto, la memoria privada del daemon MUST quedar cerca de la línea de base. En 10 recargas seguidas, la mediana MUST NOT superar la línea de base en más de 64 MB, y la mediana de las recargas 6 a 10 MUST NOT superar la de las recargas 1 a 5 en más de 16 MB. Vale en Windows y en Linux. En Linux con glibc, el hilo dueño de la sesión MUST devolver al sistema las páginas libres al terminar, y `clawg serve` MUST dejar todos sus hilos en una sola arena de `malloc`, fijada antes de crear cualquier hilo ([diseño, D6](design.md#d6-una-sola-arena-de-malloc-en-linux)).

#### Escenario: diez recargas
- GIVEN `CLAWG_EMBEDDER_IDLE_MINUTES=1` y la línea de base medida en el mismo build
- WHEN diez veces una búsqueda semántica (`POST /v1/search`) carga el modelo y el tiempo de ocio lo suelta
- THEN la mediana de las diez muestras con el modelo suelto no supera la línea de base en más de 64 MB, y la mediana de las recargas 6 a 10 no supera la de las recargas 1 a 5 en más de 16 MB

#### Escenario: un parse en toda la corrida
- GIVEN la misma corrida de diez recargas
- WHEN termina
- THEN el log tiene una sola línea `tokenizer.json parsed`

**Medición:** el procedimiento de [diseño, D5, «R7 sobre lo medido»](design.md#r7-sobre-lo-medido); `only_serve_loads_the_model` (`cli.rs`) fija qué comando pide la arena, y ningún test comprueba la arena ni el trim, en Windows (`PrivateMemorySize64`) y en Linux (`RssAnon`). La búsqueda es `POST /v1/search`, nunca `clawg memories search`, que es una búsqueda por texto y no carga el modelo.

## R8. Un solo tokenizer por proceso

El daemon MUST parsear `tokenizer.json` a lo sumo una vez por proceso, y sólo de bytes cuyo SHA-256 verificó. El chunker, el índice de docs y el embedder MUST compartir esa copia. Una recarga MUST NOT volver a parsearlo. El embedder MUST NOT cambiar la configuración del tokenizer compartido.

#### Escenario: arranque con el modelo en disco, en cualquier orden
- GIVEN el modelo en disco
- WHEN el worker y la carga del modelo piden el tokenizer, en cualquier orden
- THEN se parsea una vez y los dos reciben el mismo `Arc`

#### Escenario: primer arranque
- GIVEN un directorio sin modelo
- WHEN la carga del modelo baja y verifica los archivos
- THEN la carga parsea el tokenizer y llena la celda, y el chunker sigue partiendo por bytes hasta el próximo arranque

#### Escenario: `tokenizer.json` alterado
- GIVEN un `tokenizer.json` con otro SHA-256
- WHEN el worker intenta cargarlo
- THEN no lo parsea

#### Escenario: recarga
- GIVEN el tokenizer ya parseado y el modelo suelto
- WHEN una llamada recarga el modelo
- THEN no hay ningún parse

**Tests:** `the_worker_and_the_model_load_share_one_parse`, `a_reload_does_not_parse_the_tokenizer_again` (`bootstrap.rs`), `a_tokenizer_file_with_another_sha256_is_not_parsed` (`worker.rs`).

## R9. Los mismos vectores que fastembed

Para el mismo texto y el mismo rol, el embedder MUST devolver los mismos bits que devolvió fastembed 5.13.4 en una CPU y un sistema operativo como los que generaron la referencia. ONNX Runtime elige sus kernels según la CPU y viene compilado aparte para cada sistema, así que en otra CPU o en otro sistema la diferencia máxima por componente MUST NOT pasar de 1e-6. Eso exige una ventana de `EMBED_MAX_TOKENS` que cuenta `<s>` y `</s>`, relleno hasta el texto más largo del pedido con `pad_token_id`, `token_type_ids` cuando el modelo los pide, pooling por media sobre la máscara y L2. La carga MUST rechazar (`LoadFailed`) un tokenizer con truncación o relleno propios, un token de `special_tokens_map.json` que el tokenizer no trata como especial, un `model_max_length` menor que `EMBED_MAX_TOKENS` y un modelo sin salida elegible.

#### Escenario: un texto largo conserva `</s>`
- GIVEN un texto de más de `EMBED_MAX_TOKENS` tokens
- WHEN se codifica
- THEN se corta a la ventana y `</s>` queda al final, dentro de los 512 tokens

#### Escenario: un lote de largos mezclados
- GIVEN un pedido con textos de largos distintos
- WHEN se codifica
- THEN cada texto se rellena hasta el más largo del pedido con `pad_token_id`, y ids, máscara y tipos son los de fastembed

#### Escenario: tokens especiales literales en el texto
- GIVEN un texto con `<s> </s> <pad> <unk> <mask>` escritos
- WHEN se codifica y se hace el pooling
- THEN el vector es el que dio fastembed

**Tests:** `an_encoding_matches_the_tokenizer_configured_like_fastembed`, `a_long_text_is_cut_to_the_window_and_keeps_its_closing_token`, `mean_pooling_matches_fastembeds_formula_bit_for_bit`, `the_load_refuses_a_tokenizer_with_its_own_truncation_or_padding`, `the_load_refuses_a_special_token_the_tokenizer_does_not_treat_as_special`, `the_load_refuses_a_model_window_shorter_than_embed_max_tokens`, `the_output_is_picked_like_fastembed` (`embedder.rs`), `the_vectors_are_the_ones_fastembed_gave` y `the_comparison_is_by_bits_only_on_the_cpu_that_made_the_fixture` (`crates/clawg-core/tests/e5_golden.rs`; el primero ignorado por defecto: necesita los archivos del modelo por `CLAWG_E5_MODEL_DIR` y se corre con `--ignored`).
