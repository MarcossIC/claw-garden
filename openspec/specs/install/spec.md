# install — Especificación

Fuente de verdad de cómo la suite prueba el binario `clawg` mismo: el build de test y las dos variables que sólo él lee (el home de prueba y el embedder de prueba), la regla que las mantiene fuera del binario que se entrega, las pruebas que lanzan el binario como proceso hijo, y el CI. Nada de esto cambia el binario que se entrega. El directorio de datos, el lock y el outbox están en [`specs/storage`](../storage/spec.md); el puente y su reconexión, en [`specs/mcp`](../mcp/spec.md).

**Origen:** [install-smoke](../../changes/archive/2026-10-16-install-smoke/proposal.md), archivado el 2026-10-16.

## Definiciones

- **Feature `testing`:** la feature de clawg-core que compila los sustitutos de prueba (`crates/clawg-core/Cargo.toml:25`).
- **Build de test:** un binario compilado con `testing`. Es el que cargo arma para los tests de integración de clawg-cli (`CARGO_BIN_EXE_clawg`, `crates/clawg-cli/tests/binary.rs:23`), porque clawg-cli enciende la feature en sus `[dev-dependencies]` (`crates/clawg-cli/Cargo.toml:68`). `cargo build --release` no compila las dev-dependencies, así que el binario que se entrega no la tiene. Efecto aceptado: después de un `cargo test`, `target/debug/clawg` es un build de test. Sólo lee las variables si están puestas, y como build debug ya está limitado al perfil dev.
- **Home de prueba:** el valor no vacío de `CLAWG_TEST_HOME` (`TEST_HOME_VAR`, `crates/clawg-core/src/paths.rs:169`).
- **Embedder de prueba:** el valor de `CLAWG_TEST_EMBEDDER` (`TEST_EMBEDDER_VAR`, `crates/clawg-core/src/memory/bootstrap.rs:260`), `fake` o `stalled` (`TestEmbedder`, `bootstrap.rs:265`).

## El home de prueba

En un build de test, el home MUST ser el home de prueba cuando `CLAWG_TEST_HOME` no está vacío, y el del sistema (`dirs::home_dir()`) cuando falta o está vacío. El directorio de datos sigue siendo `<home>/<perfil>` (`data_dir_in`, `paths.rs:159`). Un build sin `testing` MUST NOT leer la variable. `home_dir` tiene dos versiones según `cfg` (`paths.rs:182` y `paths.rs:187`), y la decisión es la función pura `home_in` (`paths.rs:174`). Un valor de sólo espacios no cuenta como vacío.

La resolución del perfil MUST NOT cambiar: un build debug rechaza `prod` también con un home de prueba.

Se reemplaza el home y no el directorio de datos para que el perfil siga eligiendo `.clawg-dev` o `.clawg` debajo. Así la regla de que un build debug nunca abre `.clawg` alcanza sin un caso nuevo. Redirigir `HOME` o `USERPROFILE` del hijo no sirve: en Windows, `dirs` pide la carpeta del perfil al shell.

#### Escenario: el home de prueba gana
- GIVEN `CLAWG_TEST_HOME=/tmp/smoke`
- WHEN un build de test resuelve el perfil dev
- THEN el directorio de datos es `/tmp/smoke/.clawg-dev`

#### Escenario: debug sigue sin prod
- GIVEN un build debug de test con un home de prueba
- WHEN corre `clawg status --profile prod`
- THEN sale con 1, el error dice que un build debug no abre prod, y no existe `<home>/.clawg`

**Tests:** `a_test_home_replaces_the_system_home` y `an_unset_or_empty_test_home_keeps_the_system_home` (`paths.rs`, mod tests); `a_debug_binary_refuses_prod_even_with_a_test_home` (`binary.rs:443`).

## El embedder de prueba

`clawg serve` arma el bootstrap del embedder con `serve_deps` (`bootstrap.rs:319`, llamado en `crates/clawg-cli/src/daemon/serve.rs:94`). En un build de test, según el embedder de prueba:

- con `fake`, MUST cargar `FakeEmbedder` sin descargar nada: la «descarga» devuelve archivos vacíos al instante y la carga devuelve el falso, con `model_id` `fake-embedder`;
- con `stalled`, la descarga MUST NOT terminar nunca, así que el embedder queda en `loading` y cada save responde `202`;
- sin la variable, o con un valor vacío o de sólo espacios, MUST usar el modelo real (`production_deps`, `bootstrap.rs:232`);
- con cualquier otro valor, MUST negarse a arrancar con un error que nombra la variable y los valores aceptados (`TestEmbedder::parse`, `bootstrap.rs:274`; `ServeError::Embedder`, `serve.rs:95`). Un typo en un test no puede disparar la descarga de 470 MB.

