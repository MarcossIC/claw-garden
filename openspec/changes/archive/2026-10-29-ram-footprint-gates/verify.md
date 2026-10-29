# ram-footprint-gates — Verificación

Contra RM1 a RM5 y la puerta de la [spec](spec.md), el [diseño](design.md) y las [tareas](tasks.md), sobre `6984ba4` (commits `b04ac88` a `6984ba4`, `git log --oneline 8a7c18c..HEAD`).

## Windows

- `cargo test --workspace --target-dir target-test -j 2` pasa, sin fallas. Los `#[ignore]` son los de antes del cambio y `a_partly_reused_doc_matches_a_full_reindex_within_tolerance`.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings` salen con 0. `cargo test --locked -p clawg-core --lib build_info --target-dir target-test`, lo que el CI corre del lado de la release, también pasa.
- Cada test nuevo se escribió antes que su código y falló por la aserción o porque no compilaba. Como varios eran rojos sólo por no compilar, se probaron además por mutación: 13 mutantes en los dos primeros commits (dos sobrevivientes, cerrados por `342859b` y `0042cdc`), tres en el de la descarga (un sobreviviente, cerrado por `2fab1e0` y acelerado por `912f91c`: 0,03 s en lugar de 11 s), la automutación de `verdict` y `schedule` en `load-eval` (un sobreviviente, cerrado con `the_median_is_the_lower_middle_load`) y cinco en la reutilización de docs (dos sobrevivientes, cerrados con casos nuevos). Esta verificación no vuelve a compilar los rojos de cada commit.
- `the_embedding_contract_is_pinned` pasa de entrada, porque fija lo que existe. Se comprobó a mano que falla con su mensaje cambiando un literal (tasks.md, tarea 6).
- `a_partly_reused_doc_matches_a_full_reindex_within_tolerance` (`#[ignore]`) corrió una vez en Windows con el modelo real bajado a un directorio temporal, no a un perfil. Pasó en 126,6 s, y el directorio se borró.

## CI

Falta el pull request. Lo que sólo corre en Linux queda para su CI: la rama Unix de RM1 (sin `share_mode`, la guarda es el `0700`), el `None` de `private_peak_bytes` de RM3 y el lado `cfg(not(windows))` de `is_sharing_conflict`. A la inversa, RM1 en Windows, RM3 y la parte de Windows de RM2 sólo se ejercitan acá.

## Por requisito

