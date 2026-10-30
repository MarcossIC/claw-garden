# import-job — Especificación

Suma requisitos a [`specs/portability`](../../specs/portability/spec.md), IJ1 a IJ11 y IJ15 a IJ17, y modifica R3 y HI16; a [`specs/daemon-access`](../../specs/daemon-access/spec.md), IJ12 a IJ14, que agregan las rutas de trabajos, el bloque `import` de `/health` y las filas de `PROJECT_INPUTS`; y a [`specs/storage`](../../specs/storage/spec.md), IJ18, que modifica «Cada fase del apagado tiene tope». IJ19 es transversal: no cambia ningún esquema.

Las decisiones y lo descartado están en la [propuesta](proposal.md) y la [exploración](exploration.md); acá sólo el contrato y la prueba que lo fija. Una «prueba propuesta» es una que hoy no existe y falla (o no compila) antes del cambio; una «guarda» ya pasa hoy y debe seguir pasando. Los nombres de prueba son los que el cambio debe escribir; el diseño puede afinarlos sin cambiar la intención.

## Definiciones

- **Trabajo de importación:** una importación que el daemon corre en una tarea propia, con un id del proceso que empieza en 1, visible por `GET /v1/imports/{id}`. Distinto de `import_id` (V26, `memory_import`), que se toma en la primera escritura (`crates/clawg-core/src/memory/portability/import.rs:965`) y no existe en una pasada en seco ni en una importación que no añade nada.
- **Trabajo activo:** el que no llegó a un estado final. **Estado final:** `done`, `failed`, `cancelled` o `interrupted`.
- **Parada:** la señal única que levantan `DELETE /v1/imports/{id}` (causa cancelación) y el apagado (causa apagado). Se mira en los puntos de IJ6.

## IJ1. `POST /v1/imports`: la ruta, el cuerpo y el 202

`POST /v1/imports` MUST aceptar el cuerpo de `POST /v1/import` (`ImportRequest`, `crates/clawg-cli/src/daemon/api.rs:1943-1965`) con `path` en lugar de `text`. El cuerpo MUST rechazar campos desconocidos, de modo que un `text` es un `400` y no se ignora (como `POST /v1/import/undo`, `api.rs:2075-2076`). Antes del `202`, el daemon MUST comprobar que `path` es absoluta, que nombra un archivo regular, que su tamaño no pasa de `MAX_IMPORT_BYTES` (`import.rs:65`) y que se puede leer; cualquiera de esas fallas MUST ser un `400` que nombra la causa, sin crear trabajo. El daemon MUST leer y parsear el archivo (el `read` de hoy, `api.rs:2013-2052`) dentro del `POST`, antes del `202`, y el trabajo MUST recibir lo parseado: el texto del archivo no vive más allá de la respuesta y un archivo de clawg se parsea una vez (hoy dos, `api.rs:2034` y `import.rs:2079`). Aceptado, MUST responder `202` con el estado del trabajo (IJ4) en `phase` de su primera fase.

La ruta MUST NOT tener el tope de cuerpo de `2 * MAX_IMPORT_BYTES` (`api.rs:1063`, `:1941`), que existe sólo por el escape JSON del archivo.

#### Escenario: se acepta sin esperar
- GIVEN un export de clawg en un archivo del perfil de prueba y un embedder de prueba que suelta los lotes de a uno
- WHEN se manda `POST /v1/imports` con su ruta absoluta
- THEN responde `202` con `job_id` y `phase` sin esperar el embedding, y `GET /v1/imports/{id}` responde el estado

#### Escenario: la ruta no sirve
- GIVEN una ruta relativa, un directorio, un archivo de `MAX_IMPORT_BYTES + 1` bytes y, en Unix, un archivo sin permiso de lectura
- WHEN se manda cada uno
- THEN cada uno es un `400` antes del `202`, sin trabajo y sin que cambie el store

#### Escenario: un campo que no existe
- GIVEN un cuerpo con `path` y `text`
- WHEN se manda
- THEN es un `400` que nombra el campo y no hay trabajo

