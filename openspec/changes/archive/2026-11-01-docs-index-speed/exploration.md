# docs-index-speed — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 3 (y transversal: tiempo de las mediciones) | S–M, todo detrás de puertas | **#94 `docs-index-cost`** (mide dónde va el tiempo y quita el desperdicio obvio; no existe todavía como carpeta). Sin sus números ninguna puerta de abajo se puede evaluar. Se coordina con 62 embedder-footprint, 74 perf-bench, 91 tokenizer-footprint y 17 model-reembed (ver «Interacciones») | exploración cerrada; las decisiones abiertas se cierran en la propuesta, **después** de que #94 publique sus medidas |

**Tesis:** indexar este repo con el modelo real tardó 2407 s en un build de depuración en Windows para 727 docs y 17 584 pasajes (`openspec/changes/archive/2026-10-31-docs-search-quality/verify.md:11`). #94 averigua en qué fase se va ese tiempo y quita lo que sobra sin discusión (sobre todo re-embeber los mismos pasajes en cada corrida de eval). Este cambio guarda las mejoras **cuyo beneficio depende de esa medición** o que tienen un costo que hay que pagar a propósito: ordenar por largo, el tamaño de tanda, optimizar las dependencias del perfil de depuración, qué se indexa y el tamaño de los pasajes. Cada una es un paso con puerta, como los pasos 2 a 4 de docs-search-quality: una métrica que produce #94, con el umbral fijado antes de mirar. Si ninguna puerta abre, el cambio se cierra sin tocar código.

## Premisas del orquestador: verificadas y cuáles no

