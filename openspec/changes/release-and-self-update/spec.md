# release-and-self-update — Especificación

Al archivar, este archivo se reparte en tres dominios:

| Dominio destino | Tipo | Requisitos |
|---|---|---|
| `specs/release` | nuevo | R1 a R10 (**ADDED**) |
| `specs/self-update` | nuevo | S1 a S12 (**ADDED**) |
| [`specs/install`](../../specs/install/spec.md) | existente | I1 (**MODIFIED**: reemplaza por completo la sección «CI») |

No existe `specs/update`: esa es la de `update_memory` y no se toca. Nada de lo que no se nombra aquí cambia, y no hay requisitos **REMOVED**.

## Definiciones

- **Release:** un release de GitHub del repositorio, no borrador ni prerelease, con un tag `vX.Y.Z`.
- **Binario instalado:** el `clawg` que corre `clawg update`.
- **Marca de distribución:** la constante de compilación `CLAWG_DIST_TARGET=<triple>`; sólo la ponen los builds de `release.yml`. Un `cargo build --release` local no la tiene.
- **Lista de claves:** las claves públicas minisign que el binario acepta, embebidas en él.
- **Daemon:** el proceso `clawg serve` del perfil `prod`.
- **Candidato:** el binario nuevo bajado y preparado junto al destino, todavía sin instalar.
- **Intercambio:** el paso que renombra el binario instalado a `.old-*` y pone el candidato en su lugar.

Estado actual que estos requisitos cambian (verificado contra `e13dca7`):

- `license` y `repository` están vacíos y `authors` nombra a una persona (`Cargo.toml:7-9`); `reqwest` usa sus features por defecto (`Cargo.toml:19`); el único workflow es `ci.yml`.
- El binario sólo distingue debug de release (`Profile::compiled_default`, `crates/clawg-core/src/paths.rs:60-66`) y compara versiones por igualdad, sólo en `clawg memories` (`version_warning`, `crates/clawg-cli/src/memories.rs:332-338`).
- `install.ps1` para el daemon, renombra el binario a `.old-*` y no vuelve a levantar nada (`scripts/install.ps1:174-211`); un puente abierto nunca arranca un daemon (`ClientHandle::reresolve`, `crates/clawg-cli/src/mcp/startup.rs:96-102`) y el autoarranque lanza `current_exe()` (`ProcessSpawner::spawn_serve`, `crates/clawg-cli/src/client.rs:979-982`).

---

# Dominio `release`

## R1. Metadatos y licencia

El repositorio MUST llevar un `LICENSE` MIT cuyo titular es «The Claw Garden contributors». `[workspace.package]` MUST fijar `license = "MIT"`, `repository = "https://github.com/MarcossIC/claw-garden"` y `authors = ["The Claw Garden contributors"]`. Nada commiteado MUST nombrar a una persona.

#### Escenario: metadatos completos
- GIVEN el `Cargo.toml` raíz
- WHEN el test lee `[workspace.package]`
- THEN `license` es `MIT`, `repository` no está vacío y ningún `authors` es un nombre de persona
- AND existe `LICENSE` con el titular

**Test (falla hoy):** `the_manifest_carries_the_publication_metadata` en `crates/clawg-cli/tests/conventions.rs`.

## R2. TLS sin libssl

`reqwest` MUST declararse en la raíz del workspace con `default-features = false` y las features `rustls-tls-native-roots`, `charset`, `http2` y `system-proxy`, más las que el código use hoy (`json`, `stream`). MUST NOT quedar `default-tls` ni `native-tls`. La confianza MUST seguir siendo la de las CA del sistema.

#### Escenario: manifiesto
- GIVEN `reqwest` en el `Cargo.toml` raíz
- WHEN el test lo lee
- THEN tiene `default-features = false`, `rustls-tls-native-roots` y ni `default-tls` ni `native-tls`