**Tests (propuestos):** `an_import_job_is_accepted_without_waiting_for_the_embedder` y `a_job_import_refuses_a_path_that_is_not_an_absolute_readable_regular_file_within_the_limit` y `a_job_import_refuses_an_unknown_field` (`crates/clawg-cli/tests/api.rs`; hoy `/v1/imports` es un `404`); `the_client_sends_a_path_and_not_the_file` (`crates/clawg-cli/src/client.rs`, mod tests, sobre el cuerpo que arma `import_job`; hoy `import` arma `text`, `client.rs:1419-1423`); `a_clawg_file_is_parsed_once_for_a_job` (unidad en `import.rs`: la entrada que parte de lo parseado da el mismo informe que `import` sobre el texto y no vuelve a llamar a `format::parse`, medido con el contador de parseos de la prueba).

## IJ2. Cada rechazo de `POST /v1/import` sigue antes del 202, con la misma respuesta

Cada `400`, `404`, `409` y `503` que hoy da `POST /v1/import` antes de importar (`api.rs:2013-2052`, `import_error`, `:1992-2000`) MUST ser, en `POST /v1/imports`, la misma respuesta (estado y `error`) y MUST NOT dejar un trabajo: el agente en blanco, `map_roots` sin raíz, el mapa de la otra fuente, `from` desconocido, el formato o la versión desconocidos, un encabezado ausente, el proyecto del mapa que no está (`404`), ambiguo (`409 project_ambiguous`) o borrado (`409 project_deleted`), el `503 tokenizer_not_ready` de otra fuente sin tokenizer, también en seco, y, para una importación real, el `503 embedder_not_ready` o `embedder_failed` sin modelo (`api.rs:1439-1446`). Una pasada en seco MUST NOT pedir el modelo, como hoy.

#### Escenario: las dos rutas dicen lo mismo
- GIVEN cada uno de esos casos armado como cuerpo de `POST /v1/import` y como `POST /v1/imports` con el mismo contenido en un archivo
- WHEN se manda a las dos rutas
- THEN el estado y el `error` coinciden, y `GET /v1/health` no muestra trabajo después de ninguno

**Test (propuesto):** `a_job_import_refuses_what_the_synchronous_import_refuses`, una tabla de casos que recorre las dos rutas con el mismo cuerpo salvo `text`/`path` (`crates/clawg-cli/tests/api.rs`); falla hoy porque `/v1/imports` da `404` en todos.

## IJ3. Un solo trabajo activo; `wake()` al aceptar

Con un trabajo activo, `POST /v1/imports` MUST responder `409` `import_running` con el id del activo en `job_id` del cuerpo de error (como `memories` en un save fallido, `api.rs:123-133`), sin abrir ni leer el archivo. Terminado ese trabajo (cualquier estado final), el siguiente `POST` MUST aceptarse. Aceptado un trabajo real, el daemon MUST llamar a `wake()` del embedder (como lo hace un registro, `api.rs:2207-2211`); uno en seco MUST NOT despertarlo.

#### Escenario: un segundo POST
- GIVEN un trabajo en `embedding` que el embedder de prueba retiene
- WHEN llega otro `POST /v1/imports`, con otro archivo
- THEN es `409 import_running` con el `job_id` del primero; liberado el primero y terminado, el mismo `POST` da `202`

**Tests (propuestos):** `a_second_import_job_while_one_runs_is_refused_with_its_id` y `accepting_a_real_job_wakes_the_embedder_and_a_dry_run_does_not` (`crates/clawg-cli/tests/api.rs`, con `Harness::start_unloaded`, `tests/api.rs:227-229`, y el contador de cargas del embedder de prueba).

**Ambigüedad señalada:** la propuesta no dice si el `409 import_running` va antes o después de los `400` de forma del cuerpo y de campos que no leen el archivo. Esta spec fija sólo que va antes de abrir el archivo.

## IJ4. `GET /v1/imports/{id}`: el estado sale de la memoria del proceso

`GET /v1/imports/{id}` MUST responder, sin tocar la conexión del store: `job_id`; `phase` (`planning`, `preparing` o `translating`, `loading`, `deciding`, `embedding`, `snapshot`, `writing`, `linking`, y los finales `done`, `failed`, `cancelled`, `interrupted`), la que corresponde al paso que corre de verdad (`Step`, `crates/clawg-core/src/memory/portability/timing.rs:13-33`); `dry_run`; `from`; `started_at_ms`; `phase_started_at_ms`; los contadores de IJ5; `import_id` en cuanto el primer lote confirmado lo toma, y ausente antes; `stopping` (la causa pedida) mientras se pidió parar y no paró; `stopped_in` (el paso donde paró) en un final `cancelled` o `interrupted`; `report` con `done` y con una parada después de la primera escritura; y, con `failed`, `error` y `detail` con los códigos de hoy (`embedder_not_ready` de una recarga que falla; `project_deleted` de la comprobación dentro de la transacción). Una pasada en seco MUST terminar `done` tras `deciding`, sin pasar por `embedding`, `snapshot`, `writing` ni `linking`. El daemon MUST recordar el último trabajo terminado mientras viva el proceso; un id que no es el activo ni el último terminado MUST ser `404`.

