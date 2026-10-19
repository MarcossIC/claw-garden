# ci-hardening — Especificación (delta sobre `specs/install` y `specs/release`)

C1 a C4 reemplazan y amplían la sección «CI» de [`specs/install`](../../specs/install/spec.md); C5 y C6 son workflows nuevos; C7 modifica R3 de [`specs/release`](../../specs/release/spec.md) y C8 su R8. Todas las pruebas están en `crates/clawg-cli/tests/conventions.rs` y leen los archivos como texto; lo que hace GitHub con ellos sólo se ve en una corrida real.

## C1 (modifica «CI»). El CI compila lo que se publica

Los triggers, las rutas ignoradas, las dos plataformas y `fail-fast: false` no cambian. Los dos comandos existentes MUST llevar `--locked`, así un `Cargo.lock` atrasado respecto de `Cargo.toml` pone rojo el CI en vez de reescribirse:

- `cargo clippy --locked --workspace --all-targets -- -A clippy::all -D warnings`
- `cargo test --locked --workspace`

Y el job MUST correr, en las dos plataformas, la configuración de producción, sin la feature `testing` de clawg-core:

- `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`: sin `--all-targets`, no entran las dev-dependencies, y con ellas `testing`, que es lo que construye `release.yml`.
- `cargo test --locked -p clawg-core --lib build_info`: las pruebas de `build_info` en su variante `cfg(not(feature = "testing"))`.

El CI MUST NOT leer secretos, usar un `environment`, pedir `id-token` ni escribir contenidos.

#### Escenario: lock atrasado
- GIVEN un commit que sube la versión en `Cargo.toml` sin correr `cargo update --workspace`
- WHEN corre el CI
- THEN `clippy` falla porque el lock tendría que cambiar

#### Escenario: un error que sólo existe sin `testing`
- GIVEN un error de compilación dentro de un bloque `cfg(not(feature = "testing"))` de clawg-core
- WHEN corre el CI de un PR
- THEN el paso de `clippy` de `-p clawg-cli` falla antes del merge

**Prueba:** `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` (ajustada) y `the_ci_workflow_checks_the_locked_production_build`.

## C2. Acciones fijadas y sin credenciales

Toda acción de terceros de todo workflow de `.github/workflows/` MUST estar fijada por un SHA completo de commit, con la etiqueta o rama que representa en un comentario al final de la línea. Todo `actions/checkout` MUST llevar `persist-credentials: false`. `Swatinem/rust-cache` MUST llevar `save-if: ${{ github.ref == 'refs/heads/main' }}`: un PR lee la caché y sólo un push a `main` la escribe.

#### Escenario: una acción por etiqueta
- GIVEN un workflow nuevo con `uses: actions/checkout@v7`
- WHEN corre la suite
- THEN falla nombrando el archivo y la línea

**Prueba:** `third_party_actions_are_pinned_to_a_commit` (sobre todos los workflows), `every_checkout_drops_the_token` y `the_ci_workflow_checks_the_locked_production_build` (el `save-if`).

## C3. Un toolchain fijado

`rust-toolchain.toml` MUST fijar `channel` a una versión exacta `X.Y.Z` y pedir `clippy`. Cada `dtolnay/rust-toolchain` de cada workflow MUST pasar esa misma versión en `toolchain:`. `[workspace.package] rust-version` MUST ser al menos 1.88, el mayor `rust-version` de las dependencias del lock (`ort`, `ort-sys`, `rmcp`, `ignore`), y no mayor que la versión del toolchain. El 1.88 es una constante de la prueba, que se revisa al subir una dependencia.

#### Escenario: subir el toolchain
- GIVEN `rust-toolchain.toml` sube a otra versión y un workflow queda con la anterior
- WHEN corre la suite
- THEN falla nombrando el workflow y las dos versiones

**Prueba:** `the_toolchain_is_pinned_once_for_every_workflow`.

## C4. Dependabot para las acciones

`.github/dependabot.yml` MUST actualizar el ecosistema `github-actions` del directorio `/` una vez por mes, con `target-branch: develop`. No actualiza crates: de los avisos se ocupa C5.

**Prueba:** `dependabot_updates_the_pinned_actions_on_develop`.

## C5. Avisos de seguridad, informativos

`.github/workflows/advisories.yml` MUST correr `cargo deny --locked check advisories` con el `deny.toml` de la raíz en cada PR y en cada push a `main` que cambie un `Cargo.toml`, `Cargo.lock`, `deny.toml` o el propio workflow, y cada semana (`schedule:`). `cargo-deny` MUST bajarse de una URL con versión exacta y compararse con un SHA-256 fijado antes de usarse. El workflow MUST NOT llamarse `CI`, así no es un disparador de `release.yml` ni la puerta de un release, y MUST NOT leer secretos. Cada `ignore` de `deny.toml` MUST llevar su `reason`.

#### Escenario: aviso nuevo sin cambio en el repo
- GIVEN una dependencia del lock recibe un aviso de RustSec
- WHEN corre la ejecución semanal
- THEN el workflow de avisos queda rojo con el id del aviso, y los PR y el release siguen sin él

**Prueba:** `the_advisories_workflow_runs_a_pinned_cargo_deny_apart_from_ci`. Que pase sobre el lock lo dice `cargo deny --locked check advisories` al implementar ([verify](verify.md)).

## C6. Análisis de `install.ps1`

`.github/workflows/installer.yml` MUST correr PSScriptAnalyzer sobre `scripts/` en un runner de Windows, en cada PR y push a `main` que cambie `scripts/**` o el propio workflow, con `scripts/PSScriptAnalyzerSettings.psd1`. El módulo MUST bajarse de una URL con versión exacta y compararse con un SHA-256 fijado. Cualquier hallazgo MUST hacer fallar el job. Cada regla excluida en la configuración MUST llevar su motivo en un comentario. El workflow MUST NOT llamarse `CI` ni leer secretos.

#### Escenario: un hallazgo nuevo
- GIVEN un cambio en `install.ps1` que usa un alias (`gci`)
- WHEN corre el workflow
- THEN falla con el hallazgo `PSAvoidUsingCmdletAliases`

**Prueba:** `the_installer_is_analyzed_with_a_pinned_psscriptanalyzer`. Que hoy no haya hallazgos lo dice PSScriptAnalyzer al implementar ([verify](verify.md)).

## C7 (modifica R3 de `specs/release`). Toolchain y reintento del build

El build de `release.yml` MUST usar el toolchain de C3 y MUST reintentar `cargo build` hasta tres veces, con espera creciente, para que una caída de `cdn.pyke.io` (de donde `ort-sys` baja ONNX Runtime, con su SHA-256 fijado) no frene un release. El comando no cambia: `--release --locked -p clawg-cli --target ${{ matrix.target }}` con `CLAWG_DIST_TARGET`.

**Prueba:** `the_release_workflow_builds_only_the_cli_with_the_dist_mark` (ajustada).

## C8 (modifica R8 de `specs/release`). `install.ps1` analizado

La prueba de R8 suma el análisis estático de C6. La instalación en un Windows limpio sigue siendo a mano.