#### Escenario: binario de Linux
- GIVEN el binario de Linux construido por el workflow
- WHEN el paso de humo corre `ldd` sobre él
- THEN la salida no contiene `libssl`

**Pruebas:** `reqwest_uses_rustls_with_the_system_roots` (convención, `conventions.rs`); el paso `ldd` de `release.yml`, fuera de la suite.

## R3. Sólo `clawg-cli`, con marca

`release.yml` MUST compilar con `cargo build --release -p clawg-cli` y MUST NOT usar `--workspace`: `clawg-eval` enciende la feature `eval` de clawg-core (`crates/clawg-eval/Cargo.toml:34`), que los builds de producción no enlazan (`crates/clawg-core/Cargo.toml:16-19`), y la unificación de features la llevaría al binario. Cada build MUST poner `CLAWG_DIST_TARGET` con su triple. Linux MUST compilarse en `ubuntu-22.04` fijo.

#### Escenario: comando de build
- GIVEN `.github/workflows/release.yml`
- WHEN el test lo lee
- THEN todo `cargo build` lleva `-p clawg-cli`, ninguno lleva `--workspace`, y cada build define `CLAWG_DIST_TARGET`
- AND el job de Linux corre en `ubuntu-22.04`

**Test (falla hoy, el archivo no existe):** `the_release_workflow_builds_only_the_cli_with_the_dist_mark` (convención, `conventions.rs`).

## R4. Disparador y etiqueta

`release.yml` MUST dispararse por `workflow_run` de `CI` y publicar sólo si `event == 'push'`, `conclusion == 'success'` y `head_branch == 'main'`, y la versión de `[workspace.package]` leída en el `head_sha` de esa ejecución no tiene tag `vX.Y.Z`. MUST etiquetar ese `head_sha`. La ejecución de CI de un pull request, aun desde un fork con una rama llamada `main`, MUST NOT publicar. Si el tag ya existe, el workflow MUST terminar sin publicar ni fallar.

#### Escenario: bump en main
- GIVEN una CI exitosa por push a `main` cuyo `Cargo.toml` dice `0.19.0` y sin tag `v0.19.0`
- WHEN corre `release.yml`
- THEN publica `v0.19.0` sobre ese `head_sha`

#### Escenario: pull request de un fork
- GIVEN una CI exitosa con `event == 'pull_request'` y `head_branch == 'main'`
- WHEN corre `release.yml`
- THEN no firma ni publica nada

#### Escenario: versión ya publicada
- GIVEN una CI exitosa por push a `main` y la versión ya tiene tag
- WHEN corre `release.yml`
- THEN termina sin publicar y sin error

**Pruebas:** `the_release_workflow_publishes_only_for_a_successful_push_to_main` (convención sobre las condiciones del job); las ramas de tag existente y de fork, en el workflow, fuera de la suite.

## R5. Assets y publicación atómica

Cada release MUST llevar exactamente `clawg-x86_64-pc-windows-msvc.exe`, `clawg-x86_64-unknown-linux-gnu`, `install.ps1`, `SHA256SUMS` (de los tres anteriores) y `SHA256SUMS.minisig`, sin comprimir, y la atestación de procedencia de GitHub. El release MUST crearse cuando todos los archivos existen: `releases/latest` MUST NOT ver nunca un release a medias. Si un paso falla, MUST NOT quedar ni tag ni release.

#### Escenario: release completo
- GIVEN un build exitoso en las dos plataformas
- WHEN el workflow publica
- THEN el release aparece con los cinco archivos y su atestación

#### Escenario: una plataforma falla
- GIVEN que el build de Windows falla
- WHEN corre el workflow
- THEN no existe tag ni release, y la próxima CI exitosa en `main` reintenta

**Prueba:** el paso de publicación y la lista de archivos, en el workflow, fuera de la suite; `the_release_workflow_publishes_last` (convención: el paso que crea el release depende de todos los jobs de build).

## R6. Firma y claves

