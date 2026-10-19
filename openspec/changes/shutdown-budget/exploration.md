# shutdown-budget — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — (53 outbox-lifecycle acota la fase de drenaje; ver «Interacciones») | exploración (borrador) |

**Tesis:** `clawg stop` y `clawg update` terminan dentro de un presupuesto conocido, haga lo que haga el daemon, y el daemon también se apaga en orden cuando el sistema se lo pide (SIGTERM en Unix, los eventos de consola y de apagado en Windows). Hoy el presupuesto de 15 s sólo cubre dos de las cuatro fases del apagado (el drenaje y lo que viene después), y su prueba lo declara con margen cero; la fase HTTP y la espera de una carga del modelo no tienen tope. Una petición larga, o un modelo cargándose, hace que `stop` salga con 1 y que `update` lo trate como fallo y deshaga. Y un SIGTERM mata el proceso sin pasar por el cierre, así que `daemon.json` y `token` quedan viejos. Nada de lo aceptado se pierde en ningún caso (el outbox ya lo garantiza); lo que falla es el plazo y el orden.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `with_graceful_shutdown` no tiene límite | **Cierta.** `axum::serve(...).with_graceful_shutdown(shutdown_signal(shutdown_rx)).await` (`crates/clawg-cli/src/daemon/serve.rs:182-184`); axum 0.8.9 (`Cargo.lock`) espera a todas las conexiones en vuelo y no trae tope. Hasta que vuelve, `ctx.runtime.shutdown(workers)` (`:188`) no empieza. |
| `shutdown_signal` sólo escucha Ctrl-C y la bandera de la API | **Cierta.** `tokio::signal::ctrl_c()` (`serve.rs:205`) y el `watch` (`:209-220`) en un `select!` (`:222-225`). `rg "SignalKind\|ctrl_close\|ctrl_shutdown\|ctrl_logoff"` no da nada en `crates/`. El feature `signal` de tokio ya está habilitado en `clawg-cli` (`crates/clawg-cli/Cargo.toml:41`). |
| Hay peticiones largas posibles | **Cierta.** `SAVE_CONFIRM_TIMEOUT` = 10 s (`crates/clawg-cli/src/daemon/api.rs:98`, usado en `:1118`); `POST /v1/backup` (`api.rs:1464`) espera un `VACUUM INTO` (`crates/clawg-core/src/memory/backup.rs:317`), que desde `b1f22f3` corre en una conexión propia dentro de `spawn_blocking` (`MemoryStore::snapshot`, `crates/clawg-core/src/memory/db.rs`). La búsqueda de docs que refresca el índice y `restore_memory` también; no medí cuánto tardan, así que no les doy cifra. |
| `STOP_TIMEOUT` es 15 s y su comentario está viejo | **Cierta en el plazo** (`stop.rs:19`). El comentario (`stop.rs:12-18`) decía que el daemon escribe lo que no pudo ingerir en `ingest-pending.jsonl`; ya dice que lo deja en el outbox (`46045aa`). |
| El test de presupuesto no deja margen ni cuenta todas las fases | **Cierta.** `the_drain_deadline_leaves_room_inside_the_stop_timeout` (`stop.rs:63-69`) afirma `DRAIN_DEADLINE + AFTER_DRAIN <= STOP_TIMEOUT` con 10 s (`crates/clawg-core/src/runtime.rs:54`) + 5 s (`stop.rs:61`) = 15 s <= 15 s. No suma la fase HTTP ni la espera del runtime. `AFTER_DRAIN` es una constante del test, no del código: nada la liga a lo que `shutdown_by` hace después (`runtime.rs:267-285`: vaciar el rastreador de accesos, abortar mantenimiento, `checkpoint`). |
| `clawg update` depende de `stop` | **Cierta.** `stop_in(..., STOP_TIMEOUT)` en `crates/clawg-cli/src/update/effects.rs:176` y `:330`; un error se devuelve como `String` (`map_err(|e| e.to_string())`) y el plan lo toma como fallo. `main.rs:155` lo usa para `clawg stop`. |
| El ítem del backlog «`clawg stop` mientras carga el modelo» sigue vigente | **Cierta, con las líneas movidas.** El backlog cita `bootstrap.rs:492` y `main.rs:47`; hoy la carga en `spawn_blocking` está en `crates/clawg-core/src/memory/bootstrap.rs:529` (arranque) y `:396` (recargas tras descarga por inactividad), y el runtime se construye en `crates/clawg-cli/src/main.rs:56` y se suelta al volver de `block_on` (`:62-73`). Ya hay un precedente: `runtime.shutdown_background()` para el hook (`main.rs:74-78`), con el comentario de por qué. |
| Las recargas del embedder (46) también corren en `spawn_blocking` | **Cierta** (`bootstrap.rs:396`). El mismo arreglo del runtime las cubre. |
| El `checkpoint` del apagado vuelve enseguida si otra conexión lo bloquea | **Falsa.** Corre `PRAGMA wal_checkpoint(TRUNCATE)` (`crates/clawg-core/src/runtime.rs:376`, llamado en `:295`), y rusqlite 0.37.0 configura un busy timeout de 5 s en toda conexión que abre (`sqlite3_busy_timeout(db, 5000)`, `rusqlite-0.37.0/src/inner_connection.rs:119`). Según la documentación del SQLite empaquetado (`libsqlite3-sys-0.35.0/sqlite3/sqlite3.h:9872-9923`), el modo TRUNCATE invoca ese manejador mientras espera al escritor y a los lectores que no leen la instantánea más nueva, y mientras espera bloquea a los escritores nuevos. Una conexión abierta sin lectura en curso no lo frena; una lectura en curso de otro proceso sí (`clawg status` o `clawg secrets` con el daemon andando, que abren su propia conexión de lectura, `crates/clawg-core/src/memory/backup.rs:528-537`). Pasados los 5 s sigue como PASSIVE y devuelve «bloqueado», que el código ya registra (`runtime.rs:381-386`). La fase «Resto» puede tardar entonces hasta 5 s sólo en el checkpoint. |
| Un backup cortado a medias deja un archivo que pasa por snapshot | **Falsa.** `snapshot_into` escribe a `<nombre>.db.tmp` y renombra al terminar; si `VACUUM INTO` falla, borra el temporal (`backup.rs:313-328`). Lo que queda abierto es otra cosa: un proceso que termina en medio de `VACUUM INTO` deja ese `.db.tmp`, y como cada snapshot lleva su propia marca de tiempo en el nombre, nada vuelve a borrarlo. Además, cortar la petición HTTP no corta el `VACUUM INTO`, que sigue en su tarea bloqueante hasta terminar. |
| SIGTERM termina el proceso sin drenaje ni checkpoint | **Cierta por construcción, no ejecutada.** Sin manejador, la acción por defecto de Unix mata el proceso y no corre `clear_state` (`serve.rs:192`) ni `Drop` de `StoreLock`. El `memory.lock` lo suelta el SO. |
| Lo aceptado no se pierde | **Cierta.** La fila del outbox se escribe antes del `201`/`202` con `synchronous = FULL` y el daemon reingiere lo pendiente al arrancar (`.claude/CLAUDE.md`, sección del outbox). El WAL se repite al abrir. El costo de un SIGTERM es sólo estado viejo y trabajo de reingesta. |
| El puente tolera un `daemon.json` viejo | **Cierta en lo que alcancé a ver.** `on_failure` vuelve a resolver ante `Unauthorized`, `NotRunning` y `StartTimeout` (`crates/clawg-cli/src/mcp/startup.rs:39-44`). El camino completo de arranque con un archivo viejo lo describe `local-isolation` (`probe_running`, `client.rs:1223`); no lo repetí. |
| En Windows el daemon es un proceso de consola y recibe esos eventos | **No verificada.** El autoarranque lo lanza con `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` (`crates/clawg-cli/src/client.rs:1161-1163`), o sea sin consola; Ctrl-C del padre no le llega a propósito (comentario en `:1155-1156`). Si Windows entrega `CTRL_SHUTDOWN_EVENT`/`CTRL_LOGOFF_EVENT` a un proceso de consola sin consola adjunta es algo que hay que **probar en una máquina Windows**, no suponer. Además esos eventos dan al proceso un plazo del sistema de pocos segundos, bastante menor que 15 s (cifra de la documentación de Windows, que no verifiqué acá). |

