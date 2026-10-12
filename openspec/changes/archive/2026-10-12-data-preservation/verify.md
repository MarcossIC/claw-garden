# data-preservation — Verificación (partes A + F)

**Código:** PR #5, 10 commits, uno por tarea.
**Estado:** verificado. Los diez requisitos tienen su test en verde en Linux y en Windows, y la prueba manual pasó completa en Windows con el perfil dev (2026-10-12). Queda un hallazgo sin pérdida de datos, que pasa al backlog (ver al final).

## Suite

`cargo test --workspace --no-fail-fast` en Linux:

| | Passed | Failed |
|---|---|---|
| `main` (antes) | 1395 | 12 |
| PR #5 | 1418 | 11 |

- **+23 tests pasando:** 21 nuevos, más `write_failure_returns_workspace_io_error`, que ahora es portable y también pasa en Linux.
- **Los 11 que fallan, fallan igual en `main`.** Son tests que sólo pasan en Windows: `doctor::tests::*` (7) y los fixtures de `rag-eval` en `diff`/`report` (4). Ninguno toca este cambio.
- **La base de 1304 de `config.yaml`** se midió en Windows. En Linux la suite tiene más tests y estos 11 fallos de plataforma.
- **Warnings:** los mismos 9 que en `main`.

`cargo test --workspace --no-fail-fast --target-dir target/verify-win` en Windows, sobre `main` en `791ad6b` (ya incluye ingest-durability y retrieval-hygiene):

| | Passed | Failed |
|---|---|---|
| `main` (`791ad6b`) | 1479 | 1 |

- Los 7 de `doctor::tests` que fallan en Linux pasan acá.
- **El único que falla** es `report::tests::a_committed_results_file_without_the_block_still_diffs_against_one_with_it` (`rag-eval`). Lee `docs/eval/results/2026-08-15-fake.json`, que está fuera de git (`.gitignore`: `docs/eval/**`) y no existe en esta máquina. Es de la familia de los 4 de `rag-eval` que fallan en Linux por la misma razón, y no toca este cambio.
- Contra la base de 1304: +175 tests, que suman este cambio y los que se mergearon después de medir esa base.
- **Warnings:** 9, los mismos.
- **`rag-eval`:** no aplica, no cambia ranking ni recuperación.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Ningún evento aceptado se pierde | `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_spills_events_held_for_an_embedder_that_never_loaded`, `shutdown_spills_what_the_drain_deadline_left_unstarted` (`crates/clawg-core/tests/shutdown.rs`), más 3 unit de `drain_step` en `worker.rs` | ✓ |
| R2. Reinyección al arrancar | `spilled_events_are_ingested_first_on_startup`, `a_corrupt_spill_line_is_set_aside_and_the_rest_ingested`, `a_shutdown_during_replay_keeps_the_unreplayed_lines_first` | ✓ |
| R3. Formato del volcado | `spill_lines_round_trip_events_with_newlines_and_overrides`, `invalid_spill_lines_are_set_apart_and_the_valid_ones_keep_their_order`, `write_replaces_the_spill_file_with_exactly_the_given_events` (`spill.rs`) | ✓ |
| R4. Flush final del tracker | `access_tracker_flushes_its_buffer_on_shutdown`, `access_tracker_flushes_its_buffer_when_every_sender_is_dropped`, `shutting_down_twice_is_harmless` | ✓ |
| R5. Checkpoint | `shutdown_checkpoints_the_wal_into_memory_db` | ✓ |
| R6. Plazo dentro de `STOP_TIMEOUT` | `the_drain_deadline_leaves_room_inside_the_stop_timeout` (`stop.rs`) | ✓ |
| R7. Sin home | `data_dir_without_a_home_is_an_error`, `data_dir_in_a_home_is_the_profile_directory_under_it` (`paths.rs`) | ✓ |
| R8. Migraciones atómicas | `a_failing_migration_rolls_back_every_migration_of_its_run`, `a_v8_database_opens_unchanged_under_grouped_migrations` (`db.rs`) | ✓ |
| R9. Checksums fijados | `published_migration_checksums_are_pinned` (`migrations.rs`) | ✓ |
| R10. `workspace.md` atómico | `a_failed_workspace_write_leaves_the_previous_file_intact` (`workspace.rs`) | ✓ |

**RED comprobado:** cada test falló antes de su código, salvo dos casos que el diseño ya preveía o que cambió el orden de las tareas:

- `a_v8_database_opens_unchanged_under_grouped_migrations` es un test de protección: pasa antes y después, como dice la spec.
- `the_drain_deadline_leaves_room_inside_the_stop_timeout` pasó desde el principio: `DRAIN_DEADLINE` ya existía desde la tarea 7.

## Desvíos del diseño