| Requisito | Veredicto | Tests (todos pasan) |
|---|---|---|
| RM1, la vista | Cumple en Windows | `the_cache_check_hands_the_onnx_over_as_a_read_only_view`, `a_view_of_a_changed_onnx_is_refused`, `a_session_is_built_from_the_verified_view` (`embedder.rs`), `a_load_holds_the_model_file_against_writes_until_the_session_returns` y `the_model_file_is_free_again_after_a_failed_load` (los dos `#[cfg(windows)]`, en rutas con «ñ»), `a_downloaded_onnx_is_not_kept_in_memory_and_is_loaded_from_the_cache`, `each_file_is_hashed_once_per_run` (cambiado), `a_body_that_outgrows_the_cap_while_streaming_is_refused`; se conservan `the_cache_check_leaves_the_runtime_thread_free`, `a_file_larger_than_any_model_file_is_refused_without_holding_it` y `the_worker_resolves_the_tokenizer_off_the_runtime_thread` |
| RM2, carga y reparación | Cumple en Windows | `a_repair_waits_for_a_load_to_release_the_model_file`, `a_sharing_violation_on_open_is_retried_within_the_wait` (`#[cfg(windows)]`), `a_persistent_sharing_violation_answers_not_ready_without_a_repair` y `only_a_busy_file_spares_the_repair` (`bootstrap.rs`), `a_persistent_sharing_violation_is_not_a_missing_file_at_start` y `a_held_file_wins_over_a_missing_one` (`#[cfg(windows)]`: fuera de Windows no hay choque que provocar), `the_wait_for_an_open_is_bounded_and_only_windows_sharing_errors_count` |
| RM3, el pico | Cumple, con W1 y W2 | `a_load_line_names_the_peak_when_the_load_raised_it`, `a_load_line_says_when_the_load_did_not_pass_the_peak`, `a_load_line_without_a_peak_probe_is_unchanged` (`idle_embedder.rs`), `the_peak_is_not_written_to_the_metrics_file` (`daemon/metrics.rs`) |
| RM4, el contrato | Cumple | `the_embedding_contract_is_pinned` |
| RM5, la reutilización | Cumple | `an_edited_doc_embeds_only_the_passages_that_changed`, `a_partly_reused_doc_equals_a_full_reindex`, `a_reused_vector_is_the_stored_blob`, `a_repeated_text_is_reused_in_all_its_passages`, `a_repeated_text_takes_the_blob_of_its_first_passage`, `a_doc_of_another_model_reuses_nothing`, `a_moved_file_reuses_nothing`, `a_failed_embedding_of_the_rest_keeps_the_doc_as_it_was`, `reused_counts_passages_not_distinct_texts`, `the_docs_log_line_shows_the_reused_passages` (`api.rs`); se conservan `an_unchanged_project_embeds_nothing`, `another_model_reindexes_every_doc` y `a_second_refresh_does_not_wait_for_the_first`; con el modelo real, la corrida de arriba |
| La puerta | `View`: queda C′ | `plan.rs`, `gate.rs` y `main.rs` de `load-eval` fijan `schedule`, las tres reglas, el directorio y el archivo de resultado antes de medir |

Lo que se leyó del código, además de los tests:

- **RM1.** `verified_onnx` (`model_download.rs:814`) abre con `FILE_SHARE_READ` en Windows, trata el largo 0 como `Altered`, mapea con `MmapOptions::new().no_probe_handle().map` (nunca `map_copy`) y hashea los bytes mapeados. En `memmap2` 0.9.12, `no_probe_handle` crea la sección y la vista con `PAGE_READONLY` y `FILE_MAP_READ` (`src/windows.rs:232-263`, `src/lib.rs:495-521` de su fuente en la caché de cargo). `InMemory` sólo existe bajo `cfg(any(test, feature = "testing"))`. En producción, `model_loader` consume los `files` y los suelta al volver (`bootstrap.rs:249-254`, llamado en `spawn_blocking` en `:479` y `:648`), así que el archivo se suelta cuando termina la carga, con o sin sesión.
- **RM2.** La recarga contesta `Err` ante `Busy` sin reparar (`bootstrap.rs:472`, `needs_repair` en `:500`). `IdleEmbedder` lo convierte en `NotReady` con su `warn`, y en el arranque es `BootstrapError::Download`, reintentable. El código no compara ningún `ErrorKind`, sólo códigos crudos, y los tests tampoco.
- **RM5.** `stored_vectors` lee pares de texto y blob, nunca ids, y compara el `model_id` en la misma sentencia. `reuse_plan` y `fill` son puras. `replace_doc` no cambió.

## La puerta (tarea 5b)

`load-eval gate` corrió el 2026-10-29 en Windows, con un build release de clawg 0.66.0 y el modelo real bajado a un `load-eval-<uuid>` del temporal, que se borró al terminar. Hubo once rondas: la 0, de calentamiento, se descartó, y quedaron diez cargas por brazo, cada una en su proceso. Las cifras son las de `exploration.md` («La puerta de C′: lo medido»). El JSON está en `docs/eval/results/2026-10-29-real-load-eval.json`, fuera de git (lo ignora `.gitignore:32`), y coincide con ellas.

| Brazo | Mediana del pico de commit | Baja contra `Vec` | Mediana del tiempo total | Reglas |
|---|---|---|---|---|
| `Vec` | 1.285,7 MiB | — | 555 ms | — |
| `File` | 1.013,4 MiB | 272,3 MiB | 687 ms | no llega a los 300 MiB |
| `View` | 837,1 MiB | 448,6 MiB | 623 ms | pasa las tres |

