# client-setup — Especificación

## Definiciones

- **Directorio de configuración:** `$CLAUDE_CONFIG_DIR` si está definido, si no `~/.claude`. Ahí viven `settings.json` y `CLAUDE.md`.
- **Archivo de servidores:** `$CLAUDE_CONFIG_DIR/.claude.json` si la variable está definida, si no `~/.claude.json`; los servidores de usuario están en `mcpServers`.
- **Servidor del perfil:** `clawg` para `prod`, `clawg-dev` para `dev` (`Profile::mcp_server_name`).
- **Plan del servidor:** `command` = la ruta absoluta del binario en ejecución, `args` = `mcp --profile P --agent claude-code`.
- **Hook de clawg de un evento y un perfil:** un hook cuyo `command` empieza por un programa llamado `clawg` (o `clawg.exe`), sigue con `hook <evento>` y lleva `--profile P`.
- **Bloque:** el texto de `protocol.md` entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->`.

## R1. `clawg setup claude-code`

`clawg setup claude-code [--profile P] [--dry-run] [--remove]` MUST resolver el perfil como los otros subcomandos (un build debug MUST rechazar `prod`), MUST rechazar una ruta del binario que no sea absoluta, y MUST NOT abrir el store ni arrancar un daemon. Antes de escribir MUST leer los tres archivos; si uno no se puede leer, no es JSON válido (los dos JSON) o tiene un marcador sin su par (el `CLAUDE.md`), MUST fallar sin escribir ninguno.

**Tests:** `setup_takes_a_client_a_profile_dry_run_and_remove` (`cli.rs`), `a_relative_binary_path_is_refused`, `an_invalid_settings_file_writes_nothing`, `a_lone_marker_writes_nothing` (`setup.rs`).

## R2. El servidor MCP

Sin servidor del perfil en el archivo de servidores, el plan MUST ser `claude mcp add --scope user <servidor> -- <command> <args…>`. Si existe y coincide (mismo `command` y `args`), MUST NOT hacer nada. Si existe y no coincide, MUST reportarlo como conflicto y MUST NOT reemplazarlo. Sin `claude` en el PATH, MUST imprimir el comando exacto, MUST NOT editar el archivo de servidores, MUST aplicar los otros pasos y MUST salir con 1.

#### Escenario: otro registro con el mismo nombre
- GIVEN un `.claude.json` con `mcpServers.clawg` apuntando a otro programa
- WHEN corre `clawg setup claude-code`
- THEN el reporte dice que el servidor está en conflicto, `claude mcp add` no se corre y sale con 1

**Tests:** `an_empty_state_plans_the_server_the_hooks_and_the_block`, `a_matching_server_is_left_alone`, `a_different_server_is_a_conflict`, `without_claude_the_command_is_printed` (`setup.rs`), `setup_without_claude_prints_the_command_and_installs_the_rest` y `setup_registers_through_a_fake_claude_and_remove_undoes_it` (`tests/binary.rs`, Unix).

## R3. Los hooks

El plan MUST agregar a `settings.json`, al final de cada arreglo, un grupo con un hook `command` de `<exe> hook session-start --agent claude-code --profile P` (matcher `startup|resume|clear|compact`) y otro de `<exe> hook user-prompt-submit --agent claude-code --profile P` (sin matcher), con `timeout` 5. Un hook de clawg idéntico MUST quedar como está; uno de clawg distinto del mismo evento y perfil MUST reportarse como conflicto y MUST NOT reemplazarse. Los hooks y las claves ajenas MUST conservarse, en su orden.

**Tests:** `foreign_hooks_and_keys_keep_their_order`, `a_different_clawg_hook_is_a_conflict`, `a_hook_of_the_other_profile_is_not_ours` (`setup.rs`).

## R4. El bloque del protocolo

El plan MUST agregar el bloque al final de `CLAUDE.md`, separado por una línea en blanco, o crear el archivo con él. Un bloque idéntico MUST quedar como está y uno distinto MUST reescribirse; el texto fuera de los marcadores MUST NOT cambiar.

**Tests:** `the_block_is_appended_and_rewritten_in_place` (`setup.rs`).

## R5. Idempotencia y `--dry-run`

Una segunda corrida sobre el estado que dejó la primera MUST NOT escribir nada. `--dry-run` MUST reportar el mismo plan y MUST NOT escribir ningún archivo ni correr `claude mcp add|remove`.

**Tests:** `a_second_run_changes_nothing`, `dry_run_writes_nothing` (`setup.rs`), `setup_dry_run_writes_nothing` (`tests/binary.rs`).

## R6. `--remove`

`--remove` MUST quitar el servidor del perfil si es de clawg (`command` llamado `clawg` y primer argumento `mcp`), los hooks de clawg del perfil y el bloque, salvo que el servidor del otro perfil siga en el archivo de servidores. Un servidor ajeno con el nombre del perfil MUST reportarse como conflicto y quedarse. `setup` seguido de `--remove` MUST devolver `CLAUDE.md` al texto de antes (un archivo que sólo tenía el bloque se borra) y `settings.json` al mismo contenido JSON, con la sangría de dos espacios que escribe Claude Code; uno que no existía queda como `{}`. MUST NOT tocar el directorio de datos de clawg salvo las copias de R7.

**Tests:** `remove_after_setup_gives_back_the_files`, `remove_keeps_the_block_while_the_other_profile_is_registered`, `remove_leaves_a_foreign_server` (`setup.rs`).

## R7. Escritura

Cada archivo MUST escribirse por un temporal en su directorio y un `rename`. Un archivo que existía MUST copiarse antes a `<data>/backups/setup/`, y el reporte MUST nombrar la copia.

**Tests:** `apply_copies_the_previous_file_first` (`setup.rs`).
