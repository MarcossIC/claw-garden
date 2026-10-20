# save-bounds — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | — (conviene antes de 44; ver «Interacciones») | exploración (borrador) |

**Tesis:** el costo de un save está acotado y se conoce antes de que clawg lo acepte: ningún save puede ocupar el único hilo de SQLite y el único trabajador de ingest durante minutos. Hoy el costo no tiene techo por dos razones que se suman. `content` no tiene tope (sólo se rechaza el vacío, y el único límite es el de 2 MB del extractor `Json` de axum). Y el progreso de un save largo se reescribe entero en cada pieza, así que los bytes escritos crecen con el cuadrado del número de piezas. Lo segundo hace que un tope generoso siga costando caro; lo primero hace que arreglar sólo lo segundo deje abierta una entrada de miles de piezas, cada una con su embedding y su KNN. Es de tamaño S–M: un tope y una prueba que cuenta bytes bastan casi seguro; la tabla por pieza (migración) sólo si la medición lo pide.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El progreso se reescribe entero por pieza | **Cierta.** `note_for` copia `noted` y agrega una entrada (`crates/clawg-core/src/memory/worker.rs:536-553`); `apply_note` serializa todo `progress` y corre `UPDATE ingest_outbox SET progress = …` (`crates/clawg-core/src/memory/outbox.rs:117-122`). Se llama por pieza desde `store_insert.rs:381` (duplicado absorbido) y `:458` (inserción), a través de `worker.rs:715` y `worker.rs:815` (`insert_unless_twin`, `store_insert.rs:429`). |
| `NotedPiece` lleva la cabecera completa en cada entrada | **Cierta, con un matiz.** Lleva `position`, `hash`, `id`, `inserted` y `context` (`outbox.rs:85-91`). `context` va vacío para la posición 0 y para un duplicado absorbido (`worker.rs:718-724`), y es la cabecera del save (hasta `SAVE_HEAD_TOKENS` = 48, `chunker/mod.rs:29`) en cada pieza insertada después de la primera (`worker.rs:683-686`, `:808-814`). El hash es el de la pieza (`content_hash`). Los bytes escritos suben como n²/2 entradas. |
| Cifras de la simulación: 500 piezas, 42 MB; 1 000, 169 MB; 2 000, 678 MB | **Vienen de una simulación en Python con SQLite 3.45, no del daemon.** El patrón (un `UPDATE` por pieza con el JSON completo, WAL, `synchronous = NORMAL`) coincide con el código, pero el tiempo (0,5 s, 1,9 s, 7,6 s) es de ese entorno. Se usan sólo como orden de magnitud; la medición propia es el primer paso (ver «Cómo se mide»). |
| Un cuerpo de 2 MB son unas 7 000 piezas y unos 8 GB de JSON | **Extrapolación, no medida.** Con el estimador de 4 caracteres por token que usa el repo, 2 MB son ~500 000 tokens; las piezas son de 96 tokens con 24 compartidos (`PIECE_MAX_TOKENS`, `PIECE_OVERLAP_TOKENS`, `chunker/mod.rs:39`, `:44`), paso de 72, y se cortan en cuanto el save no cabe en un chunk de 480 (`chunker/chat.rs:114-121`). 500 000 / 72 ≈ 7 000. Los 8 GB salen de extender la curva cuadrática. Además, `collapse_duplicate_chunks` (`worker.rs:680`) puede reducir el número de entradas si el texto se repite, y la pieza que llega a un duplicado ya guardado no lleva `context`: el peor caso es texto sin repeticiones. |
| `content` no tiene tope | **Cierta en el daemon.** Sólo `body.content.trim().is_empty()` (`crates/clawg-cli/src/daemon/api.rs:1015`). No hay `DefaultBodyLimit` en `daemon/` ni en `mcp/` (`rg` da cero), así que rige el defecto de 2 MB de axum para `Json`. **El puente tampoco lo acota:** `save_memory_impl` sólo comprueba que no esté vacío (`crates/clawg-cli/src/mcp/server.rs:700-702`) y reenvía `args.content.clone()` (`:712`). La documentación del campo (`:155-157`) no menciona tamaño. |
| El embedding de todas las piezas va en una sola llamada y hay un KNN por pieza | **Cierta.** Etapa 2: un `embed_batch` con todos los sobrevivientes (`worker.rs:758-760`); etapa 3: un `detect_contradiction` por pieza, en secuencia y antes de cada inserción (`worker.rs:797-811`). Cada uno es un `search_all_memories` con un pool de `CANDIDATE_POOL` (5) más un lugar por hermano (`crates/clawg-core/src/memory/contradiction.rs:58`, `:198-200`): un escaneo completo de `memory_vectors`, con un posible segundo escaneo de ventana 4× y la lectura de vínculos, en llamadas propias a la única conexión, seguido de la transacción de inserción de la pieza. Nada divide el lote: la memoria de los vectores y el tiempo hasta la primera inserción crecen con el save entero. El `embed_batch` real se parte o no por dentro en `embedder.rs:405`; no se leyó y queda a medir. |
| El puente responde `unconfirmed` a los 10 s | **Matiz: lo hace el daemon, no el puente.** El handler espera el reporte del trabajador `SAVE_CONFIRM_TIMEOUT` = 10 s (`api.rs:98`, `:1118-1121`) y responde `202 unconfirmed` (`api.rs:944`); el save sigue en el outbox. El cliente HTTP del puente no pone otro plazo (`client.rs:590`; el único `.timeout(` es el de `/health`, comprobado por un test en `:1459-1462`). Entonces el save largo no se pierde: se acepta, se responde a los 10 s y se sigue guardando. El costo es el tiempo del hilo, no la respuesta. |
| Piezas de 96 tokens con 24 compartidos pasados los 480 | **Cierta**, y el 480 es el `CHUNK_MAX_TOKENS` (`chunker/mod.rs:15`): un texto que entra en un chunk no se parte (`chat.rs:89-92`). Ojo: `.claude/CLAUDE.md` habla de «96 tokens, vecinos que comparten 24», consistente con esos valores. |
| El backlog de `openspec/README.md` registra «El daemon no pone tope a una conexión ni al tamaño de un save» | **Cierta** (`openspec/README.md:221`). Este cambio toma la mitad del tamaño del save. La mitad de conexiones (`axum::serve` sin `Timer`, `header_read_timeout`, tope de conexiones) se queda en el backlog: es otro mecanismo (un bucle sobre `hyper_util`), no comparte código ni prueba con el tope de `content`, y su motivo es la robustez del puerto, no el costo de un save aceptado. |
| El tope no debe rechazar lo que 44 piensa importar | **Medido por 44, y alcanza con holgura razonable.** La exploración de `history-import` midió 3 554 notas: mediana 2 030 caracteres, p90 5 749, p99 25 143, máximo 45 372 (`openspec/changes/history-import/exploration.md:18`). Con 4 caracteres por token, el máximo son ~11 300 tokens, es decir ~157 piezas. Un tope en bytes de un orden de magnitud sobre el máximo medido (ver «Decisiones abiertas») no rechaza ninguna. |

