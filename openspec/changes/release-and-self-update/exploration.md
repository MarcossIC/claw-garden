# release-and-self-update — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L | — (se coordina con client-setup (26), doctor-checks (31) y daemon-hardening (38); ver «Interacciones») | exploración (borrador) |

**Tesis:** hoy la única forma de tener una versión nueva es clonar, compilar en local (`cargo build --release`, con una variable de entorno de rustup en Windows), copiar `clawg.exe` al lado del script y correr `install.ps1` (`README.md:107`). Cada versión repite ese ritual, y no hay licencia, ni tags, ni releases, ni forma de preguntarle al binario si hay algo nuevo. El cambio hace que el CI publique el binario cuando `main` compiló sin fallas, y que el usuario actualice con un comando (`clawg update`) que verifica lo que baja, detiene el daemon con el binario viejo, intercambia el archivo y vuelve a levantar el daemon. Y se hace sin firma Authenticode, con una firma minisign propia y una atestación de GitHub.

## Decisiones ya tomadas (no se reabren acá)

1. Licencia MIT, con el titular «The Claw Garden contributors» (los archivos commiteados no nombran personas). Repositorio `https://github.com/MarcossIC/claw-garden`.
2. Disparador: un workflow que corre después de `CI` en `main` y publica sólo si la versión de `[workspace.package]` de ese commit no tiene tag `vX.Y.Z`. El `chore: bump version to X.Y.Z` es el disparador.
3. Assets: `x86_64-pc-windows-msvc` y `x86_64-unknown-linux-gnu`, `SHA256SUMS`, firma minisign (ed25519) con la clave privada en un secreto, y atestación de GitHub.
4. `clawg update [--check]` en casa: `reqwest`, el sha256 que ya existe y `minisign-verify`. Se descartan cargo-dist/axoupdater y `self_update`.
5. Sin Authenticode al principio. SmartScreen depende de la marca de la web (Zone.Identifier) que agregan los navegadores; `reqwest`, `curl.exe` e `Invoke-WebRequest` no la agregan. Salvedad: Smart App Control en modo de aplicación bloquea binarios sin firma desconocidos con o sin marca. SignPath Foundation es el camino posterior.
6. La búsqueda de versión nueva sólo corre cuando el usuario la pide; el daemon nunca consulta nada.
7. Primera instalación: `scripts/install.ps1` evoluciona a un bootstrap (`irm <url> | iex`) que baja el último release verificado, conservando lo que hace hoy.

## Lo que contradice o matiza esas decisiones

Nada las invalida, pero tres supuestos del encargo no se sostienen con el código, y uno es una dependencia de runtime que el plan no menciona.

| Supuesto | Resultado |
|---|---|
| «`ort` con `tls-native` hace de `libssl.so` una dependencia de runtime» | **Falso.** `tls-native` de `ort-sys` sólo activa `ureq/native-tls`, y `ureq` es una *build-dependency* que baja el ONNX Runtime durante la compilación (`ort-sys-2.0.0-rc.12/Cargo.toml:138-142`, `features tls-native`). No entra al binario. El `libssl` de runtime viene de `reqwest` con sus features por defecto (`Cargo.toml:19`: `default-tls` = `hyper-tls` + `native-tls`, que en Linux es OpenSSL y en Windows es Schannel). Cambiar `tls-native` de `ort` no cambia nada en el binario. |
| «el binario es autocontenido» | **Falso en los dos sistemas.** `ort-sys` enlaza ONNX Runtime **estático** (`build/main.rs:160`, `static=onnxruntime`), así que no hay `onnxruntime.dll` ni `.so`. Pero el binario de Windows importa `MSVCP140.dll`, `VCRUNTIME140.dll` y `VCRUNTIME140_1.dll` (el Visual C++ Redistributable 2015-2022), verificado sobre un `target/release/clawg.exe` local con `rg -a` sobre sus cadenas. En Linux enlaza `libstdc++` dinámico (`build/static_link/mod.rs:24-32`: `stdc++` salvo msvc y apple), además de glibc y `libssl.so` por reqwest. |
| «lo que importa es el .exe sin firmar» | Cierto, pero hay otro costo: **cada release cambia el hash**, así que la reputación de SmartScreen/Defender se reinicia con cada versión. Y un binario sin firma que se reemplaza a sí mismo, lanza procesos desacoplados y edita el PATH de usuario es de los perfiles que más falsos positivos de antivirus levantan. **Sin verificar:** no se pudo medir sin compilar ni publicar. |

El binario de Windows importa además `d3d12.dll`, `directml.dll` y `dxgi.dll` como importaciones **normales**: `objdump -p target/release/clawg.exe` las lista en la tabla de importación junto a `MSVCP140.dll`, `MSVCP140_1.dll`, `VCRUNTIME140.dll` y `VCRUNTIME140_1.dll`, y el directorio de carga diferida está vacío (`Delay Import Directory` con dirección y tamaño 0). `dxcore.dll` aparece sólo como cadena, no como importación. Si falta cualquiera de esas DLL, Windows no arranca el proceso. DirectML viene con Windows desde 10 1903; **sin verificar** en una máquina limpia, así que no se promete «funciona en cualquier Windows» hasta probarlo.

## Estado actual (verificado contra el código)

### Versión, licencia y repo

- `[workspace.package]` tiene `version = "0.18.0"`, `authors = ["MarcossIC"]`, `license = ""` y `repository = ""` (`Cargo.toml:6-9`). No hay `LICENSE*` en la raíz (`git ls-files`), ni ningún tag (`git tag` vacío). Con el primer push, el workflow vería `v0.18.0` sin tag y publicaría la versión actual de `main`.
- `authors` nombra una persona, y la regla del repo es que lo commiteado no nombra personas. Hay que decidir si se cambia por el titular colectivo al llenar los metadatos (ver «Decisiones abiertas»).
- La versión la reportan `clawg --version` (`cli.rs:15`), `GET /health` (`daemon/api.rs:772`), el `serverInfo` del MCP (`mcp/server.rs:1207`), `clawg doctor` (`doctor.rs:121`) y el estado que escribe el daemon (`daemon/serve.rs:152`). Todo sale de `env!("CARGO_PKG_VERSION")`: es lo que el actualizador compara.

### CI actual

