# client-setup — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El plan puro (R1 a R6, D1 a D5, D8)

- [x] Tests primero en `setup.rs` y `cli.rs` (los de R1 a R6): no compilan.
- [x] `Command::Setup`, el módulo `setup` con `State`, `plan` y el reporte; `serde_json` con `preserve_order`.
- Commit: `feat(cli): add clawg setup claude-code` (con la tarea 2: el plan y su aplicación entraron en un solo commit, `385e011`)

## 2. Aplicar el plan (R2, R7, D2, D6, D7, D9)

- [x] Tests primero: `apply_copies_the_previous_file_first`, `without_claude_the_command_is_printed` contra directorios temporales.
- [x] Lectura del estado, `claude` en el PATH, escritura atómica con copia, `main`; `paths::home_dir` público.
- Commit: el de la tarea 1

## 3. El binario real

- [x] `tests/binary.rs`: `setup_dry_run_writes_nothing`, `setup_without_claude_prints_the_command_and_installs_the_rest` y `setup_registers_through_a_fake_claude_and_remove_undoes_it` (los dos últimos en Unix: un `claude` falso que anota sus argumentos).
- Commit: `test(cli): run clawg setup against a fake claude`

## 4. Docs

- [x] README (conectar con `clawg setup claude-code`, desinstalar con `--remove`), `.claude/CLAUDE.md`, `openspec/README.md`, `state.yaml`.
- Commit: `docs: describe clawg setup claude-code`

## 5. Verificación

- [x] Suite y clippy en Linux sobre el último commit.
- [x] Con Claude Code 2.1.303 en un home temporal y el perfil dev: ver el [verify](verify.md).
- [x] `verify.md`.
