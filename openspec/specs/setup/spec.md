# setup — Especificación

Fuente de verdad del comportamiento actual de `clawg setup <cliente>`: el subcomando que conecta clawg a un cliente de agentes en un paso que se puede repetir y deshacer. Hoy el único cliente es Claude Code. Registra el servidor MCP con la identidad del agente fijada ([`specs/visibility`](../visibility/spec.md), R1 y R6), instala los hooks de [`specs/hooks`](../hooks/spec.md) y deja el protocolo de memoria de [`specs/mcp`](../mcp/spec.md) en el archivo de instrucciones de usuario.

**Origen:**
- [client-setup](../../changes/archive/2026-10-19-client-setup/proposal.md), archivado el 2026-10-19. Los ids de requisito son los de esa spec; el porqué de cada decisión está en su [diseño](../../changes/archive/2026-10-19-client-setup/design.md).
- [setup-exec-hooks](../../changes/archive/2026-10-19-setup-exec-hooks/proposal.md), archivado el 2026-10-19: los hooks en forma exec y la migración de la forma heredada (R3, R6), la escritura por el destino de un enlace con el modo del archivo y la poda de copias (R7), y R8. El porqué, en su [diseño](../../changes/archive/2026-10-19-setup-exec-hooks/design.md): la forma de shell que reemplaza la había elegido client-setup porque `args` era nuevo, y un shell entre el cliente y clawg puede no ejecutar una ruta entre comillas (PowerShell, en Windows sin Git Bash) o imprimir antes del JSON del hook.
- [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23: el bloque del protocolo sigue la preferencia del perfil (LP13).
- [cli-language](../../changes/archive/2026-10-23-cli-language/proposal.md), archivado el 2026-10-23: el reporte en el idioma del perfil (R7).

## Definiciones

- **Directorio de configuración:** `$CLAUDE_CONFIG_DIR` si está definido y no vacío, si no `~/.claude`. Ahí viven `settings.json` y `CLAUDE.md` (`locations`, `crates/clawg-cli/src/setup.rs:628`).
- **Archivo de servidores:** `$CLAUDE_CONFIG_DIR/.claude.json` con la variable, si no `~/.claude.json`; los servidores de usuario están en `mcpServers` (verificado con Claude Code 2.1.303).
- **Servidor del perfil:** `clawg` para `prod`, `clawg-dev` para `dev` (`Profile::mcp_server_name`, `crates/clawg-core/src/paths.rs:120`).
- **Plan del servidor:** `command` = la ruta absoluta del binario en ejecución sin el prefijo `\\?\` (`current_exe`, `setup.rs:1098`), `args` = `mcp --profile P --agent claude-code` (`AGENT`, `setup.rs:31`).
- **Argumentos del hook de un evento y un perfil:** `hook <evento> --agent claude-code --profile P`, con `<evento>` `session-start` o `user-prompt-submit` (`hook_args`, `setup.rs:298`).
- **Hook del plan:** `{"type": "command", "command": <exe>, "args": <argumentos del hook>, "timeout": 5}`, con `<exe>` la ruta absoluta del binario tal como la da `current_exe`, sin cambiar separadores ni agregar comillas (`planned_hook`, `setup.rs:305`). El cliente lo corre sin shell: `args` existe desde Claude Code 2.1.139.
- **Hook de clawg de un evento y un perfil:** en forma exec, un hook con `args` cuyo `command` es un programa llamado `clawg` (o `clawg.exe`), cuyos dos primeros argumentos son `hook <evento>` y que lleva `--profile P` (o `--profile=P`); en forma heredada, un hook sin `args` cuyo `command`, partido como lo partiría un shell simple, cumple lo mismo (`is_clawg_hook`, `setup.rs:403`; `hook_words`, `setup.rs:391`).
- **Forma heredada idéntica:** una forma heredada cuyas palabras son exactamente `<clawg> hook <evento> --agent claude-code --profile P` y cuyo `command` es la cadena que generaban las versiones anteriores de `setup` para esa ruta: la ruta sin comillas si sólo tiene letras ASCII, dígitos y `/._-:`, si no entre comillas dobles con `"`, `$`, `` ` `` y `\` escapados, seguida de los argumentos separados por un espacio (`is_legacy_hook`, `setup.rs:413`; `legacy_command`, `setup.rs:312`, que sólo reconoce y ya no escribe). En Windows esas versiones ya habían pasado la ruta a `/`.
- **Bloque:** el texto de `crates/clawg-cli/src/mcp/protocol.md` entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->`.

## R1. `clawg setup claude-code`

`clawg setup claude-code [--profile P] [--dry-run] [--remove]` (`Command::Setup`, `crates/clawg-cli/src/cli.rs:174`) MUST resolver el perfil como los otros subcomandos (un build debug MUST rechazar `prod`), MUST rechazar una ruta del binario que no sea absoluta (`plan`, `setup.rs:201-203`), y MUST NOT abrir el store ni arrancar un daemon. Antes de escribir MUST leer los tres archivos; si uno no se puede leer, no es JSON válido o no tiene la forma que escribe Claude Code (los dos JSON, `parse_object`, `setup.rs:252`), o tiene un marcador sin su par (el `CLAUDE.md`, `find_block`, `setup.rs:570`), MUST fallar sin escribir ninguno.

**Tests:** `setup_takes_a_client_a_profile_dry_run_and_remove` (`cli.rs`), `a_relative_binary_path_is_refused`, `an_invalid_settings_file_writes_nothing`, `a_lone_marker_writes_nothing` (`setup.rs`).

## R2. El servidor MCP

Sin servidor del perfil en el archivo de servidores, el plan MUST ser `claude mcp add --scope user <servidor> -- <command> <args…>`, corrido con los argumentos como argv. Si existe y coincide (mismo `command` y `args`, `type` ausente o `stdio`; `matches_server`, `setup.rs:264`), MUST NOT hacer nada. Si existe y no coincide, MUST reportarlo como conflicto y MUST NOT reemplazarlo. El archivo de servidores MUST NOT escribirse desde clawg. Sin `claude` en el PATH (`find_claude`, `setup.rs:671`), MUST imprimir el comando exacto, MUST aplicar los otros pasos y MUST salir con 1.

#### Escenario: otro registro con el mismo nombre
- GIVEN un `.claude.json` con `mcpServers.clawg` apuntando a otro programa
- WHEN corre `clawg setup claude-code`
- THEN el reporte dice que el servidor está en conflicto, `claude mcp add` no se corre y sale con 1

**Tests:** `an_empty_state_plans_the_server_the_hooks_and_the_block`, `a_matching_server_is_left_alone`, `a_different_server_is_a_conflict`, `without_claude_the_command_is_printed` (`setup.rs`), `setup_without_claude_prints_the_command_and_installs_the_rest` y `setup_registers_through_a_fake_claude_and_remove_undoes_it` (`tests/binary.rs`, Unix).

## R3. Los hooks

El plan MUST agregar a `settings.json`, al final del arreglo de cada evento, un grupo con el hook del plan de `session-start` (matcher `startup|resume|clear|compact`) y otro con el de `user-prompt-submit` (sin matcher) (`install_hooks`, `setup.rs:459`). Un hook de clawg del mismo evento y perfil igual al del plan (mismo `command` y `args`) MUST quedar como está. Una forma heredada idéntica, con cualquier ruta de clawg, MUST reescribirse al hook del plan en su lugar dentro de su grupo, que conserva su posición y su matcher, y el reporte MUST decir `migrated`; si el evento ya tiene el hook del plan, la forma heredada idéntica MUST quitarse, y su grupo con ella si queda vacío (`migrate_hooks`, `setup.rs:502`). Cualquier otro hook de clawg del mismo evento y perfil, en cualquiera de las dos formas, MUST reportarse como conflicto, mostrado como `command` seguido de los `args`, y entonces MUST NOT cambiar nada de ese evento. Los hooks y las claves ajenas MUST conservarse, en su orden (`serde_json` con `preserve_order`).

#### Escenario: una ruta con espacio y no ASCII
- GIVEN el binario en `C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe` (o `/home/Ana María/bin/clawg` en Unix)
- WHEN se planea sobre un estado vacío
- THEN cada hook tiene `command` igual a esa ruta, sin comillas ni `/` en lugar de `\`, y `args` igual a los argumentos del hook

#### Escenario: la forma heredada se migra
- GIVEN un `settings.json` con la cadena que generaba la versión anterior, en un grupo con el matcher de clawg, para una ruta de clawg cualquiera
- WHEN corre `clawg setup claude-code`
- THEN ese hook pasa a ser el hook del plan en el mismo grupo, el reporte dice `migrated`, y una segunda corrida no escribe nada

#### Escenario: una cadena editada sigue en conflicto
- GIVEN una cadena de clawg del mismo evento y perfil con otro `--agent`, o con un argumento de más
- WHEN corre `clawg setup claude-code`
- THEN el reporte la da como conflicto, no se reemplaza, y sale con 1

**Tests:** `the_hook_runs_the_binary_without_a_shell`, `an_old_shell_hook_of_clawg_is_migrated_in_place`, `an_edited_shell_hook_is_still_a_conflict`, `a_different_clawg_hook_is_a_conflict`, `foreign_hooks_and_keys_keep_their_order`, `a_hook_of_the_other_profile_is_not_ours` (`setup.rs`).

## R4. El bloque del protocolo

El plan MUST agregar el bloque al final de `CLAUDE.md`, separado por una línea en blanco, o crear el archivo con él, con los finales de línea que el archivo ya usa (`install_block`, `setup.rs:586`). Un bloque idéntico MUST quedar como está y uno distinto MUST reescribirse; el texto fuera de los marcadores MUST NOT cambiar.

**Tests:** `the_block_is_appended_and_rewritten_in_place` (`setup.rs`).

## R5. Idempotencia y `--dry-run`

Una segunda corrida sobre el estado que dejó la primera MUST NOT escribir nada. `--dry-run` MUST reportar el mismo plan y MUST NOT escribir ningún archivo ni correr `claude mcp add|remove`.

**Tests:** `a_second_run_changes_nothing`, `dry_run_writes_nothing` (`setup.rs`), `setup_dry_run_writes_nothing` (`tests/binary.rs`).

## R6. `--remove`

`--remove` MUST quitar el servidor del perfil si es de clawg (`command` llamado `clawg` y primer argumento `mcp`; `is_clawg_server`, `setup.rs:273`), los hooks de clawg del perfil en las dos formas (`remove_hooks`, `setup.rs:522`) y el bloque (`remove_block`, `setup.rs:611`), salvo que el servidor del otro perfil siga en el archivo de servidores (`setup.rs:238-241`). Un servidor ajeno con el nombre del perfil MUST reportarse como conflicto y quedarse. `setup` seguido de `--remove` MUST devolver `CLAUDE.md` al texto de antes (un archivo que sólo tenía el bloque se borra, o queda vacío si es un enlace, R7) y `settings.json` al mismo contenido JSON, con la sangría de dos espacios; uno que no existía queda como `{}`. MUST NOT tocar el directorio de datos de clawg salvo las copias de R7 y su poda.

#### Escenario: las dos formas
- GIVEN un `settings.json` con un hook exec de `SessionStart` y una cadena heredada de `UserPromptSubmit`, los dos de clawg y del perfil, y un hook ajeno
- WHEN corre `clawg setup claude-code --remove`
- THEN quedan sólo el hook ajeno y las claves ajenas

**Tests:** `remove_after_setup_gives_back_the_files`, `remove_keeps_the_block_while_the_other_profile_is_registered`, `remove_leaves_a_foreign_server`, `remove_takes_both_forms` (`setup.rs`).

## R7. Escritura y reporte

Cada archivo MUST escribirse por un temporal en el directorio del archivo real y un `rename` (`write_file`, `setup.rs:789`; `replace`, `setup.rs:808`). Si la ruta es un enlace simbólico, el archivo real es su destino resuelto (`link_target`, `setup.rs:817`), y el enlace MUST seguir siendo enlace; borrar un archivo enlazado (un `CLAUDE.md` que sólo tenía el bloque) MUST dejarlo vacío en lugar de quitar el enlace o el destino, y un enlace roto MUST reportarse como un archivo que no se pudo escribir, sin tocarlo. En Unix, el temporal de un archivo que existía MUST tener el modo del archivo real desde que se crea (`create_fresh`, `setup.rs:857`). El temporal MUST sincronizarse a disco antes del `rename`. Un archivo que existía MUST copiarse antes a `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>`, con modo 0600 en Unix, y el reporte MUST nombrar la copia. Después de cada copia, MUST conservarse sólo las 5 más recientes de ese nombre (`KEPT_COPIES`, `setup.rs:51`; la elección es `stale_copies`, `setup.rs:748`, pura sobre los nombres), el reporte MUST nombrar cada copia borrada, y las copias de otro nombre y cualquier otro archivo del directorio MUST NOT tocarse. Un paso que falla no detiene los otros (`apply`, `setup.rs:708`). El reporte (`report`, `setup.rs:883`) da un renglón por paso, en el idioma preferido del perfil que `read_state` lee en `State.language` (en inglés sin preferencia; [`specs/preferences`](../preferences/spec.md), CL2 y CL3), con los comandos, las rutas y los nombres de servidor y de evento tal cual, y el código de salida MUST ser 1 ante un conflicto, el paso manual sin `claude`, un `claude` que falló, un archivo que no se pudo escribir o una copia que no se pudo borrar, y 0 si no.

#### Escenario: un `settings.json` enlazado
- GIVEN `settings.json` como enlace simbólico a un archivo de otro directorio, con modo 0600
- WHEN corre `clawg setup claude-code`
- THEN `settings.json` sigue siendo el mismo enlace, el archivo al que apunta tiene los hooks y conserva 0600, y no queda ningún temporal

#### Escenario: la poda
- GIVEN 6 copias de `settings.json` en `backups/setup`
- WHEN corre `clawg setup claude-code` y cambia `settings.json`
- THEN quedan las 5 más recientes, la nueva entre ellas, y el reporte nombra las borradas

**Tests:** `apply_copies_the_previous_file_first`, `a_linked_file_is_written_through_its_link` (con el enlace roto), `a_file_keeps_its_mode_and_copies_are_owner_only` (Unix), `only_the_newest_copies_are_kept`, `stale_copies_are_the_oldest_beyond_the_limit`, `the_setup_report_speaks_spanish` (`setup.rs`).

## R8. `clawg` sin subcomando

`clawg` sin ningún argumento MUST imprimir la ayuda en stderr y salir con 1, no con 2: es lo que corre un cliente anterior a 2.1.139, que no conoce `args` y ejecuta `command` solo, y 2 bloquea la sesión (`is_bare_invocation`, `crates/clawg-cli/src/cli.rs:344`; la rama de `Cli::try_parse` en `crates/clawg-cli/src/main.rs:44-56`). Los demás errores de uso MUST seguir saliendo con 2, salvo los de `clawg hook`, que salen con 1 ([`specs/hooks`](../hooks/spec.md), R3).

#### Escenario: un cliente que ignora `args`
- GIVEN un hook en forma exec y un cliente anterior a 2.1.139
- WHEN el cliente corre `command` sin `args`
- THEN `clawg` sale con 1 y la sesión no se bloquea

**Tests:** `a_bare_clawg_never_blocks` (`cli.rs`), `a_bare_clawg_exits_1` (`tests/binary.rs`).

## LP13. El bloque sigue la preferencia

`clawg setup claude-code` MUST escribir entre los marcadores el protocolo de [`specs/mcp`](../mcp/spec.md) LP9 para la preferencia del perfil (`protocol::text`; el archivo tal cual sin preferencia), que `read_state` lee en `State.language`. Con un bloque instalado para otro idioma MUST reescribirlo, y `--dry-run` MUST decirlo sin escribir.

**Test:** `the_block_follows_the_preferred_language` y `another_language_rewrites_the_block` (`setup.rs`, sobre `plan`).

## Verificación

Con el `claude` real en un home temporal y el perfil dev, en Linux (2.1.303 y 2.1.304) y en Windows (2.1.304): `claude mcp get` da el servidor conectado con la ruta absoluta y `--agent claude-code`, la forma heredada se migra y una segunda corrida no cambia nada, `clawg` a secas sale con 1 y `--remove` deja los archivos como estaban ([verify de client-setup](../../changes/archive/2026-10-19-client-setup/verify.md), [verify de setup-exec-hooks](../../changes/archive/2026-10-19-setup-exec-hooks/verify.md)). En Windows, con el binario en una ruta con un espacio y un carácter no ASCII, una sesión real de Claude Code recibió el contexto del hook exec. Quedan sin verificar Windows sin Git Bash (Claude Code 2.1.304 encuentra el de Git for Windows aunque no esté en el PATH), un archivo enlazado en Windows y un cliente anterior a 2.1.139.
