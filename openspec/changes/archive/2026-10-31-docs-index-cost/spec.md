# docs-index-cost — Especificación

Suma IC1 e IC2 a [`specs/project-docs`](../../../specs/project-docs/spec.md) («Refresco incremental») e IC3 a IC8 a [`specs/eval`](../../../specs/eval/spec.md) («`rag-eval --docs`: medir el índice de documentación»). `specs/metrics` no cambia: ningún requisito agrega una serie a `metrics.db`. Origen: [proposal](proposal.md).

Las salidas de `rag-eval` son las de `crates/clawg-eval/src/bin/rag-eval/main.rs:52-62`: 1 fallo de ejecución o de uso, 2 golden inválido, 3 sin modelo, 4 archivo de resultados existente, 6 regresión.

IC6 a IC8 y la parte de `--diff` de IC8 se apoyan en DQ3 y DQ4 de [docs-search-quality](../2026-10-31-docs-search-quality/spec.md) (el archivo de resultados de docs y su `--diff`), que todavía no se archivaron; hoy ya existen en el código (`DocsResults`, `crates/clawg-eval/src/bin/rag-eval/docs.rs:302`; `DocsFile`, `docs.rs:386`; `render_diff`, `docs.rs:543`). El cambio que se archive segundo integra el del otro.

Todos los tests de este cambio usan embedders falsos, no necesitan el modelo y no dejan nada en el temporal del sistema (`crate::test_dir::test_dir()` en `clawg-core`, `TempPath` en `rag-eval`).

## Medir el refresco (`specs/project-docs`)

### IC1. El refresco mide sus fases

`refresh` (`crates/clawg-core/src/memory/project_docs/mod.rs:422`) MUST medir con `Instant`, por doc, el tiempo de cada fase y MUST devolverlo aparte de `RefreshReport`: `RefreshReport` se compara por igualdad en los tests del refresco (`an_unchanged_project_embeds_nothing`, `a_changed_doc_replaces_its_passages_and_a_deleted_one_loses_them`, `a_doc_left_out_is_neither_indexed_nor_counted`, `mod.rs:873-939`) y MUST NOT ganar un campo que dependa del reloj. Las fases son siete:

| Fase | Qué cubre |
|---|---|
| `scan` | Recorrer, leer y hashear todo el árbol, más las huellas guardadas, el plan y lo que se arma antes del bucle (`mod.rs:432-446`). Una vez por refresco, no por doc |
| `read` | `corpus::read` del doc a reindexar (`mod.rs:453`) |
| `mask` | `redact` (`mod.rs:457`) |
| `split` | `passages` (`mod.rs:458`) |
| `lookup` | Los vectores guardados del doc y `reuse_plan` (`mod.rs:459-466`) |
| `embed` | `embed_passages` (`mod.rs:468-475`) |
| `write` | `fill`, los contadores y `replace_doc` de cada doc (`mod.rs:476-481`), y `remove_docs` (`:483`) |

Entre dos fases no queda trabajo sin medir: las fases de un doc suman todo su tiempo en el bucle.

Lo que el refresco devuelve de cada doc MUST llevar su ruta relativa, su `DocKind` (`crates/clawg-core/src/memory/project_docs/corpus.rs:37`), cuántos pasajes escribió y la duración de `read`, `mask`, `split`, `lookup`, `embed` y `write`; la ruta sirve para repartir por área (IC7) y MUST NOT llegar a ningún archivo de resultados. La medición MUST ser pura (`Duration` y conteos). El refresco MUST escribir exactamente lo que escribía: los mismos docs, pasajes, vectores y huellas, y el mismo `RefreshReport`.

#### Escenario: los tiempos no tocan el reporte
- GIVEN dos refrescos del mismo árbol, uno de los cuales entrega también sus tiempos
- WHEN se comparan los `RefreshReport`, los pasajes y los vectores guardados
- THEN son iguales byte a byte

#### Escenario: una fase por doc
- GIVEN un árbol con `a.md`, `b.md` y `c.rs` y un embedder falso que no tarda
- WHEN se refresca
- THEN los tiempos traen una entrada por doc reindexado, con su ruta, su tipo y su cantidad de pasajes, y un `scan` aparte

