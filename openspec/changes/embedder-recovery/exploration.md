# embedder-recovery — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S | — (53 outbox-lifecycle depende de éste; ver «Interacciones») | exploración (borrador) |

**Tesis:** un hilo de inferencia roto se recupera solo, también con `CLAWG_EMBEDDER_IDLE_MINUTES=0`, y el daemon no gasta inferencia en peticiones que nadie espera. Hoy, si el hilo `embedder-worker` muere, cada llamada siguiente devuelve `WorkerGone` y nada lo reemplaza hasta que el daemon se reinicie (o, con el ocio activo, hasta que pase un período completo sin una sola llamada, que casi nunca ocurre porque las llamadas fallidas cuentan como uso). Mientras tanto `/health` dice `ready` y `embedder_resident: true`. No se encontró un camino concreto que haga entrar en pánico al hilo, así que la gravedad es de resiliencia, no de un fallo observado. Nada de esto toca `memory.db` ni el contrato de embeddings: los vectores que produce el modelo no cambian.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El bucle del hilo no tiene `catch_unwind` | **Cierta.** El bucle está en `crates/clawg-core/src/memory/embedder.rs:548-553` y llama a `inference.embed(&texts)` sin protección; `rg catch_unwind` no da nada en el repositorio. El `Cargo.toml` raíz sólo define `[profile.test]` (`Cargo.toml:71`): no hay `panic = "abort"`, así que un pánico desenrolla y mata sólo ese hilo. |
| Con el hilo muerto toda llamada devuelve `WorkerGone` | **Cierta.** El `send` falla (`embedder.rs:413-414`) o el oneshot se cae sin respuesta (`:417`). Lo prueba el mock con el receptor soltado (`embedder.rs:1309`). |
| `IdleEmbedder::lease` devuelve el modelo guardado mientras exista | **Cierta.** `idle_embedder.rs:101-103`. La celda sólo se vacía en `unload_if_idle` (`:146`) y en `unload`, que es de pruebas (`:197-200`, `#[cfg(any(test, feature = "testing"))]`). Ninguna corre ante `WorkerGone`: `embed_with_role` y `embed_batch` devuelven el error tal cual (`:206-214`). |
| El temporizador de ocio no rescata el caso | **Cierta.** `InFlight::drop` sella `last_used` en cada llamada, también en las fallidas (`idle_embedder.rs:88-93`), y `should_unload` exige `in_flight == 0` y el período completo (`:53-55`). Con una sola llamada por período no se descarga nunca; el síntoma es un 500 persistente. |
| Con ocio `0` no hay recuperación posible | **Cierta.** `serve.rs:114-123`: con `idle = None` el `load` queda sin envolver, no existe `IdleEmbedder` y la celda guarda directamente el `E5Embedder` muerto. |
| `/health` sigue `ready` y `resident` en true | **Cierta.** `embedder_status` sólo cambia en el arranque; `embedder_resident` lee `resident()` (`api.rs:920`), que en `IdleEmbedder` es un `AtomicBool` que sólo escriben la carga (`idle_embedder.rs:116`) y el descarte (`:145`); en un `E5Embedder` pelado es el `true` por defecto (`embedder.rs:93-95`). |
| Hay un camino concreto de pánico | **Falsa, no encontrado.** Revisé `window_encodings` (`embedder.rs:216-231`, errores propagados con `?`), `pad_right` (`:235-255`, la resta `len - encoding.len()` no desborda porque `len` es el máximo), `pool_and_normalize` (`:277-316`, valida longitudes antes de indexar; `hidden.chunks(dim.max(1))` evita el cero), `session.run` (`ort-2.0.0-rc.12/src/session/mod.rs:212-320`, el fallo de ORT vuelve como `Err` por `?`). Quedan dos puntos: `outputs[self.output.as_str()]` (`embedder.rs:488`) indexa y entra en pánico si falta la clave, pero el nombre se tomó de la propia sesión (`:538-539`); y `tokenizers` (`encode_batch`) y el código nativo de ORT, que no audité. |
| Existe una variante de error para devolver | **Cierta.** `EmbedError::InferenceFailed(String)` (`embedder.rs:192-193`). |
| Una petición cancelada igual se infiere | **Cierta.** `let _ = resp.send(inference.embed(&texts))` (`embedder.rs:552`) no mira si el receptor se cerró. |
| Eso puede acumular trabajo sin tope | **Falsa.** `embed_batch` envía un lote de a lo sumo `EMBED_BATCH` = 16 (`embedder.rs:160`, `:407`) y espera la respuesta antes del siguiente (`:417`). El canal es ilimitado (`:542`), pero lo que hay en cola está acotado por los llamadores concurrentes, y un futuro soltado desperdicia a lo sumo un lote. |
| Un pánico deja la sesión de ORT inconsistente | **Matiz.** Un pánico del lado Rust antes o después de `Run` no deja nada a medias en ORT, pero `Inference` se tomó por `&mut` y no hay garantía de que reusarla no repita el pánico. Soltar la sesión y recargar lo evita sin depender de eso. |

## Problema (verificado contra el código)

