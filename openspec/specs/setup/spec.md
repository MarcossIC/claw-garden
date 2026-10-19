# setup — Especificación

Fuente de verdad del comportamiento actual de `clawg setup <cliente>`: el subcomando que conecta clawg a un cliente de agentes en un paso que se puede repetir y deshacer. Hoy el único cliente es Claude Code. Registra el servidor MCP con la identidad del agente fijada ([`specs/visibility`](../visibility/spec.md), R1 y R6), instala los hooks de [`specs/hooks`](../hooks/spec.md) y deja el protocolo de memoria de [`specs/mcp`](../mcp/spec.md) en el archivo de instrucciones de usuario.

**Origen:**
- [client-setup](../../changes/archive/2026-10-19-client-setup/proposal.md), archivado el 2026-10-19. Los ids de requisito son los de esa spec; el porqué de cada decisión está en su [diseño](../../changes/archive/2026-10-19-client-setup/design.md).

## Definiciones

- **Directorio de configuración:** `$CLAUDE_CONFIG_DIR` si está definido y no vacío, si no `~/.claude`. Ahí viven `settings.json` y `CLAUDE.md` (`locations`, `crates/clawg-cli/src/setup.rs:563`).
- **Archivo de servidores:** `$CLAUDE_CONFIG_DIR/.claude.json` con la variable, si no `~/.claude.json`; los servidores de usuario están en `mcpServers` (verificado con Claude Code 2.1.303).
- **Servidor del perfil:** `clawg` para `prod`, `clawg-dev` para `dev` (`Profile::mcp_server_name`, `crates/clawg-core/src/paths.rs:120`).
- **Plan del servidor:** `command` = la ruta absoluta del binario en ejecución sin el prefijo `\\?\` (`current_exe`, `setup.rs:810`), `args` = `mcp --profile P --agent claude-code` (`AGENT`, `setup.rs:27`).
- **Hook de clawg de un evento y un perfil:** un hook cuyo `command`, partido como lo partiría un shell simple, empieza por un programa llamado `clawg` (o `clawg.exe`), sigue con `hook <evento>` y lleva `--profile P` (`is_clawg_hook`, `setup.rs:371`).
- **Bloque:** el texto de `crates/clawg-cli/src/mcp/protocol.md` entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->`.

## R1. `clawg setup claude-code`

`clawg setup claude-code [--profile P] [--dry-run] [--remove]` (`Command::Setup`, `crates/clawg-cli/src/cli.rs:166`) MUST resolver el perfil como los otros subcomandos (un build debug MUST rechazar `prod`), MUST rechazar una ruta del binario que no sea absoluta (`plan`, `setup.rs:189-191`), y MUST NOT abrir el store ni arrancar un daemon. Antes de escribir MUST leer los tres archivos; si uno no se puede leer, no es JSON válido o no tiene la forma que escribe Claude Code (los dos JSON, `parse_object`, `setup.rs:242`), o tiene un marcador sin su par (el `CLAUDE.md`, `find_block`, `setup.rs:505`), MUST fallar sin escribir ninguno.

**Tests:** `setup_takes_a_client_a_profile_dry_run_and_remove` (`cli.rs`), `a_relative_binary_path_is_refused`, `an_invalid_settings_file_writes_nothing`, `a_lone_marker_writes_nothing` (`setup.rs`).

## R2. El servidor MCP

Sin servidor del perfil en el archivo de servidores, el plan MUST ser `claude mcp add --scope user <servidor> -- <command> <args…>`, corrido con los argumentos como argv. Si existe y coincide (mismo `command` y `args`, `type` ausente o `stdio`; `matches_server`, `setup.rs:254`), MUST NOT hacer nada. Si existe y no coincide, MUST reportarlo como conflicto y MUST NOT reemplazarlo. El archivo de servidores MUST NOT escribirse desde clawg. Sin `claude` en el PATH (`find_claude`, `setup.rs:593`), MUST imprimir el comando exacto, MUST aplicar los otros pasos y MUST salir con 1.

#### Escenario: otro registro con el mismo nombre
- GIVEN un `.claude.json` con `mcpServers.clawg` apuntando a otro programa
- WHEN corre `clawg setup claude-code`
- THEN el reporte dice que el servidor está en conflicto, `claude mcp add` no se corre y sale con 1

