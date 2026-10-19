# local-isolation — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (se beneficia de doctor-checks (31) para verificar modos, pero no lo necesita) | exploración (borrador) |

**Tesis:** `daemon-hardening` cerró el paso a las páginas web y dejó escrito que otro usuario local "sigue frenado sólo por el token". Ese cambio dejó fuera, a propósito, el modo del directorio de datos y recortar `/health`. Acá el adversario es **otro usuario de la misma máquina** (Unix sobre todo) y un archivo de estado que quedó viejo. Hay tres debilidades que no se resuelven por separado: (1) el directorio de datos y casi todo lo que contiene (`memory.db`, copias, logs, workspaces) nace con el `umask`, y sólo `token` y `daemon.json` son 0600; (2) el cliente confía en cualquier proceso que conteste `/health` en el puerto de un `daemon.json` que sobrevivió a su daemon, y a ese proceso le manda el token y cada guardado; (3) `/health` es abierto y nombra agentes, operaciones y causas de fallo. La (2) tiene una defensa barata (comparar el pid que dice `/health` con el dueño real del candado de la base) que **sólo es sólida cuando la (1) está cerrada**, porque hoy `memory.owner` es legible y un impostor puede repetir el pid. Por eso es un cambio y no tres. Es de tamaño M: un único ayudante de creación de directorios usado por todos los creadores, una verificación de identidad en seis sitios del cliente y la división de `/health`.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El directorio de datos se crea con el `umask` por defecto | **Cierta.** `create_dir_all(&base_dir)` sin modo (`crates/clawg-cli/src/daemon/serve.rs:79`); `data_dir_in` sólo arma `<home>/<perfil>` (`crates/clawg-core/src/paths.rs:159-161`). |
| Hay **ocho** sitios que pueden crearlo primero | **Cierta, y son más.** Además de los de la tesis: `client.rs:1135` (`logs/`, en `spawn_serve_at`), `client.rs:1260` (`start_daemon`, **antes** del daemon), `serve.rs:79`, `db.rs:187` (`MemoryStore::open`, error ignorado con `let _`), `backup.rs:310` (`snapshot_into`) y `:362` (`restore`), `sharing.rs:179`, `update/lock.rs:32`, `setup.rs:668` y `:679`, `workspace.rs:89` y `:133` (`workspaces/`), `model_download.rs:425` (`models/…`). Son doce lugares y cuatro caminos de entrada distintos; el ayudante tiene que vivir en `clawg-core` (lo usan los dos crates). |
| Sólo `token` y `daemon.json` son 0600 | **Cierta.** `create_owner_only` (`crates/clawg-cli/src/daemon/lifecycle.rs:94-112`), usado por `write_state` (`:118`) y `write_token` (`:137`). Nada más en el repo fija un modo: `rg "mode\("` sólo da `lifecycle.rs`. |
| `memory.db`, `-wal`, `-shm`, backups y logs quedan legibles por otros | **Cierta por construcción, sin ejecutar con dos usuarios.** `Connection::open` (`db.rs:191`) y `VACUUM INTO` (`backup.rs:317`) crean con el modo por defecto de SQLite. Que `-wal`/`-shm` copien el modo de la base lo sé por la documentación de SQLite, no lo probé acá. |
| Un directorio de datos con modo abierto expone a otros usuarios | **Depende del directorio personal.** Si `~` es 0700 no hay exposición; con `~` en 0755 (por ejemplo en distribuciones que lo crean así) sí. No se puede afirmar para "todas las instalaciones": hay que medirlo. |
| Otro usuario puede tomar `memory.lock` y bloquear el arranque | **Plausible, no ejecutado.** `acquire` lo abre con `create/read/write` (`store_lock.rs:122-128`) y pide `flock` exclusivo (`:134`); `flock(2)` acepta un descriptor de sólo lectura, y con 0644 otro usuario puede abrirlo así. **No lo probé con dos usuarios reales.** Ojo: `current_owner` abre con `write(true)` (`:195`), así que ese camino falla para otro usuario y devuelve `None`; el riesgo es de `acquire` por otro lado, no de la sonda. |
| `clear_state` corre sólo tras `axum::serve` | **Cierta.** `serve.rs:182-192`. `shutdown_signal` espera Ctrl-C y la bandera de `/v1/shutdown` (`serve.rs:203-226`; `ctrl_c` en `:205`). No hay `SignalKind::terminate` ni eventos de cierre de Windows en el árbol (`rg "SignalKind\|ctrl_close\|ctrl_shutdown"` no da nada). |
| Tras un SIGTERM, `kill` o corte de luz quedan `daemon.json`, `token` y `memory.owner` | **Cierta para SIGTERM y SIGKILL** (la acción por defecto mata el proceso sin pasar por `:192`). `memory.owner` se borra en `Drop` de `StoreLock` (`store_lock.rs:213-216`) que tampoco corre; el candado sí lo suelta el SO (`store_lock.rs:34-37`). |
| El puerto es el fijo del perfil, o sea predecible | **Cierta con el autoarranque.** `serve_args(profile, None)` sin `--port` (`client.rs:1148`, test `:1299`) y el defecto es 27380/27381 (`paths.rs:139-142`). |
| `probe_running` acepta cualquier respuesta de `/health` | **Cierta.** `client.health().await.is_ok()` (`client.rs:1223-1226`); el sondeo de `start_daemon` igual (`:1271-1274`). `health()` no manda token (`:656-665`). La respuesta basta con que parsee como `Health` (`client.rs:117-135`). |
| Todo el que usa el cliente pasa por `probe_running` | **Falsa.** `hook.rs:154`, `stop.rs:24-27`, `main.rs:128-129` y `update/effects.rs:273` usan `from_data_dir` directo y mandan el token (o piden `pid`) sin ninguna sonda previa. Son seis sitios, no dos. |
| `/health` sólo lleva contadores | **Falsa.** `ingest.problems` trae `operation_id`, `agent_id`, `state`, `attempts` y `error` por fila (`outbox.rs:145-163`), y `embedder_error` es `e.to_string()` del cargador (`bootstrap.rs:606-609`) y puede incluir rutas del usuario; no lo verifiqué para cada variante de error. |
| `hmac` ya es dependencia | **Falsa.** Sólo `sha2 = "0.10"` (`Cargo.toml:25`, usada en core y cli); `rg "hmac"` no da nada. |