Decisión: `View`. Las tareas de `File` y de `Revert` no aplican. La predicción del código de ONNX Runtime (−272 y −448 MiB) se cumplió. Las reglas leen «la mejor de hoy» como el menor pico del brazo `vec` de la misma corrida, con `<` estricto, y toman como mediana de una cantidad par el valor medio inferior. Esa lectura está confirmada. La corrida de Linux (`VmHWM`), que según la spec confirma el mecanismo sin decidir, no se hizo.

## Lo que se comprobó además

- **`.claude/CLAUDE.md`.** La descarga (`:27`), la recarga salvo el pico (`:28`, W1), el índice de docs (`:74`), la fila de `clawg-eval` (`:11`) y el párrafo de `load-eval` (`:174`) dicen lo que hace el código.
- **`clawg-eval` fuera de la release.** Con `cargo tree --locked -p clawg-cli -e features -i clawg-core --target all`, `-p clawg-cli` enlaza `clawg-core` con `default`, y con `testing` sólo en las dev-dependencies, nunca con `eval`. `LoadArm`, `load_once`, `fetch_model` y `CacheLock::read_now` viven bajo `eval` (o `test`). Lo único nuevo que entra en la release es `memmap2`, por `clawg-core`.
- **El `unsafe` del mapeo** lleva su comentario `SAFETY` (`model_download.rs:825-831`). Es el del diseño más una frase sobre `no_probe_handle`, que la fuente de `memmap2` confirma. Los `unsafe` del job object de `load-eval` (`measure.rs`) llevan el suyo.
- **`Cargo.lock`** sólo suma `memmap2 0.9.12`, cuyo checksum es el SHA-256 del `.crate` de la caché de cargo, y la línea de `windows-sys 0.61.2` en `clawg-eval`. `cargo update --workspace --dry-run` bloquea 0 paquetes. El `rust-version` de `memmap2` (1.65) no pasa el del workspace (1.90).
- **Mensajes y nombres.** Ni el código, ni los comentarios, ni los tests, ni los mensajes de commit nombran el cambio, la spec, fases o requisitos. Sólo los commits `docs(openspec)` nombran el cambio, como en los cambios anteriores. Ningún commit lleva `Co-Authored-By` ni atribución.
- **Directorios de build.** Al terminar sólo quedan `target` y `target-test`. `target-test` no tiene builds release y no queda ningún `load-eval-*` en el temporal.

## Frente al diseño

- **D1.** El diseño decía `Mmap::map`. El código usa `MmapOptions::new().no_probe_handle().map`, que garantiza la vista de sólo lectura sin sondear el handle. Es más estricto que el diseño.
- **D4.** El diseño decía que `each_file_is_hashed_once_per_run` vería el ONNX bajado una vez en `READS` y los chicos bajados ninguna. Como `READS` ahora anota también el hash que se calcula mientras llegan los bytes (`model_download.rs:1043`), el test ve el ONNX dos veces y cada chico una. Es lo que dicen la spec y la tarea 3.
- Lo demás (D2, D3, D5 a D9 y la tabla de archivos) se siguió.

## Hallazgos

**CRITICAL:** ninguno.

**WARNING:**