Un build sin `testing` MUST usar siempre el modelo real (`serve_deps`, `bootstrap.rs:332`).

En los dos sustitutos, la verificación del contrato de embedding MUST ser la real: `TestEmbedder::deps` (`bootstrap.rs:287`) usa la de `production_deps`. Un store de prueba registra el contrato de `fake-embedder` y lo verifica al reiniciar, por el mismo camino que recorre el modelo real.

#### Escenario: fake
- WHEN el bootstrap corre con `fake`
- THEN no hay descarga, el modelo es `fake-embedder` y un store nuevo registra su contrato

#### Escenario: valor desconocido
- WHEN la variable vale `real`
- THEN el error nombra `CLAWG_TEST_EMBEDDER` y los valores aceptados

**Tests:** `the_test_embedder_is_named_by_its_variable_or_absent`, `the_fake_test_embedder_loads_at_once_and_records_its_contract` y `the_stalled_test_embedder_never_finishes_the_download` (`bootstrap.rs`, mod tests). Del error de un valor desconocido, el test comprueba la variable y `"fake"`. Que `serve` no arranque con ese valor no tiene test a través del binario.

## Sólo las dev-dependencies encienden `testing`

Ningún `Cargo.toml` del workspace (el raíz y el de cada crate bajo `crates/`) MUST encender la feature `testing` de clawg-core fuera de una sección cuyo nombre termine en `dev-dependencies]`, también la de un target. Cuenta tanto un `features = ["testing"]` como un `clawg-core/testing` o un `default = ["testing"]` en una tabla de features. Hoy la enciende sólo `crates/clawg-cli/Cargo.toml:68`.

El test MUST fallar con `manifiesto:línea` de cada hallazgo. Reconocerlos es la función pura `testing_outside_dev_dependencies` (`crates/clawg-cli/tests/conventions.rs:87`), que examina línea por línea, ignora lo que sigue a un `#`, y no cuenta la declaración `testing = []` porque no lleva comillas.

**Tests:** `only_dev_dependencies_turn_on_the_test_stand_ins` (`conventions.rs:122`) y el puro `testing_outside_dev_dependencies_reads_the_section` (`conventions.rs:105`).

## El binario sobrevive a lo que sólo ve un proceso

`crates/clawg-cli/tests/binary.rs` MUST lanzar el build de test como proceso hijo y probar los escenarios de abajo. Cada prueba MUST correr en su propio home de prueba (`Sandbox`, `binary.rs:32`), con `fake` salvo que el escenario pida otro y con `--port 0`. El puerto y el token se leen de `daemon.json` y `token` cuando `daemon.json` nombra el pid del hijo (`published`, `binary.rs:269`), así que las pruebas corren en paralelo sin chocar. Un puente MUST arrancar en una carpeta vacía propia (`Sandbox::bridge`, `binary.rs:93`), porque registra su raíz con `index_docs` y si no, indexaría lo que haya. Cada línea del stdout de un puente MUST ser un mensaje JSON-RPC: otra cosa falla la prueba con la línea a la vista (`Bridge::reply`, `binary.rs:296`).

#### Escenario: kill duro
- GIVEN un daemon con `stalled` que respondió `202` a un save
- WHEN el proceso se mata sin apagado (SIGKILL en Unix, `TerminateProcess` en Windows) y arranca otro con `fake`
- THEN el save queda guardado y la búsqueda lo encuentra

#### Escenario: dos daemons
- GIVEN un daemon corriendo
- WHEN arranca un segundo `serve` sobre el mismo directorio de datos
- THEN el segundo sale con 1, su error nombra el pid del primero y el primero sigue respondiendo

#### Escenario: status y stop
- GIVEN un home de prueba sin daemon
- THEN `status` sale con 1
- WHEN arranca un daemon
- THEN `status` sale con 0 e informa el directorio de datos del home de prueba
- WHEN corre `stop`
- THEN sale con 0 y nombra el pid, el proceso termina limpio, no queda `daemon.json`, y `status` vuelve a salir con 1

#### Escenario: backup y restore
- GIVEN un save, un `backup` y otro save
- WHEN se intenta `restore` con el daemon corriendo
- THEN sale con 1 y el error dice `clawg stop`
- WHEN se detiene el daemon, se restaura y se arranca otra vez
- THEN la salida de `restore` nombra el snapshot `pre-restore`, y está la fila de antes del backup y no la de después