- `ci.yml` corre en `push` a `main` y en `pull_request`, con `paths-ignore` para `docs/**`, `openspec/**`, `.claude/**`, `.cursor/**`, `.gemini/**`, `scripts/**`, `README.md` y `AGENTS.md` (`ci.yml:5-26`); permisos `contents: read` (`:28-29`); `concurrency: ci-${{ github.ref }}` con `cancel-in-progress: true` (`:31-33`); matriz `ubuntu-latest` y `windows-latest` con `fail-fast: false` (`:39-41`); `clippy` con `-D warnings` y `cargo test --workspace` (`:53`, `:56`).
- Consecuencias para un workflow que dependa de él (detalle en «El workflow»): un commit que sólo toca `scripts/**` o docs **no genera ejecución de CI**, así que no puede disparar un release (correcto para el bump, que toca `Cargo.toml` y `Cargo.lock`, pero hay que decidir de dónde se sirve el script de arranque); y un push posterior cancela la ejecución anterior de la misma rama, con conclusión `cancelled`.

### El instalador actual (`scripts/install.ps1`)

- Copia sólo `clawg.exe` desde el directorio del script a `%LOCALAPPDATA%\Programs\clawg` (`:139-144`, `:207`).
- Antes de tocar nada decide: si hay una instalación legada («Claw Garden» en el registro) u otro `clawg.exe` en el PATH fuera del directorio de instalación, se niega salvo `-RemoveLegacy` (`:146-163`).
- Detiene el daemon `prod` **con el binario anterior**: `stop` y luego `status`; `stop` sale con 1 tanto si no había daemon como si no terminó a tiempo, y `status` sale con 0 sólo si todavía contesta, que es el caso que se rechaza (`:165-183`).
- Borra los `clawg.exe.old-*` de corridas anteriores (los que siguen mapeados quedan para la próxima) y **renombra** el binario previo a `clawg.exe.old-<marca>` en vez de sobrescribirlo, porque Windows no deja sobrescribir un ejecutable mapeado pero sí renombrarlo; si la copia falla, devuelve el viejo (`:187-213`).
- Corre `clawg path add --dir` y verifica con el PATH fresco del registro que una terminal nueva resuelva al archivo instalado (`:215-223`).
- Es el molde del intercambio que tiene que hacer `clawg update`. Lo que **no** hace: volver a levantar el daemon, ni cuidar los puentes MCP abiertos.

### Comandos y reuso (pregunta 1)

- El parseo es puro y está separado de la ejecución: `Command` en `cli.rs:31-137`, `loads_the_model` en `cli.rs:166`, y el reparto en `main.rs:78-257` (`dispatch`). Un subcomando nuevo agrega una variante y un brazo; `update` no carga el modelo, así que no entra en `loads_the_model`.
- `stop`: `stop_in(base_dir, STOP_TIMEOUT)` toma el pid de `/health` (no de `daemon.json`, `stop.rs:25-27`), pide `POST /v1/shutdown` y espera a que el proceso termine (`stop.rs:29-42`); `STOP_TIMEOUT` es 15 s (`stop.rs:19`) y cubre el vaciado del outbox, que tiene tope de 10 s (`DRAIN_DEADLINE`, `clawg-core/src/runtime.rs:54`). Lo que el daemon no alcanza a ingerir queda en `ingest_outbox` y se retoma al arrancar (`.claude/CLAUDE.md`, sección del outbox), así que parar para actualizar no pierde saves aceptados. El comentario de `stop.rs:12-18` todavía habla de `ingest-pending.jsonl`, que el outbox ya reemplazó: queda desactualizado, no afecta a este cambio.
- `status`: `main.rs:93-119`; `probe_running(base_dir, false)` (`client.rs:1043`) es la pregunta «¿hay un daemon que contesta?» sin arrancar ninguno, y es lo que usan `memories`, `undelete` y `projects`.
- `doctor`: `doctor::analyze` (`doctor.rs:46-74`) es pura y dice cuál es el `clawg` del PATH, si está tapado (`shadowed`) y si hay copias (`duplicates`); `doctor::run` (`:112-125`) la alimenta con `current_exe` y el PATH. `update` puede reusarla para avisar que está por actualizar una copia que no es la que una terminal nueva ejecutaría.
- `path`: sólo Windows (`main.rs:259-302`), idempotente a propósito para que el instalador lo corra siempre.

### Arranque del daemon (pregunta 2)

- `ProcessSpawner::spawn_serve` (`client.rs:939-1001`) lanza **`std::env::current_exe()`** con `serve --profile <p>`, con stdin nulo, stdout y stderr al log `logs/serve.log` (en modo append), `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` en Windows y `disinherit_std_handles()` (`detach.rs:29-35`) para no heredar los pipes del host; en Unix, `process_group(0)`.
- **Trampa para el actualizador:** `current_exe()` es el proceso que corre `clawg update`, o sea el binario **viejo**. Después del intercambio, `ProcessSpawner` levantaría el daemon viejo. Es el caso en que hay que arrancar el binario nuevo por ruta explícita. En Linux, además, `/proc/self/exe` pasa a decir `(deleted)` si el archivo se reemplazó. En Windows, qué devuelve `current_exe` tras renombrar un ejecutable en uso **no se verificó**; no hay que depender de eso. Un spawner nuevo (o `ProcessSpawner` con la ruta como campo) resuelve ambos; hay un trait `DaemonSpawner` pensado justamente para inyectar el arranque en los tests (`client.rs:922-930`).
- `start_daemon` (`client.rs:1075`) sondea `/health` hasta que contesta o vence el plazo, y es lo que el actualizador usa después de arrancar.
- Los puentes **no** lo arrancan a mitad de sesión: ante un 401, una conexión rechazada o un arranque vencido, `ClientHandle::reresolve` relee `daemon.json` y `token` y repite la llamada una vez, pero «never starts one: an open bridge may be the binary an update just replaced, and a daemon stopped on purpose must stay stopped» (`mcp/startup.rs:96-118`, `on_failure` en `:39-44`). El código ya prevé este cambio. Consecuencia de diseño: **si el actualizador detuvo un daemon, tiene que dejar uno corriendo** (el nuevo), o los editores abiertos se quedan sin memoria hasta reiniciarse; y si no había daemon antes, no tiene que arrancar uno.

### Compatibilidad de un puente viejo con un daemon nuevo (pregunta 3)