## Problema (verificado contra el código)

### 1. El progreso es cuadrático en el número de piezas

Cada pieza que se guarda escribe, en su propia transacción, el JSON de todas las anteriores más la suya (`outbox.rs:117-122`). Una entrada ocupa del orden de 300 bytes en una pieza insertada con cabecera (64 de hash, hasta ~200 de cabecera con 4 caracteres por token, más posición, id y banderas). Es una estimación del tamaño del JSON, no una medición. Con esa cifra, n piezas escriben ~n²/2 × 300 bytes: ~1,5 MB a 100 piezas, ~3,7 MB a 157 (el máximo de 44), ~150 MB a 1 000 y ~7 GB a 7 000, del mismo orden que la simulación.

Lo que cuesta es el hilo, no sólo el disco. Cada `UPDATE` ocurre dentro de `insert_unless_twin`, que usa la única conexión de escritura (`store_insert.rs:429`), así que búsquedas, tracker y mantenimiento esperan detrás. El WAL crece con cada versión de la fila y el checkpoint se alarga.

La reanudación también paga: `resume_positions` recorre todo el progreso y compara con `hashes` (`outbox.rs:97-107`), y `process_tracked` lo clona (`worker.rs:697`). Es O(n) una vez por reanudación, no el problema.

### 2. Un escaneo KNN por pieza

