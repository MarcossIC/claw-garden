# client-setup-cursor — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, sobre `813e856`, el último commit que toca código (el que sigue sólo toca `openspec/`):

- `cargo test --workspace`: pasa entero; los ignorados son los de antes (los que piden el modelo real o un ensayo a mano).
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin bendecir: lo que ve el modelo no cambió.
- Los tests de `setup claude-code` (`setup.rs` y los tres de `tests/binary.rs`) y los de `clawg hook` que ya existían pasan sin cambiar lo que esperan; sólo se les sumó el cliente donde construyen un `Command::Hook` o llaman a `respond` o a `envelope`.
- En Windows, el CI del pull request (`windows-latest`, sobre `1f51c7a`, cuyo código es el de `813e856`) pasa los dos clippy y `cargo test --locked --workspace` sin ningún arreglo. Ahí `Shell::HOST` es PowerShell (`crates/clawg-cli/src/setup/cursor.rs:46`) y `setup_cursor_installs_and_remove_undoes_it` escribe la línea con `& "<ruta>"`; `the_installed_cursor_hook_runs_through_a_shell` es sólo de Unix, así que nada corrió esa línea por PowerShell.

## Los tests en rojo

Escritos antes del código de cada commit; fallaban sin compilar.

| Commit | Antes | Después |
|---|---|---|
| `feat(hook): answer Cursor's sessionStart with --client cursor` | 99 errores: `cannot find type HookClient`, `variant Command::Hook has no field named client`, `no field workspace_roots`/`cursor_version` on `HookInput`, `cannot find function folder`, y las llamadas a `respond` y `envelope` con un argumento de más | pasan |
| `feat(setup): add clawg setup cursor` | 160 errores: el módulo `setup/cursor.rs` sólo tenía sus tests (`cannot find function plan`, `run_in`, `hook_line`, `report`; `cannot find type Shell`, `Step`, `State`), y `no variant Cursor` en `SetupClient` | pasan |

## «Cómo se mide», test por test

| Lo que pedía la exploración | Test |
|---|---|
| un `mcp.json` con otros servidores queda con esos servidores intactos, y `--remove` deja el archivo como estaba | `other_servers_stay_and_remove_gives_the_file_back` (`setup/cursor.rs`; byte por byte con la sangría de dos espacios, como JSON si no), `setup_cursor_installs_and_remove_undoes_it` (`tests/binary.rs`) |
| una segunda corrida no cambia nada | `a_second_run_changes_nothing` (`setup/cursor.rs`: los mismos bytes y ninguna copia nueva) |
| un JSON inválido no escribe nada | `an_invalid_file_writes_nothing` (`setup/cursor.rs`: seis archivos rotos o con otra forma, al instalar y al quitar) |
| una entrada de `sessionStart` con una raíz imprime `additional_context` y no registra `~/.cursor/` | `a_cursor_session_start_reads_its_one_root_and_not_its_directory` (`tests/binary.rs`, el hook corriendo en `<home>/.cursor`) |
| una con dos raíces no registra ningún proyecto | `a_cursor_session_with_two_roots_registers_no_project` (`tests/binary.rs`), `cursor_without_one_root_gets_only_the_language_line` (`hook.rs`, contra un daemon que cuenta los pedidos: ninguno) |
| una entrada con `cursor_version` en un hook sin `--client cursor` no imprime nada y sale con 0 | `a_claude_code_hook_that_cursor_runs_does_nothing` (`tests/binary.rs`, con la preferencia `es`, que sin esto imprimiría la línea del idioma), `a_claude_code_hook_run_by_cursor_prints_nothing` (`hook.rs`) |

La entrada de Cursor de los tests (`cursor_input` en `hook.rs`, `cursor_session_start` en `tests/binary.rs`) lleva los campos que la exploración leyó en la documentación. Desde la sesión en la nube `cursor.com` no se pudo abrir (la red no lo permite), así que no está copiada de un ejemplo de la documentación: es una de las cosas de «Pendiente en prod».

## Una corrida a mano