#### Escenario: la fase avanza dentro de embedding
- GIVEN un trabajo real con un embedder de prueba que suelta un lote por vez
- WHEN se consulta entre lote y lote
- THEN `phase` es `embedding` y `rows_embedded` crece sin pasar de `rows_to_embed`

#### Escenario: el último terminado se recuerda
- GIVEN un trabajo que terminó `done` y después otro que terminó
- WHEN se consulta el id del primero y el del segundo
- THEN el primero es `404` y el segundo trae su estado y su `report`

#### Escenario: una recarga que falla
- GIVEN un modelo soltado por inactividad cuya recarga falla
- WHEN se acepta un trabajo real y llega a `embedding`
- THEN termina `failed` con `error: embedder_not_ready`, sin filas nuevas, sin snapshot `pre-import` y sin fila de `memory_import`

**Tests (propuestos):** `a_job_reports_its_phase_and_rows_embedded_advance_inside_embedding`, `only_the_last_finished_job_is_remembered`, `an_unknown_job_is_a_404`, `a_dry_run_job_ends_done_after_deciding`, `a_failing_reload_fails_the_job_without_writing` (`crates/clawg-cli/tests/api.rs`); por el binario con `CLAWG_TEST_EMBEDDER=fake`, `a_job_import_ends_done_with_the_report_of_the_synchronous_import` (`crates/clawg-cli/tests/binary.rs`, cerca de `:792` y `:830`: el `report` del trabajo es igual, salvo snapshot e `import_id`, al de `POST /v1/import` para el mismo archivo).

**Ambigüedades señaladas:** (1) la propuesta pone el `report` sólo con `done` o con una parada tras la primera escritura, pero la decisión 6 dice que, al cancelar el embedding, «el informe lo dice»; esta spec lo deja en `stopped_in` y los contadores (sin `report`) hasta que el diseño decida otra cosa. (2) La propuesta no fija los valores de `stopping`; esta spec sólo exige que nombre la causa, cancelación o apagado.

## IJ5. Los contadores y sus invariantes

El estado MUST llevar, contados en filas y notas y nunca en lotes: `rows` y `notes` (al terminar `prepare` o `translate`), `rows_to_embed`, `rows_embedded`, `rows_written` y `notes_written`. Una «nota» es la unidad que un lote de escritura mantiene entera (`Prepared::units`, `import.rs:369-382`). `ImportTimings` MUST seguir registrando cada paso al terminar, sin cambios (`timing.rs:157-161`). Al terminar `done`: `rows_embedded == rows_to_embed`, igual a las filas insertadas que no están borradas (`import.rs:4951`); `rows_written == report.rows` y `notes_written == notes`. Al parar en `writing`, el informe parcial MUST contar exactamente las filas y notas de los lotes confirmados, y `rows_written`/`notes_written` MUST coincidir con él. Los contadores MUST NOT depender de `EMBED_BATCH` (`import.rs:71`) ni de `WRITE_BATCH` (`:69`).

#### Escenario: el final coincide con el informe
- GIVEN un trabajo de un archivo de clawg y otro de uno de otra herramienta, con notas largas en piezas
- WHEN terminan `done`
- THEN los contadores cumplen las igualdades de arriba contra su `report`

#### Escenario: la parada en writing
- GIVEN un trabajo parado en `writing` tras algunos lotes
- WHEN se compara el `report` con los contadores
- THEN las filas y notas del informe son las de los lotes confirmados

**Tests (propuestos):** `a_finished_jobs_counters_match_its_report` y `a_job_stopped_in_writing_counts_exactly_the_committed_batches` (`crates/clawg-cli/tests/api.rs`); `the_counters_do_not_depend_on_the_batch_sizes` (unidad en `import.rs`, con `EMBED_BATCH` y `WRITE_BATCH` ejercidos en dos tamaños de archivo).

## IJ6. `DELETE /v1/imports/{id}`: cancelar en un límite seguro

