# embedder-footprint — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M | — (se coordina con 54, 17, 20 y 61; ver «Interacciones») | exploración (borrador) |

**Tesis:** el daemon es un proceso de fondo y no debería quitarle todos los núcleos al IDE y al compilador del usuario mientras indexa o re-embebe, ni guardar memoria que no necesita. Hoy el embedder pide un hilo de cálculo por núcleo lógico, deja los hilos de ONNX Runtime con su espera activa por defecto, arranca sin saberlo un segundo grupo de hilos (el de `rayon`, por el tokenizer) que vive hasta que el proceso termina, rellena con padding hasta el texto más largo de cada tanda, y tokeniza tres veces un guardado largo dentro del worker asíncrono. Además hay tres preguntas de memoria que nadie midió. El cambio es de **medición primero**: nada que mueva vectores o sume complejidad se hace si la medición no muestra que importa. Una corrección barata sí va sin puerta: si `available_parallelism()` falla, hoy el modelo no carga nunca. Con la puerta, el resultado correcto puede ser cerrar con una sola línea de código y un registro de lo medido (enfoque F).

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La sesión usa un hilo intra-op por núcleo lógico | **Cierta.** `build_session` (`crates/clawg-core/src/memory/embedder.rs:500-513`) lee `available_parallelism()` (`:502`) y lo pasa a `with_intra_threads` (`:509`): incluye los hilos de SMT. La documentación de ONNX Runtime (https://onnxruntime.ai/docs/performance/tune-performance/threading.html) dice que el valor por defecto de los hilos intra-op es el número de núcleos **físicos**; acá se lo pisa. |
| Si `available_parallelism()` falla, la carga falla | **Cierta por construcción, no ejecutada.** El error sale como `LoadFailed` (`embedder.rs:502`) y cada carga vuelve a llamarlo; la biblioteca estándar documenta que puede fallar. No hay valor de reemplazo. Es un fallo de corrección, no de rendimiento. |
| ONNX Runtime deja girar a sus hilos antes de dormirlos | **Cierta según la documentación de la dependencia, no medida.** `ort-2.0.0-rc.12/src/session/builder/impl_config_keys.rs:132-136` dice que `with_intra_op_spinning` está **activado por defecto**; ningún lugar de `crates/` lo toca. Cuánta CPU gasta de verdad con una tanda corta, en este modelo, no está medido. |
| El comentario «otro número de hilos podría mover los últimos bits» (`embedder.rs:495-499`) es cierto | **Falsa según lo medido.** `openspec/changes/archive/2026-10-17-embedder-idle-unload/verify.md:174`: con `taskset` a 1, 2 y 4 núcleos los 32 vectores de `e5_golden` dan los mismos bits. **Matiz:** una máquina Linux de 4 hilos con AVX-512F; Windows y más núcleos no se midieron. `taskset` cambia `available_parallelism()` en Linux, así que midió lo que se quiere cambiar. |
| `e5_golden` ata los bits a la cantidad de hilos | **Falsa.** Compara bits sólo si la huella (sistema, arquitectura, AVX2, AVX-512F, FMA) coincide con la de la fixture (`crates/clawg-core/tests/e5_golden.rs:28-53`); la huella no incluye hilos. Fuera de esa huella la tolerancia es 1e-6 (`:26`). |
| El tokenizer arranca un segundo grupo de hilos | **Cierta.** `encode_batch` usa `into_maybe_par_iter` (`tokenizers-0.22.2/src/tokenizer/mod.rs:1278-1290`), que elige paralelo salvo que `TOKENIZERS_PARALLELISM` diga que no (`tokenizers-0.22.2/src/utils/parallelism.rs:40-58`, `:91-97`). `rayon` y `rayon-cond` son dependencias incondicionales (`tokenizers-0.22.2/Cargo.toml:206-210`) y el `Cargo.toml` raíz sólo apaga las features por defecto (`:52`). El grupo global de `rayon` toma un hilo por `available_parallelism()` (`rayon-core-1.13.0/src/lib.rs:463`) y no se cierra. Se llama desde `window_encodings` (`embedder.rs:223`). **Matiz:** un grupo ocioso duerme; el costo es la contención durante la tanda y los hilos y sus pilas mientras vive. Lo segundo no está medido, y que el grupo viva también con el modelo suelto es lo esperable pero se mide. |
| Apagar el paralelismo del tokenizer exige mutar el entorno (`unsafe` en edición 2024) | **Falsa.** La edición es 2021 (`Cargo.toml:10`) y además existe una función pública y segura, `tokenizers::utils::parallelism::set_parallelism(false)` (`parallelism.rs:60`), que guarda un atómico y gana sobre la variable (`:30-33`). No hace falta tocar el entorno. |
| El re-embeber agrupa en tandas | **Falsa.** `reembed_all` lee 200 filas por vuelta (`crates/clawg-core/src/memory/maintenance.rs:364`) pero infiere **una por una**: `embed_with_role(&row.text, ...)` (`:626`). Ahí no hay tanda que ordenar por largo; sí el refresco del índice de docs (`project_docs/mod.rs:407-409`, una sola llamada a `embed_batch` con los pasajes de un archivo). Ordenar por largo sólo toca ese camino y los guardados con varias piezas. |
| Las piezas de un guardado tienen largo parecido, los pasajes no | **Cierta con matiz.** Las piezas miden 96 tokens salvo la última (`chunker/mod.rs`, constantes que usa `split_save`); los pasajes de docs y código salen del árbol sintáctico y varían hasta los 512 tokens de `EMBED_MAX_TOKENS` (`embedder.rs:153`). El padding a la tanda (`pad_right`, `:235-255`) cuesta cálculo sobre ceros en el segundo caso, no en el primero. |
| `split_save` tokeniza tres veces | **Cierta.** Un `chunk_with` sobre todo el texto (`chunker/chat.rs:116`) y, si hay más de un pedazo, el `split` de la cabecera (`:120`) y el de las piezas (`:121`); cada uno llama `tok.encode(text, false)` entero (`chunker/token_splitter.rs:81`). Un texto de un pedazo codifica una vez. |
| Esa codificación corre en el worker asíncrono | **Cierta.** El worker llama `split_save` en línea (`worker.rs:761`, `:765`) y `worker.rs` no tiene ningún `spawn_blocking`. save-bounds (54) acota el peor caso por bytes, no quita la triple pasada. |
| Una sola arena de `malloc` está medida | **Cierta a medias.** Medido: memoria suelta en diez recargas (`verify.md:155-166`, +7,2 MB con `MALLOC_ARENA_MAX=1` contra +60,4 por defecto) y la inferencia, 19,2-20,1 s contra 19,5-19,8 s en `e5_golden` (`:166`). **No medido:** la latencia de búsqueda mientras un refresco de docs corre, ni 1 contra 2 arenas. `mallopt` (`embedder.rs:595-604`, llamado en `crates/clawg-cli/src/main.rs:54-57` sólo si `loads_the_model`) también pisa un `MALLOC_ARENA_MAX` del usuario. |
| El pico de memoria de la carga está medido | **Falsa.** 46 lo dejó explícito: «No está medido» (`archive/2026-10-17-embedder-idle-unload/exploration.md:77`). Lo medido es el estado estable con modelo (739-749 MB en Windows, 796-866 MB en Linux, `verify.md:106-117`, `:133-144`), no el máximo. |
| El patrón de memoria de ORT crece con formas distintas | **Sin verificar.** `with_memory_pattern` nunca se llama (`rg` sobre `crates/` sin resultados) y la dependencia aconseja apagarlo si la entrada varía (`ort-2.0.0-rc.12/src/session/builder/impl_options.rs:127`). Cada tanda tiene forma `(filas, largo)` distinta. Lo medido en 46 son diez recargas, no miles de inferencias de largo variable. |
| El refresco completo de este repo tardó 645 s | **Cierta, pero de otro cambio.** Está en `archive/2026-10-15-project-indexing/verify.md:188`: 226 docs y 4.984 pasajes en un binario debug. No es de docs-search-quality (20). Sirve de orden de magnitud para la puerta, no de dato de release. |

## Problema (verificado contra el código)

### 1. Cuánta CPU toma el embedder

Un solo hilo (`embedder-worker`, `embedder.rs:543-556`) corre la sesión y cada inferencia reparte su trabajo entre `available_parallelism()` hilos de ONNX Runtime. En una máquina de 16 hilos lógicos un refresco de docs o un re-embeber larga 16 hilos de cálculo, más hasta 16 del grupo de `rayon` que despierta el tokenizer en cada tanda. Con la espera activa por defecto, esos hilos siguen ocupando núcleos después de terminar una tanda. Para un servicio en segundo plano eso compite con lo que el usuario está haciendo. Ninguna de estas cifras es medida; son las que la puerta debe producir.

### 2. Un grupo de hilos que no se pidió

El tokenizer compartido (`Arc<Tokenizer>`, el mismo que usa el chunker) codifica las tandas con `rayon`. Para ≤ 16 textos (`EMBED_BATCH`, `embedder.rs:160`) el paralelismo gana poco: cada codificación es corta frente a la inferencia que viene. El grupo se crea en la primera tanda y no se destruye, tampoco cuando el modelo se suelta por inactividad (46): el cierre del embedder (`:554-555`) no lo alcanza.

### 3. Padding

`pad_right` rellena cada texto hasta el más largo de la tanda (`embedder.rs:235-255`) y la atención crece con filas × tokens² (comentario `:155-159`). Un archivo con pasajes de 30 y de 500 tokens en la misma tanda de 16 paga el máximo en todas las filas. Agrupar por largo baja ese costo; pero cambia qué texto comparte tanda con cuál y, con ello, el largo rectangular de cada llamada.

### 4. Memoria sin medir

(a) Si el patrón de memoria de ORT guarda un plan por forma de entrada, un daemon con `CLAWG_EMBEDDER_IDLE_MINUTES=0` (no suelta nunca el modelo) podría crecer con miles de formas. (b) La carga junta los bytes del onnx (~470 MB, cifra de 46) con la sesión en construcción; el pico de cada recarga es el máximo posible del proceso y nunca se leyó. (c) Una arena frente a dos y la latencia de búsqueda durante un refresco: falta el dato para saber si D6 de 46 tiene costo.

**(a) y (b) pasaron a [ram-footprint (90)](../archive/2026-10-24-ram-footprint/exploration.md)**, que suma la medición en prod del 2026-10-24 (el modelo cargado el 73 al 81 % de los minutos y un máximo de 1.278 MiB sin dueño) y las mediciones con las que se atribuye cada pico. 90 se archivó con esas mediciones, y el día de uso que las lee pasó a [ram-footprint-gates (92)](../ram-footprint-gates/exploration.md). Sus filas de la tabla de la puerta se miden allá. (c) queda acá.

## Lo que ya existe y se reutiliza

- `e5_golden` (`tests/e5_golden.rs`, 32 vectores, un lote de 17 en `:211-218`, textos de ~2.000 tokens y de 509 a 511): es la prueba de que los bits no se mueven; con `CLAWG_BLESS=1` reescribe la fixture (`:13-14`).
- `taskset` y el procedimiento de `verify.md:127` (`RssAnon` de `/proc/<pid>/status`, 8 lecturas cada 0,5 s) para medir en Linux; `PrivateMemorySize64` en Windows (`verify.md:104`).
- `CLAWG_EMBEDDER_IDLE_MINUTES` (`0` deja el modelo cargado) para aislar la pregunta (a).
- `rag-eval --diff` y `--fail-on-regression` (`.claude/CLAUDE.md`, «Testing») si algún vector se mueve.
- `set_parallelism` del tokenizer y `with_intra_op_spinning` / `with_memory_pattern` de `ort`: cada opción es una llamada, sin dependencias nuevas.

## La puerta: qué se mide antes de elegir

Necesita el modelo real (`CLAWG_E5_MODEL_DIR` para `e5_golden`; un perfil dev autorizado para el daemon), un build que se corra igual en las dos variantes, y el set dorado fuera de git en `docs/eval/` sólo si algún vector se mueve. Los números de 46 son de un Xeon de 4 hilos y de una máquina Windows; la puerta se corre en **Linux y Windows**, con al menos una máquina de 8 hilos lógicos o más, porque con 4 la diferencia lógico/físico casi no se ve.

| Medición | Cómo | Qué decide |
|---|---|---|
| CPU total y tiempo de pared de `reembed_all` y de un refresco de docs, por combinación: hilos = lógicos / físicos / mín(lógicos, 4); espera activa sí/no; `rayon` sí/no | Mismo store y mismos textos; CPU de proceso (usuario + sistema) y tiempo de pared; 5 corridas, mediana | Si «físicos» o «mín(lógicos, 4)» baja la CPU sin subir la pared más de un umbral, se elige B; si la espera activa explica la CPU residual, se elige A |
| Latencia de `POST /v1/search` mientras corre un refresco (p50 y p99) | Un cliente de búsquedas cada 100 ms durante el refresco, por combinación | Si el p99 mejora con menos hilos, se justifica B aunque la pared suba |
| CPU en reposo del daemon tras una tanda | Muestra de CPU de proceso durante 60 s después de la última inferencia | Si la espera activa deja CPU medible, A; si no, se descarta |
| Hilos vivos del proceso con el modelo suelto | Contar hilos antes y después de unos cuantos guardados, con y sin `set_parallelism(false)` | Si `rayon` deja N hilos con el modelo suelto, C |
| Tiempo de `split_save` y de la tanda, para un guardado del tamaño de 54 | Texto de 64 KiB y de 5.000 tokens; reloj en el worker | Si la triple pasada pasa de un umbral (decisión abierta), E |
| Vectores | `e5_golden` en cada combinación, y `rag-eval --diff` si algún vector cambia | Cualquier combinación que mueva bits queda fuera salvo decisión expresa |
| Memoria: máximo de `RssAnon` / privados durante la primera carga y una recarga (se mide en 90) | `/usr/bin/time -v clawg serve` (máximo residente) en Linux; `process.private_peak_bytes` (el máximo de la memoria privada desde que arrancó el proceso) en Windows | Fija el pico real de (b); si se acerca al pico de indexar, se anota y no se hace nada |
| Memoria: tendencia tras miles de búsquedas e inferencias de largos distintos con `IDLE_MINUTES=0` (se mide en 90) | 5.000 consultas de 1 a 512 tokens, lectura cada 500 | Si crece sin aplanarse, se prueba `with_memory_pattern(false)` y se vuelve a medir |
| Una arena contra dos | Misma prueba de búsqueda durante un refresco, `MALLOC_ARENA_MAX=1` y `=2` | Si el p99 de búsqueda difiere más que el umbral, se reabre D6 de 46 |

## Enfoques posibles

### A. `with_intra_op_spinning(false)` (recomendado si la CPU residual se ve)

Una llamada en `build_session`. **A favor:** no mueve el reparto del trabajo, así que se espera que no mueva bits (a comprobar con `e5_golden`); baja la CPU tras cada tanda. **En contra:** puede alargar la inferencia al despertar los hilos desde el sueño; la puerta lo cuantifica.

### B. Menos hilos intra-op: físicos, o `min(lógicos, N)`

Reemplazar `available_parallelism()` por un número acotado. **A favor:** deja núcleos libres al usuario; la medición de 46 sugiere que los bits no cambian. **En contra:** el cálculo físico no está en la biblioteca estándar y exigiría una dependencia (descartado sin uso real: ver «Descartado»); un tope fijo tarda más en máquinas grandes. Se vuelve decisión de producto: cuánta velocidad se cambia por cuánta CPU libre. Se agrega una variable de entorno sólo si la puerta lo pide.

### C. Tokenizar en serie (`set_parallelism(false)`) (recomendado si los hilos de `rayon` se ven)

Se llama una vez al empezar `serve`, antes de cualquier tanda. **A favor:** el grupo no se crea; sin `unsafe` ni entorno; el chunker ya llama a `encode` de a un texto, así que no cambia. **En contra:** el cambio es de proceso entero (todos los `encode_batch` de la dependencia); hoy el único llamador es `window_encodings`. Los bits no cambian: la codificación es la misma función por texto.

### D. Tandas ordenadas por largo

Ordenar los índices por cantidad de tokens, agrupar de a `EMBED_BATCH` y devolver los vectores en el orden original. **A favor:** menos padding en refrescos de docs. **En contra:** cambia el largo rectangular de cada llamada, y el comentario de `pool_and_normalize` (`embedder.rs:270-274`) sólo garantiza el orden de las operaciones, no que otra composición de tanda dé los mismos bits; el lote de 17 de `e5_golden` lo ejerce. Si los bits se mueven, hay que decidir entre reescribir la fixture o tolerancia. No sirve para el re-embeber (infiere de a una fila), así que lo gana 17 si ese cambio agrupa.

### E. Codificar una sola vez en `split_save`, y/o `spawn_blocking` sobre un tamaño

Codificar el texto una vez y sacar las tres cortes de los offsets. **A favor:** un tercio del trabajo y el worker no se bloquea. **En contra:** los bordes de pieza tienen que quedar byte por byte iguales: `resume_positions` y el dedup comparan hashes por posición (`outbox.rs:93-110`), y un corte distinto invalida el progreso de un guardado en vuelo. Con 54 el peor caso ya está acotado; E sólo va si la puerta lo mide.

### F. Sólo el paso 0 y el registro de lo medido (también recomendado como puerta)

Si ninguna medición pasa su umbral, se cierra con el arreglo de `available_parallelism` y el comentario de `build_session` corregido (los hilos no mueven bits, con la cita de 46).

### Recomendación: paso 0 directo, después puerta, después A/B/C/D/E según el resultado

0. **Sin puerta:** `available_parallelism()` que falla cae a un valor chico (1 o 2, decisión abierta) y se registra un aviso. Una prueba pura sobre una función que decide el número de hilos a partir de un `Result`.
1. Medir (tabla de arriba). Cada opción entra sólo con su umbral.
2. Orden de menor a mayor riesgo para los vectores: C, A, B, E (sin cambio de bits por construcción, con prueba), y D al final.

## Descartado

| Opción | Razón técnica |
|---|---|
| Cuantizar el modelo (int8) para gastar menos | Cambia el significado de los vectores guardados; es el cambio de contrato que este cambio excluye (aparte, 17). |
| Dependencia para contar núcleos físicos | «No agregues dependencias sin un uso real»; un tope por número lógico se mide primero. |
| `intra_op_num_threads` en 1 siempre | Alarga el refresco de docs y el re-embeber (645 s en debug para este repo) sin evidencia de que haga falta. |
| Un proceso hijo para el embedder | Resuelve memoria con otro costo; 46 lo descartó porque su paso 0 midió que la memoria vuelve sin salir del proceso (`archive/2026-10-17-embedder-idle-unload/proposal.md:23`). |
| Subir `EMBED_BATCH` | El comentario `:155-159` lo fijó en 16 por el pico de atención; nada aquí lo contradice. |
| Mutar `TOKENIZERS_PARALLELISM` desde el código | En la edición 2021 no es `unsafe`, pero `set_var` con otros hilos vivos es una carrera (y pasa a `unsafe` en la 2024); `set_parallelism` hace lo mismo sin tocar el entorno. |

## Interacciones

- **embedder-idle-unload (46, archivado).** Sus mediciones son la línea de base (`verify.md:119`, `:146-166`, `:174`). Cualquier cambio de hilos o de `malloc` vuelve a correr la recarga de diez vueltas en las dos plataformas.
- **save-bounds (54).** Acota los bytes de un guardado y por tanto el costo de E; conviene medir E con 54 entregado.
- **model-reembed (17).** Si agrupa el re-embeber en tandas, D se vuelve útil ahí también, y la migración de vectores ya pasa por `e5_golden`.
- **docs-search-quality (20).** Toca el índice de docs, el principal consumidor de tandas con largos distintos (`project_docs/mod.rs:407-409`).
- **read-connections (57).** Compite por CPU con el embedder: un lector extra más hilos de inferencia sin tope suma al mismo problema; la medición de latencia de búsqueda de acá sirve a las dos.
- **model-download (60) y embedder-recovery (61).** Hermanos nuevos. 61 toca el bucle del hilo del embedder y `build_session` está en el mismo archivo: los cambios chocan en texto, no en diseño; el paso 0 conviene hacerlo antes o junto con 61.
- **shutdown-budget (55).** Las tareas bloqueantes de carga también esperan al desmontar el runtime; menos hilos no cambia ese plazo.
- **Specs:** `openspec/specs/storage/spec.md` («Ciclo de vida del embedder») sólo si cambia algo observable; si no, ninguna.

## Alcance candidato

1. Paso 0: valor de reemplazo cuando `available_parallelism()` falla, con prueba pura y aviso.
2. La puerta: una guía de medición reproducible (qué necesita cada paso) y su registro.
3. Según el resultado: A, B, C, D y/o E, cada una con su prueba y su re-chequeo de `e5_golden`.
4. Corregir el comentario de `build_session` con la cita de 46.

### No-alcance

- Cambiar el modelo, el contrato de embedding o el significado de un vector guardado.
- Cuantización, GPU u otro proveedor de ejecución.
- Subir o bajar `EMBED_BATCH`.
- Hacer que el re-embeber agrupe (17).
- Recortar el modelo cargado o la política de descarga por inactividad (46).

## Decisiones abiertas

Son del usuario.

- [ ] **Valor de reemplazo del paso 0.** ¿1 o 2 hilos? Candidato: 2, porque 1 alarga todo y la falla es rara.
- [ ] **Umbrales de la puerta.** ¿Cuánta CPU de proceso justifica A o B (por ejemplo, 20 % menos de CPU total) y cuánta pared se paga por ella (por ejemplo, hasta 25 % más)? ¿Qué p99 de búsqueda durante un refresco es aceptable?
- [ ] **Bits.** Si D o B mueven algún bit: ¿se reescribe la fixture o se acepta la tolerancia de 1e-6? Por defecto, nada que mueva bits entra sin decisión.
- [ ] **Variable de entorno para hilos.** ¿Una variable `CLAWG_EMBEDDER_THREADS` o sólo la constante?
- [ ] **E y el tamaño.** ¿`spawn_blocking` sobre qué tamaño de texto?
- [ ] **`MALLOC_ARENA_MAX` del usuario.** ¿Se respeta si está definida (en vez de pisarla con `mallopt`) o se mantiene la garantía de 46?
- [ ] **Máquinas de medición.** ¿Qué Linux y Windows se usan, con cuántos hilos?

## Cómo se mide

1. **Paso 0 (RED).** Una función pura que, dado un `Result<NonZeroUsize, io::Error>`, devuelve el número de hilos; hoy `build_session` falla en el `Err`.
2. **Hilos.** Prueba que cuenta los hilos del proceso (en Linux, `/proc/self/task`) antes y después de embeber con el modelo falso y con `set_parallelism(false)`; sólo C la hace pasar.
3. **Bits.** `CLAWG_E5_MODEL_DIR=<dir> cargo test -p clawg-core --features eval --test e5_golden -- --ignored` por combinación; la huella de la máquina se anota con el resultado. `rag-eval --diff` y `--fail-on-regression` si algún vector se movió, leyendo `recall@5` y `search.found` de `.claude/CLAUDE.md`.
4. **Piezas.** Para E, una prueba que compara los cortes del `split_save` actual con el nuevo sobre un texto largo, y que `resume_positions` acepta el progreso de uno con el otro.
5. **Memoria y CPU.** Los pasos de la tabla de la puerta, con el procedimiento de `verify.md:127`.

## Riesgos

- **Que ninguna medición pase su umbral.** Es un resultado válido (F), no un fracaso.
- **Un cambio de hilos mueve bits en otra máquina.** Lo medido es un Xeon de 4 hilos; por eso la puerta corre en máquinas más grandes y en las dos plataformas. Los binarios de ONNX Runtime con OpenMP ignoran `with_intra_threads` (`impl_options.rs:45-51`): hay que mirar el recuento de hilos antes de creer una medición de B.
- **Espera activa apagada que alarga la inferencia corta** de una búsqueda (una consulta es una tanda de uno). La puerta mide p50 y p99 de la búsqueda, no sólo el refresco.
- **D invalida el progreso o la fixture** si cambia la composición de tanda.
- **E rompe la reanudación** de un guardado en vuelo si el corte difiere en un byte.
- **Los números de debug no son los de release.** Las cifras del repo (645 s de un refresco completo, unos 11 s de la primera petición después del ocio, `archive/2026-10-17-embedder-idle-unload/exploration.md:61`) son de binarios debug; cada medición dice qué perfil usó.
