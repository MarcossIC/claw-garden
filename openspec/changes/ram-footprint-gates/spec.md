# ram-footprint-gates — Especificación

RM1 cambia «Un hash por archivo, fuera de los hilos del runtime» de [`specs/storage`](../../specs/storage/spec.md). RM2 se suma a `specs/storage`, junto a ese requisito. RM3 amplía «Cada carga dice quién la pidió y cuánto costó» de `specs/storage`. RM4 se suma a `specs/storage`, junto al id del modelo en el contrato de embedding. RM5 cambia «Refresco incremental» de [`specs/project-docs`](../../specs/project-docs/spec.md), que conserva sin cambios «Sólo MUST correr un refresco por proyecto a la vez».

Lo que dicen de ONNX Runtime y de protobuf [`proposal.md`](proposal.md) y la revisión que adoptó es una predicción del código de ONNX Runtime 1.24.2, sin medir. Ningún requisito depende de ella: la puerta de más abajo la decide.

## RM1. La carga recibe los bytes que se hashearon, sin una copia en memoria

Los bytes que recibe la carga del ONNX MUST ser los mismos que se hashearon. MAY ser una vista de sólo lectura del archivo; MUST NOT ser una vista copy-on-write (`map_copy`), que cobra commit por la vista entera. Hoy `read_verified` lee el archivo a un `Vec` y lo hashea (`crates/clawg-core/src/memory/model_download.rs:605`), `check_cache` cuenta cualquier error de lectura como archivo faltante (`:577`) y `build_session` pasa el `Vec` a `commit_from_memory` (`crates/clawg-core/src/memory/embedder.rs:513-526`).

- **Windows.** El archivo MUST abrirse sin compartir escritura ni borrado y MUST seguir así desde la apertura hasta que `commit_from_memory` vuelve, haya armado la sesión o no. Entonces MUST soltarse la vista y el archivo.
- **Unix.** La guarda es el directorio de datos `0700`. clawg nunca escribe `model.onnx` en el lugar: baja a `.partial` y renombra (`commit_file`, `:875`).
- **La corrida que baja.** MUST NOT juntar el cuerpo del ONNX en memoria (hoy, `Vec::with_capacity(content_length)`, `:730`). MUST escribirlo a `.partial` hasheándolo mientras llega, hacer `sync_all`, renombrarlo y cargarlo como una recarga, desde la caché. Sólo en esa corrida el ONNX se hashea dos veces.
- **Los archivos chicos.** El tokenizer y la configuración se siguen entregando desde memoria sin releerlos, y `MAX_FILE_BYTES` (`:836`) sigue valiendo para ellos y para el límite de lo que se baja.

La frase de «Un hash por archivo» que dice que cada archivo se hashea una vez por corrida, y que el archivo bajado queda en memoria, pasa a valer sólo para los archivos chicos.

#### Escenario: la caché devuelve el ONNX como una vista
- GIVEN una caché completa con un ONNX mínimo cuyo SHA-256 se inyecta como esperado
- WHEN corre `ensure`
- THEN el ONNX verificado llega a la carga como una vista, sin copiar sus bytes, y la sesión se arma desde esa vista

#### Escenario: un byte cambiado
- GIVEN el mismo `model.onnx` con un byte cambiado
- WHEN corre `ensure`
- THEN el archivo se rechaza y se pide de nuevo

#### Escenario: la carga retiene el archivo (sólo Windows)
- GIVEN una carga que tiene el archivo
- WHEN otro handle intenta abrirlo para escribir, renombrarlo, borrarlo o renombrar otro archivo encima
- THEN cada intento falla con un error de compartir (5 o 32), y una lectura con `FILE_SHARE_READ` abre
- AND cuando la carga vuelve, con la sesión armada o con error, todo eso anda

#### Escenario: la corrida que baja el modelo
- GIVEN una caché sin `model.onnx`
- WHEN corre `ensure` contra un servidor que lo sirve
- THEN el ONNX se escribe a `.partial` con su hash, se renombra y se carga desde la caché, sin guardar el cuerpo, y se hashea dos veces en esa corrida
- AND un archivo chico faltante se entrega desde memoria, sin releerlo

