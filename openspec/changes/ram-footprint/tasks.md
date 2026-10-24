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
- [ ] **[perfil con uso real, modelo real, con autorización]** Un día de uso normal con Claude Code: `metrics report --by minute` sobre `process.private_bytes`, `process.private_peak_bytes` y `embedder.resident`, contra las líneas de carga, descarga y refresco de `serve.log`. Las mismas lecturas que la exploración tomó en prod, ahora con A. Responde las cinco preguntas del problema.
- [ ] `verify.md`.

## 8. Según la medición

- [ ] B (despertar sólo al empezar una sesión), si M3 muestra que más de la mitad de las cargas de un día son de un `wake` que nadie usó. Antes, elegir entre B1, B2 y B3.
- [ ] C (cargar el ONNX con `commit_from_file`), si M2 muestra que la carga es el máximo del proceso; antes, aceptar la ventana entre la verificación y la carga.
- [ ] D (embeber el import por tanda), con import-job (87), si M4 muestra que los vectores pesan; antes, aceptar que un import cortado quede parcial y deshacible.
- [ ] E, al final, si todavía hace falta.

## 9. Archive

- [ ] Consolidar RF1 a RF5 (y lo que entre de B, C, D) en `specs/project-docs`, `specs/project`, `specs/metrics`, `specs/storage` y `specs/portability`; `git mv` a `changes/archive/`; subir el minor.