`SHA256SUMS` MUST firmarse con minisign usando una clave privada y su contraseña guardadas como secretos del environment `release`, limitado a `main`, sin revisores obligatorios. La clave pública correspondiente MUST estar commiteada y MUST estar en la lista de claves. El workflow MUST verificar su propia firma (`minisign -Vm SHA256SUMS`) con la clave del repositorio antes de publicar. La lista de claves MUST admitir más de una clave desde el primer día y MUST NOT contener la clave de prueba.

#### Escenario: firma verificada antes de publicar
- GIVEN `SHA256SUMS` y `SHA256SUMS.minisig` recién generados
- WHEN el workflow verifica con la clave del repositorio
- THEN pasa, y sólo entonces se crea el release

#### Escenario: clave de prueba fuera de la lista
- GIVEN la lista de claves embebida
- WHEN el test la compara con la clave de prueba de la suite
- THEN no la contiene, y contiene la clave pública del repositorio

**Pruebas:** `the_embedded_key_list_has_the_repository_key_and_not_the_test_key` (convención, medida 9 de la propuesta); la verificación con `minisign`, en el workflow, fuera de la suite.

## R7. Verificación en el workflow

Antes de publicar, el workflow MUST ejecutar el binario de cada plataforma y comprobar que `--version` responde la versión del tag. Las acciones de terceros MUST fijarse por SHA de commit. La clave privada MUST NOT estar disponible para jobs que no firman.

#### Escenario: versión del tag
- GIVEN el binario construido para el tag `v0.19.0`
- WHEN el paso de humo corre `--version`
- THEN la salida contiene `0.19.0`; si no, el workflow falla antes de publicar

**Pruebas:** `third_party_actions_are_pinned_to_a_commit` (convención sobre `release.yml`); el humo `--version`, en el workflow, fuera de la suite.

## R8. Bootstrap: `install.ps1` desde el release

`install.ps1` MUST adjuntarse a cada release y ser alcanzable en `https://github.com/MarcossIC/claw-garden/releases/latest/download/install.ps1`. Sin un `clawg.exe` local al lado, MUST bajar el binario del último release, compararlo con `SHA256SUMS` y, si no coincide, MUST abortar sin tocar la instalación existente. Con un `clawg.exe` local al lado MUST instalar ese, como hoy. MUST conservar lo que hace: rechazar otra instalación, parar el daemon con el binario anterior, renombrar a `.old-*` y correr `clawg path add` (`scripts/install.ps1:146-223`).

#### Escenario: instalación en limpio
- GIVEN Windows sin clawg y sin `clawg.exe` junto al script
- WHEN corre `irm <url> | iex`
- THEN se instala el binario del último release y `clawg update --check` dice «al día»

#### Escenario: hash que no coincide
- GIVEN que el binario bajado no coincide con `SHA256SUMS`
- WHEN corre el script
- THEN falla con un mensaje y el `clawg.exe` instalado queda intacto

**Prueba:** a mano en Windows (máquina limpia), fuera de la suite; el script no tiene tests automáticos hoy (`scripts/` sólo contiene `install.ps1` y `compute_sha.ps1`).

## R9. Falta del VC++ Redistributable

Si falta el VC++ Redistributable, `install.ps1` MUST decirlo con el enlace oficial y MUST NOT instalar nada: ni el paquete ni DLL junto al `.exe`. El sha256 de `install.ps1` prueba la integridad de la descarga, no su autoría, y la documentación MUST decirlo.

#### Escenario: Windows sin el redistribuible
- GIVEN Windows sin VC++ Redistributable
- WHEN corre el script
- THEN termina con un mensaje con el enlace y no deja archivos nuevos en el directorio de instalación

**Prueba:** a mano en Windows limpio, fuera de la suite.

## R10. Documentación

El README MUST traer la instalación en una línea, cómo verificar lo bajado (`minisign` con la clave del repositorio), la glibc mínima de Linux, SmartScreen y Smart App Control, qué pasa con una clave comprometida, y «Actualizar» con la vuelta atrás a mano. `.claude/CLAUDE.md` MUST describir el flujo de Versioning y de CI con el release, y listar `update` en Commands.

