# doctor-checks — Verificación

## Linux (2026-10-20)

Sobre `be1235d` (el código no cambia después de `f87836f`), en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.
- Cada commit de código de las [tareas](tasks.md) (`2d520a3`, `9bf8bcd`, `f87836f`) compiló y pasó clippy y los tests de lo que tocaba (`inspect::`, `doctor::`, los de `doctor` en `binary.rs`) antes de commitearse; la suite completa corrió sobre el último.
- TDD: los tests de las tareas 1 y 2 se escribieron junto con su código y no se vieron fallar antes. El de la tarea 3 sí falló primero, y encontró que con el modelo falso `doctor` comparaba el contrato con el de producción y no con el que verifica el daemon de ese build.

## El binario real

Con `target/debug/clawg` de un `cargo build`, `HOME` en un directorio temporal del scratchpad y `CLAUDE_CONFIG_DIR` vacío: sin directorio de datos, `clawg doctor` imprime las claves de antes más `data_dir`, `checks` y `summary`; `data_dir` y `store.schema` dicen `missing`, `mcp` dice `not_registered` con `clawg setup claude-code --profile dev`, y `socket` da `error` `path_unusable` porque la ruta del temporal tenía 121 bytes, así que sale 1. Con daemon y store, lo cubren `doctor_reads_a_running_daemons_store_without_changing_it` y `doctor_prints_no_row_content` (`crates/clawg-cli/tests/binary.rs`), con el modelo falso.

## Pendiente

- El CI del pull request, en Linux y Windows.
- Ranking: el cambio no toca retrieval, no se corre `rag-eval`.
