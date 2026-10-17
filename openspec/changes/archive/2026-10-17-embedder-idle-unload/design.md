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
- **Windows no está medido.** Su asignador devuelve los bloques grandes por su cuenta (el trim es un no-op fuera de Linux con glibc), pero eso es lo que tiene que confirmar la verificación en Windows ([verify](verify.md)).
- *Nota posterior:* ya está medido ([verify](verify.md#resultado)) y no se cumple: en Windows la memoria crece con cada recarga. La causa y lo que reemplaza a esta decisión están en [D5](#d5-un-solo-tokenizer-por-proceso-la-inferencia-la-arma-clawg).

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

## D5. Un solo tokenizer por proceso: la inferencia la arma clawg

La verificación en Windows ([verify, «Resultado» y «Causa»](verify.md#resultado)) mostró que D1 no alcanza: con el modelo suelto la memoria privada sube 110 MB en cinco recargas y nunca baja. No hay objetos vivos de más: el heap de Windows se queda con lo que libera cada parse de `tokenizer.json`, ~1,2 millones de asignaciones chicas. El arnés lo separó: el tokenizer por sí solo crece (T), la sesión de ONNX Runtime sola queda plana en 19-25 MB (S), y el tokenizer parseado una vez con la sesión armada y soltada en cada vuelta queda plano (K), con el mismo vector que fastembed.

**Elegido:** un tokenizer por proceso, el que el daemon ya guarda en `MemoryRuntime::tokenizer` (`crates/clawg-core/src/runtime.rs:124`). fastembed no puede usarlo: `TextEmbedding` tiene la sesión `pub(crate)` y su único constructor desde bytes parsea el tokenizer (`fastembed-5.13.4/src/text_embedding/init.rs:127-134`, `impl.rs:162`); su `load_tokenizer` además lo clona entero (`common.rs:111`). Así que clawg arma la inferencia: la sesión con las opciones de hoy, el tokenizer compartido, el pooling por media sobre la máscara y la normalización L2. fastembed sale del árbol. Una recarga sólo vuelve a armar la sesión, y con el modelo cargado deja de haber dos parses del mismo archivo (~250 MB).

`FastEmbedLocal` pasa a llamarse `E5Embedder` (ya no es fastembed) y su hilo, `embedder-worker`. El contrato del canal (textos ya prefijados de entrada, vectores en el mismo orden de salida) y sus tests no cambian.

### Las mismas codificaciones, sin tocar el tokenizer compartido

El chunker y el divisor del índice de docs cuentan con `encode(texto, false)` sin truncar (`crates/clawg-core/src/memory/chunker/token_splitter.rs:41`, `:80`): ponerle al tokenizer compartido la truncación o el relleno de fastembed cortaría los saves largos. El embedder reproduce cada paso de fastembed por fuera del tokenizer:

| Paso | fastembed (`common.rs:52-149`, `text_embedding/impl.rs:367-417`) | `E5Embedder` |
|---|---|---|
| Ventana | Truncación a `min(512, model_max_length)` con los parámetros por defecto (derecha, sin stride), contando `<s>` y `</s>` | `EMBED_MAX_TOKENS`, el mismo número que lee el compositor; la carga rechaza un `tokenizer_config.json` con `model_max_length` menor. Por texto: `encode(t, false)`, `Encoding::truncate(EMBED_MAX_TOKENS − added_tokens(false), 0, Right)` y `post_process(enc, None, true)`, los tres pasos que hace `post_process` con truncación (`tokenizers-0.22.2/src/tokenizer/mod.rs:1206-1258`): `</s>` queda al final |
| Relleno | `BatchLongest` con el `pad_token_id` de `config.json` (0 si falta), tipo 0, a la derecha | Igual, a mano sobre ids, máscara y tipos |
| Tokens especiales | `add_special_tokens` con cada entrada de `special_tokens_map.json` | No se llama. Para un token que `tokenizer.json` ya marca especial sólo cambia su entrada del decodificador: los tries salen de la lista del archivo (`added_vocabulary.rs:258-342`). La carga rechaza un token del mapa que el tokenizer no tenga como especial |
| `token_type_ids` | Sólo si la sesión tiene esa entrada (`impl.rs:180-183`, `:412-417`), con los tipos de la codificación | Igual |
| Lote | Una corrida por pedido de hasta `EMBED_BATCH` textos (su lote de 256 nunca actúa) | Igual: los mismos tensores, con las mismas formas |
| Salida | La única, o la primera de `text_embeds`, `last_hidden_state`, `sentence_embedding` (`output.rs:13-20`) | Igual, resuelta al armar la sesión; sin ninguna, `LoadFailed` |
| Pooling | `Σₜ mₜ·xₜ` en orden de t con el relleno incluido (`sum_axis` sobre un eje no contiguo, `ndarray-0.17.2/src/numeric/impl_numeric.rs:268-283`), dividido por `Σₜ mₜ`; después `v / (‖v‖ + 1e-12)` (`pooling.rs:34-76`, `common.rs:151-157`) | Las mismas operaciones en el mismo orden, en una función pura |

La carga también rechaza un tokenizer con truncación o relleno propios: su `encode` cortaría o rellenaría antes que la ventana del embedder. Los archivos del modelo de hoy pasan esos controles: `tokenizer.json` trae `"truncation": null` y `"padding": null`, y marca `special` sus cinco tokens agregados (`<s>`, `<pad>`, `</s>`, `<unk>`, `<mask>`), los mismos que nombra `special_tokens_map.json`; `tokenizer_config.json` dice `model_max_length` 512, y `config.json`, `pad_token_id` 0. `onnx/model.onnx` declara `input_ids`, `attention_mask` y `token_type_ids` de entrada y `last_hidden_state` de salida, así que los tipos se pasan y la salida es la única. El arnés K probó la sesión y el pooling con un tokenizer configurado como el de fastembed; la ventana y el relleno a mano son nuevos, y los prueban los tests de [abajo](#probar-que-los-vectores-son-los-mismos).

### Un dueño del tokenizer

Hoy un arranque con el modelo en disco parsea dos veces, sin orden entre las dos: el worker, al empezar y sin mirar el SHA-256 (`resolve_tokenizer_and_chunker`, `crates/clawg-core/src/memory/worker.rs:809`, por `load_tokenizer`, `crates/clawg-core/src/memory/model_download.rs:133`), y fastembed, de los bytes verificados, en el `spawn_blocking` del bootstrap. En el primer arranque el worker no encuentra el archivo: el chunker parte por bytes toda la vida del proceso y la celda queda vacía, así que el índice de docs (`crates/clawg-cli/src/daemon/api.rs:1210`, `:1299`) también; el único parse es el de fastembed.

**Elegido:** la celda es la única copia, y los dos caminos la llenan con `OnceCell::get_or_try_init`: el primero parsea y el otro espera y se lleva el mismo `Arc`.

- **El worker** recibe en `resolve_tokenizer_and_chunker` la función que carga, y el daemon le pasa una que lee `tokenizer.json` y compara su SHA-256 con `TOKENIZER_JSON_SHA256` antes de parsearlo: ahora los vectores dependen de este parse, y hasta hoy sólo los cortes del chunker. `load_tokenizer` queda como está para las herramientas de evaluación.
- **La carga del embedder** (`production_deps`, `crates/clawg-core/src/memory/bootstrap.rs:232`, que pasa a recibir la celda; `serve_deps` y `crates/clawg-cli/src/daemon/serve.rs:93` le pasan `runtime.tokenizer`) toma la celda, o la llena con los bytes que `ensure_model` acaba de verificar.
- **Cada parse deja una línea en el log** (`tokenizer.json parsed in N ms`): es lo que cuenta R7.

| Caso | Parses |
|---|---|
| Arranque con el modelo en disco, en cualquier orden | 1 |
| Primer arranque | 1, el de la carga. El chunker sigue partiendo por bytes hasta el próximo arranque, como hoy; el índice de docs ya recibe el tokenizer cuando el modelo carga |
| `tokenizer.json` alterado | 0 en el worker; `ensure_model` lo baja de nuevo y parsea la carga: 1 |
| Recarga, reintento del bootstrap | 0 |
| `CLAWG_TEST_EMBEDDER=fake` con `tokenizer.json` | 1, el del worker: la línea de base de R7 |

`EvalEmbedder::real` (`crates/clawg-core/src/eval_api.rs:427`) parsea su propia copia de los bytes verificados: las herramientas de evaluación no son el daemon.

### Dependencias

fastembed sólo aparece en `clawg-core` (`embedder.rs:10-12`, `model_download.rs:17`, `bootstrap.rs:298` y `:741`, y `eval_api.rs:431` por `FastEmbedLocal`); `clawg-cli` y `clawg-eval` no lo nombran. Sale de `Cargo.toml:43` y de `crates/clawg-core/Cargo.toml:50`, y `TokenizerFiles` pasa a ser un tipo de `model_download.rs` con los mismos cuatro campos. Las versiones son las del `Cargo.lock` y las features, la unión que hoy pide fastembed, así que no se compila nada nuevo:

| Crate | Declaración | Por qué |
|---|---|---|
| `ort` | `=2.0.0-rc.12`, `default-features = false`, `features = ["std", "ndarray", "api-24", "download-binaries", "tls-native"]` | Lo que pide fastembed (`fastembed-5.13.4/Cargo.toml:177-184`, más `ort-download-binaries`, que es `ort/download-binaries` y `ort/tls-native`, `:100-104`). `ndarray` queda aunque clawg no la use: `std` ya trae ndarray (`ort/std` incluye `ndarray/std`), y sacarla recompila ort sin ganar nada |
| `tokenizers` | `0.22.2`, `default-features = false`, `features = ["onig"]` | `onig` hoy llega sólo por fastembed (`fastembed-5.13.4/Cargo.toml:196-199`); sin ella tokenizers no compila (`tokenizers-0.22.2/src/utils/mod.rs:14-15`) |
| `ndarray` | `0.17.2`, `default-features = false`, en `[dev-dependencies]` de `clawg-core` | Sólo para el test que compara el pooling con la fórmula de fastembed; ya está en el build por ort |

El lock pierde `fastembed` y `safetensors` y no suma nada. El comentario de `ort` en `Cargo.toml` deja de decir «subirlo con fastembed»: subirlo pasa a ser una decisión propia, con el test de los vectores.

### La recarga

Pasa por el mismo `ensure` y el mismo `load` (D3): lee y verifica los cinco archivos, arma sólo la sesión de `model.onnx` y descarta sin parsear los bytes de `tokenizer.json` (16 de los 465 MB que hashea). Los dos JSON chicos se leen otra vez para el `pad_token_id` y los rechazos. La sesión usa las opciones de hoy: `cpu_provider` sin arena (`crates/clawg-core/src/memory/embedder.rs:146`), optimización de nivel 3, tantos hilos intra-op como `available_parallelism` y `commit_from_memory` (`impl.rs:133-159`).

Un solo hilo `embedder-worker` es dueño de la sesión: tokeniza (`encode_batch(textos, false)`, en paralelo como fastembed), corre y hace el pooling. Al cerrarse el canal suelta la sesión y llama a `return_freed_memory` (`embedder.rs:315-316`); su `Arc` del tokenizer sólo baja la cuenta.

### Probar que los vectores son los mismos

| Dónde | Qué | Cómo |
|---|---|---|
| CI | La codificación | Un tokenizer armado en código (`WordLevel` con `RobertaProcessing`) y un clon configurado como lo configura fastembed (`common.rs:98-111`): para textos bajo, en y sobre la ventana, en lotes de largos mezclados, los mismos ids, máscara y tipos |
| CI | El pooling | Estados ocultos al azar con una máscara con relleno, contra la fórmula de fastembed escrita con ndarray: los mismos bits (`to_bits`) |
| CI | El resto | Los rechazos de la carga, la elección de la salida, un parse por proceso (con un cargador que cuenta) y una recarga que no parsea; los tests del canal siguen |
| Con el modelo | Los bits de fastembed | `crates/clawg-core/tests/e5_golden.rs` y su fixture, commiteados antes del cambio y generados con fastembed (`CLAWG_BLESS=1`, como el contrato del agente); el commit que cambia la inferencia no toca el fixture. Textos: consultas cortas, español, CJK y emoji, uno de ~2.000 tokens, tres cuyo contenido tiene 509, 510 y 511 tokens, `<s> </s> <pad> <unk> <mask>` literales, uno vacío y un lote de 17. Lleva `#[ignore]` con la razón (el primero del repo: la suite va a decir 1 ignorado, y es éste); se corre con `CLAWG_E5_MODEL_DIR` y `--ignored`, y sin la variable falla. Compara bits si la CPU coincide con la del fixture (arquitectura, AVX2, AVX-512, FMA), porque ONNX Runtime elige sus kernels por CPU; si no, una diferencia máxima de 1e-6, y lo dice |
| Con el modelo | El pipeline entero | `rag-eval --diff` con `--real` antes (`main`) y después, también con `--agent-view`: ningún caso se mueve |

### R7 sobre lo medido

El «menos de 300 MB» salió de un arnés sin el tokenizer del daemon. El piso real es el daemon con el tokenizer y sin modelo: 262-265 MB en debug ([verify](verify.md#causa)). El criterio pasa a ser relativo a ese piso:

- **Margen: 64 MB sobre la mediana de 10 recargas.** En las 40 vueltas del arnés, soltar la sesión deja 16-21 MB sobre el arranque en S y 27-48 en K. Una muestra suelta es bimodal (algunas vueltas de S quedan en 66-70 MB y algunas de K en 384), por eso la mediana. Hoy, contra 265 MB: +45, +70, +83, +129 y +133 en las recargas 1 a 5.
- **Sin crecer: la mediana de las recargas 6 a 10 no supera la de la 1 a 5 en más de 16 MB.** Entre esas dos mitades K se mueve ~4 MB y S ~1; hoy cada recarga suma de 4 a 46 MB.
- **Un parse:** una sola línea `tokenizer.json parsed` en el log de toda la corrida.

Procedimiento, en Windows y en Linux, con el mismo build debug con `testing` (`cargo test --workspace --target-dir target-test`):

1. Línea de base: un `CLAWG_TEST_HOME` con sólo `.clawg-dev/models/multilingual-e5-small/tokenizer.json` (el real, que pasa el SHA-256) y `CLAWG_TEST_EMBEDDER=fake`; `clawg serve --profile dev --port 0`. Con `embedder` en `ready`, una muestra estable: 8 lecturas cada 0,5 s dentro de 1 MB.
2. Con el modelo: otro `CLAWG_TEST_HOME` con los cinco archivos, sin `CLAWG_TEST_EMBEDDER` y con `CLAWG_EMBEDDER_IDLE_MINUTES=1`. Diez vueltas: un `POST /v1/search` con el token de ese home y una muestra con `embedder_resident` en `true`; después, esperar `false` y tomar la muestra estable. Nunca `clawg memories search`: es la búsqueda por texto (`search_args`, `crates/clawg-cli/src/memories.rs:368`) y no carga el modelo.
3. Windows: `PrivateMemorySize64`. Linux: `RssAnon` de `/proc/<pid>/status`, con el trim de glibc todavía en el hilo.
4. La `distance` de cada hit, igual en las diez vueltas.

### Alternativas

| Alternativa | Por qué no |
|---|---|
| mimalloc como asignador y `mi_collect(true)` tras soltar (M) | Queda plano, pero en 117-132 MB tras soltar contra 19-25 de la sesión sola; es un asignador global nuevo para todo el proceso, y con el modelo cargado siguen los dos parses |
| El proceso hijo (C de la exploración) | Devuelve todo al morir, pero suma un binario, un protocolo y su ciclo de vida, y el hijo tendría su propio tokenizer: dos parses con el modelo cargado y uno más en cada recarga |
| fastembed más `HeapCompact` | Devuelve de 1 a 2,5 MB: tras 40 vueltas, 506 y 478 MB contra 518 |
| `CLAWG_EMBEDDER_IDLE_MINUTES=0` por defecto en Windows | Renuncia al cambio donde más pesa: ~1 GB ocioso |
| Un fork de fastembed, o esperar un constructor con un tokenizer ya parseado | Un fork entero para usar una parte; esperar deja a Windows creciendo |
| Darle a fastembed un clon del tokenizer compartido | No lo acepta, y `Tokenizer::clone` copia el vocabulario entero: las mismas asignaciones que un parse |

### Archivos

| Archivo | Cambio |
|---|---|
| `crates/clawg-core/src/memory/embedder.rs` | `E5Embedder`: la sesión, la ventana, el relleno, la salida, el pooling y los rechazos de la carga |
| `crates/clawg-core/src/memory/model_download.rs` | `TokenizerFiles` propio; la carga de `tokenizer.json` con su SHA-256 |
| `crates/clawg-core/src/memory/worker.rs`, `crates/clawg-core/src/runtime.rs` | `resolve_tokenizer_and_chunker` con `get_or_try_init` y el cargador verificado |
| `crates/clawg-core/src/memory/bootstrap.rs`, `crates/clawg-cli/src/daemon/serve.rs` | La celda del tokenizer llega a la carga del embedder |
| `crates/clawg-core/src/eval_api.rs` | `EvalEmbedder::real` arma `E5Embedder` |
| `crates/clawg-core/tests/e5_golden.rs` y su fixture | Los vectores de fastembed, con el modelo |
| `Cargo.toml`, `crates/clawg-core/Cargo.toml`, `Cargo.lock` | Sin fastembed; `ort` y `tokenizers` con sus features; `ndarray` de desarrollo |
| `.claude/CLAUDE.md` y las instrucciones de los otros agentes, `openspec/README.md:177`, los comentarios de `chunker/mod.rs:13` e `idle_embedder.rs:9` | Ya no dicen fastembed |

## D6. Una sola arena de malloc en Linux

**Problema.** En Linux con glibc, R7 no se cumple: con el modelo suelto el daemon queda 60-76 MB sobre la línea de base y sube a escalones ([verify, «R7 en Linux»](verify.md#r7-en-linux)). Cada escalón es una arena de `malloc` que retiene de 6 a 24 MB después del `malloc_trim(0)`. Los hilos que arman, usan y sueltan la sesión cambian en cada carga, así que cada recarga puede estrenar otra arena hasta llegar al tope de glibc ([verify, «Por qué no cumple»](verify.md#por-qué-no-cumple-las-arenas-de-glibc)).

**Decisión.** `clawg serve`, en Linux con glibc, fija `mallopt(M_ARENA_MAX, 1)` antes de crear cualquier hilo. Medido con `MALLOC_ARENA_MAX=1`: la memoria suelta queda 7,2 MB sobre la base y plana en diez recargas, la recarga tarda lo mismo y la inferencia no se ve más lenta.

- **Antes de cualquier hilo.** glibc le asigna la arena a un hilo en su primer `malloc`, y `#[tokio::main]` arma los workers antes del cuerpo de `main`. Por eso `main` es síncrona: fija la arena y después construye el runtime con `tokio::runtime::Builder::new_multi_thread`. Sólo para `serve`, porque los demás comandos no cargan el modelo.
- **Lo puro aparte.** Qué comando lo pide es `Command::loads_the_model` (`crates/clawg-cli/src/cli.rs`), pura y con test; el `mallopt` es `keep_one_malloc_arena`, una línea `unsafe` al lado de `return_freed_memory` en `embedder.rs`, con el mismo `cfg` y vacía en las demás plataformas.
- **El trim sigue.** Con una sola arena, `malloc_trim(0)` es lo que devuelve las páginas libres al soltar la sesión.
- **Windows y otros asignadores, sin cambios:** R7 ya se cumple en Windows.

**Alternativas.**

| Alternativa | Por qué no |
|---|---|
| `MALLOC_ARENA_MAX=2` | Cumple (+42,5 y +1,2) pero oscila entre 311 y 329 MB según qué arena toque cada vuelta |
| Un hilo dueño de la sesión que vive lo que vive el proceso, con el pool global de hilos de ONNX Runtime | Acota las arenas y las reusa, pero cambia el ciclo de vida del worker, el pool de hilos y cómo se arma la sesión; deja igual la primera arena retenida, y una sola arena ya alcanza |
| mimalloc u otro asignador global | Un asignador nuevo para todo el proceso; ya se descartó en D5 por el piso que deja |
| Que el usuario fije `MALLOC_ARENA_MAX` | El daemon lo arranca el puente; el objetivo no puede depender de una variable que nadie pone |

**Verificación.** La de R7 en Linux, en las dos corridas (línea de base y diez recargas) con el build de la suite, y la suite con clippy. En Windows no cambia nada; el CI lo cubre.

**La huella de los vectores, en el mismo cierre:** la huella de `e5_golden` no distingue el sistema operativo, y en Linux, con la misma huella de CPU que la fixture de Windows, los bits difieren por 8,9e-8 ([verify, «Los vectores en Linux»](verify.md#los-vectores-en-linux-r9)). La huella suma `os` (`std::env::consts::OS`), la fixture dice `windows`, y R9 dice «en una CPU y un sistema operativo como los que generaron la referencia».
