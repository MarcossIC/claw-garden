# client-setup — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El plan puro (R1 a R6, D1 a D5, D8)

- [ ] Tests primero en `setup.rs` y `cli.rs` (los de R1 a R6): no compilan.
- [ ] `Command::Setup`, el módulo `setup` con `State`, `plan` y el reporte; `serde_json` con `preserve_order`.
- Commit: `feat(cli): plan clawg setup claude-code from the client's configuration`

## 2. Aplicar el plan (R2, R7, D2, D6, D7, D9)

- [ ] Tests primero: `apply_copies_the_previous_file_first`, `without_claude_the_command_is_printed` contra directorios temporales.
- [ ] Lectura del estado, `claude` en el PATH, escritura atómica con copia, `main`; `paths::home_dir` público.
- Commit: `feat(cli): apply clawg setup claude-code`

## 3. El binario real

- [ ] `tests/binary.rs`: `setup_without_claude_prints_the_command`, `setup_registers_through_a_fake_claude` (Unix: un `claude` falso que anota sus argumentos).
- Commit: `test(cli): run clawg setup against a fake claude`

## 4. Docs

- [ ] README (conectar con `clawg setup claude-code`, desinstalar con `--remove`), `.claude/CLAUDE.md`, `openspec/README.md`, `state.yaml`.
- Commit: `docs: describe clawg setup claude-code`

## 5. Verificación

- [ ] Suite y clippy en Linux sobre el último commit.
- [ ] `verify.md`.
