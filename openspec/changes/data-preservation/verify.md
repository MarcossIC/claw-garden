# data-preservation — Verificación (partes A + F)

**Código:** PR #5, rama `claude/apply-data-preservation-ao9liz`, 10 commits, uno por tarea.
**Estado:** los diez requisitos tienen su test en verde. Queda pendiente un paso de la prueba manual (ver al final).

## Suite

`cargo test --workspace --no-fail-fast` en Linux:

| | Passed | Failed |
|---|---|---|
| `main` (antes) | 1395 | 12 |
| PR #5 | 1418 | 11 |

- **+23 tests pasando:** 21 nuevos, más `write_failure_returns_workspace_io_error`, que ahora es portable y también pasa en Linux.
- **Los 11 que fallan, fallan igual en `main`.** Son tests que sólo pasan en Windows: `doctor::tests::*` (7) y los fixtures de `rag-eval` en `diff`/`report` (4). Ninguno toca este cambio.
- **La base de 1304 de `config.yaml`** se midió en Windows. En Linux la suite tiene más tests y estos 11 fallos de plataforma. Falta correrla en Windows para comparar contra esa cifra.
- **Warnings:** los mismos 9 que en `main`.
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

La hice con `HOME` apuntando a un directorio temporal, no con el perfil dev real. El binario fue el debug.

1. Arranqué `clawg serve --port 0` y guardé dos memorias por HTTP mientras el modelo se descargaba. Respondieron `queued`.
2. Corrí `clawg stop`, que terminó en 0,3 s. Quedó `ingest-pending.jsonl` con las 2 líneas, el `memory.db-wal` ya no existía, y el log dice "saved 2 events not yet ingested" y "checkpointed the WAL".
3. Volví a arrancar. El log dice "replaying 2 spilled ingest events". El modelo no se pudo descargar (en este entorno está bloqueado el host del modelo), así que el worker quedó esperándolo.
4. Corrí `clawg stop` otra vez. El volcado se reescribió con las mismas 2 líneas.

**Pendiente:** ver las filas en la base y el volcado borrado con el modelo real. Hace falta un entorno que pueda bajar el modelo, o el perfil dev con autorización. Los tests de R2 cubren ese camino con el embedder de prueba.
