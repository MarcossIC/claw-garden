# setup-local-hooks — Verificación

## Linux (2026-11-08)

En Linux x86_64, build debug, toolchain 1.99.1, sobre el commit de la documentación (`144925b`):

- `cargo test --workspace` pasa entero.
- `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings` salen con 0, sin ningún warning.

Cada requisito tiene sus tests:

| Requisito | Tests |
|---|---|
| SL1 | `setup_takes_a_folder_for_claude_code_s_hooks` (`cli.rs`), `the_home_folder_and_a_missing_one_are_refused` (`setup/local.rs`), `setup_cursor_refuses_a_folder` y la parte final de `setup_local_installs_in_the_folder_and_remove_undoes_it` (`tests/binary.rs`) |
| SL2 | `local_hooks_go_in_the_folder_settings_alone`, `a_different_clawg_hook_in_the_folder_is_a_conflict` (`setup/local.rs`), `setup_local_installs_in_the_folder_and_remove_undoes_it` (`tests/binary.rs`) |
| SL3 | `remove_deletes_a_file_left_with_nothing`, `remove_keeps_what_is_not_clawg_s`, `remove_takes_away_the_emptied_claude_folder` (`setup/local.rs`), `setup_local_installs_in_the_folder_and_remove_undoes_it` (`tests/binary.rs`) |
| SL4 | `the_copy_is_named_by_its_folder` (`setup/local.rs`) |
| SL5 | `the_report_says_when_git_does_not_ignore_the_file`, `git_says_whether_it_ignores_the_file` (`setup/local.rs`) |
| SL6 | `a_second_local_run_changes_nothing`, `a_different_clawg_hook_in_the_folder_is_a_conflict`, `the_local_report_speaks_spanish` (`setup/local.rs`) |

Los tests de `setup.rs` y de `setup/cursor.rs` pasan sin cambios después de sacar lo común (`plan_hooks`, `hooks_line` y el nombre de la copia en `write_files`).

A mano, con el binario debug, un home temporal (`CLAWG_TEST_HOME`), `CLAUDE_CONFIG_DIR` dentro de él, la preferencia `es` y una carpeta con `git init`:

1. `--local --dry-run` da el plan y el aviso de git, y no escribe nada.
2. `--local` escribe los dos grupos en `.claude/settings.local.json`.
3. Una segunda corrida dice `ya están` y no escribe nada.
4. `--local --remove` quita los hooks, borra el archivo y la carpeta `.claude`, y copia el archivo a `mi-repo.settings.local.json-<stamp>`.
5. En el home, `--local` sale con 1, y `setup cursor --local` también.

## Pendiente en el CI de Windows

El CI no corre en un push a `develop`. Los tests de este cambio no tienen ninguna parte sólo de Unix, así que el CI del PR que lleve `develop` a `main` los corre también en `windows-latest`. Ahí se ve la ruta resuelta sin el prefijo `\\?\`, que en Linux no aparece.

## Pendiente en prod

En el piloto del [ROADMAP](../../ROADMAP.md), con la versión publicada y el servidor ya registrado para el usuario, en Windows:

1. `clawg setup claude-code --local --profile prod`, corrido en la carpeta del repositorio, deja los hooks en su `.claude/settings.local.json`, y una sesión nueva de Claude Code abierta en esa carpeta recibe el contexto: `clawg doctor` lo muestra en `hooks.runs`, con cliente `claude-code`.
2. Con el archivo sin ignorar, el reporte avisa. Si Claude Code ya agregó `**/.claude/settings.local.json` a las exclusiones globales de git, no avisa.
3. `clawg setup claude-code --local --remove --profile prod` deja la carpeta como estaba, y la sesión siguiente arranca sin el contexto de clawg.
