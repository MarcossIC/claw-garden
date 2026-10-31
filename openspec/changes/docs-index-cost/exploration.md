# docs-index-cost — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (soporte de medición) | S–M | nada para el paso 1 y 2; la medición con el modelo real necesita autorización y el perfil dev. Se coordina con docs-search-quality (20), cuyos ocho `--real --docs` son el beneficiario, y con 95 (`docs-index-speed`), que recibe lo que esta medición decida | exploración cerrada; las decisiones abiertas se cierran en la propuesta |

**Tesis:** `rag-eval --real --docs` re-embebe el corpus entero en cada corrida (2.407 s en debug para 17.584 pasajes) aunque casi todos los textos que embebe sean los mismos que la corrida anterior, y no dice en qué se va el tiempo. Dos cosas, en este orden: (1) **instrumentar** el indexado para que una corrida diga dónde se gasta (fases, tokens reales contra tokens con relleno, docs contra código, archivo contra el resto) y (2) **dejar de recalcular lo que no cambió**, con una memoria de vectores por contrato y texto exacto que vive fuera del índice. Todo lo que cambiaría el rendimiento de verdad (orden por largo, solapar partido y embebido, release, caches de sentencias) queda como lista para 95, decidida por la medición del punto 1 y no antes.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `EvalDocIndex::open` abre un store nuevo cada corrida, así que cada corrida embebe todo | **Cierta.** `docs.rs:627` crea `rag-eval-docs-<uuid>` en el temp del sistema, `run_in` lo borra al terminar (`docs.rs:633-645`) y `EvalDocIndex::open` abre `MemoryStore` ahí (`eval_api.rs:1270-1279`). Un store vacío no tiene huellas, así que `plan_refresh` indexa todo y `stored_vectors` no se pregunta (`mod.rs:460-464`, un doc nuevo no pregunta). |
| El refresco de producción ya conserva docs sin cambios y reutiliza vectores | **Cierta, y por eso no sirve sola para la evaluación.** `plan_refresh` decide por `content_hash` y `model_id` (`mod.rs:124`; las citas de docs-search-quality que decían `:104-117` se corrieron). `reuse_plan` y `stored_vectors` reutilizan por el `indexed_text` completo cuando el doc fue embebido con ese `model_id` (`mod.rs:500-554`). Es el RM5 de `specs/project-docs`. Pero opera **dentro de un store que persiste**; el de la evaluación no. |
| El refresco embebe doc por doc, en el orden del recorrido, tandas de 16 rellenadas al más largo, sin ordenar por largo | **Cierta.** Un bucle secuencial por doc (`mod.rs:447-482`): leer, enmascarar, partir (`:457-458`), preguntar vectores guardados, `embed_passages` con **todos** los pasajes del doc en una llamada (`:558-582`). `E5Embedder::embed_batch` los corta en `EMBED_BATCH = 16` (`embedder.rs:184`, `:436`), cada tanda se rellena al más largo con `pad_right` (`:259`) y se mide con `embedder.batch_tokens` (`:509`). No hay orden por largo ni cruce entre docs: un doc con 3 pasajes corre una tanda de 3. |
| Un build debug sólo frena el lado Rust y el C empaquetado, no ONNX Runtime | **No verificada con una medición; plausible, no probada.** ONNX Runtime viene precompilado (`ort` con `download-binaries`, `Cargo.toml:44`). SQLite (`rusqlite` con `bundled`, `Cargo.toml:33`), `sqlite-vec` y `tree-sitter` son C compilado por `cc` con el nivel del perfil, y `tokenizers` es Rust. El `Cargo.toml` raíz sólo tiene `[profile.test]` (`:72`, sólo `debug = "line-tables-only"`): no hay `[profile.dev.package."*"]`. Cuánto pesa cada parte **es justo lo que hay que medir**; no se decide por la intuición. |
| La importación ya mide por fase y embebe de más largo a más corto | **Cierta, con una diferencia que importa.** `portability/timing.rs` deja una línea `info` por fase y `import.rs` ordena (`longest_first`, `embedding_order`, `EMBED_BATCH` 32 propio, `import.rs:75`). 85 midió que ese orden bajó `embed` un 30,4 % y que **no movió ni un bit** en la máquina medida (5.000 filas, cuatro órdenes, 65 comparadas contra embebidas solas: `archive/2026-10-27-embed-throughput/proposal.md:16`, `:31`). El refresco de docs no tiene nada equivalente, y además no puede ordenar entre docs sin dejar de tener "un doc en memoria a la vez" (`mod.rs:450-452`). |
| El recorder ya tiene lo necesario | **A medias.** Hay `embedder.batch`, `embedder.batch_tokens` (con relleno), `embedder.wait`, `embedder.load`, `docs.refresh.duration` y `docs.refresh.files` (`embedder.rs:441`, `:509`, `:643-644`; `api.rs:3016-3025`). **Faltan** los tokens reales (no hay con qué calcular el relleno), el tiempo de tokenizar, de correr la sesión y de pooling por separado, y el reparto de un refresco por fase. Y **la evaluación no puede leer ninguno**: `EvalEmbedder::real` le da a `E5Embedder::from_parts` un `Arc::default()` que nadie conserva (`eval_api.rs:569`). |
| 479 de ~730 archivos indexados están bajo `openspec/changes/archive/` | **Cierta.** `git ls-files` da 479 `.md` bajo `archive/` de 534 `.md` y 195 `.rs/.py/.ts` rastreados. Los 727 docs de `verify.md:11` son esos 534 `.md` y el código más manifiestos, menos lo ignorado. Dos tercios del corpus son historia que ninguna de las 40 preguntas del golden set tiene como respuesta. Quitarlos del corpus **cambiaría qué se mide** (y su dificultad); queda como decisión de 20, no de este cambio. |
| Los vectores no dependen de con quién comparten tanda | **Cierta en la máquina medida, no garantizada entre máquinas.** 85 midió igualdad bit a bit entre órdenes y contra embebidos solos. Pero `embedder.rs:294-298` sólo garantiza el orden de las operaciones de pooling, y `e5_golden.rs:24-45` documenta que otra CPU, incluso con las mismas características pero otro sistema, movió los últimos bits. Un vector reutilizado de otra corrida es exacto **en la misma máquina**; entre máquinas hay que invalidar (ver enfoque B). |
| El hilo del embedder cambia los bits | **No en la máquina medida.** `session_builder` fija un hilo por núcleo (`embedder.rs:534-545`); 62 cita que 46 midió que 1, 2 y 4 hilos dan los mismos bits. No se toca aquí. |

