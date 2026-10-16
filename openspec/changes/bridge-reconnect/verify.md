# bridge-reconnect — Verificación

**Código:** rama `claude/project-thread-7b3nb7`, un commit por tarea, sobre `main` en `638812b`.
**Modo:** Strict TDD (`openspec/config.yaml`). Cada test de un comportamiento nuevo falló antes de su código, por la aserción o por no compilar. Antes de la tarea 2, sobre el código de `main`: `a_restarted_daemon_is_reached_with_its_new_token`, `memory_status_after_a_restart_reports_the_new_daemon`, `a_daemon_that_comes_up_after_the_start_gave_up_is_used` y `a_refused_token_is_reread_once` fallaban por la aserción (igual que después los de las tareas 3 y 4 antes de su código); `an_error_the_daemon_answered_reaches_it_once` y `only_health_sets_a_timeout` ya pasaban, como se esperaba: fijan las invariantes de las que depende el reintento (no se repite lo que el daemon contestó, y ninguna llamada que escribe tiene timeout).
**Estado:** R1 a R6 pasan en Linux, commit por commit, y con el binario real en un `HOME` descartable. La suite pasa entera en Windows. Falta la prueba con Claude Code en el perfil dev ([pendiente](#pendiente-máquina-de-marcos)).

## Suite

`cargo test --workspace --no-fail-fast` en Linux:

| Commit | Pasan | Fallan |
|---|---|---|
| `638812b` (base, `main`) | 1886 | 10 |
| `cdb1e88` docs(openspec): propose how the MCP bridge reaches a restarted daemon | 1886 | 10 |
| `ae8b86e` feat(cli): decide which bridge failures a fresh daemon lookup can fix | 1888 | 10 |
| `d163189` fix(cli): reach a restarted daemon from an open MCP bridge | 1893 | 10 |
| `532646f` fix(cli): tell the agent how to get its daemon back | 1895 | 10 |
| `ae86f09` fix(cli): register the project again when its first registration never arrived | 1896 | 10 |
| `0c6ff27` docs: describe how the MCP bridge finds a restarted daemon | 1896 | 10 (sólo docs) |

En la primera corrida de `ae8b86e` falló un test más (1.887 y 11), con otras compilaciones corriendo a la vez en la máquina; no quedó el nombre en la salida. La segunda corrida de ese commit dio 1.888 y los 10 de siempre, y ninguno de los tests nuevos falló en ninguna corrida.

- **+10 tests**, ninguno reemplazado.
- **Los 10 que fallan son los de `main`** (por nombre): `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera).
- **Warnings:** los de `main` (`spanning`, `search_all_memories_with_scoring`); ninguno nuevo. `cargo clippy -p clawg-cli --all-targets` da los mismos 16 que `main`.

`cargo test --workspace --no-fail-fast --target-dir target-test` en Windows, sobre `main` en `f8a361e` (este cambio y quality-guards ya mergeados): **1.918 pasan, 0 fallan, 0 ignorados**. Es la base de 1.902, más los 10 de acá y los 6 de quality-guards.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Qué se resuelve releyendo | `only_a_refused_token_a_refused_connection_or_an_unfinished_start_is_reresolved` (`mcp/startup.rs`) | ✓ |
| R2. Releer y reintentar una vez | `a_restarted_daemon_is_reached_with_its_new_token`, `a_refused_token_is_reread_once` (2 pedidos), `an_error_the_daemon_answered_reaches_it_once` (1 pedido) (`mcp/server.rs`); `only_health_sets_a_timeout` (`client.rs`) | ✓ |
| R3. Un arranque que no terminó | `a_daemon_that_comes_up_after_the_start_gave_up_is_used` (`mcp/server.rs`); los cuatro de arranque de `mcp/mod.rs` siguen pasando sin cambios | ✓ |
| R4. Textos | `a_daemon_stopped_mid_session_is_reported_without_starting_one`, `the_refused_token_text_does_not_say_to_delete_it` (`mcp/server.rs`) | ✓ |
| R5. Registro del proyecto | `a_project_registration_the_old_daemon_refused_is_sent_to_the_new_one` (`mcp/server.rs`) | ✓ |
| R6. `memory_status` | `memory_status_after_a_restart_reports_the_new_daemon` (`mcp/server.rs`) | ✓ |

`rag-eval` no aplica: no cambia el ranking ni el contexto.

## Binario real (Linux)

`target/debug/clawg` con `HOME` en un directorio descartable (perfil dev, sin el modelo), manejado por stdio con un script que habla MCP como un host:

| Paso | Resultado |
|---|---|
| `clawg mcp` autoarranca el daemon; `memory_status` y `list_memories` | ok |
| `clawg stop`; `list_memories` | error: «the clawg daemon is not running (it may have been stopped or updated); start it with `clawg serve`, or reconnect clawg in your MCP client, then call again» |
| `clawg status` después de esa llamada | código 1: el puente no lanzó un daemon |
| `clawg serve` de nuevo (mismo puerto 27381, token nuevo); `list_memories` y `memory_status` con el mismo puente | ok; `memory_status` muestra el pid nuevo, y el log del puente dice `reached the daemon again at http://127.0.0.1:27381 (daemon 0.5.1, bridge 0.5.1)` |

Como el daemon volvió al mismo puerto, este recorrido ejercita el camino del 401; el de la conexión rechazada con otro puerto lo cubre `a_restarted_daemon_is_reached_with_its_new_token`.

`clawg stop` dijo las dos veces «accepted the shutdown but is still running after 15s». No es de este cambio: en Linux el daemon que termina queda como zombi hasta que su padre lo recoge (el puente que lo autoarrancó, o el script), y el comentario de `ProcessSpawner` ya lo anticipa (`crates/clawg-cli/src/client.rs`, «leaves one zombie entry until the parent exits»). Al cerrar el puente y el script no quedó ningún proceso `clawg`.

## Pendiente (máquina de Marcos)

1. ~~**Suite en Windows**~~: hecha, ver [Suite](#suite).
2. **Perfil dev con Claude Code**, con el binario debug de `main`. El que corría el editor era 0.4.0, anterior al cambio, así que hay que recompilarlo antes:
   1. Abrí una sesión de Claude Code con el servidor `clawg-dev` y pedile `memory_status`.
   2. En otra terminal, `clawg stop --profile dev`. Pedile `list_memories`: tiene que decir «the clawg daemon is not running (it may have been stopped or updated)…», y `clawg status --profile dev` tiene que seguir diciendo que no corre.
   3. `clawg serve --profile dev` en esa terminal. Sin tocar `/mcp`, pedile `save_memory` y después `search_memory` del mismo texto: los dos tienen que responder sin error, y `memory_status` tiene que mostrar el pid nuevo.
