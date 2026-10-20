# local-transport — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | local-isolation (50), parte 1; después de shutdown-budget (55) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`5aa7843`).

1. **Otra cuenta se hace pasar por el daemon.** El daemon sólo escucha en `127.0.0.1:<puerto del perfil>` (`crates/clawg-cli/src/daemon/serve.rs:142-144`), y el cliente marca ese puerto con lo que dice `daemon.json` (`from_data_dir`, `crates/clawg-cli/src/client.rs:599-610`). Cuando el daemon muere sin apagarse quedan `daemon.json` y `token`; cualquier cuenta de la máquina puede tomar el puerto, y el siguiente cliente le manda el token y cada save.
2. **`/health` es legible por cualquier cuenta**, sin token: nombra agentes, operaciones y causas de fallo.
3. **Dos usuarios no pueden usar el mismo perfil a la vez:** el puerto es fijo por perfil (`crates/clawg-core/src/paths.rs:139-144`) y el daemon del segundo falla en el `bind` (`ServeError::Bind`, `serve.rs:142-144`) mientras su puente espera `AUTOSTART_TIMEOUT` (60 s, `client.rs:41`).
4. **El autoarranque del puente no se prueba por el binario:** el daemon que lanza toma el puerto fijo del perfil y chocaría con un daemon dev de la máquina (el «Límite conocido» de [`specs/install`](../../../specs/install/spec.md)).

## Objetivo que sirve

Transversal: el daemon de una cuenta sólo lo alcanza esa cuenta (y root), en Linux y macOS, sin dejar sin daemon a los clientes de versiones anteriores.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **Sólo la versión de transición.** En Unix el daemon escucha en `<data>/clawg.sock` y, además, en TCP. Apagar TCP por defecto queda para un cambio posterior, al menos una minor después, cuando los actualizadores y los puentes abiertos ya conozcan el socket.
- **Chequeo del uid del par (A).** El listener del socket descarta toda conexión cuyo uid no es el del proceso, antes de que hyper lea un byte.
- **`daemon.json` publica una marca** (`unix_socket: true`), no la ruta: el cliente deriva la ruta del directorio de datos, como ya ignora el host del archivo.
- **TCP en la transición.** `--port` explícito enciende TCP y un `bind` que falla es fatal, como hoy (regla permanente). Sin `--port`, el daemon intenta también el puerto del perfil; si el `bind` falla lo registra, sigue sólo por el socket y no publica `port`.
- **`update` no pasa `--port` en Unix (i).** El daemon nuevo arranca como en el autoarranque y se vigila por el socket. En Windows, donde TCP es el único transporte, sigue pasando el puerto del daemon viejo.
- **Stand-ins de `testing`.** `CLAWG_TEST_DEFAULT_PORT` (`off` o un número) reemplaza el puerto del perfil de un `serve` sin `--port`, para probar el autoarranque y el puerto ocupado sin tocar 27381; el campo `transport: "tcp"` del sello hace que una copia del binario se comporte como un clawg anterior (sólo TCP, y como actualizador pasa `--port`).
- **`clawg status`** muestra el transporte en lo que ya imprime de `daemon.json` (`unix_socket`, `port`).

## Alcance

1. `admits_peer` pura y un `Listener` que compara el uid del par (`#[cfg(unix)]`).
2. `serve`: socket huérfano borrado con el candado tomado, tope de `sun_path` con un error que nombra la ruta, `bind` del socket, TCP según la regla, los dos servidores con una sola señal y una sola gracia, el socket borrado en `clear_state`.
3. `DaemonState`: `port` opcional y `unix_socket` (`serde(default)`).
4. `endpoint` pura y `from_data_dir` por el socket cuando está publicado, sin caer a TCP.
5. `update`: `Running.port` opcional; el daemon nuevo y el viejo, al deshacer, sin `--port` en Unix.
6. Los stand-ins de `testing` (sello `transport`, `CLAWG_TEST_DEFAULT_PORT`).
7. Escenarios de `binary.rs` y docs (`.claude/CLAUDE.md`, `README.md`; al archivar, `specs/daemon-access`, `specs/install`, `specs/self-update`, `specs/mcp`).

## Fuera de alcance

- Apagar TCP por defecto y sacar `port` de `daemon.json` (cambio posterior).
- Windows (windows-pipe, 59).
- Quitar el token, que sigue en todos los transportes.
- Los tests en proceso y `rag-eval`, que siguen en TCP efímero.
- macOS en CI: su `getpeereid` lo implementa tokio y queda sin ejecutar.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), con el modelo falso, sin red ni perfiles reales:

- `admits_peer` acepta el uid propio y rechaza otro; `endpoint` da el socket cuando está publicado aunque haya `port`, y TCP cuando no;
- en proceso, el router sobre el listener del socket contesta `/health` por `http://localhost` y 403 con otro `Host` o con `Origin`;
- con el binario (Unix): `serve` sin `--port` crea `<data>/clawg.sock`; `status`, `stop`, el puente y los hooks llegan por él; `stop` lo borra; un socket huérfano tras un kill no impide arrancar; una ruta larga falla nombrándola; con el puerto ocupado arranca sólo por el socket y no publica `port`; el puente autoarranca el daemon;
- versiones: un actualizador sellado `transport: "tcp"` instala la versión nueva y termina bien; un puente sellado así llega al daemon nuevo; un actualizador nuevo deja al daemon nuevo sin `--port`.

`rag-eval` no aplica: no cambia el ranking ni lo que ve el modelo.

## Datos y vuelta atrás

No toca `memory.db`, el esquema, los identificadores ni el contrato de embedding. `daemon.json` gana un campo y `port` pasa a opcional: un clawg anterior ignora `unix_socket` y lee `port` mientras esté (siempre que TCP esté encendido). Volver a una versión anterior es instalar su binario: escucha en TCP como antes y un `clawg.sock` que quede es inerte (el próximo daemon nuevo lo borra con el candado tomado).