Un save de unos 5.000 tokens son unas 69 piezas (paso de 72 tokens: 96 con 24 compartidos), es decir, unos 69 escaneos completos del detector, más los reintentos. Una medición sintética fuera del repo, con el SQLite empaquetado (3.50.2) y sqlite-vec 0.1.9, dio del orden de 11 ms por escaneo a 10.000 filas y 54 ms a 50.000: entre 0,8 y 3,7 s de hilo de SQLite por ese save. Con el tope candidato de 64 KiB (~220 piezas) serían de 2,4 a 12 s, más que los 10 s de `SAVE_CONFIRM_TIMEOUT` (`crates/clawg-cli/src/daemon/api.rs:98`). No bloquea de una vez, porque cada pieza es una llamada aparte y entre ellas entran las demás, pero alarga la cola de la conexión y la confirmación del propio save. Las cifras sirven como orden de magnitud; el tiempo de hilo con un store sembrado es parte de la medición de este cambio.

### 3. El save no tiene tope

Con 2 MB de cuerpo entran miles de piezas, cada una con su embedding (un solo lote, `worker.rs:760`) y su KNN de contradicción (`worker.rs:804`) en el único trabajador de ingest. Mientras tanto la cola del canal (`INGEST_QUEUE_CAPACITY` = 1024, `runtime.rs:47`) se llena detrás y los saves cortos esperan. El handler responde `202 unconfirmed` a los 10 s, que es correcto pero esconde que el trabajador sigue ocupado.

Un 400 antes del outbox es el único lugar donde se puede decir que no sin perder lo aceptado: una vez en el outbox el save «sólo» puede fallar después de la respuesta (`api.rs:1028-1029`, mismo criterio que `topic_key`).

## Lo que ya existe y se reutiliza

- La forma de rechazar antes del outbox: `ApiError::bad_request` en `api.rs:1015-1037` (vacío, tipo, `topic_key`, `derived_from`). El tope es una línea más con el mismo estilo.
- `split_save` ya recalcula la cabecera desde el texto (`chunker/chat.rs:114-121`): un `progress` sin `context` puede reconstruirla de la misma forma al reanudar, siempre que el texto y el tokenizador no hayan cambiado, que es justo lo que `resume_positions` ya verifica con los hashes (`outbox.rs:97-107`).
- `resume_positions` ya trata un `progress` que no encaja como «otro troceo» y deja que el dedup tome el relevo (`worker.rs:699-702`): es la red si el formato nuevo no puede leer una fila vieja.
- El mecanismo de migraciones, la última es V17 (`crates/clawg-core/migrations/V17__revival.sql`).
- `protocol.md` ya habla de saves largos (`crates/clawg-cli/src/mcp/protocol.md:59-60`) y tiene su prueba de 2 048 caracteres para la apertura.

## Enfoques posibles

### A. Un tope de `content`, sólo eso

Un 400 antes del outbox, con el límite en el mensaje, y una línea en `protocol.md`. El tope puede ser en bytes (puro: `content.len()`, sin tokenizador) o en piezas (exige correr el chunker en el handler, que no lo tiene).

- **A favor:** cierra el peor caso entero con cambio mínimo y sin migración; el costo del progreso queda acotado por el tope, no por un arreglo estructural. Con un tope en bytes de 64 KiB (candidato, ver decisiones) el peor caso son ~16 000 tokens, ~220 piezas y ~7 MB de JSON totales según la estimación de arriba.
- **En contra:** el costo sigue siendo cuadrático; sólo se acota. Es un rechazo que hoy no existe: un agente que pegaba 100 KB ahora recibe un 400. Hay que decirle que parta el texto, y `protocol.md` ya dice «un hecho por llamada» (`:65`). Si cambia la descripción de alguna herramienta, `agent_contract.json` se bendice (`CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`).

