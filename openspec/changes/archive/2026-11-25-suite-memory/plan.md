# suite-memory — Plan

Corre en: local (Windows, para medir) · Necesita a la persona: para aprobar el plan

## Problema

En Windows, `cargo test --workspace --target-dir target-test` con los jobs por defecto agota la memoria de una máquina de 16 núcleos y 32 GB: los rustc salen con `memory allocation of … failed` o `0xc0000409`, y link.exe da `LNK1201`. Con `-j 4` y unos 10 GB libres, Claude Code cortó la compilación dos veces por falta de memoria del sistema.

Medido con un muestreador del working set de `rustc.exe`, `link.exe`, `rust-lld.exe` y `cl.exe` una vez por segundo, sobre `cargo test --workspace --no-run --target-dir target-test` después de `cargo clean --workspace`, con las dependencias ya compiladas:

| Variante | Jobs | Pico de un rustc | Pico de un enlace | Pico de la suma | Tiempo |
|---|---|---|---|---|---|
| hoy (link.exe) | 1 | 2,77 GB | 1,66 GB | — | 342 s |
| `CARGO_INCREMENTAL=0` | 1 | 2,64 GB | 1,67 GB | — | 306 s |
| `profile.test.package."*".debug = false` | 1 | 2,76 GB | 1,60 GB | — | 375 s |
| linker `rust-lld` | 1 | 2,78 GB | 1,05 GB | — | 550 s ¹ |
| hoy (link.exe) | 4 | 2,68 GB | 1,65 GB | 7,7 GB | 188 s |
| linker `rust-lld` | 4 | 2,66 GB | 1,29 GB | 5,9 GB | 192 s |

¹ Esa corrida recompiló el código C de las dependencias (aparece `cl.exe`); con 4 jobs, la misma variante tarda lo mismo que hoy.

Lo que lo causa no es un proceso: cada rustc y cada enlace de un binario de test pesa entre 1 y 2,8 GB, y cargo corre por defecto tantos como núcleos haya:

- Ningún archivo de configuración fija los jobs: no hay `.cargo/config.toml`, y los perfiles del `Cargo.toml` raíz sólo bajan los símbolos de test a `line-tables-only` (`Cargo.toml:89-90`).
- `cargo test --workspace` enlaza unos 30 binarios, y cada uno arrastra el árbol entero (ort, tokenizers, sqlite):
  - por cada test de integración, un binario: 6 en `crates/clawg-cli/tests/`, 5 en `crates/clawg-core/tests/` y 3 en `crates/clawg-eval/tests/`;
  - además, la librería y el ejecutable de cada crate, y los 7 bins de `clawg-eval` (`crates/clawg-eval/Cargo.toml:20-72`), cada uno como ejecutable y como harness de sus tests unitarios.
- CI compila con 4 jobs, porque sus máquinas tienen 4 núcleos: `cargo test --locked --workspace` (`.github/workflows/ci.yml:68`) y `cargo test --locked --workspace --no-run` (`.github/workflows/cache.yml:54`).

## Objetivo y fuera de alcance

Que la suite compile y corra entera, con el comando de `.claude/CLAUDE.md` y sin `-j`, en una máquina de desarrollo de 16 núcleos que tiene otras aplicaciones abiertas. Sirve a todos los objetivos del producto, porque ningún cambio se cierra sin la suite.

Qué cambia:

1. En Windows se enlaza con `rust-lld`, el enlazador que trae el toolchain: los tests y también el release.
2. Cada crate tiene un solo binario de tests de integración, `integration`. Los archivos de `tests/` no se mueven: pasan a ser módulos de `tests/integration.rs`. `e5_golden` queda en su propio binario porque necesita la feature `eval` (`crates/clawg-core/Cargo.toml:68-70`). Pasan de 14 binarios a 4.
3. `.cargo/config.toml` fija un tope de jobs. Los workflows de CI lo levantan con `CARGO_BUILD_JOBS: default`: en una máquina de 4 núcleos, un `jobs = 8` fijo correría 8 procesos, no 4.

Fuera de alcance:

