# embedder-footprint — Spec

Delta sobre `openspec/specs/storage/spec.md`, «Ciclo de vida del embedder», «Los mismos vectores que fastembed». Al archivar, su última frase («La sesión usa … tantos hilos intra-op como tiene la máquina») se reemplaza por el requisito EF1.

## EF1. Los hilos de la sesión, con respaldo

La sesión de ONNX Runtime MUST usar tantos hilos intra-op como informa `std::thread::available_parallelism()` (`session_threads`, `crates/clawg-core/src/memory/embedder.rs:540`). Si esa llamada falla, la sesión MUST armarse con `FALLBACK_SESSION_THREADS` = 2 (`embedder.rs:532`) y el daemon SHOULD registrar un `warn` con la causa. Una falla de esa llamada MUST NOT impedir la carga del modelo. La decisión es pura (`threads_from`, `embedder.rs:535`): no lee el sistema.

Los binarios de evaluación (`EvalMemo::open` y el bloque `index` de `rag-eval --docs`) MUST usar el mismo número, y MUST NOT fallar ni dejar de usar la memoria de vectores porque falte el dato.

#### Escenario: la máquina informa sus hilos
- GIVEN `available_parallelism()` devuelve 12
- WHEN se decide el número de hilos
- THEN es 12

#### Escenario: la máquina no los informa
- GIVEN `available_parallelism()` devuelve un error
- WHEN se decide el número de hilos
- THEN es 2, y la carga sigue

**Tests:** `the_session_takes_every_thread_the_machine_reports` y `an_unknown_parallelism_falls_back_to_two_threads` (`embedder.rs`); `session_threads_is_the_count_the_session_builds_with` (`eval_api.rs`).