Con `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y el binario de tests de `target/debug/clawg`, sobre un `~/.cursor/mcp.json` con un servidor ajeno:

1. `clawg setup cursor --profile dev --dry-run`: `would add it` para el servidor y el hook, el aviso de la configuración de terceros, salida 0 y ningún archivo nuevo.
2. `clawg setup cursor --profile dev`: `added` en los dos, la copia de `mcp.json` en `.clawg-dev/backups/setup/`, el aviso y `Restart Cursor for the change to take effect.`, salida 0. `mcp.json` quedó con el servidor ajeno primero y `clawg-dev` después (`type`, `command`, `args`); `hooks.json`, con `"version": 1` y la línea `<ruta> hook session-start --agent cursor --client cursor --profile dev`.
3. Con la preferencia `es`, otra corrida: `ya está` en los dos y el aviso en español, sin copias nuevas.
4. `--remove`: `quitado` en los dos y una copia de cada archivo; `mcp.json` volvió a tener sólo el servidor ajeno y `hooks.json` quedó en `{"version": 1}`.

`the_installed_cursor_hook_runs_through_a_shell` (`tests/binary.rs`, Unix) hace lo que falta de esa corrida: toma la línea que quedó en `hooks.json`, la corre con `sh -c` en `~/.cursor` con la entrada de Cursor y lee `additional_context`.

## El spec contra el código

| Requisito | Dónde |
|---|---|
| CU1 | `Command::Setup` con `SetupClient::Cursor` (`crates/clawg-cli/src/cli.rs:923`), el despacho (`crates/clawg-cli/src/main.rs:281`), `plan` (`crates/clawg-cli/src/setup/cursor.rs:197`), que lee los dos archivos con `parse_object` antes de decidir, y `locations` y `read_state` (`cursor.rs:325`, `:331`) |
| CU2 | `server_step` (`cursor.rs:233`), con `matches_server` e `is_clawg_server` de `setup.rs` |
| CU3 | `hook_line` (`cursor.rs:145`), `split_powershell` y `powershell_word` (`:67`, `:125`), `sh_word` (`crates/clawg-cli/src/setup.rs:318`), `is_clawg_hook` (`cursor.rs:154`) con `runs_clawg_hook` (`setup.rs:419`), `hook_step` (`cursor.rs:266`) |
| CU4 | `hook_step` y `server_step` en `Mode::Remove`; `run_in` (`cursor.rs:447`) |
| CU5 | `apply` (`cursor.rs:338`) con `write_files` (`setup.rs:754`), `report` (`cursor.rs:345`) con `done_or_planned` y `file_lines` (`setup.rs:1077`, `:1092`) |
| CU6 | `HookClient` y `--client` (`cli.rs:916`, `:281`), `envelope` (`crates/clawg-cli/src/hook.rs:177`), `respond` (`hook.rs:226`; `user-prompt-submit` a Cursor en `:234`), `HookInput.workspace_roots` (`hook.rs:57`) |
| CU7 | `folder` (`hook.rs:194`) |
| CU8 | `HookInput.cursor_version` (`hook.rs:61`) y la salida temprana de `respond` (`hook.rs:241`), antes de buscar el daemon |

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| Visibilidad en `setup.rs` | Lo común «al alcance del módulo de Cursor» | Sin cambios de visibilidad | `setup/cursor.rs` es un submódulo de `setup` y ve sus funciones privadas |
| `runs_clawg_hook` y `done_or_planned` | En el commit del refactor | En el de `setup cursor` | Salieron al escribir `is_clawg_hook` y `report` de Cursor; los tests de `setup.rs` siguen sin cambiar |
| Los tests de `clawg hook` que ya existían | Sólo el campo `client` de `Command::Hook` | También el cliente en cada llamada a `respond` y a `envelope` | Las dos funciones lo reciben; lo que esperan no cambió |
| `the_installed_cursor_hook_runs_through_a_shell` | No estaba | Sumado a CU3 | Es la única prueba de que la línea de `hooks.json` llega entera a `clawg hook --client cursor` por un shell |

## Pendiente en prod

Con Cursor 3.28 o posterior, una versión publicada de clawg instalada (no un build de desarrollo), y después de `clawg setup cursor --profile prod` y de reiniciar Cursor. Antes, apagar en Cursor «Include Third-Party Plugins, Skills, and Other Configs» (Settings → Agents → Third-Party Imports), salvo en el punto donde se dice otra cosa. La prueba se hace en una ventana con exactamente UNA carpeta abierta: con varias, el hook no inyecta nada a propósito (`folder`, `crates/clawg-cli/src/hook.rs:194`), y eso se ve igual que una falla. El contexto que el hook inyecta sólo muestra lo que ese agente guardó: WORKSPACE y RECENT siguen siendo del agente que pide (`openspec/specs/sharing/spec.md`, el párrafo que dice que RECENT del contexto sin consulta «sigue siendo sólo del agente», y el del workspace, que carga «sólo filas de L»; el código lo arma `recent_row!` en `crates/clawg-core/src/memory/prompt_builder.rs`). La excepción es LAST SESSION, que sigue el alcance de la búsqueda y puede traer el `session_summary` más nuevo del proyecto de otro agente. Por eso un agente `cursor` recién creado recibe `empty_context` (en `hooks.runs` de `clawg doctor`) hasta que guarde algo, salvo que otro agente haya dejado un `session_summary` en ese proyecto. Por eso el orden es: primero guardar una nota desde un chat de Cursor (con `save_memory`) en el repositorio de la prueba, y abrir otro chat. Recién después van los puntos numerados de abajo, en ese mismo repositorio.

1. **Que el hook corra por PowerShell sin abrir ventanas** (Windows): abrir un chat nuevo no muestra ninguna ventana, también con el binario en una ruta con un espacio.
2. **Que `additional_context` llegue al agente:** en el chat nuevo, el agente conoce el save sin buscarlo. Con la preferencia `es`, que el texto con acentos llegue entero por PowerShell. **Riesgo:** cursor.com/docs/hooks dice de `sessionStart`: "This hook runs as fire-and-forget; the agent loop does not wait for or enforce a blocking response." Así que que un chat reciba el contexto no prueba que siempre llegue: probar en varios chats nuevos.
3. **Que la decisión 7 sea cierta para la entrada que Cursor manda de verdad:** aplica sólo cuando `clawg setup claude-code --profile prod` está instalado en el mismo perfil; hasta entonces este punto queda pendiente. Con la configuración de terceros encendida, el hook de Claude Code que corre Cursor no imprime nada (si Cursor respeta `args`, porque la entrada trae `cursor_version`; si no, porque corre `clawg` solo, que sale con 1 sin escribir en stdout): el agente recibe el contexto una sola vez, el del hook de Cursor, y `clawg projects list --profile prod` no lista `~/.cursor`. Es el único punto que necesita la configuración de terceros encendida. Los hooks de Claude Code todavía no están instalados en prod: su instalación depende de la segunda pasada de [ram-footprint-gates](../2026-10-29-ram-footprint-gates/exploration.md), que necesita autorización (fila 92 de `openspec/README.md`).
4. **Resuelto leyendo la documentación** (cursor.com/docs/hooks, leído el 2026-11-04): `hooks.json` es `{"version": 1, "hooks": {...}}` y cada entrada tiene un `command` obligatorio; en Windows, la app de escritorio corre los comandos de hook por PowerShell y les pasa el JSON por stdin; los hooks de usuario corren desde `~/.cursor/`. Lo que queda por mirar es sólo que Cursor acepta el archivo: su log deja de decir `No user hooks configuration found` (`%APPDATA%/Cursor/logs/<sesión>/window*/output_*/cursor.hooks.*.log`). Ese log muestra sólo que la configuración se cargó y `Hook step requested: workspaceOpen`; no prueba que un hook haya corrido.
5. **La forma de `workspace_roots` en Windows:** el hook toma la raíz tal como llega. Si Cursor la manda como `/c:/…` y no como `C:\…`, el proyecto no sería el mismo que el del `cwd` de Claude Code para el mismo repositorio. La forma que llega la muestra el campo `folder` del [registro de corridas del hook](../../hook-run-log/proposal.md) (`<data>/hook-runs.jsonl`), que `clawg doctor` muestra en `hooks.runs`: tal como llegó, sin normalizar.
6. **Una ruta con espacios en macOS o Linux:** que Cursor corra la línea por un shell, que es para lo que está escrita.
7. **Que la entrada de los tests sea la de verdad:** la que el [registro de corridas del hook](../../hook-run-log/proposal.md) deja para un `sessionStart` real (los nombres de los campos de la entrada, en `input_keys` de `hooks.runs` en `clawg doctor`; los valores no se guardan), comparada con `cursor_input`.

## Al archivar (2026-11-04)

- Se archiva sin la corrida con Cursor real. Esa prueba se hace en prod, después de publicar, con el binario publicado (los puntos de «Pendiente en prod»).
- Nada en este cambio prueba todavía que `additional_context` llegue al agente: los tests fijan lo que el hook imprime, no lo que Cursor hace con ello.
- Un hook que no inyecta nada no deja rastro (`crates/clawg-cli/src/hook.rs:6-7`). Por eso el cambio [hook-run-log](../../hook-run-log/proposal.md) agrega el registro de corridas: cada inicio de sesión deja una línea y `clawg doctor` muestra las últimas (`hooks.runs`), lo que hace diagnosticable una falla ahí.
- La versión: el código de este cambio viene después del bump a 0.75.0 (`b425b3c`), que no tiene tag. Por eso sale en 0.76.0, y ese único bump se commitea después del código de [hook-run-log](../../hook-run-log/proposal.md), para que las dos cosas salgan juntas.

## Veredicto

Sin CRITICAL en Linux ni en el CI de Windows. Los puntos de «Pendiente en prod» quedan como WARNING hasta probarlos en prod.
