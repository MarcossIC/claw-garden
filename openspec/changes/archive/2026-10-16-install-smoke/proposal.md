# install-smoke — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (después de ingest-outbox, 36, y bridge-reconnect, 39, que ganan acá su prueba de proceso) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); lo que sigue está verificado otra vez contra `main` (`f7ce973`).

1. **Ningún test lanza el binario.** `crates/clawg-cli/tests/` sólo tiene `api.rs`, que sirve el router dentro del proceso del test («The daemon binary is never spawned», `crates/clawg-cli/tests/api.rs:1-9`), y `conventions.rs`. No hay un `CARGO_BIN_EXE_clawg` en el repo. Un kill duro, el lock entre dos procesos, los archivos `daemon.json` y `token` que lee el puente, el stdio real y el reinicio del daemon con un puente abierto se prueban a mano en cada `verify.md`.
2. **El binario no puede apuntar a un directorio aislado.** El directorio de datos es `<home>/.clawg-dev` o `<home>/.clawg` (`crates/clawg-core/src/paths.rs:158-166` en `main`), con el home de `dirs::home_dir()`. En Windows ese home viene de la API de carpetas conocidas, así que cambiar `USERPROFILE` no lo mueve: un test con el binario escribiría en el `~/.clawg-dev` real del usuario.
3. **`serve` siempre baja el modelo real.** `serve` arma el bootstrap con `production_deps` (`crates/clawg-cli/src/daemon/serve.rs:87-88` en `main`), que descarga 470 MB. `FakeEmbedder` existe en los builds de test (`crates/clawg-core/src/memory/embedder.rs:427`), pero el binario no tiene forma de elegirlo.
4. **No hay CI.** No existe `.github/`. Nada corre la suite sin que alguien se acuerde, y en Linux la suite ni siquiera pasaba: los 7 tests de `doctor` usaban rutas de Windows (`crates/clawg-cli/src/doctor.rs:127-263` en `main`) y fallaban al unir el nombre del ejecutable.

## Decisiones

Defaults tomados donde la exploración dejaba la pregunta abierta. La del CI se preguntó en el hilo con una tarjeta y se eligió Windows y Linux.

- **Aislamiento: `CLAWG_TEST_HOME` reemplaza el home, no el directorio de datos.** El perfil sigue eligiendo `.clawg-dev` o `.clawg` debajo, así que la regla de que un build debug nunca abre `.clawg` no cambia ni necesita un caso nuevo ([diseño, D1](design.md#d1-el-aislamiento)).
- **Embedder: `CLAWG_TEST_EMBEDDER=fake|stalled`.** `fake` carga `FakeEmbedder` al instante y `stalled` deja la descarga sin terminar, que es como se consigue un `202` antes del kill. Un valor desconocido es un error al arrancar, nunca el modelo real ([D2](design.md#d2-el-embedder)).
- **Las dos variables sólo existen en un build con la feature `testing` de clawg-core**, que sólo encienden las dev-dependencies. El binario que se entrega no las lee, y un test de convenciones falla si un manifiesto enciende `testing` fuera de `[dev-dependencies]` ([D3](design.md#d3-sólo-en-el-binario-de-test)).
- **El modelo real queda fuera del smoke.** Con el falso, la suite corre en segundos y el CI no baja nada. El camino real ya lo cubre cada verificación en Windows.
- **El smoke corre siempre con `cargo test`**, sin variable que lo active: tarda alrededor de un segundo.
- **El escenario del puente abierto entra ahora**, sin `#[ignore]`: bridge-reconnect (39) ya está archivado.
- **CI: un workflow con `clippy` y la suite en `ubuntu-latest` y `windows-latest`**, en cada PR y en cada push a `main`.

## Alcance

1. `paths.rs`: `CLAWG_TEST_HOME` en los builds con `testing`.
2. `bootstrap.rs`: `TestEmbedder` y `serve_deps`; `serve` lo usa en lugar de `production_deps`.
3. `crates/clawg-cli/tests/binary.rs`: kill duro con un save en `202`, lock entre dos `serve`, `status` y `stop`, prod rechazado en debug, `backup` y `restore`, los dos handshakes por stdio real con herramientas que llegan al daemon, reinicio del daemon con el puente abierto, y que una prueba que falla a la mitad no deja el store tomado.
4. `conventions.rs`: `testing` sólo en `[dev-dependencies]`.
5. Los tests de `doctor` con rutas de la plataforma, para que la suite pase en Linux.
6. `.github/workflows/ci.yml`.
7. `.claude/CLAUDE.md`, sección Testing.

## Fuera de alcance

- **El autoarranque por el binario.** El daemon que lanza el puente usa el puerto fijo del perfil (`crates/clawg-cli/src/client.rs:895-898`), así que en la máquina del usuario chocaría con su daemon dev. Sigue cubierto en proceso (`crates/clawg-cli/src/mcp/mod.rs`). Probarlo por el binario pide antes que el puerto se pueda elegir, y eso es superficie de producto.
- Scripts de smoke sobre el binario instalado y un instalador para Linux o macOS.
- El modelo real en el smoke o en el CI.
- Una compuerta de `rag-eval` en CI: el set de evaluación es privado (`.gitignore:30-32`).
- macOS en la matriz.

## Datos guardados

No cambia el esquema, ningún identificador ni ningún estado. El binario que se entrega se comporta igual: las dos variables no existen en él. Ningún test toca `~/.clawg` ni `~/.clawg-dev`: cada uno tiene su home temporal.

## Éxito

- Los 9 tests de `binary.rs` pasan en Linux y en Windows, y una corrida no deja ningún proceso `clawg` hijo vivo.
- `the_stalled_test_embedder_never_finishes_the_download`, `the_fake_test_embedder_loads_at_once_and_records_its_contract`, `a_test_home_replaces_the_system_home` y `only_dev_dependencies_turn_on_the_test_stand_ins` fallan sin el cambio (no compilan).
- Los 7 tests de `doctor` pasan en Linux.
- El workflow queda verde en las dos plataformas en el PR que lo agrega.

`rag-eval` no aplica: no cambia el ranking.