`DELETE /v1/imports/{id}` MUST levantar la señal con causa cancelación y responder `202` con el estado; el trabajo MUST llegar a `cancelled` en su siguiente punto de parada. Los puntos de parada por paso MUST ser los de la propuesta (decisión 13), y lo que cada uno deja:

| Paso | La señal se mira | Lo que deja |
|---|---|---|
| `plan` | entre pasadas (a lo sumo `PLAN_PASSES`, `import.rs:461`) | nada |
| `prepare` | al terminar | nada |
| `translate` | compite con el `spawn_blocking` | nada |
| `load` | entre llamadas de `LOAD_BATCH` (`import.rs:1272`) | nada |
| `decide` | por lote, como hoy (`import.rs:1569-1572`) | nada |
| `embed` | compite con cada lote y con el orden por tokens, y se mira antes del snapshot | nada: los vectores se sueltan |
| `snapshot` | no se corta; se mira al terminar | el snapshot `pre-import`, sin filas |
| `write` | entre lotes de `note_batches` (`import.rs:2183`) | notas enteras (IJ7) |
| `links` | no se corta (una transacción, `import.rs:2310-2315`) | la importación entera: el trabajo termina `done` |

Hasta el snapshot, MUST NOT escribirse ninguna fila de `memory`, ninguna fila de `memory_import*` ni ningún snapshot `pre-import`: se conserva «nada antes del snapshot» (`import.rs:2267`) y el orden global de más a menos tokens. Soltar el `await` de un lote de embedding MUST abandonar ese lote sin API nueva en `Embedder`. `DELETE` de un trabajo en estado final MUST responder su estado sin cambiar nada (idempotente); un id desconocido, `404`.

#### Escenario: cancelar en embedding
- GIVEN un trabajo real en `embedding`, con un embedder que retiene el lote en vuelo
- WHEN llega `DELETE`
- THEN el trabajo termina `cancelled` con `stopped_in: embed`, sin filas nuevas, sin fila de `memory_import` y sin snapshot `pre-import`, y el lote retenido no impide que termine

#### Escenario: cancelar dos veces y cancelar lo terminado
- GIVEN un trabajo ya `cancelled` y otro `done`
- WHEN llega otro `DELETE` a cada uno
- THEN cada uno contesta su estado y nada cambia

#### Escenario: cancelar durante links
- GIVEN un trabajo cuyo `DELETE` llega durante `linking`
- WHEN la transacción termina
- THEN el trabajo termina `done` con la importación entera

**Tests (propuestos):** `deleting_a_job_in_embedding_leaves_no_row_no_journal_and_no_snapshot`, `a_delete_on_a_finished_job_changes_nothing`, `a_delete_during_links_ends_done` (`crates/clawg-cli/tests/api.rs`); `each_step_of_an_import_stops_where_the_table_says` (unidad en `import.rs`, un paso por caso con la señal levantada de antemano o por un gancho de la prueba); `an_abandoned_embedding_batch_does_not_block_the_stop` (unidad en `import.rs` con un `Embedder` de prueba que no responde).

## IJ7. Cancelar en `write` deja notas enteras y libera el número

Una parada en `write` MUST devolver lo confirmado (el `Ledger` de los lotes que se confirmaron y el `import_id`) en vez de un error, y el `report` del trabajo se arma con eso. El store MUST quedar con notas enteras: ninguna fila de `memory_piece` sin la cabeza de su save, y cada nota de otra herramienta completa o ausente. `memory_import.finished_at` MUST quedar `NULL` (la transacción de `links` no corrió, `import.rs:2310-2315`). El número MUST soltarse al soltarse el `Journal` (`import.rs:1015`), de modo que `clawg import --undo N` deje de ser `Running` (`UndoError::Running`, `crates/clawg-core/src/memory/portability/undo.rs:1219`) y quite lo escrito; una reimportación del mismo archivo MUST hallar presentes esas notas y terminar el resto.

#### Escenario: parada a mitad de la escritura
- GIVEN un trabajo de notas largas que se para entre dos lotes de escritura
- WHEN se mira el store, el estado y se corre `--undo N`
- THEN no hay pieza sin cabeza, el `report` cuenta las notas de los lotes confirmados, el estado trae `import_id` y `--undo` lo quita sin 409

#### Escenario: terminar con una reimportación
- GIVEN el mismo trabajo parado
- WHEN se importa otra vez el archivo
- THEN las notas escritas son presentes, se escribe el resto y los vínculos que faltaron se rellenan

