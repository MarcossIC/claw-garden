# bridge-reconnect — Verificación

**Código:** rama `claude/project-thread-7b3nb7`, un commit por tarea, sobre `main` en `638812b`.
**Modo:** Strict TDD (`openspec/config.yaml`). Cada test de un comportamiento nuevo falló antes de su código, por la aserción o por no compilar. Antes de la tarea 2, sobre el código de `main`: `a_restarted_daemon_is_reached_with_its_new_token`, `memory_status_after_a_restart_reports_the_new_daemon`, `a_daemon_that_comes_up_after_the_start_gave_up_is_used` y `a_refused_token_is_reread_once` fallaban por la aserción (igual que después los de las tareas 3 y 4 antes de su código); `an_error_the_daemon_answered_reaches_it_once` y `only_health_sets_a_timeout` ya pasaban, como se esperaba: fijan las invariantes de las que depende el reintento (no se repite lo que el daemon contestó, y ninguna llamada que escribe tiene timeout).
**Estado:** R1 a R6 pasan en Linux, commit por commit, y con el binario real en un `HOME` descartable. La suite pasa entera en Windows, y en el perfil dev, con Claude Code como cliente, un puente abierto sobrevive a un `clawg stop` y a un `clawg serve` ([Perfil dev](#perfil-dev-con-claude-code-windows)).

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

## Perfil dev con Claude Code (Windows)

El binario debug que corría el editor era 0.4.0, anterior al cambio. Antes de la prueba: se cerró Claude Code, se paró el daemon dev con `clawg stop --profile dev`, se recompiló `target\debug\clawg.exe` desde `main` (`clawg 0.5.1`) y se abrió una sesión nueva con el servidor `clawg-dev`. Todas las herramientas se llamaron desde esa sesión, siempre con el mismo puente (pid 23972) y sin tocar `/mcp`.

| Paso | Resultado |
|---|---|
| `memory_status` | ok: daemon 0.5.1, pid 39692, que el puente arrancó solo |
| `clawg stop --profile dev` desde una terminal; después `list_memories` | «stopped the dev daemon (pid 39692)»; `list_memories` dice «the clawg daemon is not running (it may have been stopped or updated); start it with `clawg serve`, or reconnect clawg in your MCP client, then call again» |
| `clawg status --profile dev` después de esa llamada | código 1, `running: false`; el único `clawg.exe` del perfil dev era el puente: no lanzó un daemon |
| `clawg serve --profile dev` desde la terminal (pid 27352, mismo puerto 27381, token nuevo); `save_memory` con el modelo todavía cargando | «accepted; it will be stored once the embedding model finishes loading» |
| `search_memory` del mismo texto, con el modelo listo; `memory_status` | el texto guardado sale primero (score 0.50); `memory_status` muestra 0.5.1 y el pid nuevo, 27352 |

La memoria de prueba se borró después. La línea «reached the daemon again» del puente no aparece en el log de MCP de Claude Code, que en este archivo sólo registra las llamadas a herramientas. En Linux sí se vio en el log del puente ([Binario real](#binario-real-linux)).

## Diferencias entre la spec y el código

Al archivar, cada `path:línea` de la spec se volvió a verificar contra el código de `main` (`f8a361e`). La spec consolidada en [`specs/mcp`](../../../specs/mcp/spec.md#reconectar-con-el-daemon-tras-un-reinicio) ya recoge lo que sigue.

- **Línea del texto de `StartTimeout`:** R3 cita `crates/clawg-cli/src/mcp/server.rs:418-422`. El texto está hoy en `server.rs:454-458`, porque el archivo creció desde que se escribió la spec. Es una referencia vencida; el texto no cambió.
- **Relectura que devuelve `StoreHeld`:** R2 dice que, si la relectura no encuentra un daemon, se informa el texto de R4. Cuando lo que encuentra es un almacén que tiene otro proceso que no es un daemon (`client.rs`, `probe_running`, comprobación de `owner.holder`), el puente informa el mensaje de `StoreHeld`, no el de R4 (`crates/clawg-cli/src/mcp/server.rs:411` y `:420`). Ningún test de la tabla lo cubre; `mod.rs:416` prueba el mismo error sólo en el arranque.
- **Puerto del reinicio:** el escenario «el daemon se reinició» habla de «otro puerto». El test publica B en `127.0.0.1:0` (`server.rs:2887`), así que casi siempre cambia el puerto, pero el test no lo afirma: lo que prueba es que el puente usa el puerto y el token que B publica.
- **Una sola relectura por llamada:** R2 dice «repetir una vez». El código además no vuelve a releer si la llamada ya se releyó al pedir el cliente (`server.rs:418`, `!reresolved`), así que una caída de arranque seguida de un 401 informa el 401 sin otro intento. Está de acuerdo con la spec, pero no lo decía.

Ningún otro apartado de [`specs/mcp`](../../../specs/mcp/spec.md) afirma lo contrario: no hay texto del puente que mande a borrar el token. El de `ClientError::Unauthorized` (`client.rs:51`) sí lo manda, pero sólo lo ven los comandos de la CLI, que releen los archivos en cada corrida.
