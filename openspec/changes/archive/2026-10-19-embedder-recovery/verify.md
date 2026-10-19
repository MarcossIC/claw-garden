# embedder-recovery — Verificación

## Linux (2026-10-19)

Sobre `4cee085` (el último commit de código), en `target`, con rustc 1.97.0:

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Cada commit de código de las [tareas](tasks.md) (`2d4d5b5`, `c5e987c`, `4cee085`) pasó la suite y clippy por separado.
- `the_agent_contract_matches_its_file` pasa sin bendecir: nada de lo que el modelo ve del servidor MCP cambió.

## CI

`ci.yml` corre en el pull request de la rama, sobre Linux y Windows. Windows no se corrió a mano: el cambio no toca código que dependa de la plataforma (el `malloc_trim` y la arena única siguen igual), y el CI cubre la suite y clippy allí.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `a_panicking_inference_answers_its_caller_and_ends_the_worker`: no compilaba sin `serve_requests`; con el bucle de antes dentro de `serve_requests`, el pánico desenrollaba el hilo y el llamante recibía `WorkerGone`, no `InferenceFailed` |
| R2 | `a_request_whose_caller_is_gone_is_not_inferred`: con el bucle de antes, la inferencia corría dos veces |
| R3 | `a_model_whose_worker_is_gone_is_loaded_again_within_the_call` y `calls_that_find_the_worker_gone_together_share_one_reload`: no compilaban sin `RECOVERY_INTERVAL`; con la constante sola, la llamada devolvía `WorkerGone` y no había recarga |
| R4 | `a_model_that_keeps_dying_is_reloaded_at_most_once_a_minute`: con la constante sola, la primera llamada devolvía `WorkerGone` |
| R5 | `with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered`: no compilaba con el ocio como `Duration`; después de la tarea 2, su segunda mitad y `without_an_idle_time_the_loaded_model_is_still_wrapped` devolvían `WorkerGone` |

## Lo que encontró la verificación

- **El pánico de la primera llamada se ve.** La llamada cuya inferencia entra en pánico responde `InferenceFailed` (un `500` en búsqueda y `recall_context`; en el ingest, `embed failed: inference panicked: …`, que deja el save `failed` con esa causa como cualquier `InferenceFailed` de hoy, `crates/clawg-core/src/memory/worker.rs:765-766`; reintentarlo es de 53); es la siguiente la que encuentra el hilo muerto y recarga. Es lo que decide D1: esa llamada recibe la causa, y repetirla sobre una sesión recién cargada no se intentó porque un pánico determinista la repetiría.
- **Una recarga de recuperación que falla no retiene las siguientes.** Si la caché no pasa al recuperar, la llamada responde «no listo», lanza la reparación de 60, y las llamadas siguientes cargan como con el modelo suelto, sin esperar el minuto: la retención sólo se arma cuando un modelo recién cargado vuelve a morir. Coincide con R4, que sólo limita las recargas que reemplazan un modelo muerto.

## Diferencias entre el diseño y el código

- **D3** describe la retención como «hasta cuándo se retienen las recargas»; el código guarda `recovering` y `last_recovery` en `Slot` y compara contra `RECOVERY_INTERVAL` al recargar, que es lo mismo sin guardar el instante final.

## No aplica

- `rag-eval`: el ranking no cambia.
- Probar a mano con el modelo real: no hay un camino conocido de pánico en la inferencia ([diseño](design.md#probar-a-mano)).
