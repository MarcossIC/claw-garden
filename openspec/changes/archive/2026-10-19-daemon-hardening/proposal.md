# daemon-hardening — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); lo que sigue está verificado otra vez contra `develop` (`51d04c0`).

1. **El token es la única barrera de la API en loopback.** `router` deja `/health` fuera de `require_bearer` y pone `/v1` detrás (`crates/clawg-cli/src/daemon/api.rs:725-752`, `:779-796`). Nada mira `Host` ni `Origin`. Una página abierta en el navegador del usuario puede mandar peticiones a `127.0.0.1:<puerto>`, y con DNS rebinding leer las respuestas de lo que no pide token (hoy `/health`; mañana, cualquier ruta que alguien agregue fuera de `/v1`).
2. **El token queda en disco con los permisos del `umask` hasta el `chmod`.** `write_token` hace `File::create`, escribe el secreto y recién después `restrict_to_owner` lo deja en 0600 (`crates/clawg-cli/src/daemon/lifecycle.rs:122-127`, `:88-92`). `write_state` igual (`lifecycle.rs:100-108`). El test mira el modo después de escribir y no ve la ventana (`lifecycle.rs:254-263`).

## Objetivo que sirve

Transversal: protege lo guardado en todos los objetivos frente a una página web, sin cambiar la API ni a sus clientes.

## Decisiones

Las decisiones abiertas de la exploración se cerraron con su recomendación; las que quedaban sin una, con la opción más chica que cierra el ataque.

- **Un guardia en `router`, para toda ruta incluida `/health`**, antes del token: una petición cuyo `Host` no sea `127.0.0.1`, `localhost` o `[::1]` (sin distinguir mayúsculas, con o sin puerto) o que traiga cualquier `Origin`, también `null`, recibe `403 {"error":"forbidden"}`. Va dentro de `router` y no en `serve` para que lo ejerciten el daemon, `serve_ephemeral` y los tests de la API sin cambiar ninguno.
- **Sólo el nombre, no el puerto.** El rebinding se corta por el nombre; comprobar el puerto pediría un campo nuevo en `AppCtx`, que `serve_ephemeral` y los tests arman antes de abrir el socket.
- **`[::1]` se admite** aunque el socket sea IPv4: un nombre de loopback no abre nada, y un cliente que lo use no tiene por qué recibir un 403. **`localhost.` con punto final no**: ningún cliente lo manda.
- **Sin `Host` se lee la autoridad de la URI** (lo que trae una petición HTTP/2: `hyper-util` llega con `http2` a través de `reqwest`), y sin ninguna de las dos es un 403. HTTP/1.1 exige `Host`.
- **403 y no 421**, con un solo cuerpo para los dos motivos, como el 401 (`api.rs:791-794`): distinguirlos le diría a una página qué cambiar.
- **Un registro `warn` como mucho por minuto**, con cuántas peticiones se rechazaron desde el anterior y sin los valores de las cabeceras: un sitio que insiste no inunda el log, y lo que manda no llega al log.
- **El token y `daemon.json` se crean ya con 0600 en Unix**: se borra el archivo anterior y se crea uno nuevo con `create_new` y modo 0600, así un archivo dejado por una versión anterior con otro modo tampoco ve el secreto nuevo, ni quien lo tuviera abierto. `restrict_to_owner` deja de hacer falta y se va.
- **El directorio de datos no se toca (B2 fuera).** Llevarlo a 0700 cubriría `memory.db` y los backups, pero excede el token y cambia un directorio que el usuario ya tiene; queda para un cambio que lo mida.
- **Windows sigue igual**: sin ACL explícita, sostenido por la del directorio personal, por la razón ya escrita en el código. Un chequeo de `doctor-checks` puede reabrirlo con evidencia.

## Alcance

1. `clawg-cli`: la función pura `admits(host, uri_authority, origin)`, la capa en `router` y el registro con tope; la apertura con modo en `lifecycle.rs`.
2. Docs: `.claude/CLAUDE.md` (una línea en «Daemon HTTP API»), README si nombra la seguridad de la API, `openspec/README.md`, y una spec nueva `specs/daemon-access`.

## Fuera de alcance

- Una barrera contra un proceso del mismo usuario o un agente con shell: puede leer el token. Otro usuario local sigue frenado sólo por el token.
- ACL explícita en Windows, recortar `/health`, pipes con nombre o sockets Unix, rotar el token o tokens por cliente.
- Cambiar el formato de `daemon.json` o la API.
- El modo del directorio de datos.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)): la tabla de `admits`; `Host: evil.example` da 403 en `/health` y en `/v1/stats`, con y sin token; `Origin` da 403 con token en `POST /v1/search` y en `/health`; el 403 es el mismo cuerpo para los dos motivos; `DaemonClient` contra el router real sigue guardando, buscando, leyendo `/health` y apagando; en Unix el token y `daemon.json` nacen en 0600 y uno anterior en 0644 queda en 0600 en un archivo nuevo. `tests/binary.rs` (`status`, `stop`, MCP por stdio contra un daemon `--port 0`) sigue pasando sin cambios.

`rag-eval`: no aplica, no toca ranking. `rag-eval --agent-view` usa `serve_ephemeral` y `DaemonClient` sobre `127.0.0.1`, el mismo camino que cubren los tests de arriba.

## Datos y vuelta atrás

No toca el store, el esquema, los identificadores ni el contrato de embedding. El token y `daemon.json` se regeneran en cada arranque. Volver atrás es instalar la versión anterior: un cliente viejo manda `Host: 127.0.0.1:<puerto>` sin `Origin` y pasa el guardia, y un daemon viejo no lo tiene.