| Premisa | Resultado |
|---|---|
| El refresco embebe doc por doc, en orden de documento | **Cierta.** `refresh` recorre `plan.index` (`project_docs/mod.rs:447`); por doc: lee, enmascara, parte (`:453-458`), pide los vectores guardados (`:460-465`), llama a `embed_passages` (`:468`) y escribe con `replace_doc` (`:480`). |
| Tandas de `EMBED_BATCH` = 16, rellenadas al más largo, sin ordenar por largo | **Cierta.** `EMBED_BATCH = 16` (`embedder.rs:184`); `embed_batch` corta `texts.chunks(EMBED_BATCH)` (`:436`); `pad_right` rellena a `max` de la tanda (`:259-261`). `embed_passages` manda **todos** los pasajes de un doc en una llamada (`mod.rs:566-568`), así que la tanda es la rebanada de 16 consecutivos del doc. Ninguna ordenación en ningún punto. |
| El refresco retiene un solo doc cambiado en memoria | **Cierta y deliberada.** Comentario en `mod.rs:450-452`: se vuelve a leer en lugar de guardar el escaneo, «so a refresh holds one changed doc in memory at a time». Ordenar entre docs lo rompe (ver enfoque A2). |
| Pasajes de markdown con tope 480 y solape 64; código con 1200 caracteres | **Cierta, con un matiz que importa.** `CHUNK_MAX_TOKENS = 480`, `CHUNK_OVERLAP_TOKENS = 64` (`chunker/mod.rs:26`, `:32`) son **constantes compartidas con el chunker de chat/memorias** (`chunker/chat.rs:116`, `:141`); docs las importa (`project_docs/chunk.rs:8`, usadas en `:132`, `:137`). Cambiarlas para docs exige constantes propias, no mover éstas. `CODE_CHUNK_MAX_CHARS = 1_200` es de código solamente (`project_docs/code.rs:13`). |
| ONNX Runtime viene precompilado y optimizado; el build de depuración frena el tokenizer/splitter y las bibliotecas C | **Verosímil, no medido.** ORT se baja ya compilado de `cdn.pyke.io` (`.claude/CLAUDE.md`, sección de release); el nivel de optimización de Cargo no lo toca. Lo que sí corre en depuración: `tokenizers` (en `TokenSplitter` y en `window_encodings`, `embedder.rs:240`), el splitter por palabras (`token_splitter.rs`), `regex` de `redact`, tree-sitter y SQLite si se compilan por `cc` sin optimizar. **Cuánto pesa cada uno es justo lo que #94 tiene que dar.** |
| El `Cargo.toml` raíz no tiene `[profile.dev.package."*"]` | **Cierta.** El único perfil es `[profile.test] debug = "line-tables-only"` (`Cargo.toml:72-73`); no hay `opt-level` en ningún manifiesto (búsqueda en `Cargo.toml` y `crates/*/Cargo.toml`). |
| El import embebe de mayor a menor y afirma que el orden no cambia ningún vector | **Cierta, y medida.** `longest_first` y `embedding_order` (`portability/import.rs:2520-2541`). El archivo `embed-throughput` midió −30,4 % en la fase `embed` (1187,2 a 825,8 s; 45,8 a 65,9 filas/s) con relleno de 1,293× los tokens y 2,042× la atención (`archive/2026-10-27-embed-throughput/verify.md:30`, `:49`), y «mismo vector, bit por bit» en los cuatro órdenes sobre 5000 filas, y igual al embebido solo en 65 de ellas (`proposal.md:16`, M4). **Matiz:** es una medición de **filas de memoria sobre una CPU**; no cubre pasajes de docs ni otra huella de CPU. El mismo archivo dejó escrito que adoptar `longest_first` en el refresco de docs «es otro cambio, con su propia medición» (`design.md:54`): este es ese cambio. |
| `session_builder` dice que el número de hilos puede mover los últimos bits | **El comentario existe y la medición lo contradice.** Comentario en `embedder.rs:529-533`. embedder-idle-unload midió que 1, 2 y 4 núcleos dan los mismos bits en `e5_golden` (`archive/2026-10-17-embedder-idle-unload/verify.md:174`), en un Xeon de 4 hilos; Windows y máquinas mayores no. Es tema de 62, no de éste. |
| 479 de ~730 archivos indexados están en `openspec/changes/archive/` | **Cierta.** `rg --files openspec/changes/archive -g '*.md'` da 479; el árbol de Markdown, Rust, TOML, TS y Python da 751 en total, y la corrida indexó 727 con 2 omitidos (`verify.md:11`), cifras de la misma magnitud (la diferencia es `.gitignore` y el tope de tamaño). Casi dos de cada tres docs son archivados. |
| El golden set de docs-search-quality espera algunas copias archivadas | **Cierta.** 5 de las 51 rutas esperadas están bajo `archive/` (`crates/clawg-eval/golden/project-docs.json:44, 68, 107, 115, 128`, p. ej. `.../2026-10-12-project-identity/spec.md`). |
| Las cifras del verify (727/17 584/2407 s) valen hoy | **A medias.** Son de antes de que se ensanchara el golden set (`d810a76`) y de que `archive/` creciera (84 carpetas hoy). #94 debe repetirlas. |

## Problema (verificado contra el código)

- **El tiempo se gasta en una fase que no se conoce.** `refresh` mezcla leer, enmascarar, partir, consultar vectores guardados, embeber y escribir en un solo bucle (`mod.rs:447-482`), y el único tiempo que se registra de todo el refresco es el total. El embedder ya mide `embedder.batch_tokens` (filas × largo rellenado) y `embedder.batch` (textos) por rol (`embedder.rs:441`, `:509`) y `docs.refresh.duration` por estado (`CLAUDE.md`, métricas), pero no por fase del refresco ni la razón rellenado/real para pasajes de docs. Es lo que #94 pone.
- **El relleno de docs puede ser grande, pero no está medido.** Un doc de markdown se parte por encabezado (`chunk.rs`), así que mezcla pasajes de una línea con otros de ~480 tokens; en una tanda de 16 consecutivos el más largo manda (`embedder.rs:259-261`) y la atención crece con filas × tokens² (`embedder.rs:179-181`). El único antecedente numérico es el de filas de memoria (1,293× tokens, 2,042× atención).
- **Un doc con pocos pasajes no gana nada ordenando dentro de sí.** Si tiene 16 o menos, `embed_batch` lo manda en una sola tanda (`embedder.rs:436`) y el relleno ya es a su propio máximo. La ganancia está en docs con más de 16 pasajes, o entre docs.
- **El costo de la primera indexación en producción es otro problema, con otro presupuesto.** El daemon espera `FIRST_INDEX_WAIT` = 20 s con el índice vacío y `REFRESH_GRACE` = 3 s después (`clawg-cli/src/daemon/api.rs:3034`, `:3040`, `:3119`) y sigue en una tarea propia (`:2984`). Con el modelo real y un build de release el tiempo es otro que el de 2407 s en depuración; **no hay que optimizar para el número de depuración si en release ya cabe**. Esa es la primera puerta de todo el cambio.