**Tests (propuestos):** `a_job_stopped_in_writing_leaves_whole_notes_and_frees_its_number` y `importing_again_finishes_a_stopped_job` (`crates/clawg-cli/tests/api.rs`); `a_stop_between_write_batches_returns_the_committed_ledger` (unidad en `import.rs`; hoy una parada es un error `"the import was stopped"` sin ledger, `import.rs:2235`). Para el flujo `--undo`, `undoing_a_stopped_job_works` (`crates/clawg-cli/tests/api.rs`).

## IJ8. Un trabajo sobrevive al cliente; `POST /v1/import` sigue cancelando

Un trabajo MUST seguir y llegar a `done` si el cliente que lo aceptó cierra la conexión tras el `202`, o si muere el proceso `clawg import` que lo sigue. `POST /v1/import` (síncrono) MUST seguir cancelando la importación cuando el cliente se va, como hoy (el futuro del handler se suelta y `StopOnDrop` levanta la bandera de `decide`, `import.rs:1581`).

#### Escenario: el cliente se va
- GIVEN un trabajo en `embedding` y el cliente que lo aceptó
- WHEN el cliente cierra la conexión (en el binario, se mata el proceso de `clawg import`)
- THEN el trabajo llega a `done`

#### Escenario: guarda del síncrono
- GIVEN `POST /v1/import` con un embedder lento
- WHEN el cliente cierra la conexión TCP
- THEN el futuro del handler se suelta y no queda ninguna fila ni `memory_import`

**Tests:** `a_job_outlives_the_client_that_accepted_it` (propuesta, `crates/clawg-cli/tests/api.rs`) y `killing_clawg_import_does_not_stop_the_job` (propuesta, `crates/clawg-cli/tests/binary.rs`, con un proceso que se mata por su propio handle). **Guarda que pasa hoy:** `the_synchronous_import_still_cancels_when_the_client_goes` (`crates/clawg-cli/tests/api.rs`); la exploración marca que la premisa se verificó por lectura y que falta la prueba que la ejecute, así que esta prueba es nueva pero MUST pasar con el código actual.

## IJ9. El modelo: `wake()` al aceptar, falla en `embed`

Aceptar un trabajo real MUST despertar el modelo (IJ3), de modo que un modelo que la inactividad soltó esté cargado, o su recarga ya haya fallado, antes de la fase `embedding`. No hay guardia de retención: el modelo puede soltarse otra vez por inactividad. Si la recarga falla al llegar a `embed`, el trabajo MUST terminar `failed` con `embedder_not_ready` y MUST NOT escribir nada (ver IJ4).

#### Escenario: recarga antes de embedding
- GIVEN un daemon con el modelo soltado por inactividad (`Harness::start_unloaded`) y un embedder de prueba que cuenta cargas
- WHEN se acepta un trabajo real
- THEN el contador de cargas sube antes de que `phase` sea `embedding`

**Test (propuesto):** `accepting_a_job_reloads_a_released_model_before_embedding` (`crates/clawg-cli/tests/api.rs`; hoy el handler pide `ready_embedder`, `api.rs:2052`, no `wake`). La falla de la recarga es el escenario de IJ4.

## IJ10. La memoria: el archivo no se copia ni se retiene

El cliente MUST NOT leer el archivo: manda la ruta (IJ15). El daemon MUST NOT bufferear el archivo como cuerpo de la petición ni conservar su texto después del `202` (IJ1). No hay requisito de pico: la medición de memoria (el pico privado del cliente y del daemon por cuerpo y por ruta, en el mismo build, con `CLAWG_TEST_EMBEDDER=fake`) es una anotación de `verify.md`, no una prueba de la suite, y su resultado no cambia ningún requisito.

**Tests:** los de IJ1 (`the_client_sends_a_path_and_not_the_file`, `a_clawg_file_is_parsed_once_for_a_job`); `the_job_route_has_no_body_limit_sized_for_the_file` (propuesta, `crates/clawg-cli/tests/api.rs`: el cuerpo de `POST /v1/imports` no necesita `IMPORT_BODY_LIMIT`, `api.rs:1941`).

## IJ11. `POST /v1/import` no cambia

`POST /v1/import` y `POST /v1/import/undo` MUST NOT cambiar de contrato, de respuestas ni de comportamiento, y las funciones de importación del núcleo MUST recibir, para esas rutas, un control que nadie levanta (el `StopOnDrop` sigue cortando `decide`). El informe, el formato de los archivos y lo que una importación decide MUST NOT cambiar.

