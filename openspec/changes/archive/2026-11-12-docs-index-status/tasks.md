# docs-index-status — Tareas

Strict TDD: los tests de «Cómo se mide» van primero y fallan antes del código. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia.

## 1. Los tests de «Cómo se mide», en rojo

- [x] `health_names_the_project_whose_refresh_runs_and_how_far_it_is`, `health_shows_a_running_refresh_without_the_connection`, `health_says_when_a_refresh_ended_and_what_it_did`, `health_keeps_the_cause_of_a_failed_refresh` y `health_lists_no_docs_index_before_any_refresh` (`crates/clawg-cli/tests/api.rs`).
- [x] `status_shows_the_docs_index_and_the_log_says_when_a_refresh_starts` (`crates/clawg-cli/tests/binary.rs`).

Leen `/health`, el log y la salida de `clawg status` como JSON, así que compilan y fallan por su aserción.

## 2. El estado en memoria (IS1, IS2, IS6)

- [x] Tests en `crates/clawg-core/src/memory/project_docs/mod.rs` (escritos junto con el código, no antes: los que se vieron fallar primero son los de la tarea 1): `a_refresh_reports_its_plan_and_each_file_it_finishes`, `the_status_follows_a_refresh_from_its_plan_to_its_end`, `a_failed_refresh_leaves_its_cause_until_one_succeeds`, `a_claimed_refresh_shows_as_running_until_it_is_released` y `an_index_status_serializes_what_health_shows`.
- [x] `RefreshStep` y el observador de `refresh`; `current` y `last` en `Progress`; `IndexStatus` y `DocIndex::statuses`; el disparador en `begin` y `begin_if_due`. `clawg-eval` pasa un observador vacío.

## 3. `/health` y la línea de log (IS3, IS4)

- [x] Test primero: `the_docs_start_line_names_the_project_the_trigger_and_the_plan` (`crates/clawg-cli/src/daemon/api.rs`).
- [x] `docs_index` en `HealthBody`, `refresh_start_line` y la línea en `refresh_docs`. Los tests de `/health` de la tarea 1 pasan.

## 4. `clawg status` (IS5)

- [x] Tests primero: `a_health_without_a_docs_index_still_parses` (`client.rs`) y `the_status_report_shows_the_docs_index` (`status.rs`).
- [x] `docs_index` en el `Health` del cliente. El test del binario de la tarea 1 pasa.

## 5. Documentación

- [x] `docs/reference/daemon.md` (`/health` y el log del refresco) y `docs/reference/commands.md` (`clawg status`).

## 6. Verificación

- [x] `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -D warnings` en Linux, una vez, sobre el último commit.
- [x] El contrato fijado del agente pasa sin regenerarse (IS7).
- [x] El CI en verde en Linux y Windows.
- [ ] Pendiente en local: lo que dice [verify.md](verify.md).

## 7. Al archivar

- [ ] Consolidar IS1 a IS7 en `specs/project-docs` y `specs/daemon-access`, como dice la [spec](spec.md#specs-que-se-consolidan-al-archivar), mover la fila del mapa y subir la versión.
