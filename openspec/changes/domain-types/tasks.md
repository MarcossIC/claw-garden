# domain-types — Tareas

## Cómo se trabaja cada tarea

- **TDD estricto.** Cada tarea de código empieza con su test en rojo; el rojo es el del compilador cuando el tipo todavía no existe.
- **Commits.** Cada tarea es un commit convencional que compila y pasa los tests de los archivos que cambia, nombrados al final de la tarea. La suite entera y los dos clippy corren una vez, sobre el último commit de código (tarea 4).
- **Windows.** Cada `cargo` lleva `--target-dir target-test`.

La tanda 1 de la exploración son las tareas 1 a 5. Las tandas 2 a 5 están en [«Tandas siguientes»](#tandas-siguientes).

## 1. Artefactos

- [x] `proposal.md`, `spec.md`, `design.md`, `tasks.md` y `state.yaml`.
- Commit: `docs(openspec): propose domain-types`

## 2. El tipo y el estado, tipados de la base al cliente (R1 a R5)

Es un solo commit: cambiar el tipo de un campo de `RankedHit` o de `MemoryAdminRow` no compila hasta que cada lector, en los tres crates, lo lee con el tipo nuevo.

- [ ] **Tests en rojo** en `crates/clawg-core/src/memory/domain/record.rs`: `a_stored_name_the_code_knows_reads_as_its_variant`, `a_stored_name_the_code_does_not_know_keeps_its_text`, `every_status_has_its_stored_name`, `only_an_active_or_disputed_row_is_live`, `a_stored_value_compares_by_its_text`, `a_stored_value_serializes_as_its_bare_text`, `a_stored_value_round_trips_through_sqlite_unchanged` y `a_stored_value_that_is_not_text_is_a_type_error`. No compilaban: los tipos no existían.
- [ ] `MemoryStatus`, `StoredName`, `Stored<T>`, `StoredType` y `StoredStatus`, con `ToSql`, `FromSql`, `Serialize`, `Deserialize`, `Display`, `Debug`, `PartialEq<T>` y `Default` para el estado (el `DEFAULT 'active'` de la columna); `Display` para `MemoryType`; reexportados en `clawg_core::memory` ([D1](design.md#d1-dos-direcciones-lo-que-entra-es-estricto-lo-leído-es-storedt)).
- [ ] **Tests en rojo de conservación:** `a_row_of_a_type_the_code_does_not_know_is_still_found` (`search.rs`), `rows_of_a_type_or_status_the_code_does_not_know_stay_listed` (`listing.rs`), `an_event_of_a_type_the_code_does_not_know_parses_and_keeps_it` (`events.rs`), `a_status_the_code_does_not_know_never_ranks` (`score.rs`), `the_tier_udf_keeps_the_tier_of_a_type_it_does_not_know` (`tier.rs`) y el caso `archived` de `source_state_cases` (`synthesis.rs`).
- [ ] El núcleo: `RawHit`, `RankedHit`, `MemoryAdminRow`, `ListParams`, `SaveText`, `UpdatedRow`, `RestoredRow`, `ResolvedRow`, `PairRow`, `TopicRow`, `UnitRow`, `MoveFilter`, `MemoryLine`, las filas de `project_delete.rs`, `undo.rs` e `import.rs`, `MemoryRow` y `MemoryEvent` leen `StoredType` y `StoredStatus`. `lambda_for`, la puerta de estado de `score`, `compute_tier`, `retier`, `preference_mark`, `SourceState::of`, `PairRow::replaced_by`, `revive_after_delete` y las vidas de import y translate deciden con el enum ([D2](design.md#d2-una-decisión-con-un-nombre-desconocido-toma-el-camino-de-hoy), [D5](design.md#d5-el-evento-del-outbox-lleva-optionstoredtype)). `MemoryType::is_protected_name` se va: cada llamada usa `known()` e `is_protected`. `UpdateError::NeedsUser` y `MoveNeedsUser` llevan un `MemoryType`.
- [ ] El daemon: el filtro de estado de un listado se parsea con `MemoryStatus::from_name` y `STATUSES` se va; los filtros de tipo pasan a `StoredType::read`; `RestoredBody::status` es un `StoredStatus` ([D4](design.md#d4-los-filtros-son-lecturas-el-estado-de-la-api-se-valida)).
- [ ] El cliente y el puente: `Hit`, `MemoryRow`, `Updated`, `Restored` y la lectura de un save llevan los mismos tipos; `mcp/tools.rs`, `undelete.rs` y `memories.rs` deciden con el enum; `StatusArg` se convierte a `MemoryStatus` ([D6](design.md#d6-lo-que-no-se-tipa)).
- [ ] `eval_api` y `clawg-eval` convierten en su borde.
- [ ] Los tests que construían o comparaban texto, con `Stored::read` y `.as_str()` ([D7](design.md#d7-los-tests)).
- Commit: `refactor: read memory types and statuses as typed names`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --features testing
  ```

  ```bash
  cargo test -p clawg-cli --lib
  ```

  ```bash
  cargo test -p clawg-cli --test api
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

## 3. Los nombres del SQL (R6)

- [ ] **Test en rojo:** `compared_names_reads_the_literals_a_line_compares` (`crates/clawg-cli/tests/conventions.rs`). No compilaba: la función no existía.
- [ ] `compared_names` y `every_status_and_type_the_sql_compares_is_one_the_code_knows` ([D3](design.md#d3-los-literales-siguen-en-el-sql-un-test-los-comprueba)).
- Commit: `test(conventions): check the statuses and types the SQL compares`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

## 4. Verificación

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

- [ ] Los dos contratos del agente pasan sin `CLAWG_BLESS`, y `crates/clawg-cli/tests/api.rs` no cambia.
- [ ] Windows: la suite y los dos clippy con `--target-dir target-test`.
- [ ] `verify.md`.

## 5. Archivo

Con la verificación hecha, en local: R1 a R6 a `specs/storage` y `specs/retrieval`, el cambio a `changes/archive/`, su fila de `openspec/README.md` a `changes/archive/README.md`, `openspec/ROADMAP.md`, y el minor de la versión (`chore: bump version to X.Y.0`).

## Tandas siguientes

Cada una es un cambio propio, con su propuesta, en el orden de la exploración:

2. Ids por tabla: `MemoryId` y, si sobreviven a retire-legacy-kinds, los de las otras tablas.
3. `ProjectId` y `AgentId`.
4. Structs de parámetros para las funciones más anchas (`KnnQuery`, los `build_*_prompt`).
5. El predicado de alcance como tipo propio en lugar del JSON de `reach_bind`.