#### Escenario: MCP por stdio real con `initialize`
- GIVEN `clawg mcp --no-autostart` con un daemon corriendo
- WHEN el host manda `initialize` con la versión 2025-06-18
- THEN `serverInfo` es `clawg-dev` con la versión del crate, `tools/list` da 12 herramientas, `memory_status` responde, y un `save_memory` se encuentra con `search_memory`

#### Escenario: MCP por stdio real sin `initialize`
- GIVEN `clawg mcp --no-autostart` con un daemon corriendo
- WHEN el host manda cada pedido con el `_meta` de 2026-07-28 y nunca `initialize`
- THEN `server/discover` lista 2026-07-28 y 2025-06-18, `tools/list` da 12 herramientas y `memory_status` responde

#### Escenario: reinicio con el puente abierto
- GIVEN un puente que ya respondió una herramienta
- WHEN el daemon se detiene con `clawg stop` y arranca otro, con otro token
- THEN la herramienta siguiente responde sin error

**Tests:** `a_save_accepted_before_a_hard_kill_is_stored_after_the_restart` (`binary.rs:372`), `a_second_daemon_on_the_same_data_directory_is_refused_and_names_the_first` (`binary.rs:402`), `status_and_stop_follow_the_daemon_process` (`binary.rs:418`), `a_restored_backup_brings_back_its_rows_and_keeps_the_replaced_store` (`binary.rs:454`), `a_bridge_on_real_stdio_answers_initialize_and_the_tools_reach_the_daemon` (`binary.rs:491`), `a_bridge_on_real_stdio_serves_a_host_that_never_initializes` (`binary.rs:515`) y `a_bridge_left_open_reaches_the_daemon_after_a_restart` (`binary.rs:535`).

**Límite conocido:** el autoarranque del puente no se prueba a través del binario. El daemon que lanza el puente usa el puerto fijo del perfil (`spawn_serve_at` sin puerto, `crates/clawg-cli/src/client.rs:1008`; `serve_args`, `:1014-1021`), así que en la máquina del usuario chocaría con su daemon dev. Lo cubren los tests en proceso de `crates/clawg-cli/src/mcp/mod.rs`. Probarlo a través del binario pide antes poder elegir ese puerto, y eso es superficie de producto.

## Ningún hijo sobrevive a su prueba

Cada proceso que lanza la suite MUST terminarse por su propio handle cuando su prueba termina, también si la prueba entra en pánico. `Daemon` y `Bridge` matan y esperan a su hijo al soltarse (`binary.rs:262` y `binary.rs:338`). Un hilo cosecha cada daemon apenas sale (`Daemon::watch`, `binary.rs:149`): en Unix, un hijo sin cosechar queda zombi, y `clawg stop`, que pregunta por el pid, esperaría hasta su tope.

La suite MUST NOT terminar procesos por nombre: un `clawg` que corre el editor del usuario tiene que sobrevivirla.

#### Escenario: prueba que falla a la mitad
- GIVEN una prueba que entra en pánico con su daemon corriendo
- THEN otro daemon puede tomar el mismo directorio de datos enseguida

**Test:** `a_test_that_fails_midway_leaves_no_daemon_holding_the_store` (`binary.rs:554`). Que no se termine nada por nombre no tiene test: se cumple por construcción, porque el repo no tiene ningún `taskkill`, `pkill`, `killall` ni `Stop-Process`.

## CI

Cada pull request y cada push a `main` MUST correr, en `ubuntu-latest` y en `windows-latest` (`.github/workflows/ci.yml:3-6` y `ci.yml:21`), el comando de `clippy` que hace fallar los warnings del compilador y `cargo test --workspace`, con las pruebas del binario incluidas (`ci.yml:33` y `ci.yml:36`). Las dos plataformas MUST correr hasta el final aunque una falle (`fail-fast: false`, `ci.yml:19`). El job no descarga el modelo: la suite usa `fake`.

La suite MUST pasar en Linux y en Windows. Para eso, las fixtures de los tests de `doctor` arman sus rutas bajo la raíz de la plataforma, `C:\` o `/` (`dir`, `crates/clawg-cli/src/doctor.rs:135`), y el caso de las grafías de Windows (mayúsculas, barras y `\\?\`) corre sólo en Windows (`windows_spellings_of_one_file_are_the_same_install`, `doctor.rs:217`).

**Verificación:** el workflow quedó en verde en las dos plataformas en el PR #32, el que lo agrega, y la suite pasó en Linux y en Windows al archivar ([verify](../../changes/archive/2026-10-16-install-smoke/verify.md)).
