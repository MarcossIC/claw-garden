# install-smoke — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional. La suite completa corre una vez, sobre el último commit (regla del proyecto).

## 1. `doctor` en Linux (R6)

- [x] Los 7 tests de `doctor` fallan en Linux (`main`, `f7ce973`).
- [x] Fixtures con la raíz de la plataforma; el caso de Windows sigue sólo en Windows.
- Commit: `test(cli): build the doctor fixtures with the platform's own paths`

## 2. Home y embedder de prueba (R1, R2, D1, D2)

- [x] Tests `a_test_home_replaces_the_system_home`, `an_unset_or_empty_test_home_keeps_the_system_home` (`paths.rs`) y los tres de `TestEmbedder` (`bootstrap.rs`): no compilan.
- [x] `CLAWG_TEST_HOME`, `TestEmbedder`, `serve_deps`; `serve` usa `serve_deps`.
- Commit: `feat(core): let a test build run the binary in a temporary home with a stand-in model`

## 3. Sólo dev-dependencies (R3, D3)

- [x] `testing_outside_dev_dependencies_reads_the_section` y `only_dev_dependencies_turn_on_the_test_stand_ins` (`conventions.rs`).
- Commit: `test(cli): keep the testing feature out of release builds`

## 4. Pruebas con el binario (R4, R5, D4)

- [x] `crates/clawg-cli/tests/binary.rs` con los 9 escenarios.
- Commit: `test(cli): run the clawg binary through kill, lock, backup and real stdio`

## 5. CI (R6, D5)

- [x] `.github/workflows/ci.yml`.
- Commit: `ci: run clippy and the suite on Linux and Windows`

## 6. Docs

- [x] `.claude/CLAUDE.md` (Testing) y el mapa de `openspec/README.md`.
- Commit: `docs: describe the binary smoke tests and the CI`

## 7. Verificación

- [x] Suite y clippy en Linux sobre el último commit.
- [ ] El workflow en verde en Linux y Windows.
- [ ] Suite en Windows en la máquina de Marcos.