- El puente solo habla con el daemon por HTTP (`client.rs`); no abre el store. Un puente viejo (el binario viejo, mapeado, que el editor sigue corriendo) llama al daemon nuevo con las formas de petición de su versión.
- Precedente de compatibilidad hacia atrás: `/v1/disputes/resolve` «para puentes más viejos» (`.claude/CLAUDE.md`, API del daemon); los campos nuevos de `Health` llevan `#[serde(default)]` para leer un daemon anterior (`client.rs:119-140`, tests `a_health_without_ingest_still_parses` y similares, `client.rs:1122-1136`); un hit sin `exact` o sin `agent_id` se lee como antes (`client.rs:1840-1849`).
- Riesgo: varias peticiones del daemon son `#[serde(deny_unknown_fields)]` (`daemon/api.rs:378`, `:407`, `:420`, `:430`, `:595`, `:666`, `:1374`). Un puente **nuevo** contra un daemon **viejo** que reciba un campo desconocido recibe un 400. Es el sentido que importa menos acá (el actualizador reemplaza el daemon antes de que haya puentes nuevos), pero se da si el usuario abre un editor mientras el daemon viejo sigue vivo. La regla que hay que fijar es la inversa y es la que sí cubre una actualización: **el daemon nuevo acepta todo lo que mandaba el anterior**; quitar o renombrar un campo de la API es un cambio que rompe puentes abiertos.
- `version_warning` (`memories.rs:332-339`, usado en `:398`) sólo avisa en `clawg memories`, y sólo compara por igualdad: no distingue «más nuevo» de «más viejo». El puente sólo registra ambas versiones en el log (`startup.rs:109-114`). No hay hoy ninguna política de qué versiones de puente y de daemon son compatibles. El diseño tiene que decidir si la ventana de compatibilidad es «la versión inmediata anterior» y probarla, o si `update` avisa que los editores abiertos deben reiniciarse.
- El contrato que ve el modelo (herramientas y esquemas) está fijado en `agent_contract.json`; un puente viejo mantiene sus herramientas hasta que el editor lo relance, aunque el daemon sea nuevo.

### Migraciones y vuelta atrás (pregunta 4)

- Al abrir un store con migraciones pendientes se escribe antes un snapshot `pre-migration` a `<data>/backups` (`clawg-core/src/memory/db.rs:200-215`, razón `Reason::PreMigration`, `backup.rs:121`); si el store es de un clawg **más nuevo** se rechaza sin tocarlo (`check_schema`, `db.rs:389-397` → `MemoryError::SchemaNewer`), igual que una migración divergida (`SchemaDiverged`). El texto del error ya dice «Install that version again, or restore a backup taken before the upgrade with `clawg restore`» (`memory/error.rs:55-64`).
- Por lo tanto la vuelta atrás de una actualización es: el binario anterior (que `clawg update` conserva como `.old-*`) **más** `clawg restore` del snapshot `pre-migration`. Lo escrito entre la actualización y el restore se pierde salvo que el usuario lo respalde antes (`clawg backup`). Hay que documentarlo, y que el reporte de `update` diga qué snapshot es el de vuelta atrás.
- Riesgo ya conocido en el backlog: la poda conserva sólo los 5 snapshots automáticos más nuevos y no mira el motivo (`backup.rs:230` `AUTO_KEEP`, `:235` `to_prune`), así que varias actualizaciones seguidas pueden sacar el `pre-migration` que era la vuelta atrás. Con actualizaciones fáciles de correr, ese riesgo sube. Conviene cerrar ese arreglo (conservar el más nuevo de cada motivo) antes o junto con este cambio.
- Una actualización no necesita un snapshot propio mientras la migración lo haga sola; pero una versión **sin** migración pendiente cambia el binario sin snapshot y la vuelta atrás es sólo el binario. Eso es seguro: el esquema no cambió.

### ¿El binario del release es autocontenido? (pregunta 5)

Verificado leyendo `ort-sys-2.0.0-rc.12` y un binario local:

| Aspecto | Windows (`x86_64-pc-windows-msvc`) | Linux (`x86_64-unknown-linux-gnu`) |
|---|---|---|
| ONNX Runtime | estático, bajado en build de `cdn.pyke.io` con hash fijo (`build/download/dist.txt` filas `none`; `build/main.rs:97-160`) | ídem |
| C++ runtime | `MSVCP140.dll`, `VCRUNTIME140.dll`, `VCRUNTIME140_1.dll` (VC++ Redistributable) | `libstdc++.so.6` dinámico (`static_link/mod.rs:24-32`) |
| TLS de reqwest | Schannel (del sistema) | OpenSSL: `libssl.so.3` (o `.1.1` según la distro del runner) |
| libc | UCRT (`api-ms-win-crt-*`, viene con Windows 10+) | glibc del runner: la versión mínima del binario es la de `ubuntu-latest` |
| tamaño | 42.878.976 bytes el `target/release/clawg.exe` local | sin medir |

Que `ort-sys` sólo trae binarios de `x86_64` y `aarch64` para `gnu` y `msvc` (no hay `musl` ni `x86_64-apple-darwin`; `dist.txt`) cierra una salida: **un binario estático con musl no es posible** con `download-binaries`.

Opciones para Linux:

| Opción | A favor | En contra |
|---|---|---|
| A. `ubuntu-latest` tal cual | cero trabajo, es lo que ya prueba CI | glibc y OpenSSL del runner (hoy 24.04: glibc 2.39, OpenSSL 3); no corre en distros más viejas; el requisito cambia solo cuando GitHub mueve `latest` |
| B. fijar un runner más viejo (`ubuntu-22.04`) | glibc 2.35; sigue siendo un runner de GitHub | los runners viejos se retiran; la versión de glibc de los binarios de pyke **sin verificar** (si pidieran más que la del runner, el enlace fallaría de entrada, lo que al menos avisa) |
| C. contenedor tipo manylinux | glibc mínima elegida | más infraestructura; pyke publica sus `.a` para su propia toolchain y no se verificó que se enlacen con otra |
| D. `rustls` en lugar de `native-tls` | saca `libssl` de la lista de dependencias de runtime (queda `libstdc++` y glibc) | cambia el comportamiento de TLS de la descarga del modelo y de `update`; raíces de confianza distintas (ver abajo); agrega `ring` (C/asm) al build |

