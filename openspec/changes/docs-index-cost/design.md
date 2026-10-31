# docs-index-cost — Diseño

Decisiones con alternativas. Requisitos en la [spec](spec.md) (IC1 a IC8); decisiones de alcance en la [propuesta](proposal.md). Cada pieza dice qué es puro y qué es I/O.

## D1. Cómo entrega el refresco sus tiempos (IC1, IC7)

| Opción | A favor | En contra |
|---|---|---|
| **a. Una entrada por doc con su ruta relativa, su `DocKind`, sus pasajes escritos y sus seis fases, más `scan` y `remove` aparte** (elegida) | Datos y nada más: el daemon los suma, `rag-eval` los reparte por área. Producción no sabe qué es un área ni nombra `archive/` | Un `Vec` de una entrada por doc reindexado en cada refresco del daemon (unos cientos de bytes por doc, que el daemon suelta al loguear) |
| b. `refresh` recibe un clasificador y devuelve los tiempos por área | Devuelve poco | El concepto de área entra en la firma de producción, que pasa siempre la misma constante; un test del daemon no tiene qué clasificar |
| c. Un observador (`&mut dyn FnMut(DocTiming)`) | No acumula | Una abstracción con un consumidor por llamador; el daemon necesita un cierre sólo para sumar, y el observador ve lo mismo que el `Vec` |

Los tipos, en `crates/clawg-core/src/memory/project_docs/mod.rs`, son `pub` como el resto del módulo (`RefreshReport`, `mod.rs:74`) porque los leen el daemon (otro crate) y `eval_api`:

- `DocPhases { read, mask, split, lookup, embed, write: Duration }`, `Copy`, con una suma.
- `DocTiming { path: String, kind: DocKind, passages: usize, phases: DocPhases }`. `passages` son los pasajes que el doc escribió: 0 para uno que no se leyó (`mod.rs:453-456`) o que quedó como estaba (`mod.rs:470-473`).
- `RefreshTiming { scan: Duration, docs: Vec<DocTiming>, remove: Duration }` y `RefreshTiming::phases() -> DocPhases`, la suma de los docs con `remove` dentro de `write`. Puro.

`DocKind` es `Copy` y `pub` (`crates/clawg-core/src/memory/project_docs/corpus.rs:36-41`). La ruta se queda en memoria: ni el log del daemon (D4) ni el archivo de resultados (D13) la llevan.

## D2. Cómo viajan los tiempos sin tocar `RefreshReport` (IC1, IC2)

| Opción | A favor | En contra |
|---|---|---|
| **a. `refresh` y `RefreshGuard::run` devuelven `RefreshRun { report, timing }`; `DocIndex::try_refresh` sigue devolviendo `RefreshOutcome::Refreshed(RefreshReport)`** (elegida) | `RefreshReport` y `RefreshOutcome` no cambian, ni sus `PartialEq`; los tests que pasan por `try_refresh` (los de `mod.rs:860-1095` vía `refreshed`, y `crates/clawg-cli/tests/api.rs:3240`, `:3405`, `:3486`) no se tocan | Dos llamadores leen `.report`: `refresh_docs` y `EvalDocIndex::index` |
| b. Un campo de tiempos en `RefreshReport` con un `PartialEq` a mano que lo ignora | Un solo tipo | Un `PartialEq` que miente; `Debug` imprime duraciones en cada fallo de un test |
| c. Un parámetro de salida `&mut RefreshTiming` | El tipo de retorno no cambia | Idioma raro en Rust; cambia igual cada llamada, y `RefreshGuard::run` lo tendría que reenviar |

`refresh` (`pub(crate)`, `mod.rs:422`) tiene dos llamadores, `RefreshGuard::run` (`mod.rs:198`) y `EvalDocIndex::index` (`crates/clawg-core/src/eval_api.rs:1285`). El daemon llama `guard.run` (`crates/clawg-cli/src/daemon/api.rs:3001`); `try_refresh` (`mod.rs:281-292`) sólo lo usan tests y mapea `.report`. `RefreshRun` es `pub` porque `RefreshGuard::run` es `pub`.

El único test de `RefreshReport` que llama `refresh` directamente, `a_doc_left_out_is_neither_indexed_nor_counted` (`mod.rs:934`), lee `.report` en la llamada; su aserción (`mod.rs:938`) no cambia. Lo mismo `a_doc_index_answers_from_the_root_it_indexed` (`eval_api.rs:2519-2522`) con `EvalDocIndex::index` (D13).

## D3. Dónde se corta cada fase (IC1)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un reloj de vueltas: cada fase vale el tiempo desde la marca anterior, y la marca avanza** (elegida) | Las fases de un doc suman todo su tiempo en el bucle y no queda trabajo sin medir entre dos fases; la cota del 5 % de las corridas de cierre mide el reloj, no un olvido | Lo que no es la llamada que da nombre a la fase (preparar una lista, contar) cae en la fase siguiente |
| b. Un `Instant` alrededor de cada llamada | Cada fase es exactamente su llamada | Lo de en medio (`fill`, los contadores, armar `IndexedDoc`) no se mide en ninguna, y la suma queda corta por una cantidad que nadie ve |