- **W1.** `.claude/CLAUDE.md:28` no dice que la línea de cada recarga nombra, en Windows, el pico privado de esa carga (o que siguió donde estaba), ni que ese pico no va a `metrics.db`. La tarea 1 lo marca hecho (`tasks.md:25`), pero `b04ac88` no tocó `.claude/CLAUDE.md`, y ningún commit posterior lo agrega. D9 pide que esa línea cambie en el mismo commit que cambia el comportamiento.
- **W2.** RM3 se prueba sólo sobre la pura `load_line`. Lo que la conecta en `Shared::load` (`idle_embedder.rs:181-182`, `:194-196`, `peak_before.zip(peak_after)`) no tiene test: con `peak_before` y `peak_after` cruzados, o leyendo `private` en lugar de `private_peak`, la suite sigue pasando. Los escenarios dicen «WHEN se recarga el modelo», así que su cobertura es parcial. Es un hueco conocido y aceptado: el crate no tiene con qué capturar el log, y armarlo no valía una línea. No bloquea.
- **W3.** `tasks.md:118` («Verificación: suite y clippy» del commit 7) quedó sin marcar. La corrida de esta verificación, sobre `6984ba4`, es esa verificación.

**SUGGESTION:**

- **S1.** La sentencia de `stored_vectors` (`project_docs/mod.rs:527`) es una sola línea de unos 330 caracteres, con tiras de espacios donde había saltos: parece una continuación `\` perdida. Anda, porque a SQL no le importan los espacios, pero no se lee. El `dump` de los tests del mismo archivo usa `\`.
- **S2.** `a_partly_reused_doc_matches_a_full_reindex_within_tolerance` carga el modelo con `ensure_model` (`project_docs/mod.rs:1465`), que baja lo que no verifica, aunque espera «the model is cached». Su `#[ignore]` (`:1459`) apunta a `<data dir>/models/multilingual-e5-small`, así que apuntado a un perfil podría escribir en él. `load_cached_model`, que usa `the_embedding_phase_of_a_real_export` (`portability/import.rs:5332`), nunca escribe.
- **S3.** Que la carga de producción suelte el archivo depende de quién es dueño de `files`, no de un test. Los tests sueltan la vista con `drop(view)` o `drop(load)`. Si un cambio futuro dejara vivos los `ModelFiles` (por ejemplo, guardados en el embebedor), una reparación esperaría en `blocking_write` mientras el modelo esté cargado, para siempre con `CLAWG_EMBEDDER_IDLE_MINUTES=0`. Un test de `unloading_when_idle` con una `OnnxFile` real y un renombre después lo fijaría.
- **S4.** En el párrafo de `load-eval` (`.claude/CLAUDE.md:174`), «`load_once` and `fetch_model` … are the only code that builds the first two» le atribuye a `fetch_model` algo que no hace: sólo baja el modelo, no arma ningún brazo.
- **S5.** La corrida de Linux de la puerta (`VmHWM`) confirmaría el mecanismo. No es normativa.

## TDD

El proyecto no escribe `apply-progress`: la evidencia está en `tasks.md`, donde cada «Test primero» dice por qué fallaba, y en las mutaciones de arriba. Cada test que nombra la spec existe y pasa. RM1, RM2 y RM5 tienen varios casos por escenario. RM3 está probado sólo en la función pura (W2).

Por capas:

- Los tests nuevos viven en `#[cfg(test)] mod tests`, varios con archivos reales, un servidor `mockito`, una sesión de ONNX Runtime desde el ONNX mínimo o un store SQLite.
- Hay uno con el modelo real (`#[ignore]`).
- No hay tests nuevos en `tests/binary.rs`.

No hay herramienta de cobertura instalada (`cargo llvm-cov`, `cargo tarpaulin`).

## Veredicto

Pasa con advertencias. RM1 a RM5 se cumplen en Windows, y la puerta dejó `View` con lo medido. Antes de archivar faltan la frase del pico en `.claude/CLAUDE.md` (W1), la marca de `tasks.md:118` (W3) y el CI del pull request en Linux. W2 queda aceptado.

Después de la verificación, W1 se cerró con `43771c6`: la frase de la recarga en `.claude/CLAUDE.md` dice ahora que en Windows cada carga nombra el pico privado que alcanzó, o que siguió donde lo dejó otra carga, y que eso no suma nada a `metrics.db`. W3 se cerró marcando `tasks.md:118`. Queda el CI del pull request en Linux.
