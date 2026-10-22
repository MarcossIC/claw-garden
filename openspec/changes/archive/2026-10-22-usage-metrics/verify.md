# usage-metrics — Verificación

## Linux (2026-10-22)

Sobre `07e7d92`, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.
- Cada commit de código de las [tareas](tasks.md) pasó su parte de la suite antes de commitearse; los que tocan `clawg-cli` (`60f02e1`, `05db38f`, `994493d`, `dd42185`, `9d47962`, `f2c8c96`), la suite entera y clippy.
- `Cargo.lock` no cambió con las features nuevas de `windows-sys`.

## Windows

El código de Windows de `daemon/process.rs` (`GetProcessMemoryInfo` con `PROCESS_MEMORY_COUNTERS_EX`, `GetProcessHandleCount`, `GetProcessTimes`, `GetTimeZoneInformation`) se compiló con `cargo check --target x86_64-pc-windows-msvc` en un crate aparte, contra `windows-sys` 0.61.2 con las mismas features: el workspace entero no se cruza desde Linux por sus dependencias en C. Que corra y que `the_sample_of_this_process_has_its_memory` pase allí lo dice el CI del pull request.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`, un daemon `dev` en `--port 0`:

- tres `GET /health` por el puerto TCP y tres por el socket; pasado el cambio de minuto, con el daemon corriendo, `clawg metrics report --metric http.duration --label route=/health --by hour` contó 6 con `not_stopped: 1` (la ejecución en marcha), y `metrics.db` quedó `0600`;
- los gauges del proceso (`process.rss_bytes`, `process.cpu_ratio`, `rt.worker_busy_ratio`, …) se escribieron en el minuto y `report` los lee como `gauge`;
- `clawg stop` lo paró y la ejecución quedó con `stopped_at_ms` (`not_stopped: 0`).

Con un directorio temporal cuya ruta de socket pasaba de 108 bytes, el daemon con `--port` sirvió sólo por TCP, como pide `specs/daemon-access`; las métricas siguieron igual.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1, R2, R3, R4 | Los tests de `histogram.rs`, `recorder.rs` y `store.rs` no compilaban: los módulos no existían |
| R5 | `every_request_is_timed_by_route_status_and_transport`, `refused_requests_are_timed_too`, `no_label_comes_from_the_request` y `a_server_marked_unix_labels_its_requests_so` no compilaban sin `AppCtx::metrics` ni `router_on` |
| R6 | Los tests de `daemon/process.rs` no compilaban sin el módulo. `the_runtime_sample_counts_its_workers` falló por la aserción en su primera versión, que esperaba contar la tarea del test, que no es una tarea de tokio; ahora cuenta una que el test lanza |
| R3 (escritor) | `metrics_off_is_the_only_way_to_turn_writing_off` y `a_failed_write_turns_writing_off_and_serving_goes_on` no compilaban sin `Writer` |
| R7 | `metrics_needs_the_token_and_answers_the_minute_and_the_run` y `metrics_speaks_prometheus_text` fallaban con 404. Después, el segundo falló por la aserción: `50 * 1e-6` se escribía `0.000049999999999999996`; las fronteras se dividen |
| R8 | `a_report_groups_by_version_day_and_hour`, `a_label_filter_keeps_only_matching_series` y el parseo no compilaban sin `report` ni `MetricsAction` |
| R9 | Se escribieron después del escritor y de `report`, y pasaron al primer intento: guardan la conducta de punta a punta |

## Lo que no se midió

- El sobrecoste con el modelo real (guardado corto y `POST /v1/search` con y sin `CLAWG_METRICS=off`) pide el perfil dev con el modelo, que no se usó sin autorización. El registro de una petición son dos búsquedas binarias sobre 24 fronteras, un lock sin contención y dos cadenas de etiquetas, frente a los milisegundos de un embebido; queda medirlo cuando se use ese perfil.
- El peso del fichero por día de uso real (estimado en el [diseño](design.md), D3, en unos 2 MB por día activo) se ve con `ls -l <data>/metrics.db` tras unos días.
- Ningún ranking cambia: no hay `rag-eval`. `agent_contract.json` no cambia.