#### Escenario: nada cambió
- GIVEN un proyecto ya indexado
- WHEN se refresca sin tocar el disco
- THEN los tiempos no traen ningún doc, `embed` vale cero, `write` es sólo la llamada a `remove_docs`, que vuelve sin tocar el store cuando no hay nada que quitar (`mod.rs:673-675`): casi cero, no cero, y `scan` está medido

#### Escenario: un doc que no se leyó
- GIVEN un doc que desaparece entre el recorrido y la lectura (`mod.rs:453-456`)
- WHEN se refresca
- THEN no entra a los pasajes ni a `RefreshReport::indexed`, y su `read` cuenta igual en el total de la fase

**Tests (fallan hoy: no hay nada que medir):** `a_refresh_times_each_phase_of_every_doc` (embedder falso, árbol temporal), `timing_does_not_change_what_a_refresh_writes` (compara índices, byte a byte, de dos refrescos) y `an_unchanged_refresh_has_no_doc_timings`, en `crates/clawg-core/src/memory/project_docs/mod.rs`. Las aserciones de los tests existentes de `RefreshReport` no cambian; el único que llama `refresh` directamente (`a_doc_left_out_is_neither_indexed_nor_counted`, `mod.rs:934`) lee el reporte de lo que devuelve.

### IC2. La línea del daemon

Después de `indexed the docs of …` (`refresh_docs`, `crates/clawg-cli/src/daemon/api.rs:3005`) el daemon MUST escribir una línea `info`:

```
docs refresh of {project_id}: done in {s} s: scan {s}, read {s}, mask {s}, split {s}, lookup {s}, embed {s}, write {s}
```

con `done in` el tiempo total del refresco y cada fase en segundos, `read` a `write` sumados sobre los docs, en el formato de segundos del resumen de una importación (`Summary`, `crates/clawg-core/src/memory/portability/timing.rs:184-190`). La línea MUST salir sólo cuando el refresco terminó bien, sin nombres de doc ni rutas. `refresh_line` (`api.rs:2989-2994`) y `the_docs_log_line_shows_the_reused_passages` (`api.rs:3157-3170`) MUST NOT cambiar. MUST NOT haber series nuevas en `metrics.db`: `record_refresh` (`api.rs:3016-3025`) sigue registrando `docs.refresh.duration` y `docs.refresh.files` y nada más.

#### Escenario: la línea
- GIVEN los tiempos de un refresco con `scan` 0,5 s, `read` 0,25 s, `mask` 0,5 s, `split` 1 s, `lookup` 0 s, `embed` 3 s y `write` 0,75 s, y 6 s de total
- WHEN se arma la línea
- THEN es `docs refresh of p: done in {6} s: scan {0,5}, read {0,25}, mask {0,5}, split {1}, lookup {0}, embed {3}, write {0,75}`, cada valor con el formato de `seconds` de `timing.rs` (el test fija los literales que ese formato da)

#### Escenario: lo demás no cambia
- GIVEN un `RefreshReport` con `docs: 4, indexed: 2, removed: 1, skipped: 3, passages: 9, reused: 7`
- WHEN se arma `refresh_line`
- THEN sigue siendo `indexed the docs of p: 4 docs, 2 reindexed, 1 removed, 3 skipped, 9 passages, 7 reused`

**Tests (el primero falla hoy):** `the_docs_timing_line_names_every_phase` (tabla de duraciones, función pura que arma la línea) en `crates/clawg-cli/src/daemon/api.rs`; se conservan `the_docs_log_line_shows_the_reused_passages` y el test que fija las series de `metrics.db`.

## Medir el índice y no pagar dos veces el mismo vector (`specs/eval`)

### IC3. La clave de la memoria de vectores

`rag-eval --real --docs` MUST identificar cada vector guardado por una clave: el SHA-256 de, en este orden, `model_id`, `dimensions`, el prefijo de consulta y el de pasaje (`E5_QUERY_PREFIX` y `E5_PASSAGE_PREFIX`, `crates/clawg-core/src/memory/embedder.rs:120` y `:141`), el SHA-256 del `model.onnx` (`MODEL_SHA256`, `crates/clawg-core/src/memory/model_download.rs:63`), el rol, el texto exacto con su prefijo y la marca de la máquina: sistema, arquitectura, AVX2, AVX-512F y FMA, los campos de `Cpu` (`crates/clawg-core/tests/e5_golden.rs:28-75`), y los hilos con que se arma la sesión (`session_builder`, `crates/clawg-core/src/memory/embedder.rs:530-536`: otro número de hilos cambia cómo ONNX Runtime reparte y suma el trabajo, y puede mover los últimos bits). Cada componente MUST entrar a la clave de modo que no se confunda con el vecino (con su largo, o con un separador que el componente no pueda llevar). Si cambia cualquiera, la clave cambia y el vector guardado MUST NOT usarse (`from_memo = 0` para ese texto). La clave es pura: no lee disco ni reloj.

