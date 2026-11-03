# cli-dispatch — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, sobre `2aed17a`, el último commit de código:

- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también. El `#[expect(clippy::too_many_arguments)]` de `import::command` se cumple.
- `cargo test --workspace --no-fail-fast` pasa. Contra la base (`a7f7337`, corrida antes del primer commit), los mismos ignorados; lo que se suma son los tests de la [spec](spec.md) y `a_command_speaks_english_without_a_readable_preference` (`preferences.rs`).
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: lo que ve el modelo no cambió.
- Windows queda para el CI del pull request, que compila el código `cfg(windows)`: `user_path::command`, que reemplaza a `run_path` de `main.rs`.

## `tests/binary.rs`

`git diff a7f7337 -- crates/clawg-cli/tests/binary.rs` sale vacío, y el archivo pasa entero, también en cada commit que tocó un comando que corre.

## Los tests en rojo

| Test | Antes | Después |
|---|---|---|
| `status_without_a_daemon_reports_the_store_and_exits_1`, `status_with_a_daemon_reports_its_health_and_exits_0` | no compilan: `cannot find function command` en `status.rs` | pasan |
| Los de CD1, CD2 y la tabla de CD4 | no compilan: la función o el tipo que llaman no existe (`CliError`, `Running`, `stop::command`, `backup::command`, `ProjectsAction::profile`…) | pasan |

## El binario de antes y el de después

El binario debug de `a7f7337` y el de `2aed17a`, los dos con la feature `testing` como los construye la suite, copiados a la misma ruta (así `doctor` nombra el mismo ejecutable), corren los mismos 244 casos en un home temporal (`CLAWG_TEST_HOME`), con el embedder falso y el daemon en su socket:

- **Sin daemon**, con cuatro estados de la preferencia (sin archivo, `es`, `en` y un `preferences.json` que no es JSON): `status`, `stop`, `backup`, `restore`, `export` con y sin `--project`, `import` y `import --undo`, `secrets` con y sin `--mask-reasons` y `--yes`, `purge` de una memoria y de un borrado con y sin `--yes`, `undelete`, `outbox retry|discard`, `sharing` (ver, ocultar, mostrar), `preferences`, `metrics report` con `--events` y con `--metric`, las nueve acciones de `projects`, las cuatro de `memories`, `doctor`, `path add|remove`, `hook session-start`, `setup claude-code --dry-run`, `--profile prod` en debug, `--profile bogus` y `CLAWG_PROFILE=bogus`.
- **Con un daemon** de cada binario, en `es` y sin preferencia: `status`, `memories list|stats`, `projects` (listar, renombrar, crear dos veces, borrar, agregar una carpeta que existe y una que no, ubicar, unir, mantener separados), `undelete`, `outbox`, `import` (un archivo que no existe, un `--undo` que no existe), `secrets`, `purge` y `restore` negados por el lock del daemon, `backup`, `export` (con `--project`, en markdown, y uno que ya existe), `sharing` y `stop` dos veces.

Sólo se normalizan la hora de cada línea de log, los números de cuatro cifras o más (pids, puertos, tiempos), la hora del nombre de un backup y el sufijo al azar del id de un proyecto creado.

**Resultado:** stdout, stderr y el código de salida son idénticos en los 244 casos. Con el `preferences.json` inválido, la línea de `language_in` en stderr sale en los mismos casos que antes: sí en `export --out`, que lee el idioma antes de fallar sin store; no en `export --project`, que falla al resolver el proyecto, ni en `secrets`, que falla al leer el store.

## El spec contra el código

| Requisito | Dónde |
|---|---|
| CD1 | `crates/clawg-cli/src/error.rs`; `dispatch` devuelve `Result<ExitCode, CliError>` (`crates/clawg-cli/src/main.rs:118`), y `main` imprime `clawg: {error}` y sale con 1 (`:76-85`). `CliError` no implementa `From<String>`. |
| CD2 | `stop::explain` (`crates/clawg-cli/src/stop.rs`), `Running::find` y `Running::unavailable` (`crates/clawg-cli/src/running.rs`); ni `main.rs` ni `dispatch` componen una frase. |
| CD3 | `status::command` (`crates/clawg-cli/src/status.rs`). |
| CD4 | `backup::command`, `restore::command`, `export::command`, `import::command`, `secrets::command`, `secrets::purge`, `metrics::command`, `projects::command` con `ProjectsAction::profile`, `user_path::command`. |

`dispatch` pasó de 445 a 172 líneas y `main.rs` de 612 a 289; `.map_err(|e| e.to_string())` ya no aparece en `main.rs`.

## Lo que queda

- El CI del pull request en `windows-latest`.
- Archivar y subir la versión, fuera de este cambio.
