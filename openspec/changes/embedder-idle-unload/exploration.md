# embedder-idle-unload — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (el daemon es de todos los objetivos) | M | — (después del arreglo del arena de ORT, que fija el piso de partida; se coordina con 36 y 25; ver «Interacciones») | exploración (borrador) |

**Tesis:** el daemon es un proceso sin interfaz que corre todo el día, y durante casi todo ese tiempo nadie le pide un embedding. Aun así mantiene el modelo (`multilingual-e5-small`, fp32, `model.onnx` de 470 MB) cargado durante toda su vida, y ese modelo cargado es el piso de memoria: cerca de 1 GB de bytes privados, con el arreglo del arena ya aplicado. Cualquier cosa que se haga con el arena, el lote o el troceo no baja de ahí, porque el piso **es** el modelo. Entonces la palanca que falta es soltar el modelo cuando el daemon está ocioso y volver a cargarlo cuando alguien lo necesita, de modo que un daemon ocioso quede en decenas de MB en lugar de ~1 GB. El costo es real y está medido en el log: una carga tarda entre 9 y 16 s en el perfil dev, así que la primera petición después de un rato ocioso la paga, salvo que el daemon se entere antes de que va a hacer falta.

Todo lo que sigue es exploración: no se escribió código.

> Convención de este documento: las líneas de `embedder.rs` son las que tenía el archivo al leerlo; otra persona lo está editando ahora (tope de 16 textos por inferencia y arena de ORT apagado), así que pueden correrse unas pocas líneas. Las cifras de memoria del piso (1,0 GB ocioso, 1,6 GB de pico indexando, ~17 GB antes del arreglo) vienen de la medición del 2026-10-15 que dio origen a este cambio; **no las reprodujo esta exploración**.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| El modelo vive cargado toda la vida del daemon | **Cierta.** El bootstrap corre una vez en segundo plano (`crates/clawg-cli/src/daemon/serve.rs:87-98`), carga con `FastEmbedLocal::from_bytes` (`crates/clawg-core/src/memory/bootstrap.rs:238-242`) y guarda el resultado en un `OnceCell` con `embedder_cell.set(arc)` (`bootstrap.rs:382`). El comentario del campo lo dice a propósito: un `OnceCell` es de una sola escritura y el embedder no se puede cambiar en tiempo de ejecución (`crates/clawg-core/src/runtime.rs:84-92`). Nada lo suelta antes de que el proceso muera. |
| Quién es dueño del `TextEmbedding` | **Un hilo propio.** `from_bytes` carga el modelo en el hilo que llama (`embedder.rs:254-266`) y después lanza el hilo `fastembed-worker`, que se queda con el modelo para siempre; los llamadores le mandan `EmbedRequest` por un `mpsc` sin tope (`embedder.rs:183-186`, `268-282`). El hilo sale solo cuando se sueltan todos los emisores (`embedder.rs:271-272`). **Consecuencia útil: soltar el modelo es soltar el `FastEmbedLocal`.** No hace falta una API de descarga: al caer el último `Arc`, cae el `req_tx`, el bucle termina y el `TextEmbedding` se destruye en ese hilo. |
| fastembed no retiene los bytes del onnx | **Cierta.** `try_new_from_user_defined` los pasa por referencia a `commit_from_memory(&model.onnx_file)` (`fastembed-5.13.4/src/text_embedding/impl.rs:159`), que llama a `CreateSessionFromArray` de ORT (`ort-2.0.0-rc.12/src/session/builder/impl_commit.rs:93-112`); el `Vec<u8>` cae al volver `from_bytes`. La sesión se arma con optimización de grafo en nivel 3 y un hilo intra-op por núcleo (`impl.rs:135-141`). El piso de ~1 GB es entonces la sesión (pesos más las copias que genera la optimización), no los bytes. |
| Los consumidores del embedder | **Son seis caminos, no uno** (tabla siguiente). Ninguno guarda el `Arc<dyn Embedder>` más allá de la operación en curso: lo toman del `OnceCell` con `get().cloned()` al empezar y lo sueltan al terminar. |
| `EmbedError::NotReady` es lo que pasa hoy mientras el modelo carga | **Falsa.** Ningún código de producción construye `NotReady`: sólo lo devuelven dobles de prueba (`crates/clawg-core/src/memory/admin.rs:2943`, `worker.rs:789`). `FastEmbedLocal` sólo produce `WorkerGone` o `InferenceFailed` (`embedder.rs:203-214`). Lo que ve cada consumidor mientras el modelo no está es **una celda vacía**, no un error del embedder. La rama `Err(EmbedError::NotReady) \| Err(EmbedError::WorkerGone)` del worker (`worker.rs:512`) existe, pero sólo se alcanza por `WorkerGone`. |
| El estado del embedder se publica en `/health` y `memory_status` | **Cierta, con tres valores.** `EmbedderStatus` es `Loading`, `Ready` o `Failed { error, retryable }` (`bootstrap.rs:36-41`). `/health` lo traduce a la cadena `loading`/`ready`/`failed` (`crates/clawg-cli/src/daemon/api.rs:629-634`), el cliente la guarda tal cual en `Health.embedder: String` (`crates/clawg-cli/src/client.rs:115-130`) y `memory_status` imprime el `Health` entero (`crates/clawg-cli/src/mcp/tools.rs:325-349`). El único consumidor de la cadena es `embedder_ready()`, que compara con `"ready"` (`client.rs:135-137`) y sólo lo usan tests. Un valor nuevo es aditivo para el puente. |
| El mantenimiento periódico mantiene el modelo caliente | **Falsa, y es buena noticia.** `maintenance_worker_future` recibe sólo el `store` (`runtime.rs:372-376`); corre a los 5 min de arrancar y cada 24 h (`maintenance.rs:24-27`, `301-311`) y hace degradación de niveles, barrido de vectores huérfanos, disputas huérfanas y un log de conteos (`maintenance.rs:91-131`). No tiene el embedder ni lo pide. |
| El re-embebido (`maintenance.rs`) despierta el modelo en operación normal | **Falsa.** `reembed_all` se llama desde `verify_or_reembed` (`bootstrap.rs:296`), que corre **dentro del bootstrap**, es decir con el modelo recién cargado y sólo si el almacén venía con pooling CLS. En operación normal nadie lo llama. |
| Una carga tarda unos 9 s | **Parcial.** Es la mediana de una distribución más ancha. Ver «Costo de una recarga». |
| El indexado de docs de un proyecto necesita el modelo cada vez que se registra | **Parcial.** `POST /v1/projects` lanza `spawn_doc_refresh` (`api.rs:1079`, `1087-1108`), que espera a que el estado deje de ser `Loading`, toma el embedder de la celda y llama a `try_refresh`. Pero `refresh` sólo embebe los documentos cuyo hash cambió o cuyo modelo cambió (`crates/clawg-core/src/memory/project_docs/mod.rs:358-389`, con `plan_refresh`); si nada cambió, no llama al modelo. Con descarga por ocio, tomar el embedder **antes** de saber si hace falta es lo que habría que evitar. |

