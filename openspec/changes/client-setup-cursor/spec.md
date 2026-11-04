# client-setup-cursor — Especificación

`clawg setup cursor` conecta clawg a Cursor, y `clawg hook session-start --client cursor` responde el hook `sessionStart` de Cursor. Al archivar, CU1 a CU5 se suman a [`specs/setup`](../../specs/setup/spec.md) y CU6 a CU8 a [`specs/hooks`](../../specs/hooks/spec.md). Lo que esas specs dicen de Claude Code no cambia.

## Definiciones

- **Directorio de Cursor:** `~/.cursor`, con el home de `clawg_core::paths::home_dir` (el de `CLAWG_TEST_HOME` en un build de tests). Ahí viven `mcp.json` (los servidores de usuario, en `mcpServers`) y `hooks.json` (los hooks de usuario, en `hooks.<evento>`).
- **Servidor del perfil:** `clawg` para `prod`, `clawg-dev` para `dev` (`Profile::mcp_server_name`), como en `setup claude-code`.
- **Servidor del plan:** `{"type": "stdio", "command": <exe>, "args": ["mcp", "--profile", P, "--agent", "cursor"]}`, con `<exe>` la ruta absoluta del binario en ejecución sin el prefijo `\\?\`.
- **Argumentos del hook:** `hook session-start --agent cursor --client cursor --profile P`.
- **Shell del hook:** PowerShell en Windows, un shell POSIX en los demás.
- **Línea del hook:** en un shell POSIX, la ruta del binario sin comillas si sólo tiene letras ASCII, dígitos y `/._-:`, si no entre comillas dobles con `"`, `$`, `` ` `` y `\` escapados por `\`, seguida de un espacio y los argumentos del hook separados por un espacio. En PowerShell, `& "` y la ruta con `` ` ``, `$` y `"` escapados por `` ` ``, `" ` y los argumentos.
- **Hook del plan:** `{"command": <línea del hook>}`.
- **Hook de clawg del perfil:** una entrada de `hooks.sessionStart` cuyo `command`, partido como lo partiría el shell del hook (y sin un `&` inicial en PowerShell), empieza por un programa llamado `clawg` (o `clawg.exe`) seguido de `hook session-start`, y lleva `--profile P` o `--profile=P`.
- **Entrada de Cursor:** el objeto JSON que Cursor pasa por stdin. Lleva `conversation_id`, `generation_id`, `model`, `hook_event_name`, `cursor_version`, `workspace_roots`, `user_email` y `transcript_path`; `sessionStart` suma `session_id`, igual a `conversation_id`. No lleva `cwd`.
- **Salida de Cursor:** `{"additional_context": T}` en stdout.

## CU1. `clawg setup cursor`

`clawg setup cursor [--profile P] [--dry-run] [--remove]` MUST resolver el perfil como los otros subcomandos (un build debug MUST rechazar `prod`), MUST rechazar una ruta del binario que no sea absoluta y MUST NOT abrir el store ni arrancar un daemon. Antes de escribir MUST leer `mcp.json` y `hooks.json`; si uno no se puede leer, no es JSON válido o no tiene la forma que Cursor lee (un objeto; `mcpServers` y `hooks` objetos; `hooks.sessionStart` una lista), MUST fallar sin escribir ninguno. MUST NOT escribir un archivo de reglas ni ningún otro archivo de Cursor.

#### Escenario: un `hooks.json` roto
- GIVEN un `mcp.json` válido y un `hooks.json` que no es JSON
- WHEN corre `clawg setup cursor`
- THEN sale con error nombrando `hooks.json`, y ninguno de los dos archivos cambia ni aparece una copia

**Tests:** `setup_takes_a_client_a_profile_dry_run_and_remove` (`cli.rs`), `an_invalid_file_writes_nothing`, `a_relative_binary_path_is_refused` (`setup/cursor.rs`).

## CU2. El servidor en `mcp.json`

