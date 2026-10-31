# embedder-footprint — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S, más una puerta de medición que puede cerrar sin código | — (la puerta se corre después de 54 y de 20; ver la [exploración](exploration.md), «Interacciones») | ver [`state.yaml`](state.yaml) |

## Problema

Verificado contra el código de hoy:

- **Si la máquina no dice cuántos hilos tiene, el modelo no carga nunca.** `session_threads` (`crates/clawg-core/src/memory/embedder.rs:530-534` antes de este cambio) convertía el error de `std::thread::available_parallelism()` en `EmbedError::LoadFailed`, y `session_builder` (`:541-552`) lo propagaba con `?`. Cada carga y cada recarga del `IdleEmbedder` vuelve a llamarlo, así que el error no se recupera solo. La biblioteca estándar documenta que la llamada puede fallar (por ejemplo, un cgroup o un `/proc` que no se puede leer). No hay valor de reemplazo, y los dos llamadores de evaluación (`EvalMemo::open`, `crates/clawg-core/src/eval_api/vector_memo.rs:395-407`, y el bloque `index` de `rag-eval --docs`, `crates/clawg-eval/src/bin/rag-eval/docs.rs:736-739`) cargaban con un camino de error propio por el mismo motivo.
- **El comentario de `session_builder` dice algo que lo medido contradice.** Decía que otro número de hilos «could move the last bits of a vector» (`embedder.rs:536-540`). En Linux x86_64 con AVX2, AVX-512F y FMA y 4 hilos, `taskset` a 1, 2 y 4 núcleos dio los mismos bits para los 32 vectores de `e5_golden` ([verify de embedder-idle-unload](../archive/2026-10-17-embedder-idle-unload/verify.md), línea 174). Ninguna otra máquina está medida.
- **Lo demás no está medido.** Cuánta CPU toman los hilos intra-op, la espera activa de ONNX Runtime, el grupo de `rayon` que arranca `encode_batch` y la triple codificación de `split_save` son las preguntas de la [exploración](exploration.md) («La puerta»). Responderlas pide el modelo real en Linux y en Windows y una máquina de 8 hilos lógicos o más, con SMT, porque con 4 hilos sin SMT la diferencia entre núcleos lógicos y físicos no se ve.

## Objetivo

Transversal: que el daemon, un proceso de fondo, no deje de cargar el modelo por un dato que no es esencial, y que lo que cueste a la máquina se decida con números. Si ninguna medición pasa su umbral, el cambio cierra con el respaldo y el registro de lo medido (enfoque F de la exploración).

## Decisiones

1. **Respaldo de 2 hilos.** Si `available_parallelism()` falla, la sesión se arma con `FALLBACK_SESSION_THREADS` = 2 y el log dice un `warn` con la causa. Se descarta 1: la falla es rara, y con un hilo cada carga, refresco de docs y re-embebido tardaría más. Se descarta seguir fallando: el número de hilos no cambia qué vector sale, así que no justifica dejar al daemon sin modelo.
2. **La decisión es pura.** `threads_from(io::Result<NonZeroUsize>) -> usize` decide y `session_threads()` lee el sistema y registra el aviso. Así la prueba del respaldo no necesita una máquina que falle.
3. **`session_threads` deja de devolver `Result`.** También la de `eval_api` (`pub fn session_threads() -> usize`). El camino «the vector memo is not used: the session's threads are unknown» de `EvalMemo::open` y el `EXIT_RUNTIME_FAILURE` de `docs.rs` desaparecen porque ya no pueden ocurrir. La clave del memo sigue incluyendo los hilos (spec de eval, «memoria de vectores»): sólo una máquina mostró que no mueven bits.
4. **El comentario dice lo medido y dónde.** «En Linux x86-64 de 4 hilos con AVX-512F, 1, 2 y 4 hilos dieron los mismos bits; otras máquinas no están medidas.»
5. **La puerta queda abierta y sus umbrales también.** Este cambio fija qué se mide y cómo (tareas 3 a 8), no los umbrales: siguen como decisiones abiertas de la exploración. Ninguna opción A a E entra sin su medición.

## Alcance

1. El respaldo de `available_parallelism()`, con su prueba pura y el aviso.
2. El comentario de `session_builder` corregido.
3. La guía de medición de la puerta, en [tasks.md](tasks.md), con lo que necesita cada paso.

## No-alcance

- Elegir A (espera activa), B (menos hilos), C (tokenizer en serie) o E (una sola codificación en `split_save`) sin su medición.
- Ordenar las tandas por largo (D): pasó a [docs-index-speed](../docs-index-speed/proposal.md).
- Las preguntas de memoria (a) y (b): pasaron a ram-footprint (90) y ram-footprint-gates (92), archivados.
- Cambiar el modelo, el contrato de embedding, `EMBED_BATCH` o la política de descarga por inactividad.

## Datos guardados

Ninguno cambia: ni esquema, ni identificadores, ni el contrato de embedding. Sin migración ni vuelta atrás.

## Cómo se mide el éxito

- `an_unknown_parallelism_falls_back_to_two_threads` (`embedder.rs`) falla hoy por no compilar: no existe una decisión pura con respaldo. Pasa con el cambio.
- `the_session_takes_every_thread_the_machine_reports` fija que, cuando el dato existe, nada cambia.
- `session_threads_is_the_count_the_session_builds_with` (`eval_api.rs`) sigue pasando con el tipo nuevo.
- La puerta, cuando se corra, registra sus números en `verify.md` y decide cada opción con los umbrales que se fijen antes de leerlos.