### B. Un `progress` más delgado, sin cambio de esquema

Escribir la cabecera una vez (o recalcularla) y que cada entrada lleve sólo lo que no se reconstruye: `position`, `id`, `inserted` y, como el hash se obtiene del troceo del texto, tal vez ni siquiera el hash. Baja la entrada de ~300 a ~30 bytes; el total sigue siendo cuadrático pero 10 veces menor.

- **A favor:** sin migración; el JSON viejo se sigue leyendo si `NotedPiece` acepta `context` y `hash` opcionales (`#[serde(default)]`), que es lo que haría falta para una fila pendiente de la versión anterior. Ese es el requisito duro: una base con filas pendientes debe abrirse y terminar.
- **En contra:** sigue siendo n². Sin el hash, `resume_positions` pierde la comprobación de que el troceo no cambió (`outbox.rs:101`), que es la que evita mezclar piezas de otro troceo. Hay que conservar el hash o sustituirlo por otra comprobación (un hash del save entero en la fila). Reconstruir el `context` al reanudar depende de que el tokenizador dé la misma cabecera; si no, la fila reanudada quedaría con un `context` distinto del vector ya guardado, que es justo lo que el campo existe para impedir (`worker.rs:687-689`).

### C. Una tabla por pieza (migración V18)

`ingest_outbox_piece (row, position, memory_id, inserted, context)`, una fila por pieza escrita en la misma transacción que la inserción. O(1) por pieza; la reanudación lee las filas.

- **A favor:** la solución exacta: el costo total es lineal y no depende de un tope.
- **En contra:** es una migración, y por las reglas del proyecto lleva vuelta atrás y una prueba que siembra datos en V17. Además cambia el formato de `progress` y hay que leer el viejo para las filas pendientes. **Coordinación de numeración:** `secret-coverage` (51), opción I, también considera una V18 (guardar `operation_id` en `memory`). Si las dos se eligen, una toma V18 y la otra V19 según cuál se archive primero; no se asume nada acá. La tabla nueva además es otro lugar con texto que `purge` tendría que limpiar (el `context` es parte del texto del save), lo que se suma al alcance de 51.

### D. Embeber por lotes en vez de una llamada

Dividir el `embed_batch` de `worker.rs:760` en lotes de tamaño fijo.

- **A favor:** acota la memoria y permite que el drenaje y el cierre intervengan entre lotes.
- **En contra:** no cambia el costo total ni el del progreso. Con el tope de A el lote ya está acotado (~220 vectores de 384 floats no son memoria relevante). Depende de cómo trocee `embedder.rs:405` por dentro, que no se leyó. Sólo se justifica si la medición muestra que la espera previa a la primera inserción es el problema. **No recomendado ahora.**

### E. Las búsquedas del detector de todas las piezas en una sola llamada

Correr los KNN de todas las piezas dentro de una `call`, con una sentencia preparada que se reusa.

- **A favor:** quita los viajes de ida y vuelta al hilo de SQLite y la preparación repetida.
- **En contra:** no quita ningún escaneo, que es el costo; y una sola `call` larga deja a todo lo demás esperando detrás de ella de una vez, en lugar de intercalarse pieza por pieza. Lo que baja el costo de cada escaneo es knn-prefilter (37). **No recomendado** salvo que la medición muestre que los viajes pesan.

### Recomendación: medir, A y luego decidir B o C

1. **Medir primero.** Una prueba que guarda textos de 100, 1 000 y 2 000 piezas con el embedder falso y cuenta los bytes escritos a `progress`. Hoy tiene que dar una curva cuadrática y servir de prueba roja para lo que sigue.
2. **A (tope en bytes).** Barato y reversible, da el 400 antes del outbox y acota todo lo demás. Es lo que hace falta antes de 44.
3. **B si hace falta**, o **C** si no. Con un tope de decenas de KiB, B probablemente alcance: el peor caso baja a cientos de KB. C se justifica si se quiere un `progress` lineal para siempre, o si 53 (outbox-lifecycle) necesita leer el progreso por pieza para pausar o reintentar. La decisión se toma con la medición del paso 1 delante, no antes.

