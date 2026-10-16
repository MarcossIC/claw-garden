# install-smoke — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas descartadas.

## D1. El aislamiento

**Elegido:** `CLAWG_TEST_HOME` reemplaza el home que devuelve `dirs::home_dir()` (`home_in`, `crates/clawg-core/src/paths.rs:174`). `data_dir` sigue siendo `<home>/<perfil>`.

| Alternativa | Por qué no |
|---|---|
| Una variable que reemplace el directorio de datos entero | Puede apuntar a `~/.clawg` desde un build debug, y entonces hace falta una regla nueva para rechazarlo. Reemplazando el home, el perfil sigue decidiendo y la regla de siempre alcanza |
| Redirigir `HOME`/`USERPROFILE` del hijo | En Linux funciona. En Windows, `dirs` 6 pide la carpeta del perfil al shell y no lee `USERPROFILE`: un test escribiría en el `~/.clawg-dev` real |
| Un flag `--data-dir` | Superficie de producto para algo que hoy sólo usa una prueba |

La lectura de la variable es un `home_dir()` con dos versiones por `cfg` (`paths.rs:182` y `:187`); la decisión es la función pura `home_in`, con sus tests. Una variable vacía cuenta como ausente, igual que `CLAWG_PROFILE`.

## D2. El embedder

**Elegido:** `TestEmbedder` (`crates/clawg-core/src/memory/bootstrap.rs:264`) arma el trío del bootstrap para un sustituto:

- `fake`: la «descarga» devuelve archivos vacíos al instante y la carga devuelve `FakeEmbedder`.
- `stalled`: la descarga no termina nunca, así que el daemon queda en `loading` y cada save responde `202`.

La verificación del contrato es la real en los dos: el store de prueba registra `fake-embedder` y lo verifica al reiniciar, que es el mismo camino que recorre el modelo real. `serve_deps` (`bootstrap.rs:318`) decide entre el sustituto y `production_deps`; `serve` lo llama en su lugar (`crates/clawg-cli/src/daemon/serve.rs:91`).

Un valor desconocido es un error al arrancar (`ServeError::Embedder`), no el modelo real: un typo en un test no debe disparar una descarga de 470 MB.

| Alternativa | Por qué no |
|---|---|
| El modelo real copiado a la caché del home temporal | 470 MB en cada máquina y en el CI, y una carga lenta, para probar supervivencia y no ranking |
| Las dos según una variable (la recomendación de la exploración) | El camino real lo recorre la verificación en Windows de cada cambio, con el modelo real en la máquina de Marcos (por ejemplo, la de ingest-outbox); sumar su caché al CI es costo sin una falla que hoy lo pida. Queda abierto |
| Un `202` con el modelo falso y un retardo | Una carrera contra el reloj; `stalled` da el `202` siempre |

## D3. Sólo en el binario de test

**Elegido:** las dos variables existen sólo con la feature `testing` de clawg-core. `clawg-cli` ya la enciende en sus `[dev-dependencies]`, y cargo compila con ella el binario que usan sus tests de integración (`CARGO_BIN_EXE_clawg`). `cargo build --release` no compila las dev-dependencies, así que el binario que se entrega no la tiene.

El riesgo es que alguien la encienda en `[dependencies]`. Lo cubre `only_dev_dependencies_turn_on_the_test_stand_ins` (`crates/clawg-cli/tests/conventions.rs`), que lee cada `Cargo.toml` del workspace.

Efecto secundario aceptado: después de un `cargo test`, `target/debug/clawg` es el binario de test. Lee las variables sólo si están puestas, y un build debug ya está limitado al perfil dev.

## D4. Las pruebas

`crates/clawg-cli/tests/binary.rs`:

- **Un home por test** (`Sandbox`), `--port 0` siempre y el puerto y el token leídos de `daemon.json` cuando nombra el pid del hijo. Los tests corren en paralelo sin chocar.
- **Guardas por handle.** `Daemon` y `Bridge` terminan su hijo al soltarse, también cuando el test entra en pánico. Nunca se mata por nombre. Un hilo cosecha el daemon apenas sale: en Unix un hijo sin cosechar queda zombi y `clawg stop`, que pregunta por el pid (`crates/clawg-cli/src/process.rs:98`), esperaría hasta su tope.
- **Cada línea del stdout del puente tiene que ser JSON-RPC.** Una escritura accidental a stdout falla el test con la línea a la vista.
- **El puente arranca en una carpeta vacía propia**, porque registra su raíz con `index_docs` y si no indexaría el crate.

| Alternativa | Por qué no |
|---|---|
| Un subcomando `clawg selftest` | Superficie de producto para una prueba (quality-guards, 32, ya lo descartó) |
| Scripts de PowerShell y sh | Dos implementaciones fuera de `cargo test`; sirven para el instalador, que queda fuera |

## D5. CI

`.github/workflows/ci.yml`: `ubuntu-latest` y `windows-latest`, `clippy` con `-D warnings` y `cargo test --workspace`, en cada PR y en cada push a `main`, con la caché de `Swatinem/rust-cache`. Sin el modelo real, el job no descarga nada aparte de lo que baja el build (`ort-download-binaries`).

Para que Linux arranque en verde, los tests de `doctor` arman sus rutas con la raíz de la plataforma (`C:\` o `/`) en lugar de literales de Windows. El caso de Windows (mayúsculas, barras y `\\?\`) sigue sólo en Windows.