Una función privada `lap(&mut Instant) -> Duration`. Los cortes, sobre `mod.rs:430-485`:

| Fase | Desde | Hasta |
|---|---|---|
| `scan` | el comienzo de `refresh` | después de armar `files` e `indexed_before` (`:432-446`) |
| `read` | — | `corpus::read` (`:448-456`, con el `clone` del `DocFile`) |
| `mask` | — | `redact` (`:457`) |
| `split` | — | `passages` (`:458`) |
| `lookup` | — | `stored_vectors`, `reuse_plan` y la lista `to_embed` (`:459-467`) |
| `embed` | — | `embed_passages` (`:468-475`) |
| `write` | — | `fill`, los contadores, `IndexedDoc` y `replace_doc` (`:476-481`) |
| `remove` | — | `remove_docs` (`:483-484`), aparte de los docs |

Respuestas a la spec:

- **Un doc que quedó como estaba** (`DocIndexError::Vector`, `mod.rs:470-473`) **o que no se leyó** (`mod.rs:453-456`): se anota su entrada en el `continue`, con las fases que corrió y las demás en cero, y `passages = 0`. Su tiempo cuenta en los totales porque se gastó. Una corrida de `rag-eval` con un doc así falla igual (`eval_api.rs:1290-1292`), así que el reparto por área nunca ve uno.
- **El trabajo sin llamada propia** (`fill`, `mod.rs:476`; los contadores, `:477-478`) va a `write`, por la tabla de arriba; no hay trabajo sin fase dentro de `refresh`.
- **Un refresco sin cambios** no trae docs; `remove` es la llamada a `remove_docs`, que vuelve sin tocar el store cuando no hay rutas (`mod.rs:673-675`): casi cero, no cero.
- **Un `?` que corta el refresco** (`mod.rs:461`, `:474`, `:480`, `:483`) devuelve el error sin tiempos: el daemon sólo loguea fases de un refresco que terminó (IC2).

Puro: `lap`, los tipos y la suma. I/O: nada nuevo; se mide lo que `refresh` ya hacía.

## D4. La línea del daemon (IC2)

| Opción | A favor | En contra |
|---|---|---|
| a. Reutilizar `Summary` de `timing.rs` | La línea de la importación | `Summary` lee `ImportTimings` y nombra `Step` (`crates/clawg-core/src/memory/portability/timing.rs:13-33`, `:179-191`): habría que meter las fases del refresco en el enum de los pasos de una importación |
| **b. Una función pura propia junto a `refresh_line`, con los segundos de `timing::seconds`** (elegida) | La línea del daemon vive con la otra (`api.rs:2989-2994`) y su test con el de esa (`api.rs:3156-3171`); el formato de los segundos es el de la importación por construcción | `seconds` (`timing.rs:102-104`, privada) pasa a `pub` para que la lea otro crate |
| c. La misma función con su propio `{:.1}` | Ninguna visibilidad nueva | Dos definiciones de cómo un log imprime segundos; la spec ata la línea a la de la importación |

`refresh_timing_line(project_id: &str, took: Duration, timing: &RefreshTiming) -> String` en `crates/clawg-cli/src/daemon/api.rs`, pura:

```
docs refresh of {project_id}: done in {s} s: scan {s}, read {s}, mask {s}, split {s}, lookup {s}, embed {s}, write {s}
```

`took` es el `started.elapsed()` que `refresh_docs` ya mide (`api.rs:3000-3002`), el mismo valor que va a `docs.refresh.duration`, así que la línea y la serie dicen el mismo total. `refresh_docs` la loguea en la rama `Ok`, después de `refresh_line` (`api.rs:3005`), y pasa `refreshed.as_ref().ok().map(|r| &r.report)` a `record_refresh`, cuya firma y series no cambian (`api.rs:3016-3025`). No lleva rutas: sólo las sumas de `RefreshTiming`.

## D5. La marca de la máquina (IC3, IC8)

| Opción | A favor | En contra |
|---|---|---|
| **a. Mover `Cpu` de `crates/clawg-core/tests/e5_golden.rs:32-75` a `eval_api` como `EvalCpu`, que `e5_golden` pasa a usar; los hilos, de una función que `session_builder` también usa** (elegida) | Una sola definición de «la misma máquina» para el test que fija bits y para la memoria; los mismos campos y derives, así que el fixture (`e5_golden.rs:103-106`) se lee igual | Un tipo `pub` más en `eval_api` y un cambio en un archivo de test |
| b. Duplicarlo en `eval_api` | `e5_golden` no cambia | Dos definiciones que pueden separarse: el día que una sume una característica, la memoria y el fixture discrepan en qué es otra máquina |
| c. Leerlo desde `rag-eval` | Nada en `clawg-core` | La clave (D6) vive en `eval_api` y necesita la marca; `rag-eval` no tiene `is_x86_feature_detected!` envuelto en nada |