## Lo que ya existe y se reutiliza

- `longest_first` (`import.rs:2537`, pura y probada) y `TokenSplitter::count_all` (conteo por grupos, `token_splitter.rs`): el orden por largo ya existe para el import.
- El banco de medición de orden del import: una prueba ignorada que compara órdenes por filas/s, por razón de relleno y por bits contra el vector embebido solo (`import.rs:5500-5597`, con `report_padding`, `orders`). Se puede reusar el patrón para pasajes de docs.
- `e5_golden` (`crates/clawg-core/tests/e5_golden.rs`): tolerancia 1e-6 fuera de la huella de la fixture; con `CLAWG_BLESS=1` reescribe la fixture. Incluye un lote de 17.
- `rag-eval --diff` y `--fail-on-regression` (primarias `recall@5` y `search.found`), y el barrido «recompilar con cada valor en un worktree descartable» (docs-search-quality).
- `plan_refresh`: un cambio de `model_id` reindexa todo (`mod.rs:104-117`); es el mecanismo para forzar un reindexado si cambia el contrato de lo que se indexa.
- `load-eval` y `transport_bar` ya usan `--release --target-dir target-test` como forma de medir en release (`.claude/CLAUDE.md`, Testing): precedente para la opción de correr la eval en release.
- `left_out` en `refresh` (`mod.rs:428`, `:405`): un mecanismo existente para dejar docs fuera del índice, usado hoy sólo por la eval.

## Enfoques y su puerta

Cada uno se mide **solo** contra el anterior. Los umbrales son **propuestos**; se fijan en la propuesta antes de mirar números. Los nombres de las métricas de #94 son los que este cambio necesita; si #94 los llama distinto, se alinean en la propuesta.

### Puerta 0 (previa a todo): ¿hay un problema en release?

Con el modelo real en un build de release, el tiempo de indexar este repo y el de la primera indexación (docs + código, sin lo archivado y con ello). **Si en release la primera indexación cabe en `FIRST_INDEX_WAIT` y la corrida de eval en depuración se arregla con el enfoque C, A y B no se hacen** (no valen su riesgo). Umbral propuesto: primera indexación de producción ≤ 20 s en una máquina de referencia; si no, A y B quedan abiertos. La dimensión «depuración» se resuelve con C o corriendo en release, no con cambios de producción.

### A. Ordenar por largo antes de las tandas

Mecanismo: tandas de pasajes de largo parecido rellenan menos; el costo de inferencia baja a razón de lo que baje el relleno (el import: −30,4 % con 2,04× de atención).

- **A1. Dentro de un doc.** `embed_passages` ordena los textos por tokens, embebe, y deshace el orden antes de `fill` (`mod.rs:514`). Cambio local y puro (índices a tandas, vectores de vuelta por índice), sin tocar la memoria. **A favor:** no rompe «un doc en memoria» ni la transacción por doc (`replace_doc`). **En contra:** sólo ayuda a docs de más de 16 pasajes; con docs chicos, nada. Contar tokens cuesta tokenizar dos veces (el embedder vuelve a tokenizar en `window_encodings`); el import midió 5,4 s en depuración para todas las filas (`archive/2026-10-27-embed-throughput/verify.md:50`), pero eso es otro corpus.
  - **Puerta A1:** la razón rellenado/real de los pasajes de docs de más de 16 pasajes, por tipo (docs, código), supera 1,3× **y** esos docs son al menos un 30 % de los tokens reales. Si no, A1 no vale su prueba.