Sin servidor del perfil, el plan MUST agregar el servidor del plan al final de `mcpServers`, que se crea si falta. Si existe y coincide (mismo `command` y `args`, `type` ausente o `stdio`), MUST NOT hacer nada. Si existe y no coincide, MUST reportarlo como conflicto y MUST NOT reemplazarlo. Los otros servidores y las otras claves MUST conservarse, en su orden.

#### Escenario: otros servidores
- GIVEN un `mcp.json` con dos servidores ajenos y una clave ajena
- WHEN corre `clawg setup cursor` y después `clawg setup cursor --remove`
- THEN después de la primera los dos servidores y la clave siguen iguales y en su orden, con `clawg-dev` al final, y después de la segunda el archivo es el de antes

**Tests:** `an_empty_state_plans_the_server_and_the_hook`, `a_different_server_is_a_conflict`, `other_servers_stay_and_remove_gives_the_file_back` (`setup/cursor.rs`).

## CU3. El hook en `hooks.json`

El plan MUST agregar el hook del plan al final de `hooks.sessionStart`, creando lo que falte; un `hooks.json` que no existía (o vacío) MUST empezar con `"version": 1`. MUST NOT instalar ningún otro evento, `beforeSubmitPrompt` incluido. Un hook igual al del plan MUST quedar como está; otro hook de clawg del perfil MUST reportarse como conflicto, mostrado como su `command`, y MUST NOT reemplazarse. Los hooks y las claves ajenas, y los hooks de clawg de otro perfil, MUST conservarse, en su orden.

#### Escenario: una ruta con espacio
- GIVEN el binario en `C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe`
- WHEN se arma la línea del hook para PowerShell
- THEN es `& "C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe" hook session-start --agent cursor --client cursor --profile dev`, y partida como la parte PowerShell da esa ruta y los argumentos del hook

#### Escenario: un hook de clawg distinto
- GIVEN un `hooks.json` con `clawg hook session-start --agent claude-code --profile dev` en `sessionStart`
- WHEN corre `clawg setup cursor --profile dev`
- THEN el reporte lo da como conflicto, el archivo no cambia y sale con 1

**Tests:** `the_hook_line_is_written_for_the_shell_that_runs_it`, `a_hook_line_reads_back_as_its_words`, `a_different_clawg_hook_is_a_conflict`, `foreign_hooks_and_keys_keep_their_order`, `a_hook_of_the_other_profile_is_not_ours`, `nothing_is_installed_before_a_prompt` (`setup/cursor.rs`).

## CU4. Idempotencia, `--dry-run` y `--remove`

Una segunda corrida sobre lo que dejó la primera MUST NOT escribir nada. `--dry-run` MUST reportar el mismo plan y MUST NOT escribir nada. `--remove` MUST quitar el servidor del perfil si es de clawg (`command` llamado `clawg` y primer argumento `mcp`), y `mcpServers` si queda vacío, y los hooks de clawg del perfil, y `hooks.sessionStart` y `hooks` si quedan vacíos. Un servidor ajeno con el nombre del perfil MUST reportarse como conflicto y quedarse. `setup` seguido de `--remove` MUST devolver cada archivo al mismo contenido JSON, con la sangría de dos espacios; un `mcp.json` que no existía queda como `{}` y un `hooks.json` que no existía como `{"version": 1}`.

**Tests:** `a_second_run_changes_nothing`, `dry_run_writes_nothing`, `remove_after_setup_gives_back_the_files`, `remove_leaves_a_foreign_server` (`setup/cursor.rs`); `setup_cursor_installs_and_remove_undoes_it` (`tests/binary.rs`).

## CU5. Escritura y reporte

Cada archivo MUST escribirse como en `setup claude-code` ([`specs/setup`](../../specs/setup/spec.md), R7): por un temporal en el directorio del archivo real y un `rename`, a través de un enlace simbólico, con el modo del archivo, con la copia previa en `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>` y la poda a las 5 más recientes de ese nombre. Un paso que falla no detiene el otro. El reporte MUST dar un renglón por archivo en el idioma preferido del perfil, con las rutas, el nombre del servidor y el evento tal cual. Al instalar, también con `--dry-run`, MUST avisar que en Cursor hay que apagar «Include Third-Party Plugins, Skills, and Other Configs» (Settings → Agents → Third-Party Imports), porque con eso encendido Cursor también carga los hooks y plugins de Claude Code y clawg no puede leer ese ajuste. El código de salida MUST ser 1 ante un conflicto, un archivo que no se pudo escribir o una copia que no se pudo borrar, y 0 si no; el aviso no lo cambia.