Recomendación: **D + B**, y declarar en el README la glibc mínima. `rustls` es independiente de la distro y la descarga del modelo (`reqwest::get`, `model_download.rs:671`) y los tests (mockito, HTTP plano) no dependen de OpenSSL. Detalle de D:

- `reqwest` está declarado una sola vez en `[workspace.dependencies]` con las features por defecto (`Cargo.toml:19`) y lo usan `clawg-core` (`model_download.rs:671`) y `clawg-cli` (`client.rs:482`, `api.rs:1660` sólo en tests). Un miembro no puede quitar `default-features` si el espacio de trabajo las deja; el cambio tiene que hacerse en la raíz (`default-features = false` más `rustls-tls-native-roots`, `charset`, `http2`, `system-proxy`, que son las que se pierden).
- `rustls-tls` usa las raíces empaquetadas de Mozilla (`webpki-roots`); `rustls-tls-native-roots` usa las del sistema, que es el comportamiento de Schannel/OpenSSL y el que respeta una CA corporativa. Recomendado `native-roots` para no cambiar quién es de confianza.
- Alternativa más chica: `rustls` sólo en Linux y Schannel en Windows, con features por `cfg(target)`; evita traer `ring` a Windows pero deja dos pilas que probar. Recomendado una sola.
- `Cargo.lock` ya contiene `hyper-rustls`, `rustls` y `ring` por `ureq` (build-dependency), así que no es un árbol nuevo, pero **sí** son dependencias del binario cuando pasan a ser de `reqwest`.

Para Windows: el VC++ Redistributable no es parte de Windows. Opciones: (i) el bootstrap lo detecta y avisa con el enlace oficial; (ii) el bootstrap lo instala (necesita elevación o `/quiet` por usuario no existe); (iii) enlazar el CRT estático (`+crt-static`), que **choca** con las `.lib` de ORT, compiladas con `/MD`, y no se puede usar; (iv) distribuir las DLL junto al `.exe` (despliegue local de la aplicación está permitido) a costa de que `update` maneje varios archivos. Recomendado (i) en el primer cambio, con una verificación al arrancar el bootstrap, y dejar (iv) como salida si la gente lo pisa.

## Detectar «esto es un binario de release instalado» (pregunta 6)

- Hoy el binario no sabe de dónde vino. `Profile::compiled_default` (`paths.rs:75-81`) decide `dev` o `prod` por `cfg!(debug_assertions)` y `Profile::resolve` rechaza `prod` en debug; no distingue un `cargo build --release` local de uno del pipeline.
- `std::env::current_exe` y `doctor::run` dan la ruta (`doctor.rs:113`), pero una heurística por ruta («no está bajo un `target/`») es frágil: un `cargo install` o un binario copiado a mano pasan.
- Propuesta: una **marca positiva de compilación**, `option_env!("CLAWG_DIST_TARGET")` (el triple de la plataforma), que el workflow de release pone y un `cargo build` local no. Con ella, `update` (a) rechaza todo build sin marca («compilado en local: actualizá con `git pull` y recompilando»), (b) sabe por sí mismo qué asset pedir, sin adivinar el triple en runtime (un `x86_64-unknown-linux-musl` jamás pediría el asset `gnu`), y (c) un build debug queda excluido de todos modos por `cfg!(debug_assertions)`. Costo: ninguno en dependencias; `option_env!` se rastrea en el dep-info de rustc, así que cambiar la variable recompila.
- Para los tests la marca y el release se inyectan: la lógica de decisión es una función pura `(marca, debug_build, exe, perfil) -> Permitido | Rechazado(motivo)`.
- `update` también debe rechazar si `ProfileArg::resolve` da `dev` (un build de release que usa `--profile dev` no es el que se instala), o actualizar sólo el perfil `prod`. A decidir (ver abiertas).

## Estrategia de pruebas (pregunta 7)

Lo que ya existe y se reusa:

- `crates/clawg-cli/tests/binary.rs` lanza el binario real en un home temporal (`Sandbox`, `binary.rs:32-70`) con `CLAWG_TEST_HOME` (`paths.rs:168-189`) y `CLAWG_TEST_EMBEDDER=fake` (`bootstrap.rs:295`); sólo un build con la feature `testing` lee ambas, y `tests/conventions.rs:84-100` falla si un manifiesto enciende `testing` fuera de `[dev-dependencies]`.
- La descarga del modelo tiene una URL base inyectable (`CLAWG_HF_BASE_URL`, `model_download.rs:196-198`), que **no** está detrás de `testing`: se lee en producción.
- El trait `DaemonSpawner` (`client.rs:928`) es el patrón para probar el arranque sin lanzar el modelo real.

Qué es puro (se prueba sin disco ni red, con la política de `.claude/CLAUDE.md` de separar decisión de E/S):

1. Comparación de versiones (`semver` sin dependencia nueva: `X.Y.Z` con tres enteros, rechazando todo lo demás; sólo se actualiza hacia una versión **estrictamente mayor**, lo que cierra el ataque de volver a una versión vieja con firma válida).
2. Nombre del asset por triple (`clawg-<triple>[.exe]`).
3. Elección del release: del JSON de la API, el último que no es borrador ni prerelease, con el asset del triple propio y con `SHA256SUMS` y su `.minisig`.
4. Parseo de `SHA256SUMS` (una línea `hash␣␣nombre`), y la comparación contra el hash calculado.
5. La decisión «¿este binario puede actualizarse?» (arriba).
6. El **plan de pasos** como datos: `[Download, VerifySignature, VerifyHash, SmokeNewBinary, StopDaemon?, SwapFiles, StartDaemon?, VerifyVersion]` según el estado inicial (había daemon o no), y qué deshacer ante cada falla. Es lo que hace probable el «volver atrás» sin el sistema operativo.

Qué es E/S: HTTP (con la URL base inyectable), escribir el archivo preparado junto al destino, el intercambio por renombre, parar y arrancar el daemon.

Pruebas con el binario (en `binary.rs`, o un archivo hermano):

