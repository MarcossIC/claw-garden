# import-job — Verificación

## Windows (2026-10-30)

- `target-test` se limpió de los artefactos del workspace (`cargo clean --workspace --target-dir target-test`) al empezar la aplicación y se reusó durante todo el ciclo, como manda «Build directories». No se volvió a limpiar para verificar.
- Sobre el árbol final (`788408e`), los tres comandos salen con 0:
  - `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`;
  - `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`, lo que compila el release, sin `testing`;
  - `cargo test --workspace --target-dir target-test`, con `tests/binary.rs`. Pasa entero en la primera corrida. `hook::tests::session_start_in_a_declared_subfolder_reads_its_project`, que a veces falla con la máquina cargada y no es de este cambio, pasó esta vez.
- Cada prueba que nombran la spec y las tareas aparece `ok` en la corrida. La única que no corre en Windows es `ctrl_c_sends_the_delete_and_prints_what_was_written` (`crates/clawg-cli/tests/binary.rs:1140`, `#[cfg(unix)]`); ver CI.
- Las dos mediciones `#[ignore]` de `crates/clawg-cli/tests/import_memory.rs` quedan ignoradas en la suite, como `transport_bar.rs`.
- En la raíz del repositorio no hay otro directorio de build que `target` y `target-test`, y `git worktree list` muestra un solo árbol.

## Qué se vio fallar antes de cada cambio

Según lo que anotó la aplicación, las pruebas de las tareas 2, 3, 4, 6, 7 y las unidades de la 8 se escribieron primero y fallaron por no compilar o por la aserción. Hubo tres huecos, y esta verificación los cubrió leyendo cada prueba contra el código y, donde una mutación local era barata, mutando el código.

**Las pruebas de `tests/api.rs` de la tarea 5 (`5b5dbf3`)** se escribieron antes que el código, pero no se corrieron antes. Contra un router sin `/v1/imports`, todas fallan por el `404` sin cuerpo que da el fallback de axum: las que aceptan un trabajo, por la aserción del `202` de `accepted_job` (`crates/clawg-cli/tests/api.rs:4102-4108`); las de rechazo, porque esperan un `400`, o el mismo estado que la ruta síncrona; `an_unknown_job_is_a_404` (`:4468`), porque ese `404` no trae `error`; `the_job_routes_require_the_guard_and_the_token` (`:4553`), porque sin la ruta `require_bearer` no corre y no hay `401`; y `every_route_says_which_project_it_takes`, porque la tabla nombra rutas que el router no tiene. Hay una excepción: sin la ruta, `the_job_route_has_no_body_limit_sized_for_the_file` (`:4538`) puede pasar por su brazo `Err(e) if e.is_request()`. Igual distingue lo que fija: con el tope `IMPORT_BODY_LIMIT` en la ruta, el cuerpo de 3 MiB se lee entero y la respuesta es el `400` de una ruta relativa, no un `413`.

**Las tres pruebas por el binario de la tarea 8 (`febaf12`)** nunca mostraron un RED. Las mutaciones de abajo lo muestran en dos de ellas. La tercera, `killing_clawg_import_does_not_stop_the_job` (`binary.rs:1043`), se juzgó leyéndola:

- con el CLI de antes, que mandaba el texto a `POST /v1/import`, no hay trabajo 1 y la espera de `GET /v1/imports/1` vence;
- con un trabajo atado a la conexión de quien lo sigue, matar el proceso lo cancelaría y `wait_for_job(…, "done")` vencería.

No afirma que el trabajo seguía activo cuando se mató el proceso (ver Sugerencias).

**Las mutaciones de esta verificación.** Se aplicaron cuatro a la vez, cada una a una prueba distinta. Se corrieron con `cargo test --workspace --no-fail-fast --target-dir target-test -- <las cinco pruebas>` y se revirtieron con el editor. `git status` quedó limpio, como estaba.

