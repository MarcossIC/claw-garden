# ci-hardening — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — (después del arreglo suelto de `rustls`, `rustls-webpki` y `h2`, `c50dff3`) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y citas en la [exploración](exploration.md#problema-verificado-contra-el-código), verificada contra `develop` (`a75f34b`).

1. **El CI y el release compilan cosas distintas.** `ci.yml` corre `clippy` y la suite sin `--locked` (`.github/workflows/ci.yml:53,56`) mientras `release.yml` construye con `--locked` (`release.yml:97`); `--workspace` une las features `testing` y `eval` de clawg-core, así que los nueve bloques `cfg(not(feature = "testing"))` y clawg-cli sin `testing` se compilan por primera vez en `release.yml`, después del merge a `main`; y los dos usan un `stable` flotante (`ci.yml:48`, `release.yml:90`). Un fallo de esos tres tipos frena el release sin haber puesto rojo ningún PR.
2. **Ningún paso mira los avisos de seguridad.** Corrido sobre el lock de `a75f34b`, `cargo deny check advisories` encuentra RUSTSEC-2026-0204 (`crossbeam-epoch` 0.9.18), RUSTSEC-2026-0097 (`rand` 0.9.2), RUSTSEC-2024-0436 (`paste`, sin mantenimiento) y `der` 0.8.0 retirado (yanked); nada en el repo los hubiera mostrado.
3. **El CI es la puerta del release y sus acciones no están fijadas.** `actions/checkout@v4`, `dtolnay/rust-toolchain@stable` y `Swatinem/rust-cache@v2` (`ci.yml:47-51`) siguen una etiqueta o una rama; `checkout` deja el `GITHUB_TOKEN` en `.git/config` todo el job; y cualquier corrida, la de un PR incluida, escribe la caché que leen después las de `main`. La prueba de pins sólo lee `release.yml` (`crates/clawg-cli/tests/conventions.rs:307-308`).
4. **`rust-version` miente.** `Cargo.toml:11` declara 1.77.2; el código usa `Option::is_none_or` (1.82) `ort`, `ort-sys`, `rmcp` e `ignore` piden 1.88 y `tree-sitter-language` 0.1.9, 1.90 (la exploración decía 1.88; la verificación compiló con 1.88.0 y 1.90.0, y sólo la segunda pasó).
5. **`install.ps1` es un asset firmado de cada release y nada lo analiza.** El CI ignora `scripts/**` (`ci.yml:14,24`).
6. **Un `cdn.pyke.io` que no contesta frena el release.** El build de `ort-sys` baja ONNX Runtime de ahí (con su SHA-256 fijado), y el paso de build de `release.yml:97` es un solo intento.

## Decisiones

Las abiertas de la exploración se respondieron en el hilo:

- **Fidelidad con el release (A).** `--locked` en los dos comandos de `ci.yml`, más dos pasos de producción en las dos plataformas: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` (sin `--all-targets`, así no entran las dev-dependencies ni `testing`) y `cargo test --locked -p clawg-core --lib build_info` (la variante `cfg(not(feature = "testing"))` de sus pruebas). Sin build release en cada PR (B).
- **Avisos: informativos, en un workflow aparte** (C). `.github/workflows/advisories.yml` corre `cargo deny --locked check advisories` con un `deny.toml`, en cada PR y push a `main` que cambia un manifiesto, el lock, `deny.toml` o el propio workflow, y cada semana por `schedule:` para que un aviso nuevo se vea sin esperar un cambio. No se llama `CI`, así que no es la puerta del release (`release.yml:8` espera `workflows: [CI]`) y un aviso nuevo no bloquea PR ni release. `cargo-deny` 0.20.2 se baja de su release y se compara con un SHA-256 fijado, como minisign en `release.yml:155-171`; ninguna acción de terceros la instala. Los avisos que hoy marca se resuelven antes con un `cargo update` compatible (`crossbeam-epoch`, `rand`, `der`); `paste` no tiene versión mantenida, sólo entra como proc-macro de `tokenizers` y queda en `ignore` con su motivo.
- **Pins de `ci.yml` (F).** Las tres acciones por SHA completo con la etiqueta en comentario, `persist-credentials: false` en cada `checkout` de cada workflow, y `save-if: ${{ github.ref == 'refs/heads/main' }}` en `rust-cache`: los PR leen la caché y sólo `main` la escribe. Se acepta que el primer PR después de un cambio de dependencias compile de cero.
- **Toolchain (G).** `rust-toolchain.toml` con `channel = "1.99.1"` (la stable del 2026-10-13) y `clippy`; los dos workflows pasan esa misma versión a `dtolnay/rust-toolchain`, y `rust-version` pasa a `"1.90"`. Subir el toolchain es un commit deliberado que cambia los tres lugares; una prueba los compara.
- **Dependabot (E), sólo `github-actions`, mensual, hacia `develop`.** Mantiene al día los SHA de las acciones; los avisos de crates los muestra el workflow de avisos.
- **`install.ps1` (I).** `.github/workflows/installer.yml` corre PSScriptAnalyzer 1.25.0 (bajado de su release y comparado con un SHA-256 fijado) sobre `scripts/` en `windows-2022`, cuando cambia `scripts/**` o el workflow. Las reglas que hoy dan falsos positivos se excluyen en `scripts/PSScriptAnalyzerSettings.psd1`, cada una con su motivo; cualquier otro hallazgo pone rojo el job. Tampoco se llama `CI`.
- **Reintento de ORT (J).** El paso de build de `release.yml` reintenta `cargo build` hasta tres veces con espera creciente.
- **Gate de rutas (H): fuera.** Sólo sirve si `main` exige checks, y eso no se ve desde el repo.

## Alcance

1. `cargo update` de `crossbeam-epoch`, `rand` y `der`.
2. `ci.yml`: `--locked` y los dos pasos de producción.
3. `ci.yml`: acciones por SHA, `persist-credentials: false`, `save-if`; la prueba de pins y la de `checkout` sobre todos los workflows.
4. `rust-toolchain.toml`, `rust-version = "1.90"` y la versión exacta en los dos workflows.
5. `deny.toml` y `advisories.yml`.
6. Reintento del build en `release.yml`.
7. `installer.yml` y `scripts/PSScriptAnalyzerSettings.psd1`.
8. `.github/dependabot.yml`.
9. `.claude/CLAUDE.md` (Testing y Versioning) y, al archivar, `specs/install` («CI») y `specs/release` (R3, R8).

## Fuera de alcance

- Los ajustes del repositorio en GitHub: checks requeridos, revisor del environment `release`, ruleset de `main`, protección de etiquetas. `gh secure` (la extensión de `gh` de GitHub Security Lab) los cubre en parte: protección de rama, reporte privado de vulnerabilidades, secret scanning con push protection, alertas de Dependabot y CodeQL. Cambia la configuración del repositorio, no un archivo, y pide permisos de administrador, así que lo corre quien administra el repositorio (`gh secure --dry-run` muestra qué cambiaría); las alertas de Dependabot complementan el workflow de avisos.
- `cargo fmt --check` en el CI. El código no sigue ningún estilo de `rustfmt`: con la configuración por defecto hay 3.882 diferencias, y con `max_width = 140` y `use_small_heuristics = "Max"`, 2.087. Sumarlo pide reformatear todo el workspace en un commit propio, que choca con cada rama abierta; es un cambio aparte.
- Un crate `xtask`: los workflows corren comandos de `cargo` sueltos, y no hay hoy una tarea repetida que lo justifique.
- El gate de rutas ignoradas (H).
- Quitar `-A clippy::all` y arreglar los lints propios de `clippy`.
- Vendorear ONNX Runtime.
- Las licencias, fuentes y duplicados de `cargo deny` (`licenses`, `sources`, `bans`): `deny.toml` queda listo para sumarlos.

## Datos guardados

No toca el store, el esquema ni ningún identificador. Cambia el lock (tres crates, versiones compatibles) y lo que corre en GitHub. Vuelta atrás: revertir los commits.

## Éxito

Tests que fallan hoy en `crates/clawg-cli/tests/conventions.rs` (detalle en la [spec](spec.md)):

- `ci.yml` corre `clippy` y la suite con `--locked`, y los pasos de producción de `-p clawg-cli` y `build_info`;
- toda acción de todo workflow está fijada por SHA con la etiqueta en comentario, todo `checkout` lleva `persist-credentials: false` y `rust-cache` lleva `save-if`;
- `rust-toolchain.toml` fija una versión exacta, cada `dtolnay/rust-toolchain` pasa la misma y `rust-version` está entre 1.90 y ella;
- `advisories.yml` y `installer.yml` existen, bajan su herramienta por versión y hash, no se llaman `CI` y no leen secretos; `deny.toml` existe;
- el build de `release.yml` se reintenta;
- `dependabot.yml` actualiza `github-actions` hacia `develop`.

Además, `cargo deny --locked check advisories` pasa sobre el lock nuevo, PSScriptAnalyzer no da hallazgos con la configuración, y la suite y `clippy` pasan con el toolchain fijado. Lo que sólo muestra GitHub (que `-p clawg-cli` compila sin `testing` en las dos plataformas, que la caché de un PR no escribe, que los workflows nuevos corren) se lee en el CI del PR. `rag-eval` no aplica.