**Tests** (`model_download.rs`, salvo que se diga otro): `the_cache_check_hands_the_onnx_over_as_a_read_only_view` (nuevo); `a_view_of_a_changed_onnx_is_refused` (nuevo); `a_session_is_built_from_the_verified_view` (nuevo, `embedder.rs`, con el ONNX mínimo y `commit_from_memory`); `a_load_holds_the_model_file_against_writes_until_the_session_returns` (nuevo, `#[cfg(windows)]`, también en un directorio temporal con «ñ»); `the_model_file_is_free_again_after_a_failed_load` (nuevo, `#[cfg(windows)]`); `a_downloaded_onnx_is_not_kept_in_memory_and_is_loaded_from_the_cache` (nuevo); `each_file_is_hashed_once_per_run` cambia para decir que sólo el ONNX bajado se hashea dos veces y los chicos una; se conservan `the_cache_check_leaves_the_runtime_thread_free`, `a_file_larger_than_any_model_file_is_refused_without_holding_it` y `the_worker_resolves_the_tokenizer_off_the_runtime_thread` (`runtime.rs`).

## RM2. La carga y la reparación no se pisan

La reparación corre en el mismo proceso que la carga (`repair_in_background`, `crates/clawg-core/src/memory/bootstrap.rs:489`, que corre `ensure` bajo una marca `AtomicBool` y sin ningún candado contra la carga), y ningún comando de `clawg-cli` escribe en `<data>/models/`.

- Un candado del proceso MUST cubrir la carga desde que abre el archivo hasta que vuelve `commit_from_memory`, y la reparación MUST tomarlo antes de renombrar `.partial` sobre `model.onnx`. Una reparación MUST esperar a que la carga suelte el archivo.
- Si la apertura de la carga falla por compartir (errores 32, 33 o 5 de Windows, por ejemplo un antivirus), MUST reintentarse con esperas crecientes hasta unos 2 s.
- Si sigue fallando, el pedido MUST contestar que el modelo no está listo, con una línea en el log, y MUST NOT lanzar una reparación, MUST NOT contar el archivo como faltante en el arranque (hoy `Err(_) => missing`, `model_download.rs:577`) y MUST NOT volver a leer el ONNX a un `Vec`.
- Ni el código ni los tests MUST comparar `ErrorKind`: Rust deja el 32 como `Uncategorized`.

#### Escenario: una reparación con una carga en curso
- GIVEN una carga que tiene el archivo
- WHEN una reparación quiere renombrar `.partial` sobre `model.onnx`
- THEN espera, y renombra después de que la carga lo soltó

#### Escenario: una apertura que se libera dentro del plazo
- GIVEN un archivo abierto por otro con un plazo de compartir que termina antes de 2 s
- WHEN la carga lo abre
- THEN reintenta y carga

#### Escenario: una apertura que sigue chocando
- GIVEN un archivo que no se libera en el plazo
- WHEN la carga lo abre
- THEN el pedido contesta que el modelo no está listo, queda una línea en el log y no se lanza ninguna reparación ni ninguna descarga

**Tests:** `a_repair_waits_for_a_load_to_release_the_model_file` (nuevo, `model_download.rs`, donde viven el candado y el renombre); `a_sharing_violation_on_open_is_retried_within_the_wait` (nuevo, `model_download.rs`, `#[cfg(windows)]`); `a_persistent_sharing_violation_answers_not_ready_without_a_repair` (nuevo, `bootstrap.rs`, con la apertura inyectada, sin depender de Windows); `a_persistent_sharing_violation_is_not_a_missing_file_at_start` (nuevo, `model_download.rs`). Que no se vuelve a un `Vec` lo garantiza el tipo: fuera de los tests, `OnnxFile` no tiene una variante en memoria (diseño, D1).

## RM3. Cada carga dice su pico privado (sólo Windows)

La línea de `info` de cada recarga (`load_line`, `crates/clawg-core/src/memory/idle_embedder.rs:85`) MUST decir, además de lo que dice hoy, el pico de memoria privada del proceso durante esa carga cuando el daemon le dio la sonda del pico. `IdleEmbedder` MUST leerla antes y después de la carga. La sonda es un `MemoryProbe` (`crates/clawg-core/src/metrics/mod.rs:15`) que va junto a la de memoria privada. `clawg serve` las pasa juntas (`crates/clawg-cli/src/daemon/serve.rs:153`), y la del pico lee `PeakPagefileUsage` (`crates/clawg-cli/src/daemon/process.rs:114`).

- Si el pico subió, la línea MUST nombrar el valor nuevo. Es el pico del proceso en la ventana de la carga: otra tarea que corra a la vez puede ser la que lo subió.
- Si no subió, MUST decir que el pico sigue en el valor de antes, de antes de esta carga.
- Sin sonda, o en Linux (donde no hay pico de `RssAnon`), la línea MUST NOT decir nada del pico. MUST NOT escribirse en `metrics.db`.

Línea con el pico:

