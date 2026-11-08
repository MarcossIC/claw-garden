# setup-local-hooks — Tareas

Strict TDD: los tests de cada tarea van primero y fallan, por la aserción o porque no compilan. Cada tarea es un commit convencional que compila y pasa los tests de los archivos que cambia. La suite entera y los dos clippy corren una vez, sobre el último commit.

## 1. Artefactos

- [x] `proposal.md`, `spec.md`, `tasks.md` y `state.yaml`; la fila en el mapa y el piloto del ROADMAP.
- Commit: `docs(openspec): propose setup-local-hooks`

## 2. Lo común de los hooks

Sin cambio de conducta: los tests de `setup.rs` y `setup/cursor.rs` pasan igual.

- [x] El plan de los hooks sobre un archivo (`plan_hooks`) sale de `plan`, y el renglón de los hooks sale de `report`, con el comando que quita un conflicto como parámetro.
- [x] `write_files` recibe el nombre de la copia de cada archivo.
- Commit: `refactor(setup): plan and report the hooks of one settings file on their own`

## 3. Los hooks de una carpeta (SL2 a SL6)

- [x] Tests en rojo en `setup/local.rs`: `local_hooks_go_in_the_folder_settings_alone`, `a_second_local_run_changes_nothing`, `a_different_clawg_hook_in_the_folder_is_a_conflict`, `remove_deletes_a_file_left_with_nothing`, `remove_keeps_what_is_not_clawg_s`, `remove_takes_away_the_emptied_claude_folder`, `the_copy_is_named_by_its_folder`, `the_report_says_when_git_does_not_ignore_the_file`, `git_says_whether_it_ignores_the_file`, `the_home_folder_and_a_missing_one_are_refused`, `the_local_report_speaks_spanish`.
- [x] `setup/local.rs`: estado, plan, escritura, reporte y `run`.
- Commit: `feat(setup): install Claude Code's hooks in one folder`

## 4. Las opciones (SL1)

- [x] Tests en rojo: `setup_takes_a_folder_for_claude_code_s_hooks` (`cli.rs`), `setup_local_installs_in_the_folder_and_remove_undoes_it` y `setup_cursor_refuses_a_folder` (`tests/binary.rs`).
- [x] `--local` y `--path` en `Command::Setup`; la elección en `main.rs`.
- Commit: `feat(cli): take --local and --path in setup claude-code`

## 5. Docs

- [x] `docs/reference/commands.md` y `README.md`.
- Commit: `docs: describe the hooks of setup claude-code in one folder`

## 6. Verificación

- [x] La suite y los dos clippy en Linux; `verify.md`.
- Commit: `docs(openspec): record that setup-local-hooks passes on Linux`

## 7. Archivo

- [x] SL1 a SL6 a `specs/setup`; la carpeta a `changes/archive/`; la fila al mapa de archivados; el minor sube.
- Commits: `docs(openspec): archive setup-local-hooks and fold it into the specs` y `chore: bump version to X.Y.Z`