## Descartado

| Opción | Razón técnica |
|---|---|
| Truncar `content` en silencio | Pierde texto que el agente creyó guardado; el save se acepta con otro contenido. Un 400 es la única respuesta honesta. |
| Partir el save en varios en el handler | Cambia lo que es un save (un `operation_id`, un `topic_key`, un vínculo `memory_piece`) y rompe `topic_key` y el reemplazo entero (`specs/pieces` R10). |
| Tope sólo en el puente | Cualquier cliente del daemon con el token lo saltea; el daemon es quien tiene el contrato. Puede sumarse el mismo chequeo en el puente para dar el error sin ir a la red; no hace falta. |
| `DefaultBodyLimit` solo | Rechaza con un 413 genérico antes de llegar al handler y sin decir el límite en el formato de error del daemon; además un cuerpo JSON con escapes pesa más que su `content`. Sirve de segunda línea, no de reemplazo. |
| Comparar sólo la primera pieza y una de cada N contra el store | Las piezas vecinas comparten 24 tokens, así que podría alcanzar, pero cambia qué pares encuentra el detector: es un cambio del detector, que se mide con `conflict-eval`, no un tope del save. |
| Tope de conexiones y `header_read_timeout` | Otra mitad del backlog (`openspec/README.md:221`), distinta causa y mecanismo; se queda allí. |

## Interacciones

- **knn-prefilter (37).** Este cambio acota cuántos escaneos hace un save; aquel, cuánto cuesta cada uno.
- **outbox-lifecycle (53).** Comparte el bucle de piezas (`worker.rs:797-852`). Una pausa entre piezas, un corte del drenaje y una política de reintento descansan en el `progress`: si 53 cambia qué se anota o cuándo, este cambio y 53 tocan las mismas líneas. Conviene que este cambio fije el formato del progreso primero, o que ambos se diseñen juntos antes de escribir código.
- **secret-coverage (51).** Posible V18 en ambos (ver C). Además, si este cambio agrega una tabla con `context`, 51 tiene que hacer que `purge` la limpie. Si se elige B (sin tabla), no hay interacción.
- **history-import (44).** Lo importa por el camino normal de `save_memory`. El máximo medido (45 372 caracteres) cabe en cualquier tope razonable, pero 44 debe decir qué hace con una nota que lo supere (partirla en varios saves a propósito, o rechazarla en el `--dry-run`). Este cambio va antes de 44 para que la importación de miles de notas no sea la primera carga real del progreso cuadrático: 1 931 de las 3 554 notas son saves de más de una pieza.
- **ingest-outbox (36, archivado).** El formato de `progress` y su reanudación están en `openspec/specs/ingest/spec.md` y `openspec/specs/storage/spec.md` («Apagado sin pérdida»); un cambio de formato los modifica con un delta. La regla «lo aceptado no se pierde» no cambia: un 400 no acepta nada.
- **Saves largos (15, 19).** `openspec/specs/pieces/spec.md` (R2, el troceo; R10, reemplazo entero con clave) debe seguir cumpliéndose para todo save bajo el tope. El tope no cambia el troceo.
- **memory-protocol (22).** `protocol.md` dice cuándo y cómo guardar; la línea del tope va en una sección posterior a la apertura, que está acotada a 2 048 caracteres (hoy 1 699 según `openspec/README.md`). Poner el tope en la apertura exigiría medir el margen.

## Alcance candidato

1. Prueba de medición de los bytes escritos a `progress` para N piezas (la curva de hoy) y de los escaneos del detector que corre un save de N piezas.
2. Tope de `content` en bytes en `save_memory` del daemon (`api.rs:1015`), 400 antes del outbox que nombra el límite y escribe nada; la misma comprobación se refleja en el mensaje que el puente muestra.
3. Una línea en `protocol.md` y, si cambia una descripción, `agent_contract.json` bendecido.
4. Según la medición: B (progreso delgado, lee el formato viejo) o dejarlo.
5. Spec: delta de `specs/ingest` (el tope y el 400) y, si se toca el formato, de `specs/storage`.

