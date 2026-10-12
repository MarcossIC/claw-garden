# data-preservation — Especificación (partes A + F)

**Dominio destino:** `specs/storage/spec.md` (se fusiona al archivar; hoy `openspec/specs/` está vacío, así que es una spec completa, no un delta).

**Alcance:** el apagado del daemon (`serve.rs`, `MemoryRuntime`, el worker de ingesta y el `AccessTracker`), el volcado y la reinyección de eventos sin ingerir, la resolución del directorio de datos, el runner de migraciones y la escritura de `workspace.md`. No cambia el esquema.

## Definiciones

- **Evento aceptado:** un `MemoryEvent` que entró al canal de ingesta. Para el agente es un save que respondió `queued`.
- **Apagado ordenado:** el que empieza con Ctrl-C o con `POST /v1/shutdown`. Un kill del proceso no es un apagado ordenado.
- **Plazo de drenaje (`DRAIN_DEADLINE`):** el tiempo máximo que el apagado dedica a ingerir lo encolado.
- **Volcado:** el archivo `<data>/ingest-pending.jsonl`, un `MemoryEvent` en JSON por línea.

## R1. Ningún evento aceptado se pierde en un apagado ordenado

En un apagado ordenado, cada evento aceptado MUST terminar en uno de dos lugares: insertado en la base (o absorbido por el gate de dedup), o escrito en el volcado. El apagado MUST NOT descartar un evento.

#### Escenario: embedder listo
- GIVEN el embedder cargado y 20 eventos en el canal
- WHEN se hace un apagado ordenado
- THEN la base tiene las filas de los 20 eventos y no existe el volcado

#### Escenario: embedder sin cargar
- GIVEN el embedder sin cargar y 3 eventos en el canal (uno retenido por `wait_for_embedder`)
- WHEN se hace un apagado ordenado
- THEN el volcado tiene 3 líneas, en el orden en que entraron, y el log no dice "dropped"

#### Escenario: vence el plazo
- GIVEN un embedder que tarda más que `DRAIN_DEADLINE` en procesar los eventos encolados
- WHEN se hace un apagado ordenado
- THEN el evento en curso termina su insert, y los que no empezaron quedan en el volcado

**Tests:** `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_spills_events_held_for_an_embedder_that_never_loaded`, `shutdown_spills_what_the_drain_deadline_left_unstarted` (integración de `clawg-core`, con `MemoryRuntime::new_for_test` y un embedder de prueba controlable). Fallan hoy: no hay apagado que esperar, y el worker descarta con un warn (`worker.rs:152-159`).

## R2. Reinyección al arrancar

Si el volcado existe al arrancar, el worker MUST procesar sus eventos antes que cualquier evento nuevo del canal, en el orden del archivo. MUST borrar el volcado sólo cuando procesó todas sus líneas. Si un apagado ordenado llega a mitad de la reinyección, el volcado nuevo MUST contener las líneas no procesadas del anterior, seguidas de lo encolado.

Una línea que no se puede parsear MUST NOT bloquear las demás: se registra con un warn y se mueve a `<data>/ingest-pending.rejected.jsonl`, que nada borra.

#### Escenario: reinyección
- GIVEN un volcado con 2 eventos y un save nuevo en el canal
- WHEN arranca el worker con el embedder listo
- THEN los 3 quedan en la base, los 2 del volcado con ids menores que el nuevo, y el volcado ya no existe

#### Escenario: línea corrupta
- GIVEN un volcado con una línea válida, una truncada y otra válida
- THEN se ingieren las dos válidas, y la truncada queda en `ingest-pending.rejected.jsonl`

#### Escenario: apagado durante la reinyección
- GIVEN un volcado de 3 eventos, el embedder sin cargar y 1 save nuevo
- WHEN se hace un apagado ordenado
- THEN el volcado tiene 4 líneas: las 3 anteriores y luego la nueva

**Tests:** `spilled_events_are_ingested_first_on_startup`, `a_corrupt_spill_line_is_set_aside_and_the_rest_ingested`, `a_shutdown_during_replay_keeps_the_unreplayed_lines_first`. Los dos primeros fallan hoy porque nada lee el volcado; el tercero, porque nada lo escribe.

## R3. Formato del volcado

Las funciones que serializan y parsean el volcado MUST ser puras (sin I/O). El volcado MUST escribirse de forma atómica: archivo temporal en el mismo directorio y `rename`. Un evento MUST sobrevivir ida y vuelta sin cambios, incluidos `memory_type` e `importance`.

#### Escenario
- GIVEN un `MemoryEvent` explícito con `memory_type = "decision"`, `importance = 8.0` y texto con saltos de línea
- WHEN se serializa y se parsea
- THEN el resultado es igual al original y ocupa una sola línea

