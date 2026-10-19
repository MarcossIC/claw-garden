# local-isolation — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — (es requisito de local-transport (58)) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop` (`8b7b445`).

En Unix el directorio de datos nace con el `umask`: lo crea `create_dir_all` sin modo en trece lugares, y ninguno aprieta uno que ya existe. Con el `umask` habitual (022) queda 0755, y todo lo que guarda (`memory.db` con su `-wal`, los backups, las copias de `setup`, `workspaces/*/workspace.md`, `logs/serve.log`, `memory.lock`, `memory.owner`, `sharing.json`) lo lee cualquier otro usuario de la máquina que atraviese el directorio personal. Sólo `token` y `daemon.json` nacen 0600 (`create_owner_only`, `crates/clawg-cli/src/daemon/lifecycle.rs:95`).

Los lugares que crean el directorio o uno de sus subdirectorios:

| Lugar | Qué crea | Proceso |
|---|---|---|
| `crates/clawg-cli/src/daemon/serve.rs:79` | el directorio de datos | `clawg serve` |
| `crates/clawg-cli/src/client.rs:1260` (`start_daemon`) | el directorio de datos, antes del daemon | puente que autoarranca |
| `crates/clawg-cli/src/client.rs:1135` (`spawn_serve_at`) | `logs/` | puente, `update` |
| `crates/clawg-core/src/memory/db.rs:187` (`MemoryStore::open`, error ignorado) | el directorio de datos | todo lo que abre el store |
| `crates/clawg-core/src/memory/backup.rs:310` (`snapshot_into`) | `backups/` | daemon, `backup`, migraciones |
| `crates/clawg-core/src/memory/backup.rs:365` (`restore`) | el directorio de datos | `clawg restore` |
| `crates/clawg-core/src/memory/sharing.rs:186` (`save`) | el directorio de datos | `clawg sharing` |
| `crates/clawg-cli/src/update/lock.rs:32` | el directorio de datos | `clawg update` |
| `crates/clawg-cli/src/setup.rs:777` (`write_file`) | `backups/setup/` | `clawg setup` |
| `crates/clawg-core/src/memory/workspace.rs:89`, `:133` | `workspaces/<agente>/<proyecto>/` | daemon |
| `crates/clawg-core/src/memory/model_download.rs:451`, `:669` | `models/multilingual-e5-small/…` | daemon |

`replace` (`setup.rs:810`) también llama a `create_dir_all`, pero sobre el directorio de configuración del cliente (`~/.claude`), que no es de clawg: queda como está.

## Objetivo que sirve

Transversal. Ninguna memoria del usuario es legible por otro usuario de la máquina; y es la garantía en la que se apoya el socket de local-transport (58), porque el permiso sobre un socket no es portable y el del directorio sí.

## Decisiones

Cerradas con el usuario; el porqué está en el [diseño](design.md).

- **Un ayudante, `ensure_private_dir`, en `clawg-core`** (enfoque A1): crea lo que falte con modo 0700 y, si el directorio ya existe con bits de grupo u otros, se los quita y lo registra con el modo anterior y el nuevo. Lo usan los trece lugares.
- **Se aprieta siempre** un directorio existente, con un `warn` en el registro. Quien compartía `~/.clawg` por grupo pierde ese acceso; el camino de vuelta es `chmod`.
- **Sin `umask` 077** en `serve` (enfoque A2): sumaría `unsafe` y no cubre al cliente ni a `backup` o `setup` en otro proceso; el directorio ya cierra el acceso.
- **`memory.lock`:** se acepta como plausible que otro usuario pudiera tomarlo con el directorio abierto; el directorio 0700 lo cierra y no tiene requisito propio. La prueba con dos usuarios queda en `verify.md`.

## Alcance

1. `ensure_private_dir` en `crates/clawg-core/src/private_dir.rs`, con la decisión del modo pura.
2. Los trece lugares de la tabla lo usan; `MemoryStore::open` deja de ignorar el error.
3. `clawg setup` aprieta también el directorio de datos antes de copiar, porque puede ser lo primero que corre sobre una instalación anterior.
4. Requisitos en `specs/daemon-access`: uno nuevo (el directorio de datos es privado en Unix) y R5 pasa a remitir a él; el párrafo `Límite` dice que el directorio frena a otro usuario.
5. Docs: README y `.claude/CLAUDE.md`.

## Fuera de alcance

- Un proceso del **mismo** usuario, y root.
- La identidad del daemon, `/health` abierto (58 y 59) y el SIGTERM (55).
- Windows: el ayudante sólo crea, como `create_owner_only` (`lifecycle.rs:110`); lo sostiene la ACL del perfil. Un chequeo de `doctor` es de doctor-checks (31).
- El modo de cada archivo de adentro (`memory.db`, backups, logs): con el directorio 0700 nadie más llega a ellos. Un archivo copiado fuera del directorio lleva su propio modo.
- Cifrar `memory.db`, rotar el token, cambiar `daemon.json` o el puerto.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), en Linux:

- `ensure_private_dir` crea anidados en 0700 con `umask` 022, aprieta uno 0755 a 0700 y deja uno 0700 como está (`private_dir.rs`).
- Cada lugar que crea un directorio lo deja 0700: `sharing::save`, `snapshot_into`, `restore`, `MemoryStore::open`, `UpdateLock::acquire`, las copias de `setup`, `workspace.md` y la descarga del modelo.
- Con el binario real (`tests/binary.rs`): un puente que autoarranca el daemon en un home vacío deja el directorio de datos y `logs/` en 0700; un directorio 0755 con un store poblado queda 0700 tras arrancar el daemon y el store devuelve las mismas filas.

`rag-eval`: no aplica, no hay cambio de ranking.

## Datos y vuelta atrás

No cambia el esquema, los identificadores, los estados ni el contrato de embedding; no hay migración. Lo único que cambia es el modo de directorios: el de datos y los subdirectorios que clawg crea. El contenido no se toca. Volver atrás: `chmod 755 ~/.clawg` (o el modo que tenía, que queda en el registro); un binario anterior no vuelve a abrirlo, pero tampoco lo cierra.