#### Escenario: README completo
- GIVEN el README
- WHEN el test lo lee
- THEN contiene la URL de `install.ps1`, la glibc mínima y la sección «Actualizar»

**Prueba:** `the_readme_documents_the_install_and_update_path` (convención, `conventions.rs`).

---

# Dominio `self-update`

Todo lo siguiente es el comando `clawg update [--check]`.

## S1. Versiones

Una versión MUST ser exactamente `X.Y.Z` de tres enteros sin signo, con un `v` inicial opcional sólo en el tag. `v0.19`, `0.19.0-rc1` y la cadena vacía MUST rechazarse. `update` MUST ofrecer sólo una versión estrictamente mayor que la instalada; una igual o menor MUST tratarse como «al día». La comparación MUST ser numérica (`0.9.0 < 0.18.0`). Sin dependencia nueva.

#### Escenario: orden
- GIVEN `0.18.0` instalada
- WHEN se compara con `0.19.0`, con `0.18.0` y con `0.17.9`
- THEN sólo `0.19.0` es una actualización

**Test (no compila hoy):** `version_parse_and_order_cases` (módulo puro, `#[cfg(test)]`).

## S2. Elegibilidad del binario

La decisión de si este binario puede actualizarse MUST ser una función pura, y MUST negarse, con un mensaje que diga por qué, cuando el binario: es un build debug, no lleva la marca de distribución, está dentro de un directorio de target de cargo, o corre en el perfil `dev`. Un release marcado en `prod` MUST pasar. `--check` MUST obedecer la misma decisión.

#### Escenario: cada negativa
- GIVEN un build debug, otro sin marca, uno bajo `target/release/` y uno en `dev`
- WHEN se evalúa la elegibilidad
- THEN cada uno se niega con un motivo distinto

#### Escenario: release marcado
- GIVEN un build release con marca, fuera de un target, en `prod`
- WHEN se evalúa
- THEN pasa

**Test:** `update_eligibility_cases` (puro).

## S3. Elección del release y del asset

Entre los releases que devuelve la API de GitHub, `update` MUST ignorar borradores y prereleases y elegir el más nuevo. El asset MUST elegirse por triple de compilación (`clawg-<triple>[.exe]`). Si el release no trae el asset propio, el error MUST nombrar el triple.

#### Escenario: salto de borradores
- GIVEN un JSON con un borrador y un prerelease más nuevos que el último release
- WHEN se elige
- THEN se elige el último release publicado

#### Escenario: asset ausente
- GIVEN un release sin el asset del triple propio
- WHEN se resuelve el asset
- THEN falla y el mensaje contiene el triple

**Test:** `release_choice_and_asset_cases` (puro, sobre JSON fijo).

## S4. `SHA256SUMS`

El analizador de `SHA256SUMS` MUST rechazar una línea faltante para el asset, un hash que no tiene 64 dígitos hexadecimales y un nombre duplicado.

#### Escenario: líneas inválidas
- GIVEN archivos con la línea del asset ausente, con un hash corto y con el nombre repetido
- WHEN se analizan
- THEN los tres fallan con un error distinto

**Test:** `sha256sums_parse_cases` (puro).

## S5. Firma

`update` MUST verificar la firma minisign de `SHA256SUMS` con alguna clave de la lista (`minisign-verify`, la única dependencia nueva) antes de confiar en un hash. MUST fallar para una clave fuera de la lista, un `SHA256SUMS` alterado y un `.minisig` de otro archivo.

#### Escenario: firma válida con la segunda clave
- GIVEN una lista de dos claves y un archivo firmado con la segunda
- WHEN se verifica
- THEN pasa

#### Escenario: firma que no vale
- GIVEN un archivo firmado con una clave fuera de la lista, un archivo alterado, o el `.minisig` de otro archivo
- WHEN se verifica
- THEN falla y `update` no continúa