## Quién usa el embedder y qué pasa hoy si no está

| Consumidor | Dónde toma el modelo | Con la celda vacía (cargando) | Con `Failed` |
|---|---|---|---|
| **`search_memory`** (`POST /v1/search`) | `ready_embedder` y después `embed_with_role(query, Query)` (`api.rs:775-782`, `791`, `795-798`) | Responde `503 embedder_not_ready` (`api.rs:778-780`). El puente lo traduce a «la memoria sigue cargando el modelo; guardar funciona y buscar funcionará en un minuto» (`crates/clawg-cli/src/mcp/server.rs:423-427`). | `503 embedder_failed` con la causa y si reintenta (`api.rs:777`). |
| **`recall_context`** (`POST /v1/context`) | `app.runtime.embedder.get().cloned()` (`api.rs:868`), pasado como `Option` a `build_memory_prompt` | **Degrada en silencio:** responde éxito con el workspace y sin recuperación, y deja un `log::warn!` («embedder not ready; retrieval skipped this turn», `crates/clawg-core/src/memory/prompt_builder.rs:134-175`). El log del dev lo muestra dos veces (2026-10-05 11:17:41 y 2026-10-11 01:15:40). Es el hallazgo que ya recoge context-budget (23). | Igual que con la celda vacía: la celda está vacía. |
| **`save_memory`** (`POST /v1/memories`) | No lo toma el handler: lo toma el worker de ingesta (`worker.rs:458`) | El handler mira el **estado**, no la celda: con `Loading` no espera y responde `202 embedder_loading` (`api.rs:726-732`, `764-766`). El evento queda en la cola en RAM y el worker lo retiene (`hold_for_embedder`, sondeando la celda cada 100 ms: `worker.rs:92`, `328-347`). | Se rechaza antes de encolar (`api.rs:727-729`). |
| **Ingesta (worker)**: un lote por evento | `embedder_cell.get().cloned()` (`worker.rs:458`) y `embed_batch(.., Passage)` (`worker.rs:510`); la sonda de contradicciones **reutiliza esos mismos vectores**, sin una segunda inferencia (`worker.rs:495-500`) | Si llegara a ejecutarse sin celda, descarta el evento con `EmbedderUnavailable` (`worker.rs:458-466`); en la práctica `hold_for_embedder` lo evita. | — |
| **Indexado de docs** (`spawn_doc_refresh` al registrar el proyecto) | Celda, después de `wait_for(!Loading)` (`api.rs:1089-1093`); `embed_batch(.., Passage)` por documento (`project_docs/mod.rs:384`, `396-407`) | Espera el cambio de estado y sigue. Si terminó en `Failed`, loguea y no indexa (`api.rs:1093-1096`). | Idem. |
| **`search_project`** (`POST /v1/projects/docs/search`) | `ready_embedder`, y vuelve a refrescar el índice si pasaron más de 60 s (`api.rs:1164-1180`; `REFRESH_INTERVAL`, `project_docs/mod.rs:32`) | `503 embedder_not_ready`. | `503 embedder_failed`. |
| **`restore_memory`** (`POST /v1/memories/memory/{id}/restore`) | `ready_embedder` (`api.rs:1001`), porque la fila restaurada se embebe otra vez | `503` como la búsqueda. | `503 embedder_failed`. |
| **Verificación del contrato y re-embebido de arranque** | El `Arc` recién cargado (`bootstrap.rs:378`, `269-307`) | Corre dentro del bootstrap; mientras dura, el estado es `Loading`. | — |
| `synthesize` (`POST /v1/synthesis`), `list`, `read`, `update`, `delete`, `resolve_conflict` | **No usan el modelo** (`synthesis_core` y el resto sólo tocan el `store`: `api.rs:889-896`) | Funcionan. | Funcionan. |

