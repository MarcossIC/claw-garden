# shutdown-budget — Verificación

## Linux (2026-10-20)

Sobre `f7cba30` (el último commit de código), en `target`, con el compilador de `rust-toolchain.toml`:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0.
- Cada commit de código de las [tareas](tasks.md) (`b09d4a9`, `a04fc6b`, `6e30918`, `71b9c15`, `f7cba30`) pasó sus tests y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` pasa sin bendecir: nada de lo que ve el modelo cambió.

## El binario real

Con `target/debug/clawg` y `CLAWG_TEST_HOME` en un directorio temporal:

- con `CLAWG_TEST_EMBEDDER=blocked`, el log dice `the test model load blocks its thread and never returns`; `clawg stop` sale con 0 en unos 2 s (el `RUNTIME_TEARDOWN`), y el log termina en `checkpointed the WAL into memory.db` y `clawg serve dev stopped`, sin el «model load task panicked» ni el «retrying in 30s» de antes;
- con `fake`, `kill -TERM` al daemon: sale con 0, el log dice `received SIGTERM, shutting down` y hace el checkpoint, y en el directorio de datos quedan sólo `memory.db` y `memory.lock`.

## CI

`ci.yml` corre en el pull request de la rama, sobre Linux y Windows. Windows no se corrió a mano: el brazo de SIGTERM es `#[cfg(unix)]` y su test también; en Windows `terminated` queda pendiente para siempre, y el resto no depende de la plataforma.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1, HTTP | `a_server_that_does_not_finish_is_left_after_the_grace` y sus vecinos no compilaban sin `within_grace` |
| R1, después del drenaje | `shutdown_gives_up_on_a_worker_that_never_ends` no compilaba sin `AFTER_DRAIN`; sin el tope, `shutdown_by` espera al worker para siempre |
| R1, desmontaje | `stop_ends_a_daemon_whose_model_load_never_returns`, con `blocked` ya implementado y antes de `shutdown_timeout`: `clawg stop` salió con 1 tras sus 15 s |
| R2 | `every_phase_of_a_shutdown_fits_inside_the_stop_timeout` falló con 15 s: «a shutdown that uses every phase (22s) must end before `clawg stop` gives up on the process» |
| R3 | `sigterm_stops_the_daemon_in_order`: el proceso terminó por la señal, sin éxito, con `daemon.json` y `token` en su lugar |
| R4 | Los tests de `plan.rs` no compilaban sin `StepError`; `a_daemon_still_shutting_down_says_how_to_start_it_once_it_ends` fallaba: el informe decía «Its daemon is not running» |

## Lo que no se midió

- Ningún ranking ni recuperación cambia, así que no hay `rag-eval`.
- La espera del pid en `undo StartOldDaemon` (`wait_for_end`) no tiene test propio: es `Watched`, que ya prueban los tests de `process.rs`, y el orden (esperar antes de arrancar) lo fija el código, no un test con procesos.
- Los eventos de consola y de apagado de Windows no se atienden; queda por probar en una máquina Windows si un daemon sin consola los recibe.
- Ningún tope está medido: son valores de partida elegidos para el margen.
