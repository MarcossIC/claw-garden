# embedder-idle-unload — Diseño

Las decisiones de la [propuesta](proposal.md), con las alternativas descartadas.

## D1. La memoria vuelve (medición)

El paso 0 de la [exploración](exploration.md#paso-0-la-memoria-vuelve-al-soltar-el-modelo) decide entre soltar el modelo en el proceso (A) y un proceso hijo (C). Se corrió en Linux (glibc, contenedor de desarrollo, build debug) con un ejemplo descartable que no se commitea: `ensure_model` sobre un directorio temporal, `FastEmbedLocal::from_bytes`, una consulta y un lote de 64 pasajes, soltar el `Arc`, 3 s de pausa y leer `/proc/self/status`. Cinco vueltas en el mismo proceso. Criterio fijado antes: menos de 300 MB tras soltarlo y sin crecer entre la primera y la quinta vuelta.

| Momento | Sin devolver la memoria | Con `malloc_trim(0)` al terminar el hilo del modelo |
|---|---|---|
| Arranque del proceso | 13 MB | 13 MB |
| Modelo cargado | 1.009-1.107 MB | 1.009-1.073 MB |
| Tras soltarlo, vueltas 1 a 5 | 659, 716, 762, 745, 716 MB | 60, 60, 41, 41, 54 MB |

(VmRSS; RssAnon da lo mismo menos ~28 MB de páginas compartidas.)

- **La memoria se libera, pero glibc la retiene.** Sin el trim, el proceso queda en ~700 MB y no crece de una vuelta a otra: el modelo se destruye (no hay fuga) y las arenas de glibc se quedan con las páginas. Un `malloc_trim(0)` llamado desde afuera, en una corrida anterior, las devolvía (40-120 MB).
- **Con el trim cumple el criterio en las cinco vueltas.** Va en el hilo `fastembed-worker`, justo después de soltar el modelo (`crates/clawg-core/src/memory/embedder.rs:333`): ese hilo es el que destruye la sesión, así que el trim corre cuando ya no queda nada que devolver más tarde.
- **Los vectores son los mismos.** El vector de la misma consulta es idéntico en las cinco vueltas (`same vector as round 1: true`).
- **Costo de una recarga en este build debug:** leer y hashear los archivos, 9,9-12,1 s (30 s la primera vez, que descargó el modelo); armar la sesión, 6,4-9,4 s. La primera consulta después de cargar tarda 11-14 ms: no hay una penalidad de calentamiento que medir.
- **Windows no está medido.** Su asignador devuelve los bloques grandes por su cuenta (el trim es un no-op fuera de Linux con glibc), pero eso es lo que tiene que confirmar la verificación en la máquina de Marcos ([verify](verify.md)).

| Alternativa | Por qué no |
|---|---|
| C, el proceso hijo | El paso 0 muestra que la memoria vuelve en el mismo proceso; C es otro binario, un protocolo y el ciclo de vida del hijo para el mismo resultado |
| Llamar a `malloc_trim` desde el barrido | Al soltar el `Arc` el modelo se destruye después, en su propio hilo; el barrido trimaría antes de que haya algo que devolver |
| Un asignador propio (mimalloc, jemalloc) | Dependencia nueva, y la mayor parte de la memoria la pide ONNX Runtime, que no pasa por el asignador de Rust |

## D2. Un envoltorio en la celda (A2)

**Elegido:** `IdleEmbedder` (`crates/clawg-core/src/memory/idle_embedder.rs`) envuelve al modelo y es lo que se guarda en la celda. `dimensions`, `model_id` y los prefijos los copia al construirse; `embed_with_role` y `embed_batch` piden el modelo con un préstamo que lo carga si no está.

- **Una sola carga.** El modelo vive en un `tokio::sync::Mutex<Option<..>>` que se mantiene tomado durante la carga: quien llega mientras tanto espera esa misma carga.
- **Los usos se cuentan desde antes de la carga** (`InFlight`), así el barrido nunca suelta un modelo que alguien está por usar o usando. Al soltarse el contador, un uso cancelado también cuenta como uso.
- **La decisión es pura:** `should_unload(ahora, último uso, en curso, ocio)` (`idle_embedder.rs:53`). El barrido corre cada cuarto del tiempo de ocio, entre 1 s y 60 s, y termina cuando se suelta el envoltorio.
- **La residencia es un `AtomicBool`**, para que `/health` la lea sin esperar una carga que tiene tomado el mutex.

| Alternativa | Por qué no |
|---|---|
| A1, una celda reiniciable con un estado `idle` | Cambia el contrato de la celda, `hold_for_embedder` (`crates/clawg-core/src/memory/worker.rs:389`) y una veintena de tests, y cada consumidor tiene que decidir qué hace con `idle`. Lo único que compra es un estado más honesto, y eso lo da `embedder_resident` sin tocar nada |
| Un temporizador que suelta sin mirar el uso | Soltaría el modelo con un evento retenido o un indexado corriendo y obligaría a recargarlo enseguida |

## D3. La recarga y su identidad

**Elegido:** `bootstrap::unloading_when_idle` (`crates/clawg-core/src/memory/bootstrap.rs:344`) envuelve el `load` del arranque: cada embedder que construye sale envuelto, y la recarga vuelve a pasar por el mismo `ensure` (lectura y SHA-256 de los cinco archivos) y el mismo `load` en `spawn_blocking`. Loguea por separado la lectura con hash y la sesión, que es el registro de fases que pedía la exploración.

- **El contrato no se verifica otra vez.** Lo verificó el arranque contra el almacén; la recarga rechaza un modelo con otro `model_id` u otra dimensión (`same_model`), que es lo único que podría cambiar entre dos cargas de los mismos archivos.
- **Una recarga fallida es `NotReady`.** En la ingesta es `EmbedderUnavailable`, un fallo transitorio que no gasta un intento del outbox; en búsqueda y restauración es `503 embedder_not_ready` (`embed_failure`, `crates/clawg-cli/src/daemon/api.rs:860`), el mismo que antes de la primera carga.
- **Un apagado con saves en la cola puede recargar el modelo** para ingerirlos dentro del plazo de drenaje, como hoy espera a que cargue. Con el outbox (36) lo que no llega a ingerirse queda pendiente para el próximo arranque, así que no se pierde nada si el plazo vence durante la carga.
- **El SHA-256 se mantiene en cada recarga.** En este build debug es la mitad del costo de la recarga; en release no está medido. Saltearlo pierde la detección de un archivo alterado entre dos cargas, así que queda como decisión aparte, con el log de fases para medirlo.

| Alternativa | Por qué no |
|---|---|
| Guardar los bytes del onnx en RAM | 470 MB: anula el propósito |
| Un cierre de recarga propio de `serve` | Dos caminos de carga que pueden divergir; usar el mismo `ensure` y `load` del bootstrap lo evita |

## D4. Despertar y estado visible

- **`Embedder::wake` y `Embedder::resident`** tienen cuerpo por defecto (nada que hacer, siempre residente), porque es la verdad para todo embedder que no suelta su modelo; el comentario del trait contra los cuerpos por defecto habla de los prefijos, donde un default sería una afirmación falsa.
- **`POST /v1/projects` llama a `wake`** después de registrar (`api.rs:1188`). El puente registra su raíz en la primera herramienta de cada sesión, así que la carga empieza antes de la búsqueda que la necesita, aunque esa primera herramienta todavía espere una parte. El hook de inicio de sesión (25) puede usar la misma ruta antes.
- **`embedder_resident` en `/health`** sale de la celda (`api.rs:688`): `null` mientras está vacía. El cliente lo lee con `#[serde(default)]`.
- **La variable y no un flag.** El daemon lo lanza el puente, que no pasa flags de `serve`; una variable del entorno del editor sí llega.