El patrón: **buscar** y **restaurar** ya saben esperar o fallar con un 503 que el puente traduce a un consejo; **guardar** ya sabe aceptar sin modelo; el **contexto** es el único que degrada sin decirlo. Un modelo que desaparece por ocio vuelve a caer en esas tres conductas, pero con una frecuencia mucho mayor que la única vez por arranque de hoy.

## Costo de una recarga

### Qué hace el arranque del modelo (código)

Una carga, en orden:

1. **`ensure_model` → `try_load_cached`** (`crates/clawg-core/src/memory/model_download.rs:298-304`, `447-470`). Para cada uno de los cinco archivos hace `exists`, `std::fs::read` completo y **SHA-256 del contenido entero** contra la constante fijada. Es la lectura de 470 MB del onnx más 17 MB del tokenizer, y un hash de ~490 MB. No emite eventos en este camino (`model_download.rs:380-386`).
2. **`FastEmbedLocal::from_bytes` en `spawn_blocking`** (`bootstrap.rs:366-367`): arma la sesión de ORT con `CreateSessionFromArray`, optimización nivel 3 e hilos por núcleo (`fastembed impl.rs:135-159`), y el tokenizer. Los bytes están en memoria **a la vez** que la sesión, así que durante la carga el proceso tiene el `Vec` de 470 MB más la sesión en construcción.
3. **`verify_or_reembed`** (`bootstrap.rs:378`, `269-283`): una consulta al almacén para comparar el contrato. Con el contrato al día no hace nada más (`ContractStatus::Match`).
4. **No hay precalentamiento.** El comentario de `from_bytes` habla de «~100 ms warm-up» (`embedder.rs:244`), pero ninguna línea de ese camino ejecuta una inferencia: la primera petición real es la primera pasada por la red. No medí cuánto cuesta esa primera pasada; ORT suele diferir parte de su inicialización (empaquetado de pesos, memoria de trabajo) a la primera ejecución, así que la primera petición después de una recarga podría ser más lenta que las siguientes. **Sin verificar.**

### Qué dice el log (`~/.clawg-dev/logs/serve.log`, sólo lectura)

El intervalo entre «listening» y «embedder bootstrap: done» de cada arranque completo del log (25 arranques; se excluyen dos cargas canceladas por un `stop` a mitad de carga, 2026-09-30 00:18:38 y 09:05:42):

| Estadística | Valor |
|---|---|
| Mediana | 11 s |
| Rango sin los dos mínimos | 9 a 16 s |
| Último arranque (2026-10-15 11:25:18 → 11:25:27) | 9 s |
| Dos arranques más rápidos | 2 s y 3 s (2026-09-30 08:59:57 y 09:01:25) |
| Valores repetidos | 13 s seis veces; 10 s cinco veces; 11 s cuatro veces; 9 s tres veces |

Lo que **no** se puede leer del log: el reparto entre lectura, hash, creación de la sesión y verificación. El log sólo marca el inicio y el fin (`bootstrap.rs` no loguea fases). Lo que sí se puede razonar:

- El perfil dev es un build **debug** (regla del perfil en `.claude/CLAUDE.md`). En debug, el `sha2` del crate corre sin optimizar y hashear ~490 MB es plausiblemente varios segundos; la sesión de ORT, en cambio, es código nativo ya compilado y no depende del perfil de Rust. **Hipótesis, no medida:** en el perfil dev una parte grande de los ~11 s es el SHA-256, y un build release lo baja a una fracción.
- Los dos arranques de 2-3 s son anteriores al resto (versión 8 de migraciones en el log) y no se sabe con qué binario corrieron. Pueden ser un build optimizado, o un caso de caché de archivos. **Con el log no se puede distinguir.** Sirven sólo como indicio de que 2-3 s es alcanzable en algún build.
- El archivo está en la caché de archivos del sistema operativo en la mayoría de estos arranques (se repiten reinicios en minutos). Una recarga después de horas de ocio puede tener que leer del disco. Esa cola no está en el log.

### Qué pagaría la primera petición después de un rato ocioso

