# ci-hardening — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — (la subida de `Cargo.lock` es un arreglo suelto previo; ver «Interacciones») | exploración (borrador) |

**Tesis:** el CI que da paso a cada release revisa la configuración que se publica, con el `Cargo.lock` y el toolchain que usa el release, avisa cuando una dependencia recibe un aviso de seguridad, y no se puede poner en verde ni envenenar con una etiqueta movida o una caché escribible. Hoy el CI compila y prueba otra cosa que el release (`testing` encendido, lock reescribible, `stable` flotante), no mira ningún aviso de seguridad, y usa acciones por rama o etiqueta mientras el release las fija por commit. Nada de esto rompe hoy; lo que se arriesga es un release que falla después del merge a `main`, o un CI verde que no dice nada de lo que se publica.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El CI no usa `--locked` y el release sí | **Cierta.** `ci.yml:53` (`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`) y `:56` (`cargo test --workspace`); `release.yml:97` (`cargo build --release --locked -p clawg-cli ...`). Si `Cargo.lock` quedó atrás de `Cargo.toml` (el riesgo del bump que describe `.claude/CLAUDE.md`, Versioning), el CI lo reescribe en silencio y pasa; el release falla después del merge. |
| El CI nunca compila la configuración de producción | **Cierta.** `cargo tree --offline --workspace -e features -i clawg-core` da `default`, `eval` (de clawg-eval) y `testing` (dev-dependency de clawg-cli, `crates/clawg-cli/Cargo.toml:72-73`); `cargo tree --offline -p clawg-cli -e normal,features -i clawg-core` da sólo `default`, que es lo que arma `release.yml`. Hay 9 bloques `#[cfg(not(feature = "testing"))]`: 7 en `build_info.rs` (`:19,33,58,70,125,137,295`), 1 en `paths.rs:188` y 1 en `memory/bootstrap.rs:367`. Es el W3 que dejó abierto release-and-self-update (`openspec/changes/archive/2026-10-18-release-and-self-update/verify.md:64`): «Un error que sólo exista en esa configuración pasaría el CI y frenaría el release, sin publicar nada». |
| `-p clawg-cli` sin `--all-targets` no trae `testing` | **Cierta por construcción, no ejecutada.** Con `resolver = "2"` (`Cargo.toml:2`) las dev-dependencies sólo entran al grafo cuando se compilan tests, ejemplos o benches; el segundo `cargo tree` (normal, sin dev) lo muestra. Costo: se recompilan sólo `clawg-core` y `clawg-cli` con otro conjunto de features; las dependencias externas no cambian de grafo, así que la caché las sirve. Hay que confirmarlo en una corrida real. |
| La variante de producción de `update_overrides_are_read_only_in_a_test_build` nunca corre en el CI | **Cierta.** Está bajo `#[cfg(not(feature = "testing"))]` en `build_info.rs:295`; `verify.md:15` la corrió a mano con `cargo test -p clawg-core --lib build_info`. |
| Hay avisos de seguridad sobre el lock actual | **Cierta en lo que alcancé a ver.** `Cargo.lock` fija `rustls 0.23.37`, `rustls-webpki 0.103.10` y `h2 0.4.13`, los tres en el grafo de producción (`cargo tree --offline -p clawg-cli -e normal --target all -i <crate>`: cuelgan de `reqwest 0.12.28`, `hyper-rustls` y `axum 0.8.9`). En la base de RustSec están RUSTSEC-2026-0349 (rustls, corregido en `>=0.23.46`, https://rustsec.org/advisories/RUSTSEC-2026-0349.html), RUSTSEC-2026-0258 (h2, `>=0.4.16`, https://rustsec.org/advisories/RUSTSEC-2026-0258.html) y RUSTSEC-2026-0098 (rustls-webpki, `>=0.103.12`, https://rustsec.org/advisories/RUSTSEC-2026-0098.html); los demás (-0285, -0099, -0104) no se abrieron uno por uno: los lista la herramienta al implementar. |
| No hay `deny.toml`, `rust-toolchain.toml` ni `dependabot.yml` | **Cierta.** `.github/` sólo contiene `workflows/` con `ci.yml` y `release.yml`; en la raíz no hay `deny.toml` ni `rust-toolchain*`. |
| `ci.yml` usa acciones sin fijar y `release.yml` las fija | **Cierta.** `ci.yml:47-51`: `actions/checkout@v4` (sin `persist-credentials: false`), `dtolnay/rust-toolchain@stable` (una rama), `Swatinem/rust-cache@v2`. `release.yml` fija por SHA con la etiqueta en un comentario (`:34,84,88,134,159,163,199`). |
| Una convención ya fija las acciones | **Sólo del release.** `third_party_actions_are_pinned_to_a_commit` (`crates/clawg-cli/tests/conventions.rs:307`) lee únicamente `release.yml` (`:308`). Ninguna prueba mira las acciones de `ci.yml`. |
| `rust-version = "1.77.2"` es el mínimo real | **Falsa.** `Cargo.toml:11` lo declara, pero `Option::is_none_or` (estable desde 1.82, https://doc.rust-lang.org/std/option/enum.Option.html#method.is_none_or) se usa en `crates/clawg-cli/src/setup.rs:255` y `crates/clawg-cli/src/daemon/guard.rs:72`, y las versiones del lock de `ort 2.0.0-rc.12`, `ort-sys 2.0.0-rc.12`, `rmcp 3.5.1` e `ignore 0.4.32` declaran `rust-version = "1.88"` (en `~/.cargo/registry/src/*/<crate>/Cargo.toml:14`). El mínimo efectivo es 1.88, no 1.77.2. |
| El CI y el release usan un toolchain flotante | **Cierta.** `ci.yml:48` (`@stable`) y `release.yml:88-90` (`toolchain: stable`); el release sólo registra la versión en el log con `rustc -Vv` (`:91`). Con `-D warnings` un lint nuevo del compilador puede poner el CI en rojo sin que el repo cambie. |
| El CI ignora `scripts/**` aunque `install.ps1` se publica | **Cierta.** `ci.yml:14` y `:24`; el release copia `scripts/install.ps1` al conjunto firmado (`release.yml:177`). `openspec/specs/release/spec.md:138` (R8) dice que el script «no tiene tests automáticos hoy». |
| `main` exige checks para fusionar | **No verificable desde el repo.** Son ajustes de GitHub. Si exige el check `test (...)`, un cambio que sólo toca rutas ignoradas lo deja pendiente: «Associated checks stay in a "Pending" state and block merging» (https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/collaborating-on-repositories-with-code-quality-features/troubleshooting-required-status-checks, «Handling skipped but required checks»). Decisión abierta. |
| La descarga de `cdn.pyke.io` está protegida | **Cierta en la integridad, no en la disponibilidad.** `Cargo.toml:43` pide `download-binaries`; `ort-sys-2.0.0-rc.12/build/download/dist.txt` trae una URL de `cdn.pyke.io` y un SHA-256 por plataforma, y `verify.rs` los compara. Si el CDN no contesta, el build falla; el CI de prueba también depende de él. |

## Problema (verificado contra el código)

1. **El CI y el release compilan cosas distintas.** Lock reescribible (`ci.yml:53,56` sin `--locked`), features con `testing` (premisa 2) y `stable` flotante en los dos (`ci.yml:48`, `release.yml:90`) sin versión fija. Un fallo de esos tres tipos aparece recién en `release.yml`, después del merge, y el release no se publica.
2. **Ningún paso mira avisos de seguridad.** Con `rustls`, `rustls-webpki` y `h2` en el camino de producción (el daemon escucha por `axum` y el cliente descarga el modelo y las actualizaciones por `reqwest`), nada avisa cuando uno recibe un aviso; el release publica el lock que haya.
3. **El CI es la puerta del release, así que su integridad importa aunque no lea secretos.** `release.yml:25-28` exige `workflow_run.conclusion == 'success'` de CI para un push a `main`. `ci.yml:28-29` sólo da `contents: read` y no lee secretos (la prueba `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` lo exige, `conventions.rs:554-556`), pero: una etiqueta movida de `actions/checkout@v4`, `Swatinem/rust-cache@v2` o de la rama `dtolnay/rust-toolchain@stable` puede devolver un verde cualquiera; el `build.rs` de una dependencia puede escribir la caché de Actions que leen corridas posteriores en `main`; y sin `persist-credentials: false` el `GITHUB_TOKEN` queda en `.git/config` durante todo el job.
4. **El rango de versiones del compilador no está dicho.** El `rust-version` declarado miente (premisa 9), y el hallazgo «un ítem más nuevo que el MSRV» del backlog «Hallazgos de `clippy`» de `openspec/README.md` no puede resolverse hasta que haya un mínimo real.
5. **`install.ps1` es un asset firmado de cada release y no pasa por ningún análisis.** El CI lo ignora (`ci.yml:14,24`) y su spec lo declara sin pruebas automáticas.

## Lo que ya existe y se reutiliza

- Las convenciones de workflow de `conventions.rs`: `workflow_code` (`:208`), `indent_of` (`:221`), `job` (`:302`), `third_party_actions_are_pinned_to_a_commit` (`:307`) y `ignored_paths` (`:519`) se extienden a `ci.yml` sin parser YAML nuevo.
- El patrón de pins de `release.yml` (SHA completo más `# vX.Y.Z`), y el job `plan` que ya lee la versión con `cargo metadata`.
- `Cargo.toml:2` ya usa `resolver = "2"`: la separación de dev-dependencies es la del propio cargo.
- `dist.txt`/`verify.rs` de `ort-sys` ya fijan el hash del binario de ONNX Runtime: lo que falta es sólo reintentar.

## Enfoques posibles

### Fidelidad con el release

**A. Pasos de producción y `--locked` en el CI (recomendado).** `--locked` en los dos comandos existentes y un paso `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` (sin `--all-targets`, sin `testing`), más `cargo test --locked -p clawg-core --lib build_info` para la variante `cfg(not(feature = "testing"))` (`build_info.rs:295`). Se ejecuta en las dos plataformas.
- **A favor:** cierra W3 con dos pasos; mismo grafo que el release; sin dependencias nuevas.
- **En contra:** unos minutos más por plataforma (dos crates recompilados). `--locked` hace que un lock atrasado ponga el CI en rojo, que es lo buscado, pero cambia el flujo de quien olvide `cargo update --workspace`.

**B. Un job que replique `release.yml` sin publicar (`cargo build --release --locked -p clawg-cli`).** Máxima fidelidad (también `ldd` y el humo).
- **A favor:** detecta incluso fallos de enlazado.
- **En contra:** un build release por plataforma en cada PR; el repo no corre builds release en local y el job de release ya los hace. Sobra frente a A.

### Avisos de seguridad

**C. `cargo deny check advisories` como job propio, con `deny.toml` (recomendado).** Además de un `schedule:` semanal para que un aviso se vea sin esperar a un push. Instalación fijada: una acción por SHA, o `cargo install --locked cargo-deny@<versión exacta>` (más lento; con caché).
- **A favor:** un solo archivo de política; `ignore` con motivo escrito; cubre después licencias y fuentes si se quiere.
- **En contra:** una herramienta más que mantener fijada; un aviso nuevo pone rojo un CI sin que el repo cambie (si es requerido bloquea los PR, si no, avisa). Decisión abierta.

**D. `cargo audit`.** Más simple, sin `deny.toml`.
- **A favor:** menos configuración.
- **En contra:** sólo avisos, sin política de fuentes ni licencias; `ignore` va en otro archivo (`.cargo/audit.toml`).

**E. Dependabot (`cargo` y `github-actions`) (opcional, sumable a C).** Un PR que sólo cambia `Cargo.lock` no sube versión, así que no publica nada, pero el próximo release lo lleva. Sus PR van a la rama por defecto (`main`) salvo `target-branch`, y el repo trabaja en `develop` y fusiona a `main` (ramas observadas; el flujo no está escrito en `README.md` ni en `.claude/CLAUDE.md`, `rg develop` no lo encuentra).
- **A favor:** mantiene al día los pins por SHA de las acciones.
- **En contra:** ruido de PR; hay que fijar `target-branch: develop` y decidir quién los revisa.

### Cadena de suministro del CI

**F. Fijar `ci.yml` como `release.yml` (recomendado).** SHA completo con etiqueta en comentario para las tres acciones, `persist-credentials: false` en `checkout`, `save-if: ${{ github.ref == 'refs/heads/main' }}` en `rust-cache` (los PR leen la caché, sólo `main` la escribe) y una prueba que extienda `third_party_actions_are_pinned_to_a_commit` a los dos workflows.
- **A favor:** misma regla en los dos archivos; la prueba impide volver atrás.
- **En contra:** con `save-if` los PR no refrescan la caché, así que tras un cambio de dependencias el primer PR compila de cero; los pins hay que subirlos a mano o con E.

### Toolchain y MSRV

**G. `rust-version = "1.88"` y `rust-toolchain.toml` con versión exacta (recomendado).** El archivo lo leen el CI y el release; subirlo es un commit deliberado. `dtolnay/rust-toolchain` con `toolchain: stable` (`release.yml:90`) instala `stable`, pero rustup da precedencia al archivo del directorio: hay que pasar la misma versión exacta al input (o dejar la acción sin input), y una prueba de convenciones compara los tres lugares. Cómo interactúa la acción con el archivo no lo probé: se prueba en una corrida real.
- **A favor:** un clippy o rustc nuevo no pone rojo el CI sin cambio del repo; el release queda reproducible; el MSRV deja de mentir.
- **En contra:** hay que subir la versión a propósito (cada tantas semanas); el binario local pasa a usar ese toolchain (rustup lo instala solo).

### Gate de rutas ignoradas

**H. Job que siempre corre y filtra por dentro.** Un `gate` sin `paths-ignore` que decide con `dorny/paths-filter` (o `git diff`) y reemplaza el check requerido.
- **A favor:** resuelve «Pending» si `main` exige checks.
- **En contra:** reescribe los triggers que `the_ci_workflow_ignores_only_non_code_paths_...` fija y que `openspec/specs/install/spec.md` («CI», `:127`) describe; sólo vale si `main` exige checks.

**I. PSScriptAnalyzer sobre `scripts/install.ps1` en un job aparte en `windows-latest`.** Corre sólo cuando cambia `scripts/**` (el `paths` inverso al de `ci.yml`).
- **A favor:** da una verificación estática a un asset firmado.
- **En contra:** no ejecuta el script; R8 pide prueba manual en Windows limpia. Es un cambio aparte del resto.

### Descarga de ORT

**J. Reintentar el paso de build (`release.yml:97`) tres veces con espera.** Un `cdn.pyke.io` caído no frena un release.
- **A favor:** local, sin infraestructura.
- **En contra:** un bucle de shell en un paso que hoy es un solo comando; la prueba `the_release_workflow_builds_only_the_cli_with_the_dist_mark` (`conventions.rs:363`) lo lee y hay que ajustarla.

## Descartado

| Opción | Razón |
|---|---|
| Vendorear el archivo de ONNX Runtime | Sube cientos de MB al repo y reemplaza un hash ya fijado en `dist.txt`; se vigila, no se cambia. |
| `cargo audit` y `cargo deny` a la vez | Misma base de avisos (RustSec); dos herramientas, un solo resultado. |
| Hacer el job de avisos requerido sin decidirlo | Un aviso nuevo bloquearía todos los PR; ver «Decisiones abiertas». |
| Un build release en cada PR (B) | Costo alto y el release ya lo ejecuta. |
| Sustituir `rust-cache` por nada | Sin caché el CI de Windows se vuelve lento; `save-if` cierra el riesgo. |

## Interacciones

- **Subida de `Cargo.lock` (rustls, rustls-webpki, h2).** Es un arreglo suelto previo, ya hecho (`c50dff3`, en 0.27.5); este cambio es el proceso para que el aviso se vea la próxima vez. Con C activo, ese arreglo debe estar hecho antes o `deny.toml` arranca con `ignore`.
- **Backlog «Hallazgos de `clippy`» de `openspec/README.md`:** G da el MSRV real para el ítem «un ítem más nuevo que el MSRV» y, cuando esos lints se arreglen, el comando de `ci.yml:53` pierde `-A clippy::all`; este cambio no lo quita.
- **release-and-self-update (47, archivado).** Cierra su W3. Modifica `openspec/specs/install/spec.md` (sección «CI», `:127-141`: comandos, acciones, escenarios y la prueba que fija `ci.yml`) y `openspec/specs/release/spec.md` (R4 sobre CI como único disparador, y R8 si se agrega I).
- **`.claude/CLAUDE.md` (Testing, `:111-113`)** describe el CI y el release; se actualiza en el mismo commit que cualquier cambio de workflow, y `conventions.rs` lee los dos workflows como texto, así que workflow y prueba cambian juntos.
- **Releases y versión.** Un push a `main` que sólo cambia `ci.yml` o `deny.toml` corre CI y no publica nada (sin bump no hay etiqueta nueva).

## Alcance candidato

1. `--locked` y los pasos de producción (A) en `ci.yml`, con su prueba en `conventions.rs`.
2. Job de avisos con política (C), `schedule:` semanal y versión de la herramienta fijada.
3. Acciones de `ci.yml` por SHA, `persist-credentials: false`, `save-if` (F), y la prueba de pins ampliada a los dos workflows.
4. `rust-version` real y `rust-toolchain.toml` leído por CI y release (G), con prueba de que coinciden.
5. Reintento del build por la descarga de ORT (J).
6. Specs y `.claude/CLAUDE.md` actualizados en el mismo commit.

### No-alcance

- La subida de `rustls`, `rustls-webpki` y `h2` en `Cargo.lock`: arreglo suelto previo, ya hecho (`c50dff3`).
- Ajustes del repositorio en GitHub (revisor obligatorio en el environment `release`, ruleset en `main`, protección de etiquetas `v*`): son configuración fuera del repo y decisión aparte; hoy el environment está «sin revisores obligatorios» (`openspec/specs/release/spec.md:92`).
- Quitar `-A clippy::all` y arreglar los lints.
- Gate de rutas (H) e `install.ps1` (I), salvo decisión abajo.
- Vendorear ORT.

## Decisiones abiertas

Son del usuario.

- [ ] **Job de avisos:** ¿requerido (bloquea PR) o informativo? ¿`cargo deny` o `cargo audit`? ¿Cómo se instala fijado: acción por SHA o `cargo install --locked <crate>@<versión>`?
- [ ] **Dependabot (E):** ¿sí o no? Si sí, ¿`target-branch: develop`? ¿cada cuánto?
- [ ] **Toolchain (G):** ¿qué versión exacta (≥ 1.88) y cada cuánto se sube? ¿`rust-version` pasa a 1.88?
- [ ] **Checks requeridos en `main`:** ¿existen? De eso depende si H entra.
- [ ] **`install.ps1` (I):** ¿PSScriptAnalyzer ahora, en un cambio aparte o nunca?
- [ ] **`save-if` en la caché (F):** ¿se acepta que el primer PR tras un cambio de dependencias compile de cero?

## Cómo se mide

Tests RED primero (`crates/clawg-cli/tests/conventions.rs`, que hoy fallan):

1. `ci.yml` usa `--locked` en `clippy` y `test` y tiene el paso de producción `-p clawg-cli` sin `--all-targets` (hoy `ci.yml:53,56` no lo tienen).
2. Todas las acciones de **los dos** workflows están fijadas por SHA con la etiqueta en comentario (hoy `ci.yml:47,48,51` no).
3. `checkout` de `ci.yml` lleva `persist-credentials: false` y `rust-cache` lleva `save-if`.
4. `rust-toolchain.toml` existe con una versión exacta, `release.yml` y `ci.yml` pasan la misma, y `Cargo.toml` declara un `rust-version` no menor que el mayor de sus dependencias (1.88, premisa 9) ni mayor que la versión del archivo. Lo primero no lo puede leer una prueba sin el registro de cargo: queda como constante fijada en la prueba, que se revisa al subir una dependencia.
5. `deny.toml` existe y el workflow lo corre con la herramienta fijada y un `schedule:`.
6. La prueba existente `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` (`:540`) se ajusta a los pasos nuevos y sigue exigiendo que `ci.yml` no use `secrets.`, `environment:`, `id-token` ni `contents: write` (`:554-556`).

Sólo una corrida real en GitHub lo muestra: que `-p clawg-cli` no arrastra `testing` y compila en las dos plataformas; que el job de avisos pasa y falla cuando corresponde (con un `ignore` de prueba); que `dtolnay/rust-toolchain` respeta el archivo; que la caché de un PR no escribe; que el reintento de ORT corta ante un fallo de red. Los tests y `clippy` locales en Windows corren en `target-test` (`cargo test --workspace --target-dir target-test`); no se hacen builds release en local ni se crea tag o release a mano.

## Riesgos

- **Un aviso nuevo pone rojo el CI sin cambio del repo** (si el job es requerido). Mitigación: `ignore` con motivo y fecha, o job informativo.
- **`rust-toolchain.toml` mal alineado con la acción** deja a CI y release en versiones distintas, el mismo problema que se quiere cerrar; la prueba de coincidencia lo vigila.
- **`--locked` rompe el flujo de quien olvide `cargo update --workspace`** tras un bump (el CI se lo dice antes del merge, que es la intención).
- **Las pruebas leen los workflows como texto** (`conventions.rs:202-204`): reformatear sin cambiar el sentido las rompe; se ajustan en el mismo commit.
- **Pins por SHA envejecen** sin Dependabot o recordatorio.
