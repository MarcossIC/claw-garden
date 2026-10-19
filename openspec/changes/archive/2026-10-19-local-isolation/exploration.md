# local-isolation — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — (es requisito de local-transport (58); se beneficia de doctor-checks (31) para verificar modos, pero no lo necesita) | cerrada; sigue en la [propuesta](proposal.md) |

**Tesis:** `daemon-hardening` cerró el paso a las páginas web y dejó escrito que otro usuario local "sigue frenado sólo por el token". Ese cambio dejó fuera, a propósito, el modo del directorio de datos. Acá el adversario es **otro usuario de la misma máquina** (Unix sobre todo): el directorio de datos y casi todo lo que contiene (`memory.db`, copias, logs, workspaces) nace con el `umask`, y sólo `token` y `daemon.json` son 0600. Un único ayudante que crea el directorio en 0700, y aprieta el que ya existe, cierra la lectura a otros usuarios aunque los archivos de adentro sean 0644. Es también la garantía en la que se apoya el socket de local-transport (58): el permiso sobre el socket no es portable, el del directorio sí. Este cambio sólo protege el directorio; la suplantación de un daemon muerto y el `/health` abierto los cierran 58 en Unix y windows-pipe (59) en Windows, y el SIGTERM es de shutdown-budget (55) (ver «Lo que salió de este cambio»).

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El directorio de datos se crea con el `umask` por defecto | **Cierta.** `create_dir_all(&base_dir)` sin modo (`crates/clawg-cli/src/daemon/serve.rs:79`); `data_dir_in` sólo arma `<home>/<perfil>` (`crates/clawg-core/src/paths.rs:159-161`). |
| Hay **ocho** sitios que pueden crearlo primero | **Cierta, y son más.** Además de los de la tesis: `client.rs:1135` (`logs/`, en `spawn_serve_at`), `client.rs:1260` (`start_daemon`, **antes** del daemon), `serve.rs:79`, `db.rs:187` (`MemoryStore::open`, error ignorado con `let _`), `backup.rs:310` (`snapshot_into`) y `:362` (`restore`), `sharing.rs:186`, `update/lock.rs:32`, `setup.rs:668` y `:679`, `workspace.rs:89` y `:133` (`workspaces/`), `model_download.rs:425` (`models/…`). Son doce lugares y cuatro caminos de entrada distintos; el ayudante tiene que vivir en `clawg-core` (lo usan los dos crates). |
| Sólo `token` y `daemon.json` son 0600 | **Cierta.** `create_owner_only` (`crates/clawg-cli/src/daemon/lifecycle.rs:94-112`), usado por `write_state` (`:118`) y `write_token` (`:137`). Nada más en el repo fija un modo: `rg "mode\("` sólo da `lifecycle.rs`. |
| `memory.db`, `-wal`, `-shm`, backups y logs quedan legibles por otros | **Cierta por construcción, sin ejecutar con dos usuarios.** `Connection::open` (`db.rs:191`) y `VACUUM INTO` (`backup.rs:317`) crean con el modo por defecto de SQLite. Que `-wal`/`-shm` copien el modo de la base lo sé por la documentación de SQLite, no lo probé acá. |
| Un directorio de datos con modo abierto expone a otros usuarios | **Depende del directorio personal.** Si `~` es 0700 no hay exposición; con `~` en 0755 (por ejemplo en distribuciones que lo crean así) sí. No se puede afirmar para "todas las instalaciones": hay que medirlo. |
| Otro usuario puede tomar `memory.lock` y bloquear el arranque | **Plausible, no ejecutado.** `acquire` lo abre con `create/read/write` (`store_lock.rs:122-128`) y pide `flock` exclusivo (`:134`); `flock(2)` acepta un descriptor de sólo lectura, y con 0644 otro usuario puede abrirlo así. **No lo probé con dos usuarios reales.** Ojo: `current_owner` abre con `write(true)` (`:195`), así que ese camino falla para otro usuario y devuelve `None`; el riesgo es de `acquire` por otro lado, no de la sonda. |

