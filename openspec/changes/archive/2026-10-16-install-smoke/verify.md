# install-smoke — Verificación

## Linux

Sobre el último commit de la rama, una sola corrida (regla del proyecto):

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace`: **2.044 pasan, 0 fallan, 0 ignorados.** En `main` fallaban los 7 tests de `doctor`; ahora pasan. El cambio suma 16 tests: 9 en `binary.rs`, 2 en `conventions.rs`, 2 en `paths.rs` y 3 en `bootstrap.rs`.
- `cargo test -p clawg-cli --test binary` cinco veces seguidas: 9 de 9 en cada una, entre 0,8 y 1 segundo.
- Después de las corridas no queda ningún proceso `target/debug/clawg` vivo, y no existe `~/.clawg` ni `~/.clawg-dev`.

`rag-eval` no aplica: no cambia el ranking.

## CI

- [x] El workflow en verde en `ubuntu-latest` (2 min) y `windows-latest` (7,5 min) en el PR #32, sobre `e134a3c`: clippy y la suite completa, con las pruebas del binario incluidas.

## Windows, con el modelo real

Sobre `84ee27b` (`main`, el merge del PR #32, versión 0.11.0), el 2026-10-16:

- [x] `cargo test --workspace --target-dir target-test`: **2.050 pasan, 0 fallan, 0 ignorados**, que son la base de 2.034 más los 16 de este cambio. La compilación no da ningún warning.
- [x] `binary.rs`: 9 de 9 en 1,39 s.
- [x] `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- [x] Antes y después de la suite, los únicos `clawg.exe` vivos son los dos del editor (`clawg mcp --profile prod` y su `clawg serve --profile prod`, los dos de `%LOCALAPPDATA%\Programs\clawg`), con los mismos pid. No queda ningún hijo vivo.
- [x] La suite corrió de 06:52:03 a 06:53:15, y el archivo más nuevo de `~/.clawg-dev` es del 2026-10-15 a las 23:32. La suite no tocó el perfil dev real.

## Al archivar

Los requisitos pasan a [`specs/install`](../../../specs/install/spec.md), con cada `ruta:línea` verificada contra `84ee27b`. Diferencias entre los docs del cambio y el código:

1. R2 pide que el error de un valor desconocido nombre la variable y los valores aceptados. El mensaje nombra los dos (`crates/clawg-core/src/memory/bootstrap.rs:279`), pero `the_test_embedder_is_named_by_its_variable_or_absent` sólo comprueba la variable y `"fake"`.
2. Que `serve` no arranque con un valor desconocido (`ServeError::Embedder`, `crates/clawg-cli/src/daemon/serve.rs:91-92`) no tiene test a través del binario. Lo cubre la función pura `TestEmbedder::parse`.
3. Las dos variables no tratan igual un valor de sólo espacios. `TestEmbedder::parse` recorta (`bootstrap.rs:274`), así que `" "` cuenta como ausente. `home_in` sólo descarta el vacío (`crates/clawg-core/src/paths.rs:176`), así que `CLAWG_TEST_HOME=" "` es un home llamado `" "`. R1 dice «no vacío», y eso es lo que hace el código.
4. R5 prohíbe terminar procesos por nombre y no tiene test. Se cumple por construcción: fuera de los directorios `target*`, el repo no tiene ningún `taskkill`, `pkill`, `killall` ni `Stop-Process`.
5. R6 habla de «los 7 tests de `doctor`», que eran los que fallaban en Linux en `main`. `doctor.rs` tiene 9: 8 corren en todas las plataformas y `windows_spellings_of_one_file_are_the_same_install` (`crates/clawg-cli/src/doctor.rs:217`) corre sólo en Windows. El spec consolidado dice qué hacen las fixtures en lugar de contar tests.
6. Este cambio e ingest-outbox (36) se escribieron en ramas separadas del PR #32. El archivo de 36 verificó sus citas sin el código de este cambio, y este cambio corrió líneas de `bootstrap.rs`, `serve.rs`, `paths.rs` y `.claude/CLAUDE.md`. Se corrigieron:
   - `specs/ingest`: `run_embedder_bootstrap_with_retry` de `bootstrap.rs:464` a `:550`, y el lanzamiento en `serve.rs` de `:89` a `:93`.
   - `specs/storage`: el `shutdown` del daemon de `serve.rs:152` a `:156`; el `spawn_blocking` de la carga de `bootstrap.rs:367` a `:453`; `verify_or_reembed` de `:269` a `:355` e `is_pooling_upgrade` de `:255` a `:341`. El cableado ahora pasa por `serve_deps` (`bootstrap.rs:318`, llamado en `serve.rs:91`).
   - `specs/sharing`: `sharing_file` de `paths.rs:218` a `:246`.
   - `specs/quality`: la línea `MCP tools` de `.claude/CLAUDE.md:54` a `:57`, el comando de `clippy` de `:93` a `:97` y la bendición del contrato de `:98` a `:105`.
   - El backlog de `openspec/README.md`: el `spawn_blocking` de `bootstrap.rs:367` a `:453`, y `#[tokio::main]` de `main.rs:34` a `:37`.
7. `specs/storage` (lo pendiente al arrancar) y `specs/mcp` (el reinicio con el puente abierto) suman los tests de `binary.rs` que prueban lo mismo con procesos de verdad. `specs/storage` (directorio de datos) remite a `specs/install` para el home de prueba.

La base de tests pasa a 2.050 en `.claude/CLAUDE.md` y `openspec/config.yaml`.

## Observación fuera de alcance

`cargo build -p clawg-cli` solo (sin la feature `eval` que suma el workspace) avisa que `ImportanceFactor::spanning` (`crates/clawg-core/src/memory/score.rs:88`) no se usa. Ya pasaba en `main`; no rompe el CI porque el workspace compila con `eval`.
