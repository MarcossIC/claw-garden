# embedder-recovery — Diseño

## D1. El bucle del hilo, aparte y con `catch_unwind` (R1, R2)

**Elegido:** el bucle de `E5Embedder::from_parts` pasa a `serve_requests(req_rx, infer)`, una función que recibe la cola y la inferencia como un `FnMut(&[String]) -> Result<Vec<Vec<f32>>, EmbedError>`. Para cada petición:

1. Si el llamante ya soltó su respuesta (`oneshot::Sender::is_closed`), la salta.
2. Corre `infer` dentro de `std::panic::catch_unwind(AssertUnwindSafe(..))`.
3. Si entra en pánico, contesta `InferenceFailed("inference panicked: <causa>")`, deja un `error!` y vuelve. `from_parts` suelta entonces la sesión y devuelve la memoria, como al terminar normalmente.

Al volver, la cola se suelta: las peticiones que esperaban pierden su respuesta y su llamante ve `WorkerGone`, que D2 recupera. Separar el bucle deja probarlo con una inferencia falsa, sin una sesión de ONNX.

| Alternativa | Por qué no |
|---|---|
| A1: el hilo sigue con la misma sesión | `AssertUnwindSafe` es una promesa que nada comprueba; una sesión tomada por `&mut` a mitad de un pánico puede repetirlo. Soltarla y recargar no depende de eso |
| `panic = "abort"` | Mata el daemon entero por un fallo de inferencia, y el puente no arranca otro a mitad de sesión |

**Saltar lo cancelado** tiene una carrera aceptada: el llamante puede irse justo después de la comprobación, y eso cuesta a lo sumo un lote (`EMBED_BATCH` = 16 textos).

## D2. `IdleEmbedder` descarta y reintenta una vez (R3)

**Elegido:** las dos llamadas de `Embedder` pasan por `with_model`, que pide el modelo con `lease`, llama, y si la respuesta es `WorkerGone`:

1. `discard(&model)`: bajo el cerrojo del modelo, si la celda todavía tiene ese mismo `Arc` (`Arc::ptr_eq`), la vacía y pone `resident = false`; si tiene otro, otra llamada ya lo recuperó y no toca nada.
2. Pide el modelo otra vez con `lease`, que lo recarga (una sola vez para todas las llamadas que esperan el cerrojo) o devuelve el que ya recargó otra, y repite la llamada.
3. Si la repetición vuelve a dar `WorkerGone`, descarta otra vez y responde `NotReady`.

Así `WorkerGone` no sale de `IdleEmbedder`: el ingest ya lo trataba como `NotReady` (`crates/clawg-core/src/memory/worker.rs:762`), y búsqueda y `recall_context` dan el mismo `503 embedder_not_ready` que una recarga que falla, sin tocar `api.rs`.

| Alternativa | Por qué no |
|---|---|
| Fallar y que recargue la llamada siguiente | La llamada que descubre el fallo es la de alguien que espera un vector; con el reintento sólo ve un error si la recarga también falla |
| Reintentar sin descartar | El mismo `Arc` apunta a un canal sin receptor: volvería a fallar |
| Un estado nuevo en `/health` | `failed` hoy es «no cargó» y trae `embedder_retryable`; la recuperación es pasajera y ya tiene salida. `embedder_resident` dice `false` mientras dura |

## D3. Una recuperación por minuto (R4)

**Elegido:** junto al modelo, bajo el mismo cerrojo, `Slot` guarda cuándo fue la última recuperación y hasta cuándo se retienen las recargas. `discard` que vacía la celda mira la anterior: si fue hace menos de `RECOVERY_INTERVAL` (60 s), retiene las recargas hasta que se cumpla ese minuto; siempre anota la nueva. `lease` con la celda vacía y una retención vigente responde `NotReady` sin recargar.

Un fallo determinista en la inferencia recarga el modelo (leer y verificar unos 470 MB y armar la sesión) a lo sumo una vez por minuto en vez de en cada llamada. La primera recuperación nunca espera: un hilo que muere una vez vuelve en la misma llamada. La recarga del modelo soltado por ocio no pasa por `discard` y no se retiene.

| Alternativa | Por qué no |
|---|---|
| Contar fallos seguidos y dejar de recargar | Deja la memoria apagada hasta reiniciar; con un minuto, un fallo pasajero se va solo |
| Un retardo creciente | Más estado para un caso sin camino conocido; un minuto ya acota el costo a una recarga por minuto |

## D4. Ocio `0`: siempre envuelto, sin barrido (R5)

**Elegido:** `IdleEmbedder::new` y `unloading_when_idle` reciben `Option<Duration>`; con `None` no se arma el barrido. `clawg serve` envuelve siempre, con `idle_after` tal cual (`None` para `0`). La recarga, su vuelo único, la reparación de la caché de 60 y la recuperación son las mismas.

| Alternativa | Por qué no |
|---|---|
| C2: que `E5Embedder` reponga su hilo | Duplica la carga de los archivos y la verificación de identidad que ya hace la recarga |
| C3: documentarlo | Deja un hilo muerto sin remedio hasta reiniciar |

## Probar a mano

No hay un camino conocido que haga entrar en pánico a la inferencia real, así que no hay una prueba a mano con el modelo: los tests de R1 a R5 usan una inferencia y modelos falsos.
