# release — Especificación

Fuente de verdad de cómo se publica `clawg`: los metadatos y la licencia, el TLS sin libssl, el workflow `release.yml` que compila, firma y publica los binarios cuando una CI de un push a `main` pasó y la versión no tiene tag, los assets de cada release, las claves de firma, el `install.ps1` que se baja del release y la documentación de la instalación. El comando que instala una versión nueva está en [`specs/self-update`](../self-update/spec.md); cómo la CI prueba el binario, en [`specs/install`](../install/spec.md).

**Origen:** [release-and-self-update](../../changes/archive/2026-10-18-release-and-self-update/proposal.md), archivado el 2026-10-18. Los ids de requisito son los de esa spec. [ci-hardening](../../changes/archive/2026-10-19-ci-hardening/proposal.md), archivado el 2026-10-19, modificó R3 (el toolchain fijado y el reintento del build) y R8 (el análisis de `install.ps1`).

## Definiciones

- **Release:** un release de GitHub del repositorio, no borrador ni prerelease, con un tag `vX.Y.Z`.
- **Marca de distribución:** la constante de compilación `CLAWG_DIST_TARGET=<triple>`; sólo la ponen los builds de `release.yml`. Un `cargo build --release` local no la tiene.
- **Lista de claves:** las claves públicas minisign que el binario acepta, embebidas en él. En producción son dos: la clave primaria (su mitad privada está en el environment `release`) y la clave de respaldo (su mitad privada está fuera de GitHub).

## R1. Metadatos y licencia

El repositorio MUST llevar un `LICENSE` MIT cuyo titular es «The Claw Garden contributors». `[workspace.package]` MUST fijar `license = "MIT"`, `repository = "https://github.com/MarcossIC/claw-garden"` y `authors = ["The Claw Garden contributors"]`. Nada commiteado MUST nombrar a una persona.

#### Escenario: metadatos completos
- GIVEN el `Cargo.toml` raíz
- WHEN el test lee `[workspace.package]`
- THEN `license` es `MIT`, `repository` no está vacío y ningún `authors` es un nombre de persona
- AND existe `LICENSE` con el titular

**Test:** `the_manifest_carries_the_publication_metadata` en `crates/clawg-cli/tests/conventions.rs`.

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

`release.yml` MUST compilar con `cargo build --release -p clawg-cli` y MUST NOT usar `--workspace`: `clawg-eval` enciende la feature `eval` de clawg-core (`crates/clawg-eval/Cargo.toml:34`), que los builds de producción no enlazan (`crates/clawg-core/Cargo.toml:16-19`), y la unificación de features la llevaría al binario. Cada build MUST poner `CLAWG_DIST_TARGET` con su triple. Linux MUST compilarse en `ubuntu-24.04` fijo: el ONNX Runtime precompilado que `ort-sys` enlaza estático llama símbolos de glibc 2.38 (`__isoc23_strtol`, `__isoc23_strtoll`, `__isoc23_strtoull`), que `ubuntu-22.04` (glibc 2.35) no tiene y el enlace falla.

El build MUST usar el toolchain fijado en `rust-toolchain.toml` (`toolchain:` de `dtolnay/rust-toolchain`, `.github/workflows/release.yml:90`; [`specs/install`](../install/spec.md), «Toolchain fijado») y MUST reintentar `cargo build --release --locked -p clawg-cli --target <triple>` hasta tres veces con espera creciente (`release.yml:102-111`): `ort-sys` baja ONNX Runtime de `cdn.pyke.io` mientras compila, con su SHA-256 fijado, y un pedido fallido ahí no tiene que frenar un release.

#### Escenario: comando de build
- GIVEN `.github/workflows/release.yml`
- WHEN el test lo lee
- THEN todo `cargo build` lleva `-p clawg-cli`, `--release` y `--locked`, ninguno lleva `--workspace`, cada build define `CLAWG_DIST_TARGET` y se reintenta
- AND el job de Linux corre en `ubuntu-24.04`

**Test:** `the_release_workflow_builds_only_the_cli_with_the_dist_mark` (convención, `conventions.rs`).

## R4. Disparador y etiqueta

`release.yml` MUST dispararse por `workflow_run` de `CI` y publicar sólo si `event == 'push'`, `conclusion == 'success'` y `head_branch == 'main'`, y la versión de `[workspace.package]` leída en el `head_sha` de esa ejecución no tiene tag `vX.Y.Z`. MUST etiquetar ese `head_sha`. La ejecución de CI de un pull request, aun desde un fork con una rama llamada `main`, MUST NOT publicar. Si el tag ya existe, el workflow MUST terminar sin publicar ni fallar.

#### Escenario: bump en main
- GIVEN una CI exitosa por push a `main` cuyo `Cargo.toml` dice `X.Y.Z` y sin tag `vX.Y.Z`
- WHEN corre `release.yml`
- THEN publica `vX.Y.Z` sobre ese `head_sha`

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

`SHA256SUMS` MUST firmarse con minisign usando una clave privada cifrada con contraseña; la clave y la contraseña son secretos del environment `release`, limitado a `main`, sin revisores obligatorios. El paso de firma MUST pasar la contraseña por la entrada estándar (`printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S -s <clave> -t "clawg vX.Y.Z SHA256SUMS" -m SHA256SUMS`), nunca como argumento, y MUST NOT imprimirla ni activar la traza de la shell; los secretos MUST llegar sólo como `env` de ese paso, y GitHub enmascara sus valores en los registros.

