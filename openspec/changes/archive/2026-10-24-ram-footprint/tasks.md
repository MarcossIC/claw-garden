# ram-footprint — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. A: sólo un registro que lo pide refresca (RF1)

- [x] Test primero (`crates/clawg-cli/tests/api.rs`): `registering_an_accepted_root_without_asking_leaves_its_index_alone`. Sin el arreglo falla con 2 docs en lugar de 1.
- [x] `register` lanza `spawn_doc_refresh` sólo con `body.index_docs`.

## 2. M1: `--by minute` (RF2)

- [x] Tests primero: `a_report_by_minute_keys_each_local_minute` y `a_report_by_minute_starts_where_the_minutes_are_kept` (`report.rs`), `a_report_by_minute_reads_the_minutes_of_the_file` (`metrics.rs`) y el parseo en `cli.rs`. No compilan sin `By::Minute`.
- [x] `By::Minute`, `since_for`, `ReportBy::Minute`.

## 3. M2: `process.private_peak_bytes` (RF3)

- [x] Tests primero: el escritor guarda la serie (`each_minute_and_the_stop_are_written`) y la muestra del proceso la trae sólo en Windows.
- [x] `ProcessSample::private_peak_bytes` desde `PeakPagefileUsage` y la serie en el escritor.

## 4. M3: la causa y la memoria de cada carga (RF4)

- [x] Tests primero (`idle_embedder.rs`): `the_sweep_tells_a_woken_model_nobody_used_from_one_a_call_used`, `a_model_a_call_loaded_is_never_counted_as_woken` y `a_load_names_who_asked_and_the_memory_around_it`; `memory_reads_in_mib` (`metrics/mod.rs`).
- [x] `Cause`, `load_line`, `Slot::woken_at`, `unload_if_idle` devuelve `Unloaded`; `MemoryProbe` y `mib`; el parámetro en `IdleEmbedder::new` y `unloading_when_idle`; `process::private_bytes` desde `clawg serve`.

## 5. M4: la memoria por paso del import (RF5)

- [x] Tests primero (`timing.rs`): el caso nuevo de `a_step_reads_as_its_numbers` y `each_step_carries_the_memory_as_it_ended_when_there_is_a_probe`.
- [x] `Timing::private_bytes`, `ImportTimings::with_memory` y la sonda en el handler de `POST /v1/import`.

## 6. Docs

- [x] `.claude/CLAUDE.md` (el registro, `metrics report`, la serie nueva, las líneas de carga y del import), la fila 90 de `openspec/README.md` y este cambio.

## 7. Verificación

- [x] Suite y clippy de `.claude/CLAUDE.md` en Linux.
- [x] El CI en verde en Linux y Windows (RF3 y la muestra de Windows sólo corren allá).
- [x] `verify.md`.

El día de uso real con el modelo real (`metrics report --by minute` sobre `process.private_bytes`, `process.private_peak_bytes` y `embedder.resident`, contra las líneas de carga, descarga y refresco de `serve.log`) no se hizo en este cambio. Pasó a [ram-footprint-gates](../../ram-footprint-gates/exploration.md) (92) al partirlo, porque no verifica RF1 a RF5: decide B, C y D ([verify](verify.md#el-día-de-uso-real)).

## 8. Según la medición

B (despertar sólo al empezar una sesión), C (cargar el ONNX con `commit_from_file`), D (embeber el import por tanda, con import-job, 87) y E pasaron a ram-footprint-gates (92), con sus puertas y sus decisiones previas.

## 9. Archive

- [x] RF1 en `specs/project-docs` y `specs/project`, RF2 y RF3 en `specs/metrics`, RF4 en `specs/storage` y RF5 en `specs/portability`; `git mv` a `changes/archive/2026-10-24-ram-footprint`; `state.yaml` archivado; las filas 62, 90 y 91, la nueva 92 y el orden de ejecución del `README.md` de openspec; subir el minor. También se realinearon las citas de las secciones que este cambio movió ([verify](verify.md#al-archivar-2026-10-24)).