**Tests:** `apply_copies_the_previous_file_first`, `setup_warns_to_turn_off_third_party_configs`, `the_report_speaks_spanish` (`setup/cursor.rs`).

## CU6. `clawg hook session-start --client cursor`

`clawg hook` MUST tomar `--client claude-code|cursor`, con `claude-code` por defecto; sin `--client`, MUST hacer lo de [`specs/hooks`](../../specs/hooks/spec.md). Con `--client cursor`, `session-start` MUST leer `workspace_roots` y `session_id` de la entrada de Cursor, tolerando campos desconocidos y ausentes, MUST registrar la raíz y la sesión como R6 de `specs/hooks`, y MUST imprimir la salida de Cursor con el mismo texto que el sobre de Claude Code (R4, R7 y AL6 de `specs/hooks`), o nada cuando ese texto no existe. Con `--client cursor`, `user-prompt-submit` MUST NOT imprimir nada ni consultar al daemon.

#### Escenario: una raíz
- GIVEN un daemon con un save del agente `smoke` en el proyecto de `/work/repo`, y el hook corriendo en `~/.cursor`
- WHEN corre `clawg hook session-start --agent smoke --client cursor` con `workspace_roots` igual a `["/work/repo"]`
- THEN stdout es `{"additional_context": T}`, T trae ese save, y entre los proyectos del daemon no hay ninguno con raíz `~/.cursor`

**Tests:** `hook_takes_an_event_an_agent_and_a_profile` y `hook_takes_the_client_that_runs_it` (`cli.rs`), `the_hook_input_reads_what_cursor_sends`, `cursor_gets_additional_context`, `user_prompt_submit_says_nothing_to_cursor` (`hook.rs`), `a_cursor_session_start_reads_its_one_root_and_not_its_directory` (`tests/binary.rs`).

## CU7. La carpeta sale de la entrada

Con `--client cursor`, la carpeta del proyecto MUST ser la única raíz de `workspace_roots`, y MUST NOT salir del directorio actual. Con varias raíces o ninguna, el hook MUST NOT registrar ningún proyecto ni sesión y MUST imprimir sólo la línea del idioma, en la salida de Cursor, cuando hay preferencia, y nada si no.

#### Escenario: dos raíces
- GIVEN un daemon y una entrada de `sessionStart` con dos raíces
- WHEN corre `clawg hook session-start --agent smoke --client cursor`
- THEN stdout está vacío, sale con 0 y el daemon no tiene ningún proyecto

**Tests:** `cursor_reads_the_project_of_its_one_workspace_root`, `cursor_without_one_root_gets_only_the_language_line` (`hook.rs`), `a_cursor_session_with_two_roots_registers_no_project` (`tests/binary.rs`).

## CU8. Un hook de Claude Code que corre Cursor no hace nada

Sin `--client cursor`, una entrada con `cursor_version` (de cualquier tipo) MUST NOT imprimir nada, tampoco la línea del idioma, MUST NOT consultar al daemon ni registrar nada, y MUST salir con 0.

#### Escenario: Cursor corre el hook de Claude Code
- GIVEN la preferencia `es`, un daemon y la entrada de `sessionStart` de Cursor con una raíz
- WHEN corre `clawg hook session-start --agent claude-code` en `~/.cursor`
- THEN stdout está vacío, sale con 0 y el daemon no tiene ningún proyecto

**Tests:** `a_claude_code_hook_run_by_cursor_prints_nothing` (`hook.rs`), `a_claude_code_hook_that_cursor_runs_does_nothing` (`tests/binary.rs`).

## Verificación

En Linux, con directorios temporales, el embedder falso y `--port 0`. Lo que sólo Cursor reproduce queda en [`verify.md`](verify.md) como pendiente en local, con Cursor instalado y el perfil `dev`.