- Con la carga actual en el perfil dev: **del orden de 10 s** (9 a 16 s), más la primera inferencia.
- En release: **sin medir**. Los dos arranques de 2-3 s son lo único que lo acota, y no se sabe a qué corresponden.
- Un pico transitorio de memoria por cada recarga: bytes del onnx más la sesión en construcción. No está medido; si la sesión pesa ~1 GB y los bytes 470 MB, el pico de la recarga puede acercarse al de indexar. Cada despertar vuelve a pagarlo.

### Qué se puede conservar en RAM para abaratar la recarga

| Qué conservar | Costo en RAM | Qué ahorra | Veredicto |
|---|---|---|---|
| Los bytes del onnx | **470 MB** | La lectura del archivo, no el hash ni la sesión | **Descartado:** anula el propósito. |
| La sesión de ORT en sí (no soltarla) | ~1 GB | Todo | Es el estado actual. |
| El tokenizer ya parseado | Decenas de MB (sin medir) | Parsear 17 MB de JSON | Chico, pero el worker ya guarda un `Tokenizer` aparte en `runtime.tokenizer` (`worker.rs:601-611`, `model_download.rs:133-142`) que **no** se suelta; el de fastembed es otra copia y se recrea con el modelo. |
| Nada, apoyarse en la caché de archivos del sistema operativo | 0 bytes privados (la caché de archivos no cuenta como memoria privada del proceso; sí aparece como memoria en espera del sistema) | La lectura del disco en recargas cercanas | **Recomendado como punto de partida:** es gratis y no es del proceso. |

Lo que sí es barato de ahorrar sin RAM: **no volver a calcular el SHA-256 en cada recarga.** La verificación completa se hace al arrancar el daemon (donde hoy está); en una recarga alcanza con comprobar que tamaño y fecha de modificación no cambiaron desde esa verificación. Sin hash no hay lectura completa dos veces. Es una decisión del usuario porque cambia una garantía (un archivo alterado en disco entre dos cargas no se detectaría hasta el próximo arranque). Si el SHA-256 es la parte dominante del costo en el perfil dev, esto solo recorta mucho; lo dirá la medición.

Fuera del alcance de una opción simple, pero para dejarlo anotado: ORT puede guardar el grafo ya optimizado en un archivo y cargar eso la próxima vez (`optimized_model_filepath`), y puede cargar un modelo desde la ruta en lugar de desde bytes. fastembed 5 sólo expone `from_bytes` para modelos definidos por el usuario, así que usarlo implica saltearse fastembed y armar la sesión con `ort` directamente. Es una dependencia con más superficie y no se recomienda sin que la medición lo pida.

## Opciones

La pregunta que ordena todo: ¿cuánto vale bajar de ~1 GB a decenas de MB, contra un despertar de ~10 s?

### A. Descartar el modelo tras N minutos sin uso y recargarlo en la próxima petición

Sub-forma importante, porque cambia cuánto código se toca:

- **A1. Reemplazar el `OnceCell` por una celda reiniciable.** El estado pasa a ser explícito (`Idle`, `Loading`, `Ready`). Cada consumidor decide qué hace con `Idle`. **Toca** `runtime.embedder` (campo público que usan `tests/api.rs`, `tests/shutdown.rs`, `tests/public_api.rs`, `eval_api.rs:478` y unas veinte pruebas de `worker.rs` y `bootstrap.rs`) y `hold_for_embedder`, que sondea esa celda (`worker.rs:328-347`).
- **A2. Un decorador dentro de la celda.** El `OnceCell` conserva para siempre un `Arc<dyn Embedder>` que es un envoltorio: `dimensions()` y `model_id()` son constantes (`embedder.rs:296-298`) y no necesitan el modelo; `embed_batch` toma el modelo interno, y si no está lo recarga (una sola recarga a la vez) y lo marca como usado. Un temporizador suelta el modelo interno tras N minutos sin uso. **Ningún consumidor cambia**, `hold_for_embedder` ve la celda siempre llena, `EmbedderStatus` sigue en `Ready`. La carga la hace un cierre que el envoltorio recibe (el mismo `load` que ya inyecta `production_deps`, `bootstrap.rs:238-242`), así que se prueba con embedders falsos y sin el modelo real.

| | A1 (celda reiniciable) | A2 (decorador) |
|---|---|---|
| RAM ociosa | Decenas de MB si el sistema devuelve la memoria (ver «Riesgos») | Igual |
| Latencia de la primera petición | Cada consumidor decide: esperar con tope o dar 503 | Todos esperan **dentro** de `embed_batch`, sin tope propio |
| Complejidad | Alta: cambia el contrato de la celda | Media: un tipo nuevo y un temporizador |
| Qué ve el agente | Un estado honesto (`idle`) | Un `ready` que a veces tarda 10 s: no distingue «tarda» de «colgado» |
| Contrato de estado | Un valor nuevo (`idle`) en `/health` | Se puede dejar `ready` y sumar un campo aditivo (`embedder_resident`) |
| `recall_context` | Hay que decidir si despierta o degrada | Despierta solo: nadie degrada |
| Timeout de `save_memory` (10 s: `api.rs:90`) | Se decide el camino explícitamente | La recarga ocurre dentro del worker; si pasa de 10 s el guardado responde `202 unconfirmed` (`api.rs:767-770`) |