- los 7 bins de `clawg-eval` y sus harnesses;
- la compilación incremental y los símbolos de las dependencias, que la medición descartó;
- el perfil de release, salvo el enlazador;
- partir `clawg-cli` en crates más chicos.

## Hecho

- H1. Dado el repositorio, cuando corre la convención, entonces `.cargo/config.toml` MUST decir `linker = "rust-lld"` en `[target.x86_64-pc-windows-msvc]`. → `conventions::windows_links_with_rust_lld`
- H2. Dado un crate del workspace con una carpeta `tests/`, cuando corre la convención, entonces su manifiesto MUST decir `autotests = false` en `[package]`. → `conventions::each_crate_builds_one_integration_binary`
- H3. Dado un `.rs` en la raíz de `tests/` de un crate, cuando corre la convención, entonces MUST ser `integration.rs`, o un `mod` que `integration.rs` declara, o la ruta de un `[[test]]` propio del manifiesto. → `conventions::each_crate_builds_one_integration_binary` y su par puro `test_files_outside_the_integration_binary_names_each_one`
- H4. Dado `.cargo/config.toml`, cuando corre la convención, entonces `[build]` MUST fijar `jobs` con un entero positivo. → `conventions::the_build_caps_its_jobs`
- H5. Dado un workflow de `.github/workflows/` que corre `cargo`, cuando corre la convención, entonces su `env` MUST decir `CARGO_BUILD_JOBS: default`. → `conventions::ci_lifts_the_jobs_cap`
- H6. Los tests que corren antes de juntar los binarios MUST seguir corriendo. Se cuenta con `cargo test --workspace -- --list` sobre el commit anterior a juntarlos: después, la cantidad MUST ser la misma más las convenciones nuevas, y cada nombre de un test de integración MUST reaparecer con su módulo como prefijo. Ningún test se ignora ni se borra. → la comparación de `Verificación`
- H7. Dado el binario de release de Windows enlazado con `rust-lld`, cuando corre el paso de humo de `release.yml` (`--version` y el grep de `CLAWG_TEST_`), entonces MUST pasar. Se comprueba en local con `cargo build --release -p clawg-cli --target-dir target-test`, sólo con autorización (`.claude/CLAUDE.md` prohíbe los builds de release sin ella); si no hay autorización, queda para el primer release.
- No cambia nada fuera de: `crates/clawg-cli/tests/conventions.rs` (los tests de H1 a H5), `.cargo/config.toml`, los `Cargo.toml` de los tres crates, los tres `tests/integration.rs` nuevos, las líneas `--test X` de `crates/clawg-cli/tests/{transport_bar,perf_bench,import_memory}.rs`, los `env` de `ci.yml`, `cache.yml` y `release.yml`, `docs/reference/testing.md` y `openspec/specs/{quality,eval,daemon-access}/spec.md`.
- No se afloja ni se borra ningún test existente.

## Medición

Métrica: el pico de la suma del working set de `rustc.exe`, `link.exe`, `rust-lld.exe` y `cl.exe` durante `cargo test --workspace --no-run --target-dir target-test`, sin `-j` (con el tope de `.cargo/config.toml`), después de `cargo clean --workspace --target-dir target-test`, con las dependencias compiladas. El muestreador es el de la tabla del Problema.

Línea base: 7,7 GB y 188 s con 4 jobs. Con 16 jobs, la compilación se corta: no hay número.

Umbral, fijado antes de medir: el pico MUST quedar en 8 GB o menos, y el tiempo MUST quedar en 188 s más un 10 % (207 s) o menos. El tope es `jobs = 8`. Si con 8 el pico supera 8 GB, se baja a 6; si con 6 también lo supera, el cambio no sale y vuelve a la persona.

Además, `cargo test --workspace --target-dir target-test`, sin `-j`, MUST terminar entero y sin que lo corten.

## Decisión

Se eligió combinar tres medidas: `rust-lld` en todo Windows, un binario de integración por crate y un tope de jobs que CI levanta.

