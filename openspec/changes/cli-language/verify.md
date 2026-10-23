# cli-language — Verificación

## Linux (2026-10-23)

En `target`, con el compilador de `rust-toolchain.toml`. ONNX Runtime 1.24.2, la versión que baja `ort-sys`, se enlazó de forma dinámica (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`) porque el entorno no llegaba a `cdn.pyke.io`; el código de clawg no cambia por eso.

- `cargo test --workspace`: pasa, ningún test falla. Corrió antes del primer cambio, como base, y al final, con agent-language encima.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` y `cargo clippy -p clawg-cli -- -A clippy::all -D warnings`: salen sin un aviso.
- Cada commit corrió los tests del crate que tocaba (`clawg-cli --lib`, `--test binary`, `clawg-core --lib sharing`) y clippy del workspace.

## RED antes de cada cambio

- **Se vieron fallar antes del código:** ninguno por la aserción; los tests nuevos piden un `Lang` en funciones que no lo recibían, así que no compilaban.
- **Se escribieron antes que el código y no compilaban contra el anterior:** los de `update/report.rs`, `setup.rs` (`the_setup_report_speaks_spanish`), `secrets.rs`, `export.rs`, `import.rs`, `restore.rs`, `undelete.rs`, `outbox.rs`, `projects.rs`, `sharing.rs`, `metrics.rs`, `memories.rs` y `doctor/mcp.rs` (`the_mcp_findings_are_the_same_in_both_languages_but_their_message`).
- **Se escribieron junto con el código:** los dos de `doctor/checks.rs` (`the_findings_are_the_same_in_both_languages_but_their_message`, `a_finding_in_spanish_agrees_with_its_number`), después de pasar `Lang` a cada chequeo. El de paridad de `mcp.rs` encontró un defecto antes de llegar al commit: la `evidence` de `config_unreadable` llevaba la causa traducida; ahora la causa queda en inglés y sólo cambia el `message`.
- **Los tests de antes** pasan con `Lang::En` sin cambiar el texto que esperan.

## El binario real

Con `CLAWG_TEST_HOME` en un directorio temporal (`crates/clawg-cli/tests/binary.rs`):

- `doctor_speaks_the_preferred_language`: con `en` y con `es`, los hallazgos son iguales salvo `message`, el código de salida es el mismo, y `store.schema` y `preferences` dicen su frase en cada idioma.
- `stop_speaks_the_preferred_language`: con `es`, `clawg stop` dice `se detuvo el daemon dev (pid N)`, y sin daemon la negativa sale en español con código 1; con un `preferences.json` roto la misma negativa sale en inglés con el mismo código.
- `doctor_names_a_broken_preferences_file`, de 78, sigue pasando con sus mensajes en inglés.

## Lo que no se verificó

- Windows: lo corre el CI del pull request. `clawg path` es de Windows y queda en inglés (no tiene perfil).
- A mano, sobre el perfil `dev`, con autorización: `clawg preferences set language es --profile dev` y después `clawg doctor`, `clawg update --check` y `clawg setup claude-code --dry-run`.