## Problema (verificado contra el código)

### Cómo se indexa hoy en la evaluación

`measure_with` llama `index.index(&docs_root, left_out)` una vez y mide `took` alrededor (`docs.rs:709-717`). `EvalDocIndex::index` es `project_docs::refresh` sobre un store vacío (`eval_api.rs:1283-1294`). Por doc: `corpus::read`, `redact`, `passages` (parte con el tokenizer real, en el hilo asíncrono), `embed_passages`, `replace_doc` (una transacción: borra, y por pasaje tres `INSERT` en `project_doc_chunk`, `project_doc_vectors` y `project_doc_fts`, `mod.rs:637-670`). Todo secuencial: mientras se parte o se escribe, el hilo del modelo espera, y viceversa.

El texto que se embebe es `prefijo + breadcrumb + "\n\n" + cuerpo` (`chunk.rs:28-30`, prefijo `"query: "` por `embedder.rs:141`). Un mismo texto sale igual mientras no cambien el chunker, el breadcrumb ni el prefijo.

### Lo que no se sabe

`verify.md:11` tiene un solo número, 2.407,1 s, de stdout. El reporte de `rag-eval --docs` imprime pasajes y segundos totales (`docs.rs:227`, `index_seconds` en `DocsResults`, `docs.rs:313`, `:349`) y nada más: ni cuántos de esos segundos son el modelo, ni cuántos tokens son relleno, ni cuánto cuesta el código frente a la prosa. Sin eso, 95 decidiría a ciegas, y cualquier "arreglo de velocidad" de esta exploración sería una hipótesis.

