# setup-local-hooks — Spec

Requisitos que este cambio agrega a [`specs/setup`](../../../specs/setup/spec.md). Las definiciones (hook del plan, hook de clawg de un evento y un perfil, forma heredada idéntica) son las de esa spec.

## Definiciones

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

Con una carpeta destino, el plan MUST cambiar sólo el archivo local y MUST hacerlo con las reglas de R3 sobre ese archivo: los mismos dos grupos al final de cada evento, el hook igual al del plan sin cambios, la forma heredada idéntica migrada, otro hook de clawg del evento y del perfil reportado como conflicto, y las claves y hooks ajenos en su orden. MUST NOT correr `claude` ni escribir el archivo de servidores, el `settings.json` del usuario ni el `CLAUDE.md` del usuario. La carpeta `.claude` MUST crearse si falta.

#### Escenario: un repositorio sin `.claude`
- GIVEN una carpeta sin `.claude`
- WHEN corre `clawg setup claude-code --path <carpeta>`
- THEN existe `<carpeta>/.claude/settings.local.json` con el grupo de `SessionStart` (matcher `startup|resume|clear|compact`) y el de `UserPromptSubmit`, cada uno con el hook del plan, y el `settings.json` del usuario no cambió

**Tests:** `local_hooks_go_in_the_folder_settings_alone`, `a_different_clawg_hook_in_the_folder_is_a_conflict` (`setup/local.rs`), `setup_local_installs_in_the_folder_and_remove_undoes_it` (`tests/binary.rs`).

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
