# setup — Especificación

Fuente de verdad del comportamiento actual de `clawg setup <cliente>`: el subcomando que conecta clawg a un cliente de agentes en un paso que se puede repetir y deshacer. Hoy el único cliente es Claude Code. Registra el servidor MCP con la identidad del agente fijada ([`specs/visibility`](../visibility/spec.md), R1 y R6), instala los hooks de [`specs/hooks`](../hooks/spec.md) y deja el protocolo de memoria de [`specs/mcp`](../mcp/spec.md) en el archivo de instrucciones de usuario.

**Origen:**
- [client-setup](../../changes/archive/2026-10-19-client-setup/proposal.md), archivado el 2026-10-19. Los ids de requisito son los de esa spec; el porqué de cada decisión está en su [diseño](../../changes/archive/2026-10-19-client-setup/design.md).
- [setup-exec-hooks](../../changes/archive/2026-10-19-setup-exec-hooks/proposal.md), archivado el 2026-10-19: los hooks en forma exec y la migración de la forma heredada (R3, R6), la escritura por el destino de un enlace con el modo del archivo y la poda de copias (R7), y R8. El porqué, en su [diseño](../../changes/archive/2026-10-19-setup-exec-hooks/design.md): la forma de shell que reemplaza la había elegido client-setup porque `args` era nuevo, y un shell entre el cliente y clawg puede no ejecutar una ruta entre comillas (PowerShell, en Windows sin Git Bash) o imprimir antes del JSON del hook.
- [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23: el bloque del protocolo sigue la preferencia del perfil (LP13).
- [cli-language](../../changes/archive/2026-10-23-cli-language/proposal.md), archivado el 2026-10-23: el reporte en el idioma del perfil (R7).
- [client-setup-cursor](../../changes/archive/2026-11-04-client-setup-cursor/proposal.md), archivado el 2026-11-04, sale en 0.76.0: `clawg setup cursor` (CU1 a CU5), que escribe el servidor en `~/.cursor/mcp.json` y el hook de `sessionStart` en `~/.cursor/hooks.json`, con la misma escritura, copia y reporte que R7. Lo que sólo Cursor real reproduce queda en prod ([pendiente](../../changes/archive/2026-11-04-client-setup-cursor/verify.md#pendiente-en-prod)).
- [setup-local-hooks](../../changes/archive/2026-11-08-setup-local-hooks/proposal.md), archivado el 2026-11-08, sale en 0.80.0: `clawg setup claude-code --local` y `--path DIR` (SL1 a SL6), que instalan sólo los hooks de Claude Code en `<carpeta>/.claude/settings.local.json`, con la misma escritura, copia y reporte que R7. Lo que sólo Claude Code real reproduce queda para el piloto ([pendiente](../../changes/archive/2026-11-08-setup-local-hooks/verify.md#pendiente-en-prod)).
- [session-usage](../../changes/archive/2026-11-11-session-usage/proposal.md), archivado el 2026-11-11, sale en 0.81.0: SU10 de esa spec, repartido en los requisitos que listan los hooks (R3 y R6 para Claude Code, CU1, CU3 a CU5 para Cursor y SL2 para `--local`) en lugar de una sección propia: el hook `SessionEnd` de Claude Code, con y sin `--local`, y `sessionEnd` de Cursor. Qué hace ese hook y qué cuenta el daemon con él, en [`specs/hooks`](../hooks/spec.md) (SU9) y [`specs/metrics`](../metrics/spec.md) (SU3 y SU10).
- [protocol-block](../../changes/archive/2026-11-17-protocol-block/proposal.md), archivado el 2026-11-17, que sale en 0.88.0: el sello con la versión del bloque (PB1), las variantes `full`, `short` y `off` (PB5) y `--protocol` (PB6). La frescura del bloque y su revisión están en [`specs/doctor`](../doctor/spec.md) (PB2 y PB3) y el aviso de `session-start` en [`specs/hooks`](../hooks/spec.md) (PB4); el brazo `a3-short`, en [`specs/eval`](../eval/spec.md) (PB7). La campaña que dejó el valor por defecto en `full` está en su [verify](../../changes/archive/2026-11-17-protocol-block/verify.md#la-campaña).
- [repeated-errors](../../changes/archive/2026-11-19-repeated-errors/proposal.md), archivado el 2026-11-19, que sale en 0.90.0: los hooks `PostToolUse` y `PostToolUseFailure`, con matcher `Bash`, en R3 y R6 (RE7), y en SL2 para `--local`. Qué hacen esos hooks está en [`specs/hooks`](../hooks/spec.md) (RE1 a RE6).
- [contract-diet](../../changes/archive/2026-11-20-contract-diet/proposal.md), archivado el 2026-11-20, sale en 0.92.0: la variante `lean` del bloque (CD1 y CD3), el sello ilegible cuando la variante es desconocida (CD4) y `lean` como valor por defecto (CD7). CD2 y CD6 no se consolidan aquí: son pruebas y lo que no cambia. Lo que se midió, en su [verify](../../changes/archive/2026-11-20-contract-diet/verify.md#la-campaña).

## Definiciones

- **Directorio de configuración:** `$CLAUDE_CONFIG_DIR` si está definido y no vacío, si no `~/.claude`. Ahí viven `settings.json` y `CLAUDE.md` (`locations`, `crates/clawg-cli/src/setup.rs:670`).
- **Archivo de servidores:** `$CLAUDE_CONFIG_DIR/.claude.json` con la variable, si no `~/.claude.json`; los servidores de usuario están en `mcpServers` (verificado con Claude Code 2.1.303).
- **Servidor del perfil:** `clawg` para `prod`, `clawg-dev` para `dev` (`Profile::mcp_server_name`, `crates/clawg-core/src/paths.rs:120`).
- **Plan del servidor:** `command` = la ruta absoluta del binario en ejecución sin el prefijo `\\?\` (`current_exe`, `setup.rs:1180`), `args` = `mcp --profile P --agent claude-code` (`AGENT`, `setup.rs:34`).
- **Argumentos del hook de un evento y un perfil:** `hook <evento> --agent claude-code --profile P`, con `<evento>` `session-start`, `user-prompt-submit`, `session-end`, `post-tool-use` o `post-tool-use-failure` (`hook_args`, `setup.rs:329`).
- **Hook del plan:** `{"type": "command", "command": <exe>, "args": <argumentos del hook>, "timeout": 5}`, con `<exe>` la ruta absoluta del binario tal como la da `current_exe`, sin cambiar separadores ni agregar comillas (`planned_hook`, `setup.rs:336`). El cliente lo corre sin shell: `args` existe desde Claude Code 2.1.139.
- **Hook de clawg de un evento y un perfil:** en forma exec, un hook con `args` cuyo `command` es un programa llamado `clawg` (o `clawg.exe`), cuyos dos primeros argumentos son `hook <evento>` y que lleva `--profile P` (o `--profile=P`); en forma heredada, un hook sin `args` cuyo `command`, partido como lo partiría un shell simple, cumple lo mismo (`is_clawg_hook`, `setup.rs:438`; `hook_words`, `setup.rs:426`).
- **Forma heredada idéntica:** una forma heredada cuyas palabras son exactamente `<clawg> hook <evento> --agent claude-code --profile P` y cuyo `command` es la cadena que generaban las versiones anteriores de `setup` para esa ruta: la ruta sin comillas si sólo tiene letras ASCII, dígitos y `/._-:`, si no entre comillas dobles con `"`, `$`, `` ` `` y `\` escapados, seguida de los argumentos separados por un espacio (`is_legacy_hook`, `setup.rs:453`; `legacy_command`, `setup.rs:360`, que sólo reconoce y ya no escribe). En Windows esas versiones ya habían pasado la ruta a `/`.
- **Bloque:** el texto de `crates/clawg-cli/src/mcp/protocol.md` entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->`, con el sello que sigue al marcador de apertura (`BEGIN_MARKER`, `END_MARKER`, `crates/clawg-cli/src/setup.rs:36-37`; `find_block`, `setup.rs:612`).
- **Sello:** la línea que sigue al marcador de apertura, `<!-- clawg X.Y.Z -->` o `<!-- clawg X.Y.Z protocol=V -->`, con `X.Y.Z` como lo lee `Version::parse` (`crates/clawg-cli/src/update/version.rs:21`) y `V` una de las variantes (PB1).
- **Variante:** `full`, `short`, `lean` u `off` (PB5).
- **Cuerpo:** el bloque sin el sello, con los finales de línea pasados a `\n`.
- **Versión propia:** la de `clawg_core::build_info::version()`.
- **Directorio de Cursor:** `~/.cursor`, con el home de `clawg_core::paths::home_dir` (el de `CLAWG_TEST_HOME` en un build de tests). Ahí viven `mcp.json` (los servidores de usuario, en `mcpServers`) y `hooks.json` (los hooks de usuario, en `hooks.<evento>`).
- **Servidor del plan de Cursor:** `{"type": "stdio", "command": <exe>, "args": ["mcp", "--profile", P, "--agent", "cursor"]}`, con `<exe>` la ruta absoluta del binario en ejecución sin el prefijo `\\?\`. Su nombre es el del perfil (`Profile::mcp_server_name`), igual que en Claude Code.
- **Argumentos del hook de Cursor de un evento:** `hook <evento> --agent cursor --client cursor --profile P`, con `<evento>` `session-start` (para `sessionStart`) o `session-end` (para `sessionEnd`) (`hook_args`, `crates/clawg-cli/src/setup/cursor.rs:140`).
- **Shell del hook de Cursor:** PowerShell en Windows, un shell POSIX en los demás (`Shell::HOST`, `crates/clawg-cli/src/setup/cursor.rs:47`).
- **Línea del hook de Cursor:** en un shell POSIX, la ruta del binario sin comillas si sólo tiene letras ASCII, dígitos y `/._-:`, si no entre comillas dobles con `"`, `$`, `` ` `` y `\` escapados por `\`, seguida de un espacio y los argumentos del hook. En PowerShell, `& "` y la ruta con `` ` ``, `$` y `"` escapados por `` ` ``, `" ` y los argumentos.
- **Hook del plan de Cursor:** `{"command": <línea del hook de Cursor>}`.
- **Hook de clawg de Cursor de un evento y del perfil:** una entrada de `hooks.sessionStart` (para `session-start`) o de `hooks.sessionEnd` (para `session-end`) cuyo `command`, partido como lo partiría el shell del hook (y sin un `&` inicial en PowerShell), empieza por un programa llamado `clawg` (o `clawg.exe`) seguido de `hook <evento>`, y lleva `--profile P` o `--profile=P`.

## R1. `clawg setup claude-code`

`clawg setup claude-code [--profile P] [--dry-run] [--remove]` (`Command::Setup`, `crates/clawg-cli/src/cli.rs:289`) MUST resolver el perfil como los otros subcomandos (un build debug MUST rechazar `prod`), MUST rechazar una ruta del binario que no sea absoluta (`exe_text`, que `plan` llama, `setup.rs:253-257`), y MUST NOT abrir el store ni arrancar un daemon. Antes de escribir MUST leer los tres archivos; si uno no se puede leer, no es JSON válido o no tiene la forma que escribe Claude Code (los dos JSON, `parse_object`, `setup.rs:283`), o tiene un marcador sin su par (el `CLAUDE.md`, `find_block`, `setup.rs:612`), MUST fallar sin escribir ninguno.

**Tests:** `setup_takes_a_client_a_profile_dry_run_and_remove` (`cli.rs`), `a_relative_binary_path_is_refused`, `an_invalid_settings_file_writes_nothing`, `a_lone_marker_writes_nothing` (`setup.rs`).

## R2. El servidor MCP

Sin servidor del perfil en el archivo de servidores, el plan MUST ser `claude mcp add --scope user <servidor> -- <command> <args…>`, corrido con los argumentos como argv. Si existe y coincide (mismo `command` y `args`, `type` ausente o `stdio`; `matches_server`, `setup.rs:295`), MUST NOT hacer nada. Si existe y no coincide, MUST reportarlo como conflicto y MUST NOT reemplazarlo. El archivo de servidores MUST NOT escribirse desde clawg. Sin `claude` en el PATH (`find_claude`, `setup.rs:713`), MUST imprimir el comando exacto, MUST aplicar los otros pasos y MUST salir con 1.

#### Escenario: otro registro con el mismo nombre
- GIVEN un `.claude.json` con `mcpServers.clawg` apuntando a otro programa
- WHEN corre `clawg setup claude-code`
- THEN el reporte dice que el servidor está en conflicto, `claude mcp add` no se corre y sale con 1

**Tests:** `an_empty_state_plans_the_server_the_hooks_and_the_block`, `a_matching_server_is_left_alone`, `a_different_server_is_a_conflict`, `without_claude_the_command_is_printed` (`setup.rs`), `setup_without_claude_prints_the_command_and_installs_the_rest` y `setup_registers_through_a_fake_claude_and_remove_undoes_it` (`tests/binary.rs`, Unix).

## R3. Los hooks

El plan MUST agregar a `settings.json`, al final del arreglo de cada evento, un grupo con el hook del plan de `session-start` (`SessionStart`, matcher `startup|resume|clear|compact`), otro con el de `user-prompt-submit` (`UserPromptSubmit`, sin matcher) y otro con el de `session-end` (`SessionEnd`, sin matcher). Desde RE7, MUST agregar también dos grupos con matcher `Bash`, uno con el hook del plan de `post-tool-use` (`PostToolUse`) y otro con el de `post-tool-use-failure` (`PostToolUseFailure`), que van después de los tres anteriores (`install_hooks`, `setup.rs:552`; los eventos, en `HOOK_EVENTS`, `setup.rs:56`). Desde summary-on-stop, MUST agregar al final un grupo sin matcher con el hook de `stop` (`Stop`, [`specs/hooks`](../hooks/spec.md), SS1); `clawg setup cursor` no lo instala. Sobre una instalación anterior, MUST agregar sólo los eventos que faltan y dejar byte a byte los hooks que no hace falta migrar: una instalación de los tres eventos de antes, en la forma exec, recibe sólo los dos de `Bash`. La forma heredada idéntica se migra igual, como dice lo que sigue. En `PostToolUse` y `PostToolUseFailure` sólo cuentan los hooks de un grupo que el cliente corre después de una llamada a Bash: sin matcher, con uno vacío o `*`, o con `Bash` entre sus alternativas (`runs_after_shell_call`). Un matcher de sólo letras, dígitos, `_`, `-`, espacios, `,` y `|` es, para el cliente, una lista de nombres exactos separados por `|` o `,` con espacios opcionales alrededor, así que `Edit| Bash` y `Edit, Bash` corren después de Bash; cualquier otro es una expresión regular, y se lee sólo por sus alternativas separadas por `|`, donde un espacio es un carácter más. Un hook de clawg bajo otro matcher, como `Edit`, no cuenta como instalado ni como conflicto, el plan agrega igual el grupo de `Bash`, y una migración no toca ese grupo. En los otros tres eventos cuenta el hook bajo cualquier matcher, así una instalación que le dio otro a `SessionStart` no recibe un segundo grupo que correría clawg dos veces. Un hook de clawg del mismo evento y perfil igual al del plan (mismo `command` y `args`) MUST quedar como está. Una forma heredada idéntica, con cualquier ruta de clawg, MUST reescribirse al hook del plan en su lugar dentro de su grupo, que conserva su posición y su matcher, y el reporte MUST decir `migrated`; si el evento ya tiene el hook del plan, la forma heredada idéntica MUST quitarse, y su grupo con ella si queda vacío (`migrate_hooks`, `setup.rs:544`). Cualquier otro hook de clawg del mismo evento y perfil, en cualquiera de las dos formas, MUST reportarse como conflicto, mostrado como `command` seguido de los `args`, y entonces MUST NOT cambiar nada de ese evento. Los hooks y las claves ajenas MUST conservarse, en su orden (`serde_json` con `preserve_order`).

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

#### Escenario: quien ya instaló los tres eventos (RE7)
- GIVEN un `settings.json` con los hooks de `SessionStart`, `UserPromptSubmit` y `SessionEnd` del plan
- WHEN corre `clawg setup claude-code`
- THEN el reporte dice que agrega `PostToolUse` y `PostToolUseFailure` y que los otros tres ya estaban, y lo que había queda igual byte a byte

**Tests:** `the_hook_runs_the_binary_without_a_shell`, `an_old_shell_hook_of_clawg_is_migrated_in_place`, `an_edited_shell_hook_is_still_a_conflict`, `a_different_clawg_hook_is_a_conflict`, `foreign_hooks_and_keys_keep_their_order`, `a_hook_of_the_other_profile_is_not_ours`, `a_run_over_an_install_without_the_session_end_hook_adds_only_it`, `a_run_over_an_install_without_the_tool_hooks_adds_only_them`, `the_tool_hooks_match_bash_only`, `a_tool_hook_counts_only_under_a_matcher_that_runs_after_a_shell_call` y `an_old_tool_hook_is_migrated_only_under_a_matcher_that_runs_after_a_shell_call` (`setup.rs`, RE7). Qué hace el hook de `SessionEnd` está en [`specs/hooks`](../hooks/spec.md), SU9, y qué cuenta el daemon con él, en [`specs/metrics`](../metrics/spec.md), SU10.

## R4. El bloque del protocolo

El plan MUST agregar el bloque al final de `CLAUDE.md`, separado por una línea en blanco, o crear el archivo con él, con los finales de línea que el archivo ya usa (`install_block`, `setup.rs:628`). Un bloque idéntico MUST quedar como está y uno distinto MUST reescribirse; el texto fuera de los marcadores MUST NOT cambiar. Idéntico es lo que dice PB6: un bloque que sólo difiere en la versión del sello queda como está.

**Tests:** `the_block_is_appended_and_rewritten_in_place` (`setup.rs`).

## R5. Idempotencia y `--dry-run`

Una segunda corrida sobre el estado que dejó la primera MUST NOT escribir nada. `--dry-run` MUST reportar el mismo plan y MUST NOT escribir ningún archivo ni correr `claude mcp add|remove`.

**Tests:** `a_second_run_changes_nothing`, `dry_run_writes_nothing` (`setup.rs`), `setup_dry_run_writes_nothing` (`tests/binary.rs`).

## R6. `--remove`

`--remove` MUST quitar el servidor del perfil si es de clawg (`command` llamado `clawg` y primer argumento `mcp`; `is_clawg_server`, `setup.rs:304`), los hooks de clawg del perfil de los cinco eventos (los de `Bash`, desde RE7, también), en las dos formas (`remove_hooks`, `setup.rs:615`) y el bloque (`remove_block`, `setup.rs:653`), salvo que el servidor del otro perfil siga en el archivo de servidores (`setup.rs:238-245`). Un servidor ajeno con el nombre del perfil MUST reportarse como conflicto y quedarse. `setup` seguido de `--remove` MUST devolver `CLAUDE.md` al texto de antes (un archivo que sólo tenía el bloque se borra, o queda vacío si es un enlace, R7) y `settings.json` al mismo contenido JSON, con la sangría de dos espacios; uno que no existía queda como `{}`. MUST NOT tocar el directorio de datos de clawg salvo las copias de R7 y su poda.

#### Escenario: las dos formas
- GIVEN un `settings.json` con un hook exec de `SessionStart` y una cadena heredada de `UserPromptSubmit`, los dos de clawg y del perfil, y un hook ajeno
- WHEN corre `clawg setup claude-code --remove`
- THEN quedan sólo el hook ajeno y las claves ajenas

**Tests:** `remove_after_setup_gives_back_the_files`, `remove_keeps_the_block_while_the_other_profile_is_registered`, `remove_leaves_a_foreign_server`, `remove_takes_both_forms` y `remove_takes_the_tool_hooks_too` (`setup.rs`).

## R7. Escritura y reporte

Cada archivo MUST escribirse por un temporal en el directorio del archivo real y un `rename` (`write_file`, `setup.rs:853`; `replace`, `setup.rs:892`). Si la ruta es un enlace simbólico, el archivo real es su destino resuelto (`link_target`, `setup.rs:881`), y el enlace MUST seguir siendo enlace; borrar un archivo enlazado (un `CLAUDE.md` que sólo tenía el bloque) MUST dejarlo vacío en lugar de quitar el enlace o el destino, y un enlace roto MUST reportarse como un archivo que no se pudo escribir, sin tocarlo. En Unix, el temporal de un archivo que existía MUST tener el modo del archivo real desde que se crea (`create_fresh`, `setup.rs:921`). El temporal MUST sincronizarse a disco antes del `rename`. Un archivo que existía MUST copiarse antes a `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>`, con modo 0600 en Unix, y el reporte MUST nombrar la copia. Después de cada copia, MUST conservarse sólo las 5 más recientes de ese nombre (`KEPT_COPIES`, `setup.rs:55`; la elección es `stale_copies`, `setup.rs:812`, pura sobre los nombres), el reporte MUST nombrar cada copia borrada, y las copias de otro nombre y cualquier otro archivo del directorio MUST NOT tocarse. Un paso que falla no detiene los otros (`apply`, `setup.rs:759`). El reporte (`report`, `setup.rs:947`) da un renglón por paso, en el idioma preferido del perfil que `read_state` lee en `State.language` (en inglés sin preferencia; [`specs/preferences`](../preferences/spec.md), CL2 y CL3), con los comandos, las rutas y los nombres de servidor y de evento tal cual, y el código de salida MUST ser 1 ante un conflicto, el paso manual sin `claude`, un `claude` que falló, un archivo que no se pudo escribir o una copia que no se pudo borrar, y 0 si no.

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

`clawg` sin ningún argumento MUST imprimir la ayuda en stderr y salir con 1, no con 2: es lo que corre un cliente anterior a 2.1.139, que no conoce `args` y ejecuta `command` solo, y 2 bloquea la sesión (`is_bare_invocation`, `crates/clawg-cli/src/cli.rs:987`; la rama de `Cli::parse_args` en `crates/clawg-cli/src/main.rs:53-61`). Los demás errores de uso MUST seguir saliendo con 2, salvo los de `clawg hook`, que salen con 1 ([`specs/hooks`](../hooks/spec.md), R3).

#### Escenario: un cliente que ignora `args`
- GIVEN un hook en forma exec y un cliente anterior a 2.1.139
- WHEN el cliente corre `command` sin `args`
- THEN `clawg` sale con 1 y la sesión no se bloquea

**Tests:** `a_bare_clawg_never_blocks` (`cli.rs`), `a_bare_clawg_exits_1` (`tests/binary.rs`).

## LP13. El bloque sigue la preferencia

`clawg setup claude-code` MUST escribir entre los marcadores el protocolo de [`specs/mcp`](../mcp/spec.md) LP9 para la preferencia del perfil (`protocol::text`; el archivo tal cual sin preferencia), que `read_state` lee en `State.language`. Con un bloque instalado para otro idioma MUST reescribirlo, y `--dry-run` MUST decirlo sin escribir.

**Test:** `the_block_follows_the_preferred_language` y `another_language_rewrites_the_block` (`setup.rs`, sobre `plan`).

## PB1. El sello

`setup claude-code` MUST escribir el sello en la línea siguiente al marcador de apertura, con la versión propia, y con ` protocol=V` sólo cuando la variante se eligió (PB6). Los dos marcadores MUST NOT cambiar, así que `find_block` y `remove_block` de un binario anterior encuentran y quitan un bloque con sello. Un bloque sin sello MUST leerse como uno sin versión y sin variante elegida. Un sello que no se puede leer (otra forma, una versión mal escrita, una variante desconocida) MUST leerse como si no estuviera, y la línea cuenta como parte del cuerpo.

#### Escenario: el sello de un bloque nuevo
- GIVEN un `CLAUDE.md` que no existe y la versión propia `0.88.0`
- WHEN `setup claude-code` planea instalar sin `--protocol`
- THEN el archivo empieza por `<!-- BEGIN CLAWG MEMORY PROTOCOL -->`, sigue con `<!-- clawg 0.88.0 -->` y después `# Claw Garden memory protocol`

#### Escenario: un bloque de antes del sello
- GIVEN un bloque sin sello
- WHEN se lee su sello
- THEN no tiene versión ni variante elegida

**Tests:** `the_block_carries_the_version_that_wrote_it` y `a_stamp_reads_its_version_and_its_choice` (`setup.rs`).

## PB5. Las variantes del bloque

`block` MUST armar, entre los marcadores y después del sello:

- `full`: `protocol::text(language)`, el protocolo entero (`crates/clawg-cli/src/mcp/protocol.rs:58`);
- `short`: el título del protocolo del idioma, la línea fija de [design](../../changes/archive/2026-11-17-protocol-block/design.md#d4-tres-variantes-de-un-solo-archivo) y las secciones «Session summary» y «After a compaction» (en español, «Resumen de sesión» y «Después de una compactación») tal como están en ese mismo texto, cada una hasta la sección siguiente;
- `lean`: `protocol::lean(language)`, el protocolo sin su apertura, con sus secciones (CD1);
- `off`: nada; el bloque son los dos marcadores y el sello.

El bloque `short` MUST ocupar menos de 2.000 bytes en los dos idiomas, y MUST NOT contener la apertura.

#### Escenario: el bloque corto en español
- GIVEN la preferencia `es`
- WHEN se arma el bloque `short`
- THEN contiene `## Resumen de sesión` con sus secciones (`Objetivo:` … `Validación:`) y `## Después de una compactación`
- AND no contiene `## Cómo escribir un save` ni la primera oración de la apertura

**Tests:** `the_short_protocol_keeps_the_session_summary_in_both_languages` y `the_short_protocol_has_no_opening` (`protocol.rs`); `each_variant_is_its_block` (`setup.rs`).

## PB6. `--protocol`

`clawg setup claude-code` MUST tomar `--protocol full|short|lean|off`. Con la opción MUST escribir esa variante, con `protocol=V` en el sello. Sin la opción MUST mantener la variante del sello del bloque instalado cuando trae `protocol=`, y si no, escribir la de por defecto sin `protocol=`. La de por defecto es `lean` desde 0.92.0 (CD7). Antes fue `full`, y la campaña que lo cambió está en su [verify](../../changes/archive/2026-11-20-contract-diet/verify.md#la-campaña). `--protocol` MUST ser un error de uso (clap, sale con 2) con `--remove`, con `--local` o `--path`. Con `setup cursor` MUST salir con 1, con `SetupError::ProtocolForCursor`, como el `FolderForCursor` de antes: no es un error de uso de clap, y no escribe nada. El reporte MUST nombrar la variante que quedó.

Con el bloque instalado igual al planeado salvo por la versión del sello, el paso MUST ser `Unchanged` y el sello MUST quedar como estaba (R4 y R5).

#### Escenario: elegir y conservar
- GIVEN un `CLAUDE.md` con el bloque `full` de antes del sello
- WHEN corre `clawg setup claude-code --protocol short --profile dev`
- THEN el bloque es el `short` con `protocol=short` en el sello, y el texto fuera de los marcadores no cambió
- AND una segunda corrida sin `--protocol` no escribe nada

#### Escenario: sin elección, la de por defecto
- GIVEN un bloque `full` sin `protocol=` en el sello
- WHEN corre `clawg setup claude-code` sin `--protocol`
- THEN el bloque es el de `lean`, sin `protocol=` en el sello (CD7)

#### Escenario: con `--local`
- WHEN corre `clawg setup claude-code --local --protocol short`
- THEN sale con un error de uso (2) y no escribe nada

#### Escenario: con Cursor
- WHEN corre `clawg setup cursor --protocol short`
- THEN sale con 1 y no escribe nada

**Tests:** `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it`, `a_new_version_with_the_same_text_rewrites_nothing` (`setup.rs`); `the_protocol_option_parses_and_excludes_local_remove_and_cursor` (`cli.rs`); `setup_protocol_short_writes_the_short_block` y `setup_cursor_refuses_a_protocol` (`tests/binary.rs`).

## CD1. El texto de `lean`

`protocol::lean(language)` MUST devolver, con finales de línea `\n`, el título de `protocol::text(language)` (su primera línea, sin la apertura), seguido de las secciones de `protocol.lean.md`, o de `protocol.lean.es.md` para `Some(Es)`. Con la preferencia `en`, la regla del idioma del save MUST quedar reescrita como la reescribe `text` (`SAVE_RULE`, `protocol.rs:28`), y MUST aparecer una vez en `protocol.lean.md`; la otra regla del idioma, la de la respuesta, va sólo en las instrucciones del servidor. `opening(lean(l))` MUST ser vacío para `None`, `Some(En)` y `Some(Es)`: la apertura la ponen en cada pedido las instrucciones del servidor, `opening(text(l))` ([D11](../../changes/archive/2026-11-20-contract-diet/design.md#d11-después-de-la-campaña-lean-sin-la-apertura)).

El texto MUST cumplir, en las dos lenguas:

- los mismos encabezados `## ` que `text(language)`, en el mismo orden;
- la viñeta de `topic_key` de `text(language)` entera, y las secciones «Session summary» y «After a compaction» (en español, «Resumen de sesión» y «Después de una compactación») iguales a las de `text(language)`;
- junto con las instrucciones del servidor, los mismos nombres que `text(language)` y los mismos marcadores de texto plano (`ACTIVE CONFLICTS`, `LAST SESSION`, `supersedes`, `conflicts`, `inferred`, `imported`, `conclusion`, `merge`, `apart`);
- como mucho el 70 % de los bytes de `text(language)`.

`protocol.md` y `protocol.es.md` MUST NOT cambiar: `text` y `short` devuelven lo mismo que antes.

#### Escenario: sin la apertura
- GIVEN cualquier preferencia de idioma, o ninguna
- WHEN se arma `lean`
- THEN `opening(lean)` es vacío, y la apertura es la que las instrucciones del servidor mandan en `initialize`

#### Escenario: lo que queda entero
- GIVEN la preferencia `es`
- WHEN se arma `lean`
- THEN contiene la viñeta de `topic_key` de `protocol.es.md`, `## Resumen de sesión` con `Objetivo:` … `Validación:` y `## Después de una compactación`, tal como están en `full`

#### Escenario: inglés por preferencia
- GIVEN la preferencia `en`
- WHEN se arma `lean`
- THEN dice «still in Spanish, search again with the query in Spanish.» en «How to write a save», y no dice «Save memories and search in English:»
- AND las instrucciones del servidor dicen «Save memories and search in English:» y «Reply to the user in English unless»

**Tests:** `the_lean_protocol_keeps_the_title_and_leaves_the_opening_to_the_servers_instructions`, `the_lean_protocol_keeps_the_topic_key_rule_and_the_session_summary`, `the_lean_protocol_and_the_servers_instructions_name_what_the_full_one_names`, `a_language_rewrites_the_save_rule_of_the_lean_protocol_and_the_server_says_the_other` y `the_lean_protocol_is_at_most_seventy_percent_of_the_full_one` (`protocol.rs`); las pruebas del protocolo de `server.rs` con `lean` (CD2).

## CD3. El bloque `lean` y su sello

`block` MUST armar, para `Variant::Lean`, `protocol::lean(language)` entre los marcadores, después del sello. `Variant::ALL` MUST ser `full`, `short`, `lean`, `off`, y `Variant::parse("lean")` MUST dar `Lean`. `setup claude-code --protocol lean` MUST escribir el bloque `lean` con `protocol=lean` en el sello; sin la opción, sobre un bloque con `protocol=lean`, MUST conservarlo (PB6). `freshness` MUST dar `current` a un bloque `lean` con su sello, en su idioma, y `stale` en el otro idioma. El error de uso de un `--protocol` desconocido MUST nombrar las cuatro variantes. El reporte MUST decir `(protocol=lean)`.

#### Escenario: elegir `lean` y conservarlo
- GIVEN un `CLAUDE.md` con el bloque `full` de antes del sello
- WHEN corre `clawg setup claude-code --protocol lean --profile dev`
- THEN el bloque es el de `lean` con `<!-- clawg X.Y.Z protocol=lean -->`, y el texto fuera de los marcadores no cambió
- AND una segunda corrida sin `--protocol` no escribe nada
- AND `clawg doctor --profile dev` da `protocol_block` `ok` `healthy`

**Tests:** `each_variant_is_its_block`, `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it` y `a_new_version_with_the_same_text_rewrites_nothing` (`setup.rs`); `a_variant_is_named_by_its_option` (`protocol.rs`); `the_protocol_option_parses_and_excludes_local_remove_and_cursor` (`cli.rs`); `doctor_marks_an_old_block_stale_without_a_preference` (`tests/binary.rs`).

## CD4. Un sello con una variante desconocida

Un sello cuya variante este binario no conoce (la `lean` leída por un binario anterior, o una variante posterior leída por este) MUST leerse como un sello ilegible (PB1): `Stamp::parse` da `None`, la línea cuenta como parte del cuerpo, y `freshness` MUST dar `stale`, no `newer`, aunque la versión del sello sea posterior a la propia, porque esa versión no se llega a leer. `setup` sin `--protocol` sobre ese bloque escribe la variante por defecto. El chequeo `doctor` de la frescura está en [`specs/doctor`](../doctor/spec.md) (PB2).

#### Escenario
- GIVEN un bloque con el sello `<!-- clawg 0.99.0 protocol=leaner -->` y la versión propia `0.88.0`
- WHEN se clasifica
- THEN es `stale`
- AND su sello se lee como ausente

**Tests:** `a_stamp_with_an_unknown_variant_reads_as_stale` (`setup.rs`).

## CD7. `lean` es el valor por defecto

`setup claude-code` sin `--protocol` MUST escribir `lean` sin `protocol=` en el sello cuando el bloque instalado no trae `protocol=`, y MUST mantener la variante elegida cuando lo trae (PB6). Un bloque `full` sin `protocol=` MUST leerse `stale` cuando su sello no tiene una versión posterior a la propia (también sin sello o con uno que no se lee), y `newer` cuando la tiene, como dice PB2 de [`specs/doctor`](../doctor/spec.md); en los dos casos `setup` sin la opción MUST reescribirlo como `lean`, porque el bloque difiere del planeado (R4). `--protocol full` MUST seguir eligiendo el entero. Este requisito reemplaza la frase de PB6 que fijaba `full` como la variante por defecto.

#### Escenario: una máquina con el bloque entero de antes
- GIVEN un `CLAUDE.md` con el bloque `full` sin `protocol=`
- WHEN corre `clawg setup claude-code --profile dev` sin `--protocol`
- THEN el bloque es el de `lean`, sin `protocol=` en el sello, y el texto fuera de los marcadores no cambió
- AND el reporte dice `(protocol=lean)`

**Tests:** `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it`, `a_stamp_with_an_unknown_variant_reads_as_stale`, `each_variant_is_its_block`, `a_block_is_current_stale_or_newer` y `the_block_follows_the_preferred_language` (`setup.rs`); `session_start_says_when_the_block_is_stale` (`hook.rs`).

## CU1. `clawg setup cursor`

`clawg setup cursor [--profile P] [--dry-run] [--remove]` MUST resolver el perfil como los otros subcomandos (un build debug MUST rechazar `prod`), MUST rechazar una ruta del binario que no sea absoluta y MUST NOT abrir el store ni arrancar un daemon. Antes de escribir MUST leer `mcp.json` y `hooks.json`; si uno no se puede leer, no es JSON válido o no tiene la forma que Cursor lee (un objeto; `mcpServers` y `hooks` objetos; `hooks.sessionStart` y `hooks.sessionEnd`, si están, listas), MUST fallar sin escribir ninguno. MUST NOT escribir un archivo de reglas ni ningún otro archivo de Cursor.

#### Escenario: un `hooks.json` roto
- GIVEN un `mcp.json` válido y un `hooks.json` que no es JSON
- WHEN corre `clawg setup cursor`
- THEN sale con error nombrando `hooks.json`, y ninguno de los dos archivos cambia ni aparece una copia

**Tests:** `setup_takes_a_client_a_profile_dry_run_and_remove` (`cli.rs`), `an_invalid_file_writes_nothing`, `a_relative_binary_path_is_refused` (`setup/cursor.rs`).

## CU2. El servidor en `mcp.json`

Sin servidor del perfil, el plan MUST agregar el servidor del plan de Cursor al final de `mcpServers`, que se crea si falta. Si existe y coincide (mismo `command` y `args`, `type` ausente o `stdio`), MUST NOT hacer nada. Si existe y no coincide, MUST reportarlo como conflicto y MUST NOT reemplazarlo. Los otros servidores y las otras claves MUST conservarse, en su orden.

#### Escenario: otros servidores
- GIVEN un `mcp.json` con dos servidores ajenos y una clave ajena
- WHEN corre `clawg setup cursor` y después `clawg setup cursor --remove`
- THEN después de la primera los dos servidores y la clave siguen iguales y en su orden, con `clawg-dev` al final, y después de la segunda el archivo es el de antes

**Tests:** `an_empty_state_plans_the_server_and_the_hook`, `a_different_server_is_a_conflict`, `other_servers_stay_and_remove_gives_the_file_back` (`setup/cursor.rs`).

## CU3. El hook en `hooks.json`

El plan MUST agregar el hook del plan de Cursor de `session-start` al final de `hooks.sessionStart` y el de `session-end` al final de `hooks.sessionEnd`, creando lo que falte; un `hooks.json` que no existía (o vacío) MUST empezar con `"version": 1`. MUST NOT instalar ningún otro evento, `beforeSubmitPrompt` incluido. Cada evento tiene su paso y su línea en el reporte, y se decide por separado: un hook igual al del plan MUST quedar como está; otro hook de clawg de Cursor del evento y del perfil MUST reportarse como conflicto, mostrado como su `command`, y MUST NOT reemplazarse, y el otro evento MUST instalarse igual. Sobre una instalación anterior, MUST agregar sólo el evento que falta y dejar los demás byte a byte. Los hooks y las claves ajenas, y los hooks de clawg de otro perfil, MUST conservarse, en su orden. `hook_installed`, con lo que `doctor` dice que el hook instalado no dejó corridas, mira sólo `sessionStart`, el único que las deja. Qué hace el hook de `sessionEnd` está en [`specs/hooks`](../hooks/spec.md), SU9.

#### Escenario: una ruta con espacio
- GIVEN el binario en `C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe`
- WHEN se arma la línea del hook de Cursor para PowerShell
- THEN es `& "C:\Users\Ana María\AppData\Local\Programs\clawg\clawg.exe" hook session-start --agent cursor --client cursor --profile dev`, y partida como la parte PowerShell da esa ruta y los argumentos del hook

#### Escenario: un hook de clawg distinto
- GIVEN un `hooks.json` con `clawg hook session-start --agent claude-code --profile dev` en `sessionStart`
- WHEN corre `clawg setup cursor --profile dev`
- THEN el reporte lo da como conflicto en `sessionStart`, que no cambia, `sessionEnd` se instala igual y sale con 1

**Tests:** `the_hook_line_is_written_for_the_shell_that_runs_it`, `a_hook_line_reads_back_as_its_words`, `a_different_clawg_hook_is_a_conflict`, `foreign_hooks_and_keys_keep_their_order`, `a_hook_of_the_other_profile_is_not_ours`, `nothing_is_installed_before_a_prompt`, `a_run_over_an_install_without_the_session_end_hook_adds_only_it`, `the_session_start_hook_of_this_profile_is_found_in_hooks_json` (`setup/cursor.rs`); `the_installed_cursor_hook_runs_through_a_shell` (`tests/binary.rs`, Unix: la línea que quedó en `hooks.json`, corrida con `sh -c` en `~/.cursor` y la entrada de Cursor, imprime la salida de Cursor).

## CU4. Idempotencia, `--dry-run` y `--remove`

Una segunda corrida sobre lo que dejó la primera MUST NOT escribir nada. `--dry-run` MUST reportar el mismo plan y MUST NOT escribir nada. `--remove` MUST quitar el servidor del perfil si es de clawg (`command` llamado `clawg` y primer argumento `mcp`), y `mcpServers` si queda vacío, y los hooks de clawg de Cursor del perfil de los dos eventos, y `hooks.sessionStart`, `hooks.sessionEnd` y `hooks` si quedan vacíos. Un servidor ajeno con el nombre del perfil MUST reportarse como conflicto y quedarse. `setup` seguido de `--remove` MUST devolver cada archivo al mismo contenido JSON, con la sangría de dos espacios; un `mcp.json` que no existía queda como `{}` y un `hooks.json` que no existía como `{"version": 1}`.

**Tests:** `a_second_run_changes_nothing`, `dry_run_writes_nothing`, `remove_after_setup_gives_back_the_files`, `remove_leaves_a_foreign_server`, `remove_takes_each_hook_that_is_there` (`setup/cursor.rs`); `setup_cursor_installs_and_remove_undoes_it` (`tests/binary.rs`).

## CU5. Escritura y reporte de Cursor

Cada archivo MUST escribirse como en `setup claude-code` (R7): por un temporal en el directorio del archivo real y un `rename`, a través de un enlace simbólico, con el modo del archivo, con la copia previa en `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>` y la poda a las 5 más recientes de ese nombre. Un paso que falla no detiene los otros. El reporte MUST dar un renglón por el servidor y uno por cada evento de hook (`sessionStart`, `sessionEnd`), en el idioma preferido del perfil, con las rutas, el nombre del servidor y el evento tal cual. Al instalar, también con `--dry-run`, MUST avisar que en Cursor hay que apagar «Include Third-Party Plugins, Skills, and Other Configs» (Settings → Agents → Third-Party Imports), porque con eso encendido Cursor también carga los hooks y plugins de Claude Code y clawg no puede leer ese ajuste. El código de salida MUST ser 1 ante un conflicto, un archivo que no se pudo escribir o una copia que no se pudo borrar, y 0 si no; el aviso no lo cambia.

**Tests:** `apply_copies_the_previous_file_first`, `setup_warns_to_turn_off_third_party_configs`, `the_report_speaks_spanish` (`setup/cursor.rs`).

## Definiciones de los hooks por carpeta

- **Carpeta destino:** la carpeta de trabajo del proceso con `--local`, o `DIR` con `--path DIR` (relativa a la carpeta de trabajo si no es absoluta).
- **Archivo local:** `<carpeta destino>/.claude/settings.local.json`.
- **Copia del archivo local:** `<data>/backups/setup/<nombre>.settings.local.json-<yyyyMMddHHmmssfff>`, con `<nombre>` el último componente de la carpeta destino (`settings.local.json-<stamp>` si no tiene).

## SL1. Las opciones

`clawg setup claude-code --local` y `clawg setup claude-code --path DIR` MUST instalar en la carpeta destino. Juntas MUST ser un error de uso. Cualquiera de las dos con `setup cursor` MUST fallar sin escribir nada, con un mensaje que nombre `clawg setup cursor`. Una carpeta destino que no existe o no es una carpeta MUST rechazarse sin escribir nada, igual que la carpeta personal (el home de `clawg_core::paths::home_dir`, comparado después de resolver los enlaces). Como R1, MUST NOT abrir el store ni arrancar un daemon.

#### Escenario: la carpeta personal
- GIVEN una terminal abierta en la carpeta personal
- WHEN corre `clawg setup claude-code --local`
- THEN sale con 1, dice que corra el comando en la carpeta del proyecto o pase `--path`, y no escribe nada

**Tests:** `setup_takes_a_folder_for_claude_code_s_hooks` (`cli.rs`), `the_home_folder_and_a_missing_one_are_refused` (`setup/local.rs`), `setup_cursor_refuses_a_folder` (`tests/binary.rs`).

## SL2. Lo que se escribe

Con una carpeta destino, el plan MUST cambiar sólo el archivo local y MUST hacerlo con las reglas de R3 sobre ese archivo: los mismos grupos de R3 al final de cada evento (`SessionStart`, `UserPromptSubmit`, `SessionEnd`, y desde RE7 `PostToolUse` y `PostToolUseFailure`, con matcher `Bash`), el hook igual al del plan sin cambios, la forma heredada idéntica migrada, otro hook de clawg del evento y del perfil reportado como conflicto, y las claves y hooks ajenos en su orden. MUST NOT correr `claude` ni escribir el archivo de servidores, el `settings.json` del usuario ni el `CLAUDE.md` del usuario. La carpeta `.claude` MUST crearse si falta.

#### Escenario: un repositorio sin `.claude`
- GIVEN una carpeta sin `.claude`
- WHEN corre `clawg setup claude-code --path <carpeta>`
- THEN existe `<carpeta>/.claude/settings.local.json` con el grupo de `SessionStart` (matcher `startup|resume|clear|compact`), el de `UserPromptSubmit`, el de `SessionEnd` (los dos sin matcher), y los de `PostToolUse` y `PostToolUseFailure` (matcher `Bash`), cada uno con el hook del plan, y el `settings.json` del usuario no cambió

**Tests:** `local_hooks_go_in_the_folder_settings_alone`, `a_different_clawg_hook_in_the_folder_is_a_conflict`, `a_local_run_over_an_install_without_the_session_end_hook_adds_only_it` (`setup/local.rs`, con los cinco eventos desde RE7), `setup_local_installs_in_the_folder_and_remove_undoes_it` (`tests/binary.rs`).

## SL3. `--remove` con una carpeta

`--remove` con una carpeta destino MUST quitar del archivo local los hooks de clawg del perfil en las dos formas, con los grupos y eventos que queden vacíos, y MUST conservar lo ajeno. Si después no queda ninguna clave, el archivo MUST borrarse (o vaciarse si es un enlace, como R7), y la carpeta `.claude` MUST borrarse si queda vacía. Un archivo sin hooks de clawg del perfil MUST quedar como está.

#### Escenario: instalar y quitar
- GIVEN una carpeta sin `.claude`
- WHEN corre `clawg setup claude-code --local` y después `clawg setup claude-code --local --remove` en ella
- THEN la carpeta queda sin `.claude`

**Tests:** `remove_deletes_a_file_left_with_nothing`, `remove_keeps_what_is_not_clawg_s`, `remove_takes_away_the_emptied_claude_folder` (`setup/local.rs`), `setup_local_installs_in_the_folder_and_remove_undoes_it` (`tests/binary.rs`).

## SL4. Escritura y copias

El archivo local MUST escribirse como R7: por un temporal y un `rename`, a través de un enlace, con el modo del archivo que reemplaza. Si existía, MUST copiarse antes a la copia del archivo local. MUST conservarse sólo las 5 copias más recientes de ese nombre, así la copia de una carpeta no poda la de otra con otro nombre.

**Tests:** `the_copy_is_named_by_its_folder` (`setup/local.rs`).

## SL5. El aviso de git

Al instalar, también con `--dry-run`, si `git check-ignore` dice que el archivo local no está ignorado, el reporte MUST decirlo y nombrar `.claude/settings.local.json` como la entrada que falta en `.gitignore`. El aviso MUST NOT cambiar el código de salida. Si git no está, o la carpeta no está en un repositorio, el reporte MUST NOT decir nada de git.

**Tests:** `the_report_says_when_git_does_not_ignore_the_file`, `git_says_whether_it_ignores_the_file` (`setup/local.rs`).

## SL6. Reporte, idempotencia y `--dry-run`

El reporte MUST dar el renglón de los hooks de R7 con la ruta del archivo local, las copias, las copias borradas y las fallas, en el idioma del perfil. En un conflicto, MUST nombrar `clawg setup claude-code --path <carpeta destino> --remove` como el comando que lo quita, y salir con 1. Una segunda corrida MUST NOT escribir nada, y `--dry-run` MUST reportar el mismo plan sin escribir.

**Tests:** `a_second_local_run_changes_nothing`, `a_different_clawg_hook_in_the_folder_is_a_conflict`, `the_local_report_speaks_spanish` (`setup/local.rs`).

## Verificación

Con el `claude` real en un home temporal y el perfil dev, en Linux (2.1.303 y 2.1.304) y en Windows (2.1.304): `claude mcp get` da el servidor conectado con la ruta absoluta y `--agent claude-code`, la forma heredada se migra y una segunda corrida no cambia nada, `clawg` a secas sale con 1 y `--remove` deja los archivos como estaban ([verify de client-setup](../../changes/archive/2026-10-19-client-setup/verify.md), [verify de setup-exec-hooks](../../changes/archive/2026-10-19-setup-exec-hooks/verify.md)). En Windows, con el binario en una ruta con un espacio y un carácter no ASCII, una sesión real de Claude Code recibió el contexto del hook exec. Quedan sin verificar Windows sin Git Bash (Claude Code 2.1.304 encuentra el de Git for Windows aunque no esté en el PATH), un archivo enlazado en Windows y un cliente anterior a 2.1.139. CU1 a CU5 se verificaron en Linux y en el CI de Windows con el embedder falso; con Cursor real, la prueba queda en prod ([verify de client-setup-cursor](../../changes/archive/2026-11-04-client-setup-cursor/verify.md#pendiente-en-prod)). SL1 a SL6 se verificaron en Linux con el embedder falso; el CI de Windows y una sesión real de Claude Code en la carpeta quedan pendientes ([verify de setup-local-hooks](../../changes/archive/2026-11-08-setup-local-hooks/verify.md)).
