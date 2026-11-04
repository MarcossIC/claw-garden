# client-setup-cursor — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | M | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); verificado otra vez contra `develop` (`592a6e0`):

- `clawg setup` sólo conoce a Claude Code: `SetupClient` tiene una sola variante (`crates/clawg-cli/src/cli.rs:910-912`) y el despacho sólo la atiende (`crates/clawg-cli/src/main.rs:276`). Para Cursor, el README da un `mcp.json` a mano, sin `--agent` (`README.md:430-445`), y sin `--agent` el agente es el nombre que el cliente diga de sí mismo.
- `clawg hook` sólo habla el formato de Claude Code. `HookInput` lee `cwd`, `source`, `transcript_path` y `session_id` (`crates/clawg-cli/src/hook.rs:39`); sin `cwd`, el proyecto sale del directorio actual (`hook.rs:241`), que en un hook de usuario de Cursor es `~/.cursor/`, y esa carpeta quedaría registrada como proyecto. La salida es `hookSpecificOutput.additionalContext` (`envelope`, `hook.rs:166-172`), que Cursor no documenta: Cursor lee `additional_context`.
- Un hook que instaló `clawg setup claude-code` y que Cursor carga por la configuración de terceros corre con `--agent claude-code`, en `~/.cursor/` y con la entrada de Cursor: leería el proyecto equivocado con el agente equivocado.

## Objetivo que sirve

4: varios agentes, con el usuario controlando qué ve cada uno. Cursor pasa a ser un agente con nombre fijo (`cursor`), como Claude Code (`claude-code`), y las reglas de `sharing.json` lo nombran sin depender de lo que el cliente reporte.

## Decisiones

Las nueve de la exploración, cerradas allí; no se reabren:

1. **`clawg setup cursor [--profile P] [--dry-run] [--remove]`**, con la forma de `setup claude-code`: un `plan` puro sobre lo leído y después `apply`, la copia en `backups/setup/`, la escritura atómica y lo ajeno intacto. Un servidor o un hook de clawg que difiere se informa y no se pisa.
2. **`~/.cursor/mcp.json`:** el servidor del perfil, con la ruta absoluta del binario y `mcp --profile P --agent cursor`.
3. **`~/.cursor/hooks.json`:** sólo `sessionStart`. No se instala `beforeSubmitPrompt`, porque no puede inyectar el recordatorio.
4. **`clawg hook session-start --agent cursor --client cursor [--profile P]`:** con `--client cursor` lee `workspace_roots` y `session_id` e imprime `{"additional_context": …}`. Sin `--client`, sigue como hoy.
5. **La carpeta sale de la entrada, nunca del directorio actual.** Con una sola raíz, esa es el proyecto. Con varias o ninguna no adivina: imprime sólo la línea de idioma, si hay.
6. **El comando se escribe para PowerShell en Windows:** `& "<ruta>" hook …`, también con una ruta con espacios.
7. **Un hook de Claude Code que corre Cursor no hace nada:** sin `--client cursor`, una entrada con `cursor_version` no imprime nada y sale con 0.
8. **Sin archivo de reglas.** Cursor ya recibe las instrucciones del servidor y deja las herramientas en archivos.
9. **`setup cursor` avisa** que hay que apagar la configuración de terceros, porque no se puede leer de forma estable.

Lo que la exploración no fijaba y se decide al aplicar, sin cambiar ninguna de las nueve:

- **`--client` toma `claude-code` o `cursor`, y `claude-code` es el valor por defecto.** Así «sin `--client`» tiene un nombre en la ayuda y en el código, y los hooks que ya instaló `setup claude-code` siguen igual sin tocarse.
- **Con `--client cursor`, `user-prompt-submit` no imprime nada y no consulta al daemon.** `setup cursor` no lo instala (decisión 3); si alguien lo configura a mano, no rompe la sesión ni inventa una salida que Cursor no lee.
- **Fuera de Windows, la línea del hook es la de un shell POSIX:** la ruta sin comillas si sólo tiene `[A-Za-z0-9/._-:]`, si no entre comillas dobles con `"`, `$`, `` ` `` y `\` escapados, la misma regla con la que `setup claude-code` escribía sus hooks antes de la forma exec (`legacy_command`, `crates/clawg-cli/src/setup.rs:312`). En Windows, `& "<ruta>"` con `` ` ``, `$` y `"` escapados por un acento grave, que es como PowerShell no expande nada dentro de las comillas dobles.
- **Un hook de clawg en `hooks.json`** es una entrada cuyo `command`, partido como lo partiría el shell que lo corre (sin el `&` inicial en PowerShell), es un programa llamado `clawg` (o `clawg.exe`) seguido de `hook session-start` y con `--profile P`. Es la misma regla que `setup claude-code` usa para su forma heredada (`is_clawg_hook`, `setup.rs:403`).
- **Un `hooks.json` que se crea empieza con `"version": 1`.** Uno que ya existe no gana ni pierde claves que no sean el hook de clawg: `--remove` deja `{"version": 1}` si clawg lo había creado, como `setup claude-code --remove` deja `{}` un `settings.json` que no existía.
- **El módulo:** `crates/clawg-cli/src/setup/cursor.rs`, declarado desde `setup.rs`, que no se mueve: las specs citan sus líneas. Lo común (leer, escribir por un temporal con copia previa y poda, reconocer un programa de clawg, partir una línea como un shell) queda en `setup.rs` y lo usan los dos clientes; la conducta de `setup claude-code` no cambia.

