# clippy-enforced — Especificación

Delta sobre R6 de [`specs/quality`](../../../specs/quality/spec.md) y la sección CI de [`specs/install`](../../../specs/install/spec.md). Los lints de clippy pasan a regir el workspace desde una tabla del `Cargo.toml` raíz, y el comando de clippy, local y en el CI, deja de apagarlos. Ningún comando, ruta, herramienta ni dato cambia de conducta.

## CE1. Los paquetes toman los lints del workspace

El `Cargo.toml` raíz MUST declarar `[workspace.lints.clippy]` con:

- `all` en `warn`, con `priority = -1`, para que un lint suelto de la tabla pese más que el grupo;
- `large_futures`, `allow_attributes` y `allow_attributes_without_reason` en `warn`.

La tabla MUST NOT poner ningún lint en `allow`. Cada paquete del workspace (`[workspace] members`) MUST tener `[lints]` con `workspace = true`.

#### Escenario: un paquete sin la tabla
- GIVEN un miembro del workspace cuyo `Cargo.toml` no tiene `[lints]` con `workspace = true`
- WHEN corre la suite
- THEN el test falla y nombra el paquete

#### Escenario: una excepción del workspace
- GIVEN `[workspace.lints.clippy]` con un lint en `allow`
- WHEN corre la suite
- THEN el test falla y nombra el lint

**Test:** `every_package_takes_the_workspace_lints` (`crates/clawg-cli/tests/conventions.rs`). Falla hoy: no hay tabla.

## CE2. El CI corre clippy sin apagar lints

`ci.yml` MUST correr, con `--locked`:

- `cargo clippy --locked --workspace --all-targets -- -D warnings`;
- `cargo clippy --locked -p clawg-cli -- -D warnings`, lo que construye el release.

`cache.yml` MUST construir lo mismo (la regla de `the_cache_workflow_builds_from_main_what_ci_tests` no cambia). Ningún `cargo clippy` de un workflow MUST pasar `-A`, `--allow`, `-W`, `--warn` ni `--cap-lints`: qué lints corren lo dice la tabla de CE1, y el comando sólo hace fallar los warnings.

#### Escenario: un workflow que apaga lints
- GIVEN un workflow bajo `.github/workflows/` con `cargo clippy … -- -A clippy::all -D warnings`
- WHEN corre la suite
- THEN el test falla y nombra el workflow y el comando

#### Escenario: un lint nuevo en un pull request
- GIVEN un pull request que agrega `vec![1, 2, 3]` donde alcanza un arreglo (`clippy::useless_vec`)
- WHEN corre el CI
- THEN el paso de clippy falla en Linux y en Windows

**Tests:** `no_workflow_turns_clippy_lints_off` (nuevo; falla hoy); `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` y `the_ci_workflow_checks_the_locked_production_build`, que pasan a fijar los comandos nuevos (fallan hoy); `the_cache_workflow_builds_from_main_what_ci_tests`, que no cambia. El segundo escenario lo comprueba el CI, que la suite no ejecuta.

## CE3. Una excepción es un `#[expect]` con motivo, en el lugar

Un hallazgo que no se arregla MUST anotarse en el ítem con `#[expect(clippy::<lint>, reason = "<motivo>")]`. MUST NOT usarse `#[allow]` para un lint, ni un `#[expect]` sin `reason`. Un `#[expect]` cuyo lint deja de emitirse MUST hacer fallar el paso de clippy (`unfulfilled_lint_expectations`, que `-D warnings` vuelve error).

Sólo se anota lo que pide un refactor que tiene su propio cambio (`too_many_arguments`) o lo que el lint cuida y en ese lugar no pasa, y el motivo MUST decir cuál de las dos y por qué.

#### Escenario: un `allow`
- GIVEN `#[allow(clippy::too_many_arguments)]` sobre una función
- WHEN corre clippy
- THEN falla con `clippy::allow_attributes`

#### Escenario: una excepción que dejó de hacer falta
- GIVEN una función con `#[expect(clippy::too_many_arguments, reason = "…")]`
- WHEN un cambio la deja con siete parámetros o menos
- THEN clippy falla con `unfulfilled_lint_expectations` hasta que se quita el atributo

**Tests:** los comprueba clippy con los lints de CE1 (`allow_attributes`, `allow_attributes_without_reason`) en el paso de CE2; `every_package_takes_the_workspace_lints` fija que esos lints están en la tabla.

## CE4. Los docs dan el comando sin `-A clippy::all`

`.claude/CLAUDE.md` (sección Testing) y `docs/reference/testing.md` («Local checks» y «CI and release workflows») MUST dar el comando sin `-A clippy::all`:

```
cargo clippy --workspace --all-targets -- -D warnings
```

y `docs/reference/testing.md` MUST decir que la tabla `[workspace.lints.clippy]` del `Cargo.toml` raíz es la que dice qué lints corren, y que una excepción es un `#[expect]` con motivo. Reemplaza el comando y la nota «hasta que se arreglen los que hay» de R6 de `specs/quality`, y los dos comandos de clippy de la sección CI de `specs/install`.

**Test:** ninguno: ningún test lee un archivo de instrucciones para agentes (R6 de `specs/quality`). Se comprueba a mano.

## Invariantes

- La suite pasa entera antes y después.
- Lo que ve el modelo no cambia: `agent_contract.json` y `agent_contract.es.json` pasan sin bendecir.
- `memory.db`, `metrics.db`, los formatos de export y los mensajes de la CLI no cambian.