**Tests (guardas que pasan hoy):** las pruebas de import de `crates/clawg-cli/tests/api.rs` (entre ellas `an_import_stores_rows_with_their_state_and_a_dry_run_stores_none`, `import_answers_400_for_an_unknown_version_before_the_embedder` y `an_import_is_undone_by_its_number`) y las de `import.rs` siguen verdes sin tocarse; `the_report_of_a_job_equals_the_report_of_the_synchronous_import` (propuesta, `crates/clawg-cli/tests/api.rs`, mismo archivo por las dos rutas) fija que no cambia el informe.

## IJ12. `/health` gana un bloque `import`, de memoria

`GET /health` MUST llevar, mientras haya un trabajo activo, un bloque `import` con su id, su fase y sus contadores, sacado de la memoria del proceso sin mandar ninguna llamada a la conexión del store. Sin trabajo activo MUST estar ausente. El campo es aditivo: el cliente lee `Health` sin rechazar campos que no conoce (`crates/clawg-cli/src/client.rs:135-158`). Que `status: ok` se diga con la conexión ocupada por un lote es de otro cambio y MUST NOT cambiar acá.

#### Escenario: la conexión ocupada
- GIVEN un trabajo activo y la conexión del store ocupada por otra llamada
- WHEN se pide `/health`
- THEN contesta dentro de `HEALTH_INGEST_WAIT` (`api.rs:1165`) con el bloque `import` y sin `ingest`

**Tests (propuestos):** `health_carries_the_active_jobs_import_block_without_the_connection` y `health_has_no_import_block_without_an_active_job` (`crates/clawg-cli/tests/api.rs`); `a_client_reads_a_health_with_an_import_block` (`crates/clawg-cli/src/client.rs`).

## IJ13. `PROJECT_INPUTS` clasifica las rutas nuevas y los recorridos cubren `POST /imports`

`PROJECT_INPUTS` (`crates/clawg-cli/tests/api.rs:4520`) MUST ganar `("POST", "/imports", Some("body.project_map[].project"))`, `("GET", "/imports/{id}", None)` y `("DELETE", "/imports/{id}", None)`; sin ellas `every_route_says_which_project_it_takes` (`:4587`) falla. Los dos recorridos de campos MUST ganar su brazo para `POST /imports`, porque panican con un campo que no comprueban: `every_route_that_takes_a_project_follows_a_merge` (`:4614`): tras una fusión el trabajo termina y la fila queda en el proyecto vivo; `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule` (`:5042`): con un proyecto borrado es `409 project_deleted` antes del `202`, en seco y real.

#### Escenario: un `project_map` a un proyecto fusionado o borrado
- GIVEN un `project_map` con `{"project"}` que nombra un proyecto fusionado, y otro que nombra uno borrado
- WHEN se manda `POST /v1/imports`
- THEN el primero termina `done` con las filas en el proyecto vivo; el segundo es `409 project_deleted` sin trabajo

**Tests:** las tres filas y los dos brazos en `crates/clawg-cli/tests/api.rs`; fallan hoy porque `router()` y la tabla nombran rutas distintas en cuanto existen las rutas, y los recorridos no tienen brazo.

## IJ14. El router de trabajos

Las rutas `POST /v1/imports`, `GET /v1/imports/{id}` y `DELETE /v1/imports/{id}` MUST estar detrás del guardia y del token como todo `/v1`, y se miden como cualquier ruta por su plantilla.

**Test (propuesto):** `the_job_routes_require_the_guard_and_the_token` (`crates/clawg-cli/tests/api.rs`, con `Host` ajeno y sin token).

## IJ15. `clawg import` por la ruta de trabajos