## Problema (verificado contra el código)

`token` y `daemon.json` se protegen bien: se borran y se crean con `create_new` y modo 0600 (`lifecycle.rs:94-107`). Pero el resto del directorio no: `memory.db` (la memoria entera del usuario, con textos y vectores), los backups (`snapshot_into`, `backup.rs:310-318`), las copias de `settings.json` y `CLAUDE.md` que hace `setup` en `backups/setup` (`setup.rs:668-673`), `workspaces/*/workspace.md`, `logs/serve.log` (`client.rs:1134-1142`), `memory.lock`, `memory.owner` y `sharing.json` nacen con el `umask`. Lo que importa no es el modo de cada archivo sino el del **directorio**: con 0700 ningún otro usuario atraviesa, y los modos de adentro pasan a ser defensa en profundidad (una copia fuera del directorio los pierde).

Quién lo crea primero importa. El puente autoarranca el daemon (`client.rs:1260`), así que el primer `create_dir_all` de una instalación nueva suele ser el del **cliente**, no el del daemon; un fix sólo en `serve.rs:79` no alcanza. Y un directorio que ya existe con modo abierto (todas las instalaciones previas con `umask` 022) no se toca con `create_dir_all`: hace falta apretarlo.

Derivado, sin probar: con `memory.lock` legible por otros, otro usuario podría tomar el candado y el daemon respondería `StoreInUse` (`store_lock.rs:134-138`) leyendo un `memory.owner` ajeno.

Windows: los archivos heredan la ACL del perfil; es la decisión escrita en `lifecycle.rs:87-93`. No la verifiqué.

## Lo que salió de este cambio

La exploración original tenía tres partes; dos se fueron a otros cambios:

| Parte | Dónde queda | Por qué |
|---|---|---|
| B1, manejar SIGTERM y los eventos de cierre de Windows | shutdown-budget (55) | 55 ya lo tenía en su alcance, con su test en `binary.rs`; tenerlo en los dos era trabajo duplicado. Nunca fue una defensa: un SIGKILL o un corte de luz dejan el mismo estado viejo |
| B2, comparar el pid de `/health` con el dueño vivo del candado | No se hace en Unix; en Windows vuelve con otra forma en 59 | Sobre el socket de 58, en un directorio privado, otro usuario no puede crear el punto de conexión: no hay impostor que detectar. En Windows, 59 compara el pid del servidor de la pipe con el dueño del candado |
| B3, desafío HMAC sobre el token | No se hace | Sobra con 58 y 59 |
| C1, dividir `/health` en público y detallado | No se hace | Con 58 y 59 otro usuario no llega a `/health`; TCP queda sólo por opt-in |

Si 58 se abandonara, B2 y C1 vuelven a ser las defensas más baratas contra la suplantación y el `/health` abierto, y hay que recuperarlas de la historia de este archivo.

## Lo que ya existe y se reutiliza

- `create_owner_only` y sus tests Unix (`lifecycle.rs:94-112`, `:268-330`): el patrón de crear con modo desde el nacimiento; se generaliza a directorios.
- Precedente de ajuste global al arrancar `serve` antes del runtime: `keep_one_malloc_arena` (`main.rs:57`).
- `libc` en Unix (`crates/clawg-cli/Cargo.toml:70`).
- Arneses: `tests/binary.rs` (home temporal con `CLAWG_TEST_HOME`, modelo falso, `--port 0`) y los tests de modo de `lifecycle.rs`.

## Enfoques posibles

