# cli-language — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. `doctor` (CL1, CL2, CL4)

- [ ] Tests de `checks.rs` y `mcp.rs`: los chequeos reciben un `Lang`; los actuales pasan `Lang::En` sin cambiar sus textos; los de paridad comparan cada hallazgo en los dos idiomas salvo `message`.
- [ ] Un `match` sobre `Lang` por `message`; `doctor::run` usa el idioma que ya lee de `preferences.json`.
- Commit: `feat(doctor): write each finding's message in the preferred language`

## 2. `update` (CL3)

- [ ] Tests de `update/report.rs` y `update/mod.rs` en español, con la causa tal cual.
- [ ] `success`, `failure`, `install_dir_access` y el informe de `--check` con `Lang`.
- Commit: `feat(update): report in the preferred language`

## 3. `setup` (CL3)

- [ ] Test del informe en español: registro, ganchos, bloque, copias, conflicto y el `claude` ausente.
- [ ] `report` con `Lang`.
- Commit: `feat(setup): report in the preferred language`

## 4. `secrets` y `purge` (CL3, CL5)

- [ ] Tests de los informes y las negativas en español, con uno y con varios.
- [ ] `report`, `mask_report`, `purge_report`, `confirm`, `confirm_mask_reasons` y `explain` con `Lang`.
- Commit: `feat(secrets): report in the preferred language`

## 5. `export` e `import` (CL3, CL5)

- [ ] Tests de los dos informes en español, con uno y con varios.
- [ ] Sus `report` con `Lang`.
- Commit: `feat(portability): report exports and imports in the preferred language`

## 6. El resto de los comandos (CL3)

- [ ] Tests de `restore`, `undelete`, `outbox`, `projects`, `sharing`, `metrics`, `memories` en español.
- [ ] Sus frases con `Lang`; `stop` y `path` en `main.rs`.
- Commit: `feat(cli): speak the preferred language in the other commands`

## 7. El binario real (CL1, CL3, CL4)

- [ ] `doctor_speaks_the_preferred_language` y `stop_speaks_the_preferred_language` en `tests/binary.rs`.
- Commit: `test(cli): the commands in the preferred language through the real binary`

## 8. Documentación

- [ ] Las specs `preferences`, `doctor`, `setup`, `self-update`, `secrets`, `portability`; `.claude/CLAUDE.md`; el mapa de `openspec/README.md`.
- Commit: `docs: describe the commands in the preferred language`
