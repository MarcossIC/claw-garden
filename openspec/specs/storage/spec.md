# storage — Especificación

Fuente de verdad del comportamiento actual del almacenamiento: el apagado del daemon, el volcado y la reinyección de eventos sin ingerir, la resolución del directorio de datos, el runner de migraciones y la escritura de `workspace.md`.

**Origen:** [data-preservation](../../changes/archive/2026-10-12-data-preservation/proposal.md) (partes A + F), archivado el 2026-10-12.

## Definiciones

- **Evento aceptado:** un `MemoryEvent` que entró al canal de ingesta.
- **Apagado ordenado:** el que empieza con Ctrl-C o con `POST /v1/shutdown`. Un kill del proceso no es un apagado ordenado.
- **Plazo de drenaje (`DRAIN_DEADLINE`, `crates/clawg-core/src/runtime.rs:51`):** el tiempo máximo que el apagado dedica a ingerir lo encolado.
- **Volcado:** el archivo `<data>/ingest-pending.jsonl` (`SPILL_FILE`, `crates/clawg-core/src/memory/spill.rs:14`), un `MemoryEvent` en JSON por línea.

## Apagado sin pérdida

### Ningún evento aceptado se pierde en un apagado ordenado

En un apagado ordenado, cada evento aceptado MUST terminar en uno de dos lugares: insertado en la base (o absorbido por el gate de dedup), o escrito en el volcado. El apagado MUST NOT descartar un evento.

La decisión de seguir drenando o volcar es pura (`drain_step`, `crates/clawg-core/src/memory/worker.rs:160`). `MemoryRuntime::shutdown` (`crates/clawg-core/src/runtime.rs:191`) vuelca lo que el worker devuelve, y el daemon la llama antes de borrar su estado (`crates/clawg-cli/src/daemon/serve.rs:151-155`).

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

**Tests:** `shutdown_ingests_every_queued_event_when_the_embedder_is_ready`, `shutdown_spills_events_held_for_an_embedder_that_never_loaded` y `shutdown_spills_what_the_drain_deadline_left_unstarted` (`crates/clawg-core/tests/shutdown.rs`), más los unit de `drain_step` en `worker.rs`.

### Reinyección al arrancar

Si el volcado existe al arrancar, el worker MUST procesar sus eventos antes que cualquier evento nuevo del canal, en el orden del archivo. MUST borrar el volcado sólo cuando procesó todas sus líneas. Si un apagado ordenado llega a mitad de la reinyección, el volcado nuevo MUST contener las líneas no procesadas del anterior, seguidas de lo encolado.

Una línea que no se puede parsear MUST NOT bloquear las demás: se registra con un warn y se mueve a `<data>/ingest-pending.rejected.jsonl` (`REJECTED_FILE`, `spill.rs:18`), que nada borra. Si no se puede guardar en ese archivo, el volcado MUST NOT borrarse, porque es la única copia de esas líneas.

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

**Tests:** `spilled_events_are_ingested_first_on_startup`, `a_corrupt_spill_line_is_set_aside_and_the_rest_ingested` y `a_shutdown_during_replay_keeps_the_unreplayed_lines_first` (`crates/clawg-core/tests/shutdown.rs`).

### Formato del volcado

Las funciones que serializan y parsean el volcado MUST ser puras, sin I/O (`to_lines` y `parse_lines`, `spill.rs:109` y `spill.rs:122`). El volcado MUST escribirse de forma atómica: archivo temporal en el mismo directorio y `rename` (`write`, `spill.rs:135`). Un evento MUST sobrevivir ida y vuelta sin cambios, incluidos `memory_type` e `importance`.

#### Escenario
- GIVEN un `MemoryEvent` explícito con `memory_type = "decision"`, `importance = 8.0` y texto con saltos de línea
- WHEN se serializa y se parsea
- THEN el resultado es igual al original y ocupa una sola línea

**Tests:** `spill_lines_round_trip_events_with_newlines_and_overrides`, `invalid_spill_lines_are_set_apart_and_the_valid_ones_keep_their_order` y `write_replaces_the_spill_file_with_exactly_the_given_events` (`spill.rs`).

### Flush final del `AccessTracker`

En un apagado ordenado, el tracker MUST escribir el buffer pendiente antes de terminar, y el apagado MUST esperar a que termine (`AccessTracker::shutdown`, `crates/clawg-core/src/memory/access.rs:134`). Lo mismo vale cuando se sueltan todos los `Sender`.

#### Escenario
- GIVEN un hit registrado con `record` hace menos de 30 s
- WHEN se hace un apagado ordenado
- THEN el `access_count` de esa fila subió en 1

**Tests:** `access_tracker_flushes_its_buffer_on_shutdown`, `access_tracker_flushes_its_buffer_when_every_sender_is_dropped` y `shutting_down_twice_is_harmless` (`access.rs`).

