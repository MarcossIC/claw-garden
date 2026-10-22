# usage-metrics — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El histograma (R1, D1)

- [ ] Tests primero: `merging_two_histograms_is_the_histogram_of_the_union`, `a_percentile_falls_in_the_bucket_of_the_exact_one`, `histograms_of_different_units_do_not_merge`, `the_buckets_round_trip_through_bytes`.
- [ ] `clawg_core::metrics::histogram`.
- Commit: `feat(core): add a fixed-bucket histogram that merges and estimates percentiles`

## 2. El registro (R2, D2)

- [ ] Tests primero: `taking_the_minute_empties_it_and_keeps_the_run`, `labels_are_canonical_whatever_their_order`, `a_gauge_keeps_its_extremes_and_last_value`.
- [ ] `clawg_core::metrics::recorder`.
- Commit: `feat(core): record metric series by minute and for the whole run`

## 3. `metrics.db` (R3, R4, D3)

- [ ] Tests primero: `a_run_is_recorded_open_and_closed_on_stop`, `a_minute_is_written_with_its_series`, `a_corrupt_file_is_set_aside_and_recreated`, `the_file_is_private_to_its_owner`, `old_minutes_fold_into_their_hour`, `hours_past_the_retention_are_dropped`.
- [ ] `clawg_core::metrics::store`.
- Commit: `feat(core): keep metric minutes and hours in their own metrics.db`

## 4. La latencia HTTP (R5, D2)

- [ ] Tests primero: `every_request_is_timed_by_route_status_and_transport`, `refused_requests_are_timed_too`, `no_label_comes_from_the_request` (`tests/api.rs`).
- [ ] `AppCtx::metrics`, el middleware y el `Transport` de cada servidor.
- Commit: `feat(daemon): time every request by route, status class and transport`

## 5. Proceso y runtime (R6, D4)

- [ ] Tests primero: `the_linux_status_file_is_read_in_bytes`, `a_ratio_is_the_delta_over_the_wall_time`, `the_sample_of_this_process_has_its_memory`.
- [ ] `daemon/process.rs`; las features de `windows-sys`.
- Commit: `feat(daemon): sample the memory, CPU and handles of the process`

## 6. El escritor (R3, R4, D3)

- [ ] Tests primero: `metrics_off_is_the_only_way_to_turn_writing_off`, `a_failed_write_turns_writing_off_and_serving_goes_on`.
- [ ] `daemon/metrics.rs`: la ejecución, el volcado por minuto, la retención por hora, el cierre al apagarse; `serve` lo arranca.
- Commit: `feat(daemon): write the metrics of each minute to metrics.db`

## 7. `GET /v1/metrics` (R7, D5)

- [ ] Tests primero: `metrics_needs_the_token_and_answers_the_minute_and_the_run`, `metrics_speaks_prometheus_text`.
- [ ] La ruta y el texto de Prometheus.
- Commit: `feat(daemon): answer the current metrics as JSON or Prometheus text`

## 8. `clawg metrics report` (R8, D5)

- [ ] Tests primero: `a_report_groups_by_version_day_and_hour`, `a_label_filter_keeps_only_matching_series`, el parseo en `cli.rs`.
- [ ] `metrics::report`, `MetricsDb::rows_since`, el subcomando.
- Commit: `feat(cli): report a metric by day, hour of day or version`

## 9. De punta a punta (R9)

- [ ] `a_daemon_records_its_requests_and_report_reads_them_after_stop` y `a_killed_daemon_leaves_its_run_not_stopped` en `tests/binary.rs`.
- Commit: `test(cli): check the metrics of a real daemon through report`

## 10. Docs

- [ ] `.claude/CLAUDE.md`: `metrics.db`, `CLAWG_METRICS`, `GET /v1/metrics`, `clawg metrics report`.
- Commit: `docs: describe the usage metrics`

## 11. Verificación

- [ ] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 12. Archive

- [ ] `specs/metrics`; `git mv` a `changes/archive/<fecha>-usage-metrics`; `state.yaml` archivado; fila 71 y el orden de ejecución del `README.md` de openspec; subir el minor.