## Problema (verificado contra el código)

El apagado del daemon tiene cuatro fases y sólo dos tienen tope:

1. **HTTP en vuelo** (`serve.rs:182-184`): sin tope. Dura lo que dure la petición más lenta (un guardado esperando confirmación hasta 10 s, un backup, un refresco del índice de docs).
2. **Drenaje del ingest** (`runtime.rs:261-273`): `DRAIN_DEADLINE` = 10 s.
3. **Resto** (`runtime.rs:274-284`): accesos, mantenimiento, `checkpoint`. Sin constante propia; el test supone 5 s. El `checkpoint` por sí solo puede gastar esos 5 s en el busy handler si un lector de otro proceso tiene una lectura en curso (ver premisas), así que el supuesto no deja lugar a lo demás.
4. **Desmontaje del runtime de tokio** (`main.rs:62-73`): al soltar el `Runtime` espera a las tareas bloqueantes, entre ellas una carga del modelo en curso (`bootstrap.rs:529`, `:396`) y, desde `b1f22f3`, un snapshot en curso, que corre en `spawn_blocking`. Sin tope. Tras el log «stopped» (`serve.rs:193`) el proceso sigue vivo.

Suma peor caso: 10 s + 10 s + 5 s + la carga del modelo, contra 15 s. `stop` espera el **fin del proceso** (`stop.rs:33-42`, `process.rs`), no el log, así que la fase 4 también cuenta. El resultado: `StopTimeout` → exit 1 en `clawg stop`, y en `update` un `Err` que dispara el deshacer (`effects.rs:176`, `:330`) aunque el daemon se apague bien un instante después.