| Alternativa | Por qué no |
|---|---|
| `rust-lld` sólo en local, con una variable de entorno que no se commitea | El repositorio no gana nada para quien lo clona, y CI y el release siguen con link.exe |
| No usar `rust-lld` | El enlace es la palanca que más baja por proceso: -37 % con 1 job, y -24 % en la suma con 4 |
| Juntar los tests sin tope de jobs | Con 16 núcleos, 16 rustc de hasta 2,7 GB siguen pasando la memoria libre |
| Sólo el tope de jobs | No reduce trabajo, sólo lo reparte en el tiempo |
| `CARGO_INCREMENTAL=0` en local | -5 % en el pico de rustc, a cambio de perder la recompilación rápida del loop TDD |
| Las dependencias sin símbolos | Los PDB bajan a menos de la mitad, pero el enlace sólo baja un 4 % y se pierden las líneas de un backtrace en una dependencia |
| Mover los archivos a `tests/integration/` | Cambiaría las rutas relativas de `include_str!("../src/daemon/api.rs")` (`crates/clawg-cli/tests/api.rs:5787`) y las rutas citadas en las specs. Con `autotests = false` y un `tests/integration.rs` en la raíz, cada `mod api;` resuelve a `tests/api.rs`, que queda donde está |

Riesgos y cómo se cubren:

- Un `#![cfg(windows)]` o un `#![cfg(feature = "testing")]` al principio de un archivo (`transport_bar.rs:11`, `shutdown.rs:4`) se vuelve el atributo interno de un módulo, y sigue quitándolo entero. El clippy de Linux y el de Windows de CI lo comprueban.
- Los archivos pasan a compartir un proceso. Hoy su único estado global son un `OnceCell` y un `OnceLock` de módulo (`api.rs:26`, `shutdown.rs:17`, `agent_eval.rs:32`), y ningún test cambia variables de entorno ni el directorio actual: se verificó con `rg "set_var|remove_var|set_current_dir" crates/*/tests`. Los tests de archivos distintos ahora pueden correr a la vez; H6 y una segunda corrida de la suite muestran si alguno se vuelve inestable.
- Cambian los nombres de los filtros, por ejemplo de `--test api` a `--test integration api::`. Las referencias vivas son las de H6 y no tocan `openspec/changes/archive/`, que es historia.

## Pasos

1. `windows_links_with_rust_lld`, que falla porque no hay `.cargo/config.toml` → crear `.cargo/config.toml` con el enlazador. Commit `build: link with rust-lld on windows`.
2. `the_build_caps_its_jobs` y `ci_lifts_the_jobs_cap`, que fallan → agregar `[build] jobs = 8` y `CARGO_BUILD_JOBS: default` en el `env` de `ci.yml`, `cache.yml` y `release.yml`. Commit `build: cap local build jobs and lift the cap in ci`.
3. `each_crate_builds_one_integration_binary` y su par puro, que fallan → `autotests = false`, un `[[test]] name = "integration"` y `tests/integration.rs` en cada crate. Se actualizan las líneas `--test X` de los tres archivos de test. Commit `test: build one integration test binary per crate`.
4. Los docs y la spec: `docs/reference/testing.md` dice el binario, los filtros, el enlazador y el tope; `openspec/specs/quality/spec.md` suma los requisitos de H1 a H5; `eval/spec.md:1232` y `daemon-access/spec.md:223` usan el nombre nuevo del filtro. Commit `docs: describe the integration binary, the linker and the jobs cap`.
5. La medición de arriba y, sobre el último commit, la suite completa y los dos clippy. Los números van a `Verificación`.

## Registro

- 2026-11-25: la persona aprobó el plan y autorizó el build de release local de H7.
- Pasos 1 a 4 en `8cb3c29b`, `bfd5794c`, `83383cf4` y `87286326`, sin desvíos de código.
- H6 decía que la cantidad de tests MUST ser la misma, pero el cambio suma convenciones. La base de la comparación se tomó en `bfd5794c`, antes de juntar los binarios. Las dos revisiones lo marcaron, y el criterio se reescribió para decir lo que se compara.
- La lista de «No cambia nada fuera de» no nombraba `crates/clawg-cli/tests/conventions.rs`, donde viven los tests de H1 a H5. Se agregó.
- Ronda 1 de la revisión, W2: `ci_lifts_the_jobs_cap` buscaba el texto en todo el workflow, así que pasaba con la variable en un comentario o en un solo job de dos. Se rehízo por job, con el par puro `jobs_running_cargo_without_the_jobs_cap_reads_each_job`, en `63024078`. Las citas de R8 y las de R7, que ya estaban desactualizadas, se corrigieron en `d9d73c89`.