- **A2. Entre docs (una cola con tope).** Acumular pasajes de varios docs hasta un tope de bytes o de textos, ordenar por largo, embeber, repartir. **A favor:** es donde el relleno se recupera de verdad cuando los docs son chicos. **En contra, verificado:** rompe la invariante de un doc en memoria (`mod.rs:450-452`) y la atomicidad por doc: `replace_doc` es una transacción por doc para que una búsqueda vea el doc viejo o el nuevo (`:635-636`), y un doc cuyo vector es inutilizable se omite sin parar a los demás (`Err(DocIndexError::Vector…)`, `:470-473`). Con una cola, un fallo de un pasaje tiene que apuntar a su doc, y un cierre del daemon a mitad de cola deja docs sin escribir (se reintentan; es seguro porque el índice se deriva del disco). Más memoria: el tope de la cola × tamaño de pasaje (el comentario de `embedder.rs:179-183` ya sufrió gigabytes con una sola tanda grande).
  - **Puerta A2:** pasa A1 en la medición **y** la razón rellenado/real de **todo el refresco** supera 1,5× **y** el ahorro estimado (tokens rellenados evitables × costo medido por token) es al menos un 20 % de la fase de embeddings. Si A1 ya recupera la mayoría, A2 se descarta.
- **Riesgo común, los bits.** La composición de tanda cambia la forma del tensor, y `pool_and_normalize` sólo garantiza el orden dentro de una fila (`embed-throughput/exploration.md:40`). El import midió bits idénticos sobre una CPU. Para docs: `e5_golden` (lote de 17 incluido) más una prueba de pares como la del import (`import.rs:5579-5596`), **en cada huella de CPU que se use** (Linux y Windows). Si algún vector se mueve más allá de 1e-6, o si el reindexado de un store viejo mezcla vectores de dos composiciones, se trata como cambio de contrato (ver abajo). Un reindexado que quede sin cambio en `model_id` no se reindexa solo: `reuse_plan` reusa vectores por texto (`mod.rs:500-509`), así que un doc sin cambios conserva sus vectores viejos y no hay mezcla dentro de un mismo texto.
- **Pertenece aquí**, no a 62: 62 sólo cubre hilos, espera activa y `rayon`, y excluye subir o bajar `EMBED_BATCH` (`embedder-footprint/exploration.md`, «Fuera de alcance»); su fila de README menciona «las tandas del índice de docs se rellenan hasta el pasaje más largo sin ordenar» como hallazgo, y esta es la medición que lo decide. Se avisa a 62 para que no lo tome también.

### B. Tamaño de tanda (`EMBED_BATCH`)

Mecanismo: tandas más chicas rellenan menos pero lanzan más inferencias; más grandes lo contrario, con más memoria (atención ∝ filas × tokens²). El comentario del 16 dice que a 16 «indexing runs as fast as with one big batch» (`embedder.rs:182-183`), medido con 248 pasajes de un archivo.

- **A favor:** una constante. **En contra:** `EMBED_BATCH` es de **todo** el embedder (guardados, búsqueda, import: `import.rs:75` y `project_delete.rs:655` tienen su propio 32 y usan además el 16 por dentro). Cambiarlo cambia el pico de memoria del daemon, que 62 y 91 cuidan, y vuelve a mover la composición de tanda. Con A hecho, el rellenado ya baja y el óptimo cambia; **B se mide después de A, nunca antes**.
- **Puerta B:** sólo si #94 muestra que el tiempo por pasaje cambia al menos un 10 % entre 8, 16 y 32 sobre la misma ordenación, con el pico de memoria privada (`process.private_peak_bytes`) sin subir más de un 5 % (el criterio del import). Si el óptimo está entre 8 y 32 sin ganar 10 %, no se toca. Pertenece a 62 si la decisión resulta ser sobre CPU/memoria y no sobre tiempo de indexación; se anota allí.