## Problema (verificado contra el código)

### 1. El directorio de datos y su contenido (Unix)

`token` y `daemon.json` se protegen bien: se borran y se crean con `create_new` y modo 0600 (`lifecycle.rs:94-107`). Pero el resto del directorio no: `memory.db` (la memoria entera del usuario, con textos y vectores), los backups (`snapshot_into`, `backup.rs:310-318`), las copias de `settings.json` y `CLAUDE.md` que hace `setup` en `backups/setup` (`setup.rs:668-673`), `workspaces/*/workspace.md`, `logs/serve.log` (`client.rs:1134-1142`), `memory.lock`, `memory.owner` y `sharing.json` nacen con el `umask`. Lo que importa no es el modo de cada archivo sino el del **directorio**: con 0700 ningún otro usuario atraviesa, y los modos de adentro pasan a ser defensa en profundidad (una copia fuera del directorio los pierde).

Quién lo crea primero importa. El puente autoarranca el daemon (`client.rs:1260`), así que el primer `create_dir_all` de una instalación nueva suele ser el del **cliente**, no el del daemon; un fix sólo en `serve.rs:79` no alcanza. Y un directorio que ya existe con modo abierto (todas las instalaciones previas con `umask` 022) no se toca con `create_dir_all`: hace falta apretarlo.

Derivado, sin probar: con `memory.lock` legible por otros, otro usuario podría tomar el candado y el daemon respondería `StoreInUse` (`store_lock.rs:134-138`) leyendo un `memory.owner` ajeno.

Windows: los archivos heredan la ACL del perfil; es la decisión escrita en `lifecycle.rs:87-93`. No la verifiqué.

### 2. Un `daemon.json` viejo y un proceso en su puerto

