# core-use-cases — Tareas

## Cómo se trabaja cada tarea

- **TDD estricto.** Cada regla que baja al núcleo empieza con un test unitario en rojo en `clawg-core`, sin router ni HTTP ([D9](design.md#d9-los-tests-del-núcleo-sin-http)). El rojo es el del compilador cuando el tipo o el método todavía no existe.
- **Caracterización.** `each_refused_save_keeps_its_status_and_body` se escribe antes de tocar el handler y pasa sobre el código de ese momento ([D8](design.md#d8-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia)).
- **Commits.** Cada tarea es un commit convencional que compila y pasa los tests de los archivos que cambia, nombrados al final de la tarea. La suite entera y los dos clippy corren una vez, sobre el último commit (tarea 5).
- **Windows.** Cada `cargo` lleva `--target-dir target-test`.

Este PR aplica las tareas 1 a 5, el guardado completo. Lo demás está en [«Partes siguientes»](#partes-siguientes).

## 1. Artefactos

- [x] La exploración revisada contra `develop`, `proposal.md`, `spec.md`, `design.md`, `tasks.md` y `state.yaml`.
- Commit: `docs(openspec): propose core-use-cases`

## 2. Caracterización de los rechazos de un save (R7)

- [ ] `each_refused_save_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`): el estado y el cuerpo entero de cada rechazo, con los literales que da `develop`. Pasa antes de seguir.
- Commit: `test(daemon): pin the status and body of every refused save`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --lib daemon::api
  ```

## 3. El formato de un save, en el núcleo (R1, R2, R7)

- [ ] **Tests en rojo** en `crates/clawg-core/src/memory/ingest/save.rs`: `content_is_checked_in_bytes`, `operation_ids_are_checked`, `sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `the_format_is_checked_in_order`, `each_format_refusal_keeps_its_text`, `a_conclusion_cannot_be_stated_by_the_user` y `a_parsed_save_keeps_its_type`. No compilan: el módulo no existe.
- [ ] `memory::save` con `SaveInput`, `NewSave::parse`, `SaveRejected`, `OperationIdError`, `check_operation_id` y los tres topes ([D1](design.md#d1-dónde-vive-cada-parte), [D3](design.md#d3-un-error-con-el-texto-de-hoy-y-sus-datos), [D4](design.md#d4-el-formato-de-un-save-es-del-dominio)).
- [ ] `save_memory` parsea con `NewSave::parse` y traduce con `save_refused`. `requested_operation` usa `check_operation_id`. `MAX_CONTENT_BYTES` se reexporta en `daemon::api`.
- [ ] Se van `check_content`, `check_operation_id`, `distinct_sources` y los tres topes de `api.rs`, con los tests puros que pasan a core (`content_is_checked_in_bytes`, `sources_are_counted_once_and_sorted`, `too_many_sources_are_refused`, `operation_ids_are_checked`).
- Commit: `refactor(core): check the format of a save in the core`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --lib memory::ingest::save
  ```

  ```bash
  cargo test -p clawg-cli --lib daemon::api
  ```

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

## 4. Las reglas que leen el store y el embedder, en el núcleo (R2 a R7)

- [ ] **Tests en rojo** en `crates/clawg-core/src/runtime.rs`: `a_conclusion_from_missing_or_hidden_sources_is_refused`, `a_conclusion_from_a_replaced_source_is_refused`, `a_store_that_fails_refuses_the_save_with_its_error`, `a_save_is_refused_while_the_embedder_failed`, `a_save_waits_only_for_a_ready_embedder`, `a_save_of_a_merged_project_goes_to_its_target` y `an_admitted_save_becomes_the_event_the_daemon_queued`. No compilan: `admit` no existe.
- [ ] `MemoryRuntime::admit` y `Admitted`, con `agent_id`, `project_id`, `memory_type`, `topic_key`, `waits` e `into_event` ([D2](design.md#d2-dos-pasos-y-la-sesión-entre-los-dos), [D5](design.md#d5-el-tipo-se-parsea-y-el-evento-lleva-su-nombre)).
- [ ] `save_memory` admite con `admit`, elige la sesión con lo que da `Admitted`, arma el evento con `into_event` y cuenta el save en su sesión con `Admitted::memory_type`.
- [ ] Se van `missing_sources` y `replaced_list` de `api.rs`, con sus tests, que pasan a `save.rs` con su nombre (`missing_sources_are_the_ones_that_were_not_found`, `replaced_sources_name_what_replaced_them`).
- Commit: `refactor(core): admit a save in the core, not in the daemon`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --lib runtime
  ```

  ```bash
  cargo test -p clawg-core --lib memory::ingest::save
  ```

  ```bash
  cargo test -p clawg-cli --lib daemon::api
  ```

  ```bash
  cargo test -p clawg-cli --test api
  ```

## 5. Verificación

- [ ] Sobre el último commit de código, en Linux:

  ```bash
  cargo test --workspace
  ```

  ```bash
  cargo clippy --workspace --all-targets -- -D warnings
  ```

  ```bash
  cargo clippy -p clawg-cli -- -D warnings
  ```

- [ ] Las comprobaciones de [D8](design.md#d8-cómo-se-comprueba-que-nada-de-lo-que-ve-un-cliente-cambia).
- [ ] `verify.md`, con la sección «Pendiente».
- Commit: `docs(openspec): record core-use-cases applied`

El archivo (specs principales, `openspec/README.md`, `openspec/ROADMAP.md`) y la subida de versión no son parte de estas tareas: se hacen en local después del merge.

## Partes siguientes

Cada una es un PR, con sus requisitos agregados a `spec.md` y sus decisiones a `design.md` antes de aplicar, y el mismo método: caracterización, tests en rojo en core, el handler que sólo traduce.

- **Parte 2. Proyectos.**
  - `locate_project`: la decisión `folder_shared` (una carpeta cuyo proyecto tiene otras no se fusiona) en un caso de uso del núcleo, junto a `own_folder_count` ([D7](design.md#d7-qué-handlers-entran)).
  - `register_root`: el `root_path` vacío y el `index_docs` que no es un directorio. La traducción de `RegisterError` queda en el handler.
  - Las demás rutas de proyectos que escriben (renombrar, fusionar, borrar, mover, subcarpetas): revisar qué deciden.
- **Parte 3. Las otras escrituras de memorias.** `PATCH /v1/memories/...` (que también parsea `memory_type`), restaurar, borrar y las disputas (`resolve`).
- **Parte 4. Partir `api.rs`** en `crates/clawg-cli/src/daemon/routes/`, un archivo por dominio, como paso mecánico sin cambio de conducta.
- **Fuera:** `context`, salvo que storage-read-side le dé un caso de uso ([D7](design.md#d7-qué-handlers-entran)), y `eval_api` ([D6](design.md#d6-eval_api-no-cambia)).