| Mutación | Prueba | Resultado |
|---|---|---|
| `self.task.abort()` → nada (`crates/clawg-cli/src/daemon/imports.rs:153`) | `an_abandoned_job_sends_nothing_to_the_connection` | falla en `crates/clawg-cli/src/daemon/api.rs:3545`: 3 filas en vez de 0. Confirma lo que anotó la aplicación |
| sin `embedder.wake()` (`api.rs:2290-2292`) | `accepting_a_real_job_wakes_the_embedder_and_a_dry_run_does_not` | falla: «accepting a real job never woke the model» (`tests/api.rs:4399`) |
| la misma | `accepting_a_job_reloads_a_released_model_before_embedding` | **pasa**: no distingue el `wake` (ver WARNING 2). Desde `6c7700a` falla: «the model did not load before the job reached embedding» |
| `shows_progress` ignora `--follow` (`crates/clawg-cli/src/import.rs:126-128`) | `follow_prints_the_phases_in_order_on_stderr_and_the_same_report_on_stdout` | falla en `binary.rs:1030`: `None` en vez de `Some("planning")` |
| `route_missing` siempre `false` (`crates/clawg-cli/src/client.rs:995-998`) | `import_against_a_daemon_without_the_job_route_exits_1_without_sending_the_file` | falla en `binary.rs:1128`: sale `clawg: daemon returned 404: 404 Not Found` en vez de la frase del daemon anterior |

`cargo test --workspace` se corta en el primer binario de pruebas que falla. Para ver varias mutaciones en una corrida hace falta `--no-fail-fast`.

## La medición de memoria (D10)

Ya corrida en la tarea 9 y no repetida acá. En Windows, con el build `testing` de `target-test` y `CLAWG_TEST_EMBEDDER=fake`, un `clawg serve --port 0` nuevo por formato, y un brazo y un formato por proceso (`IMPORT_MEMORY_FORMAT`). Así el pico del cliente es sólo de ese formato. «engram» es el nombre que la prueba da al formato del export de otra herramienta. Las líneas tal como las imprimió `crates/clawg-cli/tests/import_memory.rs`:

```
{"arm":"by_body","format":"clawg","bytes":104858469,"rows":89490,"ok":true,"seconds":142.4944535,"client_peak_before_bytes":3207168,"client_peak_bytes":443416576,"daemon_peak_metric":"process.private_peak_bytes","daemon_peak_bytes":571310080,"daemon_embed":"embed 9.8 s (89490 rows, 2797 batches, 9169.4 rows/s, slowest 2 ms, private 530.9 MiB)"}
{"arm":"by_body","format":"engram","bytes":104857641,"rows":94137,"ok":true,"seconds":239.6191722,"client_peak_before_bytes":3108864,"client_peak_bytes":425783296,"daemon_peak_metric":"process.private_peak_bytes","daemon_peak_bytes":654008320,"daemon_embed":"embed 10.0 s (94137 rows, 2942 batches, 9435.4 rows/s, slowest 2 ms, private 593.5 MiB)"}
{"arm":"by_path","format":"clawg","bytes":104858469,"rows":89490,"ok":true,"seconds":136.2589604,"client_peak_before_bytes":3186688,"client_peak_bytes":3235840,"daemon_peak_metric":"process.private_peak_bytes","daemon_peak_bytes":464977920,"daemon_embed":"embed 9.3 s (89490 rows, 2797 batches, 9631.3 rows/s, slowest 2 ms, private 429.8 MiB)"}
{"arm":"by_path","format":"engram","bytes":104857641,"rows":94137,"ok":true,"seconds":234.809419,"client_peak_before_bytes":3158016,"client_peak_bytes":3207168,"daemon_peak_metric":"process.private_peak_bytes","daemon_peak_bytes":543784960,"daemon_embed":"embed 10.1 s (94137 rows, 2942 batches, 9309.7 rows/s, slowest 2 ms, private 488.4 MiB)"}
```

| Formato (100,0 MiB) | Brazo | Pico del cliente | Pico privado del daemon | Privada al terminar `embed` | Segundos |
|---|---|---|---|---|---|
| clawg, 89.490 filas | por cuerpo | 422,9 MiB (4,23 × el archivo) | 544,8 MiB | 530,9 MiB | 142,5 |
| clawg | por ruta | 3,1 MiB (antes: 3,0) | 443,4 MiB (**−101,4 MiB, −18,6 %**) | 429,8 MiB (−101,1) | 136,3 |
| otra herramienta, 94.137 notas | por cuerpo | 406,1 MiB (4,06 ×) | 623,7 MiB | 593,5 MiB | 239,6 |
| otra herramienta | por ruta | 3,1 MiB (antes: 3,0) | 518,6 MiB (**−105,1 MiB, −16,9 %**) | 488,4 MiB (−105,1) | 234,8 |

**Cómo se lee.** Es lo que la propuesta esperaba en «Cómo se mide»:

- **El cliente por ruta no tiene ninguna copia del archivo.** Su pico no sube de lo que tenía antes de importar. Por cuerpo, el pico es unas 4 veces el archivo: el `String`, el `json!` y el cuerpo.
- **En el daemon baja el tamaño del archivo, y un poco más**, tanto en el pico como en la memoria privada al terminar `embed`. Desaparece el cuerpo del extractor, y el texto ya no está residente después del `202`.
- **El pico del daemon sigue en el parseo.** Un export de otra herramienta queda 75 MiB arriba del de clawg con el mismo tamaño: es el `Value` entero que lee el adaptador.

El pico del daemon se lee de `GET /v1/metrics?format=prometheus`, del último valor de `clawg_process_private_peak_bytes`, durante 70 s después de la importación (`import_memory.rs:180-195`). Se leía así y no de los gauges JSON del minuto en curso porque el escritor toma la muestra y la vacía en el mismo cambio de minuto, así que el minuto en curso nunca la tiene. El pico es monótono y el daemon es nuevo por formato, así que la primera muestra después del final es el pico. La medición no es una prueba de la suite (IJ10).

## Cumplimiento de la spec

Cada escenario contra la prueba que lo sostiene, corrida en verde en esta verificación. ✅ cumple; ⚠️ parcial.