- Un servidor de release falso (un `tokio` + `axum` en el proceso de la prueba, ya son dependencias del crate; el repo usa `mockito` en `clawg-core`) que sirve `latest`, el binario y `SHA256SUMS` con su firma. Las claves de firma son **de prueba**, generadas en el propio test (o fijas en el repositorio y claramente de prueba), nunca la clave de producción.
- Una URL base y una clave pública sustituibles **sólo en un build `testing`**, por la misma regla que el home de prueba. No pueden ser variables de producción: una variable de entorno que redirija las actualizaciones, aunque la firma la proteja, es superficie que no hace falta.
- Problema concreto: el tests-binary es un build debug con `testing`; `update` lo rechazaría por ser debug y por no tener la marca. Hace falta que la marca y el «build debug» tengan un reemplazo en `testing` (misma regla), o que el núcleo (`plan`/`apply`) sea una función de biblioteca que el test llama con un `DaemonSpawner` y un cliente falsos, y `binary.rs` pruebe sólo el cableado. Se recomienda lo segundo para la lógica y un solo escenario por el binario real.
- El binario «nuevo» que sirve el servidor falso puede ser una copia del propio binario de prueba; su versión sería la misma que la actual, así que el test necesita una versión actual sustituible (`CLAWG_TEST_*`, también `testing`) o servir un release anunciado con una versión mayor y un humo `--version` flexible. Decisión del diseño.

Escenarios que no pueden faltar: firma inválida, hash que no coincide, release más viejo o igual, asset ausente para el triple, descarga cortada, falta de espacio, el intercambio falla y se restaura el binario viejo, daemon que no vuelve a contestar tras arrancar, había daemon / no había daemon, y un store más nuevo que el binario (el daemon nuevo se niega, `update` lo informa y **no** deja al usuario sin daemon ni con un binario que no abre su base: ver riesgos).

## El workflow (pregunta 8)

Forma propuesta (sin escribirlo todavía): un `release.yml` con `on: workflow_run: workflows: [CI], types: [completed], branches: [main]`.

- **Qué evalúa `workflow_run`.** Corre con el archivo de workflow de la rama por defecto y con acceso a secretos. Por eso mismo hay que fijar la condición completa: `github.event.workflow_run.conclusion == 'success'` **y** `github.event.workflow_run.event == 'push'` **y** `head_branch == 'main'`. Sin la de `event`, una ejecución de CI de un PR de un fork con rama llamada `main` pasaría el filtro `branches` y el workflow construiría y firmaría código ajeno con la clave del proyecto. Y el código que se construye es el de `head_sha`, nunca el de la rama en ese momento.
- **Qué commit se etiqueta.** El `head_sha` de la ejecución exitosa: `actions/checkout` con `ref: ${{ github.event.workflow_run.head_sha }}`, se relee ahí `[workspace.package] version` y se crea el release con `--target <head_sha>`. Si `main` avanzó mientras tanto, el tag sigue en el commit que pasó CI, que es lo que se probó.
- **Cancelaciones.** Con `cancel-in-progress` un push nuevo cancela la ejecución anterior, que termina con conclusión `cancelled` y no dispara nada. Si ese commit era el del bump, el release lo sacará la ejecución siguiente (la versión de **ese** commit todavía no tiene tag), así que el tag cae un commit más tarde que el bump. Es aceptable y hay que decirlo; lo que no se puede aceptar es publicar en `cancelled` o `failure`.
- **Commits que no corren CI.** Un push que sólo toca `scripts/**`, `docs/**`, `openspec/**`, `README.md`, etc. no genera ejecución de CI (`ci.yml:5-26`), por lo tanto tampoco release. El bump (`Cargo.toml` y `Cargo.lock`) sí corre CI. Hay una consecuencia: el script de arranque no puede ir adjunto a un commit que no compila, así que o se sirve desde `main` por la URL cruda (no versionado con el binario) o se adjunta como asset del release y la URL «último» apunta a `releases/latest/download/install.ps1`. Recomendado lo segundo: el script y el binario son de la misma versión, y se firma junto con el resto.
- **Idempotencia y carreras.** `concurrency: { group: release, cancel-in-progress: false }`, y un paso inicial que sale limpio si `gh release view vX.Y.Z` ya existe. Crear el release con todos los assets **al final** (un job `publish` con `needs: [build-windows, build-linux]`), de modo que nunca exista un release visible sin sus archivos: `releases/latest` lo vería y `update` fallaría con «falta el asset». Si un build falla no se publica nada y la ejecución siguiente de CI en `main` lo reintenta mientras la versión siga sin tag.
- **Permisos.** A nivel de workflow `contents: read`; los jobs de build no necesitan más. El job `publish`: `contents: write` (crear el tag y el release), `id-token: write` y `attestations: write` (atestación de procedencia). Acciones fijadas por SHA, no por etiqueta móvil.
- **Qué se compila.** `cargo build --release -p clawg-cli` por plataforma, **nunca `--workspace`**: `clawg-eval` enciende la feature `eval` de `clawg-core` y la unificación de features la llevaría al binario que se entrega (`Cargo.toml` de `clawg-core`, `eval`, «Off by default so production builds never link it»). `testing` no entra: sólo está en `[dev-dependencies]` y `cargo build` no las compila (`specs/install`, «Build de test»). Con `CLAWG_DIST_TARGET=<triple>` en el entorno. Un paso humo: `clawg --version` y `clawg doctor` sobre el binario recién compilado, y comparar su versión con el tag.
- **Firma minisign.** La clave privada protegida con contraseña, guardada como dos secretos (`MINISIGN_SECRET_KEY` y su contraseña) en un *environment* del repositorio con revisión obligatoria si se quiere un freno humano; se firma `SHA256SUMS` (que ya cubre los dos binarios y, si se adjunta, el script) y se sube `SHA256SUMS.minisig`. Firmar el archivo de sumas y no cada binario deja una sola firma y permite verificar el nombre. La clave pública va en el repositorio y **embebida en el binario**. El código del actualizador verifica la firma con `minisign-verify` (0.3.0, publicada el 2026-09-25, MIT, sin dependencias, según crates.io; **verificar su API y fijar versión al diseñar**: la versión es de hace pocas semanas y la regla del repo para acciones y herramientas externas es fijar una versión probada).
- **Rotación y compromiso de la clave.** Una clave embebida no se rota sin un binario nuevo. Diseñar desde el principio una lista de claves aceptadas (la actual y la siguiente) y decidir cómo se revoca: si la clave se filtra, un atacante con la clave y control de la red puede servir un «release» válido; el único freno que queda es la comparación de versión mayor y la atestación de GitHub, que el actualizador no verifica (sólo la verifica quien la consulta a mano con `gh attestation verify`). Hay que decirlo en el README y no vender la atestación como parte de la cadena de `update`.
- **La atestación.** `actions/attest-build-provenance` sobre los binarios. Requiere repo público (lo es) y los dos permisos de arriba. Queda para auditoría manual; la verificación que hace `update` es minisign + sha256.
- **Qué publicar como asset.** Binarios sueltos (`clawg-x86_64-pc-windows-msvc.exe`, `clawg-x86_64-unknown-linux-gnu`) en vez de un `.zip`/`.tar.gz`: no hace falta ninguna dependencia de descompresión (`zip`, `flate2`, `tar`) y cumple «ninguna dependencia sin un uso real». El costo es descargar unos 40 MB sin comprimir. Si el tamaño molesta, se agrega después un comprimido para humanos sin que `update` lo use.