```
embedding model loaded again in 9234 ms for a wake; private memory 284.0 MiB before, 740.0 MiB after; private peak rose to 1638.4 MiB
embedding model loaded again in 1102 ms for a call that embeds; private memory 301.2 MiB before, 760.5 MiB after; private peak stayed at 1622.0 MiB, set before this load
```

#### Escenario: la carga sube el pico
- GIVEN una sonda del pico que devuelve 1.000 MiB antes y 1.600 MiB después
- WHEN se recarga el modelo
- THEN la línea termina en `private peak rose to 1600.0 MiB`

#### Escenario: la carga no pasa el pico
- GIVEN una sonda del pico que devuelve el mismo valor antes y después
- WHEN se recarga el modelo
- THEN la línea termina en `private peak stayed at …, set before this load`

#### Escenario: sin sonda
- GIVEN un `IdleEmbedder` sin sonda del pico
- WHEN se recarga el modelo
- THEN la línea es la de hoy

**Tests** (`idle_embedder.rs`): `a_load_line_names_the_peak_when_the_load_raised_it` (nuevo), `a_load_line_says_when_the_load_did_not_pass_the_peak` (nuevo), `a_load_line_without_a_peak_probe_is_unchanged` (nuevo); `the_peak_is_not_written_to_the_metrics_file` (nuevo, `crates/clawg-cli/src/daemon/metrics.rs`).

## RM4. Un test fija el contrato de embedding

`E5_MODEL_ID` es una constante (`embedder.rs:135`) y es lo que distingue los espacios de vectores. Un test MUST fijar juntos `E5_MODEL_ID`, los dos prefijos (`"query: "` en ambos, `:109` y `:130`), `EMBED_MAX_TOKENS` (`:166`), `EMBEDDING_DIM` (`crates/clawg-core/src/memory/vector.rs:8`), el pooling que el id nombra y los SHA-256 fijados del modelo y del tokenizer (`model_download.rs`). Cuando cambia cualquiera, el test MUST fallar con un mensaje que pregunta si `E5_MODEL_ID` tiene que subir.

#### Escenario: un prefijo cambia
- GIVEN el contrato fijado por el test
- WHEN un prefijo, el tope de tokens, la dimensión, un SHA-256 o el id cambian
- THEN el test falla y pide revisar si `E5_MODEL_ID` sube

**Tests:** `the_embedding_contract_is_pinned` (nuevo, `embedder.rs`).

## RM5. El refresco reutiliza los vectores de los pasajes que no cambiaron

Cuando un doc cambia (otro sha256), `refresh` (`crates/clawg-core/src/memory/project_docs/mod.rs:407`) MUST reutilizar el vector guardado de cada pasaje cuyo `indexed_text` entero (`breadcrumb`, `"\n\n"` y `body`; `chunk.rs:28`) está entre los pasajes guardados de ese mismo doc, siempre que el `model_id` guardado del doc sea el del refresco (leído una vez, `:420`). MUST embeber sólo los demás pasajes (hoy, `embed_passages`, `:464`, los embebe todos).

- **Por valor.** MUST leer pares de texto y bytes del vector, nunca ids de fila (`replace_doc` borra y vuelve a insertar con ids nuevos). Un texto repetido en el doc MUST reutilizar en todos sus pasajes.
- **Todo o nada.** El doc MUST seguir reemplazándose entero en **una** transacción (`replace_doc`, `:544`); lo ya dicho sobre un embedder que falla, un vector inservible y la cuenta distinta no cambia.
- **Qué no reutiliza.** Un doc guardado con otro `model_id` MUST NOT reutilizar nada. Un archivo movido MUST NOT reutilizar nada, porque su ruta está en el `indexed_text`. Un doc que no cambió sigue sin re-embeberse.
- **`reused`.** `RefreshReport` (`:74`) MUST llevar `reused`, los pasajes reutilizados (no los textos distintos). La línea `indexed the docs of …` (`crates/clawg-cli/src/daemon/api.rs:2675`) MUST mostrarlo.
- **Equivalencia.** El doc que queda MUST ser equivalente al de un reindexado completo: los mismos pasajes y, con un embebedor falso determinista, los mismos vectores byte a byte. Con el modelo real, fuera de CI, MUST coincidir con tolerancia de 1e-5 por componente o coseno ≥ 1 − 1e-6.
- **Deriva.** Un pasaje sin cambios conserva su vector hasta que cambie el `model_id` o se reindexe. Si el contrato cambia sin que cambie el id, esa deriva también alcanza a los pasajes sin cambios de un doc cambiado, como hoy alcanza a un doc que no cambia (`plan_refresh`, `:121`). RM4 es lo que mantiene el id al día.
- **Un refresco a la vez.** «Sólo MUST correr un refresco por proyecto a la vez» no cambia. `DocIndex::begin` (`:209`) ya impide que otro refresco cambie lo leído antes de que se escriba. El `model_id` es una constante y `same_model` (`idle_embedder.rs:218`) rechaza una recarga con otro id, así que no hace falta descartar lo reutilizado si la sesión cambia.

