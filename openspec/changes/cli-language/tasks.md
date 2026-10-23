# cli-language — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. `doctor` (CL1, CL2, CL4)

- [x] Tests de `checks.rs` y `mcp.rs`: los chequeos reciben un `Lang`; los actuales pasan `Lang::En` sin cambiar sus textos; los de paridad comparan cada hallazgo en los dos idiomas salvo `message`.
- [x] Un `match` sobre `Lang` por `message`; `doctor::run` usa el idioma que ya lee de `preferences.json`.
- Commit: `feat(doctor): write each finding's message in the preferred language`

## 2. `update` (CL3)

- [x] Tests de `update/report.rs` y `update/mod.rs` en español, con la causa tal cual.
- [x] `success`, `failure` y el informe de `--check` con `Lang`; `install_dir_access` es la causa de un paso y queda en inglés.
- Commit: `feat(update): report in the preferred language`

## 3. `setup` (CL3)

- [x] Test del informe en español: registro, ganchos, bloque, copias, conflicto y el `claude` ausente.
- [x] `report` con `Lang`.
- Commit: `feat(setup): report in the preferred language`

## 4. `secrets` y `purge` (CL3, CL5)

- [x] Tests de los informes y las negativas en español, con uno y con varios.
- [x] `report`, `mask_report`, `purge_report`, `confirm`, `confirm_mask_reasons` y `explain` con `Lang`.
- Commit: `feat(secrets): report in the preferred language`

## 5. `export` e `import` (CL3, CL5)

- [x] Tests de los dos informes en español, con uno y con varios.
- [x] Sus `report` con `Lang`.
- Commit: `feat(portability): report exports and imports in the preferred language`

## 6. El resto de los comandos (CL3)

- [x] Tests de `restore`, `undelete`, `outbox`, `projects`, `sharing` (y `SharingPolicy::render` en `clawg-core`), `metrics`, `memories` en español.
- [x] Sus frases con `Lang`; `stop` en `main.rs`. `clawg path` no tiene perfil y queda en inglés.
- Commit: `feat(cli): speak the preferred language in the other commands`

## 7. El binario real (CL1, CL3, CL4)

- [x] `doctor_speaks_the_preferred_language` y `stop_speaks_the_preferred_language` en `tests/binary.rs`.
- Commit: `test(cli): the commands in the preferred language through the real binary`

## 8. Documentación

- [x] `.claude/CLAUDE.md`. Las specs se reescriben al archivar (ver la propuesta, alcance 9).
- Commit: `docs: describe the commands in the preferred language`