- `EvalCpu { os, arch, avx2, avx512f, fma }` con `Serialize`, `Deserialize`, `PartialEq`, y `EvalCpu::this()`, los dos cuerpos por arquitectura de `e5_golden.rs:55-75`. `comparison_for` y su test (`e5_golden.rs:47-53`, `:124-130`) se quedan en el test.
- Los hilos: `session_builder` toma `std::thread::available_parallelism()` (`crates/clawg-core/src/memory/embedder.rs:536`). Se extrae `pub(crate) fn session_threads() -> Result<usize, EmbedError>` en `embedder.rs`, que `session_builder` llama, y `eval_api` lo expone como `pub fn session_threads() -> Result<usize, EvalError>`. Así `index.machine.threads` y la clave nombran el número con que se arma la sesión, no otro cálculo del mismo número.
- **La clave lleva también los hilos.** `embedder.rs:530-533` dice que el número de hilos «sets how ONNX Runtime splits and sums the work, so another one could move the last bits of a vector». Medido igual en una máquina (exploración, premisa de los hilos), pero el instrumento no puede apostar a eso: con los hilos en la clave, un cambio de afinidad o de límite de CPU sólo cuesta reembeber. Esto **amplía IC3**, que nombra sólo los cinco campos de `Cpu`.

Puro: `EvalCpu` y su comparación. I/O: leer la CPU y el paralelismo del sistema, una vez por corrida.

## D6. La clave (IC3)

| Opción | A favor | En contra |
|---|---|---|
| **a. SHA-256 de los componentes en el orden de IC3, cada texto con su largo delante, sobre el texto que `apply_role_prefix` arma con los prefijos del embedder envuelto** (elegida) | La clave cubre el texto exacto que `E5Embedder::embed_batch` manda a su hilo (`embedder.rs:437-439`), por la misma función, «the single authority» (`embedder.rs:193-207`, `pub(crate)`, alcanzable desde `eval_api` sin abrir nada) | Hashea cada texto entero (unos 35 MB para este corpus), dentro de `embed`: segundos en debug |
| b. Rol y texto crudo, con los prefijos sólo en la parte del contrato | Hashea lo mismo | Si un día el prefijo se aplica de otra forma (un recorte, un separador), la clave no lo ve: no es «el texto exacto con su prefijo» |
| c. Un hash más barato (SipHash, que el núcleo ya trae) | Más rápido | 64 bits: una colisión entre 17 584 textos por contrato es improbable, pero el instrumento no puede descartarla; SHA-256 ya es dependencia (`crates/clawg-core/Cargo.toml`, `sha2`) |

`MemoContract { model_id, dimensions, query_prefix, passage_prefix, model_sha256, cpu: EvalCpu, threads }`, privado de `eval_api`. Dos funciones puras:

- `memo_key(contract, role, prefixed) -> [u8; 32]`: SHA-256 de `model_id`, `dimensions`, `query_prefix`, `passage_prefix`, `model_sha256`, el rol, `prefixed`, `os`, `arch`, `avx2`, `avx512f`, `fma` y `threads`, en ese orden. Cada texto entra como `u64` little-endian con su largo en bytes y después sus bytes; los números como `u64` LE; los booleanos y el rol como un byte (`0` consulta, `1` pasaje). Con largos de tamaño fijo, `("ab","c")` y `("a","bc")` no se confunden.
- `contract_digest(contract) -> [u8; 32]`: la misma codificación sin rol ni texto. Nombra y encabeza el archivo (D7).

**El SHA del modelo** es la constante `MODEL_SHA256` (`crates/clawg-core/src/memory/model_download.rs:63`), no un hash que el loader calcule: la carga rechaza cualquier `model.onnx` que no dé ese valor (está fijado en la lista de archivos, `model_download.rs:95`), así que cada vez que un modelo carga, la constante y el archivo coinciden, y la clave sigue siendo pura. `EvalMemo::open` arma el contrato con la constante; un constructor privado recibe el contrato entero, de modo que los tests usan un embedder falso con prefijo, modelo, dimensión y SHA configurables.

## D7. El archivo de la memoria (IC4, IC5)

| Opción | A favor | En contra |
|---|---|---|
| **a. Un archivo por contrato, `docs/eval/cache/docs-vectors-{16 hex de contract_digest}.bin`, binario, escrito entero a un temporal de la misma carpeta y renombrado** (elegida) | Una corrida lee sólo su contrato (≈27,6 MB); otra máquina u otro prefijo escribe otro archivo y nunca lee el ajeno; renombrar es atómico en la misma carpeta, así que con el nombre de la memoria sólo hay un archivo entero | Cada corrida que agrega algo reescribe el archivo completo |
| b. Un solo archivo para todos los contratos | Un archivo | Cada corrida carga los vectores de todos los contratos que hubo; tras el experimento del prefijo, el doble |
| c. Agregar al final del archivo existente | Escribe sólo lo nuevo | Una corrida cortada a la mitad deja un final truncado, y un archivo que no se lee entero no aporta ninguno (IC5): se perdería todo lo anterior |
| d. JSON | Legible | 17 584 × 384 `f32` como texto: varias veces el tamaño, y la ida y vuelta de un `f32` por texto es un riesgo que el binario no tiene |

Formato, todo little-endian:

| Parte | Bytes |
|---|---|
| marca `clawgvec` | 8 |
| versión del formato, `1` | 4 (`u32`) |
| `contract_digest` | 32 |
| `dimensions` | 4 (`u32`) |
| entradas | 8 (`u64`) |
| cada entrada: clave y `dimensions` `f32` (`to_le_bytes`) | 32 + 4 × `dimensions` |
| SHA-256 de todo lo anterior | 32 |