**Tests:** `an_empty_state_plans_the_server_the_hooks_and_the_block`, `a_matching_server_is_left_alone`, `a_different_server_is_a_conflict`, `without_claude_the_command_is_printed` (`setup.rs`), `setup_without_claude_prints_the_command_and_installs_the_rest` y `setup_registers_through_a_fake_claude_and_remove_undoes_it` (`tests/binary.rs`, Unix).

## R3. Los hooks

El plan MUST agregar a `settings.json`, al final del arreglo de cada evento, un grupo con un hook `command` de `<exe> hook session-start --agent claude-code --profile P` (matcher `startup|resume|clear|compact`) y otro de `<exe> hook user-prompt-submit --agent claude-code --profile P` (sin matcher), con `timeout` 5 (`install_hooks`, `setup.rs:421`). La ruta va en forma de shell: con `/` en Windows y entre comillas dobles sólo si tiene algo fuera de letras, dígitos y `/._-:` (`shell_word`, `setup.rs:299`). Un hook de clawg idéntico MUST quedar como está; uno de clawg distinto del mismo evento y perfil MUST reportarse como conflicto y MUST NOT reemplazarse. Los hooks y las claves ajenas MUST conservarse, en su orden (`serde_json` con `preserve_order`).

**Tests:** `foreign_hooks_and_keys_keep_their_order`, `a_different_clawg_hook_is_a_conflict`, `a_hook_of_the_other_profile_is_not_ours`, `the_hook_program_is_quoted_only_when_it_needs_it` (`setup.rs`).

## R4. El bloque del protocolo

El plan MUST agregar el bloque al final de `CLAUDE.md`, separado por una línea en blanco, o crear el archivo con él, con los finales de línea que el archivo ya usa (`install_block`, `setup.rs:521`). Un bloque idéntico MUST quedar como está y uno distinto MUST reescribirse; el texto fuera de los marcadores MUST NOT cambiar.

**Tests:** `the_block_is_appended_and_rewritten_in_place` (`setup.rs`).

## R5. Idempotencia y `--dry-run`

Una segunda corrida sobre el estado que dejó la primera MUST NOT escribir nada. `--dry-run` MUST reportar el mismo plan y MUST NOT escribir ningún archivo ni correr `claude mcp add|remove`.

**Tests:** `a_second_run_changes_nothing`, `dry_run_writes_nothing` (`setup.rs`), `setup_dry_run_writes_nothing` (`tests/binary.rs`).

## R6. `--remove`

`--remove` MUST quitar el servidor del perfil si es de clawg (`command` llamado `clawg` y primer argumento `mcp`; `is_clawg_server`, `setup.rs:263`), los hooks de clawg del perfil (`remove_hooks`, `setup.rs:458`) y el bloque (`remove_block`, `setup.rs:546`), salvo que el servidor del otro perfil siga en el archivo de servidores (`setup.rs:228`). Un servidor ajeno con el nombre del perfil MUST reportarse como conflicto y quedarse. `setup` seguido de `--remove` MUST devolver `CLAUDE.md` al texto de antes (un archivo que sólo tenía el bloque se borra) y `settings.json` al mismo contenido JSON, con la sangría de dos espacios; uno que no existía queda como `{}`. MUST NOT tocar el directorio de datos de clawg salvo las copias de R7.

**Tests:** `remove_after_setup_gives_back_the_files`, `remove_keeps_the_block_while_the_other_profile_is_registered`, `remove_leaves_a_foreign_server` (`setup.rs`).

## R7. Escritura y reporte

Cada archivo MUST escribirse por un temporal en su directorio y un `rename`. Un archivo que existía MUST copiarse antes a `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>`, y el reporte MUST nombrar la copia (`write_file`, `setup.rs:664`). Un paso que falla no detiene los otros (`apply`, `setup.rs:627`). El reporte (`report`, `setup.rs:694`) da un renglón por paso, y el código de salida MUST ser 1 ante un conflicto, el paso manual sin `claude`, un `claude` que falló o un archivo que no se pudo escribir, y 0 si no.

**Tests:** `apply_copies_the_previous_file_first` (`setup.rs`).

Verificado con Claude Code 2.1.303 en un home temporal y el perfil dev: `claude mcp get` da el servidor conectado con la ruta absoluta y `--agent claude-code`, una segunda corrida no cambia nada, el hook corre por `sh -c`, y `--remove` deja `CLAUDE.md` como estaba ([verify](../../changes/archive/2026-10-19-client-setup/verify.md)). Windows y una sesión real leyendo los hooks quedan sin verificar.
