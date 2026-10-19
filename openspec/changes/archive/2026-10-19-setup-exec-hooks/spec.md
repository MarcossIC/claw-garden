# setup-exec-hooks — Especificación

Modifica [`specs/setup`](../../../specs/setup/spec.md): la definición de hook de clawg, R3, R6 y R7. Agrega R8 (`clawg` sin subcomando).

## Definiciones

- **Argumentos del hook de un evento y un perfil:** `hook <evento> --agent claude-code --profile P`, con `<evento>` `session-start` o `user-prompt-submit`.
- **Hook del plan:** `{"type": "command", "command": <exe>, "args": <argumentos del hook>, "timeout": 5}`, con `<exe>` la ruta absoluta del binario tal como la da `current_exe` (`setup.rs:810`), sin cambiar separadores ni agregar comillas.
- **Hook de clawg de un evento y un perfil, forma exec:** un hook con `args`, cuyo `command` es un programa llamado `clawg` (o `clawg.exe`), cuyos dos primeros argumentos son `hook <evento>` y que lleva `--profile P` (o `--profile=P`).
- **Hook de clawg de un evento y un perfil, forma heredada:** un hook sin `args` cuyo `command`, partido como lo partiría un shell simple, empieza por un programa llamado `clawg`, sigue con `hook <evento>` y lleva `--profile P` (la definición de hoy, `is_clawg_hook`, `setup.rs:371`).
- **Forma heredada idéntica:** una forma heredada cuyas palabras son exactamente `<clawg> hook <evento> --agent claude-code --profile P` y cuyo `command` es la cadena que la versión anterior de `setup` generaba para esa ruta: la ruta sin comillas si sólo tiene letras ASCII, dígitos y `/._-:`, si no entre comillas dobles con `"`, `$`, `` ` `` y `\` escapados, seguida de los argumentos separados por un espacio.

## R3. Los hooks (modifica R3 de `specs/setup`)

El plan MUST agregar a `settings.json`, al final del arreglo de cada evento, un grupo con el hook del plan de `session-start` (matcher `startup|resume|clear|compact`) y otro con el de `user-prompt-submit` (sin matcher). Un hook de clawg del mismo evento y perfil igual al del plan (mismo `command` y `args`) MUST quedar como está. Una forma heredada idéntica MUST reescribirse, en su lugar dentro de su grupo, al hook del plan; si el evento ya tiene el hook del plan, la forma heredada idéntica MUST quitarse. Cualquier otro hook de clawg del mismo evento y perfil, en cualquiera de las dos formas, MUST reportarse como conflicto, y entonces MUST NOT cambiar nada de ese evento. Los hooks y las claves ajenas MUST conservarse, en su orden.

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

## R6. `--remove` (modifica R6 de `specs/setup`)

`--remove` MUST quitar los hooks de clawg del perfil en las dos formas, además de lo que R6 ya quita. MUST NOT tocar el directorio de datos de clawg salvo las copias de R7 y su poda.

#### Escenario: las dos formas
- GIVEN un `settings.json` con un hook exec de `SessionStart` y una cadena heredada de `UserPromptSubmit`, los dos de clawg y del perfil, y un hook ajeno
- WHEN corre `clawg setup claude-code --remove`
- THEN quedan sólo el hook ajeno y las claves ajenas

**Tests:** `remove_takes_both_forms`, `remove_after_setup_gives_back_the_files` (`setup.rs`).

## R7. Escritura y reporte (modifica R7 de `specs/setup`)

Cada archivo MUST escribirse por un temporal en el directorio del archivo real y un `rename`. Si la ruta es un enlace simbólico, el archivo real es su destino resuelto, y el enlace MUST seguir siendo enlace; borrar un archivo enlazado (un `CLAUDE.md` que sólo tenía el bloque) MUST dejarlo vacío en lugar de quitar el enlace o el destino. En Unix, el temporal de un archivo que existía MUST tener el modo del archivo real desde que se crea. El temporal MUST sincronizarse a disco antes del `rename`. Un archivo que existía MUST copiarse antes a `<data>/backups/setup/<nombre>-<yyyyMMddHHmmssfff>`, con modo 0600 en Unix, y el reporte MUST nombrar la copia. Después de cada copia, MUST conservarse sólo las 5 más recientes de ese nombre, y el reporte MUST nombrar cada copia borrada. Lo demás de R7 no cambia.

#### Escenario: un `settings.json` enlazado
- GIVEN `settings.json` como enlace simbólico a un archivo de otro directorio, con modo 0600
- WHEN corre `clawg setup claude-code`
- THEN `settings.json` sigue siendo el mismo enlace, el archivo al que apunta tiene los hooks y conserva 0600, y no queda ningún temporal

#### Escenario: la poda
- GIVEN 6 copias de `settings.json` en `backups/setup`
- WHEN corre `clawg setup claude-code` y cambia `settings.json`
- THEN quedan las 5 más recientes, la nueva entre ellas, y el reporte nombra las borradas

**Tests:** `apply_copies_the_previous_file_first`, `a_linked_file_is_written_through_its_link`, `a_file_keeps_its_mode_and_copies_are_owner_only` (Unix), `only_the_newest_copies_are_kept`, `stale_copies_are_the_oldest_beyond_the_limit` (`setup.rs`).

## R8. `clawg` sin subcomando

`clawg` sin ningún argumento MUST imprimir la ayuda en stderr y salir con 1, no con 2: es lo que corre un cliente que no conoce `args`, y 2 bloquea la sesión. Los demás errores de uso MUST seguir saliendo con 2, salvo los de `clawg hook`, que siguen en 1.

#### Escenario: un cliente que ignora `args`
- GIVEN un hook en forma exec y un cliente anterior a 2.1.139
- WHEN el cliente corre `command` sin `args`
- THEN `clawg` sale con 1 y la sesión no se bloquea

**Tests:** `a_bare_clawg_never_blocks` (`cli.rs`), `a_bare_clawg_exits_1` (`tests/binary.rs`).
