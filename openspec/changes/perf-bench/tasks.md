# perf-bench — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada grupo es un commit convencional atómico, que se comprueba con las pruebas de los archivos que cambia. La suite completa y los dos clippy de `.claude/CLAUDE.md` se corren una vez, sobre el último commit del cambio. Ningún test de la suite necesita el modelo real, la red ni un perfil de datos, y nada queda en el temporal. Los commits se arman con rutas explícitas, sin referencia a planes, fases ni cambios, y sin atribución.

Requisitos en la [spec](spec.md) (PB1 a PB8) y decisiones en el [diseño](design.md) (D1 a D9).

## Orden elegido

Un test de integración es un crate propio: una función que sólo usa el banco ignorado y que todavía no existe es código muerto, y `dead_code` es un error. Por eso el banco entra en un solo commit, con sus partes puras y sus tests, y el `diff` en otro: los dos archivos no se comparten código (el contrato entre ellos es el JSON).

## 1. El banco (PB1 a PB6; D1 a D7)

- [x] RED en `crates/clawg-cli/tests/perf_bench.rs`: los tests de PB1 a PB5 (`the_config_reads_its_defaults_and_refuses_what_it_cannot_use`, `the_seeded_store_is_the_same_for_a_seed_and_another_for_another`, `the_seeded_store_is_an_export_the_import_reads_whole`, `rows_share_frequent_words_and_few_share_an_identifier`, `queries_and_saves_repeat_for_a_seed`, `a_save_has_the_words_of_its_pieces_and_fits_the_content_limit`, `the_docs_corpus_repeats_for_a_seed`, `requests_leave_at_a_fixed_rate`, `streams_send_in_the_order_their_requests_fall_due`, `a_quantile_is_the_nearest_rank`, `the_date_of_a_file_is_utc`, `a_result_file_is_never_replaced`, `the_cpu_of_a_process_is_read_after_its_command`). Fallan por no compilar.
- [x] GREEN: `Rng` (SplitMix64), `seed_export`, `queries`, `save_text`, `words_for_pieces`, `docs_corpus`, `schedule`, `earliest`, `quantile`, `summary`, `Config::from_vars`, `file_name`, `utc_date`, `write_new`, `cpu_ticks_of_stat`, `cpu_ratio`, `usage_of` por sistema.
- [x] El banco ignorado `bench`: arranque, siembra, reposo, calentamiento, escenarios solos y de fondo, lectura del proceso y de `/v1/metrics`, archivo. Una corrida debug de 2 000 filas y otra de 10 000 para elegir las tasas (D4).
- Commit: `test(bench): measure the daemon under a seeded store and a fixed-rate load`

## 2. El `diff` (PB7; D8)

- [x] RED en `crates/clawg-eval/src/bin/perf-bench/`: los tests de `stats.rs`, `diff.rs` y `main.rs` que nombra PB7. Fallan por no compilar.
- [x] GREEN: `quantile`, `quantile_change`, `mann_whitney_p`, `erfc`; `BenchFile`, `parse`, `read`, `verdict`, `diff`, `render`; `parse_args` y `main` con las salidas 0, 1, 2 y 6; el `[[bin]]` en `crates/clawg-eval/Cargo.toml`.
- Commit: `feat(eval): compare two runs of the daemon bench`

## 3. La referencia (PB1, PB7)

- [x] `docs/reference/testing.md`: el banco (cómo se corre, sus variables, su archivo) junto a `transport_bar`, y `perf-bench diff` entre las herramientas de evaluación.
- Commit: `docs: describe the daemon bench and its diff`

## 4. Verificación (PB8)

- [x] Suite completa y los dos clippy sobre el último commit.
- [x] Dos corridas del banco (debug, embedder falso, 10 000 filas) sobre el mismo commit y su `diff`; una tercera con 50 ms de espera inyectados en `POST /v1/search` (sin commitear) y su `diff` contra la primera. Los números, en `verify.md`.
- [x] El falso `slower` de las dos corridas del mismo commit sube el mínimo de muestras a 40. RED: `the_verdict_needs_the_threshold_and_an_interval_without_zero` con 39 y 40 muestras. Commit: `fix(eval): decide a bench p95 only with 40 samples a side`
- Commit: `docs(openspec): record the verification of perf-bench`

## 5. Archivo

- [ ] La sección «Banco de rendimiento del daemon» en `specs/eval`; el cambio a `changes/archive/`; el mapa de `openspec/README.md`.
- Commit: `docs(openspec): archive perf-bench and fold it into the specs`
- [ ] Subir el minor, `cargo update --workspace`. Commit: `chore: bump version to 0.70.0`