### Qué se repite

docs-search-quality planea unas ocho corridas reales (línea base, 2a a 2e, paso 3, paso 4; `tasks.md:27-81`). Los pasos 2 y 3 **no cambian ningún vector**: el 2 sólo toca la consulta léxica y el 3 deja pinchado `the_embedded_and_shown_text_do_not_change` (tarea 12, `tasks.md:68`). Sólo el paso 4 (prefijo) cambia todos los vectores. Entre corridas, además, el corpus cambia poco: cada commit agrega o edita unos pocos docs. Del orden de 6 de las 8 corridas podrían ser casi todo reutilización.

## Lo que ya existe y se reutiliza

- `Embedder` ya expone `model_id()`, `dimensions()`, `query_prefix()` y `passage_prefix()` (`embedder.rs:63-95`, `:468-482`): lo necesario para construir una clave de contrato sin tocar el trait.
- `EvalContract` (`eval_api.rs:1381-1386`: modelo, dimensión y los dos prefijos) ya es el contrato de vectores que usa `--background`. Falta el hash del `model.onnx` (`model_download::MODEL_SHA256`, `model_download.rs:63`), porque `model_id` nombra el modelo y el pooling, no el archivo.
- `EvalEmbedder(Arc<dyn Embedder>)` (`eval_api.rs:546`) envuelve al embedder; un decorador cabe sin tocar producción.
- `sha256_hex` (`eval_api.rs:1336`) ya está compartido y `sha2` es dependencia del núcleo.
- `RefreshReport` ya cuenta `reused` (`mod.rs:86`) y el log del daemon lo imprime (`refresh_line`, `api.rs:2989-2994`); la evaluación lo descarta (`EvalDocReport` sólo tiene `docs`, `passages`, `skipped`, `eval_api.rs:1251-1255`).
- `docs/eval/**` ya es ignorado por git (`.gitignore`), tiene `results/` para los resultados y el golden set, y `default_out_dir` ya resuelve ahí sin depender del CWD (`cli.rs:291-298`). El caché va en ese árbol, nunca en el temp del sistema.
- El archivo de resultados ya es tolerante: `DocsFile` lee sólo `golden_sha256`, `corpus.left_out` y `per_case` y no usa `deny_unknown_fields` (`docs.rs:386-396`, `:493`), así que un bloque nuevo no rompe `--diff` de archivos viejos.
- El patrón de la importación: tiempo por fase en `info` (`portability/timing.rs`), contado, sin textos.

## Enfoques posibles

### A. Un store de evaluación que persiste entre corridas (reutiliza el refresco de producción)

`EvalDocIndex::open` sobre un directorio fijo bajo `docs/eval/`; la segunda corrida encuentra huellas y sólo reindexa lo que cambió.

- A favor: lo que ya existe, casi sin código nuevo. Se salta también leer, partir y escribir lo que no cambió, no sólo el modelo.
- En contra, y es de fondo: **la evaluación pasa a depender de la historia.** `plan_refresh` conserva un doc si su `content_hash` y su `model_id` no cambiaron (`mod.rs:124`); no mira el chunker, el prefijo ni el texto léxico. Un cambio del paso 3 (formas partidas en `project_doc_fts`) o del paso 4 (prefijo) dejaría el índice viejo como "al día" (la premisa 6 de docs-search-quality, ahora en la herramienta que mide). `replace_doc` sólo reescribe FTS cuando reemplaza el doc (`mod.rs:656-659`). Un resultado que depende de qué corrió antes no se puede comparar con `--diff`. Se arregla con una huella que cubra todo (lo que el paso 3 planea sólo para lo léxico, `design.md:51-55`), que es código de producción y cae dentro de 20.
- Además: el store persistente guarda texto de docs fuera del temp, hay que cerrarlo bien para que la corrida siguiente lo abra (`StoreHolder::Eval`, lock exclusivo), y una corrida interrumpida a la mitad deja un índice parcial.
- Esfuerzo: bajo si se confía en la huella; alto si se la arregla bien. **No se recomienda.**