| Requisito | Escenario | Prueba | |
|---|---|---|---|
| IJ1 | se acepta sin esperar | `an_import_job_is_accepted_without_waiting_for_the_embedder` (`tests/api.rs:4116`) | ✅ |
| IJ1 | la ruta no sirve | `a_job_import_refuses_a_path_that_is_not_an_absolute_readable_regular_file_within_the_limit` (`:4151`; el archivo ilegible sólo en Unix) | ✅ |
| IJ1 | un campo que no existe | `a_job_import_refuses_an_unknown_field` (`:4177`) | ✅ |
| IJ1, IJ10 | un parseo, la ruta y no el archivo | `a_clawg_file_is_parsed_once_for_a_job` (`crates/clawg-core/src/memory/portability/import.rs:5619`), `the_client_sends_a_path_and_not_the_file` (`client.rs:2053`) | ✅ |
| IJ2 | las dos rutas dicen lo mismo | `a_job_import_refuses_what_the_synchronous_import_refuses` (`tests/api.rs:4195`), con `409 project_ambiguous` y `503 embedder_failed` desde `51eedc3` | ✅ (era WARNING 3) |
| IJ3 | un segundo POST | `a_second_import_job_while_one_runs_is_refused_with_its_id` (`:4351`) | ✅ |
| IJ3 | el orden de las comprobaciones | `a_job_import_checks_the_body_before_the_running_job_and_the_job_before_the_file` (`:4369`) | ✅ |
| IJ3 | durante el apagado | `a_job_import_during_shutdown_is_refused_with_a_retryable_503` (`api.rs:3553`), `a_closed_registry_refuses_to_reserve` (`imports.rs:576`) | ✅ |
| IJ3 | reserva atómica y `wake` | `a_reservation_is_atomic_and_released_when_dropped` (`imports.rs:424`), `accepting_a_real_job_wakes_the_embedder_and_a_dry_run_does_not` (`tests/api.rs:4385`) | ✅ |
| IJ4 | la fase avanza dentro de embedding | `a_job_reports_its_phase_and_rows_embedded_advance_inside_embedding` (`:4431`) | ✅ |
| IJ4 | el último terminado se recuerda | `only_the_last_finished_job_is_remembered` (`:4454`), `only_the_last_finished_job_is_kept` (`imports.rs:465`) | ✅ |
| IJ4 | una recarga que falla | `a_failing_reload_fails_the_job_without_writing` (`tests/api.rs:4416`) | ✅ |
| IJ4 | una parada antes de escribir no tiene informe | `a_job_stopped_before_its_first_write_has_no_report` (`api.rs:3407`), `a_delete_during_snapshot_ends_cancelled_with_the_snapshot_and_no_rows` (`api.rs:3442`) | ✅ |
| IJ4 | la primera causa gana | `the_first_stop_cause_wins` (`control.rs:147`), `cancelling_after_a_shutdown_keeps_its_cause` y `closing_after_a_cancel_keeps_the_first_cause` (`imports.rs:553`, `:603`) | ✅ |
| IJ4 | el resto | `an_unknown_job_is_a_404`, `a_dry_run_job_ends_done_after_deciding` (`tests/api.rs:4468`, `:4476`), `phase_of_covers_every_step` (`imports.rs:678`), `a_job_import_ends_done_with_the_report_of_the_synchronous_import` (`binary.rs:926`) | ✅ |
| IJ5 | el final coincide con el informe | `a_finished_jobs_counters_match_its_report` (`tests/api.rs:4489`), `the_counters_of_a_finished_import_match_its_report` (`import.rs:5644`) | ✅ |
| IJ5 | la parada en writing | `a_job_stopped_in_writing_counts_exactly_the_committed_batches` (`api.rs:3346`) | ✅ |
| IJ5 | sin depender de los lotes | `the_counters_do_not_depend_on_the_batch_sizes` (`import.rs:5631`), `batch_notes_counts_the_notes_of_each_write_batch` (`:5657`) | ✅ |
| IJ6 | cancelar en embedding | `deleting_a_job_in_embedding_leaves_no_row_no_journal_and_no_snapshot` (`tests/api.rs:4236`), `an_abandoned_embedding_batch_does_not_block_the_stop` (`import.rs:5744`) | ✅ |
| IJ6 | cancelar dos veces y cancelar lo terminado | `a_delete_on_a_finished_job_changes_nothing` (`tests/api.rs:4255`), `cancelling_asks_the_running_job_and_tells_a_finished_one` (`imports.rs:536`) | ✅ |
| IJ6 | cancelar durante snapshot y durante links | `a_delete_during_snapshot_…` y `a_delete_during_links_ends_done` (`api.rs:3442`, `:3430`) | ✅ |
| IJ6 | la tabla de paradas | `each_step_of_an_import_stops_where_the_table_says` (`import.rs:5710`) | ✅ |
| IJ7 | parada a mitad de la escritura | `a_job_stopped_in_writing_leaves_whole_notes_and_frees_its_number` (`api.rs:3360`), `a_stop_between_write_batches_returns_the_committed_ledger` (`import.rs:5766`), `undoing_a_stopped_job_works` (`api.rs:3394`) | ✅ |
| IJ7 | terminar con una reimportación | `importing_again_finishes_a_stopped_job` (`api.rs:3380`); que se rellenen los vínculos que faltaron lo fija `links_lost_between_transactions_are_filled_by_importing_again` (`import.rs:2866`), que ya existía | ✅ |
| IJ8 | el cliente se va | `a_job_outlives_the_client_that_accepted_it` (`tests/api.rs:4278`), `killing_clawg_import_does_not_stop_the_job` (`binary.rs:1043`) | ✅ |
| IJ8 | guarda del síncrono | `the_synchronous_import_still_cancels_when_the_client_goes` (`tests/api.rs:3886`) | ✅ |
| IJ9 | recarga antes de embedding | `accepting_a_job_reloads_a_released_model_before_embedding` (`:4406`), que desde `6c7700a` mira la carga con el trabajo antes de `embedding` | ✅ (era WARNING 2) |
| IJ10 | sin tope de cuerpo para el archivo | `the_job_route_has_no_body_limit_sized_for_the_file` (`:4538`) | ✅ |
| IJ11 | el informe no cambia | `the_report_of_a_job_equals_the_report_of_the_synchronous_import` (`:4513`) y las pruebas de import de antes, sin tocar | ✅ |
| IJ12 | la conexión ocupada | `health_carries_the_active_jobs_import_block_without_the_connection` (`:4298`), `health_has_no_import_block_without_an_active_job` (`:4338`), `a_client_reads_a_health_with_an_import_block` (`client.rs:1854`) | ✅ |
| IJ13 | un `project_map` a un proyecto fusionado o borrado | las tres filas de `PROJECT_INPUTS` (`tests/api.rs:5225-5227`) y los brazos de `POST /imports` (`:5433`, `:5875`) | ✅ |
| IJ14 | el guardia y el token | `the_job_routes_require_the_guard_and_the_token` (`:4553`) | ✅ |
| IJ15 | stdout igual, progreso aparte | `follow_prints_the_phases_in_order_on_stderr_and_the_same_report_on_stdout` (`binary.rs:1015`), `phases_go_to_stderr_only_on_a_terminal_or_with_follow` (`import.rs:1651`) | ✅ |
| IJ15 | Ctrl-C | `ctrl_c_sends_the_delete_and_waits_for_the_final_state`, `a_second_ctrl_c_leaves_without_waiting`, `a_second_ctrl_c_leaves_a_job_that_is_still_stopping` (`import.rs:1783`, `:1692`), con la señal inyectada; `ctrl_c_sends_the_delete_and_prints_what_was_written` (`binary.rs:1140`) queda para el CI de Linux | ✅ |
| IJ15 | el resto | `any_final_state_but_done_exits_1`, `a_vanished_daemon_names_the_last_import_id`, `a_404_on_its_own_job_while_following_is_a_vanished_daemon`, `a_stop_without_a_report_says_nothing_was_written` (`import.rs:1675`, `:1707`, `:1808`, `:1722`), `the_job_poll_lives_outside_the_client_and_sets_no_timeout` (`client.rs:2093`) | ✅ |
| IJ16 | daemon anterior | `import_against_a_daemon_without_the_job_route_exits_1_without_sending_the_file` (`binary.rs:1118`), `route_missing_tells_a_bodyless_404_from_the_daemons_own` (`client.rs:2061`) | ✅ |
| IJ17 | los dos idiomas | `the_job_messages_speak_both_languages` (`import.rs:1524`) | ✅ |
| IJ18 | el lote de embedding que no vuelve | `shutdown_stops_a_job_in_embedding_within_the_grace` (`api.rs:3458`) | ✅ |
| IJ18 | el aborto se nombra | `an_abandoned_job_is_interrupted_and_named` (`imports.rs:628`); el paso, `an_abandoned_job_sends_nothing_to_the_connection` (`api.rs:3502`, en `snapshot`) | ✅ |
| IJ18 | el apagado en writing | `shutdown_stops_a_job_in_writing_between_whole_notes` (`api.rs:3475`), `an_abandoned_import_sends_nothing_after_its_cut` (`import.rs:5791`), `a_finish_after_abandon_changes_nothing` (`imports.rs:665`) | ✅ |
| IJ18, IJ19 | guardas | `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`crates/clawg-cli/src/stop.rs`), `published_migration_checksums_are_pinned` (`crates/clawg-core/src/memory/migrations.rs`); el rango no toca ninguna migración | ✅ |
| R3, HI16 (modificados) | el import por el binario y los mapas | `export_and_import_round_trip_through_the_binary`, `an_engram_export_imports_through_the_binary`, `a_project_map_must_name_a_directory`, `the_maps_are_sent_with_their_raw_paths` | ✅ (WARNING 1 por el texto) |

**31 de 31 escenarios cumplen** desde `6c7700a` y `51eedc3`. En la verificación, sobre `788408e`, eran 29: IJ2 e IJ9 cumplían en parte. Ningún escenario queda sin prueba.

## El diseño

D1 a D11 se siguen. Las desviaciones que anotó la aplicación están en el código y no rompen ningún requisito:

- **`Reservation::run`** (`imports.rs:314-320`) devuelve `Result<Arc<JobView>, Busy>`. Lanza la tarea bajo el lock, guarda el `AbortHandle` y arma la vista después de lanzar (`Jobs::run`, `:220-233`). Como el lock sigue tomado, la tarea no puede terminar antes de que el registro la conozca, y la vista del `202` dice `planning`: `Plan` y «sin paso» dan la misma fase (`phase_of`, `:77-89`). Si la reserva ya no es de la ranura, porque el apagado la abandonó, es `Busy::Closing` (`:225-228`).
- **`abandon(now_ms)`** (`:367-378`) deja su propia línea `warn` con el paso.
- **La guarda `Ending`** (`api.rs:2324-2335`) termina el trabajo como `failed` si su tarea muere sin resultado, por ejemplo con un pánico. Tras un `abandon`, su `finish` no cambia nada: la ranura ya no es la suya (`a_finish_after_abandon_changes_nothing`).
- **`cancel`** devuelve `Option<Cancel>`, con `Asked` para el `202` y `Over` para el `200` (`imports.rs:277-286`, `api.rs:2384-2390`). **`active()`** da sólo una ranura `Running` (`imports.rs:270-275`): una reserva no es todavía un trabajo, que es como IJ12 entiende «trabajo activo».
- **`an_abandoned_job_sends_nothing_to_the_connection` no llama a `close()`**, aunque D9 lo nombra. Con `close()`, el trabajo pararía solo después del snapshot (`stopped_after`, `import.rs:2425`), y la prueba pasaría también sin el aborto. Sin `close()`, sólo el aborto lo frena, y la mutación de arriba lo muestra: 3 filas sin `abort()`. Es más fuerte que lo que pedía el diseño.
- **El CLI.** `Event::Lost` no lleva datos (`crates/clawg-cli/src/import.rs:78`). `run` recibe `Watch` y el receptor de `interrupts` (`:326`), y `main` crea ese receptor antes del `POST` (`main.rs:253-254`). `ImportJob` lleva `dry_run` (`client.rs:750-754`), que `progress_line` usa para no contar filas escritas de una pasada en seco. `std::path::absolute` es `import.rs:344`. **Cualquier error** del sondeo o del `DELETE` es `Lost` (`:401`, `:410`). IJ15 pide tratar como desaparición al daemon que se fue y al `404` de su propio trabajo, y no prohíbe más. Los errores posibles en ese lazo son de transporte, un `401` (otro daemon con otro token) o un `404`, así que en la práctica todos son el daemon o el trabajo que ya no están (ver Sugerencias).
- **D10.** El pico del daemon se lee del texto Prometheus, y `IMPORT_MEMORY_FORMAT` deja correr un formato por proceso. Las dos cosas son de la medición, que no es una prueba de la suite, y ningún requisito depende de ellas.

`serve.rs` cierra el registro al observar la señal y antes de avisar a los servidores (`crates/clawg-cli/src/daemon/serve.rs:262`). Espera al trabajo como tercer futuro del `try_join!` (`:286-294`) y lo abandona cuando `within_grace` vence (`:302`), como D7. Ningún término de la suma de `stop.rs` cambió.

## `.claude/CLAUDE.md`

Dicen lo que hace el código final:

- la entrada de `clawg import`: `[--follow]`, la ruta absoluta, el sondeo de `POLL_EVERY`, stderr y stdout, Ctrl-C, los finales y sus códigos, `gone`, el `409` y el daemon anterior;
- `POST`, `GET` y `DELETE /v1/imports`, con el orden de las comprobaciones, `job_id` en el `409 import_running`, `503 shutting_down` con `retryable: true`, la vista y los `200`, `202` y `404`;
- el bloque `import` de `/health`;
- el párrafo del apagado de `clawg serve`.

No quedan frases viejas: ni «manda el archivo» ni nada de `DaemonClient::import`, `ImportError::Read` o `TooLarge`. El `README.md` no dice nada falso. Tampoco nombra el progreso, Ctrl-C ni que matar el comando deja el trabajo corriendo (ver Sugerencias).

## Reglas

- **Nada prohibido en el rango.** No hay `#[allow]`. Ningún comentario, nombre de prueba o mensaje nombra un plan, una fase de trabajo, la spec, las tareas o los números IJ o D, y no aparece ninguna persona ni ruta de un home.
- **Sin dependencias nuevas.** No cambia ningún `Cargo.toml` ni `Cargo.lock`.
- **Lo de prueba no entra en el release.**
  - `GatedEmbedder` y `Gate` están bajo `cfg(any(test, feature = "testing"))` (`crates/clawg-core/src/memory/embedder.rs:891-963`).
  - `ImportControl::on_step` también (`crates/clawg-core/src/memory/portability/control.rs:98-101`), y su llamada en `begin` (`:109-112`).
  - `import`, `import_foreign`, `finished` y el `parse` que usan son `#[cfg(test)]` (`import.rs`).
  - El clippy de lo que publica el release sale limpio.