| Diseño | Implementado | Por qué |
|---|---|---|
| D2: la señal es `watch::Receiver<bool>` | `watch::Receiver<Option<Instant>>`: la señal lleva el instante límite del drenaje | El que orquesta el apagado decide el plazo, y un test puede usar uno corto (`shutdown_within`) sin tocar la constante |
| D2: `MemoryRuntime::shutdown(self, handles)` | `shutdown(&self, Workers)`; `Workers` agrupa el sender de la señal y los `JoinHandle` de ingesta y mantenimiento | En `serve.rs` el runtime vive dentro de `Arc<AppCtx>`, así que no se puede tomar por valor |
| D3: el worker borra el volcado al terminar la reinyección | Igual. Además, si no se pueden guardar las líneas corruptas en `ingest-pending.rejected.jsonl`, el volcado no se borra, para no perder su única copia | — |
| D6: el error de `paths.rs` con `thiserror` | `PathsError { NoHome, Profile }`. `data_dir_from_env` pasa a devolverlo | El encabezado del módulo decía "sólo std y dirs". Lo actualicé para nombrar `thiserror` |
| — | Un dreno pendiente cuando se sueltan todos los `Sender` con el embedder sin cargar devuelve los eventos en vez de descartarlos | Sólo pasa fuera del daemon (tests, eval). Antes se descartaban con un warn |

## Prueba manual

### En Linux, con un `HOME` temporal

La hice con `HOME` apuntando a un directorio temporal, no con el perfil dev real. El binario fue el debug.

1. Arranqué `clawg serve --port 0` y guardé dos memorias por HTTP mientras el modelo se descargaba. Respondieron `queued`.
2. Corrí `clawg stop`, que terminó en 0,3 s. Quedó `ingest-pending.jsonl` con las 2 líneas, el `memory.db-wal` ya no existía, y el log dice "saved 2 events not yet ingested" y "checkpointed the WAL".
3. Volví a arrancar. El log dice "replaying 2 spilled ingest events". El modelo no se pudo descargar (en este entorno está bloqueado el host del modelo), así que el worker quedó esperándolo.
4. Corrí `clawg stop` otra vez. El volcado se reescribió con las mismas 2 líneas.

### En Windows, con el perfil dev (2026-10-12)

En Windows `HOME` no aísla el directorio de datos: `dirs-sys` 0.5.0 lee `HOME` sólo bajo `cfg(unix)` y en Windows usa `SHGetKnownFolderPath`. Por eso la prueba se hizo sobre `~/.clawg-dev`, con autorización y con un backup de `memory.db` tomado antes. Binario debug de `main` en `791ad6b`, con el modelo real ya descargado.

1. Arranqué `clawg serve --profile dev` y guardé dos memorias por HTTP mientras `/health` decía `embedder: loading`. Las dos respondieron `202 {"status":"queued","reason":"embedder_loading"}`.
2. `clawg stop` salió con 0 a los 9,5 s. El log dice "ingest stopped with 2 events not yet ingested", "saved 2 events not yet ingested to …ingest-pending.jsonl" y "checkpointed the WAL into memory.db". El volcado quedó con 2 líneas y no había `memory.db-wal`.
3. Volví a arrancar. El log dice "replaying 2 spilled ingest events", después "embedder bootstrap: done" y "replayed every spilled ingest event". `GET /v1/memories?agent=manual-check` devolvió las dos filas (ids 9 y 10, `active`). `ingest-pending.jsonl` ya no existía y no se creó `ingest-pending.rejected.jsonl`.
4. Borré las dos filas de prueba con `DELETE /v1/memories/memory/{id}` (`{"deleted":1}` cada una) y paré el daemon.

R1 (embedder sin cargar), R2 (reinyección), R5 y R6 quedan confirmados con el modelo real.

## Desvío de redacción en la spec

R6 dice que `DRAIN_DEADLINE` más el margen "MUST ser menor que" `STOP_TIMEOUT`. El test comprueba `<=` (`stop.rs:66`), y hoy los dos valen 15 s (10 s + 5 s de `AFTER_DRAIN`). El margen ya es el colchón, así que manda el test. La spec consolidada (`openspec/specs/storage/spec.md`) dice "MUST NOT superar".

## Hallazgo: el proceso sigue vivo mientras carga el modelo

En la prueba manual, si el apagado llega mientras carga el modelo, el apagado ordenado termina en 1–3 s y el proceso sigue vivo hasta que vuelve la carga. Con el binario debug eso fueron 8–13 s.

- **Causa:** la carga corre en `spawn_blocking` (`crates/clawg-core/src/memory/bootstrap.rs:318`), y el runtime de `#[tokio::main]` (`crates/clawg-cli/src/main.rs:30`) espera a esas tareas al soltarse. La doc de tokio 1.51 (`runtime.rs:39-44`) dice que siguen "until they return" y que el `Drop` "waits forever for this".
- **Efecto:** en una corrida con la cola vacía, `clawg stop` llegó a `STOP_TIMEOUT` y salió con 1 ("accepted the shutdown but is still running after 15s"). En la otra salió con 0 a los 9,5 s. Después de "stopped", el log dice además "model load task panicked: task N was cancelled" y "retrying in 30s", y eso confunde.
- **Datos:** no corren riesgo. El volcado y el checkpoint terminan antes.
- **Alcance:** según `bootstrap.rs:313` la carga dura unos 100 ms en release, así que se ve sobre todo en debug.
- **Decisión:** no bloquea el archivo. Pasa al backlog de `openspec/README.md`.