**Test:** `signature_cases` (puro, con claves de prueba generadas para la suite).

## S6. Sólo a pedido

El daemon MUST NOT consultar nada por su cuenta. Sólo `clawg update` y `clawg update --check` MUST tocar la red. Las peticiones MUST mandar `User-Agent`. `--check` MUST NOT descargar el binario ni modificar nada, y sus códigos de salida MUST distinguir «hay una versión nueva», «al día» y «error» (los números concretos los fija el diseño).

#### Escenario: `--check` con y sin versión nueva
- GIVEN un servidor de release falso con claves de prueba
- WHEN corre `clawg update --check` primero con una versión mayor y luego con la misma
- THEN los dos códigos de salida son distintos y distintos del de error, y el binario instalado no cambia

**Test:** `update_check_reports_a_newer_version_and_an_up_to_date_one` en `crates/clawg-cli/tests/binary.rs`; que las peticiones llevan `User-Agent`, en un test del cliente contra el servidor falso.

## S7. Verificar antes de tocar

Antes del intercambio `update` MUST: bajar el candidato junto al destino, verificar la firma de `SHA256SUMS` (S5), comprobar el sha256 del candidato contra esa lista y ejecutar su `--version` y comprobar que da la versión elegida. Si cualquiera falla, MUST dejar todo como estaba (binario instalado, daemon y datos), sin dejar el candidato. En Linux, sin permiso de escritura donde está el binario, MUST fallar antes de bajar nada con un mensaje claro, y MUST NOT usar `sudo`.

#### Escenario: hash que no coincide
- GIVEN un candidato cuyo sha256 difiere del de `SHA256SUMS`
- WHEN corre `update`
- THEN falla, el binario instalado y su daemon siguen como estaban y no queda el candidato

#### Escenario: sin permiso en Linux
- GIVEN un binario instalado en un directorio no escribible
- WHEN corre `update`
- THEN falla antes de la primera petición de descarga, con un mensaje que lo dice

**Test:** `a_hash_mismatch_never_touches_the_installed_binary` (plan, en proceso) y `a_failed_download_or_signature_leaves_everything_as_it_was` (plan); el permiso de Linux, `an_unwritable_install_dir_fails_before_downloading` (puro, sobre una función que decide con el resultado de un chequeo de escritura).

## S8. Plan e intercambio

El plan de `update` MUST ser una lista de pasos como datos, ejecutada aparte, de modo que el orden y la deshecha de cada falla se prueben sin disco ni red. Con daemon, el orden MUST ser: parar el daemon con el binario instalado (`stop_in`, `crates/clawg-cli/src/stop.rs:23`); renombrar el binario a `.old-*`; poner el candidato; arrancar el daemon nuevo por su ruta explícita (no por `ProcessSpawner`, que lanza `current_exe()`, `client.rs:979-982`); esperar `/health`. Sin daemon, `update` MUST NOT arrancar uno. Al empezar, MUST borrar los `.old-*` de corridas anteriores. Si un editor abre un puente entre la parada y el arranque, `update` MUST confirmar la versión del daemon que contesta e informarla. Dos `update` a la vez MUST NOT intercambiar a la vez: el segundo falla con un error claro (el mecanismo lo fija el diseño).

#### Escenario: actualización completa con daemon
- GIVEN un daemon corriendo y un servidor de release falso con una versión mayor
- WHEN corre `clawg update`
- THEN `/health` contesta la versión nueva, el binario viejo queda como `.old-*` y un puente abierto llega al daemon nuevo

#### Escenario: sin daemon
- GIVEN ningún daemon corriendo
- WHEN corre `clawg update`
- THEN se intercambia el binario y no se arranca ningún daemon

#### Escenario: segunda actualización a la vez
- GIVEN un `update` en curso
- WHEN arranca otro
- THEN el segundo falla con un error claro y no toca el binario