Secuencia: el daemon muere sin pasar por `clear_state` (SIGTERM, SIGKILL, caída, corte de luz). Quedan `daemon.json` y `token`. Otro usuario abre un socket en `127.0.0.1:27380` (o el del perfil). El siguiente cliente lee ambos archivos (`from_data_dir`, `client.rs:599-610`), llama a `/health`, el impostor contesta con un JSON válido y `probe_running` devuelve `Running` (`:1223-1226`). Desde ahí:

- el puente le manda el token en cada llamada y cada `save_memory` (todo lo que el agente guarda);
- lo que el impostor contesta entra al contexto del agente como memorias (`recall_context`, el hook de `session-start`): un canal de inyección de instrucciones;
- `probe_running` nunca devuelve `NeedsStart`, así que el daemon real no arranca, y si arrancara, `bind` fallaría (`serve.rs:142-144`).

`daemon.json` es 0600, así que el atacante no lee el puerto del archivo; no hace falta: el defecto es público. Mientras haya un daemon vivo el puerto está ocupado y el ataque no existe; lo que lo habilita es el estado viejo.

Qué lo deja viejo hoy: en Unix `tokio::signal::ctrl_c` sólo escucha SIGINT, así que un SIGTERM (el `kill` común, `systemctl stop`, el cierre de sesión) mata al daemon sin pasar por `clear_state`. En Windows el daemon se lanza `DETACHED_PROCESS` (`client.rs:1158-1163`): **no verifiqué** si un proceso sin consola recibe `ctrl_close` o `ctrl_shutdown`; si no, ese camino no sirve allí. Y SIGKILL, caída y corte de luz no se pueden manejar nunca.

El fallo es del cliente: confía en la **respuesta**, no en la **identidad**.

### 3. `/health` abierto

`health` (`api.rs:898-924`) devuelve `ingest: Option<OutboxSummary>` con `problems` (`outbox.rs:152`): por cada fila fallida, el `operation_id`, el `agent_id`, el estado, los intentos y el texto del error. Más `embedder_error` (`api.rs:487`). El guardia de `Host`/`Origin` (`api.rs:850-870`) frena páginas, no a un proceso local que manda `Host: 127.0.0.1`.

Consumidores de `Health` (todos verificados): `memory_status` (`mcp/server.rs:985` → `mcp/tools.rs:562`), `clawg status` (`main.rs:128`, `status.rs`), `memories.rs:397` (sólo `version`), `update/mod.rs:132` (`pid`, `version`), `hook.rs:167` (`health.ok()` para el texto de arranque), `stop.rs:27` (`pid`), `mcp/startup.rs:108` (`version`, al registrar). Tests que lo fijan: `format_status_shows_the_ingest_block` (`tools.rs:1311`), `a_health_without_ingest_still_parses` (`client.rs:1305`), `health()` y `published_health` de `tests/binary.rs` (`:211`, `:1026`, que leen `ingest.pending` sin token, `:260`), y `health_answers_without_a_token` y hermanos en `tests/api.rs:352` y siguientes.

Qué es sensible: `ingest.problems` (agentes, operaciones, causas) y `embedder_error` (texto libre). Qué no: `status`, `version`, `profile`, `embedder`, `embedder_retryable`, `embedder_resident`, `uptime_s` y los contadores `pending`/`failed`/`oldest_pending_s`. `pid` se queda público: lo necesita `stop` **antes** de apagar (`stop.rs:27`) y lo necesita la comprobación del punto 2.

La compatibilidad ya la planteó `daemon-hardening`: `OutboxSummary.problems` no tiene `serde(default)` (`outbox.rs:145-153`), así que un `clawg` viejo que reciba un `/health` sin ese campo no parsea, `probe_running` lo lee como "no hay daemon" e intenta arrancar otro. Quitar la clave rompe; dejarla **vacía** no.

## Lo que ya existe y se reutiliza