### B. Descartar, pero despertar de antemano cuando se sabe que va a hacer falta

Lo mismo que A, más una señal de «prepará el modelo» que se dispara **antes** de la primera consulta real:

- **Cuando el puente registra el proyecto.** `ClawgMcp` llama a `register_project` en la primera herramienta que usa el proceso del puente, y lo hace una vez por raíz (`mcp/server.rs:521-529`), es decir al empezar cada sesión; el daemon lanza el indexado de docs en segundo plano (`api.rs:1079`). Es el lugar natural para disparar la recarga sin que nadie espere.
- **Cuando llega un hook de inicio de sesión** (cambio 25): el hook corre antes que el puente, así que podría pedir el despertar sin esperar la respuesta.

Con B la primera herramienta de la sesión puede pagar menos de la recarga, porque ya empezó mientras el agente leía el prompt. No elimina la espera, la solapa. Requiere un punto de entrada para despertar (una ruta nueva en la API o el efecto lateral de una existente).

### C. El embedder en un proceso hijo que sale cuando está ocioso

El daemon lanza un proceso que carga el modelo y habla con él por un tubo; cuando está ocioso, el hijo termina.

- **A favor:** la memoria vuelve al sistema operativo **sin duda**, porque se va el proceso; no depende del asignador. Aísla un crash o un OOM del ONNX Runtime.
- **En contra:** es el más complejo. Otro binario o modo (`clawg embedder`), un protocolo, ciclo de vida del hijo (huérfanos si el padre muere, el mismo problema que el lock de `memory.db` resuelve para el almacén), copia de textos y vectores por el tubo, otra superficie para `clawg doctor` y para el instalador de Windows, que ya tiene que parar el daemon. Y el despertar cuesta lo mismo que en A (cargar el modelo), sumando el arranque del proceso.
- **Cuándo se justifica:** si la medición de A muestra que el sistema **no** devuelve la memoria al soltar el modelo. Es la red de seguridad de A, no una alternativa de primera elección.

### D. No hacer nada más allá del arreglo del arena

- **RAM ociosa:** ~1,0 GB, piso medido. **Latencia:** ninguna. **Complejidad:** cero.
- **En contra:** no cumple lo que pide el usuario («una aplicación sin UI en segundo plano no puede ocupar GBs»). Es la línea base contra la que se mide todo lo demás.

### E. Un modelo más chico (int8) como camino complementario

Un onnx cuantizado a int8 pesa del orden de un cuarto del fp32 (~120 MB contra 470 MB, **orden de magnitud, sin verificar para este modelo**) y la sesión ocupa menos. Baja el piso y también el costo de cada recarga. **Cambia los vectores:** es un contrato de embedding nuevo (otro `model_id`), lo que obliga al re-embebido completo del almacén con snapshot previo, y puede mover las métricas de recuperación. Eso es exactamente lo que cubre model-reembed (17), que hoy tiene urgencia baja y ningún cambio de modelo planeado (`openspec/changes/model-reembed/exploration.md`, «No hay ningún cambio de modelo planeado»). **No se diseña acá.** Se menciona porque combina bien con A: A hace que el costo se pague una vez por rato de uso; E hace que el costo sea menor cada vez. Tampoco conviene arrancar por E: es el cambio más caro y el único que altera la calidad de recuperación, y A se puede hacer con el modelo actual y sin tocar los datos.

### Resumen comparativo

| | RAM ociosa | Primera petición después del ocio | Complejidad | 36 ingest-outbox | 25 hooks | 17 model-reembed | 39 bridge-reconnect | Contrato de estado |
|---|---|---|---|---|---|---|---|---|
| **A1** | Decenas de MB (sin medir) | ~10 s dev; release sin medir | Alta | Un `202` mientras despierta es una promesa en RAM: 36 la cubre | El `SessionStart` ve `idle` y tiene que decidir | Sin efecto; el despertar usa el contrato vigente | Sin efecto | `idle` nuevo |
| **A2** | Decenas de MB (sin medir) | Igual, **invisible** en el estado | Media | Un guardado lento puede dar `202 unconfirmed`; 36 lo cubre | Despierta sin que el hook lo pida | Sin efecto | Sin efecto | Sin cambio (+ campo aditivo) |
| **B** (sobre A) | Igual | Solapada con el inicio de sesión: la primera herramienta paga una parte | Media-alta (A + punto de entrada) | Igual que A | Es justo lo que el hook podría disparar | Sin efecto | Sin efecto | Igual que A |
| **C** | Decenas de MB, **garantizado** | Igual que A + arranque del proceso | Muy alta | Igual | Igual | Más fácil de mantener el contrato en otro proceso | Sin efecto | Un estado más (`idle`) |
| **D** | ~1,0 GB | Ninguna | Cero | Sin efecto | Sin efecto | Sin efecto | Sin efecto | Sin cambio |
| **E** (complemento) | Menos que D, no medido | Menor que A solo | Baja en código, alta en datos | Sin efecto | Sin efecto | **Es 17** | Sin efecto | Sin cambio |

