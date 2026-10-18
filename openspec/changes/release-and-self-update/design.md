# release-and-self-update — Diseño

Las decisiones de fondo están en la [propuesta](proposal.md#decisiones) y su contexto en la [exploración](exploration.md). Aquí van las de implementación. Las citas de código están verificadas en `9616854` (versión `0.19.0`).

## Flujo de `clawg update`

```
clawg update [--check]
  │ cerrojo <data>/update.lock ──── ocupado ──→ error, exit 1           (D4)
  │ elegibilidad (pura) ─────────── no ───────→ motivo, exit 1          (D6)
  │ GET releases → selección (pura) ──────── al día → exit 0            (D7)
  │                                         └ --check y hay nueva → exit 100
  │ borrar .old-* y .new-* de corridas anteriores                       (D5)
  ▼
  Stage → Smoke → [StopDaemon] → SetAside → PutInPlace → [StartDaemon + espera (D3)]
     └──── cualquier falla: se deshace en orden inverso lo hecho (D5);
           si el daemon nuevo migró el store, se restaura su pre-migration (D2)
```

`[ ]`: sólo si había daemon. Sin daemon no se arranca ninguno: uno parado a propósito sigue parado, la misma regla de `ClientHandle::reresolve` (`crates/clawg-cli/src/mcp/startup.rs:96-106`).

## D1. `minisign-verify`

**Elegido:** `minisign-verify = "=0.2.5"` en `[dependencies]` de clawg-cli. Verificado en crates.io y docs.rs: MIT, edición 2018, sin `rust-version`, ninguna dependencia (su `base64` y su `crypto`, con ed25519 y BLAKE2b, son módulos propios). La API que se usa:

- `Signature::decode(&str) -> Result<Signature, Error>` y `Signature::trusted_comment(&self) -> &str`.
- `PublicKey::decode(&str)` (las dos líneas de un `.pub`) y `PublicKey::from_base64(&str)`.
- `PublicKey::verify(&self, bin: &[u8], signature: &Signature, allow_legacy: bool) -> Result<(), Error>`: devuelve `UnexpectedKeyId` si la firma es de otra clave; verifica la firma sobre el BLAKE2b-512 del contenido si es *prehashed* (`ED`), o sobre el contenido si es legada (`Ed`, sólo con `allow_legacy`); después verifica la firma global sobre firma + comentario confiable. Errores: `InvalidEncoding`, `InvalidSignature`, `IoError`, `UnexpectedAlgorithm`, `UnexpectedKeyId`, `UnsupportedAlgorithm`, `UnsupportedLegacyMode`.

`update` llama con `allow_legacy = false`: minisign firma *prehashed* por defecto desde 0.10 y anunció que dejará de aceptar las legadas.

Para firmar en las pruebas, `minisign = "=0.9.1"` como **dev-dependency** (mismo autor, publicada el mismo día que 0.2.5): `KeyPair::generate_unencrypted_keypair()` y `minisign::sign(None, &sk, reader, Some(trusted), None)`. Que firme *prehashed* no está documentado en docs.rs: si firmara en modo legado, `allow_legacy = false` lo rechaza y el primer test en rojo (`signature_cases`, caso «firma prehashed del firmante de pruebas») lo muestra. Suma ct-codecs, scrypt y rpassword (con sus dependencias) a los builds de prueba; nunca al binario. Un juego fijo (`.pub`, archivo, `.minisig`) firmado con el CLI de minisign comprueba además que el verificador acepta lo que produce la herramienta del workflow.

| Alternativa | Por qué no |
|---|---|
| `=0.3.0` (2026-09-25) | Misma API pública (verificada en docs.rs); el único cambio de código desde 0.2.5 es quitar bloques `unsafe` internos. Tiene tres semanas y decide qué ejecuta el usuario: pasar a 0.3.x es una línea cuando tenga uso |
| Firmar en las pruebas con `ring` (ya en el árbol por rustls, D9) en modo legado | Obliga a `allow_legacy = true`, en producción o sólo en `testing`: la prueba dejaría de recorrer el modo que se usa |
| Sólo firmas preparadas de antemano | El `SHA256SUMS` de una prueba por el binario contiene el hash del binario de prueba, que cambia con cada build |

## D2. El daemon nuevo migró y no contesta

**Elegido:** restaurar en el acto el `pre-migration` que escribió ese arranque, con `clawg_core::memory::backup::restore` llamado desde el propio proceso de `update` (el binario viejo, `crates/clawg-core/src/memory/backup.rs:349-366`), y después devolver el binario viejo y arrancar su daemon.

- **Si migró** se decide, con el hijo ya terminado, por `backup::probe` (sin cerrojo y sin escribir, `backup.rs:459-462`) evaluado con las migraciones del binario viejo: `Newer` o `Diverged` es «el viejo no la abre» (`db.rs:388-397`).
- **Qué snapshot:** el `pre-migration` que aparece en `<data>/backups` entre la lista tomada antes de lanzar el hijo y la de después. Los nombres ordenan por tiempo (`backup.rs:178-193`); la diferencia es una función pura.
- `restore` toma el cerrojo, comprueba el snapshot y guarda antes el store migrado como `pre-restore` (`backup.rs:377-393`): nada se destruye, y lo migrado sólo lo abre la versión nueva.
- **Si el restore falla** (su contrato: no cambió nada), se deja el binario nuevo, el único que abre ese store, y el informe da la causa del arranque, la del restore, dónde está cada archivo y los comandos a mano. Es el único caso en que el usuario queda sin daemon, y sabe por qué.

| Alternativa | Por qué no |
|---|---|
| Dejar el binario nuevo y decir cómo volver | Su daemon no arranca: el usuario queda sin memoria hasta hacer tres pasos a mano |
| Volver sólo al binario | El viejo se niega con `SchemaNewer` (`error.rs:53-58`): sin daemon |
| Restaurar con el binario nuevo (`<nuevo> restore …`) | Es el binario que acaba de fallar, en un proceso más; el viejo ya está corriendo |

Hoy ningún test restaura sobre un store migrado por una versión más nueva (sólo `probing_a_newer_store_reads_without_writing`, `backup.rs:932`); se suma uno en `backup.rs`, `restoring_over_a_store_migrated_by_a_newer_version_keeps_it_as_pre_restore`. Quien migró y falló se prueba por el binario real con `a_new_binary_that_migrated_and_failed_gets_its_pre_migration_restored`, y el restore que falla, en el plan, con `a_failed_restore_leaves_the_new_binary_and_explains_the_way_back`.

## D3. «Arrancando» o «no arrancó»

`update` lanza el daemon nuevo y conserva su `Child`. Tres señales, ninguna adivinada:

1. **El hijo terminó** (`try_wait`): no arrancó. La causa es su código de salida y el final de `logs/serve.log`, donde va su stderr (`client.rs:985-1009`).
2. **`memory.owner` nombra el pid del hijo:** tiene el store y está haciendo el snapshot o migrando. `MemoryRuntime::open` toma el cerrojo, escribe el `pre-migration` y migra antes del `bind` (`serve.rs:86-88` y `:134-136`; `db.rs:178-218`). Se lee el registro **sin** probar el cerrojo: `StoreLock::current_owner` lo prueba con un try-lock (`store_lock.rs:179-192`), y repetirlo durante un arranque puede hacer que el daemon pierda el cerrojo y termine. Hace falta `StoreLock::recorded_owner`, público (hoy `read_owner` es privado, `store_lock.rs:109-112`). El registro de un proceso caído tiene otro pid.
3. **`/health` contesta con la versión nueva:** arrancó. Lo de después del `bind` (cargar el modelo, el re-embed CLS→mean con `embedder: loading`, `serve.rs:90-125`) no se espera; se informa.

**Plazos:** 60 s (el de `AUTOSTART_TIMEOUT`, `client.rs:35-41`) hasta que el hijo tenga el store; mientras lo tenga, hasta 10 min. Valores iniciales, no medidos. Vencido uno, `update` termina el hijo por su handle (antes del `bind` sólo hay una migración en una transacción, `db.rs:376-384`, y un snapshot escrito bajo nombre temporal, `backup.rs:288-319`) y sigue con D2 y D5.

**Lo que ve el usuario:** `starting the new daemon (pid N)`; pasados 5 s con el store tomado, una línea cada 10 s, `the new daemon is preparing the store (pre-migration snapshot <ruta>), 40 s`; al final, versión, pid y estado del embedder.

La decisión es pura: `judge(observación, transcurrido) -> Wait | Ready | Exited | TimedOut | Other { pid, version }`.

| Alternativa | Por qué no |
|---|---|
| Un solo plazo de 60 s | Un `VACUUM INTO` o una migración de un store grande lo pasan, y se desharía una actualización sana |
| Sin tope mientras el hijo viva | Un hijo colgado deja colgado a `update` y al usuario sin daemon |
| `start_daemon` (`client.rs:1113-1143`) | Sólo sondea `/health`: no sabe si el proceso murió ni si tiene el store |

## D4. Concurrencia

**Dos `update`:** cerrojo exclusivo del sistema (`fs2`, como `StoreLock`) sobre `<data>/update.lock` (`paths::update_lock_file`), durante toda la corrida; el segundo falla en el acto: `another clawg update is running for this profile`. Va en el directorio de datos porque lo que dos `update` se disputan es el daemon del perfil, también desde dos copias del binario en rutas distintas, y ese directorio siempre es escribible.

| Alternativa | Por qué no |
|---|---|
| Cerrojo junto al binario | No frena dos copias instaladas parando y arrancando el mismo daemon; en `/usr/local/bin` puede no ser escribible |
| Sin cerrojo, error del usuario | Dos intercambios cruzados pueden dejar un `.old-*` como binario y ninguno en su lugar |

**Un editor abre un puente entre la parada y el arranque.** Un puente abierto nunca arranca un daemon (`startup.rs:96-106`); uno nuevo sí, al empezar, con su `current_exe()` (`client.rs:982`): el binario viejo si se lanzó antes del intercambio, el nuevo si después. Mientras el viejo vacía su cola conserva el store, así que el daemon de ese puente pierde el cerrojo y termina; sólo queda la ventana de milisegundos entre la salida del viejo y el arranque del nuevo. Regla: cuenta la versión de quien contesta.

- Contesta la versión nueva, la lanzara `update` u otro: listo, y el informe nombra el pid.
- Contesta la versión vieja con otro pid (el hijo salió con `StoreInUse`): `update` lo para con `stop_in` y lanza el suyo otra vez, una sola vez. Si vuelve a pasar, sale con 1: el binario ya es el nuevo, sirve un daemon viejo, y el mensaje dice que un editor abierto antes de actualizar lo arrancó y que `clawg stop` lo resuelve.

## D5. El plan como datos

```rust
pub struct Initial { pub daemon: Option<Running> }       // Running { pid, port, version }
pub enum Step { Stage, Smoke, StopDaemon, SetAside, PutInPlace, StartDaemon }
pub enum Undo { RemoveStaged, StartOldDaemon, RestoreAside, TakeBackNew, EndNewDaemon }
pub fn plan(initial: &Initial) -> Vec<(Step, Option<Undo>)>;
pub fn apply(plan: &[(Step, Option<Undo>)], fx: &mut dyn Effects) -> Outcome;
```

| Paso | Qué hace | Deshacer |
|---|---|---|
| `Stage` | crea el preparado junto al binario (es la prueba de permiso de escritura, antes de bajar nada), baja `SHA256SUMS` y su firma, verifica (D7), baja el binario calculando su sha256 | borrar el preparado |
| `Smoke` | `<preparado> --version` igual a `clawg <versión>`; en Windows hasta 3 intentos a 1 s si el proceso no se crea (un antivirus leyendo el archivo recién escrito) | — |
| `StopDaemon` | `stop_in(base, STOP_TIMEOUT)` con el binario viejo (`stop.rs:19-43`); si vence, no se renombra nada (`install.ps1:174-183`) | arrancar `<exe>` en el puerto anterior y confirmar por pid |
| `SetAside` | `<exe>` → `<exe>.old-<marca>` | `.old-*` → `<exe>` |
| `PutInPlace` | preparado → `<exe>` | `<exe>` → nombre del preparado |
| `StartDaemon` | lanza `<exe> serve --profile <p> --port <el que tenía>` y espera (D3) | terminar el hijo si vive; si el store ya no abre con este binario, D2 |

- `plan` es pura: con daemon, los seis; sin daemon, `Stage`, `Smoke`, `SetAside`, `PutInPlace`.
- `apply` corre en orden y apila el deshacer de cada paso hecho; ante una falla los corre al revés. El primero que falla corta la cadena, porque los siguientes lo suponen, y el informe dice dónde quedó cada archivo.
- `Effects` es la frontera con disco, red y procesos: un rasgo con una sola implementación de producción, por la razón de `DaemonSpawner` (`client.rs:961-969`).
- **Nombres:** `<exe>.old-<yyyyMMddHHmmssfff>`, el formato de `install.ps1:198`, para que cada herramienta limpie lo de la otra; el preparado es `clawg.new-<marca>.exe` (o `clawg.new-<marca>`) porque se ejecuta. Todo en el directorio del binario: un renombre dentro de un volumen.
- **Windows** no deja sobrescribir un ejecutable mapeado pero sí renombrarlo (`install.ps1:192-205`): el binario viejo se aparta a sí mismo. Linux sigue los mismos pasos (renombrar un binario en uso también vale) y así tiene el mismo `.old-*` para volver.
- **Limpieza al empezar:** `leftovers(nombres, exe)` pura elige `.old-*` y `.new-*`; se borran sin insistir y los mapeados (puentes viejos que siguen abiertos) quedan para otra corrida, como en `install.ps1:187-190`.
- **El puerto** se lee de `daemon.json` antes de parar y se repite al arrancar, el nuevo o el viejo: quien marcó un `--port` lo conserva y las pruebas siguen en puertos efímeros.
- El arranque por ruta reusa el cuerpo de `ProcessSpawner::spawn_serve` (log en append, `DETACHED_PROCESS`, `disinherit_std_handles`, `client.rs:978-1040`) extraído a una función que recibe la ruta y el puerto y devuelve el `Child`; `ProcessSpawner` la llama con `current_exe()`.

## D6. Elegibilidad

```rust
pub fn eligibility(debug: bool, dist_target: Option<&str>, in_cargo_target: bool,
                   profile: Profile, default_profile: Profile) -> Result<Triple, Refusal>;
```

En este orden, cada negativa con su motivo: build debug; sin `CLAWG_DIST_TARGET` (`option_env!`; el mensaje dice que es un build local y que se actualiza con git y cargo o instalando un release); un triple sin release publicado; corre desde un directorio de target de cargo; un perfil que no es el de la build. El perfil que maneja es el que la build usa por defecto, `Profile::compiled_default()` (`paths.rs:60-66`): `prod` en toda build que pasa las dos primeras, y `dev` sólo bajo el sello de prueba (D11), así las pruebas quedan en `<home>/.clawg-dev` y la regla de que un debug nunca abre `prod` sigue intacta (`paths.rs:86-92`).

«Dentro de un target de cargo» es un `.cargo-lock` junto al ejecutable: cargo lo escribe en cada directorio de perfil (presente en `target/debug` y `target-test/debug`). `CACHEDIR.TAG` no sirve: falta en `target/` de este repositorio. La función es pura; la lectura del disco, el triple y `debug` llegan como argumentos.

| Alternativa | Por qué no |
|---|---|
| Buscar `target/` en la ruta | `--target-dir` acepta cualquier nombre |
| Deducir el triple en runtime | Un binario `musl` pediría el asset `gnu` ([exploración](exploration.md#detectar-esto-es-un-binario-de-release-instalado-pregunta-6)) |

## D7. Versión, release, `SHA256SUMS`, firma y salida

- **Versión:** `X.Y.Z`, tres enteros decimales sin ceros a la izquierda (salvo `0`), sin prefijo ni sufijo; `Ord` derivado; sólo se acepta una estrictamente mayor. El tag es `v` + esa forma.
- **Endpoint:** `GET https://api.github.com/repos/MarcossIC/claw-garden/releases?per_page=30` (los más nuevos primero), con `User-Agent: clawg/<versión>` y `Accept: application/vnd.github+json`. Se leen `tag_name`, `draft`, `prerelease` y los nombres de `assets`; la selección pura salta borradores, prereleases y tags que no son `v<X.Y.Z>`, y elige la versión mayor. Las URL de descarga se arman (`…/releases/download/v<X.Y.Z>/<asset>`), no se toman del JSON. `releases/latest` devuelve un solo objeto, el más reciente por fecha del commit que no es borrador ni prerelease: no deja nada que saltar, y lo decide la marca «Latest» y no la versión.
- **Asset:** `clawg-<triple>`, más `.exe` en `*-windows-msvc`. Sin el propio, el error nombra el triple.
- **`SHA256SUMS`:** lo que escribe `sha256sum`, líneas `<64 hex minúsculas><espacio><espacio o *><nombre>`. Errores: línea mal formada (con su número), nombre repetido, falta el asset propio.
- **Firma:** cada clave de la lista; `UnexpectedKeyId` pasa a la siguiente y cualquier otro error rechaza. El comentario confiable tiene que ser `clawg v<X.Y.Z> SHA256SUMS` (el workflow firma con `-t`): ata los hashes firmados a la versión que anunció la API.
- **Claves:** dos claves públicas desde el primer release, ambas commiteadas y embebidas con `include_str!`: la primaria, `release/minisign.pub` (su mitad privada está en el environment `release`), y la de respaldo, `release/minisign-next.pub` (su mitad privada se guarda fuera de GitHub, nunca en el repositorio ni en la CI). Si se pierde la primaria, los releases siguen firmados con el respaldo y los binarios instalados los aceptan; con una sola clave, rotar exigiría reinstalar a mano. No protege de una filtración a los binarios que no se actualicen: siguen aceptando la clave filtrada hasta instalar un release cuya lista ya no la tenga. El README lo dice (R10).
- **Orden y topes:** primero `SHA256SUMS` (64 KiB) y `.minisig` (4 KiB), se verifican, y sólo entonces el binario (256 MiB) al preparado; el JSON, 1 MiB.
- **Salida:** `--check`: `0` al día, `100` hay una nueva (la nombra en stdout), `1` error o negativa. `update`: `0` actualizado o al día, `1` negativa o falla. El `2` es de clap (`main.rs:7-8`); el `100` es el de `dnf check-update`.

## D8. `release.yml`

```
on: workflow_run (workflows [CI], types [completed], branches [main])
permissions: contents: read        concurrency: { group: release, cancel-in-progress: false }
plan     ubuntu-22.04  si conclusion == success && event == push && head_branch == main
                        && head_repository.full_name == github.repository
                       checkout head_sha; versión de clawg-cli por `cargo metadata --no-deps`;
                       sale limpio si el tag existe o si no supera al último release
build    matriz windows-2022 / ubuntu-22.04, env CLAWG_DIST_TARGET=<triple>
                       cargo build --release --locked -p clawg-cli --target <triple>
                       humo: --version == clawg X.Y.Z; sin la cadena CLAWG_TEST_ en el binario;
                       Linux: ldd sin libssl ni libcrypto, y la glibc más alta que pide
publish  environment release; contents: write, id-token: write, attestations: write
                       SHA256SUMS de los dos binarios y install.ps1 (de head_sha);
                       minisign 0.12 (tarball fijado por sha256), contraseña por stdin:
                         printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S -s <clave> -t "clawg vX.Y.Z SHA256SUMS" -m SHA256SUMS;
                       minisign -Vm contra release/minisign.pub; attest-build-provenance;
                       gh release create vX.Y.Z --target <head_sha> con los cinco archivos
```

- **Condiciones:** `event == 'push'` es la que frena el código de un fork; `head_repository` se agrega porque no cuesta nada y cubre lo mismo por otro campo. Se construye y etiqueta `head_sha`, nunca la punta de `main`.
- **`paths-ignore` no puede frenar el bump:** una ejecución se omite sólo si **todos** los archivos del push están en la lista (`ci.yml:8-16`), y el bump toca `Cargo.toml` y `Cargo.lock`. Sí lo frenan un `[skip ci]` en el mensaje y la cancelación por un push posterior (`ci.yml:31-33`); en ese caso publica la ejecución siguiente, en su propio `head_sha`, porque la versión sigue sin tag. Un push sólo de docs no crea ejecución ni cancela nada.
- **Publicar al final:** `gh release create` crea un borrador, sube los archivos y sólo después lo publica (manual de gh), así que `releases/latest` nunca ve uno incompleto.
- **La clave privada va con contraseña** y las dos son secretos del environment `release`. El paso de firma escribe la clave en un archivo temporal del runner (permisos `0600`, borrado al terminar) y pasa la contraseña por stdin: `get_password` de minisign (`src/get_line.c` en jedisct1/minisign) lee con `fgets(stdin)`, sólo apaga el eco si `isatty(0)` y nunca abre `/dev/tty`, así que una tubería alcanza. Los dos secretos llegan como `env` de ese único paso (no del job) y GitHub enmascara sus valores en el registro; el paso no activa `set -x` ni hace `echo` de ellos, y la contraseña nunca va como argumento, que quedaría visible en la lista de procesos. Alternativa descartada: `minisign -W`, sin contraseña, un secreto menos, pero cualquier copia de la clave privada sola firma; con contraseña, la copia que se guarde fuera de GitHub sigue cifrada. Dentro de GitHub clave y contraseña viven en el mismo environment, así que ahí la contraseña agrega poco.
- **Sin caché de compilación:** un release se construye limpio; con `--locked`, sobre el `Cargo.lock` probado.
- **Acciones fijadas por SHA** (checkout, rust-toolchain, upload/download-artifact, attest-build-provenance), con el tag en un comentario. Toolchain `stable`, como CI; `rustc -Vv` queda en el log del job.
- **Los tests que fijan los workflows leen texto, no YAML:** recorren `ci.yml` y `release.yml` por líneas y expresiones regulares en `crates/clawg-cli/tests/conventions.rs`, como `testing_outside_dev_dependencies` lee los manifiestos (`conventions.rs:87-102`). Alternativa descartada: una dependencia de desarrollo que parsee YAML; hoy no hay ninguna en `Cargo.lock`, y los dos archivos son del repositorio y cortos, así que el árbol de datos no aporta lo que cuesta una dependencia sin otro uso. El costo es que reformatear un workflow sin cambiar su sentido puede romper un test, que se ajusta en el mismo commit.

## D9. `reqwest` con `rustls`

**Elegido:** en la raíz, `reqwest = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls-native-roots", "charset", "http2", "system-proxy"] }` (`Cargo.toml:19`). Verificado contra `reqwest-0.12.28/Cargo.toml`, la versión del lock (`Cargo.lock:2107-2110`): `default = ["default-tls", "charset", "http2", "system-proxy"]`, y `rustls-tls-native-roots` activa `rustls-native-certs` y `__rustls-ring`, el proveedor `ring` (ya en el lock, `Cargo.lock:2150`), no `aws-lc-rs`.

Unificación de features: sólo clawg-core y clawg-cli dependen de `reqwest`, las dos con `workspace = true` y sin features propias (`crates/clawg-core/Cargo.toml:32`, `crates/clawg-cli/Cargo.toml:33`), así que nada vuelve a encender `default-tls`. `native-tls` y OpenSSL quedan en el árbol sólo por `ureq`, *build-dependency* de `ort-sys` (`ort-sys-2.0.0-rc.12/Cargo.toml:138-142`), que entra en el script de build y no en el binario. El `ureq` de `ort` es opcional y sólo lo enciende `fetch-models` (`ort-2.0.0-rc.12/Cargo.toml:121-125`, `:243-246`), que el workspace no pide (`Cargo.toml:43`). Que `hyper-rustls` figure hoy bajo `reqwest` en el lock es efecto de la referencia débil `hyper-rustls?/http2`, no algo que se compile. El cambio sí suma `rustls-native-certs`, que hoy no está en el lock, y vuelve dependencias del binario a rustls, ring, hyper-rustls y tokio-rustls. El `ldd` del workflow lo comprueba sobre el artefacto. Por eso `minisign-verify` (D1) es la única dependencia directa nueva de runtime, no la única que entra al binario; las pruebas suman además la dev-dependency `minisign` con ct-codecs, scrypt y rpassword.

## D10. Dónde vive el código

- **clawg-cli, `src/update/`:** `version.rs`, `release.rs` (asset, selección, `SHA256SUMS`, comentario confiable), `signature.rs`, `eligibility.rs`, `plan.rs` (plan, `judge`, `leftovers`, `apply` sobre `Effects`), `report.rs`: puros. `effects.rs`: la implementación de producción (reqwest, renombres, `stop_in`, arranque, `probe`, `restore`). `mod.rs`: el cableado. Más `Command::Update { check, profile }` (`cli.rs`, con `ProfileArg` como todo subcomando, `cli.rs:402-406`) y su brazo en `main.rs`.
- **clawg-core:** sólo lo que ya es suyo o lo que exige `testing`. `paths::update_lock_file`; `StoreLock::recorded_owner` (D3); `build_info.rs`, nuevo, con `version()`, `dist_target()`, `serve_fault()` y los reemplazos de prueba de la URL y de las claves, cada uno con dos cuerpos según `testing`, como `crates/clawg-core/src/memory/bootstrap.rs:353-367`. Tiene que estar ahí: clawg-cli no puede tener una feature que encienda `clawg-core/testing`, y `tests/conventions.rs:96-97` la rechaza (su propio caso, `:113-118`).
- La versión que reporta el binario pasa a salir de `build_info::version()` en los sitios que hoy usan `env!("CARGO_PKG_VERSION")`: `cli.rs:15` (clap acepta `version = <expr>`), `api.rs:797`, `serve.rs:152`, `mcp/server.rs:1230`, `doctor.rs:121`, `memories.rs:398` y `mcp/startup.rs:113`. Sin sello es lo mismo que hoy.
- Nada de esto entra en clawg-core por dominio: el store no sabe de releases.

## D11. Pruebas por el binario real

**Elegido:** un **sello de prueba** al final de una copia del binario de prueba, que sólo lee un build `testing`: un byte `0x00`, la marca `CLAWG-TEST-STAMP` y un JSON que llega hasta el final del archivo, buscados en los últimos 4 KiB. Windows y Linux ignoran los bytes que siguen a la imagen de un ejecutable; el primer test RED, `trailing_bytes_after_the_executable_image_do_not_change_execution`, lo comprueba en las dos plataformas de CI. Campos: `version`, `dist_target`, `migrations_up_to` (hasta qué migración conoce: filtra `known_migrations`, `backup.rs:42-52`, y le pone `set_target` al runner de `run_migrations`, `db.rs:400-403`, como ya hace `seed_on_disk`, `db.rs:434-445`) y `serve_fault` (`before_open` | `after_open`). Viaja con el archivo en cada renombre, que es lo que no puede hacer una variable de entorno: el `update`, el daemon nuevo y el viejo que vuelve heredan el mismo entorno. Por entorno, sólo lo que es de la corrida: `CLAWG_TEST_RELEASE_URL` y `CLAWG_TEST_RELEASE_KEYS`.

El `CARGO_BIN_EXE_clawg` (`binary.rs:23`) no lleva sello, así que `update` se niega sobre él: ninguna prueba reemplaza el ejecutable de `target-test`. Cada escenario copia el binario a `<home>/bin` con su sello y sirve el «nuevo» desde un axum en `127.0.0.1:0` (axum ya es dependencia del crate), firmado en el momento con una clave de `minisign` (D1).

| Escenario | Sellos (instalado → servido) | Se comprueba |
|---|---|---|
| `--check` | 0.18.0 → igual, 0.19.0, servidor caído | salidas `0`, `100` con la versión, `1` |
| Actualización completa | 0.18.0 → 0.19.0 | daemon y puente lanzados desde la copia; `/health` dice 0.19.0; el `.old-*` es el binario original; el puente abierto llega al daemon nuevo (el escenario de `binary.rs:534-549`) |
| No arranca | → 0.19.0 `before_open` | salida `1`; el binario en su lugar es el original; un daemon 0.18.0 contesta |
| Migración pendiente | 0.18.0 `migrations_up_to = última - 1` → 0.19.0 | filas guardadas antes; el informe nombra el `pre-migration`; las filas siguen en el daemon nuevo |
| Migró y falló | ídem → 0.19.0 `after_open` | salida `1`; el daemon 0.18.0 sirve las filas; existe el `pre-restore` |
| Candidato que se niega ante el store | 0.18.0 (todas) → 0.19.0 `migrations_up_to = última - 1` | el daemon nuevo se niega sin tocar el store (`SchemaNewer`, `db.rs:388-398`); ningún snapshot nuevo; vuelven el viejo y su daemon (`an_update_whose_candidate_refuses_the_store_leaves_the_old_binary_and_its_daemon`) |
| Sin sello | el binario de `target-test` | se niega, «compilado en local»; nada cambia |

Con releases ordenados por versión, un store más nuevo que el candidato lo es también del binario instalado, y entonces no hay daemon viejo que actualizar: sin daemon `update` no arranca ninguno. El caso real que queda es el de un candidato cuyo daemon se niega a abrir el store (esquema más nuevo o migración divergente para él), y sólo se reproduce con un candidato que conoce menos migraciones que el binario instalado. El sello lo reproduce con `migrations_up_to`.

El daemon que lanza `update` no es hijo de la prueba: un guardián lo para con `clawg stop` en su `Drop` y espera su pid, y nunca termina un proceso por nombre (`binary.rs:10-12`).

| Alternativa | Por qué no |
|---|---|
| `CLAWG_TEST_VERSION` por entorno | Lo heredan el `--version` del preparado y los dos daemons: el viejo y el nuevo dirían lo mismo |
| Un binario falso aparte (un `example`) | No sirve un daemon real ni migra; su ruta en `target-test` no la publica cargo |
| Sólo el `apply` con `Effects` falsos | Ya va en las unitarias; no prueba renombres reales, `/health`, `memory.owner` ni el `restore` real |

El sello es la única costura de prueba que toca código de migraciones, y es un agregado a lo que fija la propuesta; sólo un build `testing` lo lee (`update_overrides_are_read_only_in_a_test_build`).

## Archivos

| Archivo | Acción | Qué |
|---|---|---|
| `Cargo.toml`, `Cargo.lock` | modificar | `license = "MIT"`, `authors`, `repository` (`:7-9`); `reqwest` (D9) |
| `LICENSE`, `release/minisign.pub`, `release/minisign-next.pub` | crear | las dos claves públicas (D7) |
| `.github/workflows/release.yml` | crear | D8 |
| `crates/clawg-cli/Cargo.toml` | modificar | `minisign-verify` (la única dependencia directa nueva de runtime); `fs2` y `sha2` del workspace (ya en el lock); dev: `minisign` |
| `crates/clawg-cli/src/update/*.rs`, `src/update/testdata/` | crear | D5 a D7 y el juego firmado por el CLI |
| `crates/clawg-cli/src/{lib,cli,main,client}.rs` | modificar | módulo, subcomando, salida, arranque por ruta |
| `crates/clawg-cli/src/{daemon/serve,daemon/api,mcp/server,mcp/startup,doctor,memories}.rs` | modificar | `build_info::version()`; `serve_fault` en `serve.rs` |
| `crates/clawg-core/src/build_info.rs`, `src/lib.rs` | crear / modificar | D10, D11 |
| `crates/clawg-core/src/{paths,memory/store_lock,memory/backup,memory/db}.rs` | modificar | `update_lock_file`, `recorded_owner`, tope de migraciones bajo `testing` |
| `crates/clawg-cli/tests/{binary,conventions}.rs` | modificar | D11; la lista embebida tiene las dos claves de `release/` y no la de `testdata`; el paso de firma de `release.yml` lee la contraseña por stdin |
| `scripts/install.ps1`, `README.md`, `.claude/CLAUDE.md` | modificar | según la propuesta |

## Pruebas unitarias (RED primero)

- `version.rs`: orden, igualdad; `v0.19`, `0.19.0-rc1`, `01.2.3` y vacío rechazados.
- `release.rs`: asset por triple; un borrador, un prerelease o un tag mal formado más nuevos se saltan y gana el último publicado; falta el asset propio y el error nombra el triple; `SHA256SUMS` con línea faltante, hash corto, nombre repetido; comentario confiable de otra versión.
- `signature.rs`: válida; con la segunda clave; clave fuera de la lista; archivo alterado; `.minisig` de otro archivo; legada rechazada; firma *prehashed* del `minisign` de pruebas aceptada con `allow_legacy = false`; el juego del CLI aceptado.
- `binary.rs`: los bytes finales de un ejecutable no cambian su ejecución (`trailing_bytes_after_the_executable_image_do_not_change_execution`, en PE y en ELF).
- `eligibility.rs`: cada negativa con su motivo, y la combinación que pasa.
- `plan.rs`: plan con y sin daemon; con `Effects` falsos, una falla en cada paso deshace exactamente lo hecho; un hash que no coincide no toca el instalado; un `StopTimeout` no renombra nada; un restore que falla deja el binario nuevo; `judge` en cada señal y en cada plazo; `leftovers` y el diff de snapshots.
- `backup.rs`: restaurar sobre un store migrado por una versión más nueva guarda ese store como `pre-restore`.

## Vuelta atrás

Revertir este cambio no toca datos: sin `release.yml` no se publica más, los binarios instalados siguen andando y `update` dice «al día». Volver a OpenSSL es devolverle a `reqwest` sus features por defecto. La vuelta atrás de una actualización es la de la propuesta: automática en D2, a mano con `clawg stop`, el `.old-*` y `clawg restore` del `pre-migration`.

## Preguntas abiertas

- [ ] Si `update` debe avisar cuando la copia que actualiza no es la que resuelve el PATH (`doctor::analyze`); hoy queda fuera.