## Cómo hace el intercambio `clawg update` (pregunta 2 y 3, síntesis)

Orden propuesto, que reproduce el de `install.ps1` y sólo agrega lo que ahí falta:

1. Decidir si puede actualizarse (marca de compilación, perfil, ruta). Sin red ni disco.
2. Consultar el último release (`releases/latest`; hay que mandar un `User-Agent`, porque `reqwest::Client::new()` no manda ninguno y GitHub contesta 403 sin él; el límite sin autenticar es de 60 consultas por hora y por IP, suficiente para algo que se corre a pedido). Comparar con `CARGO_PKG_VERSION`. `--check` termina acá con un código de salida que distinga «hay nueva» de «al día» de «error».
3. Bajar el binario a un archivo temporal **en el mismo directorio** del destino (el renombre de un volumen a otro no es atómico), bajar `SHA256SUMS` y su `.minisig`, verificar la firma con la clave embebida, luego el hash. Todo antes de tocar nada instalado.
4. Humo del binario preparado: `<nuevo> --version` debe decir la versión del release.
5. Anotar si había daemon (`probe_running(base, false)`).
6. Si lo había: `stop_in(base, STOP_TIMEOUT)` con **el binario viejo**, el único que sabe hablarle. Si no termina, no se reemplaza nada (la misma regla que `install.ps1:180-182`).
7. Renombrar el actual a `clawg.exe.old-<marca>` (en Windows, un ejecutable en ejecución se puede renombrar: `install.ps1:192-205`) y mover el preparado al lugar. En Linux basta `rename` encima. Si falla el segundo paso, devolver el viejo.
8. Si había daemon: arrancar el **nuevo** por ruta explícita (no `ProcessSpawner`, ver arriba) y esperar `/health` con la versión esperada. Si el daemon nuevo se niega (store más nuevo, migración divergida) o no contesta, **restaurar el binario viejo y arrancarlo**: el usuario no puede quedar sin daemon por haber actualizado.
9. Informar versión anterior y nueva, el snapshot `pre-migration` si se escribió, y que los editores con un puente abierto siguen con el binario viejo hasta reiniciarse.
10. Los `clawg.exe.old-*` de actualizaciones anteriores se borran al empezar (los que siguen mapeados quedan).

Notas:

- Un `update` que corre mientras otro `update` o un `serve` arranca: hay que tomar algún cierre. El lock de `memory.lock` es del store, no de la instalación; basta un archivo de bloqueo junto al binario o aceptar que dos `update` simultáneos son un error del usuario y fallar con un mensaje claro. A decidir.
- Windows puede tener el nuevo `.exe` bloqueado por el antivirus durante unos segundos tras escribirlo. `install.ps1` no lo enfrenta porque copia y sigue; un `update` que lo ejecuta de inmediato (el humo) puede toparse con eso. Reintentos con tope.
- Permisos de la ruta de instalación: `%LOCALAPPDATA%\Programs\clawg` es del usuario; en Linux no hay instalador ni ruta por defecto. Si el binario está en `/usr/local/bin` sin permiso de escritura, `update` debe decirlo antes de bajar nada, no al final.
- Lo único destructivo es el intercambio; el resto (descarga, verificación, humo) puede fallar sin efectos.

## Interacciones con cambios pendientes (pregunta 9)

- **client-setup (26):** `clawg setup claude-code` registra el MCP con la **ruta absoluta del binario en ejecución** y avisa que una reubicación del binario rompe el registro (`client-setup/exploration.md`, riesgos, y «Descartado»). `update` conserva la ruta (reemplaza en el lugar), así que no la rompe; pero si el bootstrap cambia el directorio de instalación por defecto, sí. Decisión compartida: la ruta es estable (`%LOCALAPPDATA%\Programs\clawg\clawg.exe`) y se documenta como contrato. 26 usa `plan(estado, opciones)` puro con `user_path.rs` de molde; `update` sigue el mismo molde, y ninguno edita lo del otro.
- **doctor-checks (31):** propone el chequeo `mcp.binary_missing` y lee la versión de esquema sin migrar. Una actualización que falla a medias (queda un `.old-*` sin el binario nuevo, o un `clawg.exe` ausente) es exactamente el caso que ese chequeo debería ver, y un daemon de versión distinta a la del `clawg` en el PATH (`version_warning`, hoy sólo en `memories`) es un chequeo natural. No es una dependencia dura: se deja anotado para que 31 lo sume. `update` no depende de 31.
- **daemon-hardening (38):** propone validar `Host` y rechazar todo `Origin`, y deja `/health` sin token por compatibilidad entre versiones (`daemon-hardening/exploration.md`, secciones de `/health`). El actualizador pide `/health` sin `Origin` y sin cambiar el `Host` de `127.0.0.1`, así que la regla no lo afecta; la lección de ese documento («no pedir token a `/health`: rompe `probe_running` y `clawg stop` entre versiones») es la misma que hace posible parar un daemon viejo con un binario nuevo, y hay que preservarla. Si 38 cambiara la forma de `Health`, `update` tiene que seguir leyendo la anterior.
- **Backlog de `openspec/README.md`:** (a) la poda de snapshots que no mira el motivo, ya citada (conviene cerrarla antes); (b) no hay ninguna entrada del backlog sobre «instalador que comprueba que el daemon está parado», pero `install.ps1` ya lo hace (`:165-183`) y el bootstrap lo hereda; (c) la fila `failed` del outbox sin salida: cuando `update` detiene el daemon no cambia, pero `/health` mostrará pendientes que el daemon nuevo retoma.
- **agent-lifecycle-hooks (25):** los hooks los invoca el cliente con `clawg hook <evento>`; un `update` cambia el binario que ejecutan, así que 25 debe tolerar un binario que se reemplaza entre dos hooks (cada hook es un proceso corto: no hay estado).
- **memory-portability (43) / history-import (44):** no se tocan. Sí conviene que 43 declare su versión de formato antes de que haya muchas instalaciones actualizándose solas.
- **`openspec/specs/update/`** ya existe y es la especificación de `update_memory`: la capacidad nueva MUST llamarse distinto (`release` para el pipeline y `self-update` para el comando; recomendado **dos** especificaciones, porque una describe el pipeline y la otra el comportamiento del binario).
- **`openspec/specs/install/`** describe cómo la suite prueba el binario y el CI (`specs/install/spec.md`, secciones «El binario sobrevive…» y «CI»). Este cambio la **modifica** en dos puntos: la sección CI gana el workflow de release (que no es parte de la suite) y la regla de que `testing` no entra en el binario que se entrega pasa a ser verificable sobre el artefacto del release. No hay que mezclar el pipeline de publicación dentro de esa spec.
- **`config.yaml`, regla de datos:** este cambio no toca esquema, identificadores ni contrato de embedding, pero sí es lo que **ejecuta** las migraciones de todas las instalaciones. La prueba de «base sembrada en la versión anterior» ya la cubren los tests de `db.rs`; hay que sumar un escenario de actualización completa con el store sembrado (ver «Cómo se mide»).