### C. Dependencias optimizadas en el perfil de depuración (y la eval en release)

Mecanismo: `[profile.dev.package."*"] opt-level = 3` (o 2) optimiza **las dependencias**, no el código de los crates del workspace (el selector `"*"` excluye a los miembros del workspace), así que `tokenizers`, `regex`, `tree-sitter`, `rusqlite` con SQLite y `sqlite-vec` (compilados por `cc`) pasan a ir rápidos, pero el splitter (`token_splitter.rs`), el chunker y `project_docs` propios siguen en depuración salvo que se agreguen por nombre (`[profile.dev.package.clawg-core]`). ORT no cambia: viene precompilado.

- **A favor:** no toca producción ni vectores; sólo el tiempo de **todos** los tests y evals de depuración. Una variante sin tocar `Cargo.toml`: correr `rag-eval` en release con `--release --target-dir target-test`, como `load-eval` (`.claude/CLAUDE.md`).
- **En contra, verificado:** (1) el costo de compilación de una dependencia optimizada es pagado en cada limpieza de `target-test`, y las reglas de directorios de build exigen limpiar el workspace en cada versión (`cargo clean --workspace` conserva las dependencias, lo que favorece esta opción: se compilan una vez); (2) el tamaño de `target`/`target-test` crece, y esos directorios ya llegan a decenas de GB; (3) CI corre `clippy` y la suite con `--locked` y hoy el cache de rust-cache se llena por job; un perfil nuevo cambia lo que se cachea y alarga el primer build de cada PR hasta que `cache.yml` reescriba el cache en `main`; (4) la regla del proyecto «no agregar dependencias, visibilidad o abstracciones sin un uso real» no lo prohíbe, pero un cambio de perfil afecta a todos; (5) depurar con dependencias optimizadas pierde variables y pasos; (6) el comentario de `[profile.test]` (`Cargo.toml:68-71`) muestra que el tamaño del binario de test ya es un límite (256 MiB, `BINARY_CAP`): optimizar con inlining puede cambiarlo.
- **Puerta C:** #94 reparte el tiempo de la corrida de eval por fase. Si **tokenizar + partir + enmascarar + SQLite/FTS/vec** suman al menos un 40 % del total (el resto es inferencia ORT, que C no acelera), se prueba C; se mantiene si el total de la corrida baja al menos un 25 % **y** el primer build limpio de `target-test` no sube más de un 30 % y el tamaño no más de un 20 %. Si la inferencia es más del 70 %, C no sirve y se descarta; la eval en release (sin tocar el manifiesto) se documenta como alternativa, no se implementa.
- **Pertenece aquí** (es del tiempo de los tests y evals), no a 74 perf-bench (que fija un banco de latencia por versión, no el costo de compilar).

### D. Qué se indexa: las copias archivadas de openspec

Hechos: 479 de ~730 docs son archivados (arriba). El corpus honra `.gitignore` (`corpus.rs:81`) y no tiene otra exclusión; el tope de docs es `MAX_DOCS = 2_000` y el recorrido va en orden de ruta (`corpus.rs:17`, `:68`), así que **cada cambio archivado agrega docs al costo y, si el repo llega a 2000, los últimos por ruta quedan fuera sin aviso**.