- `create_owner_only` y sus tests Unix (`lifecycle.rs:94-112`, `:268-330`): el patrón de crear con modo desde el nacimiento; se generaliza a directorios.
- `StoreLock::current_owner` (`store_lock.rs:190-203`): confirma el registro contra el candado, o sea distingue un dueño vivo de un resto de un proceso muerto, porque el SO suelta el candado con el proceso (`:34-37`). `recorded_owner` (`:167`) lee sin tocar el candado.
- `Health.pid` ya viaja (`api.rs:916`); `DaemonState.pid` también (`lifecycle.rs:39`).
- `require_bearer` y `constant_time_eq` (`api.rs:873-891`); `sha2` ya está.
- El idioma de compatibilidad: campos `serde(default)` en `Health` (`client.rs:123-139`) y `version_warning` (`memories.rs:332-339`).
- `tokio` ya tiene la característica `signal` en `clawg-cli` (`Cargo.toml:41`), `libc` en Unix (`:70`) y `windows-sys` en Windows (`:62`).
- Precedente de ajuste global al arrancar `serve` antes del runtime: `keep_one_malloc_arena` (`main.rs:57`).
- Arneses: `tests/binary.rs` (home temporal con `CLAWG_TEST_HOME`, modelo falso, `--port 0`) y los tests de modo de `lifecycle.rs`.

## Enfoques posibles

### Parte 1 — el directorio de datos

- **A1. Un ayudante `ensure_private_dir(path)` en `clawg-core` (recomendado).** Crea los directorios que falten con modo 0700 (en Unix, `DirBuilder` con `mode`, recursivo) y, si el directorio **ya existe** con bits de grupo u otros, lo aprieta a 0700 y lo registra. Lo usan los doce sitios. En Windows no hace nada (como `create_owner_only`). A favor: cierra a otros usuarios aunque los archivos de adentro sean 0644; un solo punto; sin `unsafe`. En contra: apretar un directorio existente cambia algo que el usuario pudo haber dejado a propósito (ver "Medir").
- **A2. `umask(0o077)` al arrancar `clawg serve`.** Archivos nuevos 0600 sin tocar cada creador (incluye `-wal` y `-shm`). En contra: `unsafe`, global al proceso, no cubre lo que crea el **cliente** (`logs/`, `start_daemon`) ni el `clawg backup`/`setup` que corren en otro proceso, y no aprieta lo existente. Sirve como defensa en profundidad sobre A1, no en su lugar (recomendado sólo como segunda capa, decisión abierta).
- **A3. Chequeo en `doctor`.** Informar los modos y no cambiar nada. Barato y sin riesgo, pero no cierra nada; es el complemento de A1 (ver `doctor-checks`, 31).
- **A4. Modo 0600 por archivo en cada creador.** Descartada como única medida: no cubre `-wal`/`-shm` ni lo que crea SQLite.

### Parte 2 — la identidad del daemon

- **B1. Manejar SIGTERM (y los eventos de cierre de Windows) para que corra la ruta ordenada.** Reduce cuánto estado viejo queda en el caso común. No lo elimina (SIGKILL, caída, corte, y la duda de Windows) y no protege a un cliente que encuentra estado viejo. Recomendado como mejora, **no como defensa**.
- **B2. Comparar el pid de `/health` con el dueño vivo del candado (recomendado como defensa mínima).** Tras un `/health` exitoso, `StoreLock::current_owner(base_dir)` debe devolver un dueño `Daemon` con el mismo pid; si no, el cliente **no** lo usa ni le manda el token. Detecta el caso viejo (un SIGKILL suelta el candado). A favor: no cambia el protocolo, anda contra daemons viejos (ya mandan `pid`). En contra: se apoya en que `memory.owner` no lo lea el atacante; con 0644 un impostor copia el pid y pasa. **Por eso depende de la parte 1.** En Windows depende de la ACL del perfil. Va **después** de que `/health` contestó: la sonda toma el candado un instante y, contra un daemon que está arrancando, podría hacerlo perder (la ventana que describe `store_lock.rs:177-184`); como `daemon.json` se escribe con el candado ya tomado (`serve.rs:92`, `:152-163`), si `/health` contestó y el daemon era el real, el candado ya está tomado y no hay ventana.
- **B3. Desafío: el daemon prueba que conoce el token.** El cliente manda un nonce aleatorio, el daemon contesta `HMAC-SHA256(token, nonce)` (ruta nueva fuera de `/v1` o dentro, ver abajo) y el cliente lo compara. Es independiente de los modos de archivo y de la plataforma, y no manda el token. En contra: protocolo nuevo, una dependencia (`hmac`) o una HMAC a mano sobre `sha2` (unas 15 líneas, con riesgo de equivocarse), y un daemon viejo no lo tiene: el cliente nuevo tiene que **degradar** (404 → usar B2) y no negarse a trabajar. Recomendado como fortalecimiento **si** el usuario quiere que la defensa no dependa de la ACL de Windows; decisión abierta.
- **B4. Puerto efímero en el autoarranque.** Descartada: el puerto es un contrato (defecto por perfil, `paths.rs:139-142`) y el atacante, además de adivinarlo, puede recorrer puertos.