**Pruebas:** `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` en `binary.rs` (parte del escenario de `binary.rs:535` con el binario reemplazado); `update_without_a_daemon_does_not_start_one` (plan); `plan_order_with_and_without_daemon` (plan); `two_updates_do_not_swap_at_once` (puro o binario, según el diseño).

## S9. Vuelta atrás tras el intercambio

Si el daemon nuevo no contesta `/health` con la versión nueva en el plazo, `update` MUST devolver el binario viejo a su nombre y volver a levantar su daemon; el plazo MUST distinguir «arrancando» (migración o re-embed largos) de «no arrancó». Si el daemon nuevo alcanzó a migrar el store y el binario viejo se niega (`MemoryError::SchemaNewer`, `crates/clawg-core/src/memory/error.rs:54-58`), `update` MUST resolverlo de una de dos maneras, y el diseño decide cuál: restaurar el snapshot `pre-migration` en el acto (`restore` guarda antes el store reemplazado, así que nada se destruye), o dejar el binario nuevo y decir cómo volver. Ambas satisfacen este requisito siempre que se cumpla S10. Un candidato que no arranca MUST dejar el binario viejo y su daemon.

*Decidido en `design.md`: cuál de las dos opciones de vuelta atrás tras una migración.*

#### Escenario: el nuevo no arranca
- GIVEN un candidato que no levanta el daemon
- WHEN corre `update` con daemon previo
- THEN vuelven el binario viejo y su daemon, `/health` contesta la versión vieja y `update` sale con error

#### Escenario: el nuevo migró y el viejo se niega
- GIVEN un daemon nuevo que migró el store y deja de contestar
- WHEN el binario viejo vuelve y se niega por `SchemaNewer`
- THEN el usuario queda en una de las dos situaciones permitidas, con un mensaje que lo explica y el comando de vuelta

**Pruebas:** `a_new_binary_that_does_not_start_brings_the_old_one_and_its_daemon_back` en `binary.rs`; `each_failed_step_undoes_what_it_did` (plan).

## S10. Garantías de datos

`update` MUST NOT cambiar esquema, identificadores, estados ni el contrato de embedding por sí mismo. Se cumple siempre:

1. Antes del intercambio no se toca nada: una falla de descarga, firma, hash o humo deja binario, daemon y `memory.db` como estaban.
2. Después del intercambio, o contesta `/health` con la versión nueva el daemon nuevo, o vuelven el binario viejo y su daemon, o (S9) el usuario queda en una situación explicada con un camino de vuelta.
3. El usuario MUST NOT quedar sin daemon y sin saber por qué.
4. `update` MUST NOT borrar un `.old-*` ni un snapshot de la corrida en curso.
5. Un store con migraciones pendientes MUST dejar su `pre-migration` en `<data>/backups` (`crates/clawg-core/src/memory/db.rs:207-213`) y sus datos MUST seguir alcanzables. Un store de un clawg más nuevo MUST hacer que el daemon nuevo se niegue sin tocarlo, y el usuario queda con el binario anterior y su daemon.

#### Escenario: migración pendiente
- GIVEN una base sembrada en la versión anterior con una migración pendiente
- WHEN el daemon nuevo la abre tras `update`
- THEN existe el snapshot `pre-migration` y los datos siguen alcanzables por búsqueda

#### Escenario: store de un clawg más nuevo
- GIVEN un store con esquema mayor que el del candidato
- WHEN corre `update`
- THEN el daemon nuevo se niega sin tocarlo, y el usuario queda con el binario anterior y su daemon respondiendo

**Pruebas:** `an_update_over_a_pending_migration_leaves_a_pre_migration_snapshot_and_the_data` y `an_update_over_a_newer_store_leaves_the_old_binary_and_its_daemon` en `binary.rs`, con la base sembrada en la versión anterior (regla de `openspec/config.yaml` para cambios que tocan el esquema). Prerrequisito ya fusionado: `to_prune` conserva el `pre-migration` más nuevo (`crates/clawg-core/src/memory/backup.rs:238-259`, 0.18.1), con su test.

