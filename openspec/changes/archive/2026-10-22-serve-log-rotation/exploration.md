# serve-log-rotation — Exploración

Objetivo transversal, tamaño S. `logs/serve.log` crece sin límite y no dice qué versión escribió cada parte. Es la propuesta M9 de una propuesta de medición escrita sobre 0.26.0; se separa de [usage-metrics](../2026-10-22-usage-metrics/exploration.md) (71) porque no depende de nada y se puede entregar antes.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa (sobre 0.26.0) | Estado en `develop` (0.45.0) |
|---|---|
| `serve.log` se abre en modo append y nunca se rota | **Cierta.** `spawn_serve_at` (`crates/clawg-cli/src/client.rs:1271-1300`) abre `<data>/logs/serve.log` con `create` y `append` (`:1284-1288`) y le pasa el mismo fichero como stdout y stderr del hijo. Ningún otro código lo trunca, rota ni borra. |
| Es el único sitio que lo abre | **Cierta.** Lo llaman el autoarranque del puente (`ProcessSpawner::spawn_serve`, `client.rs:1245-1254`) y `clawg update` (`crates/clawg-cli/src/update/effects.rs:184`). Un `clawg serve` a mano escribe en la terminal. |
| La línea de arranque no dice la versión | **Cierta.** `serve.rs:204-209` (`crates/clawg-cli/src/daemon/serve.rs`) registra perfil, transporte y directorio; la versión va en `daemon.json` (`serve.rs:188`), que un apagado ordenado borra. Con `update` cambiando el binario en el mismo `serve.log`, meses de log no dicen qué versión escribió qué. |
| El directorio `logs/` es privado | **Cierta desde local-isolation (50).** `spawn_serve_at` lo crea con `ensure_private_dir` (`client.rs:1282-1283`). |

## Problema

1. Un perfil usado a diario acumula `serve.log` sin tope: cada arranque del puente, cada `update` y cada `warn` se suma. Nadie lo poda.
2. Al leer un `serve.log` para diagnosticar (lo que pide `verify.md` en varios cambios, y lo que hará 71 al cruzar una ejecución sin `stopped_at_ms` con su cola de log) no se sabe qué versión escribió cada tramo.

## Enfoques posibles

### A. Rotar al abrir, en `spawn_serve_at` (recomendado)

Si `serve.log` pasa de un tope al arrancar (candidato: 10 MB), se renombra a `serve.log.1` (y este a `.2`, conservando tres). La decisión de rotar es pura (tamaño y tope) y se prueba sin disco; el renombrado, en un directorio temporal. El daemon que corre no rota nunca: un daemon que vive semanas puede pasar el tope, y eso se acepta porque el próximo arranque lo corta.

Un renombrado mientras otro daemon escribe (dos puentes que autoarrancan a la vez, y uno pierde el lock) no pierde líneas: en Unix el que escribe sigue en el inodo renombrado, y en Windows `std::fs::OpenOptions` abre con `FILE_SHARE_DELETE`, así que el renombrado no falla. Conviene un test que lo fije en los dos sistemas.

### B. Rotar en el daemon, por tamaño, mientras escribe

Pide un writer propio en lugar del fichero heredado como stdout y stderr: los `panic` y lo que ONNX Runtime escriba en stderr por su cuenta no pasarían por él. Descartado.

### C. Truncar al arrancar

Pierde justo lo que el comentario de `client.rs:1279-1280` protege: la cola de la ejecución anterior cuando el daemon murió. Descartado.

## Alcance candidato

1. Rotación al abrir en `spawn_serve_at`, con la decisión pura y su tope.
2. La versión (y el `dist_target` cuando lo hay) en la línea de arranque de `serve.rs`, también cuando el daemon escribe en la terminal.
3. Documentar en `.claude/CLAUDE.md` (`clawg serve`) dónde va el log y cuánto se guarda, y `RUST_LOG=info,clawg_core::memory::search=debug` para obtener la línea `retrieval=` sin el resto del `debug`.

### No-alcance

- Logs estructurados o migrar a `tracing`.
- Un hallazgo de `doctor` por `serve.log` grande: con la rotación deja de hacer falta.

## Decisiones abiertas

- [x] Tope y cantidad de ficheros: 10 MiB y tres ficheros (`serve.log`, `.1` y `.2`); ver la [propuesta](proposal.md).

## Cómo se mide

Un test que falla hoy: con un `serve.log` de más del tope en un home temporal, un autoarranque en `binary.rs` deja `serve.log.1` con el contenido viejo y un `serve.log` nuevo cuya primera línea del daemon nombra la versión.