### Checkpoint al terminar

Después de drenar y del flush del tracker, el apagado SHOULD hacer `PRAGMA wal_checkpoint(TRUNCATE)` (`checkpoint`, `runtime.rs:311`). Un fallo del checkpoint MUST registrarse y MUST NOT impedir el resto del apagado.

#### Escenario
- GIVEN una base en WAL con escrituras recientes
- WHEN se hace un apagado ordenado
- THEN `memory.db-wal` no existe o mide 0 bytes, y una copia de `memory.db` sola contiene las escrituras

**Test:** `shutdown_checkpoints_the_wal_into_memory_db` (`crates/clawg-core/tests/shutdown.rs`).

### El plazo de drenaje cabe en `STOP_TIMEOUT`

`DRAIN_DEADLINE` más un margen para el volcado, el flush y el checkpoint MUST NOT superar `STOP_TIMEOUT` (`crates/clawg-cli/src/stop.rs:19`). Hoy son 10 s + 5 s = 15 s. El doc de `STOP_TIMEOUT` MUST describir lo que el apagado hace.

**Test:** `the_drain_deadline_leaves_room_inside_the_stop_timeout` (`stop.rs`).

**Límite conocido:** el plazo acota el apagado ordenado, pero no la vida del proceso. Si el apagado llega mientras carga el modelo, el proceso sigue vivo hasta que esa carga vuelve: corre en `spawn_blocking` (`crates/clawg-core/src/memory/bootstrap.rs:318`), y el runtime de `#[tokio::main]` (`crates/clawg-cli/src/main.rs:30`) espera a esas tareas al soltarse. Los datos ya están a salvo para entonces. Está en el backlog de `openspec/README.md`.

## Directorio de datos

### Sin home no hay directorio de datos

Si no se puede resolver el home, la resolución MUST devolver un error tipado que lo diga (`PathsError`, `crates/clawg-core/src/paths.rs:149`), y MUST NOT usar el directorio actual. `clawg serve`, `clawg mcp`, `clawg status` y `clawg stop` MUST terminar con ese mensaje y sin crear ningún archivo.

La resolución es una función pura que recibe el home (`data_dir_in`, `paths.rs:159`), para no depender del entorno.

#### Escenario
- GIVEN un resolvedor de home que devuelve `None`
- WHEN se calcula el directorio de datos
- THEN el resultado es `Err` y el mensaje nombra el home

**Tests:** `data_dir_without_a_home_is_an_error` y `data_dir_in_a_home_is_the_profile_directory_under_it` (`paths.rs`).

## Migraciones

### Migraciones atómicas

Las migraciones pendientes de un arranque MUST aplicarse en una sola transacción junto con sus filas de historial (`configured`, `crates/clawg-core/src/memory/db.rs:367`, con `set_grouped(true)`). Si una falla, la base MUST quedar como estaba antes del arranque.

#### Escenario: fallo a mitad
- GIVEN una base vacía y la lista V1–V8 más una V9 cuyo SQL falla
- WHEN se migra con la configuración de producción
- THEN no existe la tabla `memory` ni ninguna fila en `refinery_schema_history`

#### Escenario: base existente
- GIVEN una base migrada a V8 por el runner sin agrupar, con filas
- WHEN se abre con el runner agrupado
- THEN no se aplica nada y las filas siguen igual

**Tests:** `a_failing_migration_rolls_back_every_migration_of_its_run` y `a_v8_database_opens_unchanged_under_grouped_migrations` (`db.rs`).

### Checksums fijados

Los checksums de las migraciones publicadas (V1–V8) MUST coincidir con una lista fija en el código (`PINNED`, `crates/clawg-core/src/memory/migrations.rs:44`). Cambiar una migración publicada MUST romper un test. Una migración nueva agrega su fila a la lista en el mismo commit.

**Test:** `published_migration_checksums_are_pinned` (`migrations.rs`).

## Archivos del workspace

### Escritura atómica de `workspace.md`

`regenerate_auto_section` (`crates/clawg-core/src/memory/workspace.rs:57`) MUST escribir a un archivo temporal en el mismo directorio y hacer `rename` sobre `workspace.md` (`write_atomically`, `workspace.rs:413`). Si la escritura falla, `workspace.md` MUST quedar con su contenido anterior, sección USER incluida, y MUST NOT quedar ningún temporal.

#### Escenario
- GIVEN un `workspace.md` con una sección USER escrita a mano
- WHEN la escritura del temporal falla
- THEN `workspace.md` es idéntico al anterior y no queda ningún temporal

**Test:** `a_failed_workspace_write_leaves_the_previous_file_intact` (`workspace.rs`).