## Recomendación

1. **Primero, una medición de 30 minutos que decide si A sirve** (ver «Cómo se mide», paso 0): cargar y soltar el modelo varias veces en un proceso de prueba y mirar los bytes privados desde afuera. Si la memoria no vuelve (el asignador la retiene, como ya pasó con los ~17 GB retenidos antes del arreglo del arena), A no baja el piso y la opción real es C. Toda la tesis depende de este número y hoy no existe.
2. **Si la memoria vuelve: A2 más una señal de despertar de B.** A2 porque toca un solo tipo nuevo y deja intactos la celda, `hold_for_embedder`, `EmbedderStatus::Ready` y los veinte tests que dependen de ellos; el costo de A1 (cambiar el contrato de la celda) no compra nada que A2 no compre salvo un estado más honesto, y ese estado se puede sumar de forma aditiva. La señal de B va en `POST /v1/projects`, que el puente ya llama al empezar cada sesión: dispara la recarga y sigue sin esperarla.
3. **Tiempo de ocio inicial: 10 minutos** (decisión abierta). Una sesión de trabajo hace consultas cada pocos minutos; 10 minutos evita recargar en medio de una tarea y suelta el modelo en los huecos largos.
4. **Nada de RAM extra para acelerar la recarga** (se descartan los bytes del onnx). Lo único que se ahorra es el SHA-256 en recargas, si el usuario lo acepta.
5. **E es un camino aparte:** entra por 17 si algún día se decide cambiar de modelo; este cambio no lo necesita ni lo bloquea.
6. **No hacer D como respuesta final:** deja un daemon de ~1 GB siempre vivo. D sólo es el punto de partida de la medición.

### RAM ociosa y latencia esperadas con lo recomendado

- **RAM ociosa:** *decenas de MB hasta unos 150 MB*, **estimación sin medir**: el almacén SQLite, sqlite-vec, el tokenizer que guarda el worker (17 MB de JSON en disco, tamaño en RAM sin medir), tree-sitter y el ORT cargado sin sesión. El piso real es lo que va a medir el paso 1.
- **Primera petición después del ocio:** ~10 s en el perfil dev (mediana 11 s, rango 9-16 s). En release, sin medir. Si el trabajo de la sesión empieza con una herramienta, la señal de B recorta lo que ve el agente. Las consultas siguientes, con el modelo cargado, cuestan lo de hoy.

## Interacciones

- **ingest-outbox (36).** Hoy un `202` (`embedder_loading` o `unconfirmed`) vive sólo en la cola en RAM (`api.rs:757-766`). Con descarga por ocio, un guardado que llega con el modelo suelto es un caso nuevo y **frecuente** de «aceptado y todavía no guardado» (con A2, el worker queda esperando la recarga dentro de `embed_batch`; el guardado pasa de 10 s con facilidad). 36 vuelve segura esa ventana. **Orden recomendado: 36 antes que este cambio**, o que este cambio no abra la ventana (ver la decisión abierta sobre qué hace un guardado en reposo).
- **agent-lifecycle-hooks (25).** El hook de `SessionStart` corre antes que el puente y tiene un plazo corto (la exploración de 25 propone 5 s en la configuración de ejemplo): no puede esperar una recarga de 10 s. Con el modelo suelto, el contexto de ese primer arranque sería el degradado de hoy (workspace sin recuperación, sin avisar) o tendría que decir `idle`. Es el consumidor que más gana de poder **pedir el despertar sin esperarlo** (B). Y si 25 y B se hacen juntos, 25 tiene que decidir si un `SessionStart` en reposo imprime «la memoria se está preparando» o nada.
- **model-reembed (17).** No cambia el contrato: el despertar vuelve a cargar los mismos archivos y el mismo `model_id`. Si 17 cambiara de modelo o de pooling, la recarga debería leer el contrato vigente (`verify_or_reembed`, `bootstrap.rs:269-307`) y **no** saltearlo, porque una recarga con otro modelo mezclaría espacios. En la práctica: el cierre de carga de A2 tiene que ser el mismo que usa el bootstrap, y la verificación del contrato se hace una sola vez por arranque del daemon. E (int8) es una entrada de 17.
- **bridge-reconnect (39).** No cambia: el daemon no se reinicia, y el token no tiene que ver. Sí conviene que el texto que el puente da ante `embedder_not_ready` (`server.rs:423-427`, «en un minuto») siga siendo cierto cuando el modelo se está despertando.
- **Contrato de estado.** `EmbedderStatus` tiene tres valores, `Loading`/`Ready`/`Failed`, y el handler de guardado decide con ellos (`api.rs:726-732`). Un valor nuevo hay que tratarlo en cada `match`: `health` (`api.rs:629-634`), `save_memory`, `ready_embedder` (`api.rs:775-782`) y `spawn_doc_refresh` (`api.rs:1093`). Con A2 el estado puede quedarse en `Ready` y la cadena de `/health` no cambia; si se quiere honestidad, un campo aditivo con `#[serde(default)]` en el cliente (como `embedder_error`, `client.rs:122-130`) no rompe un puente viejo contra un daemon nuevo.
- **install-smoke (40).** La prueba con el binario real es el lugar natural para medir bytes privados ociosos y la latencia de la recarga.
- **context-budget (23).** El aviso explícito cuando el embedder no está listo pasa a ser más frecuente con esta descarga; si 23 se hace antes, el caso `idle` ya tiene dónde decirse.