### B. Una memoria de vectores por contrato y texto exacto (recomendado)

Un decorador de `Embedder` en `eval_api` (por ejemplo `EvalEmbedder::memoized(path)`), usado sólo por `--real --docs`. La clave es el hash de `(contrato, rol, texto con prefijo)`, donde el contrato es `model_id`, `dimensions`, `query_prefix`, `passage_prefix` y el SHA-256 del `model.onnx`. `embed_batch` mira cada texto, embebe sólo los que faltan (en una sola llamada al embedder de verdad, con el orden relativo de los faltantes) y devuelve todos en el orden pedido. El resto del refresco (leer, partir, escribir) corre completo, igual que en producción, sobre un store fresco.

- A favor:
  - **Correcta por construcción ante cualquier cambio de lo que se embebe**: cambiar el chunker cambia los textos y falla la clave; cambiar el prefijo (paso 4) cambia el contrato y la clave; cambiar el modelo, igual. No hace falta recordar invalidar nada.
  - El índice se sigue construyendo cada vez con el código del commit, así que lo que se mide es el pipeline de ese commit. Una mala corrida no deja estado sobre el que se mida la siguiente.
  - No toca producción: sin migración, sin cambio de esquema, sin tocar `plan_refresh`.
  - Los pasos 2 y 3 y los reintentos pasan a depender sólo de partir, escribir y buscar.
- En contra:
  - Sigue pagando partir, redactar y escribir (SQLite, FTS, `sqlite-vec`) en cada corrida. Si esas fases son lo grande en debug, el beneficio es menor de lo esperado; **la medición del punto 1 lo dice, y por eso el orden es medir primero y memoizar después, en el mismo cambio pero en esa secuencia.**
  - Un archivo de ~27 MB por contrato: 17.584 pasajes × (clave de 32 bytes + 384 × 4 bytes) ≈ 27,6 MB, como cota superior sin repetidos.
  - La reutilización es exacta bit a bit sólo en la máquina que produjo los vectores (premisa de los bits). La memoria registra en su cabecera el sistema, la arquitectura y las características de CPU como `e5_golden` (`e5_golden.rs:24-45`) y se ignora si no coinciden.
- Esfuerzo: medio. Piezas puras (clave, formato de archivo, decisión de qué falta) y un decorador delgado.

### C. Un híbrido: store persistente más huella de contrato completa

Como A pero con un `docs_contract` que cubra chunker, prefijo y texto léxico. Es lo más rápido (se salta todo) y lo más invasivo: entra en producción, toca la huella de `project_doc_file`, y pisa el diseño de docs-search-quality D4. Se descarta aquí y se deja anotado: si 20 construye `docs_contract`, queda a la vista un camino para que la evaluación persista su store sin memoria aparte. No hay que duplicar: **B es independiente de esa huella.**

### D. Medir con un build release

La causa más probable de que 2.407 s sea tanto es el nivel de optimización del lado Rust y C (premisa sin medir). Un release ya está previsto como forma de medir (`load-eval`, `CLAUDE.md` «Testing»), con autorización. No arregla "no recalcular", pero separa lo que es del build de lo que es del pipeline: **una corrida de `--real --docs` en release, con la instrumentación del punto 1, es la medición que decide qué hace 95.** Queda como decisión abierta (necesita autorización explícita del usuario; no se corre aquí).

### E. Instrumentar (siempre; es el paso 1)