## Verificación

En Windows, x86_64, con 16 núcleos lógicos y 32 GB, y con entre 11 y 13 GB libres al empezar cada corrida.

**Medición**, en `87286326`. Se corrió `cargo clean --workspace --target-dir target-test` y después `cargo test --workspace --no-run --target-dir target-test`, sin `-j`, con el muestreador del Problema:

- pico de la suma: 7,3 GB, contra un umbral de 8 GB;
- tiempo: 92 s, contra un umbral de 207 s;
- 8 rustc a la vez, el mayor de 2,9 GB, y 2 `rust-lld` a la vez, el mayor de 1,2 GB.

Queda `jobs = 8`. La línea base era de 7,7 GB y 188 s con 4 jobs.

**Suite**, `cargo test --workspace --target-dir target-test` sin `-j`:

| Commit | Correctos | Fallidos | Ignorados | Tiempo |
|---|---|---|---|---|
| `87286326` | 4786 | 0 | 14 | 187 s |
| `d9d73c89` | 4787 | 0 | 14 | 180 s |

Las dos corridas terminaron sin cortes, y ningún test se volvió inestable al compartir proceso.

**Clippy**: `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings` y `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings` no dan hallazgos, en los dos commits.

**H6**: con `-- --list --format terse`, el workspace lista 4798 tests en `bfd5794c` y 4800 en `83383cf4`; la diferencia son las dos convenciones nuevas.
- Los nombres de los tests de integración, listados por binario en `bfd5794c` con `--features testing` en clawg-core, reaparecen todos en los tres binarios `integration` con su módulo como prefijo.
- `public_api` muestra 5 tests más en ese listado porque el de antes se hizo sin `testing`; en el workspace esa feature está encendida.
- `#[ignore]`: 14 antes y 14 después.

**H7**, en `87286326`:
- `cargo build --release -p clawg-cli --target-dir target-test` compiló con `rust-lld` en 3 min 4 s;
- `target-test/release/clawg.exe --version` dio `clawg 0.94.2`;
- `rg -a CLAWG_TEST_` no encontró nada en el binario.

**Revisión, ronda 1**: `change-reviewer` y `clawg review` (sesión `a2a91939-342b-45b9-9fa3-23418871b241`, gpt-5.3-codex-high), los dos con BLOCKED. Cada hallazgo y lo que se hizo:

- H7 sin evidencia: el build terminó después de pedir la revisión, y su resultado está arriba.
- H6 contaba «la misma cantidad», que el propio cambio no podía cumplir: se reescribió y se anotó su base (`Registro`).
- `conventions.rs` faltaba en la lista de alcance: se agregó.
- W2, el chequeo del tope en CI, que era por archivo: ahora es por job, en `63024078`.
- W4, el link de R8 a este plan: apunta a la ruta archivada desde el commit que archiva.
- Las citas desactualizadas de R7, anteriores al cambio: se corrigieron en `d9d73c89`.
- Que el pico que mide `import_memory` puede moverse ahora que el proceso es el binario `integration` entero: queda anotado. Sus mediciones archivadas fueron con el binario solo.
- La caché fría en el primer CI después del merge, porque cambia la clave de rust-cache: se acepta. `ci.yml` y `cache.yml` siguen con la misma clave.

**Revisión, ronda 2**, sobre `63024078`, `d9d73c89` y las secciones nuevas de este plan: `clawg review` con CLEAN, retomando la misma sesión, y `change-reviewer` con WARNINGS.
- Lo único abierto era W4, el link de R8, que se corrigió en el commit que archiva.
- Se aceptan dos sugerencias que ningún workflow de hoy alcanza:
  - el chequeo por job acepta `CARGO_BUILD_JOBS: default` también en el `env` de un paso;
  - un job se reconoce sólo con una indentación de dos espacios.

Lo pendiente que sólo CI puede comprobar: clippy y la suite en Linux, donde `transport_bar` queda vacío por su `#![cfg(windows)]` de módulo.