- **A1. Un ayudante `ensure_private_dir(path)` en `clawg-core` (recomendado).** Crea los directorios que falten con modo 0700 (en Unix, `DirBuilder` con `mode`, recursivo) y, si el directorio **ya existe** con bits de grupo u otros, lo aprieta a 0700 y lo registra. Lo usan los doce sitios. En Windows no hace nada (como `create_owner_only`). A favor: cierra a otros usuarios aunque los archivos de adentro sean 0644; un solo punto; sin `unsafe`. En contra: apretar un directorio existente cambia algo que el usuario pudo haber dejado a propósito (ver "Medir").
- **A2. `umask(0o077)` al arrancar `clawg serve`.** Archivos nuevos 0600 sin tocar cada creador (incluye `-wal` y `-shm`). En contra: `unsafe`, global al proceso, no cubre lo que crea el **cliente** (`logs/`, `start_daemon`) ni el `clawg backup`/`setup` que corren en otro proceso, y no aprieta lo existente. Sirve como defensa en profundidad sobre A1, no en su lugar (recomendado sólo como segunda capa, decisión abierta).
- **A3. Chequeo en `doctor`.** Informar los modos y no cambiar nada. Barato y sin riesgo, pero no cierra nada; es el complemento de A1 (ver `doctor-checks`, 31).
- **A4. Modo 0600 por archivo en cada creador.** Descartada como única medida: no cubre `-wal`/`-shm` ni lo que crea SQLite.

## Descartado

| Opción | Razón técnica |
|---|---|
| Modo 0600 por archivo como única medida (A4) | No cubre `-wal`/`-shm` ni lo que crea SQLite. |
| Un puerto efímero en el autoarranque | El puerto es un contrato (defecto por perfil, `paths.rs:139-142`) y el atacante, además de adivinarlo, puede recorrer puertos. Con el socket de 58 deja de importar. |
| ACL explícita en Windows con `unsafe` | Sigue descartada sin evidencia de que la ACL del perfil falle; si `doctor` la verifica o la contradice, se reabre. windows-pipe (59) la reconsidera sólo para la pipe y con la misma condición. |

Lo que esta exploración descartaba antes como «Sockets Unix o pipes con nombre: cambio L (ya descartado en `daemon-hardening`, opción D)» se reconsideró: con axum 0.8.9 y reqwest 0.12.28 el router se sirve igual por un socket o una pipe, y es local-transport (58) y windows-pipe (59).

## Interacciones

- **`daemon-hardening` (archivado, `specs/daemon-access`):** este cambio cierra uno de los dos "queda para" que dejó, el modo del directorio de datos (su opción B2). El otro, recortar `/health`, ya no hace falta con 58 y 59.
- **local-transport (58) y windows-pipe (59):** 58 necesita este cambio antes; su socket se apoya en el directorio 0700.
- **shutdown-budget (55):** tiene el SIGTERM que salió de acá.
- **Ya resuelto, fuera de alcance:** el commit `1a66e37` (el cliente del daemon no pasa por ningún proxy HTTP, `DaemonClient::new`, `client.rs:590`) y el `0.26.1` (`47ff9d3`).
- **`doctor-checks` (31):** natural lugar para informar modos del directorio y, en Windows, la ACL (A3). No es requisito.
- **`memory-portability` (43) e `history-import` (44):** el export y la importación escribirán archivos nuevos en o desde el directorio de datos; conviene que usen el ayudante. `restore` reemplaza `memory.db` con un archivo cuyo modo viene del origen (`backup.rs:365-…`); a revisar en el diseño.
- **`self-update`:** `update/lock.rs:32` crea el directorio y tiene que usar el ayudante.
- **`project-root-guard` (29):** sigue valiendo que un proceso del **mismo** usuario lee el token; nada de esto lo cambia.

## Alcance candidato

1. `ensure_private_dir` en `clawg-core` y su uso en los doce sitios; apretar un directorio existente y registrarlo una vez (modo anterior y nuevo).
2. Requisitos en `specs/daemon-access`: nuevo (directorio privado en Unix); se **modifica** R5 (de `token` y `daemon.json` al directorio entero). El párrafo `Límite` lo reescribe 58.

### No-alcance