Las entradas van ordenadas por clave, así que el mismo contenido da los mismos bytes. Para este corpus: 56 + 17 584 × 1 568 + 32 bytes, unos 27,6 MB, la cota de la propuesta.

- **Leer** (`decode(bytes, contract) -> Result<HashMap<[u8;32], Vec<f32>>, MemoUnreadable>`, pura): una marca distinta, una versión que no es `1`, otro `contract_digest`, otra dimensión, un largo que no es exactamente el que dicen la cabecera y las entradas, o un SHA final que no coincide, dan un `MemoUnreadable` (`thiserror`) y ningún vector. El SHA final separa un archivo entero de uno dañado en el medio, no sólo de uno truncado; cuesta un hash de 27,6 MB por lectura y otro por escritura, que van a `memo.seconds`. I/O: `std::fs::read`; un archivo que no existe es una memoria vacía sin aviso (la primera corrida), cualquier otro error de lectura es un `MemoUnreadable` con su causa.
- **Escribir** (`encode(&mut impl Write, contract, entries)`, sobre cualquier `Write`, y `replace_file(path, write)`, I/O): crea la carpeta si falta, crea `{nombre}.tmp-{uuid}` con `create_new` en la misma carpeta, escribe por un `BufWriter`, `sync_all`, y renombra sobre el nombre de la memoria (`std::fs::rename` reemplaza en Windows y en Unix). Si algo falla, borra el temporal y devuelve el error: la memoria anterior queda como estaba. `uuid` ya es dependencia del núcleo. Un proceso matado puede dejar un `.tmp-`, que ninguna lectura abre y que borrar la carpeta limpia.
- **Unir** (pura): lo que se escribe es la memoria leída más lo nuevo. Si la leída no se pudo leer, lo nuevo la reemplaza (IC5). Si no hay nada nuevo, no se escribe: la carpeta queda como estaba. Dos corridas a la vez en el mismo worktree: gana el último renombre, y lo que se pierde es sólo una falta en la corrida siguiente.

Los tests de IC5 cortan el archivo, cambian la cabecera, la versión o un byte del medio, y escriben con un `Write` que falla después de N bytes, todo sobre `crate::test_dir::test_dir()`.

## D8. El decorador (IC4)

| Opción | A favor | En contra |
|---|---|---|
| **a. `MemoEmbedder`, privado de `eval_api`, que implementa `Embedder` sobre el `Arc<dyn Embedder>` de verdad; `EvalMemo` lo envuelve y entrega un `EvalEmbedder` que pasa por él** (elegida) | `EvalDocIndex` lo usa como a cualquier embedder (`eval_api.rs:1278`) y el refresco no sabe que existe; `E5Embedder` y el refresco no cambian | Un `Embedder` más en el crate, compilado sólo con `eval` |
| b. Que `EvalDocIndex::index` consulte la memoria antes de llamar a `refresh` | Sin decorador | Duplica en `eval_api` el partido y los textos de `refresh`, que es justo lo que se mide |

Las reglas, todas sobre el rol `Passage`; una consulta pasa directo al embedder de verdad sin contarse ni guardarse:

- Por cada `embed_batch`, la clave de cada texto (D6) se busca **sólo en lo leído al empezar**. Lo que está se devuelve; lo que falta va al embedder de verdad **en una sola llamada, en el orden en que llegó**, y las respuestas se vuelven a poner en su lugar (puro: `split_hits` y `merge`, como `reuse_plan` y `fill` en `mod.rs:500-520`). Con la memoria vacía, cada llamada del refresco llega igual al embedder de verdad, que corta en `EMBED_BATCH` lo mismo que sin decorador (`embedder.rs:436`): las tandas son las de producción.
- Lo nuevo se junta aparte, una entrada por clave (la primera), y no entra a la búsqueda de la misma corrida.
- **Cuentas** (respuesta c): `from_memo` suma los textos devueltos desde la memoria; `embedded`, los textos mandados al embedder de verdad, repetidos incluidos. Un texto que falta dos veces se embebe las dos y se guarda una. En una corrida que termina bien, sobre un store nuevo (`reused = 0`) y sin docs fallados, `from_memo + embedded = corpus.passages`, que cuenta pasajes escritos. Por el refresco dos docs nunca comparten un `indexed_text`, porque el breadcrumb empieza por la ruta (`crates/clawg-core/src/memory/project_docs/chunk.rs:28-30`); el repetido se prueba sobre el decorador.
- **Una respuesta que no sirve** (IC4): si el embedder de verdad falla, el error sube tal cual. Si devuelve otra cantidad de vectores que los pedidos, el decorador devuelve una lista de largo `aciertos + respuesta`, distinto del pedido, así que el refresco falla con el mismo `VectorCount` que sin memoria (`mod.rs:570-576`), y no guarda nada. Un vector de otra dimensión o con un valor no finito se devuelve en su lugar, para que el refresco falle como fallaría (`to_unit`, `mod.rs:577-581`), y no se guarda.
- `dimensions`, `model_id` y los prefijos los delega en el embedder de verdad, de modo que el refresco escribe el mismo `model_id` y el conteo de tokens (D11) arma el mismo texto.