**Test:** `spill_lines_round_trip_events_with_newlines_and_overrides` (unit, módulo del volcado). Falla hoy: el módulo no existe.

## R4. Flush final del `AccessTracker`

En un apagado ordenado, el tracker MUST escribir el buffer pendiente antes de terminar, y el apagado MUST esperar a que termine.

#### Escenario
- GIVEN un hit registrado con `record` hace menos de 30 s
- WHEN se hace un apagado ordenado
- THEN el `access_count` de esa fila subió en 1

**Test:** `access_tracker_flushes_its_buffer_on_shutdown` en `access.rs`. Falla hoy: sale del loop sin flush (`access.rs:68`).

## R5. Checkpoint al terminar

Después de drenar y del flush del tracker, el apagado SHOULD hacer `PRAGMA wal_checkpoint(TRUNCATE)`. Un fallo del checkpoint MUST registrarse y MUST NOT impedir el resto del apagado.

#### Escenario
- GIVEN una base en WAL con escrituras recientes
- WHEN se hace un apagado ordenado
- THEN `memory.db-wal` no existe o mide 0 bytes, y una copia de `memory.db` sola contiene las escrituras

**Test:** `shutdown_checkpoints_the_wal_into_memory_db` (integración de `clawg-core`, con una base en disco en un tempdir). Falla hoy: no hay checkpoint.

## R6. El plazo de drenaje cabe en `STOP_TIMEOUT`

`DRAIN_DEADLINE` más un margen para el volcado, el flush y el checkpoint MUST ser menor que `STOP_TIMEOUT` (`stop.rs:15`). El doc de `STOP_TIMEOUT` MUST describir lo que el apagado hace.

**Test:** `the_drain_deadline_leaves_room_inside_the_stop_timeout` en `stop.rs`. Falla hoy: `DRAIN_DEADLINE` no existe.

## R7. Directorio de datos sin home

Si no se puede resolver el home, `data_dir` MUST devolver un error tipado que lo diga, y MUST NOT usar el directorio actual. `clawg serve`, `clawg mcp`, `clawg status` y `clawg stop` MUST terminar con ese mensaje y sin crear ningún archivo.

#### Escenario
- GIVEN un resolvedor de home que devuelve `None`
- WHEN se calcula el directorio de datos
- THEN el resultado es `Err` y el mensaje nombra el home

**Test:** `data_dir_without_a_home_is_an_error` en `paths.rs`. La resolución se separa en una función pura que recibe el home (`Option<PathBuf>`), para no depender del entorno. Falla hoy: devuelve `./.clawg` (`paths.rs:148-152`).

## R8. Migraciones atómicas

Las migraciones pendientes de un arranque MUST aplicarse en una sola transacción junto con sus filas de historial. Si una falla, la base MUST quedar como estaba antes del arranque.

#### Escenario: fallo a mitad
- GIVEN una base vacía y la lista V1–V8 más una V9 cuyo SQL falla
- WHEN se migra con la configuración de producción
- THEN no existe la tabla `memory` ni ninguna fila en `refinery_schema_history`

#### Escenario: base existente
- GIVEN una base migrada a V8 por el runner actual, con filas
- WHEN se abre con el runner agrupado
- THEN no se aplica nada y las filas siguen igual

**Tests:** `a_failing_migration_rolls_back_every_migration_of_its_run` y `a_v8_database_opens_unchanged_under_grouped_migrations` en `db.rs`. El primero falla hoy: deja aplicadas V1–V8. El segundo protege la regla de `config.yaml` y pasa hoy y después.

## R9. Checksums fijados

Los checksums de V1–V8 MUST coincidir con una lista fija en el código. Cambiar una migración publicada MUST romper un test.

**Test:** `published_migration_checksums_are_pinned` en `migrations.rs`. Falla hoy: no existe la lista.

## R10. Escritura atómica de `workspace.md`

`regenerate_auto_section` (`workspace.rs:57`) MUST escribir a un archivo temporal en el mismo directorio y hacer `rename` sobre `workspace.md`. Si la escritura falla, `workspace.md` MUST quedar con su contenido anterior, sección USER incluida.

#### Escenario
- GIVEN un `workspace.md` con una sección USER escrita a mano
- WHEN la escritura del temporal falla
- THEN `workspace.md` es idéntico al anterior y no queda ningún temporal

**Test:** `a_failed_workspace_write_leaves_the_previous_file_intact` en `workspace.rs`. Falla hoy: `tokio::fs::write` trunca el destino antes de escribir (`workspace.rs:89`).