- Un proceso del **mismo** usuario (lee el token, el directorio y la base).
- La identidad del daemon y `/health` (58 y 59) y el SIGTERM (55).
- Windows más allá de un chequeo de `doctor`.
- Cifrar `memory.db` en reposo, rotar el token, cambiar el formato de `daemon.json` o el puerto.
- Cualquier cambio de esquema o de datos: no hay migración; lo único que cambia son modos de archivo, y el camino de vuelta es `chmod`.

## Decisiones abiertas

Cerradas con el usuario; quedan en la [propuesta](proposal.md).

- [x] **Apretar un directorio existente.** Se aprieta siempre, con un aviso en el registro. ¿Se aprieta a 0700 siempre (propuesta, con un aviso en el registro), sólo se avisa, o se pide confirmación? Un usuario que compartía `~/.clawg` por grupo pierde ese acceso; el daemon único con candado exclusivo hace raro que sirva, pero un trabajo de backup de otro usuario sí podría depender de él. Con 58, sólo avisar deja el socket con la garantía no portable del permiso del archivo.
- [x] **`umask` 077 en `serve` como segunda capa (A2).** No. ¿Sí o no? Suma `unsafe` y no cubre al cliente.
- [x] **Derivado de `memory.lock`.** Se acepta como plausible; lo cierra el directorio 0700 y se prueba a mano en `verify.md`. ¿Se prueba con dos usuarios reales antes de redactar el requisito, o se acepta como plausible y el directorio 0700 lo cierra?

## Cómo se mide

"Medirlo" (lo que `daemon-hardening` pidió) significa tres cosas concretas, ninguna hecha en esta exploración:

- **Qué instalaciones tienen un directorio más abierto:** todas las de Unix creadas con `umask` 022, por construcción (`serve.rs:79`); el chequeo real es un test en Linux que arranca el daemon con ese `umask` y lee el modo del directorio, y, para instalaciones existentes, un informe de `doctor` (A3). No se puede enumerar las de los usuarios.
- **Qué le pasa a quien comparte el directorio:** se anota en la propuesta como cambio de comportamiento y se registra la transición (modo anterior y nuevo, una vez por arranque).
- **Que nada se pierde:** un test que siembra un directorio 0755 con una base ya poblada y comprueba que tras apretarlo la base se abre y devuelve las mismas filas.

Tests RED (cada uno falla hoy), con su sistema:

1. **Unix:** con `umask` 022, tras `serve` y tras `start_daemon` (sin daemon previo), el directorio de datos, `logs/`, `backups/` y `workspaces/` son 0700. Hoy son 0755.
2. **Unix:** un directorio 0755 preexistente queda 0700 y la base sigue abriendo con sus filas.
3. **Unix:** `memory.db` y `memory.lock` no son legibles por grupo ni otros (si se decide A2; si no, basta el directorio).

Los escenarios de `binary.rs` corren con **un** usuario del SO; no pueden probar "otro usuario no puede leer". Lo que se automatiza son los **modos** (que es lo que decide el acceso). La comprobación con dos usuarios queda manual, en el `verify.md`: con dos cuentas en Linux (`sudo -u otra`), `ls`/`cat` sobre `~/.clawg-dev/memory.db` debe dar "permiso denegado", y `flock` sobre `memory.lock` desde la otra cuenta no debe poder abrirlo. Un test de suite que use `sudo` no es razonable.

`rag-eval` no hace falta: no toca ranking ni recuperación. No cambia esquema, ids, estados ni contrato de embedding; la prueba de base sembrada es la del punto 2.

## Riesgos

- **Apretar un directorio puede romper a quien lo compartía.** Mitigado con aviso y marcado como decisión abierta; el camino de vuelta es `chmod`.
- **58 se apoya en este cambio:** sin el directorio 0700, la protección del socket depende de un permiso que `unix(7)` llama no portable.
- **El derivado del candado y el `-wal`/`-shm` están sin probar con dos usuarios;** el diseño no debe apoyarse en ellos hasta medirlos.
- **Falsa sensación de seguridad:** nada de esto frena a un proceso del mismo usuario ni a root.