## Alcance

1. `clawg-cli`:
   - `cli.rs`: `SetupClient::Cursor`, `HookClient` y `--client` en `Command::Hook`.
   - `hook.rs`: los campos de Cursor en `HookInput`, la carpeta según el cliente, la salida de Cursor y el silencio de la decisión 7.
   - `setup/cursor.rs`, nuevo: `State`, `plan`, `apply`, `report`, `run`.
   - `setup.rs`: lo común pasa a funciones que usan los dos clientes, sin cambiar lo que escribe ni lo que informa `setup claude-code`.
   - `main.rs`: el despacho de `setup cursor` y el `--client` del hook.
2. Docs: README (conectar Cursor con `clawg setup cursor`, en lugar del `mcp.json` a mano, y desinstalarlo) y `docs/reference/commands.md` (`clawg hook` y `clawg setup`).
3. `openspec`: esta propuesta, la [spec](spec.md), las [tareas](tasks.md), `verify.md`, `state.yaml`, la fila 115 de `README.md` y la línea de `ROADMAP.md`. Los requisitos de la spec se suman a `specs/setup` y `specs/hooks` al archivar, que queda fuera de este cambio.

## Fuera de alcance

- El proyecto por defecto del puente (`crates/clawg-cli/src/mcp/server.rs`): lo resuelve [bridge-project-identity](../bridge-project-identity/exploration.md).
- `crates/clawg-core/src/memory/prompt_builder.rs`, que toca session-start-context en paralelo.
- El contrato del agente (`agent_contract.json`, `agent_contract.es.json`): no cambia nada de lo que ve el modelo.
- Un archivo de reglas para Cursor (decisión 8) y el hook `beforeSubmitPrompt` (decisión 3).
- `clawg doctor`: sigue mirando sólo el registro de Claude Code.
- Leer o cambiar la configuración de terceros de Cursor (decisión 9): sólo se avisa.
- Subir la versión y archivar.

## Cómo se mide el éxito

Tests en rojo primero, con directorios temporales y la entrada de `sessionStart` armada con los campos que documenta Cursor (los de la exploración: `conversation_id`, `generation_id`, `model`, `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email`, `transcript_path` y `session_id`). Los nombres, en la [spec](spec.md):

- un `mcp.json` con otros servidores queda con esos servidores intactos, y `--remove` deja el archivo como estaba;
- una segunda corrida no cambia nada;
- un JSON inválido no escribe nada;
- una entrada de `sessionStart` con una raíz imprime `additional_context` y no registra `~/.cursor/`;
- una con dos raíces no registra ningún proyecto;
- una entrada con `cursor_version` en un hook sin `--client cursor` no imprime nada y sale con 0.

Además: los tests de `setup claude-code` y de `clawg hook` que ya existen pasan sin cambiar lo que esperan (sólo se suma el campo `client` donde se construye un `Command::Hook`), y la suite y `cargo clippy --workspace --all-targets -- -D warnings` pasan en Linux. El código `cfg(windows)` lo revisa el CI del pull request.

`rag-eval`: no aplica, no hay cambio de ranking.

Lo que un CI no reproduce queda en `verify.md` como pendiente en local, con Cursor instalado y el perfil `dev`: que el hook corra por PowerShell sin abrir ventanas, que `additional_context` llegue al agente y que la decisión 7 sea cierta para la entrada que Cursor manda de verdad.

## Datos y vuelta atrás

No toca el store, el esquema, los identificadores guardados ni el contrato de embedding. Escribe sólo `~/.cursor/mcp.json` y `~/.cursor/hooks.json`, y antes copia cada uno que existía a `<data>/backups/setup/`. Las memorias que Cursor guardaba con otro nombre de agente (el que reportaba su cliente, si el servidor se registró sin `--agent`) quedan con ese nombre, como en `setup claude-code`. Volver atrás es `clawg setup cursor --remove`, o restaurar esas copias.