1. **El fallo no se reconoce.** `WorkerGone` es un error de llamada, no un estado. El ingest lo convierte en `EmbedderUnavailable` (`crates/clawg-core/src/memory/worker.rs:762-763`), que por diseño es transitorio y se reintenta al próximo arranque; búsqueda y `recall_context` lo contestan como `500` (`embed_failure`, `api.rs:1136-1140`: sólo `NotReady` es `503 embedder_not_ready`). Ninguno de los dos hace nada por el hilo.
2. **La recuperación existe sólo con ocio y sólo por casualidad.** La recarga (`lease`, `idle_embedder.rs:104-117`) ya es de una sola vez bajo el `Mutex` y comparte la carga entre llamadas; falta que algo vacíe la celda cuando el modelo está roto.
3. **Con ocio `0` la recarga no está cableada** (`serve.rs:114-123`), aunque `unloading_when_idle` (`bootstrap.rs:381-412`) ya construye todo lo necesario.
4. **Inferencia para nadie.** Un hilo que acaba de recibir un lote cuyo receptor ya se cerró lo infiere igual. Es poco (un lote por llamador), pero es CPU y memoria de pico sin destinatario.

## Lo que ya existe y se reutiliza

- La recarga de un solo vuelo de `Shared::lease` y `same_model`, que rechaza una identidad distinta (`idle_embedder.rs:96-133`).
- `EmbedError::InferenceFailed` (`embedder.rs:192`) y `ReloadFn` (`idle_embedder.rs:23-24`).
- `E5Embedder::from_mock_worker` (`embedder.rs:616`) y los mocks de hilo ya usados en pruebas (`:1199`, `:1260`, `:1309`); `counting_reload` e `idle_embedder` en las pruebas de `idle_embedder.rs:262-275`, con `start_paused`.
- La política de `/health` ya escrita: el estado sigue `ready` durante las recargas (`openspec/specs/storage/spec.md`, «Ciclo de vida del embedder»).

## Enfoques posibles

**A. `catch_unwind` en el hilo (recomendado).** `std::panic::catch_unwind(AssertUnwindSafe(|| inference.embed(&texts)))`; un pánico se contesta como `InferenceFailed("inference panicked: …")` para esa petición. Variante A1: el hilo sigue. Variante A2 (recomendada): contesta y termina el bucle, soltando la sesión, de modo que la siguiente llamada vea `WorkerGone` y la recuperación de B lo recargue; no reusa una sesión que acaba de entrar en pánico.
- **A favor:** el que pidió el vector recibe un error con causa en vez de un oneshot caído; cuesta unas líneas.
- **En contra:** `AssertUnwindSafe` es una promesa que no probamos; con A2 depende de B para que valga la pena.

**B. Vaciar la celda ante `WorkerGone` (recomendado).** `IdleEmbedder` descarta el modelo que acaba de fallar, comparando por `Arc::ptr_eq` bajo el `Mutex` (una recarga concurrente ya hecha no se pierde), pone `resident = false` y la siguiente llamada recarga. La recarga es la misma de 46, la que acota 60.
- **A favor:** sin tiempo de espera ni sondeo; sólo actúa cuando falla.
- **En contra:** la llamada que descubre el fallo sigue fallando, salvo que se reintente una vez dentro de la misma llamada (decisión abierta).

**C. Ocio `0` sin `IdleEmbedder`.**
- **C1 (recomendado):** envolver siempre; `unloading_when_idle` recibe `Option<Duration>` y con `None` el barrido no se arma. Reutiliza la recarga, su vuelo único y las pruebas. `embedder_resident` pasa a ser siempre `Some`, que ya es el contrato (`api.rs:920`).
- **C2:** que `E5Embedder` reponga su propio hilo. Duplica la carga de archivos y la verificación de identidad.
- **C3:** documentar y dejarlo. Lo deja sin remedio hasta reiniciar.

**D. Saltar lo cancelado (recomendado, barato).** `if resp.is_closed() { continue }` antes de inferir (`embedder.rs:548-553`).
- **En contra:** una carrera: el receptor puede cerrarse justo después de la comprobación; sigue siendo, a lo sumo, un lote.

**E. `/health` ante el hilo muerto.** Mantener `ready` (se recargará en la próxima llamada, coherente con 46) frente a un estado nuevo. Se recomienda mantener `ready` y dejar el fallo en el log: con B el estado es transitorio y ya tiene salida.

## Descartado

| Opción | Razón |
|---|---|
| Cola con prioridad para consultas interactivas | Cada llamador tiene un lote en vuelo (`embedder.rs:407-417`), así que una consulta espera, como mucho, un lote por productor concurrente; no hay cola larga que reordenar. Se reabre si 62 mide otra cosa. |
| Canal acotado | Cambia la firma de `req_tx` y no resuelve nada que D no resuelva con menos. |
| `panic = "abort"` | Mata el daemon entero por un fallo de inferencia. Lo aceptado sigue en el outbox, pero la memoria no vuelve hasta que algo arranque otro daemon, y el puente no lo hace a mitad de sesión. |
| Estado `failed` en `/health` por un hilo muerto | `failed` hoy significa «no cargó» y trae `embedder_retryable` (`api.rs:899-903`); no es lo que pasa y cambia un contrato fijado en `specs/mcp`. |