Puro: claves, `split_hits`, `merge`, la decisión de qué guardar. I/O: ninguno; el decorador sólo llama al embedder.

## D9. Cuándo se lee y se escribe la memoria (IC4, IC5, IC8)

| Opción | A favor | En contra |
|---|---|---|
| **a. Se lee al armar la corrida, antes de medir `index_seconds`, y se escribe en cuanto `EvalDocIndex::index` vuelve, haya terminado bien o no** (elegida) | Una corrida que falla a la mitad deja lo que embebió y la siguiente lo reutiliza; cada vector guardado es entero y vino de una respuesta que pasó los controles de D8. Las preguntas no la tocan, así que después de indexar no hay nada más que agregar | Un proceso matado no escribe nada |
| b. Escribir sólo si la corrida terminó bien | Una memoria «de corridas buenas» | Tira minutos de embebido por una falla ajena al vector (una pregunta que falla, un archivo de resultados que ya existe) |
| c. Escribir por doc | Sobrevive a un proceso matado | Reescribir 27 MB por doc, o agregar al final (D7 c) |

- `EvalMemo::open(embedder, dir) -> EvalMemo` nunca falla: un archivo ilegible queda como aviso (`warning()`) y como memoria vacía con `used = false`; si `session_threads` no responde, la memoria no se lee ni se escribe y se avisa (la sesión del modelo tampoco se habría armado). `used` es `true` sólo si se leyó entero un archivo de este contrato; una carpeta vacía o sin el archivo da `false` (la corrida D de la propuesta, con la memoria vacía, sale con `used = false` y `embedded = corpus.passages`).
- `EvalMemo::finish(&self) -> EvalMemoReport { file, used, from_memo, embedded, seconds, warnings }`: une y escribe (D7) y no falla; un error de escritura es un aviso.
- **`memo.seconds`** (respuesta g) es leer y escribir el archivo, con su decodificación, su codificación y sus hashes: lo que pasa fuera de `index_seconds`. Calcular las claves y buscar en la memoria pasa dentro del refresco y cae en `embed`, como lo que reemplaza.
- `rag-eval` imprime el aviso de lectura antes de indexar y los de escritura al terminar; ninguno cambia la salida.

## D10. El recorder del embedder (IC6)

| Opción | A favor | En contra |
|---|---|---|
| **a. `EvalEmbedder` pasa a `{ inner: Arc<dyn Embedder>, metrics: Arc<Recorder> }`, privados, y `passage_tokens_ran()` suma el gauge `embedder.batch_tokens{role=passage}` de `Recorder::minute()`** (elegida) | El recorder no sale de `eval_api`; el decorador comparte el mismo `Arc`, así que lo que corre por debajo de la memoria se cuenta | Seis usos de `.0` cambian (`eval_api.rs:700`, `:705`, `:1278`, `:1425`, `:2241`, `:2578`) |
| b. Exponer el `Recorder` | Más general | Cualquier llamador podría vaciar la ventana, que es de lo que depende la lectura |
| c. Contar en el decorador | Sin recorder | Mide lo que pidió el decorador, no lo que corrió: justo lo que `padded == padded_ran` tiene que comprobar |

`embedder.batch_tokens` es un gauge (`embedder.rs:509`, `Recorder::gauge`, `crates/clawg-core/src/metrics/recorder.rs:125-135`), y `Recorder::run()` devuelve sólo histogramas (`recorder.rs:171-174`); un gauge queda en la ventana en curso, que sólo `take_minute` vacía (`recorder.rs:161-169`). La lectura de `minute()` vale porque ningún escritor recibe ese recorder: es un campo privado de `EvalEmbedder`, así que nada fuera de `eval_api` puede llamar `take_minute` sobre él. Esa privacidad es la guarda; una frase en el doc de `passage_tokens_ran` lo dice. La suma es un `f64` de enteros, exacta hasta 2^53.

`EvalEmbedder::real` le pasa a `E5Embedder::from_parts` el recorder que conserva (hoy `Arc::default()`, `eval_api.rs:569`); `fake` lleva uno vacío. Un constructor privado `EvalEmbedder::of(inner, metrics)` lo usan `real`, `fake`, `EvalMemo` y los tests, que lo arman con un embedder de prueba que escribe gauges `role=passage` y `role=query`.

## D11. Contar los tokens (IC6)

| Opción | A favor | En contra |
|---|---|---|
| **a. `EvalDocIndex::passage_lengths()` lee los pasajes escritos del store, les pone el prefijo de pasaje con `apply_role_prefix` y los tokeniza con `window_encodings`, por doc y en grupos de `EMBED_BATCH`; los rellenos se calculan en `rag-eval` sobre las listas de largos** (elegida) | Cuenta exactamente lo que el modelo recibe (`window_encodings` es la misma función, `embedder.rs:240-255`, y `Encoding::len` es lo que `pad_right` rellena, `:259-261`); tokeniza con la forma de llamada de producción, así que `count_seconds` estima lo que tokenizar cuesta dentro de `embed`; los rellenos son puros | `EvalDocIndex` guarda el tokenizer que ya carga (`eval_api.rs:1274-1277`) además del splitter |
| b. Contar dentro del refresco | Sin releer el store | Código de producción, y su costo cae en `embed`, que es lo que se mide |
| c. Contar en el decorador | Ve los textos | Con memoria ve sólo los que faltan, y tokenizar ahí también cae en `embed` |