Dónde vive cada medición, sin tocar nada guardado:

| Qué | Dónde | Notas |
|---|---|---|
| Tiempo por fase de un refresco: recorrido y hash, leer, enmascarar, partir, vectores guardados, embeber (esperando al modelo), `replace_doc` | `RefreshReport` gana un `RefreshTiming` de duraciones y contadores, sumado dentro de `refresh` (`mod.rs:422-486`) con `Instant` alrededor de cada paso | Puro: sólo `Duration` y enteros. `refresh_line` (`api.rs:2989`) lo agrega a la línea de log del daemon; `record_refresh` (`:3016`) puede emitir un `docs.refresh.phase` por fase. Sin rutas ni textos. |
| Docs contra código, archivo contra el resto | El mismo `RefreshTiming` por clase (`DocKind::Markdown`, `Manifest`, `Code`) y por "bajo `openspec/changes/archive/`" calculado desde la ruta relativa **sólo en la evaluación** | La clasificación por ruta de archivo vive en `rag-eval`, no en el núcleo: producción no tiene por qué conocer `archive/`. Se pasa un cierre o un conjunto de prefijos a `EvalDocIndex::index`. |
| Tokens reales contra tokens con relleno | Nueva serie `embedder.tokens` (real, por rol) junto a `embedder.batch_tokens` (`embedder.rs:509`): `encodings.iter().map(Encoding::len).sum()` en `Inference::embed` (`:502-526`) | La razón es el desperdicio de relleno. Sirve también en producción. Cambia el contrato de `specs/metrics` (una serie más): decisión abierta. |
| Dentro de la llamada: espera, tokenizar (`window_encodings`), correr la sesión, pooling | `embedder.wait` ya existe (`:643-644`); nuevas `embedder.tokenize`, `embedder.run` y `embedder.pool` (µs), observadas en `Inference::embed` | Más series, mismas reglas (buckets fijos por unidad, sin etiquetas libres). Alternativa sin tocar métricas: sólo para la evaluación, que `EvalEmbedder::real` conserve el `Arc<Recorder>` y exponga `run()`. |
| Que la evaluación pueda leerlas | `EvalEmbedder::real` deja de pasar `Arc::default()` (`eval_api.rs:569`) y expone un resumen de las series del run (`Recorder::run()`, `recorder.rs:172`) | Mínimo para que cualquiera de las dos anteriores llegue al informe. |
| Dónde se informa | Un bloque nuevo `index` en `DocsResults` (`docs.rs:302-316`) y su línea en `render`: `{seconds, phases{…}, texts_embedded, texts_from_memo, tokens_real, tokens_padded, by_kind{…}, archive{…}}` | Los campos viejos (`index_seconds`, `corpus`) no se mueven, `DocsFile` no usa `deny_unknown_fields`, así que `--diff` lee archivos viejos y nuevos. Un test fija que un archivo sin el bloque sigue diffeando. |

Sin textos ni rutas más allá de lo que el informe ya imprime (`left_out`, la ruta de un doc fallado); las rutas de `archive/` se cuentan, no se nombran.

## Residuos puros sin intercambio (lo obvio)

Se separa lo que se justifica sin medir de lo que no.

**Obvio, entra:**
1. El informe de la evaluación tira el `reused` que el refresco ya calcula (`EvalDocReport` no lo lleva). Pasarlo es gratis y es necesario para que un archivo de resultados diga cuánto se reutilizó (punto de comparabilidad).
2. El store y el directorio de trabajo de la evaluación no deben ser la causa de nada: sin cambios.