- **Opción D1: indexar todo (hoy).** El agente puede encontrar «por qué se decidió X» en una especificación archivada. Las 5 rutas archivadas del golden set (`project-docs.json:44, 68, 107, 115, 128`) existen para eso.
- **Opción D2: no indexar `archive/` (por defecto, o con un ajuste del usuario).** Cuesta casi dos tercios menos de docs. **En contra:** es una decisión de producto sobre `search_project`, no sólo de velocidad: ese índice es lo que el protocolo manda usar para «cómo funciona este proyecto» (`mcp` instrucciones), y los docs archivados repiten especificaciones vivas pero también guardan el razonamiento histórico. Además `docs-search-quality` mide con ese corpus: sacarlo cambia la línea base y las 5 preguntas archivadas pasan a fallar por construcción (hay que cambiar el golden set a la vez). Es genérico: no se puede escribir «`openspec/changes/archive`» en el código de producción, porque eso es una convención de este repo, no de todos los proyectos; la forma sería un ajuste por proyecto o un `.clawgignore`/regla de ignorar.
- **Opción D3: sólo en la eval** (`left_out`, ya existe): la corrida de `rag-eval --docs` deja fuera lo archivado salvo los docs que el golden set cita. Acelera la medición sin tocar producción. **En contra:** mide un corpus distinto del de producción, y los distractores archivados (duplicados de specs vivas) son parte de lo que hace difícil el ranking: se puede esconder una falla real.
- **Puerta D:** #94 reparte el tiempo y los tokens por área (`openspec/changes/archive` contra el resto). D2 o D3 sólo se discuten si lo archivado es al menos el 50 % del tiempo de embeddings **y** la eval de docs sin ello, con el golden set ajustado, no mueve `Hit@3` ni `MRR@10` más de un caso (comparado con `rag-eval --diff`; con ≤ 40 casos «seis casos moviéndose en un sentido» es el umbral de `.claude/CLAUDE.md`, por debajo no se distingue). **D2 además exige decisión del usuario**; D3 puede decidirse sólo por la medición.
- **Pertenece aquí.** Se cruza con docs-search-quality (20): D2/D3 cambian el corpus de su golden set y de sus pasos 2-4 (premisa «el corpus de eval contiene lo archivado»); hay que ordenarlos (ver Interacciones).

### E. Tamaño y solape de los pasajes de docs

Mecanismo: menos pasajes (tope mayor) o menos solape = menos texto que embeber; la inferencia crece superlineal con el largo de la tanda. Hoy docs usa 480/64 tokens, compartidos con el chunker de chat (`chunk.rs:8`), y 1200 caracteres para código (`code.rs:13`).

- **A favor:** el solape de 64 sobre 480 es ≈ 13 % de texto repetido en los pasajes largos. **En contra:** es un cambio de **calidad de recuperación** antes que de velocidad: pasajes mayores diluyen el vector (el modelo recorta a 512, `embedder.rs:177`), menores pierden contexto; cambia el contrato del índice (la huella de `plan_refresh` no incluye el partido, así que hay que meterlo o cambiar el `model_id`; ver docs-search-quality, «Contrato del índice de docs»), y las constantes son compartidas con memorias (hay que dar constantes propias a docs, sin visibilidad nueva más que `pub(crate)`).
- **Puerta E:** sólo si #94 muestra que el solape (o el exceso por pasaje) es al menos un 10 % de los tokens embebidos de docs **y** `rag-eval --real --docs --diff` contra la línea base no empeora `recall@5` ni `Hit@3` (p ≥ 0,05 en contra) con los casos movidos separados por tipo. Por defecto, **no entra**: es el candidato de menos ganancia por riesgo.
- **Pertenece aquí** para el tamaño; los cambios de partido por calidad (qué se corta y dónde) pertenecen a docs-search-quality.

### F. Otras cosas que muestra el código (todas necesitan medición)