- El orden: el store se lee por `path` y `ordinal`. Sobre un store nuevo cada doc embebe todos sus pasajes en una llamada y en orden de `ordinal` (`mod.rs:466`, `:500-510`), así que la lista de largos de un doc es la que el embedder cortó en tandas. El texto se rearma como lo hace `stored_vectors` (`mod.rs:543-549`): `Passage { breadcrumb, body }.indexed_text()`, y el prefijo del embedder del índice (el decorador delega el suyo).
- El tokenizer es el de `model_download::load_tokenizer(base_dir)` (`model_download.rs:140-149`), el mismo `tokenizer.json` que la carga del modelo ya verificó y parseó para el embedder. Con `--fake` no hay tokenizer y no se cuenta nada (`passage_lengths` da `None`).
- `count_seconds` mide sólo las llamadas a `window_encodings`, no la lectura del store.
- Puro, en `rag-eval` (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs`): `padded(lengths)` (tandas de `EMBED_BATCH` en orden, filas × el más largo), `padded_sorted(lengths)` (lo mismo después de `longest_first`) y los totales: `real`, `padded`, `padded_sorted_in_doc` (cada doc ordenado), `padded_sorted_all` (todos los largos de la corrida en una lista, ordenada) y `padded_ran`. Ninguna cota entre los tres rellenos: con una tanda final corta, ordenar puede rellenar más (spec, IC6).
- `padded != padded_ran` avisa por stderr cuando **ningún pasaje salió de la memoria** (`from_memo == 0`): entonces cada pasaje corrió en las tandas de producción y los dos números tienen que coincidir. Con `from_memo > 0`, `padded_ran` cuenta sólo lo que faltaba y el aviso no dice nada. Incluye una memoria leída que no tenía ningún texto de la corrida, que `memo.used = false` dejaría afuera (IC6).

El orden por largo:

| Opción | A favor | En contra |
|---|---|---|
| **a. Reutilizar `longest_first` de la importación** (`crates/clawg-core/src/memory/portability/import.rs:2537-2541`): pasa a `pub(crate)` y `eval_api` lo expone como `pub fn longest_first` (elegida) | La estimación de A1 de docs-index-speed usa el orden que ya está probado (`import.rs:5222-5226`), descendente y estable, y que una implementación de A1 usaría | Una función de la importación queda alcanzable desde la evaluación |
| b. Duplicarla en `rag-eval` | Nada cambia en el núcleo | Dos órdenes «iguales» que nadie compara |

`EMBED_BATCH` se reexporta en `eval_api` como `EMBED_MAX_TOKENS` y `SPECIAL_TOKEN_RESERVE` (`eval_api.rs:1015-1018`), para que `rag-eval` corte con el número del embedder.

## D12. El reparto por área (IC7)

Puro y en `index_cost.rs`: `area_of(path, code) -> Area`, con `Area::{Docs, Code, Archive}`: `Archive` si la ruta empieza con `openspec/changes/archive/`, si no `Code` para un doc de código y `Docs` para el resto. `rag-eval` junta por ruta la entrada de tiempos de cada doc (D1, D13) con su lista de largos (D11) y suma por área `docs`, `passages`, `multi_batch_docs` (más de `EMBED_BATCH` pasajes), las seis fases y `tokens.{real, padded, multi_batch_real, multi_batch_padded}`. `scan` no se reparte. Las tres áreas suman los totales por construcción, y un test lo fija.

El refresco no recibe nada del área (D1); `production_never_names_the_archive` busca `archive/` en las fuentes de `crates/clawg-core/src/memory/project_docs/` desde `CARGO_MANIFEST_DIR` (hoy no aparece en ninguna fuente del núcleo).

## D13. La frontera de `eval_api` y el bloque `index` (IC8)

| Opción | A favor | En contra |
|---|---|---|
| **a. `EvalDocIndex::index` devuelve `EvalIndexRun { report: EvalDocReport, scan, remove, docs: Vec<EvalDocTiming> }`; `EvalDocReport` sigue `Copy` con sus tres conteos** (elegida) | Los tests y el informe que leen `EvalDocReport` (`crates/clawg-eval/src/bin/rag-eval/docs.rs:898`, `:917`, `:1057`, `:1081`) no cambian; la ruta queda en `rag-eval`, que la necesita para el área y no la escribe | Un llamador (`docs.rs:710`) y un test (`eval_api.rs:2522`) leen `.report` |
| b. Ensanchar `EvalDocReport` con los tiempos, los largos y la memoria | Un tipo | Deja de ser `Copy`, cada literal suyo cambia, y mezcla lo que mide el índice con lo que mide la memoria, que viene de otro objeto |

Respuesta h: los tiempos llegan en `EvalIndexRun`, los largos en `EvalPassageLengths` (D11) y la memoria en `EvalMemoReport` (D9); `rag-eval` los junta. `EvalDocTiming { path, code: bool, passages, phases: EvalPhases }` y `EvalPhases` son tipos propios de `eval_api`, como pide su doc (`eval_api.rs:1-18`): `code` se calcula de `DocKind::Code(_)` como `is_code_path` (`eval_api.rs:1321-1323`).

El bloque, en `index_cost.rs`, sólo `Serialize`, como `DocsResults` (`docs.rs:301-316`):

```
index: {
  build: "debug" | "release",
  machine: { os, arch, avx2, avx512f, fma, threads },
  phases: { scan, read, mask, split, lookup, embed, write },     // segundos, f64
  memo: { used, from_memo, embedded, seconds },
  tokens: { real, padded, padded_ran, padded_sorted_in_doc, padded_sorted_all, count_seconds },
  by_area: { docs | code | archive: { docs, passages, multi_batch_docs,
             phases: { read, mask, split, lookup, embed, write },
             tokens: { real, padded, multi_batch_real, multi_batch_padded } } }
}
```

- `DocsResults` gana `pub index: IndexBlock` después de `index_seconds`; `index_seconds`, `corpus`, `aggregate` y `per_case` no se mueven. `IndexBlock::new` es pura sobre `EvalIndexRun`, los largos, `EvalMemoReport`, `padded_ran`, la marca y `cfg!(debug_assertions)`; con `--no-cache` la memoria entra como `{ used: false, from_memo: 0, embedded: corpus.passages, seconds: 0 }`.
- Sólo hay archivo en una corrida real (`docs.rs:591-600`), así que en el bloque nada es opcional: un lector de las puertas de docs-index-speed lee siempre los mismos campos.
- `DocsFile` (`docs.rs:386-396`) no cambia: no lee `index`, no usa `deny_unknown_fields`, y por eso `--diff` lee un archivo con bloque contra uno sin él en los dos órdenes; no hace falta `#[serde(default)]` porque el bloque no se deserializa.
- Ningún campo lleva texto ni ruta: sólo números, booleanos y las cadenas `debug`/`release`, `os` y `arch`.
- El informe por stdout conserva su primera línea (`docs.rs:229-237`; la fija `crates/clawg-eval/tests/docs_mode.rs:49`) y agrega las fases en las dos modalidades; en una corrida real, además, el archivo de la memoria con `from_memo`, `embedded` y `seconds` (o `vector memo: not used (--no-cache)`) y los tokens. `render` recibe las fases siempre y el bloque sólo en una corrida real.

## D14. La línea de comandos (IC4)

- `--no-cache` pone `Cli::no_cache`; sin `--docs` es `CliError::NoCacheWithoutDocs` (salida 1), comprobado junto a `RootWithoutDocs` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:487-489`). Con `--fake --docs` se acepta y no cambia nada.
- `Cli::cache_dir`, sin bandera, lo pone `default_cache_dir()`: `docs/eval/cache` resuelto desde `CARGO_MANIFEST_DIR` como `default_out_dir` (`cli.rs:291-298`), al lado de `results/` y bajo `docs/eval/**` (`.gitignore:32`). Es un campo para que los tests de `docs.rs` lo apunten a un temporal; ninguna bandera lo cambia.
- `memo_dir(cli) -> Option<&Path>`, pura: `Some(cache_dir)` sólo con `--real --docs` sin `--no-cache`. `docs::measure` abre `EvalMemo` sólo cuando da `Some`, así que con `--fake` o `--no-cache` la carpeta ni se lee ni se crea.
- Los tests de la carpeta (`no_cache_leaves_the_folder_byte_for_byte`, `docs_mode_with_the_fake_embedder_never_touches_the_memo`) van en `docs.rs` y no en `crates/clawg-eval/tests/docs_mode.rs`: el binario resuelve siempre `docs/eval/cache` del workspace y no hay bandera para apuntarlo a un temporal, mientras que un test de `docs.rs` arma el `Cli` con `cache_dir` en un `TempPath` y corre `run_in` (como `a_docs_run_removes_its_index`, `docs.rs:768-785`). `memo_dir` se prueba por tabla.
- `--help` describe `--no-cache`, dice dónde vive la memoria, que se acumula sin purga, que se limpia borrando la carpeta y que ocupa a lo sumo unos 27,6 MB por contrato para este corpus.

## Visibilidad nueva y código movido

| Elemento | Antes | Después | Por qué |
|---|---|---|---|
| `DocPhases`, `DocTiming`, `RefreshTiming`, `RefreshRun` (`project_docs/mod.rs`) | — | `pub` | Los lee el daemon, otro crate (D1, D2) |
| `RefreshGuard::run` | `Result<RefreshReport, _>` | `Result<RefreshRun, _>` | El daemon necesita los tiempos (D2) |
| `refresh` | `Result<RefreshReport, _>` | `Result<RefreshRun, _>`, sigue `pub(crate)` | D2 |
| `timing::seconds` (`timing.rs:102`) | privada | `pub` | La línea del daemon usa el formato de la importación (D4) |
| `embedder::session_threads` | dentro de `session_builder` (`embedder.rs:536`) | `pub(crate)`, extraída | La marca y el bloque nombran los hilos de la sesión (D5) |
| `Cpu`, `this_cpu` (`tests/e5_golden.rs:32-75`) | test | `eval_api::EvalCpu` (`pub`), que `e5_golden` usa | Una sola definición de la misma máquina (D5) |
| `import::longest_first` (`import.rs:2537`) | privada | `pub(crate)`, con `eval_api::longest_first` (`pub`) | El orden de A1 (D11) |
| `eval_api`: `EMBED_BATCH`, `session_threads`, `EvalMemo`, `EvalMemoReport`, `EvalIndexRun`, `EvalDocTiming`, `EvalPhases`, `EvalPassageLengths`, `EvalDocLengths`, `EvalDocIndex::passage_lengths`, `EvalEmbedder::passage_tokens_ran` | — | `pub`, sólo con `eval` | Lo que `rag-eval` lee (D9 a D13) |
| `EvalEmbedder` | `(Arc<dyn Embedder>)` | `{ inner, metrics }` privados | Conserva el recorder (D10) |
| `MemoContract`, `memo_key`, `contract_digest`, `encode`, `decode`, `MemoUnreadable`, `replace_file`, `MemoEmbedder` | — | privados de `crates/clawg-core/src/eval_api/vector_memo.rs` | El submódulo separa la memoria de un `eval_api.rs` de 3 110 líneas |

Nada de esto entra en un build de producción salvo los tipos de D1 y D2, la línea de D4 y la extracción de `session_threads`; ninguna dependencia nueva (`sha2` y `uuid` ya son del núcleo, `serde` con `derive` también).

## Orden de commits

Cada commit compila, pasa la suite y los dos clippy, y empieza por su test que falla.

1. `feat(docs): time each phase of a refresh` (D1 a D3, IC1): `a_refresh_times_each_phase_of_every_doc`, `timing_does_not_change_what_a_refresh_writes`, `an_unchanged_refresh_has_no_doc_timings`; `refresh_docs` y `EvalDocIndex::index` leen `.report`.
2. `feat(daemon): log where a docs refresh spent its time` (D4, IC2): `the_docs_timing_line_names_every_phase`.
3. `feat(eval): keep the recorder of the real embedder` (D10, IC6): `padded_ran_sums_only_passage_batches` y `the_real_embedder_keeps_its_recorder` con el constructor privado.
4. `feat(eval): key a vector by its contract, its text and the machine` (D5, D6, IC3): `the_memo_key_changes_with_each_component_of_the_contract`, `the_memo_key_separates_neighbouring_components`, `the_memo_key_is_stable`; `EvalCpu` pasa a `eval_api` y `e5_golden` lo usa; `session_threads`.
5. `feat(eval): read a vector memo whole or not at all` (D7, IC5): `a_truncated_memo_is_ignored_with_a_warning`, `a_memo_with_an_unknown_header_is_ignored`, `an_unreadable_memo_is_ignored`, `a_failed_write_leaves_the_previous_memo`.
6. `feat(eval): embed passages through a vector memo` (D8, D9, IC4): `a_full_memo_embeds_nothing_and_stores_the_same_vector_bytes`, `an_empty_memo_embeds_in_the_batches_production_does`, `a_changed_doc_embeds_only_its_new_texts`, `what_a_run_adds_is_not_used_until_the_next_run`, `questions_never_go_through_the_memo`, `a_vector_the_embedder_returns_unusable_is_not_stored`, `the_memo_file_holds_no_text`, `a_memo_made_under_another_contract_is_not_used`.
7. `feat(eval): count the tokens of the indexed passages` (D11, IC6): `padding_of_a_batch_is_rows_times_the_longest`, `a_doc_over_embed_batch_runs_several_batches`, `one_batch_pads_the_same_in_any_order`, `sorting_can_pad_more_when_the_last_batch_is_short`, `sorting_keeps_ties_in_their_order`, `sorting_across_docs_batches_the_whole_run`, `an_empty_corpus_has_no_tokens`, `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo`, y uno de `passage_lengths` sobre un store con el tokenizer de palabras de `chunker`.
8. `feat(eval): split the index measurement by area` (D12, IC7): `area_of_a_path_is_decided_by_its_prefix_and_kind`, `areas_add_up_to_the_totals`, `only_docs_over_a_batch_count_as_multi_batch`, `production_never_names_the_archive`.
9. `feat(eval): write the index block of a docs run` (D13, IC8): `the_index_block_reports_the_memo_and_the_phases`, `the_index_block_holds_no_text_and_no_path`, `index_seconds_and_corpus_keep_their_shape`, `a_results_file_without_an_index_block_still_diffs`, `a_results_file_with_an_index_block_still_diffs_against_an_old_one`, `a_fake_run_reports_no_tokens_and_no_memo`; la memoria entra todavía como no usada.
10. `feat(eval): use the vector memo in docs runs by default` (D14, IC4, IC8): `no_cache_without_docs_is_an_error`, `the_memo_folder_sits_next_to_the_results_folder`, `no_cache_leaves_the_folder_byte_for_byte`, `docs_mode_with_the_fake_embedder_never_touches_the_memo`, `the_docs_report_names_the_memo_file`; `--help`.
11. `docs(openspec): record the debug and release runs of the docs index`, con las corridas D y R en `verify.md`, cuando se autoricen con el modelo real del perfil dev: D en un build debug con la carpeta de la memoria vacía; R en un build release en `target-test` con `--no-cache`, que no se publica ni se instala.