**Parece desperdicio pero NO es obvio (pasa a la lista de 95):**
- *El mismo texto se tokeniza dos veces* (al partir y en `window_encodings`): no es el mismo texto. El partidor cuenta tokens del cuerpo; el embedder tokeniza `prefijo + breadcrumb + "\n\n" + cuerpo`, y el corte a 512 se aplica sobre eso (`embedder.rs:240-255`). Compartir la codificación obliga a que el partidor conozca el breadcrumb y el prefijo; es una refactorización con riesgo para el contrato. Sólo vale si `embedder.tokenize` sale grande.
- *`replace_doc` prepara tres sentencias por pasaje* (`tx.execute` sin `prepare_cached`, `mod.rs:645-659`): 81 midió ~16 % de la importación por eso. Probablemente real aquí, pero sin medición no se sabe cuánto y toca el camino del daemon.
- *Ordenar por largo dentro de la llamada de un doc* (`embed_batch`) o entre docs: 85 lo midió para la importación (1,293× tokens reales, 2,042× atención, −30,4 % de `embed`), no para docs, donde las tandas son de los pasajes de un solo archivo.
- *Solapar partido y escritura de un doc con el embebido del siguiente*: el hilo del modelo espera mientras se parte y se escribe (premisa 3). Cambia el uso de memoria (dos docs a la vez) y el orden de fallos.
- *Un perfil que optimice las dependencias en dev* (`[profile.dev.package."*"] opt-level = 3`): acelera todo build de desarrollo y CI-dev, no sólo la evaluación; sube el tiempo de compilar en frío. Es una decisión del proyecto, no un arreglo de la evaluación.
- *Excluir `archive/` del corpus*: cambia la dificultad de la medición. De 20.

## Recomendado

**E + B, en ese orden, en un solo cambio**, y D como medición de cierre si el usuario la autoriza:

1. **Instrumentar** (E): `RefreshTiming` en el refresco; tokens reales y fases del embedder; que la evaluación lea el recorder; bloque `index` en el archivo de resultados y en el informe; línea de log del daemon con las mismas fases. Primero, porque la memoria de B no se puede juzgar sin saber cuánto del tiempo es el modelo.
2. **Memoria de vectores** (B): decorador en `eval_api`, archivo en `docs/eval/cache/`, clave por contrato y texto, cabecera con la máquina, activa por defecto en `--real --docs`, con `--no-vector-memo` y `--clear-vector-memo`. El informe y el archivo de resultados dicen `texts_embedded` y `texts_from_memo`.
3. **Cerrar con una corrida real** de la línea base con la memoria vacía (la que mide fases y llena la memoria) y una segunda con la memoria llena, para dejar en `verify.md` el antes y el después y la lista de candidatos para 95 con sus números.

Lo medido decide qué entra en 95 (lista abajo). Este cambio no cambia un vector, una ranking ni un archivo guardado.

## Alcance candidato

- `RefreshReport`/`RefreshTiming` (puro), el tiempo por fase, las nuevas series del embedder (si se aprueba) y `EvalEmbedder::real` conservando su recorder.
- `EvalDocReport` con `reused` y el bloque de tiempos; `DocsResults.index`; `render`; test de compatibilidad con archivos sin el bloque.
- `VectorMemo` (formato, clave, decisión de faltantes: puros y con tests) y su decorador; flags de `rag-eval`; la ayuda de `--help`.
- Spec: `specs/eval` (el bloque nuevo, la memoria, sus invalidaciones y que no es un store), `specs/metrics` si se agregan series, `specs/project-docs` sólo para la línea del log.

## No-alcance

- Cualquier cambio de lo que se embebe, parte o guarda; `plan_refresh`, `docs_contract`, huellas (20).
- El orden por largo, el solape, `prepare_cached`, release/perfiles, compartir la tokenización, quitar `archive/` (lista para 95 / 20).
- Hilos, spinning, arena, el modelo, el tokenizer (62, 91), el banco de rendimiento (74).
- Cualquier corrida con el modelo real, release o instalación sin autorización.

## Decisiones abiertas