| Candidato | Qué muestra el código | Qué número de #94 lo decide |
|---|---|---|
| Tokenizar en paralelo con la inferencia | Cada tanda tokeniza, rellena e infiere en el mismo hilo del modelo (`Inference::embed`, `embedder.rs:502-526`); y `partes` del refresco cuenta con `splitter` aparte. | Tokenizar+armar ≥ 10 % de la fase de embeddings (el umbral de `embed-throughput/exploration.md:76`); si no, no. Toca `EmbedRequest` y el reparto de CPU: pertenece a 62. |
| Una transacción y tres inserciones por pasaje | `replace_doc` hace `INSERT` en `project_doc_chunk`, `project_doc_vectors` y `project_doc_fts` por pasaje (`mod.rs:645-660`), con un `delete_doc` previo. | `store` > 20 % del refresco **en release**. En depuración `rusqlite`/sqlite-vec pueden verse inflados: se resuelve con C antes de tocar nada. |
| Una llamada a la conexión por consulta de vectores guardados | `stored_vectors` por doc cambiado (`mod.rs:461`), sobre la única conexión del store. | Número de ida y vuelta × latencia (`db.probe_wait`). Sólo importa con muchos docs pequeños. |
| Re-embeber lo que ya se embebió | Cada corrida de eval parte de un store vacío (`EvalDocIndex::open`, `eval_api.rs:1270`) y `reuse_plan` sólo reusa dentro del mismo doc ya guardado (`mod.rs:460-465`). | **Es de #94** (cachear vectores entre corridas de eval); no se repite aquí. |
| Pool de `rayon` de tokenizers y espera activa de ORT | `session_builder` (`embedder.rs:534-545`). | **Es de 62**; aquí sólo se mide su efecto sobre el tiempo de indexar. |
| Tokenizer en memoria | 255 MB de trie (`tokenizer-footprint`). | **Es de 91**; no cambia el tiempo de indexado. |

## Descartados

- **Cambiar el modelo, cuantizar INT8 o pasar a otro proveedor de ejecución**: cambio de contrato de vectores, de 17 model-reembed; 85 lo dejó fuera. Fuera de alcance.
- **Dejar de enmascarar secretos o de respetar `.gitignore` para ir más rápido**: `redact` se aplica a cada pasaje por una razón de seguridad (`CLAUDE.md`, «Ingest masks secrets»), y `.gitignore` ya saca `target/`.
- **Indexado incremental más fino que «el pasaje con el mismo texto»**: ya existe (`reuse_plan`).
- **Paralelizar el refresco entre docs con varias llamadas al embedder a la vez**: el embedder tiene un hilo de inferencia con ORT ya usando todos los hilos (`embedder.rs:536-543`); más concurrencia sólo apila memoria. Se retoma sólo si #94 muestra CPU ociosa durante `embed`.

## Interacciones

| Cambio | Relación |
|---|---|
| **#94 docs-index-cost** | **Depende de él.** Sus números (tiempo por fase, real contra rellenado, por docs/código y por área) son las puertas de acá. Si #94 cachea vectores entre corridas de eval, las mediciones de A, B y E se hacen con caché vacío o con un modo que lo ignore: hay que decirlo en la propuesta para no medir el caché. |
| 20 docs-search-quality | D2/D3 y E cambian el corpus y el partido que sus pasos 2-4 miden. Orden: cerrar los pasos 2-4 (o fijar el corpus) antes de decidir D/E, o medir ambos contra la misma línea base. Sus mediciones (15 casos en el verify; más de 40 hoy) sufren el mismo costo de 2407 s. |
| 62 embedder-footprint | Hilos, espera activa, `rayon`, tokenizar en paralelo: de 62. Orden de tanda y `EMBED_BATCH`: de acá (A, B); 62 los excluye. Ambas puertas miden `e5_golden` en Linux y Windows: se comparte la máquina y la tabla. Si 62 baja los hilos, el óptimo de B cambia: B va **después** de 62 o con las dos variantes. |
| 74 perf-bench | Un escenario «indexar N docs» lo podría adoptar; hoy no existe. Se escribe aquí como prueba ignorada y 74 lo adopta. |
| 91 tokenizer-footprint | Si cambia el tokenizer (Unigram propio), cambia el costo de tokenizar de A y de C; ids y offsets iguales por contrato. Se coordina, no se bloquea. |
| 17 model-reembed | Un cambio de contrato (E, o A si mueve bits) se apoya en `model_id`/huella de `plan_refresh`, no en el re-embebido de arranque de memorias. |
| `embed-throughput` (archivado) | Antecedente directo de A; reutiliza `longest_first` y su prueba de bits. |