Una máquina distinta nunca usa los vectores de otra, porque ahí los últimos bits cambian (`e5_golden.rs:28-31`): no hay tolerancia.

#### Escenario: otro prefijo
- GIVEN una memoria llena con un embedder falso de prefijo de pasaje `"query: "`
- WHEN se indexa el mismo árbol con el mismo embedder con prefijo `"passage: "`
- THEN `from_memo` vale 0 y se embeben todos los pasajes

#### Escenario: otro modelo, otra dimensión
- GIVEN una memoria llena con el modelo `m1` de 384 dimensiones
- WHEN se indexa con `m2`, o con `m1` de 768 dimensiones
- THEN `from_memo` vale 0 en los dos casos

#### Escenario: otro SHA del `model.onnx`
- GIVEN una memoria llena con un SHA de modelo dado
- WHEN se indexa con otro SHA y todo lo demás igual
- THEN `from_memo` vale 0

#### Escenario: otro rol
- GIVEN una memoria con el vector de un texto como pasaje
- WHEN se pide el mismo texto como consulta
- THEN la clave es otra y no hay vector guardado

#### Escenario: otro texto
- GIVEN una memoria con el vector de un texto
- WHEN se pide el mismo texto con un solo carácter distinto, incluido un espacio, o con otro prefijo ya aplicado
- THEN la clave es otra

#### Escenario: otra máquina
- GIVEN una memoria hecha con una marca de máquina
- WHEN la marca difiere en el sistema, la arquitectura, AVX2, AVX-512F, FMA o los hilos, uno por uno
- THEN la clave es otra en cada caso y `from_memo` vale 0

#### Escenario: lo mismo da la misma clave
- GIVEN dos contratos, roles, textos y marcas iguales
- WHEN se calcula la clave dos veces, en dos procesos
- THEN es la misma

**Tests (fallan hoy: no existe la memoria):** `the_memo_key_changes_with_each_component_of_the_contract` (tabla: un componente por fila, de a uno), `the_memo_key_separates_neighbouring_components` (`("ab","c")` frente a `("a","bc")`) y `the_memo_key_is_stable` en `crates/clawg-core/src/eval_api.rs` (o el módulo del decorador); `a_memo_made_under_another_contract_is_not_used` (embedder falso de prefijo, modelo y SHA configurables) con el decorador.

### IC4. Cómo una corrida usa la memoria

La memoria de vectores MUST ser un decorador de `Embedder` dentro de `eval_api` (`EvalEmbedder`, `crates/clawg-core/src/eval_api.rs:546`); el embedder de producción MUST NOT cambiar. MUST estar activa por defecto en `rag-eval --real --docs`, y MUST valer sólo para los pasajes: las preguntas (rol de consulta) MUST NOT pasar por ella ni escribirse en ella.

Por cada pedido de pasajes el decorador MUST consultar la memoria **tal como estaba al empezar la corrida**, devolver lo que tiene y mandar el resto al embedder de verdad en **una sola llamada**, en el orden en que llegó, de modo que con la memoria vacía se embeba en las mismas tandas que sin decorador. Los vectores nuevos se agregan a la memoria **al terminar de indexar**, haya terminado bien o no (cada vector guardado pasó los controles de una respuesta), y MUST NOT entrar a la consulta de esa misma corrida. Una corrida que no agregó nada MUST NOT reescribir el archivo. Un texto repetido dentro de la corrida que la memoria no tenía se embebe cada vez, como sin memoria, y se guarda una sola vez.

