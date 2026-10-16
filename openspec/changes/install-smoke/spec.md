# install-smoke — Especificación

Al archivar, estos requisitos forman `specs/install` (nuevo dominio): el binario que prueba la suite y cómo se aísla.

## Definiciones

- **Build de test:** un binario compilado con la feature `testing` de clawg-core, como el que usan los tests de integración de clawg-cli.
- **Home de prueba:** el valor no vacío de `CLAWG_TEST_HOME`.

## R1. Home de prueba

Un build de test MUST resolver el directorio de datos como `<home de prueba>/<perfil>` cuando `CLAWG_TEST_HOME` no está vacío, y como hoy cuando falta o está vacío. Un build sin `testing` MUST NOT leer `CLAWG_TEST_HOME`. La resolución del perfil MUST NOT cambiar: un build debug rechaza `prod` también con un home de prueba.

#### Escenario: el home de prueba gana
- GIVEN `CLAWG_TEST_HOME=/tmp/smoke`
- WHEN un build de test resuelve el perfil dev
- THEN el directorio es `/tmp/smoke/.clawg-dev`

#### Escenario: debug sigue sin prod
- GIVEN un build debug de test con un home de prueba
- WHEN corre `clawg status --profile prod`
- THEN sale con 1, dice que prod está prohibido en un build debug y no crea `<home>/.clawg`

**Tests:** `a_test_home_replaces_the_system_home`, `an_unset_or_empty_test_home_keeps_the_system_home` (`paths.rs`), `a_debug_binary_refuses_prod_even_with_a_test_home` (`binary.rs`).

## R2. Embedder de prueba

En un build de test, `clawg serve` MUST cargar `FakeEmbedder` sin descargar nada con `CLAWG_TEST_EMBEDDER=fake`, MUST quedar con el embedder en `loading` con `stalled`, y MUST negarse a arrancar con cualquier otro valor no vacío. Sin la variable, o en un build sin `testing`, MUST usar el modelo real. El contrato de embedding MUST verificarse con la función real en los dos casos.

#### Escenario: fake
- WHEN el bootstrap corre con `fake`
- THEN no hay descarga, el modelo es `fake-embedder` y un store nuevo registra su contrato

#### Escenario: valor desconocido
- WHEN la variable vale `real`
- THEN el error nombra la variable y los valores aceptados

**Tests:** `the_test_embedder_is_named_by_its_variable_or_absent`, `the_fake_test_embedder_loads_at_once_and_records_its_contract`, `the_stalled_test_embedder_never_finishes_the_download` (`bootstrap.rs`).

## R3. Sólo dev-dependencies encienden `testing`

Ningún `Cargo.toml` del workspace MUST encender la feature `testing` de clawg-core fuera de una sección `[dev-dependencies]`.

**Tests:** `testing_outside_dev_dependencies_reads_the_section`, `only_dev_dependencies_turn_on_the_test_stand_ins` (`conventions.rs`).

## R4. El binario sobrevive a lo que sólo ve un proceso

La suite MUST lanzar el binario y probar, cada caso en su home de prueba y con `--port 0`:

#### Escenario: kill duro
- GIVEN un daemon con el embedder `stalled` que respondió `202` a un save
- WHEN el proceso se mata sin apagado y arranca otro con `fake`
- THEN el save queda guardado y la búsqueda lo encuentra

#### Escenario: dos daemons
- GIVEN un daemon corriendo
- WHEN arranca un segundo `serve` sobre el mismo directorio
- THEN sale con 1, nombra el pid del primero y el primero sigue respondiendo

#### Escenario: status y stop
- THEN `status` sale con 1 sin daemon y con 0 con uno, informando el directorio del home de prueba; `stop` sale con 0 cuando el proceso ya terminó limpio, sin `daemon.json`

#### Escenario: backup y restore
- GIVEN un save, un `backup` y otro save
- WHEN se intenta `restore` con el daemon corriendo
- THEN se rechaza y dice `clawg stop`
- WHEN se detiene, se restaura y se arranca otra vez
- THEN la salida nombra el snapshot `pre-restore`, está la fila de antes y no la de después

#### Escenario: MCP por stdio real
- GIVEN `clawg mcp --no-autostart` con un daemon corriendo
- THEN responde `initialize` (2025-06-18) con `clawg-dev` y la versión del crate, lista 12 herramientas, `memory_status` responde, y un `save_memory` se encuentra con `search_memory`
- AND sin `initialize`, con `_meta` de 2026-07-28, responde `server/discover` con las dos versiones, `tools/list` y `memory_status`
- AND cada línea de su stdout es un mensaje JSON-RPC

#### Escenario: reinicio con el puente abierto
- GIVEN un puente que ya respondió una herramienta
- WHEN el daemon se detiene y arranca otro con otro token
- THEN la siguiente herramienta responde sin error

**Tests:** los de `crates/clawg-cli/tests/binary.rs`.

## R5. Ningún hijo sobrevive a su prueba

Cada proceso que lanza la suite MUST terminarse por su handle cuando su prueba termina, también si falla, y la suite MUST NOT terminar procesos por nombre.

#### Escenario: prueba que falla a la mitad
- GIVEN una prueba que entra en pánico con su daemon corriendo
- THEN otro daemon puede tomar el mismo directorio enseguida

**Test:** `a_test_that_fails_midway_leaves_no_daemon_holding_the_store` (`binary.rs`).

## R6. CI

Cada PR y cada push a `main` MUST correr `clippy` con `-D warnings` y `cargo test --workspace` en Linux y en Windows. La suite MUST pasar en Linux.

**Verificación:** el workflow en verde en el PR que lo agrega; los 7 tests de `doctor` pasan en Linux.
