# clippy-enforced — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, sobre `ec0a3e7`, el último commit del cambio antes de este registro:

- `cargo clippy --locked --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning.
- `cargo clippy --locked -p clawg-cli -- -D warnings`, lo que construye el release, sale con 0.
- `cargo test --locked --workspace --no-fail-fast` pasa. Contra la base (`c0bf3af`, corrida antes del primer arreglo), cada binario de tests da los mismos pasados e ignorados, salvo `crates/clawg-cli/tests/conventions.rs`, que suma `every_package_takes_the_workspace_lints` y `no_workflow_turns_clippy_lints_off`.
- `cargo test --locked -p clawg-core --lib build_info` pasa.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: lo que ve el modelo no cambió.
- Windows lo corrió después el CI del pull request, que compila el código `cfg(windows)`: ver «Al archivar».

## El conteo, antes y después

| | Hallazgos |
|---|---|
| Base, sin `-A clippy::all` ([paso 0](exploration.md#paso-0-el-conteo)) | 124 en 34 lints |
| Arreglados, un commit por lint | 94 |
| Anotados con `#[expect(…, reason = "…")]`, un commit por lint | 30: 22 `too_many_arguments`, 6 `large_enum_variant`, 1 `result_large_err`, 1 `double_must_use` |
| `#[allow(clippy::…)]` que pasaron a `#[expect]` | 2 (`too_many_arguments`) |
| Después, con la tabla | 0 |

El clippy completo, corrido después de los arreglos, dio uno que no estaba en el conteo: `clippy::filter_next` sobre el `filter(..).next_back()` que había dejado el arreglo de `double_ended_iterator_last` en `token_splitter.rs`. Pasó a `rfind` en ese mismo commit.

Con la tabla, ninguno de los tres lints sueltos da nada: `large_futures` (la future más grande del binario, 9.504 bytes, está por debajo de 16 KiB) y `allow_attributes` con `allow_attributes_without_reason` (no queda ningún `#[allow]` de un lint).

## Los tests en rojo

| Test | Antes | Después |
|---|---|---|
| `every_package_takes_the_workspace_lints` | falla sin la tabla: `level("all")` es `None` | pasa |
| `no_workflow_turns_clippy_lints_off` | falla con `ci.yml` y `cache.yml`, que pasan `-A` | pasa |
| `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` | falla: `ci.yml` no tiene el comando sin `-A clippy::all` | pasa |
| `the_ci_workflow_checks_the_locked_production_build` | falla: falta `cargo clippy --locked -p clawg-cli -- -D warnings` | pasa |

`the_cache_workflow_builds_from_main_what_ci_tests` pasa antes y después: compara `cache.yml` con `ci.yml`, y los dos cambiaron igual.

## El spec contra el código

- **CE1.** `[workspace.lints.clippy]` del `Cargo.toml` raíz pone `all` en `warn` con `priority = -1` y `large_futures`, `allow_attributes` y `allow_attributes_without_reason` en `warn`; no pone nada en `allow`. `crates/clawg-cli/Cargo.toml`, `crates/clawg-core/Cargo.toml` y `crates/clawg-eval/Cargo.toml` tienen `[lints] workspace = true`, después de todas las claves de `[package]` (en `clawg-eval`, después de `autobins = false`).
- **CE2.** `ci.yml:65` y `:71` y `cache.yml:53` y `:55` corren clippy con `-- -D warnings` y nada más. El comentario de `ci.yml:62` dice ahora que los lints de la tabla son errores.
- **CE3.** Con un cambio temporal en `crates/clawg-core/src/memory/latency.rs`, revertido después, `cargo clippy --locked -p clawg-core --lib -- -D warnings` falla con los cuatro errores que pide la spec (y con `dead_code` por las dos funciones de prueba): `#[allow] attribute found` y `allow attribute without specifying a reason` sobre un `#[allow(clippy::too_many_arguments)]`, `useless use of vec!` sobre un `vec!` de ocho elementos, y `this lint expectation is unfulfilled` sobre un `#[expect(clippy::too_many_arguments, …)]` en una función de un parámetro. Los 32 `#[expect]` del workspace llevan `reason`.
- **CE4.** `.claude/CLAUDE.md` y `docs/reference/testing.md` dan `cargo clippy --workspace --all-targets -- -D warnings`; `docs/reference/testing.md` dice qué corre la tabla y cómo se anota una excepción. R6 de `specs/quality` y la sección CI de `specs/install` dan los comandos nuevos.
- No cambian `Cargo.lock`, las migraciones, `agent_contract.json` ni `agent_contract.es.json`.

## Lo que no se midió

- **Otros cambios abiertos.** Las tareas de otros cambios abiertos que dan el comando con `-A clippy::all` (`embedder-footprint`, `knn-prefilter`) registran lo que corrieron y no se tocan, como los `verify.md` archivados.

## Al archivar (2026-11-03)

- **Windows ya se midió.** El job `test (windows-latest)` pasó en #121 y sobre `a7f7337`, el último commit de `develop` antes de publicar. Ese job corre los dos clippy y la suite.
- **`391e0ad`, posterior a esta verificación,** hizo que `no_workflow_turns_clippy_lints_off` también lea un comando que el shell parte en varias líneas. CE2 no cambia.
- **CE1 a CE4 en las specs.** `ec0a3e7` ya los había llevado a R6 de [`specs/quality`](../../../specs/quality/spec.md) y a la sección CI de [`specs/install`](../../../specs/install/spec.md). Al comprobarlos faltaban y se agregaron: el `priority = -1` de CE1, qué se puede anotar y con qué motivo (CE3), lo que tiene que decir `docs/reference/testing.md` (CE4) y los seis escenarios de CE1 a CE3. Las citas de `ci.yml` y `cache.yml` siguen valiendo sobre `a7f7337`.
- **El código ya salió.** `git tag --contains ec0a3e7` da `v0.73.0`, así que la primera versión publicada es 0.73.0.
- **La versión sube a 0.74.0.** 0.73.0 ya tiene tag, así que no aplica la excepción de embedder-footprint y search-arm-timing. Subirla en `develop` no publica nada: publica el workflow de release cuando la CI de un push a `main` termina bien, si `v0.74.0` todavía no tiene tag y supera a la última versión publicada.
