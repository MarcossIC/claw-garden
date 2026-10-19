# model-download — Especificación

## Definiciones

- **Caché:** los cinco archivos del modelo en `<data>/models/multilingual-e5-small/`, cada uno con el SHA-256 fijado en `ALL_FILES`.
- **Intento:** una petición de un archivo, desde el envío hasta el último byte.
- **Recarga:** la carga que hace `IdleEmbedder` cuando una llamada necesita el modelo suelto.
- **Reparación:** la descarga de los archivos de la caché que no pasan su SHA-256, lanzada por una recarga fallida.

## R1. Plazos de red

La descarga MUST usar un cliente con un plazo de conexión de 30 s y un plazo por lectura de 60 s, que se reinicia con cada lectura y cubre también la espera de las cabeceras. MUST NOT tener un plazo total: un enlace lento que sigue mandando bytes termina. Un plazo vencido MUST terminar el intento como fallido, borrar su `.partial` y pasar al siguiente intento como cualquier otro error de red.

#### Escenario: el servidor se calla a mitad
- GIVEN un servidor que responde `200` con cabeceras y unos bytes y después no manda nada sin cerrar
- WHEN `ensure` baja ese archivo
- THEN cada intento termina al vencer el plazo de lectura, y `ensure` vuelve con un error que lo dice y sin `.partial`

#### Escenario: el servidor acepta y no contesta
- GIVEN un servidor que acepta la conexión y nunca manda cabeceras
- WHEN `ensure` baja ese archivo
- THEN cada intento termina al vencer el plazo de lectura y `ensure` vuelve con error

**Tests:** `a_body_that_stops_arriving_fails_the_attempt`, `a_server_that_never_answers_fails_the_attempt` y `the_download_waits_30_s_to_connect_and_60_s_per_read` (`model_download.rs`).

## R2. La causa del fallo

Cuando un archivo agota sus intentos, el error MUST nombrar el archivo, el host del origen, la cantidad de intentos y la causa del último: el estado HTTP (`HTTP 503 Service Unavailable`) o el error de red con su cadena de causas. MUST NOT contener la URL entera ni las credenciales que traiga. Cada intento fallido MUST dejar un `warn!` con el archivo, el número de intento y su causa. El texto llega sin cambios a `embedder_error` de `/health` y al `503 embedder_failed` de un save.

#### Escenario: un 503
- GIVEN un origen que responde `503` a todo
- WHEN `ensure` agota los intentos
- THEN el error contiene `503` y el host, y no la ruta del archivo en el origen

**Tests:** `an_exhausted_download_names_the_last_status_and_the_host` y `the_cause_never_carries_the_credentials_of_the_url` (`model_download.rs`).

## R3. La recarga nunca descarga

Una recarga MUST leer y verificar sólo la caché, sin tocar la red. Si la caché pasa, MUST cargar esos bytes como hoy. Si no pasa, la recarga MUST responder «no listo» (`EmbedError::NotReady`) sin esperar ninguna descarga, y MUST lanzar una reparación, fuera del cerrojo del modelo, salvo que ya haya una en curso: a lo sumo una a la vez. La llamada siguiente que encuentre la caché reparada MUST cargarla. Una reparación que falla MUST dejarlo en el log y la llamada siguiente MAY lanzar otra. El estado del embedder MUST seguir en `ready`.

#### Escenario: un archivo borrado del modelo suelto
- GIVEN el modelo suelto, `tokenizer_config.json` borrado de la caché y una descarga que tarda
- WHEN llegan dos búsquedas seguidas
- THEN las dos responden `503 embedder_not_ready` enseguida y hay una sola descarga en curso
- AND cuando la descarga termina, la búsqueda siguiente carga el modelo y responde

**Tests:** `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` (con una descarga que hace fallar el test si se llama) y `a_reload_with_a_damaged_cache_is_not_ready_and_repairs_it_once_in_the_background` (`bootstrap.rs`); `the_cached_model_never_touches_the_network` (`model_download.rs`).

## R4. Fuera de los hilos del runtime, un hash por archivo

Leer y hashear la caché MUST correr en una tarea bloqueante (`spawn_blocking`), no en un hilo del runtime asíncrono, en el arranque y en cada recarga. El worker de ingest MUST leer, verificar y parsear `tokenizer.json` en una tarea bloqueante. En una corrida de `ensure`, cada archivo MUST hashearse una sola vez: el que está en la caché al leerlo, el que se baja mientras llegan sus bytes, sin volver a leerlo del disco. Los bytes que recibe la carga MUST ser los mismos que se hashearon. Como el archivo bajado queda en memoria para entregarlo sin releerlo, un archivo que anuncia o trae más de 600 MB (lo que pide libre el chequeo de disco, más que cualquier archivo del modelo) MUST fallar sin reintentar y sin reservar esa memoria.

#### Escenario: falta un archivo chico
- GIVEN la caché completa salvo `config.json`
- WHEN corre `ensure`
- THEN el ONNX se lee y se hashea una vez y sólo se pide `config.json`

**Tests:** `the_cache_check_leaves_the_runtime_thread_free`, `each_file_is_hashed_once_per_run` y `a_file_larger_than_any_model_file_is_refused_without_holding_it` (`model_download.rs`); `the_worker_resolves_the_tokenizer_off_the_runtime_thread` (`runtime.rs`).

## R5. `CLAWG_HF_BASE_URL`

Sin la variable, o vacía, el origen MUST ser `https://huggingface.co/intfloat/multilingual-e5-small/resolve/main`. Con ella, MUST aceptar una URL `https://` con cualquier host y una `http://` sólo con host `localhost`, `127.0.0.1` o `[::1]`. Cualquier otro valor MUST ser un error que nombra la variable, MUST NOT caer en silencio al origen por defecto y MUST NOT hacer ninguna petición. Una `/` final MUST ignorarse. Los SHA-256 fijados MUST seguir verificándose con cualquier origen.

**Tests:** `the_base_url_defaults_to_hugging_face`, `the_base_url_takes_https_and_loopback_http` y `a_base_url_that_is_not_https_or_loopback_is_refused` (`model_download.rs`).

## R6. Una sola entrada y un solo lector

La descarga MUST tener una entrada configurable (origen, oráculo de disco, espera, plazos y manifiesto) y un solo lector con hash para la caché, sin cambiar ninguna conducta de las anteriores: el atajo de la caché sin eventos ni chequeo de disco, el chequeo de 600 MB antes de bajar, tres intentos con esperas de 5 y 30 s, un SHA-256 distinto que no gasta intentos y ningún `.partial` al terminar.

**Tests:** los que ya existen en `model_download.rs`, sin cambiar lo que comprueban.