- `--no-cache` MUST NOT leer ni escribir la memoria: la carpeta queda byte por byte como estaba, y no se crea si no existía.
- `--fake` MUST NOT usarla nunca; `--no-cache` sin `--docs` MUST ser un error de uso (salida 1), como `--root` sin `--docs`.
- Vive en `docs/eval/cache/`, resuelta como `default_out_dir` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:291-298`), sin depender del directorio de trabajo, junto a `docs/eval/results/` y fuera de git (`docs/eval/**`, `.gitignore:32`). No hay bandera para elegir otra carpeta.
- MUST guardar sólo claves y vectores (los `f32` exactos), nunca texto ni rutas.
- Se acumula sin purga: una corrida no borra lo que no usó. Se limpia borrando la carpeta, y `--help` MUST decirlo, con lo que ocupa: 1568 bytes (32 + 384 × 4) por texto distinto y por contrato. MUST NOT dar una cota, porque la memoria crece con el corpus y con cada corrida que embebe un texto que no tiene: con los 18 249 pasajes del 2026-10-31 ocupa 28,6 MB.
- Un doc que cambia MUST embeber sólo los textos que no están en la memoria; los de sus pasajes sin cambios salen de ella.
- Si un vector que devuelve el embedder de verdad no sirve (cuenta distinta, dimensión o valor no finito), el decorador MUST fallar igual que sin memoria y MUST NOT guardarlo.

#### Escenario: la segunda corrida no embebe nada
- GIVEN un árbol indexado una vez con la memoria vacía y un embedder falso que cuenta los textos
- WHEN se indexa otra vez el mismo árbol con el mismo contrato y la memoria que dejó la primera
- THEN el embedder no recibe ningún texto, `from_memo` es `corpus.passages`, `embedded` es 0 y los vectores guardados son los mismos bytes

#### Escenario: la memoria vacía embebe como producción
- GIVEN un árbol con un doc de 20 pasajes y otro de 3, y un embedder falso que anota el tamaño de cada llamada
- WHEN se indexa con la memoria vacía y, aparte, sin decorador
- THEN las llamadas, su orden y sus textos son los mismos

#### Escenario: sólo se embeben los textos nuevos
- GIVEN la memoria llena de un árbol, y un doc en el que se edita una sección
- WHEN se indexa de nuevo
- THEN el embedder recibe sólo los pasajes cuyo texto con prefijo no estaba, en el orden en que llegaron en una sola llamada, y `from_memo + embedded` es `corpus.passages`

#### Escenario: lo nuevo no se usa en la misma corrida
- GIVEN la memoria vacía y dos pedidos de pasajes al decorador con el mismo texto (por el refresco no se arma: el breadcrumb de cada pasaje empieza con la ruta de su doc)
- WHEN se embeben y la corrida termina
- THEN el embedder recibe el texto en los dos pedidos y `from_memo` vale 0; la memoria queda con una sola entrada para ese texto

#### Escenario: las preguntas no pasan por ella
- GIVEN una memoria llena
- WHEN se hace una pregunta (rol de consulta)
- THEN el embedder la recibe y la memoria no cambia

#### Escenario: `--no-cache`
- GIVEN una carpeta `cache/` con una memoria
- WHEN se indexa con `--no-cache`
- THEN se embebe todo, `index.memo.used` vale `false` y la carpeta queda byte por byte como estaba; si no existía, sigue sin existir

#### Escenario: `--fake`
- GIVEN `rag-eval --fake --docs`
- WHEN corre
- THEN no se abre ni se crea la carpeta de la memoria

#### Escenario: la ruta no depende del directorio de trabajo
- GIVEN el binario ejecutado desde otro directorio
- WHEN se resuelve la carpeta de la memoria
- THEN es `docs/eval/cache/` del workspace, al lado de `docs/eval/results/`

#### Escenario: sólo claves y vectores
- GIVEN una memoria escrita tras indexar un árbol con un texto reconocible
- WHEN se leen sus bytes
- THEN no contienen ese texto ni ninguna ruta

**Tests (fallan hoy):** `a_full_memo_embeds_nothing_and_stores_the_same_vector_bytes`, `an_empty_memo_embeds_in_the_batches_production_does`, `a_changed_doc_embeds_only_its_new_texts`, `what_a_run_adds_is_not_used_until_the_next_run`, `questions_never_go_through_the_memo`, `a_vector_the_embedder_returns_unusable_is_not_stored` y `the_memo_file_holds_no_text` (decorador con embedder falso que anota llamadas; carpeta con `test_dir()`); `the_memo_folder_sits_next_to_the_results_folder` y `no_cache_without_docs_is_an_error` en `crates/clawg-eval/src/bin/rag-eval/cli.rs`; `no_cache_leaves_the_folder_byte_for_byte` (compara un resumen de bytes antes y después, y la ausencia de la carpeta) y `docs_mode_with_the_fake_embedder_never_touches_the_memo` en `crates/clawg-eval/src/bin/rag-eval/docs.rs`, con la carpeta de la memoria del `Cli` en un `TempPath`: el binario resuelve siempre la del workspace y ninguna bandera la cambia.

### IC5. Una memoria que no se puede leer

Una memoria truncada, con una cabecera que no es la esperada, de una versión que no se conoce o ilegible MUST ignorarse con un aviso por stderr que nombre el archivo y la causa; la corrida MUST embeber todo, como con la memoria vacía, y `index.memo.used` MUST valer `false`. Nunca MUST usarse un vector a medias: un archivo que no se lee entero no aporta ninguno. Una corrida que ignoró una memoria ilegible MUST NOT salir con error por eso. Escribir la memoria MUST NOT dejar nunca un archivo a medias: se escribe a un archivo temporal de la misma carpeta y se renombra, y si la escritura falla la memoria anterior queda como estaba y la corrida avisa sin fallar. En el caso de una memoria ilegible, lo que la corrida embebió MUST reemplazarla al terminar.

#### Escenario: truncada
- GIVEN una memoria a la que se le cortó el final a la mitad de un vector
- WHEN se indexa
- THEN se avisa, se embebe todo y `from_memo` vale 0; ningún pasaje usa un vector parcial

#### Escenario: cabecera desconocida o ilegible
- GIVEN un archivo que no empieza con la cabecera de la memoria, o uno de una versión posterior, o uno que no se puede abrir
- WHEN se indexa
- THEN se avisa con el nombre del archivo y se embebe todo

#### Escenario: la escritura se corta
- GIVEN una escritura que falla antes de terminar
- WHEN la corrida cierra
- THEN la memoria anterior sigue intacta, no queda un archivo parcial con el nombre de la memoria, y se avisa

**Tests (fallan hoy):** `a_truncated_memo_is_ignored_with_a_warning`, `a_memo_with_an_unknown_header_is_ignored`, `an_unreadable_memo_is_ignored` y `a_failed_write_leaves_the_previous_memo` (la escritura se parte con un escritor que falla; sin tocar el modelo) en el módulo de la memoria.

### IC6. Los tokens

`rag-eval --real --docs` MUST contar los tokens del corpus que indexó, después de indexar, y el archivo de resultados MUST llevarlos en `index.tokens`. Se cuentan sobre el texto de cada pasaje escrito, con el prefijo de pasaje y como lo embebe el modelo: tokenizado y recortado a la ventana de 512 (`window_encodings`, `crates/clawg-core/src/memory/embedder.rs:240-255`). Los campos son:

| Campo | Qué es |
|---|---|
| `real` | La suma de los tokens de cada pasaje dentro de la ventana, sin relleno |
| `padded` | Por doc, en tandas de `EMBED_BATCH` (`embedder.rs:184`) y en el orden del doc, filas × el más largo de la tanda, sumado: lo que corre el modelo sin memoria |
| `padded_ran` | La suma de `embedder.batch_tokens` con `role=passage` de la corrida (`embedder.rs:509`): lo que corrió de verdad |
| `padded_sorted_in_doc` | El cálculo de `padded` con los pasajes de cada doc ordenados de más tokens a menos, los empates en el orden del doc, como `longest_first` (`crates/clawg-core/src/memory/portability/import.rs:2537`), que es el orden que ensayaría A1 de docs-index-speed |
| `padded_sorted_all` | El mismo cálculo con todos los pasajes de la corrida en ese orden, los empates en el orden de la corrida, cortados en tandas de `EMBED_BATCH` |
| `count_seconds` | Cuánto llevó contar, tokenizando como el embedder: la estimación de lo que cuesta tokenizar dentro de `embed` |

El conteo MUST ser puro sobre largos ya medidos (sin tokenizer, sin disco): `padded`, `padded_sorted_in_doc` y `padded_sorted_all` se calculan sobre listas de largos por doc. Un doc con `EMBED_BATCH` pasajes o menos corre una sola tanda de ese tamaño, así que su relleno es el mismo en cualquier orden. Con una tanda incompleta, ordenar puede rellenar más: los tres campos MUST NOT acotarse entre sí, y los cocientes que lee docs-index-speed pueden salir negativos.

`EvalEmbedder::real` MUST conservar el recorder que hoy pierde (`Arc::default()`, `eval_api.rs:569`) y `padded_ran` MUST leerse de él. `embedder.batch_tokens` es un gauge (`crates/clawg-core/src/metrics/recorder.rs:50`), que `Recorder::run()` (`recorder.rs:172`) no devuelve, así que se lee su `sum` de la ventana en curso con `role=passage`, la que en la evaluación nadie vacía. Nada de esto escribe en un `metrics.db`.

Una corrida en la que ningún pasaje salió de la memoria (`from_memo = 0`: con `--no-cache`, con la memoria vacía o con una que no tenía ninguno de sus textos) MUST avisar por stderr cuando `padded != padded_ran`: cada pasaje corrió en las tandas de producción y los dos números tienen que coincidir. Con `from_memo > 0` el aviso no sale, porque `padded_ran` cuenta sólo lo que se embebió.

#### Escenario: relleno de una tanda
- GIVEN un doc de 3 pasajes de 10, 40 y 25 tokens
- WHEN se calcula
- THEN `real` es 75, `padded` es 3 × 40 = 120 y `padded_sorted_in_doc` es 120: es una sola tanda

#### Escenario: más de una tanda
- GIVEN un doc de 20 pasajes (`EMBED_BATCH` = 16, `embedder.rs:184`) cuyos largos se alternan entre 10 y 100 tokens, empezando por 10
- WHEN se calcula
- THEN `real` es 1100, `padded` es 16 × 100 + 4 × 100 = 2000 y `padded_sorted_in_doc` es 16 × 100 + 4 × 10 = 1640

#### Escenario: ordenar puede rellenar más
- GIVEN un doc de 17 pasajes: 16 de 10 tokens y, al final, uno de 100
- WHEN se calcula
- THEN `padded` es 16 × 10 + 1 × 100 = 260 y `padded_sorted_in_doc` es 16 × 100 + 1 × 10 = 1610

#### Escenario: ordenar entre docs
- GIVEN tres docs: 3 pasajes de 10 tokens, 3 de 10 y 10 de 100
- WHEN se calcula
- THEN `padded` y `padded_sorted_in_doc` son 1060 y `padded_sorted_all` es 16 × 100 = 1600, una sola tanda con el más largo; si todos los pasajes miden lo mismo, los tres valen los pasajes por ese largo

#### Escenario: ventana de 512
- GIVEN un pasaje de 700 tokens
- WHEN se cuenta
- THEN aporta a lo sumo lo que la ventana deja (512, con los tokens especiales)

#### Escenario: sin pasajes
- GIVEN un corpus sin pasajes
- WHEN se calcula
- THEN todo vale 0

#### Escenario: el aviso
- GIVEN una corrida con `from_memo = 0`, `padded = 1000` y `padded_ran = 1200`
- WHEN termina
- THEN stderr avisa que difieren y el archivo igual los lleva

#### Escenario: el recorder
- GIVEN `EvalEmbedder` con un embedder de prueba que anota un gauge `embedder.batch_tokens` con `role=passage` y otro con `role=query`
- WHEN se pide `padded_ran`
- THEN suma sólo el de `passage`

**Tests (fallan hoy):** `padding_of_a_batch_is_rows_times_the_longest`, `a_doc_over_embed_batch_runs_several_batches`, `one_batch_pads_the_same_in_any_order`, `sorting_can_pad_more_when_the_last_batch_is_short`, `sorting_keeps_ties_in_their_order`, `sorting_across_docs_batches_the_whole_run`, `an_empty_corpus_has_no_tokens`, `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` (puros, sin modelo) en el módulo de tokens de `rag-eval`; `padded_ran_sums_only_passage_batches` y `the_real_embedder_keeps_its_recorder` en `crates/clawg-core/src/eval_api.rs`, porque el recorder no sale de `eval_api` si la construcción se puede probar sin modelo (si no, se prueba con un constructor interno que recibe el recorder).

### IC7. El reparto por área

`rag-eval --docs` MUST repartir lo que mide en tres áreas según la ruta relativa de cada doc: `archive` si empieza con `openspec/changes/archive/`; si no, `code` para `DocKind::Code` y `docs` para el resto (Markdown y manifiestos). La decisión MUST ser pura (ruta y tipo) y MUST vivir en `rag-eval`: producción MUST NOT nombrar `archive/` ni recibir un clasificador del lado de la evaluación que la cambie, y el refresco MUST NOT filtrar por área. El prefijo se compara con la ruta relativa con `/`, y `openspec/changes/archived/x.md` y `openspec/changes/archive.md` no cuentan como archivo.

`index.by_area.{docs,code,archive}` MUST llevar, para cada área: `docs` (docs reindexados), `passages`, `multi_batch_docs` (docs con más de `EMBED_BATCH` pasajes), `phases.{read,mask,split,lookup,embed,write}` (la suma de los docs del área) y `tokens.{real,padded,multi_batch_real,multi_batch_padded}`, donde `multi_batch_*` suman sólo los docs con más de `EMBED_BATCH` pasajes. `scan` no se reparte: es del recorrido entero. Tampoco la remoción de los docs que ya no están, que IC1 cuenta en `write` y no es de ningún doc. La suma de las tres áreas MUST dar los totales de `index.phases` menos `scan` (con `write` más la remoción igual a `index.phases.write`), `corpus.passages` e `index.tokens.{real,padded}`.

#### Escenario: la ruta decide
- GIVEN `openspec/changes/archive/2026-01-01-x/proposal.md`, `docs/a.md`, `Cargo.toml` y `crates/x/src/lib.rs`
- WHEN se clasifican
- THEN son `archive`, `docs`, `docs` y `code`

#### Escenario: un código bajo archive
- GIVEN `openspec/changes/archive/x/script.rs`
- WHEN se clasifica
- THEN es `archive`

#### Escenario: prefijos parecidos
- GIVEN `openspec/changes/archived/x.md` y `openspec/changes/archive.md`
- WHEN se clasifican
- THEN son `docs`

#### Escenario: las áreas suman el total
- GIVEN tiempos y largos por doc de las tres áreas
- WHEN se reparten
- THEN la suma de cada fase, de los pasajes y de los tokens por área da el total

#### Escenario: la remoción no es de ningún área
- GIVEN un refresco que borró docs que ya no están, con una remoción mayor que cero
- WHEN se reparte
- THEN el `write` de las áreas más la remoción da `index.phases.write`

#### Escenario: una tanda múltiple
- GIVEN un doc de 20 pasajes y uno de 3 en `docs`
- WHEN se reparten
- THEN `multi_batch_docs` vale 1 y `multi_batch_real` y `multi_batch_padded` suman sólo el de 20

**Tests (fallan hoy):** `area_of_a_path_is_decided_by_its_prefix_and_kind` (tabla), `areas_add_up_to_the_totals`, `the_removal_of_docs_is_in_the_total_write_and_in_no_area` y `only_docs_over_a_batch_count_as_multi_batch` en el módulo de áreas de `rag-eval`; un test que busca el texto `archive/` en las fuentes del refresco, `crates/clawg-core/src/memory/project_docs/`, y falla si aparece (`production_never_names_the_archive`).

### IC8. El bloque `index` del archivo de resultados y el informe

El archivo de `rag-eval --real --docs` (DQ3 de docs-search-quality; hoy `DocsResults`, `crates/clawg-eval/src/bin/rag-eval/docs.rs:302`) MUST ganar un bloque `index` con:

- `build`: `debug` o `release` (`cfg!(debug_assertions)`);
- `machine`: `os`, `arch`, `avx2`, `avx512f`, `fma` y `threads`, los hilos de la sesión (`std::thread::available_parallelism`, `embedder.rs:536`);
- `phases`: `scan`, `read`, `mask`, `split`, `lookup`, `embed` y `write`, en segundos, sumados sobre el refresco (IC1);
- `memo`: `used` (si se leyó una memoria utilizable), `from_memo` (pasajes que salieron de ella), `embedded` (pasajes que se embebieron; `from_memo + embedded` es `corpus.passages`) y `seconds` (lo que llevó leer y escribir la memoria, **fuera** de `index_seconds`);
- `tokens` (IC6) y `by_area` (IC7).

`index_seconds` y `corpus` MUST NOT cambiar de nombre, tipo ni de lo que miden. Ningún campo de `index` MUST llevar texto de un pasaje, de una pregunta ni una ruta; el bloque sólo lleva números, banderas y los nombres `debug`/`release`, el sistema y la arquitectura. Como la memoria no cuenta en `index_seconds`, `phases` MUST sumar a lo más `index_seconds` más el error de medir; los límites de esa suma son de las corridas de cierre y no de esta spec.

El informe por stdout MUST nombrar el archivo de la memoria (o decir que no se usó, con `--no-cache`) y mostrar `index.memo` (`from_memo`, `embedded`, `seconds`) y las fases. Con `--fake`, `rag-eval` MUST seguir sin escribir archivo (`docs.rs:591-600`), el informe no muestra tokens ni memoria, y no se cuenta nada con el tokenizer.

`rag-eval --diff` (DQ4) MUST leer un archivo sin bloque `index` contra uno que lo tiene, en cualquier orden, y seguir sin comparar tiempos: `DocsFile` (`docs.rs:386-391`) no usa `deny_unknown_fields` y no gana campos obligatorios.

#### Escenario: el bloque
- GIVEN una corrida real con memoria que sacó 30 de 100 pasajes de ella
- WHEN se arma el archivo
- THEN `index.memo` es `{ used: true, from_memo: 30, embedded: 70, seconds: … }`, `corpus.passages` es 100 y `index_seconds` no incluye `memo.seconds`

#### Escenario: sin memoria
- GIVEN una corrida con `--no-cache`
- WHEN se arma el archivo
- THEN `index.memo.used` es `false`, `from_memo` vale 0, `embedded` es `corpus.passages` y `seconds` vale 0

#### Escenario: sin texto ni rutas
- GIVEN un archivo armado con docs de rutas y textos reconocibles
- WHEN se serializa
- THEN ninguna cadena del bloque `index` contiene una ruta del corpus ni el texto de un pasaje

#### Escenario: lo de antes no se mueve
- GIVEN los resultados del mismo caso con y sin el cambio
- WHEN se comparan `index_seconds`, `corpus`, `aggregate` y `per_case`
- THEN tienen los mismos nombres y tipos

#### Escenario: `--diff` de un archivo viejo contra uno nuevo
- GIVEN A sin bloque `index` y B con él
- WHEN corre `rag-eval --diff A B` y `--diff B A`
- THEN compara las preguntas como siempre, no informa ningún tiempo y no falla

#### Escenario: el informe de stdout
- GIVEN una corrida real con memoria
- WHEN se imprime el informe
- THEN nombra el archivo de la memoria y muestra `from_memo`, `embedded` y las fases; con `--fake` no muestra tokens ni memoria

**Tests (fallan hoy):** `the_index_block_reports_the_memo_and_the_phases`, `the_index_block_holds_no_text_and_no_path` (serializa y busca), `index_seconds_and_corpus_keep_their_shape`, `a_results_file_without_an_index_block_still_diffs` y `a_results_file_with_an_index_block_still_diffs_against_an_old_one` en `crates/clawg-eval/src/bin/rag-eval/docs.rs`; `the_docs_report_names_the_memo_file` y `a_fake_run_reports_no_tokens_and_no_memo` (cadenas del informe); `docs_mode_with_the_fake_embedder_writes_no_file` en `crates/clawg-eval/tests/docs_mode.rs`.

## Cierre

Las corridas D (build debug, memoria vacía) y R (build release en `target-test`, `--no-cache`) del cambio necesitan el modelo real del perfil dev y una autorización explícita; no son requisitos de esta spec y se anotan en `verify.md` según los criterios de éxito de la [propuesta](proposal.md) (la suma de `index.phases` frente a `index_seconds`, `tokens.padded == tokens.padded_ran` y los campos que leen las puertas de `docs-index-speed`). Las dos salen con `index.memo.used = false` y `from_memo = 0`: D porque la carpeta de la memoria está vacía (y la llena al terminar), R por `--no-cache`. En D el `embed` incluye calcular las claves; en R no.
