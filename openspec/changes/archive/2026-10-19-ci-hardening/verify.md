# ci-hardening — Verificación

## Linux, toolchain 1.99.1

Sobre el último commit del código, con `target` vaciado (`cargo clean`) al cambiar de toolchain:

- `cargo clippy --locked --workspace --all-targets -- -A clippy::all -D warnings`: sin advertencias. El compilador 1.99.1 no trae ningún warning nuevo sobre el código.
- `cargo test --locked --workspace`: pasa entero, `tests/binary.rs` incluido.
- `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`: sin advertencias. `cargo tree --locked -p clawg-core -e features -i clawg-core` muestra sólo `default`: ni `testing` ni `eval` entran en esa configuración.
- `cargo test --locked -p clawg-core --lib build_info`: pasan las diez pruebas, entre ellas la variante `cfg(not(feature = "testing"))` de `update_overrides_are_read_only_in_a_test_build` (`crates/clawg-core/src/build_info.rs:295`), que hasta ahora sólo se había corrido a mano.
- `the_agent_contract_matches_its_file` no se movió: el cambio no toca lo que ve el modelo.

Cada prueba de convención nueva o ajustada falló antes de su cambio, por la aserción (`--locked`, los pasos de producción, el `save-if`, los pins y el `persist-credentials` de `ci.yml`, el reintento del build) o porque el archivo todavía no existía (`rust-toolchain.toml`, `advisories.yml`, `installer.yml`, `dependabot.yml`).

## El mínimo de Rust

La exploración decía 1.88, a partir de `ort`, `ort-sys`, `rmcp` e `ignore`. Compilado de verdad:

| Toolchain | `cargo +<v> check --locked --workspace --all-targets` |
|---|---|
| 1.88.0 | falla antes de compilar: `tree-sitter-language@0.1.9 requires rustc 1.90` |
| 1.90.0 | falla en `golden-mine` (`crates/clawg-eval/src/bin/golden-mine/synth/mod.rs:513`): 1.90 infiere `str` como elemento de `pick` a partir del `&str` de `fill` |
| 1.90.0, con el arreglo | compila |

`cargo metadata` confirma que 1.90 (`tree-sitter-language` 0.1.9) es el mayor `rust-version` del lock; le siguen `rmcp-macros`, `darling` y `time`, con 1.88. Se corrigió en dos commits: `092d26a` (`*rng.pick(...)`, como las otras llamadas del mismo bucle) y `0536952` (`rust-version = "1.90"` y la constante de la prueba). El 1.88.0 se desinstaló después; el 1.90.0 queda sólo para repetir esta comprobación.

## Avisos

`cargo-deny` 0.20.2, bajado de la misma URL que usa el workflow; su SHA-256 coincide con el `.sha256` que publica el release.

| Lock | `cargo deny --locked check advisories` |
|---|---|
| `a75f34b` (antes) | `FAILED`: RUSTSEC-2026-0204 (`crossbeam-epoch` 0.9.18), RUSTSEC-2026-0097 (`rand` 0.9.2), RUSTSEC-2024-0436 (`paste`, sin mantenimiento) y `der` 0.8.0 retirado (yanked, sólo advertencia) |
| después de `ca1359a`, sin `deny.toml` | `FAILED`: sólo `paste` |
| con `deny.toml` | `advisories ok` |

`paste` llega por `tokenizers` y `macro_rules_attribute`, y es una proc-macro: corre al compilar y nada de su código queda en el binario.

## `install.ps1`

PowerShell 7.6.7 en Linux, con PSScriptAnalyzer 1.25.0 bajado del release que usa el workflow (SHA-256 `14e634c8…`) y los mismos pasos del workflow (comparar el hash, `Expand-Archive`, `Import-Module` por ruta):

- Sin configuración, diez hallazgos, todos advertencias: cinco `PSAvoidUsingWriteHost` (los mensajes al usuario), tres `PSReviewUnusedParameter` (los parámetros se leen dentro del bloque `& { }`, que la regla no sigue) y dos `PSUseShouldProcessForStateChangingFunctions` (funciones internas, no cmdlets). Ninguno es un defecto del script.
- Con `scripts/PSScriptAnalyzerSettings.psd1`, que excluye esas tres reglas: cero hallazgos.
- Un script de prueba con un alias y una variable sin usar da dos hallazgos con la misma configuración: las demás reglas siguen activas.

## CI del PR

Pendiente: se completa con la corrida de GitHub sobre el PR.