## Lo que encontró la verificación

**CRITICAL:** ninguno.

**WARNING:**

1. **La spec dice que modifica R3 y HI16, pero no trae su texto nuevo** (`openspec/changes/import-job/spec.md:3`). La spec principal sigue diciendo que `clawg import` «MUST enviar el archivo a `POST /v1/import`» (`openspec/specs/portability/spec.md:67`) y «MUST mandar el archivo a un daemon en marcha» (`:340`). Hoy manda la ruta absoluta a `POST /v1/imports` y sigue el trabajo (`crates/clawg-cli/src/import.rs:344-373`). Si el archivo se hace con lo que hay, la spec principal contradice al código. La tarea 12 tiene que escribir el texto modificado de los dos. No es un defecto del código.
2. **IJ9, «el contador de cargas sube antes de que `phase` sea `embedding`», no lo sostiene ninguna prueba.** `accepting_a_job_reloads_a_released_model_before_embedding` (`tests/api.rs:4406-4413`) sólo afirma una carga al final, y sigue pasando sin el `wake()` de `api.rs:2290-2292` (mutación de arriba). El `wake` al aceptar lo sostiene `accepting_a_real_job_wakes_the_embedder_and_a_dry_run_does_not` (`:4385`), que importa un archivo sin filas y falla sin él. El orden frente a `embedding` no lo afirma nadie. Una prueba con `Gated` sobre un modelo soltado que mire `reloads` cuando el trabajo llega a `embedding` lo cerraría. **Cerrado en `6c7700a`:** la prueba retiene la única conexión del store, así que el trabajo aceptado no puede planear; espera a que `reloads` llegue a 1 y comprueba que la fase todavía no es `embedding`. Sin el `wake()` falla, con él pasa, y `api.rs` quedó sin tocar.
3. **La tabla de IJ2** (`tests/api.rs:4195-4228`) no tiene los casos `409 project_ambiguous` ni `503 embedder_failed`, que el requisito nombra. El `409 project_deleted` está en el recorrido de proyectos borrados (`:5875`). Los dos que faltan pasan por las mismas piezas que la ruta síncrona (`checked_fields`, `api.rs:2081`; `embedder_for`, `:2137`), así que el riesgo es bajo. En lugar de mirar que `/health` no tiene trabajo, la prueba comprueba que se acepta un trabajo después de cada rechazo, lo que dice lo mismo. **Cerrado en `51eedc3`:** la comparación pasó a `both_routes_refuse`, y los dos casos corren en un arnés propio (dos proyectos vivos llamados «Twin»; `Harness::start_failed`), con el mismo estado y el mismo código en las dos rutas. Pasaron a la primera, porque cubren lo que ya hacía el código.

