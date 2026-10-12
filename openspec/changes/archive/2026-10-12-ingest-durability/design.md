# ingest-durability — Diseño

Este documento registra sólo las decisiones con alternativas reales. Lo que ya fija la spec (códigos, textos, constantes) no se repite.

## D1. Cómo vuelve el resultado del worker al handler

El worker es el único consumidor de la cola, y el dedup depende de eso (`crates/clawg-core/src/runtime.rs:116-124`, `crates/clawg-cli/src/daemon/api.rs:420-431`). El handler tiene que esperar un resultado que produce otra tarea.

| Opción | A favor | En contra |
|---|---|---|
| **a. Sobre en la cola:** `IngestRequest { event: MemoryEvent, reply: Option<oneshot::Sender<IngestResult>> }` | El resultado va directo al request que lo pidió. Un `None` mantiene el camino "disparar y olvidar" para los tests y los productores que no esperan | Cambia el tipo público de `memory_tx` y de `ingest_worker` (`runtime.rs:50`, `:125-128`). Hay que tocar `crates/clawg-core/tests/public_api.rs:155`, `:195`, `:237` y `crates/clawg-cli/tests/api.rs:78-87` |
| b. Campo `#[serde(skip)] reply` dentro de `MemoryEvent` | No cambia el tipo de la cola | `MemoryEvent` deriva `Clone` y `Serialize` (`crates/clawg-core/src/memory/events.rs:12`). Un `oneshot::Sender` no es `Clone`, así que habría que sacar el derive o envolverlo en `Arc<Mutex<Option<…>>>`. Mezcla transporte con dato |
| c. Canal de resultados aparte, con un id por request (`broadcast` o mapa de `oneshot`) | No toca la cola | Otro estado compartido, ids de correlación y limpieza de entradas huérfanas cuando vence el timeout. Más código para el mismo resultado |

**Decisión: a.** El sobre es la forma más chica que conserva un solo consumidor. Si el handler ya no espera (timeout), el `send` del worker sobre un `oneshot` cerrado falla y se ignora: no hace falta limpieza.

`process_event` (`crates/clawg-core/src/memory/worker.rs:254-380`) pasa de `Result<(), MemoryWorkerError>` a devolver lo escrito y el fallo, si lo hubo:

```rust
pub struct IngestReport {
    pub memories: Vec<SavedChunk>,          // id + Inserted | Reinforced, en orden
    pub failure: Option<IngestFailure>,     // EmbedderUnavailable | Failed(String)
}
```

Un tipo con las dos partes, y no un `Result`, porque R3 pide devolver las filas escritas **junto** con el error. Los `return Ok(())` de los caminos de descarte (`worker.rs:323-331`, `:333-342`) y el `continue` del insert (`:369-372`) pasan a cargar `failure`. Hoy el insert sigue con los chunks siguientes después de un fallo; con R3 el evento se corta en el primer fallo, porque seguir dejaría huecos en el medio de un texto que el agente cree guardado entero o no guardado.

`absorb_duplicate` (`worker.rs:225-252`) ya recibe el id de `reinforce_if_duplicate`; pasa a devolverlo en lugar de un `bool`.

## D2. Dónde vive el estado del embedder

Hoy el estado es implícito: `OnceCell` lleno es `ready`, vacío es `loading` (`runtime.rs:59`, `:155-157`). Hace falta un tercer valor, `failed`, con causa.

| Opción | A favor | En contra |
|---|---|---|
| **a. `watch::Sender<EmbedderStatus>` junto al `OnceCell`** | Los lectores del embedder (todos los `embedder.get()` y `embedder_ready()` de producción) no cambian. El `watch` además permite esperar el cambio de estado sin polling | Dos fuentes que tienen que estar de acuerdo. Se cuida con un orden fijo: el bootstrap llena el cell **antes** de publicar `Ready` |
| b. Reemplazar el `OnceCell` por un `RwLock<EmbedderState>` con el embedder adentro | Una sola fuente | Toca todos los lectores, y pierde la garantía de "se escribe una vez" que el comentario de `runtime.rs:52-58` defiende |
| c. Un `Mutex<Option<String>>` sólo para el error | Mínimo | No sirve para esperar el cambio, y deja el estado repartido en dos tipos sin relación |

**Decisión: a.** `MemoryRuntime` suma `embedder_status: watch::Receiver<EmbedderStatus>` y el bootstrap recibe el `Sender`. `embedder_ready()` sigue mirando el cell, que es lo que importa para usarlo.