### No-alcance

- Tope y plazo de conexiones (queda en el backlog).
- Partir un save largo automáticamente.
- La tabla por pieza (C) y el lote de embedding (D), salvo que la medición lo pida.
- Una política de reintento o de pausa (53).
- Cualquier cambio de ranking: no se corre `rag-eval`.

## Decisiones abiertas

Son del usuario.

- [ ] **Unidad del tope.** ¿Bytes (recomendado: puro, sin tokenizador) o piezas?
- [ ] **Valor del tope.** Candidato: 64 KiB, cinco veces el máximo que midió 44 (45 372 caracteres, que pueden ser más bytes si hay no-ASCII) y ~220 piezas. Confirmarlo mirando los tamaños de un store real, que requiere un perfil de datos y su autorización.
- [ ] **B, C o ninguno** después de medir.
- [ ] **Numeración de la migración** si se elige C, junto con 51.
- [ ] **Qué hace 44 con una nota sobre el tope.**
- [ ] **El tope y los escaneos.** ¿El valor del tope se elige también por el tiempo de hilo que suman los KNN de sus piezas sobre un store grande, o sólo por los bytes del progreso?
- [ ] **Segunda línea.** ¿`DefaultBodyLimit` además del tope, para acotar el cuerpo antes de deserializarlo?

## Cómo se mide

Todo con el embedder falso (`CLAWG_TEST_EMBEDDER=fake` en los tests de binario; el que usen los tests de `worker.rs` en los de núcleo), sin red ni perfil.

1. **Bytes escritos a `progress`**: guardar textos de N = 100, 1 000 y 2 000 piezas y sumar `length(progress)` de cada `UPDATE` (o el crecimiento del WAL). Hoy debe dar una curva cuadrática; después de B, lineal en bytes por pieza, o de C, O(1). Es la prueba que falla hoy y no depende de una máquina.
2. **El 400**: un cuerpo sobre el tope recibe `400` antes del outbox, con el límite en el mensaje, y `ingest_outbox` queda sin filas nuevas. Uno justo en el tope se acepta.
3. **Compatibilidad**: una fila pendiente con el `progress` en el formato viejo (sembrada con el JSON de hoy) se reanuda y termina con el formato nuevo (si se elige B o C). Si hay migración, además la prueba de sembrar en V17.
4. **Latencia de un save largo antes y después.** Requiere el modelo real (la inferencia domina) y un perfil de datos dev, con autorización; las cifras de la simulación (0,5 a 7,6 s sólo de escrituras) no sustituyen esta medición.
5. **Escaneos por save**: con el embedder falso sobre un store sembrado, cuántas búsquedas del detector corre un save de N piezas (las pasadas que `KnnOutcome` ya cuenta, `crates/clawg-core/src/memory/search.rs:408-425`) y cuánto tiempo de hilo suman. Hoy da una por pieza más los reintentos.
6. **Contrato del agente**: `the_agent_contract_matches_its_file` pasa tras bendecir, y la prueba de la apertura de `protocol.md` sigue verde.

## Riesgos

- **Un tope que rechaza lo que hoy se guardaba.** Un agente que pegaba un archivo ahora recibe un 400. Se mitiga con un mensaje que dice el límite y qué hacer, y con elegir el valor mirando tamaños reales.
- **B sin el hash pierde la protección contra otro troceo.** Si el tokenizador cambia entre versiones, un progreso sin hash puede atribuir una fila a la pieza equivocada. Se conserva el hash o se compara el hash del save entero.
- **Una migración en un cambio S.** C la trae con sus reglas (vuelta atrás, prueba en V17) y pelea la numeración con 51.
- **Las cifras vienen de una simulación y de un estimador de 4 caracteres por token.** El orden de magnitud basta para decidir que hay que medir, no para fijar el tope; por eso la medición es el primer paso.
- **53 y este cambio tocan el mismo bucle.** Un cambio en el orden de lo que se anota choca con el otro si se hacen en paralelo.
