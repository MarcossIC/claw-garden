# ci-hardening — Tareas

Strict TDD: en cada tarea la prueba de convenciones va primero y falla por la aserción. Una tarea, un commit convencional que compila y pasa la suite.

## 1. Lock sin avisos (C5)

- [x] `cargo update -p crossbeam-epoch -p rand@0.9.2 -p der`; `cargo deny --locked check advisories` sólo marca `paste`.
- Commit: `fix(deps): update crossbeam-epoch, rand and der past their advisories`

## 2. El CI compila lo que se publica (C1)

- [x] Prueba `the_ci_workflow_checks_the_locked_production_build`; `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` con `--locked`.
- [x] `--locked` en `ci.yml` y los pasos de `-p clawg-cli` y `build_info`.
- Commit: `ci: check the locked graph and the production build on every pull request`

## 3. Acciones fijadas (C2)

- [x] `third_party_actions_are_pinned_to_a_commit` sobre todos los workflows; `every_checkout_drops_the_token`; `save-if` en `the_ci_workflow_checks_the_locked_production_build`.
- [x] SHA en las tres acciones de `ci.yml`, `persist-credentials: false`, `save-if`.
- Commit: `ci: pin the CI actions, drop the checkout token and write the cache only from main`

## 4. Toolchain fijado (C3, C7)

- [x] Prueba `the_toolchain_is_pinned_once_for_every_workflow`.
- [x] `rust-toolchain.toml` en 1.99.1, `toolchain: 1.99.1` en los dos workflows, `rust-version = "1.88"`.
- Commit: `build: pin the toolchain to 1.99.1 and declare 1.88 as the minimum Rust`
- [x] La verificación compila con 1.88.0 y falla: `tree-sitter-language` 0.1.9 pide 1.90, y con 1.90 `golden-mine` no infiere el tipo de un `pick`.
- Commits: `fix(eval): dereference the picked opening as the other picks do` y `fix(build): declare 1.90 as the minimum Rust`

## 5. Avisos (C5)

- [x] Prueba `the_advisories_workflow_runs_a_pinned_cargo_deny_apart_from_ci`.
- [x] `deny.toml` con `paste` en `ignore` y su motivo; `advisories.yml`.
- Commit: `ci: report RustSec advisories on the lock every week and on dependency changes`

## 6. Reintento del build del release (C7)

- [x] `the_release_workflow_builds_only_the_cli_with_the_dist_mark` exige el reintento.
- [x] Bucle de tres intentos en el paso de build.
- Commit: `ci: retry the release build when the ONNX Runtime download fails`

## 7. `install.ps1` (C6)

- [x] Prueba `the_installer_is_analyzed_with_a_pinned_psscriptanalyzer`.
- [x] `installer.yml` y `scripts/PSScriptAnalyzerSettings.psd1`.
- Commit: `ci: analyze the installer script with PSScriptAnalyzer`

## 8. Dependabot (C4)

- [x] Prueba `dependabot_updates_the_pinned_actions_on_develop`.
- [x] `.github/dependabot.yml`.
- Commit: `ci: let Dependabot keep the pinned actions current on develop`

## 9. Docs

- [x] `.claude/CLAUDE.md`: Testing (los comandos con `--locked`, los pasos de producción, los workflows nuevos, el toolchain) y Versioning.
- Commit: `docs: describe the hardened CI, the advisories and the pinned toolchain`

## 10. Verificación

- [x] `cargo test --locked --workspace`, los dos `clippy`, `build_info` sin `testing`, `cargo deny` y PSScriptAnalyzer, en [`verify.md`](verify.md); el CI del PR en las dos plataformas.

## 11. Archive

- [x] «CI» de [`specs/install`](../../../specs/install/spec.md) con C1 a C6; R3 y R8 de [`specs/release`](../../../specs/release/spec.md).
- [x] `git mv` a `changes/archive/2026-10-19-ci-hardening`; `state.yaml` archivado; fila 65, orden de ejecución y backlog del `README.md` de openspec; subir el minor.