Lo que el sistema pide no se atiende: sin `SignalKind::terminate()`, `systemctl stop`, el cierre de sesión de un contenedor o `kill` sin señal explícita matan el daemon sin `clear_state` ni checkpoint.

## Lo que ya existe y se reutiliza

- `DRAIN_DEADLINE` y `shutdown_within(workers, deadline)` (`runtime.rs:54`, `:275`): el plazo del drenaje ya es un parámetro probable.
- `shutdown_signal` ya reúne dos fuentes con `select!`; sumar fuentes es local (`serve.rs:203-226`) y su prueba existente (`:263`) muestra cómo probarla sin proceso.
- `shutdown_background()` ya se usa en `main.rs:74-78`; `shutdown_timeout(Duration)` es la forma acotada del mismo gesto.
- `AppCtx.save_timeout` ya permite acortar los 10 s en un test (`api.rs:89`, `:1855`).
- `crates/clawg-cli/tests/binary.rs` ya levanta el binario real con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake|stalled`, y tiene un `kill` por proceso hijo (`binary.rs:271`). `libc` ya es dependencia en Unix (`Cargo.toml:69-70`).

## Enfoques posibles

### Señales

**A. Escuchar SIGTERM (Unix) y los eventos de Windows en `shutdown_signal` (recomendado).** Un tercer brazo del `select!` con `tokio::signal::unix::signal(SignalKind::terminate())`; en Windows, `ctrl_close`, `ctrl_shutdown` y `ctrl_logoff` de `tokio::signal::windows`. Cuando la señal llega se sigue el mismo camino que la bandera de la API.
- **A favor:** local, sin dependencias nuevas (feature `signal` ya está), prueba de binario en Unix.
- **En contra:** en Windows depende de lo no verificado (consola ausente, plazo del sistema). Se implementa con `cfg` y la parte de Windows queda como mejor esfuerzo hasta probarla.

### Fase HTTP

**B. Plazo después de la señal (recomendado).** Rodear el futuro de `axum::serve` con un `select!` contra `señal + HTTP_GRACE`: pasado el plazo se abandona el servidor y se sigue con `runtime.shutdown`. Las peticiones cortadas se devuelven como conexión cerrada.
- **A favor:** no cambia nada en operación normal; un backup legítimo de 20 s sigue funcionando mientras nadie pida parar.
- **En contra:** hay que decidir qué pasa con una petición que se corta a medias. Un guardado ya tiene su fila en el outbox antes del `201`, así que no se pierde. Un backup no se corta: el `VACUUM INTO` sigue en su tarea bloqueante aunque se abandone la respuesta. Ya no ocupa la conexión del drenaje, pero su lectura en curso puede frenar el `checkpoint` hasta 5 s y el runtime lo espera al desmontarse (fase 4). Nunca deja un snapshot a medias con nombre de terminado (`backup.rs:313-328`); sólo un proceso que muere en el medio deja un `.db.tmp` que nada borra.

**C. `TimeoutLayer` por petición.** Corta en operación normal también. Descartado como solución de este problema (ver abajo), aunque podría ser un cambio aparte.

### Carga del modelo y runtime

**D. `runtime.shutdown_timeout(RUNTIME_TEARDOWN)` al volver `serve::run` (recomendado).** Sólo para el subcomando `serve`; los otros siguen como hoy. Una carga bloqueante en curso no se puede cancelar: se la abandona y el proceso termina de todos modos al salir de `main`. El log «model load task panicked: task N was cancelled» deja de ser engañoso si la tarea sólo se descarta.
- **A favor:** cierra el ítem del backlog y cubre las recargas de 46.
- **En contra:** hilos bloqueantes abandonados que siguen corriendo hasta que el proceso sale. Aceptable porque el proceso sale enseguida. Mejor aún si la señal de apagado hace que el bucle de reintento del bootstrap (`retrying in 30s`) no se reabra; a revisar en el diseño.

### Presupuesto

**E. Un presupuesto escrito como constantes con prueba estricta (recomendado).** `HTTP_GRACE + DRAIN_DEADLINE + AFTER_DRAIN + RUNTIME_TEARDOWN + HOLGURA < STOP_TIMEOUT`, con `AFTER_DRAIN` movida del test al código (la usa `shutdown_by` como tope del `checkpoint` y del vaciado de accesos, o al menos se documenta como supuesto probado). Ejemplo de reparto: 3 s + 10 s + 5 s + 2 s = 20 s, lo que ya **no cabe** en 15 s.
- **Consecuencia:** o se acortan fases o crece `STOP_TIMEOUT`. Con 53 el drenaje puede bajar (pausa un guardado largo entre pedazos) y el reparto cabe sin tocar `STOP_TIMEOUT`. Subir `STOP_TIMEOUT` a 30 s alarga lo máximo que un `update` espera antes de deshacer, y retrasa el mensaje de error de `clawg stop`; no pone en riesgo datos, pero es una decisión del usuario.

## Descartado

| Opción | Razón técnica |
|---|---|
| `TimeoutLayer` global para acotar el apagado | Corta también en operación normal: un `VACUUM INTO` legítimo o un refresco largo del índice fallaría sin que nadie haya pedido parar. El problema es el plazo **después** de la señal. |
| Esperar con `stop` sólo a que la API conteste | `stop` debe esperar el fin del proceso: hasta entonces el daemon tiene `memory.db` y la imagen de `clawg.exe` (`process.rs:1-12`). |
| `shutdown_background()` en vez de `shutdown_timeout` | Descarta sin esperar nada, también lo que sí tarda un instante y conviene dejar terminar. |
| Reescribir el drenaje | Ya tiene su plazo y su reanudación; lo ajusta 53. |
| Matar al daemon desde `stop` si no sale | Rompe el contrato de que el daemon es dueño de su cierre; y el repo prohíbe matar procesos `clawg.exe` por nombre. |

## Interacciones

- **outbox-lifecycle (53).** Acota el drenaje pausando un guardado largo entre pedazos; este cambio escribe el presupuesto que ese plazo debe respetar. Conviene definir primero las constantes aquí y que 53 las use.
- **release-and-self-update (47, archivado).** `openspec/specs/self-update/spec.md` descansa en `stop`; un presupuesto estricto vuelve predecible cuándo `update` deshace. No cambia su spec salvo que `STOP_TIMEOUT` crezca (entonces el texto del plazo).
- **install-smoke (40, archivado).** `binary.rs` ya levanta el binario real; los tests de SIGTERM (Unix) caben ahí sin infraestructura nueva.
- **embedder-idle-unload (46, archivado).** Sus recargas corren en `spawn_blocking` (`bootstrap.rs:396`); cubiertas por D.
- **local-isolation.** Comparte el hecho de que tras SIGTERM/SIGKILL quedan `daemon.json`, `token` y `memory.owner` viejos; A resuelve el caso de SIGTERM, el de SIGKILL sigue siendo de aquel cambio.
- **Backlog «`clawg stop` mientras carga el modelo»:** este cambio lo absorbe; al archivar hay que sacarlo de `openspec/README.md`.
- **El snapshot por una conexión propia (`b1f22f3`) y read-connections (57).** Los dos suman lectores dentro del daemon. Un `VACUUM INTO` por su propia conexión, o una búsqueda en un lector, con la lectura en curso cuando llega el `checkpoint`, lo hacen esperar hasta 5 s y frenan a los escritores mientras tanto. El orden del apagado tiene que dejar esas lecturas terminadas, o abandonadas, antes del `checkpoint`, o el presupuesto tiene que contarlas.

## Alcance candidato

1. SIGTERM en Unix y `ctrl_close`/`ctrl_shutdown`/`ctrl_logoff` en Windows en `shutdown_signal`, con `daemon.json` y `token` borrados al salir.
2. Plazo `HTTP_GRACE` después de la señal para la fase HTTP.
3. `shutdown_timeout` del runtime en `serve`, con las constantes nombradas.
4. Constantes del presupuesto en el código y una prueba que las suma con holgura estricta bajo `STOP_TIMEOUT`.
5. Quitar el ítem del backlog.

### No-alcance

- Cancelar de verdad una carga del modelo ya en marcha.
- `TimeoutLayer` en operación normal.
- Reintento o descarte de filas `failed` del outbox.
- SIGKILL y el estado viejo que deja (local-isolation).
- Cambiar el protocolo de `stop` o de `update`.

## Decisiones abiertas

Son del usuario.

- [ ] **`STOP_TIMEOUT`.** ¿Se mantiene en 15 s repartiendo menos tiempo entre las fases, o crece (por ejemplo a 30 s) con el costo para `update` descrito en E?
- [ ] **`HTTP_GRACE`.** ¿Cuántos segundos, y qué se hace con una petición cortada a medias (en particular un backup)?
- [ ] **Windows.** ¿Se implementa el brazo de eventos sin verificarlo, o se prueba antes en una máquina Windows y se declara el soporte sólo si funciona?
- [ ] **Salida del proceso.** ¿`shutdown_timeout` con un plazo, o dejar que el proceso salga y tirar las tareas bloqueantes?
- [ ] **Código de salida tras una señal.** ¿0 como con Ctrl-C, o el estándar 128+señal?

## Cómo se mide

Tests RED primero:

1. **Presupuesto.** Una prueba pura suma `HTTP_GRACE + DRAIN_DEADLINE + AFTER_DRAIN + RUNTIME_TEARDOWN` y exige `< STOP_TIMEOUT` con holgura. Hoy falla porque sólo existe la mitad de las constantes y la prueba vigente admite igualdad.
2. **Petición lenta.** Un daemon de prueba con una ruta lenta inyectada: una petición de apagado termina el servidor dentro de `HTTP_GRACE` (con `tokio::time::pause`, sin esperar de verdad). Hoy no termina.
3. **SIGTERM** (`#[cfg(unix)]`, en `binary.rs`): enviar SIGTERM al daemon de prueba lo hace salir y deja sin `daemon.json` ni `token`. Hoy los deja.
4. **Carga del modelo.** Con `CLAWG_TEST_EMBEDDER=stalled`, `clawg stop` termina dentro del presupuesto. Hoy el proceso espera a la carga.
5. **Windows.** Si se implementa, una prueba manual documentada; no hay forma de automatizar el cierre de sesión en CI.

## Riesgos

- **Un backup en curso alarga el apagado:** no se corta con la petición. Desde que corre en su propia conexión no frena el drenaje, pero puede frenar el `checkpoint` hasta 5 s y retrasa el desmontaje del runtime, que espera a las tareas bloqueantes. Se mide en el diseño antes de elegir `HTTP_GRACE`.
- **Hilos bloqueantes abandonados** (D) siguen vivos hasta salir del proceso; inocuo mientras el proceso sale enseguida.
- **Plazo de Windows menor que el presupuesto:** el sistema puede matar al daemon antes de terminar, sin pérdida de datos pero con estado viejo.
- **Subir `STOP_TIMEOUT`** alarga lo que `update` espera antes de deshacer.
- **Pruebas de tiempo** frágiles: usar tiempo pausado y constantes inyectables, no sleeps reales.