### Parte 3 — `/health`

- **C1. Dividir: público con conteos, detalle con token (recomendado).** `/health` sigue sin token y devuelve lo no sensible; `problems` sale **vacío** (la clave presente, para que un `clawg` viejo parsee) y `embedder_error` va a `null`. El detalle va en una ruta bajo `/v1` (por ejemplo `/v1/health`) que `memory_status` y `clawg status` piden **después** de haber verificado el daemon (B2/B3). A favor: nadie rompe en ninguna dirección con versiones mezcladas; `binary.rs` sigue leyendo `ingest.pending` sin token. En contra: un `clawg status` nuevo contra un daemon viejo no encuentra la ruta; degrada a lo público y lo dice (patrón de `version_warning`).
- **C2. `/health` completo sólo si llega un token válido.** Descartada: obliga a `health()` a mandar el token a un proceso aún no verificado, que es el problema del punto 2.

## Descartado

| Opción | Razón técnica |
|---|---|
| Una "sonda autenticada": mandar el token a una ruta `/v1` y esperar 200 | **No prueba nada.** Un impostor contesta 200 a todo y **ya capturó el token** al recibir la petición; la sonda es la fuga. El daemon tiene que probar que conoce el secreto sin que el cliente lo mande primero (B3), o la identidad se comprueba por otro canal (B2). |
| Confiar en el pid de `daemon.json` | El archivo sobrevive al daemon y el pid se reutiliza (`client.rs:1221-1222` ya lo dice). |
| Comparar el pid de `/health` con `daemon.json` | Un impostor responde cualquier pid. Sólo vale contra algo que el impostor no controle, como el candado. |
| Pedir token en `/health` | Rompe `probe_running`, `stop` y versiones mezcladas (ya descartado en `daemon-hardening`). |
| Sockets Unix o pipes con nombre | Cambio L (ya descartado en `daemon-hardening`, opción D). |
| ACL explícita en Windows con `unsafe` | Sigue descartada sin evidencia de que la ACL del perfil falle; si `doctor` la verifica o la contradice, se reabre. |

## Interacciones

- **`daemon-hardening` (archivado, `specs/daemon-access`):** este cambio cierra los dos "queda para" que dejó: el modo del directorio de datos (su opción B2) y recortar `/health` (su opción C). Su razón para no recortar `/health` (obligar a `health()` a mandar token) se esquiva con C1.
- **Ya resuelto, fuera de alcance:** el commit `1a66e37` (el cliente del daemon no pasa por ningún proxy HTTP, `DaemonClient::new`, `client.rs:590`) y el `0.26.1` (`47ff9d3`).
- **`doctor-checks` (31):** natural lugar para informar modos del directorio y, en Windows, la ACL (A3). No es requisito.
- **`memory-portability` (43) e `history-import` (44):** el export y la importación escribirán archivos nuevos en o desde el directorio de datos; conviene que usen el ayudante. `restore` reemplaza `memory.db` con un archivo cuyo modo viene del origen (`backup.rs:365-…`); a revisar en el diseño.
- **`self-update`:** `update/lock.rs:32` crea el directorio; `clawg update` arranca el daemon nuevo (`update/effects.rs`) y debe seguir pasando la verificación de identidad.
- **Backlog "Ningún test lleva el hook a un daemon que falla" (`openspec/README.md`):** el listener falso que hace falta acá (responde `/health`, cuenta peticiones con `Authorization`) sirve también para llevar `hook` a un daemon que devuelve un error o no contesta; lo cubriría en parte, no el camino de `shutdown_background` (`main.rs`).
- **`project-root-guard` (29):** sigue valiendo que un proceso del **mismo** usuario lee el token; nada de esto lo cambia.