1. **Series nuevas en el recorder de producción** (`embedder.tokens`, `embedder.tokenize`, `embedder.run`, `embedder.pool`) o sólo en la evaluación. Recomendado: `embedder.tokens` y `embedder.run` en producción (el relleno es dato de uso real, y `specs/metrics` ya mide `batch_tokens`), el resto sólo si la medición lo pide. Cambia `specs/metrics` y la prueba que fija las series.
2. **Memoria activa por defecto** en `--real --docs` o con `--vector-memo`. Recomendado: por defecto, con el nombre del archivo en el informe: el beneficio es para quien no lo sabe.
3. **Qué hacer con vectores de otra máquina:** ignorar la memoria (recomendado) o aceptarla con tolerancia.
4. **Cuándo se purga:** cada corrida escribe sólo las claves usadas (los pasajes de commits viejos se van) en un archivo por contrato, o acumula. Recomendado: sólo las usadas, un archivo por contrato; un `--clear-vector-memo` y borrar el archivo son los dos modos de limpiar.
5. **Medición en release:** una corrida autorizada de `--real --docs` con el build release (`--target-dir target-test`) para separar build de pipeline. Recomendado: sí, una vez, con la memoria vacía.
6. **Dónde se calcula "archivo":** por prefijo de ruta pasado a `EvalDocIndex::index` (recomendado, no entra al núcleo) o dejar sólo docs/código.
7. **El log del daemon:** agregar las fases a `refresh_line` (cambia una línea que un test fija, `api.rs:3168`) o sólo la métrica `docs.refresh.phase`.

## Interacciones

| Cambio | Dónde se cruza | Cómo no duplicar |
|---|---|---|
| docs-search-quality (20), tareas 4 y 5 y pasos 2 a 4 | Es el beneficiario: sus corridas pasan de ~40 min a lo que cueste partir y escribir, salvo el paso 4 (prefijo) y la primera. Su tarea 4 anota `index_seconds` en `verify.md`; con la memoria ese número deja de ser comparable | El archivo de resultados nuevo dice cuántos textos vinieron de la memoria; `verify.md` de 20 anota la corrida con la memoria vacía como referencia de tiempo. Este cambio no toca el chunker, la huella ni la consulta. La huella `docs_contract` es suya; B no depende de ella |
| 62 embedder-footprint | Ya lista "las tandas del índice de docs se rellenan hasta el pasaje más largo sin ordenar" y el bit de `e5_golden` (`embedder-footprint/exploration.md:41`, `:89`) | Esta exploración **mide** el relleno de docs (`embedder.tokens` contra `batch_tokens`); 62 decide el orden con ese número. No se ordena aquí |
| 74 perf-bench | Un banco por versión; sus escenarios son puertas de 57 y 62 | Las fases del refresco son entrada para un escenario de docs si 74 lo adopta; este cambio no crea banco ni `diff` nuevo |
| 91 tokenizer-footprint | Memoria del tokenizer, no tiempo | `embedder.tokenize` le sirve de control (si tokenizar es poco, nada que ganar en tiempo), sin pisar su paso 0 |
| 85 (archivado) | El orden por largo ya está probado para la importación | Referencia, no código |
| 95 docs-index-speed | Recibe la lista medida | Ver abajo |

## Candidatos para 95 (dependen de la medición)

Cada uno con la medición que lo decide:

1. Ordenar por largo dentro de `embed_batch` o entre docs. Decide: razón `batch_tokens / tokens` por clase (docs, código, archivo) y el tiempo de `embedder.run`.
2. Solapar partido y escritura de un doc con el embebido del siguiente. Decide: la suma de partir + `replace_doc` frente a `embedder.run`.
3. `prepare_cached` (o una sola sentencia por lote) en `replace_doc`. Decide: la parte de `replace_doc` en el refresco y cuánto de ésta es preparar.
4. Compartir la tokenización entre el partidor y `window_encodings`. Decide: `embedder.tokenize` y el tiempo de partir.
5. Optimizar dependencias en dev, o recomendar release para evaluar. Decide: la diferencia entre la corrida debug y la release con las mismas fases.
6. Una huella de contrato completa para persistir el store de la evaluación (C). Decide: cuánto de las fases (sin el modelo) sigue pesando con la memoria llena.
7. Excluir `archive/` o muestrear su parte (con 20). Decide: el peso de `archive/` en cada fase y qué haría a la dificultad.