```rust
pub enum EmbedderStatus {
    Loading,
    Ready,
    Failed { error: String, retryable: bool },
}
```

`wait_for_embedder` (`worker.rs:191-206`) puede seguir haciendo polling del cell. Cambiarlo al `watch` es una mejora aparte y no hace falta para esta spec.

## D3. Bucle de reintento del bootstrap

| Opción | A favor | En contra |
|---|---|---|
| **a. `run_embedder_bootstrap` devuelve un resultado y un bucle nuevo, `run_embedder_bootstrap_with_retry`, decide** | La función actual cambia poco: deja de devolver `()` (`crates/clawg-core/src/memory/bootstrap.rs:204-211`). La decisión de reintentar es una función pura (`classify` + `retry_delay`) testeable sin red | Una función más |
| b. El reintento adentro de `run_embedder_bootstrap` | Un solo punto de entrada | Mezcla un intento con la política. Los tests actuales (`bootstrap.rs:744`, `:792`, `:868`) asumen un intento por llamada y habría que reescribirlos |
| c. Reintento sólo desde `clawg status` o un comando manual | Sin tareas de fondo nuevas | El usuario tiene que darse cuenta y actuar. Es justo el silencio que este cambio quiere sacar |

**Decisión: a.**

- `classify(&BootstrapError) -> Retry | GiveUp`: `Mismatch` es `GiveUp`; descarga, carga y error de verificación son `Retry`.
- `retry_delay(attempt: u32) -> Duration`: `min(30 s · 2^(attempt-1), 30 min)`, con saturación para no desbordar.
- El bucle recibe la espera como parámetro (o un reloj inyectado), así el test corre con esperas en cero.
- `crates/clawg-cli/src/daemon/serve.rs:84-91` lanza el bucle en vez de un intento.

El bucle no tiene `JoinHandle` igual que hoy. Pararlo al cerrar el daemon es parte del apagado ordenado de data-preservation, parte A.

## D4. Cola acotada sin bloquear

El comentario de `runtime.rs:47-49` pide que el productor nunca se bloquee detrás del embedding. Eso sigue valiendo.

| Opción | A favor | En contra |
|---|---|---|
| **a. `mpsc::channel(1024)` con `try_send`** | Nunca bloquea: con la cola llena, `try_send` devuelve `Full` al instante y el handler responde `503`. El tipo ya sabe su capacidad | Cambia `UnboundedSender` → `Sender` y `UnboundedReceiver` → `Receiver` en la API pública. Los tests que hacen `send(...)` sincrónico pasan a `try_send` |
| b. `unbounded_channel` más un contador atómico de profundidad | No cambia los tipos | El contador puede desincronizarse del canal y es un segundo mecanismo para lo que el canal acotado ya hace |
| c. Sin límite (como hoy), con el rechazo por `failed` como única defensa | Cero cambios | Mientras el modelo carga (`loading`, que en la primera instalación es una descarga de 470 MB) la cola no tiene techo |

**Decisión: a.** El cambio de tipos se hace en el mismo commit que D1, porque los dos tocan la misma firma.

La capacidad se inyecta en el constructor de prueba (`MemoryRuntime::new_for_test`, `runtime.rs:159-201`) para que R9 se pruebe con capacidad 2. Producción usa la constante de 1.024.

## D5. Decisión del handler, separada del I/O

El handler hace tres cosas con I/O (leer el estado, encolar, esperar con timeout) y una decisión: qué responder. La decisión es una función pura:

```rust
fn save_response(status: &EmbedderStatus, enqueue: Enqueue, wait: Option<Waited>) -> (StatusCode, Body)
```

- `Failed` → `503 embedder_failed`, sin encolar.
- `Loading` → encola sin `reply` → `202 embedder_loading`, o `503 ingest_queue_full`.
- `Ready` → encola con `reply` → espera con `tokio::time::timeout` → `201`, `202 unconfirmed`, `500`/`503`.

El texto que muestra la herramienta MCP (R10) también es una función pura sobre la respuesta parseada, separada de `save_memory_impl` (`crates/clawg-cli/src/mcp/server.rs:246-281`).

El timeout vive en `AppCtx` (`api.rs:55-64`) con el default de 10 s, para que el test de R4 use 100 ms.

## Lo que no se decidió acá

- El valor final del timeout: sale de medir el p50 y el p99 del save con el embedder real (propuesta, "Éxito").
- Qué pasa con los eventos retenidos cuando el estado pasa a `failed` sin reintento: quedan en la cola hasta que el daemon pare. Persistirlos o drenarlos es data-preservation, partes A y D.