**SUGGESTION:**

- **Un error pasajero se lee como un daemon que se fue.** `follow` trata como `Lost` cualquier error (`import.rs:401`, `:410`), así que un error de transporte con el daemon vivo imprimiría «the daemon went away … nothing was written» mientras el trabajo sigue y escribe. Reintentar una vez antes de `Lost`, o reservar `gone` para el transporte, el `401` y el `404`, lo evitaría. **Arreglado en `a8ea3dd`**, porque le decía al usuario algo falso sobre sus datos: `gone` queda para una conexión rechazada, un token rechazado y un `404` sobre su propio trabajo (`ClientError::means_daemon_gone`); cualquier otro error se pregunta otra vez, o se manda otra vez el `DELETE`, y tras cinco seguidos (`MAX_UNANSWERED`) el CLI sale con 1 diciendo que la importación puede seguir corriendo y que `clawg status` la muestra. La spec suma el escenario «un error pasajero» a IJ15. Las pruebas fallaron primero por no compilar, y la suite y los dos clippy pasan.
- **El camino de error de los servidores no cierra el trabajo.** Si un servidor termina con error antes de la señal, `try_join!` corta (`serve.rs:294-296`, `within_grace`, `:443-450`). Ahí el trabajo no se cierra ni se abandona, y sigue mandando a la conexión durante el drenaje y el cierre. Es teórico, porque `axum::serve` reintenta los errores de `accept`, pero un `close()` y un `abandon` en ese camino lo cerrarían.
- **Ninguna prueba ejercita el cableado de `serve.rs`**: el `close` en la señal, el tercer futuro y el `abandon`. Una prueba por el binario que corra `clawg stop` con un trabajo de 50.000 filas en curso lo cubriría.
- **`killing_clawg_import_does_not_stop_the_job` no afirma que el trabajo seguía activo cuando se mató el proceso** (`binary.rs:1056-1062`). Con 2.000 filas y unos 10 ms entre la aceptación y el `kill` lo está, pero la prueba no lo dice.
- **Una ruta que no es Unicode ya no se importa.** El CLI manda `path.to_string_lossy()` (`import.rs:347`), así que el daemon contesta «no such file». Antes, el cliente leía el archivo él mismo.
- **`a_relative_path_is_sent_as_the_absolute_one` puede no probar nada en el CI de Windows** (`import.rs:1284-1307`). Si el temporal y el directorio de trabajo están en unidades distintas, `pathdiff` arma una ruta absoluta y la prueba pasa sin haber mandado una relativa.
- **Ninguna prueba HTTP afirma la `phase` del cuerpo del `202`.** La fijan la prueba del registro (`a_running_job_shows_what_was_asked_of_it`, `imports.rs:490`) y, en Unix, la primera línea de stderr de la prueba de Ctrl-C.
- **El escenario de IJ16 dice que el daemon «no recibió el archivo ni su ruta»**, pero el primer `POST` lleva la ruta, como dice D8. La prueba sostiene la lectura del diseño, que es que el contenido no viaja. El escenario se puede corregir al archivar.
- **El `README.md`** (sección de `clawg import`) podría nombrar el progreso, Ctrl-C y que matar el comando deja la importación corriendo en el daemon. **Hecho en `6d5497d`.**
- Las demás sugerencias quedan como están: una ruta que no es Unicode termina en un rechazo y no en un daño, y las rutas de `--map-root` y `--project-map` ya viajaban así; el cableado de `serve.rs` sin prueba por el binario, el error de los servidores antes de la señal, la aserción de la prueba del `kill`, la ruta relativa en otra unidad y la `phase` del `202` no cambian ninguna conducta, y las escrituras van en transacciones, así que un fallo del cableado cortaría más brusco sin dejar notas partidas.