## Cómo se mide el éxito (umbral fijado antes)

Con el modelo real y el perfil dev, autorizado, sobre el commit del cambio, `--real --docs`, misma máquina y mismo build:

- **S1.** Una segunda corrida con la memoria llena y el mismo commit embebe **0 textos** y deja en `texts_from_memo` el total; su `Hit@1/3/5` y `MRR@10` son idénticos a los de la primera, por pregunta (`--diff` sin ninguna pregunta que se mueva).
- **S2.** Con el commit siguiente (cambia unos pocos docs), embebe sólo los pasajes de esos docs: `texts_embedded` ≤ 2 × los pasajes de los docs modificados.
- **S3.** `index_seconds` de la corrida con la memoria llena ≤ 50 % del de la corrida vacía **si** la medición dice que embeber es ≥ 60 % del tiempo; si no lo es, el resultado válido es "la memoria ahorra X % y las fases que quedan son …", que es lo que recibe 95. No se reporta como fracaso.
- **S4.** Cambiar el prefijo (el experimento del paso 4) o el modelo deja la memoria vieja sin uso y embebe todo (`texts_from_memo` = 0): prueba con un embedder falso de prefijo configurable.
- **S5.** Los archivos de resultados de antes del cambio siguen diffeando con los nuevos.
- **S6.** La suma de las fases del informe queda dentro del 5 % de `index_seconds` (así se sabe que no falta una).
- La suite y los dos clippy pasan; ningún test deja archivos en el temp.

## Riesgos

- **Memoria obsoleta que oculta un cambio** (el riesgo que más importa en un instrumento de medida): se evita con la clave por texto exacto y contrato completo; el test central siembra una memoria con el contrato anterior (prefijo distinto, otro SHA de modelo) y verifica que no se usa.
- **Bits entre máquinas:** la memoria de una máquina usada en otra mueve los últimos bits. Cabecera con el sistema y las características de CPU; ignorar si difieren.
- **Una corrida con la memoria llena no mide la velocidad del embedder**, y alguien puede leer `index_seconds` como si lo midiera. El informe y el archivo lo dicen (campos de reutilización) y `--diff` advierte cuando dos archivos tienen reutilización distinta.
- **Instrumentar cambia el costo que mide:** un `Instant` por paso y por tanda es despreciable frente a una inferencia; se comprueba en la corrida real comparando `index_seconds` con y sin el bloque (el verify de 73 dejó pendiente el sobrecoste de las series con el modelo real, y comparte la limitación).
- **Archivo grande en el repositorio de trabajo:** ~27 MB por contrato; queda bajo `docs/eval/**`, ignorado. Un contrato nuevo no borra el viejo; la limpieza es del usuario.
- **Series nuevas en producción** (si se aprueba): cada una es una serie que mantener en `specs/metrics`; se limita a las que la medición pida.
- **Un solo número de partida** (2.407 s de stdout, otra máquina y build posibles): hasta que la corrida instrumentada no exista, ninguna cifra de esta exploración sobre dónde se va el tiempo es medida. Lo es la estructura (qué se repite, qué se serializa), no las proporciones.
- **Solapamiento con 20:** si 20 cierra antes con su `docs_contract`, la evaluación podría persistir el store; B sigue siendo válida y más barata de razonar, pero conviene revisar la decisión 2 de la propuesta.

## Listo para propuesta

Sí. Hay que cerrar las siete decisiones, y el usuario tiene que autorizar la corrida real (y, si quiere D, la de release) antes del cierre. El paso E y B se pueden implementar y probar sin modelo, con el embedder falso y un embedder de prefijo configurable.