## Qué se descartó y por qué

| Opción | Razón técnica |
|---|---|
| Conservar los bytes del onnx en RAM para recargar rápido | 470 MB: anula el propósito. |
| Descargar con un temporizador que ignora el uso en curso | Soltar el último `Arc` con una inferencia en vuelo es seguro (el hilo termina cuando se sueltan los emisores), pero soltar el modelo mientras el worker retiene un evento o un indexado está corriendo obliga a recargar de inmediato. El temporizador mide tiempo **sin pedidos ni trabajo en curso**. |
| Descargar sólo el tokenizer del worker | Es una copia chica; el problema son los ~1 GB de la sesión. |
| Bajar la prioridad o el tamaño de lote para recortar el piso | Ya lo hace el arreglo del arena; el piso es el modelo cargado, no el lote. |
| Cuantizar de entrada | Cambia vectores y recuperación; es 17, no este cambio. |

## Alcance candidato

1. Un tipo que envuelve al embedder de producción, con estado `Resident`/`Idle`, recarga de una sola vez (varios pedidos simultáneos comparten una carga), registro de último uso y un cierre de carga inyectado. **Puro:** la decisión «¿toca soltar ya?» como función de `(ahora, último uso, trabajo en curso, tiempo de ocio)`.
2. El temporizador de ocio y su tiempo configurable (variable de entorno o parámetro de `serve`; decisión abierta).
3. La señal de despertar en `POST /v1/projects` (B), sin esperarla.
4. Que el refresco de docs no pida el modelo si nada cambió (hoy lo toma antes de saberlo: `api.rs:1093` contra `project_docs/mod.rs:358-389`).
5. Registro de fases de la carga en el log (lectura, hash, sesión, verificación), para medir.
6. Un campo aditivo en `/health` (`embedder_resident`) y su línea en `memory_status`, si se decide.
7. Docs: `.claude/CLAUDE.md` del repo (ciclo de vida del embedder, tiempo de ocio) y la spec de `storage`/`ingest` donde hoy se describe el estado `loading`.

### No-alcance

- Cambiar de modelo, cuantizar o tocar el contrato de embedding (17).
- Mover el embedder a otro proceso (C), salvo que el paso 0 lo exija.
- Cargar el modelo desde la ruta o guardar el grafo optimizado (requiere saltear fastembed).
- Cambiar el ranking: los vectores son los mismos. No aplica `rag-eval` como medida de calidad.
- Tocar `embedder.rs` mientras el arreglo del arena siga en curso.

## Decisiones abiertas

Son del usuario.

- [ ] **Tiempo de ocio.** ¿10 minutos (propuesto), otro valor, o configurable por entorno?
- [ ] **Qué hace un guardado con el modelo suelto.** (a) Despierta y espera con el tope de 10 s, y si vence responde `202 unconfirmed`; (b) responde `202 embedder_loading` sin esperar y despierta al modelo. Con (b) el contrato de hoy no cambia pero depende de 36 para ser seguro.
- [ ] **`recall_context` con el modelo suelto.** ¿Despierta y espera con un tope (a costa de una respuesta de 10 s en el primer prompt), o devuelve el workspace con un aviso explícito de que la recuperación llega cuando el modelo termine de cargar? (23 y 25 dependen de esto.)
- [ ] **Verificar el SHA-256 en cada recarga o sólo al arrancar.** Sólo al arrancar es mucho más rápido; pierde la detección de un archivo alterado entre dos cargas.
- [ ] **Si el paso 0 dice que la memoria no vuelve:** ¿se pasa a C (proceso hijo), se acepta el piso (D) o se avanza con E (int8, que entra por 17)?
- [ ] **Estado visible.** ¿Un valor nuevo `idle` en `/health` o un campo aditivo con `ready` intacto?
- [ ] **Orden respecto de 36.** ¿Este cambio espera a 36 o abre una ventana más de «aceptado y en RAM» mientras tanto?

## Cómo se mide

Todo en el perfil **dev**, con un build debug en un directorio de destino aparte (`cargo test --workspace --target-dir <otro>`), sin tocar `~/.clawg`. Un build release necesita autorización. Los resultados de dev son una cota alta de la latencia de carga.

### Paso 0: ¿la memoria vuelve al soltar el modelo?

Es la prueba que decide la opción.