## S11. Informe

Al terminar bien, `update` MUST informar la versión anterior y la nueva, el snapshot `pre-migration` si se escribió (su ruta), que volver atrás pierde lo escrito después de él, y que los editores abiertos siguen con el puente viejo hasta reiniciarse. Un `update` fallido MUST decir en qué estado dejó la instalación y el daemon.

#### Escenario: informe de éxito
- GIVEN una actualización completa de `0.18.1` a `0.19.0` con migración
- WHEN termina
- THEN la salida contiene ambas versiones, la ruta del `pre-migration` y el aviso de reinicio de los editores

**Test:** `the_update_report_names_versions_snapshot_and_restart_notice` (puro, sobre la función que arma el texto).

## S12. Sustitución sólo en builds de test

La URL base del servidor de releases y la lista de claves MUST poder sustituirse sólo en un build con la feature `testing`, con la regla de `CLAWG_TEST_HOME` (`crates/clawg-core/src/paths.rs:169`): un build sin `testing` MUST NOT leer esas variables, y la lista embebida de producción MUST NOT contener la clave de prueba. Ningún test MUST tocar `~/.clawg*` ni la red real.

#### Escenario: build de producción
- GIVEN un build sin `testing`
- WHEN las variables de sustitución están puestas
- THEN `update` usa la URL de GitHub y la lista embebida

**Pruebas:** `only_dev_dependencies_turn_on_the_test_stand_ins` (ya existe, `conventions.rs`) más `the_embedded_key_list_has_the_repository_key_and_not_the_test_key` (R6, medida 9); `update_overrides_are_read_only_in_a_test_build` (puro, con `cfg`).

---

# Cambio sobre `specs/install`

## I1. CI (MODIFIED)

Reemplaza la sección «CI» de [`specs/install`](../../specs/install/spec.md). *(Antes: «Cada pull request y cada push a `main` MUST correr…», con las líneas `ci.yml:3-6`, `:19`, `:21`, `:33` y `:36`; ya no son las de hoy porque el workflow añadió `paths-ignore`.)*

La CI MUST correr en cada pull request y en cada push a `main` que cambie algún archivo fuera de `docs/**`, `openspec/**`, `.claude/**`, `.cursor/**`, `.gemini/**`, `scripts/**`, `README.md` y `AGENTS.md` (`.github/workflows/ci.yml:5-26`). Un push que sólo cambia esas rutas MUST NOT correr CI y, por lo tanto, MUST NOT disparar `release.yml` (R4). En `ubuntu-latest` y `windows-latest` (`ci.yml:41`) MUST correr el comando de `clippy` que hace fallar los warnings del compilador (`ci.yml:53`) y `cargo test --workspace` (`ci.yml:56`), con las pruebas del binario incluidas; las dos plataformas MUST correr hasta el final aunque una falle (`fail-fast: false`, `ci.yml:39`). El job no descarga el modelo: la suite usa `fake`. La CI es el único disparador de `release.yml`: un release MUST NOT publicarse sin una CI exitosa por push a `main`, y la CI MUST NOT leer secretos del environment `release`.

La suite MUST pasar en Linux y en Windows (el párrafo sobre las fixtures de `doctor` no cambia).

#### Escenario: bump de versión
- GIVEN un push a `main` cuyo commit cambia `Cargo.toml` y `Cargo.lock`
- WHEN la CI termina en verde
- THEN `release.yml` se dispara por ese `workflow_run`

#### Escenario: sólo documentación
- GIVEN un push a `main` que sólo cambia `README.md` y `openspec/**`
- WHEN GitHub evalúa los workflows
- THEN no corre la CI ni `release.yml`

**Pruebas:** `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` (convención sobre `ci.yml`, falla hoy porque la sección vieja cita líneas que ya no existen); el comportamiento en GitHub, en el workflow, fuera de la suite.