## CI

El ítem queda abierto: hace falta un push, y sólo se hace con autorización. El CI de Linux y Windows tiene que confirmar:

- **`ctrl_c_sends_the_delete_and_prints_what_was_written`** (`binary.rs:1140-1168`), que no corrió nunca. Leída para Linux:
  - compila: usa `BufRead` y `BufReader` del encabezado (`:17`), un `Read` local, `libc::pid_t`, `libc::kill` y `libc::SIGINT` como la prueba de SIGTERM (`:576-590`), y `libc` es dependencia `cfg(unix)` de `clawg-cli`;
  - la señal no puede llegar antes del manejador: `interrupts()` se lanza antes del `POST` en un runtime multihilo (`main.rs:74`, `:253`), y la prueba manda SIGINT recién después de leer la primera línea de stderr, que sale después del `202`;
  - con 50.000 filas, el `DELETE` llega en `plan`, `prepare` o `load`, el trabajo termina `cancelled`, el CLI imprime «stopped in …: nothing was written» por stdout y sale con 1;
  - lo que el hijo escribe antes de que se lean sus tuberías son unas pocas líneas.
- **El caso Unix de `a_job_import_refuses_a_path_…`** (`tests/api.rs:4137-4143`): un archivo en modo `000` da un `400` «cannot be read» cuando el CI no corre como root, y se saltea si lo hace.
- **`a_relative_path_is_sent_as_the_absolute_one` en Linux**, donde `std::path::absolute` conserva los `..`, que el sistema resuelve al abrir.
- **El clippy de Linux con `-D warnings`** sobre el código `cfg(unix)` del rango, que el clippy de Windows no compila: `unreadable_file` y la prueba de Ctrl-C.
- **La suite entera en Linux**, con `binary.rs`.