## Alcance candidato

1. `ensure_private_dir` en `clawg-core` y su uso en los doce sitios; apretar un directorio existente y registrarlo una vez (modo anterior y nuevo).
2. Verificación de identidad (B2) en un único punto del cliente, usado por los seis sitios que hoy mandan el token o piden el pid; sin identidad verificada no se manda el token.
3. SIGTERM en `shutdown_signal` en Unix (B1); Windows sólo si se prueba que el evento llega.
4. División de `/health` (C1) y la ruta con token para el detalle; `Health` con `serde(default)` donde falte.
5. Requisitos en `specs/daemon-access`: nuevo (directorio privado en Unix), nuevo (identidad antes de confianza: el cliente MUST NOT mandar el token a un daemon cuyo pid no coincida con el dueño del candado), nuevo (SIGTERM ordenado), nuevo (`/health` sin detalle); se **modifican** el párrafo `Límite` (otro usuario ya no queda frenado "sólo por el token") y R5 (de `token` y `daemon.json` al directorio entero); R3 (los clientes de hoy pasan) gana el caso de un daemon viejo.

### No-alcance

- Un proceso del **mismo** usuario (lee el token, el directorio y la base).
- Windows más allá de un chequeo de `doctor`; B3 (desafío) salvo que se decida.
- Cifrar `memory.db` en reposo, rotar el token, cambiar el formato de `daemon.json` o el puerto.
- Cualquier cambio de esquema o de datos: no hay migración; lo único que cambia son modos de archivo, y el camino de vuelta es `chmod`.

## Decisiones abiertas

Son del usuario.

- [ ] **Apretar un directorio existente.** ¿Se aprieta a 0700 siempre (propuesta, con un aviso en el registro), sólo se avisa, o se pide confirmación? Un usuario que compartía `~/.clawg` por grupo pierde ese acceso; el daemon único con candado exclusivo hace raro que sirva, pero un trabajo de backup de otro usuario sí podría depender de él.
- [ ] **`umask` 077 en `serve` como segunda capa (A2).** ¿Sí o no? Suma `unsafe` y no cubre al cliente.
- [ ] **Identidad: B2 solo, o B2 + B3.** B3 cubre Windows sin depender de su ACL pero agrega protocolo y probablemente `hmac`.
- [ ] **Daemon viejo sin el desafío.** Si hay B3: ¿el cliente degrada a B2 (propuesta) o se niega?
- [ ] **Qué hace el cliente ante identidad no verificada.** ¿Error claro con el siguiente paso (`clawg stop`/borrar el estado viejo), o limpiar el estado y arrancar uno propio? Limpiar sin que el dueño del candado lo confirme puede dejar al impostor en el puerto.
- [ ] **Ruta del detalle.** `/v1/health` (propuesta) o ampliar `/v1/stats`; y si `embedder_error` se queda público recortado o va entero al detalle.
- [ ] **Windows y los eventos de cierre.** ¿Se prueba primero que un proceso `DETACHED_PROCESS` los recibe? Si no, no se agrega nada.
- [ ] **Derivado de `memory.lock`.** ¿Se prueba con dos usuarios reales antes de redactar el requisito, o se acepta como plausible y el directorio 0700 lo cierra?

## Cómo se mide

"Medirlo" (lo que `daemon-hardening` pidió) significa tres cosas concretas, ninguna hecha en esta exploración:

- **Qué instalaciones tienen un directorio más abierto:** todas las de Unix creadas con `umask` 022, por construcción (`serve.rs:79`); el chequeo real es un test en Linux que arranca el daemon con ese `umask` y lee el modo del directorio, y, para instalaciones existentes, un informe de `doctor` (A3). No se puede enumerar las de los usuarios.
- **Qué le pasa a quien comparte el directorio:** se anota en la propuesta como cambio de comportamiento y se registra la transición (modo anterior y nuevo, una vez por arranque).
- **Que nada se pierde:** un test que siembra un directorio 0755 con una base ya poblada y comprueba que tras apretarlo la base se abre y devuelve las mismas filas.

Tests RED (cada uno falla hoy), con su sistema:

1. **Unix:** con `umask` 022, tras `serve` y tras `start_daemon` (sin daemon previo), el directorio de datos, `logs/`, `backups/` y `workspaces/` son 0700. Hoy son 0755.
2. **Unix:** un directorio 0755 preexistente queda 0700 y la base sigue abriendo con sus filas.
3. **Unix:** `memory.db` y `memory.lock` no son legibles por grupo ni otros (si se decide A2; si no, basta el directorio).
4. **Ambos:** un listener falso (puerto efímero) con `daemon.json` y `token` apuntándole, que contesta `/health` con un cuerpo válido y cuenta las peticiones con `Authorization`; `probe_running`, `hook`, `stop` y `status` no deben mandarle ninguna. Hoy se la mandan todas. Un segundo caso con el candado **tomado** por el proceso de prueba y su pid en `/health` debe seguir pasando.
5. **Unix, `binary.rs`:** SIGTERM al daemon (por su manejador, no por nombre) deja sin `daemon.json` ni `token` y sale limpio. Hoy quedan.
6. **Ambos:** el cuerpo público de `/health` no contiene `agent_id`, `operation_id` ni texto de `error`, y `problems` es `[]`; el detalle con token sí los trae; `a_health_without_ingest_still_parses` y un `Health` con `problems` vacío siguen parseando.
7. **Ambos:** un `Health` de un daemon anterior (sin las claves nuevas) sigue funcionando en el cliente nuevo.

Los escenarios de `binary.rs` corren con **un** usuario del SO; no pueden probar "otro usuario no puede leer". Lo que se automatiza son los **modos** (que es lo que decide el acceso). La comprobación con dos usuarios queda manual, en el `verify.md`: con dos cuentas en Linux (`sudo -u otra`), `ls`/`cat` sobre `~/.clawg-dev/memory.db` debe dar "permiso denegado", y `flock` sobre `memory.lock` desde la otra cuenta no debe poder abrirlo. Un test de suite que use `sudo` no es razonable.

`rag-eval` no hace falta: no toca ranking ni recuperación. No cambia esquema, ids, estados ni contrato de embedding; la prueba de base sembrada es la del punto 2.

## Riesgos

- **Apretar un directorio puede romper a quien lo compartía.** Mitigado con aviso y marcado como decisión abierta; el camino de vuelta es `chmod`.
- **B2 sin la parte 1 es una falsa defensa:** el impostor lee `memory.owner` y repite el pid. Por eso van juntas, y el requisito debe decir que la identidad se apoya en el directorio privado.
- **La sonda del candado puede hacer perder a un daemon que arranca** (`store_lock.rs:177-184`); se limita llamándola sólo tras un `/health` exitoso. Hay que cubrirlo con un test en el arranque.
- **Un cliente nuevo contra un daemon viejo** debe degradar y avisar, no negarse; y un cliente viejo contra un `/health` recortado no debe fallar al parsear (por eso `problems` vacío y no ausente).
- **Windows queda con la garantía de la ACL del perfil**, sin verificar; B3 y un chequeo de `doctor` son la salida.
- **El derivado del candado y el `-wal`/`-shm` están sin probar con dos usuarios;** el diseño no debe apoyarse en ellos hasta medirlos.
- **Falsa sensación de seguridad:** nada de esto frena a un proceso del mismo usuario ni a root.
