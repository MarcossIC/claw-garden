# shutdown-budget — Especificación

## Definiciones

- **Apagado ordenado:** el que empieza con Ctrl-C, con SIGTERM (Unix) o con `POST /v1/shutdown`. Un SIGKILL no es un apagado ordenado.
- **Fases del apagado**, en orden: HTTP (las peticiones en vuelo), drenaje (`DRAIN_DEADLINE`), lo que sigue al drenaje (el save en curso, el flush de accesos, el fin del mantenimiento y el `checkpoint`) y el desmontaje del runtime de tokio (las tareas bloqueantes que siguen corriendo).
- **Apagado pedido:** `clawg stop` mandó `POST /v1/shutdown` y el daemon lo aceptó.

## R1. Cada fase del apagado tiene tope

- La fase HTTP MUST terminar a lo sumo `HTTP_GRACE` (3 s) después de la señal. Pasado el plazo, el servidor se abandona con las peticiones en vuelo, se registra un warn y el apagado sigue. En operación normal, sin señal, ninguna petición tiene tope nuevo.
- Lo que sigue al drenaje MUST terminar a lo sumo `AFTER_DRAIN` (7 s) después del plazo de drenaje. Pasado, el apagado deja lo que falta (se registra un warn): el outbox conserva cada save sin terminar y el WAL se repite al abrir.
- El desmontaje del runtime de `clawg serve` MUST esperar a lo sumo `RUNTIME_TEARDOWN` (2 s) a las tareas bloqueantes; una carga del modelo en curso se abandona y el proceso sale.

#### Escenario: una petición que no termina
- GIVEN un servidor con una petición en vuelo que no termina
- WHEN llega la señal de apagado
- THEN el servidor se abandona `HTTP_GRACE` después y el apagado sigue

#### Escenario: un worker que no termina
- GIVEN un worker de ingest que no termina
- WHEN el runtime se apaga con un plazo de drenaje
- THEN el apagado vuelve a lo sumo `AFTER_DRAIN` después de ese plazo

#### Escenario: una carga del modelo que no vuelve
- GIVEN un daemon cuya carga del modelo bloquea su hilo y no vuelve
- WHEN corre `clawg stop`
- THEN sale con 0 y el proceso del daemon terminó

**Tests:** `a_server_that_does_not_finish_is_left_after_the_grace`, `a_server_that_finishes_in_time_gives_its_result` y `without_a_shutdown_the_server_has_no_deadline` (`serve.rs`, tiempo pausado); `shutdown_gives_up_on_a_worker_that_never_ends` (`crates/clawg-core/tests/shutdown.rs`, tiempo pausado); `stop_ends_a_daemon_whose_model_load_never_returns` (`binary.rs`, con `CLAWG_TEST_EMBEDDER=blocked`).

## R2. Las fases caben en `STOP_TIMEOUT` con holgura

`HTTP_GRACE + DRAIN_DEADLINE + AFTER_DRAIN + RUNTIME_TEARDOWN` más 5 s de holgura MUST NOT superar `STOP_TIMEOUT`, que pasa a 30 s. Las cuatro constantes MUST estar en el código que aplica cada tope, no en el test. El doc de `STOP_TIMEOUT` MUST nombrar las fases.

**Test:** `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` (`stop.rs`), que reemplaza a `the_drain_deadline_leaves_room_inside_the_stop_timeout`.

## R3. SIGTERM es un apagado ordenado (Unix)

En Unix el daemon MUST tratar SIGTERM como Ctrl-C: el mismo camino que `POST /v1/shutdown`, con `daemon.json` y `token` borrados al salir y código de salida 0. Si no puede instalar el manejador, MUST registrarlo y seguir sirviendo. En Windows nada cambia.

#### Escenario
- GIVEN un daemon corriendo
- WHEN recibe SIGTERM
- THEN sale con 0 y no quedan `daemon.json` ni `token`

**Test:** `sigterm_stops_the_daemon_in_order` (`binary.rs`, `#[cfg(unix)]`).

## R4. `update` ante un `stop` que vence

Cuando `StopDaemon` falla con el apagado pedido (`ClientError::StopTimeout`), `update` MUST correr su deshacer `StartOldDaemon`. Ese deshacer MUST esperar primero que termine el pid que contestó `/health`, a lo sumo `STOP_TIMEOUT`, y recién entonces arrancar el daemon con el binario instalado. Si el pid sigue vivo, MUST NOT arrancar nada, y el informe MUST decir que se pidió el apagado y cómo arrancar el daemon cuando termine. Cuando `StopDaemon` falla sin haber pedido el apagado (sin `/health`, o con `shutdown` rechazado), MUST NOT correr `StartOldDaemon`, y el informe sigue diciendo que el daemon no se tocó. En ningún caso se renombra el binario.

#### Escenario: el apagado vence
- GIVEN un daemon que acepta el apagado y no termina en `STOP_TIMEOUT`
- WHEN corre `update`
- THEN se borra el candidato, se corre `undo StartOldDaemon` y el informe no dice «not touched»

#### Escenario: el apagado no llegó
- GIVEN un `stop` que falla antes de pedir el apagado
- WHEN corre `update`
- THEN sólo se borra el candidato y el informe dice que el daemon no se tocó

#### Escenario: el daemon sigue cerrándose
- GIVEN un apagado vencido cuyo deshacer encontró el pid vivo al terminar su espera
- WHEN se arma el informe
- THEN dice que se pidió el apagado y cómo arrancar el daemon cuando termine

**Tests:** `a_daemon_that_does_not_stop_in_time_is_started_again_and_nothing_is_renamed` y `a_stop_that_never_reached_the_daemon_starts_nothing` (`plan.rs`, reemplazan a `a_daemon_that_does_not_stop_renames_nothing`); `a_daemon_still_shutting_down_says_how_to_start_it_once_it_ends` (`report.rs`). Que el informe ya no diga «not touched» lo da `undo StartOldDaemon` en `undone`, que `daemon_state` ya leía como «running again».