`clawg import FILE` MUST mandar la ruta absoluta del archivo a `POST /v1/imports` sin leerlo, sondear `GET /v1/imports/{id}` cada segundo hasta un estado final e imprimir el informe por stdout con el mismo `render` que hoy (`crates/clawg-cli/src/import.rs:104`), así un script que espera el informe no se rompe. En una terminal MUST mostrar además las fases y los contadores por stderr; `--follow` MUST mostrarlos también fuera de una; sin terminal ni `--follow`, stderr MUST NOT llevar progreso. Ctrl-C MUST mandar `DELETE`, esperar el estado final e imprimir lo escrito, con `clawg import --undo N` cuando el estado trae `import_id`; un segundo Ctrl-C MUST salir sin esperar. Si el daemon desaparece mientras sigue un trabajo, MUST decir el último `import_id` visto y cómo deshacerlo. Cualquier final distinto de `done` MUST salir con 1. `--dry-run` MUST ir también como trabajo; `--undo` MUST seguir síncrono y sin cambios. El sondeo MUST vivir en `crates/clawg-cli/src/import.rs`, no en `client.rs`, así `only_health_sets_a_timeout` (`client.rs:1962`) no cambia. No hay un modo que imprima el id y salga.

#### Escenario: stdout igual, progreso aparte
- GIVEN un export y un daemon de prueba (`CLAWG_TEST_EMBEDDER=fake`)
- WHEN se corre `clawg import --follow FILE`
- THEN stderr trae las fases en el orden de `Step` y stdout es exactamente el `render` del mismo informe

#### Escenario: Ctrl-C
- GIVEN un import en `writing`
- WHEN llega Ctrl-C una vez
- THEN el CLI manda `DELETE`, imprime lo escrito con el `--undo N` y sale con 1; con un segundo Ctrl-C antes del estado final, sale sin esperar

**Tests (propuestos):** `follow_prints_the_phases_in_order_on_stderr_and_the_same_report_on_stdout` (`crates/clawg-cli/tests/binary.rs`); `ctrl_c_sends_the_delete_and_prints_what_was_written` (`binary.rs`, Unix con SIGINT; en Windows, la misma lógica como unidad con la señal inyectada en `import.rs`); `a_second_ctrl_c_leaves_without_waiting`, `any_final_state_but_done_exits_1`, `phases_go_to_stderr_only_on_a_terminal_or_with_follow` y `a_vanished_daemon_names_the_last_import_id` (unidades puras en `crates/clawg-cli/src/import.rs`, sobre la decisión de qué imprimir y qué código devolver); `the_job_poll_lives_outside_the_client_and_sets_no_timeout` (guarda: `only_health_sets_a_timeout` sigue verde y las importaciones por el binario de `binary.rs:792` y `:830` siguen verdes).

## IJ16. Un CLI nuevo frente a un daemon sin la ruta

Si `POST /v1/imports` responde `404` sin el cuerpo de error del daemon (un `404` propio lo lleva, `api.rs:167-169`), el CLI MUST NOT volver al cuerpo de `POST /v1/import` ni mandar nada del archivo; MUST decir que el daemon que corre es anterior a este clawg y que `clawg stop` hace que el próximo arranque use esta versión, con la frase de `version_warning` (`crates/clawg-cli/src/memories.rs:378-389`) y la versión que da `/health`, y salir con 1.

#### Escenario: daemon anterior
- GIVEN un daemon de prueba que no sirve `/v1/imports`
- WHEN se corre `clawg import FILE`
- THEN sale con 1, el daemon no recibió el archivo ni su ruta, y la salida es la frase de `version_warning`

**Test (propuesto):** `import_against_a_daemon_without_the_job_route_exits_1_without_sending_the_file` (`crates/clawg-cli/tests/binary.rs`, con un daemon de prueba con la ruta ausente que registra lo que recibe).

**Ambigüedad señalada:** un `404` con cuerpo también lo da un daemon nuevo para un `project_map` a un proyecto que no está (IJ2). La detección por «`404` sin cuerpo» MUST distinguir esos dos casos por la ausencia del cuerpo de error del daemon; la propuesta lo da por sentado y el diseño debe fijar cómo (por ejemplo, ausencia de `error`).

## IJ17. El CLI habla el idioma de la preferencia

Todo lo que `clawg import` imprime por primera vez en este cambio (las fases y contadores, el mensaje de Ctrl-C, el de daemon desaparecido, el de daemon anterior, el de trabajo `failed`/`cancelled`/`interrupted`) MUST decirse en el idioma de `preferences.json`, un `match` sobre `Lang` por frase, y en inglés sin preferencia, como los demás comandos (`.claude/CLAUDE.md`, párrafo de `preferences.json`). Los identificadores (`phase`, `job_id`, `import_id`, códigos de error, comandos, códigos de salida) y la causa técnica de una falla MUST quedar como vienen.

**Test (propuesto):** `the_job_messages_speak_both_languages` (unidad en `crates/clawg-cli/src/import.rs`, junto a `the_undo_report_speaks_spanish`).

