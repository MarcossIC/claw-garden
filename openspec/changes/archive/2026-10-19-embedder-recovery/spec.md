# embedder-recovery — Especificación

## Definiciones

- **Hilo de inferencia:** el hilo `embedder-worker` que es dueño de la sesión de ONNX Runtime de un `E5Embedder`.
- **Hilo muerto:** una llamada al modelo devuelve `EmbedError::WorkerGone`: el hilo terminó, o soltó la petición sin contestarla.
- **Recuperación:** descartar el modelo cuyo hilo murió y cargarlo otra vez con la misma recarga del modelo suelto.

No se conoce un camino que haga entrar en pánico a la inferencia; estos requisitos cubren un fallo posible, no uno observado.

## R1. Un pánico contesta y suelta la sesión

Cuando la inferencia de una petición entra en pánico, el hilo MUST contestar esa petición con `EmbedError::InferenceFailed`, cuyo texto MUST decir `inference panicked` y la causa del pánico, MUST dejarlo en el log y MUST terminar, soltando la sesión. MUST NOT inferir otra petición con esa sesión. Las peticiones que esperaban en la cola MUST recibir `WorkerGone`.

#### Escenario: la primera petición entra en pánico
- GIVEN un hilo cuya inferencia entra en pánico
- WHEN llega una petición
- THEN su llamante recibe `InferenceFailed` con `inference panicked`
- AND la petición siguiente recibe `WorkerGone`, y el hilo terminó

**Tests:** `a_panicking_inference_answers_its_caller_and_ends_the_worker` (`embedder.rs`).

## R2. Lo que nadie espera no se infiere

El hilo MUST NOT inferir una petición cuyo llamante ya soltó la respuesta cuando el hilo la toma. Puede inferir una que se suelta durante la inferencia: a lo sumo un lote por llamante.

#### Escenario: un llamante que se fue
- GIVEN una petición en cola cuyo llamante se fue y otra cuyo llamante espera
- WHEN el hilo las toma
- THEN la inferencia corre una vez, para la segunda

**Tests:** `a_request_whose_caller_is_gone_is_not_inferred` (`embedder.rs`).

## R3. Un hilo muerto se recupera en la misma llamada

Cuando el modelo devuelve `WorkerGone`, `IdleEmbedder` MUST descartarlo, salvo que la celda ya tenga otro modelo (una recuperación concurrente, comparada por `Arc::ptr_eq`), MUST marcarlo no residente y MUST repetir la llamada una vez sobre un modelo cargado con la misma recarga del modelo suelto. Las llamadas que encuentran el mismo hilo muerto a la vez MUST compartir una sola recarga. Si la repetición también encuentra el hilo muerto, la llamada MUST responder `NotReady`. `WorkerGone` MUST NOT salir de `IdleEmbedder`. El estado del embedder en `/health` MUST seguir en `ready`.

#### Escenario: el hilo murió entre dos llamadas
- GIVEN un modelo residente cuyo hilo terminó
- WHEN llega una búsqueda
- THEN el modelo se recarga una vez y la búsqueda responde

#### Escenario: dos llamadas a la vez
- GIVEN dos llamadas en curso sobre el mismo modelo cuyo hilo muere
- WHEN las dos reciben `WorkerGone`
- THEN el modelo se recarga una vez y las dos responden con el modelo nuevo

**Tests:** `a_model_whose_worker_is_gone_is_loaded_again_within_the_call` y `calls_that_find_the_worker_gone_together_share_one_reload` (`idle_embedder.rs`).

## R4. A lo sumo una recuperación por minuto

Una recuperación que llega menos de un minuto después de la anterior MUST NOT recargar: el modelo queda descartado y toda llamada MUST responder `NotReady` hasta que se cumpla el minuto desde la recuperación anterior; la primera llamada después MUST recargar. Una recarga del modelo soltado por ocio no cuenta como recuperación y no espera.

#### Escenario: un modelo que muere en cada recarga
- GIVEN un modelo cuyo hilo muere en cada llamada, también después de recargarlo
- WHEN llegan llamadas seguidas durante un minuto
- THEN el modelo se recarga una vez en ese minuto, y las llamadas responden `NotReady`
- AND la primera llamada pasado el minuto lo recarga otra vez

**Tests:** `a_model_that_keeps_dying_is_reloaded_at_most_once_a_minute` (`idle_embedder.rs`).

## R5. Con ocio `0` también

Con `CLAWG_EMBEDDER_IDLE_MINUTES=0`, `clawg serve` MUST envolver el modelo en `IdleEmbedder` sin barrido: MUST NOT soltarlo por ocio y MUST recuperar un hilo muerto como en R3 y R4.

#### Escenario: ocio `0`
- GIVEN un `IdleEmbedder` sin tiempo de ocio
- WHEN pasan horas sin llamadas
- THEN el modelo sigue residente
- AND si su hilo muere, la llamada siguiente lo recarga y responde

**Tests:** `with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered` (`idle_embedder.rs`); `without_an_idle_time_the_loaded_model_is_still_wrapped` (`bootstrap.rs`).