## Alcance candidato y no-alcance

**Candidato (cada ítem tras su puerta):** puerta 0; A1, A2; B; C (o la eval en release como documentación); D (y su decisión de producto); E; las filas de F que #94 señale.

**No-alcance:** lo que #94 hace (medir fases, caché de vectores entre corridas, quitar desperdicio obvio); hilos/espera activa/`rayon` (62); el tamaño del tokenizer (91); cambiar de modelo (17); la calidad de recuperación como tal (20); la memoria (`memory`), que este índice no toca (se deriva del disco).

## Decisiones abiertas

1. ¿Qué máquina y qué build fijan «la primera indexación cabe en 20 s»? Hace falta una máquina de referencia con el modelo real, en release.
2. ¿Se acepta la tolerancia de 1e-6 de `e5_golden` si ordenar mueve bits en otra huella, o A queda fuera salvo bits idénticos? Por defecto: fuera.
3. D2: ¿se excluye `archive/` por defecto, por un ajuste, o nunca? Es decisión del usuario.
4. ¿El umbral de relleno (1,3× / 1,5×) viene del antecedente del import o se recalcula con la distribución de docs? Se fija en la propuesta con los números de #94.
5. C: ¿`"*"` sólo, o también `clawg-core`? ¿Opt-level 2 o 3?
6. Dónde viven las constantes propias de docs (E): `project_docs` privadas, sin visibilidad nueva.

## Cómo se mide

- **Puerta 0 y fases:** `rag-eval --real --docs` en release y en depuración, con la salida por fase de #94 (leer, enmascarar, partir, consultar guardados, embeber, escribir) y por tipo/área; 3 corridas, mediana. Necesita el modelo real y un perfil de datos autorizado (no se ejecuta aquí).
- **Relleno:** `embedder.batch_tokens` contra la suma de tokens reales por rol `Passage`, por doc y total.
- **A:** pruebas puras primero (índices a tandas y vuelta, estable, el orden de salida igual al de entrada), después la prueba ignorada tipo `import.rs:5500` sobre pasajes de docs; `e5_golden` y pares (diferencia máxima, vectores con los mismos bits) en cada huella de CPU; `rag-eval --diff` para confirmar que `recall@5` no se mueve (los vectores no deberían).
- **B:** barrido 8/16/32 con A fijo, pico de memoria privada (`process.private_peak_bytes`).
- **C:** tiempo total de la eval, tiempo del primer build limpio de `target-test`, tamaño del directorio.
- **D, E:** `rag-eval --real --docs --diff` con ≥ 6 casos movidos en un sentido para concluir; informar `by_split.test`.

## Riesgos

- **Medir contra un corpus que cambia.** Archivar un cambio agrega docs; las medidas de #94 y las de acá deben fijar el commit. Mitigación: el hash del árbol en cada resultado.
- **Mover bits.** Cualquier cambio de la composición de tanda (A, B) puede cambiar el último decimal; el import no lo hizo en una CPU. Mitigación: `e5_golden` y pares en cada huella.
- **Optimizar una cifra de depuración que no existe en release.** Mitigación: la puerta 0.
- **Memoria.** A2 y B suben el pico del daemon, que 62 y 91 reducen. Mitigación: tope y medición de `process.private_peak_bytes`.
- **Decidir producto por velocidad.** D2 cambia lo que el agente puede encontrar. Mitigación: decisión del usuario, no del umbral.
- **Dependencia de #94 aún no escrito.** Los nombres de sus métricas son supuestos; se ajustan en la propuesta.
