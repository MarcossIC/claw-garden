# install-smoke — Verificación

## Linux

Sobre el último commit de la rama, una sola corrida (regla del proyecto):

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace`: **2.044 pasan, 0 fallan, 0 ignorados.** En `main` fallaban los 7 tests de `doctor`; ahora pasan. El cambio suma 16 tests: 9 en `binary.rs`, 2 en `conventions.rs`, 2 en `paths.rs` y 3 en `bootstrap.rs`.
- `cargo test -p clawg-cli --test binary` cinco veces seguidas: 9 de 9 en cada una, entre 0,8 y 1 segundo.
- Después de las corridas no queda ningún proceso `target/debug/clawg` vivo, y no existe `~/.clawg` ni `~/.clawg-dev`.

`rag-eval` no aplica: no cambia el ranking.

## CI

- [ ] El workflow en verde en `ubuntu-latest` y `windows-latest` en el PR.

## Windows (máquina de Marcos)

- [ ] `cargo test --workspace --target-dir <otro-dir>`: la base más 16 tests.
- [ ] `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- [ ] Ningún `clawg.exe` hijo queda vivo después de la suite, y el `clawg.exe` del editor sigue corriendo.

## Observación fuera de alcance

`cargo build -p clawg-cli` solo (sin la feature `eval` que suma el workspace) avisa que `ImportanceFactor::spanning` (`crates/clawg-core/src/memory/score.rs:88`) no se usa. Ya pasaba en `main`; no rompe el CI porque el workspace compila con `eval`.