## Docs que cambian (pregunta 10)

- `README.md`: el paso 4 (`:107-131`) deja de ser «copiá el binario y corré el script» y pasa a ser la línea `irm <url> | iex`; el párrafo del script no firmado y `-ExecutionPolicy Bypass` (`:119`) cambia (con `irm | iex` no hay política de ejecución de archivo); la nota de Linux y macOS (`:131`: «esa ruta no está probada») pasa a decir qué se publica y cómo actualizar; el apartado de «Conectar a Claude Code» (`:221`) no cambia. Va una sección nueva «Actualizar» (`clawg update`, `--check`, la vuelta atrás con `clawg restore`) y una de «Verificar lo que bajaste» (minisign, `SHA256SUMS`, `gh attestation verify`), con el aviso de SmartScreen/Smart App Control.
- `.claude/CLAUDE.md`: la lista **Commands** suma `clawg update [--check]`; la sección «Versioning» (qué es disparador de release: el `chore: bump version to X.Y.Z`) y la de CI (el workflow de release); la sección «Rules for agents» no cambia. La lista de herramientas MCP no cambia, así que `agent_contract.json` no se toca.
- `AGENTS.md`, `.cursor/rules/cursor.mdc` y `.gemini/GEMINI.md`: punteros; revisar si repiten la lista de comandos.
- `LICENSE` (nuevo) y `license = "MIT"` / `repository` en `Cargo.toml:8-9`.
- `scripts/install.ps1`: se parte en dos usos (instalar un binario local, que sigue sirviendo para desarrollo; y bajar el release). Los `README` de `scripts/` no existen.

## Enfoques

### Del actualizador

| Opción | A favor | En contra | Esfuerzo |
|---|---|---|---|
| **A. En casa (decisión 4)** | maneja el daemon y los puentes; una sola cadena de confianza (minisign); una dependencia nueva (`minisign-verify`) | hay que escribir y probar el intercambio, que es el código más delicado del cambio | M–L |
| B. `self_update` o `axoupdater` | menos código | no entienden el daemon ni los puentes; `axoupdater` ata a cargo-dist | (descartados) |
| C. Sólo el bootstrap, sin `update` | casi nada de Rust | el usuario repite `irm \| iex` y el daemon no se maneja: justo lo que el encargo quiere evitar | S |

### Del bootstrap de Windows

| Opción | A favor | En contra |
|---|---|---|
| **A. `irm <url> \| iex` del `install.ps1` del release** | una línea; ningún `-ExecutionPolicy`; el binario baja sin marca de la web | ejecuta código sin leerlo: hay que publicar el hash del script y la firma, y decir cómo leerlo antes |
| B. Descargar el `.exe` desde el navegador | trivial | agrega la marca de la web: SmartScreen y la protección de reputación (sin firma Authenticode) |
| C. `winget`/Scoop | verificado por un tercero | un manifiesto y un hash por versión; trabajo aparte |

## Recomendación

Hacerlo en **tres tramos** que se pueden commitear por separado y que dejan algo útil cada uno:

1. **Pipeline sin actualizador:** LICENSE y metadatos, `release.yml` (con la marca de compilación, `rustls`, binarios sueltos, `SHA256SUMS`, firma y atestación), y el bootstrap. Ya elimina la compilación local y es verificable a mano.
2. **`clawg update --check`:** lógica pura (versiones, asset, release, firma), sin tocar disco. Primera prueba real de la cadena de confianza con claves de prueba.
3. **`clawg update`:** el intercambio con el daemon, la vuelta atrás y las pruebas por el binario.

El orden es también el de menor riesgo: lo irreversible (reemplazar el binario y mover el daemon) llega al final, cuando todo lo demás ya está probado.

## Riesgos

