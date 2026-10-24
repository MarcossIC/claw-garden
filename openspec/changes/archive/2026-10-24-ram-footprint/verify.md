# ram-footprint — Verificación

## Linux (2026-10-24)

La suite (`cargo test --workspace`) y los dos clippy de `.claude/CLAUDE.md` pasan. ONNX Runtime 1.24.2 se enlazó en dinámico porque la descarga de `ort-sys` (`cdn.pyke.io`) estaba bloqueada. Ningún test carga el modelo real.

- **RF1, el refresco:** `registering_an_accepted_root_without_asking_leaves_its_index_alone`, que sin el arreglo falla con 2 docs en lugar de 1. `registering_a_root_indexes_its_docs_in_the_background` y `a_root_registered_without_its_docs_is_not_indexed` siguen pasando sin cambios.
- **RF2, por minuto:** `a_report_by_minute_keys_each_local_minute` y `a_report_by_minute_starts_where_the_minutes_are_kept` (`report.rs`), `a_report_by_minute_reads_the_minutes_of_the_file` (`crates/clawg-cli/src/metrics.rs`) y el parseo de `--by minute` en `metrics_report_takes_a_metric_labels_a_grouping_and_an_age` (`cli.rs`). Antes de `By::Minute` no compilaban.
- **RF3, el pico privado:** `each_minute_and_the_stop_are_written` guarda la serie. En Linux, `the_sample_of_this_process_has_its_memory` comprueba que la muestra no la trae.
- **RF4, cada carga:** `the_sweep_tells_a_woken_model_nobody_used_from_one_a_call_used`, `a_model_a_call_loaded_is_never_counted_as_woken`, `a_load_names_who_asked_and_the_memory_around_it` y `memory_reads_in_mib`. Las pruebas del ciclo de vida del embedder que ya existían pasan sin cambiar ninguna aserción.
- **RF5, cada paso del import:** el caso nuevo de `a_step_reads_as_its_numbers` y `each_step_carries_the_memory_as_it_ended_when_there_is_a_probe`.

## CI

El CI del pull request #92 (cabeza `7cffc13`) pasó en `ubuntu-latest` y en `windows-latest`. Sólo en Windows se compilan y corren la lectura de `PeakPagefileUsage` y la aserción de que el pico es mayor o igual que `private_bytes`.

## El día de uso real

No se hizo en este cambio. La tarea pedía un día de uso normal con Claude Code en un perfil con uso real y el modelo real, y eso necesita un release con el paso 0: prod corría 0.62.0, que no lo trae. Ningún requisito de la spec depende de esa medición, porque RF1 a RF5 se verifican con sus tests. Lo que la medición decide es si entran B, C, D y E. Por eso, al archivar, el cambio se partió: el día de uso pasó con esos brazos a [ram-footprint-gates](../../ram-footprint-gates/exploration.md) (92). Esperarlo acá habría dejado a `specs/project-docs` exigiendo, en el release que publica A, el refresco que A saca.

## Al archivar (2026-10-24)

- RF1 pasa a [`specs/project-docs`](../../../specs/project-docs/spec.md). La sección «Registrar una raíz aceptada la indexa en segundo plano» se llama ahora «Un registro que lo pide indexa la raíz en segundo plano» y suma el escenario del hook. También cambió el límite conocido que dice cuándo se pone al día el índice. En [`specs/project`](../../../specs/project/spec.md) cambió la nota del registro (R4).
- RF2 y RF3 pasan a [`specs/metrics`](../../../specs/metrics/spec.md), en R8 y R6, con sus escenarios. RF4 pasa a [`specs/storage`](../../../specs/storage/spec.md) («Cada carga dice quién la pidió y cuánto costó», dentro de «Ciclo de vida del embedder»). RF5 pasa a [`specs/portability`](../../../specs/portability/spec.md), en IT1.
- Se realinearon a sus símbolos las citas que este cambio corrió:
  - en `project-docs`, las de `register`, `spawn_doc_refresh` y `refresh_docs`;
  - en `metrics`, las de `metrics.rs`, `report` y `utc_offset_s`;
  - en `storage`, las de `idle_embedder.rs`, `bootstrap.rs`, `serve.rs` y `api.rs` en «Ciclo de vida del embedder». Las de `embedder.rs`, que este cambio no tocó, quedaron como estaban;
  - en `portability`, las de `timing.rs` en «Dónde está».
- La versión sube de 0.62.1 a 0.63.0 antes de que se publique 0.62.1, así que la entrega 0 de project-registry sale en 0.63.0. Así lo dicen ahora su línea en el «Origen» de `specs/project` y sus filas del `README.md` de openspec.
- Las citas `ruta:línea` de la exploración, la propuesta y la spec quedan como estaban: describen el código contra el que se escribieron.