La lista de claves MUST contener, desde el primer release, dos claves públicas commiteadas: la primaria, cuya mitad privada es la del environment `release`, y una de respaldo, cuya mitad privada se guarda fuera de GitHub y, mientras es el respaldo, MUST NOT estar en el repositorio ni en la CI. Ambas MUST estar embebidas en el binario. Si se pierde o se filtra la primaria, el respaldo MUST promoverse: el commit del release copia `release/minisign-next.pub` sobre `release/minisign.pub`, pone en `release/minisign-next.pub` la pública de un respaldo nuevo, y la privada promovida y su contraseña reemplazan los secretos del environment `release`; los binarios instalados aceptan ese release porque su lista tiene el respaldo anterior. No protege de una filtración a los binarios que no se actualicen: siguen aceptando la clave filtrada hasta instalar un release cuya lista ya no la tenga, y la documentación MUST decirlo. El workflow MUST verificar su propia firma (`minisign -Vm SHA256SUMS`) con la clave primaria del repositorio (el `release/minisign.pub` del commit que publica) antes de publicar. La lista MUST NOT contener la clave de prueba.

#### Escenario: firma verificada antes de publicar
- GIVEN `SHA256SUMS` y `SHA256SUMS.minisig` recién generados
- WHEN el workflow verifica con la clave del repositorio
- THEN pasa, y sólo entonces se crea el release

#### Escenario: la contraseña no se filtra al registro
- GIVEN el paso de firma del workflow
- WHEN el test lee `release.yml`
- THEN la contraseña llega por `env` y por entrada estándar, sin `set -x` ni `echo` del secreto, y ni la clave ni la contraseña se declaran fuera de ese paso

#### Escenario: las dos claves y no la de prueba
- GIVEN la lista de claves embebida
- WHEN el test la compara con la clave de prueba de la suite
- THEN contiene la clave primaria y la de respaldo del repositorio, y no contiene la de prueba

**Pruebas:** `the_embedded_key_list_has_the_primary_and_backup_keys_and_not_the_test_key` (convención, medida 9 de la propuesta); `the_signing_step_reads_the_key_password_from_stdin_without_echo` (convención sobre `release.yml`); la verificación con `minisign`, en el workflow, fuera de la suite.

## R7. Verificación en el workflow

Antes de publicar, el workflow MUST ejecutar el binario de cada plataforma y comprobar que `--version` responde la versión del tag. Las acciones de terceros MUST fijarse por SHA de commit. La clave privada MUST NOT estar disponible para jobs que no firman.

#### Escenario: versión del tag
- GIVEN el binario construido para el tag `vX.Y.Z`
- WHEN el paso de humo corre `--version`
- THEN la salida contiene `X.Y.Z`; si no, el workflow falla antes de publicar

**Pruebas:** `third_party_actions_are_pinned_to_a_commit` (convención sobre `release.yml`); el humo `--version`, en el workflow, fuera de la suite.

## R8. Bootstrap: `install.ps1` desde el release

`install.ps1` MUST adjuntarse a cada release y ser alcanzable en `https://github.com/MarcossIC/claw-garden/releases/latest/download/install.ps1`. Sin un `clawg.exe` local al lado, MUST bajar el binario del último release, compararlo con `SHA256SUMS` y, si no coincide, MUST abortar sin tocar la instalación existente. Con un `clawg.exe` local al lado MUST instalar ese, como hoy. MUST conservar lo que hace: rechazar otra instalación, parar el daemon con el binario anterior, renombrar a `.old-*` y correr `clawg path add` (`scripts/install.ps1:226-308`).

#### Escenario: instalación en limpio
- GIVEN Windows sin clawg y sin `clawg.exe` junto al script
- WHEN corre `irm <url> | iex`
- THEN se instala el binario del último release y `clawg update --check` dice «al día»

#### Escenario: hash que no coincide
- GIVEN que el binario bajado no coincide con `SHA256SUMS`
- WHEN corre el script
- THEN falla con un mensaje y el `clawg.exe` instalado queda intacto

**Prueba:** a mano en Windows (máquina limpia), fuera de la suite. El script no se ejecuta en ninguna prueba automática; `.github/workflows/installer.yml` lo analiza con PSScriptAnalyzer cuando cambia `scripts/**` ([`specs/install`](../install/spec.md), «Análisis de `install.ps1`»).

## R9. Falta del VC++ Redistributable

Si falta el VC++ Redistributable, `install.ps1` MUST decirlo con el enlace oficial y MUST NOT instalar nada: ni el paquete ni DLL junto al `.exe`. El sha256 de `install.ps1` prueba la integridad de la descarga, no su autoría, y la documentación MUST decirlo.

#### Escenario: Windows sin el redistribuible
- GIVEN Windows sin VC++ Redistributable
- WHEN corre el script
- THEN termina con un mensaje con el enlace y no deja archivos nuevos en el directorio de instalación

**Prueba:** a mano en Windows limpio, fuera de la suite.

## R10. Documentación

El README MUST traer la instalación en una línea, cómo verificar lo bajado (`minisign` con la clave del repositorio), la glibc mínima de Linux, SmartScreen y Smart App Control, qué pasa con una clave comprometida (incluido que el respaldo no protege a los binarios que no se actualicen), y «Actualizar» con la vuelta atrás a mano. `.claude/CLAUDE.md` MUST describir el flujo de Versioning y de CI con el release, y listar `update` en Commands.

#### Escenario: README completo
- GIVEN el README
- WHEN el test lo lee
- THEN contiene la URL de `install.ps1`, la glibc mínima y la sección «Actualizar»

**Prueba:** `the_readme_documents_the_install_and_update_path` (convención, `conventions.rs`).