## Lo que no se corrió

- **Linux en local:** no se corrió. Lo cubre el CI.
- **La medición de memoria:** no se repitió. Los números son los de la tarea 9.
- **Los perfiles reales:** no se tocó ninguno.

## Veredicto

**PASS WITH WARNINGS.** La suite y los dos clippy pasan en Windows. Cada requisito tiene prueba, y las desviaciones del diseño no rompen ninguno. Hubo tres avisos: el texto de R3 y HI16 que falta para archivar, el orden de IJ9 sin prueba y dos casos de la tabla de IJ2. Los dos últimos los cerraron `6c7700a` y `51eedc3`, sólo con pruebas, con la suite y los dos clippy en verde (la falla intermitente de `hook` apareció una vez y pasó al repetir). Queda el primero, que es del archivo. Falta el CI de Linux, sobre todo la prueba de Ctrl-C.

## Al archivar (2026-10-30)

- **WARNING 1, cerrado.** R3 y HI16 de [`specs/portability`](../../../specs/portability/spec.md) dicen ahora que `clawg import` manda la ruta absoluta a `POST /v1/imports` y sigue el trabajo, y que `POST /v1/import` queda igual para otros clientes. El escenario de IJ16 dice que el daemon recibe la ruta y nada del contenido del archivo, que es lo que sostiene la prueba.
- **Linux en local, otra vez no.** `cargo clippy --locked --workspace --all-targets` y `cargo test --locked --workspace` no compilan en el entorno del archivo: el build de `ort-sys` baja ONNX Runtime de `cdn.pyke.io`, y la red del entorno lo rechaza (`403` del proxy). No es del código. Linux y Windows, la prueba de Ctrl-C incluida, quedan para el CI del pull request que archiva el cambio.
