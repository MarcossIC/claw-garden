# bridge-reconnect — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 (y transversal) | S | — (se coordina con ingest-outbox, 36) | ver [`state.yaml`](state.yaml) |

## Problema

El puente MCP resuelve el daemon una sola vez y se queda con ese puerto y ese token toda la sesión, pero el daemon genera un token nuevo en cada arranque. Detalle y evidencia en la [exploración](exploration.md#premisas-que-se-verificaron):

1. **El cliente es fijo.** `run_with` arma `ClientHandle::ready(client)` una vez (`crates/clawg-cli/src/mcp/mod.rs:90-98`); `ready` crea un `watch` y descarta el emisor (`crates/clawg-cli/src/mcp/startup.rs:32-35`). Nada vuelve a leer `daemon.json` ni `token`.
2. **El token cambia en cada arranque** (`crates/clawg-cli/src/daemon/serve.rs:117-118`).
3. **El consejo es equivocado.** Un 401 es `ClientError::Unauthorized` (`crates/clawg-cli/src/client.rs:476-478`), cuyo texto manda a borrar el token y reiniciar `clawg serve` (`client.rs:51`); `explain` lo deja pasar tal cual (`crates/clawg-cli/src/mcp/server.rs:440`). Eso no le da al puente el token nuevo.
4. **`memory_status` dice `ok`** porque `/health` no lleva token (`client.rs:501-510`; ruta abierta en `crates/clawg-cli/src/daemon/api.rs:577`).
5. **Un arranque que venció queda fallado para siempre.** Si el autoarranque pasa los 60 s (`client.rs:1021-1023`), el handle guarda `Startup::Failed(StartTimeout)` y cada herramienta repite el error (`startup.rs:52-58`), aunque el texto invita a «try again if it is still starting» (`server.rs:418-422`) y el daemon termine de arrancar después. *(Hallazgo nuevo, no estaba en la exploración.)*

Pasa en cada actualización, en cada `clawg stop` (también los que piden `restore` y `purge`) y cuando otro puente autoarranca tras un crash. Como el daemon es compartido, un reinicio rompe a todos los agentes a la vez. Hoy la única salida es reconectar a mano (`README.md:319-320`).

## Corrección de la exploración

La exploración pedía separar «conexión rechazada» de «timeout» en `map_transport`. No hace falta: la única llamada con timeout es `/health` (`client.rs:505`, `HEALTH_TIMEOUT`), que es un GET sin efectos; el resto usa `reqwest::Client::new()` sin timeout (`client.rs:435`). En una herramienta que escribe, `NotRunning` sólo puede venir de `is_connect()`, es decir, de una conexión que nunca llegó. El design fija esa invariante con un test en lugar de cambiar `map_transport` y, con él, lo que deciden `clawg status`, `stop` y `backup` (`crates/clawg-cli/src/main.rs:117`, `:134`).

## Decisión

**Releer y reintentar una vez, sin autoarrancar (A de la exploración).** Marcos eligió «nunca» para el autoarranque a mitad de sesión (tarjeta del hilo, 2026-10-16).

Decisiones abiertas de la exploración, con el valor por defecto que toma este cambio:

| Decisión | Por defecto | Por qué |
|---|---|---|
| Autoarranque a mitad de sesión | nunca | el puente viejo lanzaría el binario viejo (`client.rs:865`) contra un store que el nuevo quizá migró, y pelearía contra un `clawg stop` hecho a propósito |
| Timeout | no se reintenta | no hay timeouts en llamadas que escriben (ver arriba); con `operation_id` (36) se podrá revisar |
| Aviso de versión | sólo en el log | `memory_status` ya muestra la versión del daemon (`Health.version`); repetirlo en cada herramienta sería ruido para el modelo |
| Textos | ver la [spec](spec.md#r4-textos-de-falla) | en inglés, como el resto de las herramientas |

## Alcance

1. **Decisión pura** `on_failure`: qué falla se resuelve releyendo y cuál se informa.
2. **Releer y reintentar** en el puente: ante un 401, una conexión rechazada o un arranque que no terminó, el puente sondea el daemon desde `daemon.json` y `token` sin lanzar ninguno (`probe_running(base_dir, false)`, `client.rs:965`), guarda el cliente nuevo para las llamadas siguientes y repite la herramienta una vez.
3. **El registro del proyecto** que falló por un 401 o una conexión rechazada se vuelve a mandar en la llamada siguiente (hoy queda marcado como enviado aunque falle, `server.rs:522-529`).
4. **Textos** de `explain` para el token rechazado después de releer y para un daemon que no está.
5. **Docs:** `README.md` («Si algo falla»: el paso de `/mcp` deja de hacer falta tras un `clawg stop` o una actualización) y la línea de `clawg mcp` en `.claude/CLAUDE.md`.

## Fuera de alcance

- Arrancar un daemon desde un puente abierto (B de la exploración).
- El ciclo de vida del token y el formato de `daemon.json` (daemon-hardening, 38).
- Más de un reintento, o reintentos con espera.
- Cambiar `map_transport`, `ClientError` o lo que deciden los comandos de la CLI, que releen los archivos en cada corrida.
- La prueba con el binario real reiniciado bajo un puente abierto (install-smoke, 40).

## Datos guardados

No toca el esquema, los ids, los estados ni el contrato de embedding. Un reintento sólo repite llamadas que el daemon no atendió: un 401 lo corta `require_bearer` antes del handler (`crates/clawg-cli/src/daemon/api.rs:607-625`) y una conexión rechazada no llegó. Rollback: volver al binario anterior; no hay nada que deshacer en la base.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- un puente contra el daemon A sigue funcionando después de que A se detiene y B arranca con otro token y otro puerto;
- un token rechazado se relee una sola vez y el texto final no dice «delete the token file»;
- un daemon detenido se informa sin lanzar ninguno, y el texto dice cómo volver;
- un daemon que termina de arrancar después del timeout del arranque se usa en la llamada siguiente;
- un error que el daemon contestó (400, 500) llega al daemon una sola vez.

`rag-eval` no aplica: no cambia ni el ranking ni el contexto.