- **El actualizador reemplaza el binario del que depende todo.** Es el peor lugar para un error. Mitigación: todo lo verificable pasa antes del intercambio; el intercambio es por renombre y reversible; el daemon nuevo se confirma por `/health` y, si no contesta, se vuelve al viejo.
- **Una versión nueva con migración deja el store inaccesible al binario anterior.** Es el diseño (la migración sólo avanza y deja el `pre-migration`), pero la vuelta atrás pierde lo escrito después. Documentarlo y mostrarlo en la salida de `update`. Y la poda de snapshots (backlog) hay que cerrarla antes de que las actualizaciones sean fáciles.
- **Una clave de firma filtrada o perdida.** Con la clave embebida, perderla obliga a un binario nuevo y a que la gente lo instale a mano. Diseñar la lista de claves aceptadas y escribir el procedimiento.
- **Un rollback con firma válida** (servir un release viejo): se cierra con «sólo hacia una versión estrictamente mayor» y con que `SHA256SUMS` nombre los archivos.
- **VC++ Redistributable ausente en una instalación limpia de Windows**: el binario no arranca y el error del sistema es críptico. Mitigación: el bootstrap lo comprueba. **Sin verificar** en una máquina limpia.
- **glibc/OpenSSL de Linux:** ver las opciones; no se promete otra cosa que lo que el runner fija, y se escribe en el README.
- **Falsos positivos de antivirus** por el perfil del binario, y reputación de SmartScreen que se reinicia con cada versión (sin firma). Mitigación posterior: SignPath Foundation. **Smart App Control** bloquea igual.
- **`workflow_run` con secretos:** sin la condición `event == 'push'` un PR externo podría hacer firmar código ajeno. Es el riesgo más serio del workflow.
- **Un release publicado sin todos sus archivos** hace fallar a todos los que corran `update --check`. Se evita publicando al final.
- **El puente viejo contra el daemon nuevo:** la regla «el daemon acepta lo que mandaba el anterior» no está escrita ni probada hoy; es una promesa nueva que hay que fijar con un test.
- **`ProcessSpawner` levanta `current_exe()`** (el binario viejo): quien reuse esa pieza para volver a arrancar el daemon después de actualizar arranca la versión anterior.
- **Carrera entre dos `update`** y entre `update` y un `serve` que arranca un editor.
- **API de GitHub:** 60 consultas por hora sin autenticar (por IP, compartida detrás de un NAT); falta de `User-Agent` da 403; un release en borrador o prerelease no aparece en `latest`.

## Decisiones abiertas

Son del usuario.

- [ ] **`authors`.** `Cargo.toml:7` nombra una persona. ¿Pasa a «The Claw Garden contributors» o se quita?
- [ ] **TLS.** ¿`rustls` con raíces del sistema en las dos plataformas (recomendado), `rustls` sólo en Linux, o dejar OpenSSL y declarar la dependencia?
- [ ] **Runner de Linux.** ¿`ubuntu-latest` o uno fijo más viejo? ¿Se escribe la glibc mínima en el README?
- [ ] **VC++ Redistributable.** ¿El bootstrap sólo avisa (recomendado), lo instala, o se distribuyen las DLL junto al `.exe`?
- [ ] **Formato de los assets.** ¿Binarios sueltos (recomendado, sin dependencias) o comprimidos?
- [ ] **Dónde vive el script de arranque.** ¿Adjunto al release (recomendado) o crudo desde `main`?
- [ ] **Perfil.** ¿`update` actúa sólo sobre `prod`, o acepta `--profile`? ¿Qué hace si el daemon del otro perfil corre?
- [ ] **Linux y macOS.** ¿`update` funciona en Linux sin instalador (reemplazando el binario donde esté), y qué hace con una ruta sin permiso? macOS no entra (CI no lo prueba; `ort-sys` sólo trae `aarch64`).
- [ ] **Clave de firma.** ¿Una sola o lista de claves aceptadas desde el primer día? ¿Un *environment* con revisión obligatoria para el secreto?
- [ ] **Release de `main` actual.** El primer disparo publica `v0.18.0` (no hay tags). ¿Se acepta, o se sube la versión antes?
- [ ] **División.** ¿Los tres tramos de arriba, y si es así, ¿cada uno es un cambio de openspec o uno solo con tareas?
- [ ] **Poda de snapshots.** ¿Se cierra antes de este cambio o junto con él?

## Cómo se mide

Tests RED primero; ninguno toca `~/.clawg*` ni la red real.

1. Versión: `0.18.0 < 0.19.0`, `0.18.0 = 0.18.0`, `0.18.0 > 0.17.9`; basura (`v0.19`, `0.19.0-rc1`, vacío) se rechaza.
2. Asset por triple y por marca de compilación; sin marca, `update` se niega con un mensaje que dice por qué.
3. Elección del release: salta borradores y prereleases; sin asset del triple propio, error que nombra el triple.
4. `SHA256SUMS`: una línea faltante, un hash corto, un nombre duplicado.
5. Firma: válida; con una clave que no es la embebida; con el archivo alterado; con un `.minisig` de otro archivo.
6. Hash que no coincide: no se toca el binario instalado.
7. Plan de pasos: con y sin daemon; cada falla deshace lo que corresponde.
8. Por el binario real, con un servidor de release falso y claves de prueba: actualización completa con un daemon corriendo (el daemon nuevo contesta, la versión cambia, el viejo quedó como `.old-*`), actualización con el nuevo binario que no arranca (se vuelve al viejo y el daemon vuelve), y `--check` con y sin versión nueva.
9. **Base sembrada en la versión anterior** (regla de `config.yaml`): store con filas de las tres tablas, actualización a un binario con una migración pendiente, el `pre-migration` aparece y los datos siguen alcanzables. Y la inversa: un store de un clawg más nuevo, el daemon se niega y `update` deja al usuario con el binario anterior y su daemon.
10. Puente abierto durante la actualización: sigue respondiendo tras el reinicio del daemon (es el escenario `a_bridge_left_open_reaches_the_daemon_after_a_restart`, `binary.rs:535`, con el binario reemplazado).
11. Convención: ningún manifiesto enciende `testing` fuera de `[dev-dependencies]` (ya lo exige `conventions.rs`); un test nuevo comprueba que la clave pública embebida no es la de pruebas.
12. El release: el artefacto compilado con `CLAWG_DIST_TARGET` responde `--version` con la versión del tag (humo en el workflow, no en la suite).

Sin cambio de ranking: `rag-eval --diff` no aplica.

## Alcance candidato

1. `LICENSE` (MIT) y metadatos de `Cargo.toml`.
2. `release.yml`: construcción, `SHA256SUMS`, firma, atestación y publicación al final.
3. La marca de compilación y `rustls`.
4. `clawg update [--check]` y sus módulos puros.
5. `scripts/install.ps1` convertido en bootstrap.
6. Docs: README, `.claude/CLAUDE.md`, specs `release` y `self-update`, y la modificación de `specs/install`.

### No-alcance

- Firma Authenticode (SignPath queda como camino posterior).
- macOS y Windows ARM.
- Un actualizador en segundo plano o que consulte solo.
- Cambios de esquema, de identificadores o del contrato de embedding.
- Un canal «beta» o prereleases.
- Instalador para Linux (se publica el binario; la instalación es del usuario).