## IJ18. El apagado con un trabajo activo (modifica «Cada fase del apagado tiene tope», `specs/storage`)

Con un trabajo activo, el apagado MUST levantar la señal con causa apagado en el momento en que se observa la orden de parar (`crates/clawg-cli/src/daemon/serve.rs:249-253`) y MUST esperar al trabajo junto a los servidores, dentro de la misma ventana de `HTTP_GRACE` que `within_grace` cuenta desde esa orden (`serve.rs:273`, `:405-412`). Si la ventana se agota con el trabajo vivo, el registro MUST abortar su tarea y marcarlo `interrupted`. Después siguen, sin cambios, `store_probe.abort`, `writer.close`, `runtime.shutdown` y `runtime.close` (`serve.rs:284-298`); abandonado el trabajo, MUST NOT mandarse nada más a la conexión, y la llamada que el trabajo ya mandó (un lote de escritura confirma notas enteras) termina antes del drenaje y del cierre. La suma `HTTP_GRACE + METRICS_CLOSE + DRAIN_DEADLINE + AFTER_DRAIN + RUNTIME_TEARDOWN` MUST NOT ganar ningún término (`crates/clawg-cli/src/stop.rs:66-76`). Como `serve::run` no corre en pruebas, parar, esperar y abandonar MUST ser funciones del registro de trabajos, probadas sin él.

La tabla de la sección del apagado gana la fila «Un trabajo de importación activo | `HTTP_GRACE`, dentro de la misma ventana que las peticiones | el registro de trabajos | la tarea se aborta y el trabajo queda `interrupted`».

#### Escenario: el lote de embedding que no vuelve
- GIVEN un trabajo en `embedding` cuyo lote no vuelve y el tiempo pausado
- WHEN llega la orden de apagado
- THEN el trabajo queda `interrupted` dentro de `HTTP_GRACE`, sin filas nuevas ni snapshot

#### Escenario: el apagado en writing
- GIVEN un trabajo en `writing` con varias notas
- WHEN llega la orden de apagado
- THEN queda `interrupted` con notas enteras, `finished_at` nulo, y tras abandonarlo no se manda nada a la conexión

**Tests (propuestos):** `shutdown_stops_a_job_in_embedding_within_the_grace`, `shutdown_stops_a_job_in_writing_between_whole_notes` y `an_abandoned_job_sends_nothing_to_the_connection` (unidad del registro de trabajos, con tiempo pausado, en `crates/clawg-cli/src/daemon/` junto a las pruebas de `serve.rs`). **Guarda que pasa hoy:** `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`crates/clawg-cli/src/stop.rs`) sigue verde sin cambios. Los embedders `stalled` y `blocked` no sirven para esta prueba: nunca llenan la celda del modelo (`crates/clawg-core/src/memory/bootstrap.rs:375`) y una importación real sin modelo es `503` antes del `202`; el embedder de prueba del registro es uno que retiene el lote.

**Riesgo señalado:** abortar durante `snapshot` o `linking` marca `interrupted` un trabajo cuya última llamada igual puede confirmarse antes del cierre (el `links` es una transacción); el estado queda en el proceso que muere, y lo que vale es `memory_import`.

## IJ19. Sin esquema, y qué pasa entre versiones

Este cambio MUST NOT agregar migración, tabla, identificador ni estado persistido: el trabajo vive en la memoria del proceso, y `memory_import` (V26), `metrics.db`, `daemon.json` y `preferences.json` no cambian. Un CLI anterior con un daemon nuevo MUST seguir usando `POST /v1/import`, síncrono, sin progreso y que cancela al irse el cliente. Un CLI nuevo con un daemon anterior MUST comportarse como dice IJ16. Volver al binario anterior no necesita nada: lo guardado no cambió de forma y una importación cortada se deshace con `clawg import --undo N`, que existe desde V26 en las dos versiones, o con su snapshot `pre-import`. Un daemon apagado o reiniciado por `clawg update` con un trabajo activo lo deja `interrupted` (IJ18).

**Tests (guardas que pasan hoy):** el CLI anterior contra el daemon nuevo es el flujo de `POST /v1/import` de IJ11; `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs`) sigue verde sin cambios y fija que no hay migración nueva, así que no hace falta sembrar una versión anterior. El CLI nuevo contra el daemon anterior es la prueba de IJ16.