#### Escenario: se edita una sección
- GIVEN un doc indexado de varios pasajes y un embebedor falso que anota los textos que le piden
- WHEN se edita una sección y se refresca
- THEN el embebedor recibe sólo los pasajes que cambiaron, cada vector reutilizado es el blob que estaba guardado, `reused` cuenta los demás pasajes, y el doc es idéntico byte a byte al de un reindexado completo con el mismo embebedor

#### Escenario: un texto repetido
- GIVEN un doc con dos pasajes de igual `indexed_text`, que cambia en otra sección
- WHEN se refresca
- THEN ninguno de los dos se embebe y `reused` los cuenta a los dos

#### Escenario: otro modelo, o un archivo movido
- GIVEN un doc indexado con el modelo `m1`
- WHEN se refresca con `m2`, o el mismo contenido aparece bajo otra ruta
- THEN se embeben todos sus pasajes y `reused` es 0

#### Escenario: el embebedor falla
- GIVEN un doc cambiado con pasajes reutilizables y un embebedor que falla en los demás
- WHEN se refresca
- THEN el doc queda como estaba y el siguiente refresco lo reintenta

**Tests** (`project_docs/mod.rs`, salvo que se diga otro): `an_edited_doc_embeds_only_the_passages_that_changed` (nuevo); `a_partly_reused_doc_equals_a_full_reindex` (nuevo, byte a byte); `a_reused_vector_is_the_stored_blob` (nuevo); `a_repeated_text_is_reused_in_all_its_passages` (nuevo); `a_doc_of_another_model_reuses_nothing` (nuevo); `a_moved_file_reuses_nothing` (nuevo); `a_failed_embedding_of_the_rest_keeps_the_doc_as_it_was` (nuevo); `reused_counts_passages_not_distinct_texts` (nuevo); `the_docs_log_line_shows_the_reused_passages` (nuevo, test de unidad de `refresh_line` en `crates/clawg-cli/src/daemon/api.rs`); con el modelo real, fuera de CI, `a_partly_reused_doc_matches_a_full_reindex_within_tolerance` (nuevo, `#[ignore]`, `project_docs/mod.rs`, con `CLAWG_E5_MODEL_DIR` como `tests/e5_golden.rs`). Se conservan `an_unchanged_project_embeds_nothing`, `another_model_reindexes_every_doc` y `a_second_refresh_does_not_wait_for_the_first`. `plan_refresh` no cambia.

## La puerta de la medición de C (no normativa)

Decide si RM1 a RM3 quedan como están (vista mapeada con `commit_from_memory`, C′) o si el mecanismo pasa a `commit_from_file`. Necesita autorización para un build release y para bajar el modelo; no corre en CI y no toca ningún perfil.

- **Corrida.** Tres brazos de diez cargas: `commit_from_memory` con el `Vec` (hoy), `commit_from_file` (C, sólo en el arnés) y `commit_from_memory` sobre la vista mapeada (C′). Cada carga en un proceso nuevo, dentro de su propio job object; brazos intercalados; una carga de calentamiento descartada por brazo. Modelo real en un directorio temporal, release, con la función de carga real y las opciones de prod (arena apagada, nivel 3, un hilo por CPU).
- **Se lee** el `PeakProcessMemoryUsed` del job y, en el hijo, `PeakPagefileUsage`, `PeakWorkingSetSize` y `PrivateUsage` tras la carga, y los ms de verificar y de armar la sesión.
- **Pasa un brazo** si la mediana del pico de commit baja al menos 300 MiB, su peor carga queda por debajo de la mejor de hoy, y la mediana del tiempo total con caché tibia no sube más de max(150 ms, 15 %).
- **Qué queda.** Si pasa C′, sola o con C, queda C′. Si pasa sólo C, el mecanismo cambia a `commit_from_file`, con la ruta que sale del handle y la comparación de `FILE_ID_INFO`, y RM1 se reescribe. Si no pasa ninguno, C se revierte y se cierra con lo medido.
- **Linux.** La misma corrida (`VmHWM`) confirma el mecanismo, pero no decide.
- **Predicción, sin medir** (código de ONNX Runtime 1.24.2): C baja unos 272 MiB en Windows y C′ unos 448.
