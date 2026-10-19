# shutdown-budget — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — (después de 53, que acota el drenaje) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`29a5739`).

1. **El apagado tiene fases sin tope.** La fase HTTP espera a la petición más lenta (`axum::serve(...).with_graceful_shutdown(...)`, `crates/clawg-cli/src/daemon/serve.rs:182-184`), lo que sigue al plazo de drenaje no tiene constante (`shutdown_by`, `crates/clawg-core/src/runtime.rs:319-335`; el `checkpoint` solo puede esperar 5 s en el busy handler de rusqlite) y el `Runtime` de tokio, al soltarse, espera a toda tarea bloqueante, una carga del modelo incluida (`crates/clawg-cli/src/main.rs:70-85`, `crates/clawg-core/src/memory/bootstrap.rs:592`).
2. **El presupuesto escrito no cuenta todas las fases.** `the_drain_deadline_leaves_room_inside_the_stop_timeout` (`crates/clawg-cli/src/stop.rs:63-70`) suma `DRAIN_DEADLINE` y un `AFTER_DRAIN` del test, 10 s + 5 s = 15 s, contra `STOP_TIMEOUT` = 15 s, sin margen y sin la fase HTTP ni el desmontaje.
3. **`update` informa mal un `stop` que vence.** `stop_in` pide el apagado y, si el proceso no termina a tiempo, devuelve `StopTimeout` con el apagado en marcha (`stop.rs:31-41`). `run` aplana el error a `String` (`crates/clawg-cli/src/update/effects.rs:373`), `apply` no suma el deshacer de `StopDaemon` (`crates/clawg-cli/src/update/plan.rs:110-132`) y el informe dice «Its daemon was not touched.» (`crates/clawg-cli/src/update/report.rs:93-104`) mientras el daemon termina segundos después. Nada lo vuelve a levantar: los puentes no arrancan un daemon a mitad de sesión. Contra S7, S10 y S11 de [`specs/self-update`](../../specs/self-update/spec.md).
4. **SIGTERM mata sin cierre.** `shutdown_signal` sólo escucha Ctrl-C y la bandera de la API (`serve.rs:204-227`); `systemctl stop` o un `kill` sin señal dejan `daemon.json` y `token` viejos y saltan el `checkpoint`.

## Objetivo que sirve

Transversal: `clawg stop` y `clawg update` terminan dentro de un plazo conocido, y el usuario no queda sin daemon sin saberlo. Nada aceptado se pierde en ningún caso: el outbox ya lo garantiza.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **`STOP_TIMEOUT` crece a 30 s.** Fases: `HTTP_GRACE` 3 s + `DRAIN_DEADLINE` 10 s (sin cambio) + `AFTER_DRAIN` 7 s + `RUNTIME_TEARDOWN` 2 s = 22 s, con 5 s de holgura exigidos por una prueba. Un apagado normal sigue tardando menos de un segundo; los 30 s sólo se ven cuando algo se cuelga.
- **`HTTP_GRACE` = 3 s después de la señal.** Una petición cortada se ve como conexión cerrada: un save ya tiene su fila en el outbox, y un backup cortado no deja un snapshot con nombre de terminado (`snapshot_into`, `crates/clawg-core/src/memory/backup.rs:313-321`).
- **Windows queda fuera.** Que un daemon lanzado sin consola (`DETACHED_PROCESS`, `client.rs:1217-1218`) reciba los eventos de cierre no se puede verificar acá, y un manejador que no se probó no se declara.
- **Desmontaje acotado:** `shutdown_timeout(RUNTIME_TEARDOWN)` sólo para `clawg serve`; una carga del modelo en curso se abandona y el proceso sale.
- **Una señal sale con 0**, como Ctrl-C: es un apagado ordenado.
- **`update` ante un `stop` que vence (F):** el paso conserva que el apagado se pidió, su deshacer espera el fin de ese pid (otro `STOP_TIMEOUT`) y después arranca el daemon viejo. Si el pid sigue vivo, no arranca nada y el informe dice que se pidió el apagado y cómo arrancarlo cuando termine.

## Alcance

1. `serve.rs`: SIGTERM en `shutdown_signal` (Unix); `HTTP_GRACE` sobre la fase HTTP.
2. `runtime.rs`: `AFTER_DRAIN` en el código, como tope de todo lo que `shutdown_by` hace después del plazo de drenaje.
3. `main.rs`: `shutdown_timeout(RUNTIME_TEARDOWN)` para `serve`.
4. `stop.rs`: `STOP_TIMEOUT` = 30 s, su doc y la prueba del presupuesto con holgura.
5. `update`: `StepError` con `under_way`, la espera del pid en `StartOldDaemon` y el texto del informe.
6. Un embedder de prueba `blocked`, cuya carga no vuelve, para probar el desmontaje con el binario.
7. Docs: `.claude/CLAUDE.md`, y al archivar `openspec/specs/storage`, `openspec/specs/self-update` y el backlog de `openspec/README.md`.

## Fuera de alcance

- Los eventos de consola y de apagado de Windows.
- Cancelar de verdad una carga del modelo en curso.
- Un `TimeoutLayer` en operación normal.
- SIGKILL y el estado viejo que deja.
- Borrar el `.db.tmp` que deja un proceso que muere en medio de un `VACUUM INTO`.
- Cambiar el protocolo de `stop`, el orden del plan de `update` o sus otros deshacer.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- la suma de las cuatro fases más 5 s cabe en `STOP_TIMEOUT` (hoy no existen las constantes);
- un servidor con una petición que no termina vuelve `HTTP_GRACE` después de la señal (hoy espera para siempre);
- un apagado cuyo worker no termina vuelve `AFTER_DRAIN` después del plazo de drenaje (hoy espera para siempre);
- con el binario, en Unix, SIGTERM hace salir al daemon con 0 y sin `daemon.json` ni `token` (hoy los deja);
- con el binario y una carga del modelo que no vuelve, `clawg stop` sale con 0 (hoy vence y sale con 1);
- en el plan, un `StopDaemon` que vence corre `undo StartOldDaemon`; uno que no llegó al daemon no; y el informe no dice «not touched» en el primer caso.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding. Lo que un apagado acotado deja sin hacer ya tiene dónde quedar: un save sin terminar sigue en su fila del outbox y el WAL se repite al abrir. Una versión anterior abre la base sin nada que hacer.