1. En un proceso de prueba (un test ignorado en `clawg-core`, o un ejemplo descartable), cargar el modelo real con `FastEmbedLocal::from_bytes`, hacer una inferencia, soltar el `Arc` y repetir cinco veces, con una pausa de 10 s entre pasos.
2. Desde afuera, muestrear los bytes privados del proceso cada 2 s:

   ```powershell
   $p = Get-Process -Name <proceso>
   1..300 | % {
     $p.Refresh()
     "{0:o},{1},{2}" -f (Get-Date), $p.PrivateMemorySize64, $p.WorkingSet64 | Add-Content -Path <csv>
     Start-Sleep -Seconds 2
   }
   ```

   `PrivateMemorySize64` es la métrica (memoria comprometida privada); `WorkingSet64` se anota aparte porque el sistema puede recortarla sin que el proceso suelte nada.
3. Criterio a fijar **antes**: tras soltar el modelo, los bytes privados vuelven a menos de, por ejemplo, 300 MB y **no crecen** entre la primera y la quinta vuelta. Un crecimiento por vuelta es una retención del asignador y manda a C.

### Paso 1: piso ocioso con y sin el cambio

El daemon dev (`clawg serve --profile dev`), muestreado igual durante 30 minutos sin actividad, antes y después. Se reporta el mínimo, el máximo y el valor tras 2 × tiempo de ocio.

### Paso 2: latencia de la recarga

- Con el log de fases (alcance 5): lectura, hash, sesión y verificación por separado. Hoy el log sólo tiene inicio y fin.
- Latencia de la primera petición tras el ocio, con y sin la señal de despertar (B): `search_memory` y `recall_context` desde un cliente, con el reloj en el cliente. Diez repeticiones, con p50 y máximo.
- Latencia de las peticiones siguientes (modelo cargado), para comprobar que no cambió.

### Paso 3: el harness de evaluación

`rag-eval` no mide calidad acá (los vectores son los mismos), pero sirve para dos cosas:

- **Paridad:** el vector de un texto de prueba **antes** de soltar el modelo y **después** de recargarlo son iguales (o el coseno es 1,0 dentro de la tolerancia de punto flotante). Un test con el modelo real, marcado como ignorado como los demás que lo necesitan.
- **Latencia caliente:** `rag-eval --real --agent-view` ya reporta p50 y p99 por herramienta (`openspec/README.md`, fila del índice HNSW); sirve para confirmar que con el modelo cargado nada empeoró.

### Tests RED primero (con embedders falsos, sin el modelo real)

1. Un modelo que no se usó durante el tiempo de ocio se suelta; uno que se usó dentro del tiempo, no (función pura de decisión, con reloj inyectado).
2. Una petición con el modelo suelto lo recarga y devuelve el vector correcto; N peticiones simultáneas provocan **una** carga.
3. Con trabajo en curso (un evento retenido, un indexado corriendo, una inferencia en vuelo) no se suelta.
4. Una recarga que falla deja el envoltorio utilizable y la siguiente petición lo reintenta; no deja el estado en `Failed` para siempre.
5. `dimensions()` y `model_id()` no despiertan el modelo.
6. El refresco de docs sin cambios no despierta el modelo.
7. Un `POST /v1/projects` dispara la recarga y responde sin esperarla.
8. `/health` con el campo nuevo, y un cliente viejo que lo ignora.

## Riesgos

- **El asignador puede no devolver la memoria.** Ya ocurrió con los ~17 GB retenidos antes del arreglo del arena. Si soltar el `TextEmbedding` deja los bytes privados arriba, A no cumple la tesis. Se detecta con el paso 0, antes de escribir nada, y la salida es C.
- **Cada despertar vuelve a pagar un pico.** Bytes del onnx más la sesión en construcción pueden acercarse al pico de indexado. Si la recarga se dispara muchas veces por día en una máquina justa de memoria, el pico podría ser peor que mantener el piso.
- **Latencia invisible en A2.** El estado sigue en `ready` mientras una petición espera 10 s. Un agente o un hook con plazo corto lo ve como un cuelgue. Se mitiga con la señal de B y con un tope de espera propio.
- **Un guardado pasa de 10 s.** Responde `202 unconfirmed` (`api.rs:767-770`) con el texto «verificá con `list_memories`» (`server.rs:473-475`), que es una instrucción real para el agente. Más frecuente que hoy.
- **Contexto degradado y silencioso.** Si `recall_context` se queda con el comportamiento actual (`get().cloned()` → `None` → sin recuperación), el primer prompt de cada sesión después de un rato inactivo pierde la recuperación sin avisar. Es el riesgo más visible para el usuario y depende de una decisión abierta.
- **Despertares espurios.** Cualquier cosa que tome el embedder sin necesitarlo (el refresco de docs, hoy) rompe la ociosidad. Hay que revisar cada punto de la tabla.
- **El log de las pruebas dev no distingue las fases de la carga.** Hasta que exista el registro de fases, no se sabe cuánto de los ~11 s es el hash y cuánto la sesión; la conclusión de «ahorrar el SHA-256» es una hipótesis.
- **Un apagado ordenado mientras el modelo está suelto.** `drain_step` espera embedder listo para ingerir (`worker.rs:164-168`); con eventos retenidos el modelo no está ocioso, pero conviene que el apagado no recargue un modelo de 1 GB sólo para morir.