## Interacciones

- **outbox-lifecycle (53).** Su enfoque E reintenta en la sesión los fallos pasajeros y cita «el trabajador del embedder caído» como uno. Contra el mismo `Arc<dyn Embedder>` muerto el reintento fallaría para siempre: es este cambio el que lo vuelve pasajero, así que **61 aterriza antes que 53**.
- **model-download (60).** Acota la descarga y propone que la recarga no descargue bajo el `Mutex`; ambos tocan `Shared::lease` (`idle_embedder.rs:97-118`). Hay que acordar el orden de los dos parches; B añade un punto de entrada a esa misma recarga. Archivado el 2026-10-19 sin tocar `idle_embedder.rs`: la lectura de la caché y la reparación viven en el `ReloadFn` que arma `unloading_when_idle` (`crates/clawg-core/src/memory/bootstrap.rs:405`), así que `lease` sigue como lo describe esta exploración.
- **embedder-idle-unload (46, archivado).** `specs/storage` «Ciclo de vida del embedder» dice que una recarga fallida responde `NotReady` y que `embedder` sigue `ready`; no cambia. Se agrega un escenario de recuperación.
- **shutdown-budget (55).** Las recargas corren en `spawn_blocking` (`bootstrap.rs:396`); más recargas posibles, mismo tratamiento.
- **embedder-footprint (62).** Mide CPU y memoria y tocará `build_session` y `embed_batch`; D y A tocan el bucle vecino.

## Alcance candidato

1. `catch_unwind` en el bucle del hilo, con `InferenceFailed` (A2).
2. `IdleEmbedder` vacía su celda ante `WorkerGone` y marca `resident = false` (B).
3. `unloading_when_idle` también con ocio `0`, sin barrido (C1).
4. Saltar peticiones con el receptor cerrado (D).
5. Escenarios nuevos en `specs/storage` y un comentario al día en `lease`.

### No-alcance

- Cola con prioridad, canal acotado, cambios de contrato en `/health`.
- Cualquier cambio a `memory.db`, al contrato de embeddings o a los vectores.
- La descarga y verificación de archivos (60) y la medición de recursos (62).
- Reintentar filas del outbox (53).

## Decisiones abiertas

Son del usuario.

- [ ] **Tras un pánico.** ¿El hilo sigue (A1) o suelta la sesión y termina (A2)?
- [ ] **La llamada que descubre el fallo.** ¿Falla y la próxima recarga, o reintenta una vez tras la recarga? Lo segundo evita un 500 visible a costa de una carga dentro de la llamada.
- [ ] **Ocio `0`.** ¿C1, C2 o C3?
- [ ] **`/health`.** ¿Se mantiene `ready` mientras el hilo está muerto, o se añade un estado?
- [ ] **Código de la respuesta.** `WorkerGone` hoy sale `500` (`api.rs:1139`); ¿pasa a `503 embedder_not_ready` como una recarga fallida?

## Cómo se mide

Tests RED primero (TDD estricto), todo con mocks, sin modelo real, red ni perfil:

1. **Pánico en el hilo.** Un mock con `from_mock_worker` cuyo hilo entra en pánico en la primera petición: hoy la segunda devuelve `WorkerGone` para siempre; con el cambio la primera devuelve `InferenceFailed` y la segunda tiene éxito (A1) o recarga (A2 con B).
2. **Celda con `WorkerGone`.** Un `IdleEmbedder` cuyo modelo devuelve `WorkerGone`: hoy la celda se queda; con el cambio la siguiente llamada invoca la recarga (`counting_reload`) y `resident()` pasa por `false`.
3. **No pierde una recarga concurrente.** Dos llamadas fallan a la vez: la recarga ocurre una vez y el modelo nuevo no se descarta (`ptr_eq`).
4. **Ocio `0`.** Con `None`, tras `WorkerGone` la llamada siguiente recarga y el barrido no descarga nunca (`start_paused`).
5. **Cancelado.** Se envía una petición, se suelta su futuro y se comprueba que la inferencia del mock no se llama.
6. **Escenario de `specs/storage`** para cada comportamiento, con su prueba nombrada.

## Riesgos

- **Falsa seguridad:** sin un camino de pánico conocido, el cambio cubre un fallo hipotético; el costo es bajo, pero hay que decirlo en el spec.
- **Bucle de recargas:** un fallo determinista al inferir (A2 + B) recargaría el modelo en cada llamada (leer y verificar unos 470 MB y armar la sesión otra vez). Hace falta un retardo mínimo entre recargas o contar fallos seguidos; a decidir en el diseño.
- **`AssertUnwindSafe`:** no prueba nada; A2 lo limita al no reusar la sesión.
- **Carrera de `ptr_eq`:** vaciar la celda equivocada tiraría un modelo sano; lo cubre la prueba 3.
- **Choque con 60** en `lease`: no hubo; 60 se archivó sin tocar `lease`.
