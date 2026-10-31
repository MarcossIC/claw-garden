# embedder-bakeoff — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (1, 3) | M, por puertas (la fase 0 es S y no trae ningún modelo nuevo) | — para memorias; el paso 1 de [docs-search-quality](../docs-search-quality/proposal.md) (20) y la memo de [docs-index-cost](../docs-index-cost/proposal.md) (94) para la parte de docs. Alimenta a [model-reembed](../model-reembed/exploration.md) (17) | exploración (borrador) |

**Tesis:** quedarse con `multilingual-e5-small` en fp32, sin cuantizar, y decidir un cambio de modelo **midiendo**, no leyendo tablas de MTEB. Este cambio sólo mide, como [english-recall](../archive/2026-10-17-english-recall/proposal.md) (21) y [revision-eval](../archive/2026-10-21-revision-eval/proposal.md) (48): compara e5-small con candidatos de **384 dimensiones** y licencia Apache 2.0 sobre el golden propio, entre idiomas y a escala, con su costo en memoria y tiempo, contra un umbral fijado antes de correr. Sin un candidato que lo pase, cierra con la evidencia y nada cambia. Con uno que lo pase, el cambio de modelo lo hace 17, que hoy no tiene modelo destino.

**De dónde sale.** De una investigación previa sobre `develop` en `614aa3c` (2026-10-21), que no está en el repositorio, que no pudo bajar ningún modelo ni leer los archivos de Hugging Face y que propuso ocho hallazgos (H1 a H8), una tabla de candidatos, un experimento y un umbral. Esta exploración los contrasta con `develop` en 0.68.0 (`6afce36`) y con lo que publican hoy los repositorios de los modelos: los tamaños, la licencia y el acceso salen de la API de Hugging Face (`/api/models/<repo>?blobs=true`), la arquitectura de `config.json`, `1_Pooling/config.json`, `tokenizer_config.json` y `tokenizer.json`, y las salidas de cada ONNX de los nombres al final del grafo, leídos con un pedido por rango (todo el 2026-10-31). Las cifras de MTEB se leyeron en el paper de Granite R2 ([S2]); las de EmbeddingGemma quedan como las citó la investigación ([S1]), porque arxiv.org no respondió desde este entorno. **Ninguna cifra de esta exploración es una medición nueva de clawg.**

## Premisas de la investigación, verificadas contra 0.68.0

| Premisa | Resultado |
|---|---|
| H1: el protocolo pide guardar y buscar en inglés, así que la brecha `es>en` casi no se da en uso real | **Ya no es cierta.** Desde [language-preference](../archive/2026-10-23-language-preference/proposal.md) (78) y [agent-language](../archive/2026-10-23-agent-language/proposal.md) (80), el idioma de guardar y buscar es el de la preferencia: en inglés sin ella (`crates/clawg-cli/src/mcp/protocol.md:17-18,81-82`), en español con `es` (`protocol.es.md:17-18,83-85`). Con `es`, un store con historia **mezcla** idiomas: lo guardado antes, en inglés, y lo nuevo, en español (`memory.lang`, V22). El cruce ya no se esquiva: se compensa con una segunda búsqueda traducida que el puente pide cuando la primera mostró menos de `FEW_SHOWN` (3, `crates/clawg-cli/src/mcp/tools.rs:124,136-140`), sin calibrar. M1 (paridad) y M2 (store mezclado y costo de esa segunda búsqueda) de 80 no se corrieron ([verify](../archive/2026-10-23-agent-language/verify.md#lo-que-no-se-midió)). La pregunta de la investigación cambia: no «¿pasa el cruce?», sino «¿un modelo mejor entre idiomas vuelve innecesaria la segunda búsqueda, y a qué costo?» |
| La brecha medida es `es>en` R@5 6/19 | **Cierta para el coseno puro sobre `golden.json`** ([21](../archive/2026-10-17-english-recall/proposal.md)). En `golden-cross.json` (48 casos) el R@5 de `similarity` da `en>es` 21/22 y `es>en` 19/26 (exploración de 78, `:18`). Los dos goldens están en `docs/eval/`, fuera de git |
| H2: sólo un modelo de pooling mean encaja en `Inference` | **Cierta.** `pool_and_normalize` promedia una salida de tres dimensiones y sólo normaliza una de dos (`crates/clawg-core/src/memory/embedder.rs:301-341`). **Confirmado en el modelo:** `granite-embedding-97m-multilingual-r2` declara `pooling_mode_cls_token: true` y su ONNX oficial tiene una sola salida, `last_hidden_state`: `pick_output` la tomaría (`embedder.rs:284-291`, una sola salida da 0) y la promediaría, dando vectores que no son los del modelo, sin ningún error |
| H3: `pick_output` prefiere `last_hidden_state` antes que `sentence_embedding` | **Cierta** (`embedder.rs:288`, orden `text_embeds`, `last_hidden_state`, `sentence_embedding`). **Confirmado en el export:** el ONNX de `onnx-community/embeddinggemma-300m-ONNX` tiene las dos salidas, así que clawg promediaría la última capa y dejaría fuera las dos densas de Gemma. Pesa sólo si se adopta Gemma |
| H4: un archivo de más de 600 MB se rechaza, y un `.onnx_data` externo no carga | **Cierta.** `MAX_FILE_BYTES` = `MIN_FREE_BYTES` = 600 MiB (`crates/clawg-core/src/memory/model_download.rs:228,1093`). Desde [ram-footprint-gates](../archive/2026-10-29-ram-footprint-gates/proposal.md) (92) el ONNX se carga como una vista mapeada de sólo lectura (`model_download.rs:831`), pero sigue entrando por `commit_from_memory` (`embedder.rs:554-558`), que no resuelve datos externos. Tamaños reales: granite-97m fp32 **390.004.608** bytes (entra), granite-311m fp32 **1.247.170.481** (no entra; su `model_quint8_avx2.onnx` mide 313.421.909), y **todas** las variantes de embeddinggemma en onnx-community traen `.onnx_data` externo (fp32 1.234.521.088 bytes) |
| H5: `rag-eval --real` está atado a e5, y agent-view cuenta tokens con el tokenizer del modelo | **Cierta.** `EvalEmbedder::real` arma `E5Embedder` (`crates/clawg-core/src/eval_api.rs:573-581`) y el contador de agent-view es el del modelo (`crates/clawg-eval/src/bin/rag-eval/agent_view/runner.rs:225,261,270`). Lo nuevo: la memo de vectores de 94 tiene por clave el contrato y el sha256 del ONNX, así que cada brazo tiene la suya sin mezclarse. `--diff` aparea los casos por `query` (`diff.rs:5-7`), lo que sirve tal cual para comparar dos modelos sobre el mismo golden |
| H6: seis constantes están calibradas sobre los cosenos de e5 con mean | **Cierta, y siguen donde estaban:** `cosine_floor` 0,79 (`crates/clawg-core/src/memory/score.rs:187`, el percentil 5 de esa distribución según `:380`), `CONTRADICTION_THRESHOLD` 0,42, `NEAR_DUPLICATE_FLOOR` 0,10, `ESCALATE_BELOW` 0,40 e `IDENTIFIER_ESCALATE_BELOW` 0,30 (`contradiction.rs:38,46,64,71`) y `RELEVANCE_RATIO` 0,35 (`crates/clawg-cli/src/relevance.rs:19`). Se suma `FEW_SHOWN` (3), que depende de cuántos aciertos muestra el modelo |
| H7: int8 rompe la tolerancia de `e5_golden` entre CPUs | **Plausible, y más concreta:** el int8 oficial de granite es `model_quint8_avx2.onnx`, cuantizado para AVX2. `e5_golden` compara con `TOLERANCE` 1e-6 (`crates/clawg-core/tests/e5_golden.rs:26`). Pesa sólo si se cuantiza |
| H8: el 81 % de los pesos de e5-small es la tabla de embeddings | **Cierta** (250.037 × 384 ≈ 96,0M de ~117,7M). Lo mismo pasa con granite-97m: `vocab_size` 180.000 × 384 ≈ 69,1M de ~97,4M (71 %), con unos 28,3M en sus 12 capas ModernBERT, contra 21,2M de e5. Para granite-311m: 262.152 × 768 ≈ 201,3M en la tabla y ~110M en 22 capas, unas 5 veces el cálculo por token de e5 |
| El contrato de producción no está fijado por un test (17) | **Ya no es cierta.** `the_embedding_contract_is_pinned` fija el id, los prefijos, la ventana, la dimensión y los SHA-256 (`embedder.rs:1697-1720`, RM4 de 92), y `expected_contract` lo da sin cargar el modelo (`embedder.rs:155-163`) |
| La poda de snapshots puede sacar al `pre-migration` | **Ya no es cierta:** conserva el más nuevo de cada motivo (`to_prune`, `crates/clawg-core/src/memory/backup.rs:298-311`) |
| Re-embeber cuesta unos 15 ms por fila | **Hay un dato mejor.** El import real del export completo embebió en 825,8 s a 65,9 filas/s ([embed-throughput](../archive/2026-10-27-embed-throughput/verify.md), 85), ordenando por largo y en llamadas de a 32. `reembed_all`, en cambio, infiere **fila por fila** (`compute_new_embedding`, `crates/clawg-core/src/memory/maintenance.rs:694-708`), así que re-embeber el store de prod (54.436 filas importadas) tardaría más que esos ~14 minutos. Ninguna cifra para otro modelo está medida |
| El índice de docs se reindexa solo al cambiar de modelo | **Cierta, y sin mezclar vectores:** `plan_refresh` compara `model_id` (`crates/clawg-core/src/memory/project_docs/mod.rs:188-194`) y la reutilización de vectores de 92 los busca por ese `model_id` (`:533`). El costo es el de 94: 2407 s en debug para 727 docs y 17.584 pasajes |

Lo que la investigación no consideró y cambia el diseño:

- **El tokenizer de cada candidato.** El piso del daemon con el modelo suelto es casi todo el tokenizer (~255 MB para el `tokenizer.json` Unigram de 17 MB de e5; [tokenizer-footprint](../tokenizer-footprint/exploration.md), 91). Los candidatos traen otro: granite-97m un BPE de 179.934 entradas en un `tokenizer.json` de 25.301.672 bytes, granite-311m uno de 262.144 en 33.384.821 bytes. Cuánta memoria ocupa no está medido. Un cambio de modelo cambia el piso que 91 quiere bajar, en cualquier dirección, y vuelve parte de 91 obsoleta o urgente según el resultado. Es una columna del costo, no un detalle.
- **La escala.** [ranking-at-scale](../archive/2026-10-29-ranking-at-scale/proposal.md) (86) midió con `rag-eval --background` sobre dos cortes del historial que el ancla pierde 14 a 15 casos de R@5 a escala, y que ni el coseno del mejor hit ni el hueco separan las consultas con y sin respuesta. Las dos cosas dependen del espacio de embedding. Un modelo que gana en el golden chico y pierde a escala no sirve; uno que separa mejor las consultas sin respuesta reabre la abstención. `--background` se corre en cada brazo.
- **Lo que ya entra por la descarga.** granite-97m: Apache 2.0, sin acceso restringido, ONNX oficial dentro del tope, `special_tokens_map.json` presente y `model_max_length` 32.768, que pasa `check_tokenizer` (`embedder.rs:359-381`). No declara `token_type_ids`, y `Inference` ya lo maneja (`wants_token_type_ids`, `embedder.rs:610`). Su `pad_token_id` (179.935) sale de `config.json`, como hoy. Lo que no se verificó es que su `tokenizer.json` cargue en `tokenizers` 0.22 con los mismos ids que en Python.

## Veredictos

Impacto: cuánto mejora la capacidad de decidir sobre el modelo, o el producto si se adopta. Riesgo: qué puede salir mal al hacerlo o al leer sus números. Esfuerzo: Bajo, horas; Medio, 1 a 3 días; Alto, más.

| # | Propuesta | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|---|
| R1 | Quedarse con e5-small fp32 por ahora | **Incorporar** | Alto | Bajo | — |
| R2 | Un cambio sólo de medición (*bake-off*) | **Incorporar** (este cambio) | Alto | Medio | Medio |
| C1 | Brazo granite-embedding-97m-multilingual-r2 | **Incorporar** (primer brazo) | Alto | Medio | Medio |
| C2 | Brazo granite-embedding-311m-multilingual-r2 truncado a 384 | **Modificar**: segundo brazo, sólo si C1 no pasa la calidad, y sabiendo que su fp32 no entra en la descarga | Medio | Medio | Medio |
| C3 | Brazo EmbeddingGemma-300m | **Posponer** (decisión del usuario sobre la licencia; camino de 768) | Medio | Alto | Alto |
| C4 | jina v3 y v5, Qwen3-Embedding-0.6B, bge-m3, model2vec, arctic-embed-m-v2.0 | **Descartar** | — | — | — |
| C5 | harrier-oss-v1-270m | **Investigar** (licencia y arquitectura sin verificar) | Bajo | Medio | Bajo |
| H1 | Medir cuánto cruce de idioma queda en uso real | **Modificar** (fase 0, con e5, sobre la premisa nueva de 78 y 80) | Alto | Bajo | Bajo |
| H2 | Pooling CLS | **Incorporar** en el brazo y, si gana, en 17 | Medio | Bajo | Bajo |
| H3 | Salida elegida por `pick_output` | **Incorporar** en 17 (una salida declarada por modelo); hoy no pesa | Bajo hoy | Alto si se adopta Gemma | Bajo |
| H4 | Tope de 600 MB y datos externos | **Incorporar** en 17 sólo si gana un modelo que no entra | Medio | Medio | Medio |
| H5 | Eval atado a e5 y tokens contados con el tokenizer del modelo | **Incorporar** (es lo que construye este cambio) | Alto | Medio | Medio |
| H6 | Recalibrar seis constantes | **Modificar**: en el bake-off sólo `cosine_floor`; las otras, en 17 para el ganador | Medio | Medio | Medio |
| H7 | Tolerancia de `e5_golden` con int8 | **Posponer** junto con int8 | Bajo | Bajo | Bajo |
| H8 / Q | Cuantizar e5-small a int8 | **Posponer** (decide la puerta de [embedder-footprint](../embedder-footprint/exploration.md), 62, y re-embebe por 17) | Medio | Medio | Medio |
| Q2 | fp16 en CPU | **Descartar** | — | — | — |
| Q3 | Truncar e5-small por Matryoshka | **Descartar** | — | — | — |
| U | El umbral para cambiar | **Modificar** (suma escala, idiomas mezclados y la memoria del tokenizer) | Alto | Bajo | Bajo |

## Por propuesta

Se agrupan en cuatro bloques porque se deciden juntas: qué modelo (R1, C1 a C5), cómo se mide (H1, H5, R2, U), qué costaría adoptarlo (H2, H3, H4, H6) y qué hacer con la cuantización (H7, H8, Q2, Q3).

### Bloque 1. Qué modelo

**R1. Quedarse con e5-small — Incorporar.** Ninguna fuente compara e5-small con un candidato **entre idiomas** sobre un corpus como el de clawg: MTEB retrieval mide sobre todo búsquedas dentro de un idioma, y +10 puntos ahí no prometen nada sobre `es>en`. La segunda búsqueda de 80 ya compensa el cruce, con un costo no medido. Cambiar de modelo es un cambio de contrato con re-embebido, recalibración y reindexado (17). Sin medición, no hay argumento para pagarlo.

**C1. granite-embedding-97m-multilingual-r2 — Incorporar como primer brazo.** Es el cambio más barato posible: misma dimensión (384, sin migrar las vec0), cálculo por token del mismo orden (~28M de parámetros activos contra ~21M), Apache 2.0, sin acceso restringido, ONNX oficial de 390 MB que entra en el tope. [S2] le da 60,3 de MTEB-v2 Retrieval multilingüe contra 50,9 de e5-small, 60,4 contra 51,3 en código, y declara evaluación entre idiomas (MLQA). Lo que cuesta: pooling CLS (H2), otro tokenizer (BPE, 25 MB de archivo, memoria sin medir), otro manifiesto y la recalibración. **Riesgo:** que el BPE de 180.000 entradas tokenice el español peor que el Unigram de 250.000 de e5, y que los pedazos de 96 y 480 tokens (`crates/clawg-core/src/memory/chunker/mod.rs:26,50`) cubran otro largo de texto. Eso es parte del efecto del modelo y se reporta, no se corrige.

**C2. granite-embedding-311m-multilingual-r2 a 384 — Modificar.** [S2] (tabla 8) le da 65,0 a 384 dimensiones por Matryoshka, contra 65,2 a 768: truncar no cuesta casi nada y evita la migración. Pero su fp32 mide 1,25 GB (no entra en `MAX_FILE_BYTES`), su int8 oficial es para AVX2 (H7), y con ~110M de parámetros activos cuesta unas 5 veces el cálculo de e5 por save, por búsqueda y en cada reindexado. Con el umbral de costo (U4) lo más probable es que no pase. Va como segundo brazo **sólo si C1 no pasa el criterio de calidad**: sirve para saber si el techo de esa familia cierra el cruce, aunque su costo lo deje fuera del producto.

**C3. EmbeddingGemma-300m — Posponer.** Es la única con evidencia publicada fuerte entre idiomas (XTREME-UP MRR@10 47,7 según [S1]), pero acumula cuatro costos: la licencia Gemma con el repositorio oficial de acceso restringido manual (la descarga de clawg es anónima), datos externos en todos los exports de onnx-community (H4), 768 dimensiones sin 384 entrenado (camino C de 17, una migración de las cuatro vec0) y H3. Entra como brazo sólo si el usuario acepta la licencia **y** C1 y C2 no cierran el cruce: es el que contesta si el problema tiene solución por el modelo.

**C4. Descartados.** Se aceptan las razones de la investigación: jina v3 y v5 por licencia no comercial (y v3 deprecado), Qwen3-Embedding-0.6B y bge-m3 por tamaño y 1024 dimensiones, con 6,6 y 26,9 en XTREME-UP [S1], model2vec por quedar debajo de e5-small en MMTEB y no tener contexto, y arctic-embed-m-v2.0 por quedar debajo en la media multilingüe. Que [ideas evaluadas](../../README.md#ideas-evaluadas) ya descartara un modelo orientado a código para `search_project` sigue en pie: el cuello de botella de docs es léxico (20).

**C5. harrier-oss-v1-270m — Investigar.** 66,4 en [S2], pero la ficha oficial no se pudo leer y las fuentes no coinciden en la arquitectura. Entra a la tabla sólo con la licencia y la ficha verificadas. No bloquea nada.

### Bloque 2. Cómo se mide

**H1. Cuánto cruce queda — Modificar.** La premisa cambió (ver la tabla de arriba): con `es` el cruce es parte del producto. La fase 0 mide la línea de base con e5 y contesta la pregunta pendiente de la investigación sin leer ningún texto: cuántas filas vivas de prod hay por `memory.lang` y por proyecto, en una copia de un snapshot. No hace falta código: es un `SELECT lang, COUNT(*)` sobre la copia, con autorización para leer prod. Si casi todo está en un idioma y la preferencia es ese, el cruce pesa poco y el bake-off se lee por R@5 global y a escala. Si está mezclado, el bloque `es>en` y `golden-cross` pasan a primarios.

**H5. Eval atado a e5 — Incorporar.** Es el trabajo de ingeniería del cambio. Dos formas de correr un brazo:

- **Un worktree descartable por brazo** que cambia manifiesto, pooling y prefijos, como hizo 21 con los prefijos. No toca el producto ni deja abstracciones sin uso. Cada worktree compila su propio build, y la regla de «Build directories» de `.claude/CLAUDE.md` pide borrarlo con su directorio al terminar.
- **Un embedder candidato en `eval_api`**, detrás del feature `eval`, que lee el manifiesto y el pooling de un archivo. Evita recompilar por brazo, pero es código nuevo que sólo se justifica si se van a comparar más de dos o tres modelos.

**Recomendación:** el worktree para la primera ronda. Si un candidato pasa, 17 vuelve genérico el manifiesto de producción y el eval lo hereda. En cualquiera de las dos, **los tokens de agent-view se cuentan con el tokenizer de e5** en todos los brazos, si no `--diff` compara tokens de dos vocabularios.

**Control de identidad por brazo.** Antes de medir, los vectores de clawg para los textos de `e5_golden` contra los del modelo de referencia en `sentence-transformers`: coseno ≥ 0,999 en cada texto. Sin eso, un pooling equivocado (H2, H3) se lee como «el modelo es malo». Pide Python con `sentence-transformers` en la máquina que mide, fuera del repositorio.

**R2. El procedimiento — Incorporar**, con lo que suma 0.68.0:

1. **Fase 0, A0 (e5-small, sin código):** `rag-eval --real` y `--real --agent-view` sobre `golden.json` y sobre `golden-cross.json`, `--background` sobre la copia del snapshot que usó 86, y `--real --docs` cuando 20 haya hecho su paso 1 (sin él, docs mide la contaminación del `verify.md` archivado de 12). Más el conteo por idioma de H1.
2. **Fase 1, B1 (granite-97m):** el control de identidad y las mismas corridas, con `--sweep-cos-floor` de 0,30 a 0,90 para elegir el piso en `by_split.calibration` e informar en `by_split.test`, la regla de [eval-rigor](../archive/2026-10-15-eval-rigor/proposal.md) (33).
3. **Fase 2, B2 (granite-311m a 384), sólo si B1 no pasa U1 o U2.** Mismas corridas; en el worktree el ONNX se carga del disco, sin pasar por el tope de la descarga.
4. **B3 (EmbeddingGemma), sólo con la licencia aceptada** y con las vec0 a 768 en su worktree; sus dos formas de prefijo (la asimétrica de la ficha y una simétrica) son dos brazos, como enseñó 21.
5. **Comparación:** `rag-eval --diff a0.json bX.json --fail-on-regression` por par, leyendo el global, `es>en`, `golden-cross`, `search.found`, `context.found` y el bloque a escala.
6. **Costo por brazo:** la memoria privada con el modelo cargado y con el modelo suelto (el piso del tokenizer), el pico de la carga, por `metrics.db` (`process.private_bytes`, `process.private_peak_bytes`, `embedder.resident`) como midió 92; el tiempo de `embed` del import del export completo (el proxy de 85) y el de indexar los docs (el de 94).

**Criterio de 33:** McNemar exacto sobre los casos que se mueven, y una conclusión necesita al menos 6 en una dirección. Con 19 casos `es>en` y 6 aciertos, ganar con significancia pide recuperar al menos 6 de los 13 que faltan casi sin perder ninguno. Con menos, «no distinguible», nunca «no mejora».

**U. El umbral — Modificar.** Se fija antes de la fase 1 y queda en la propuesta:

1. **Calidad (primario):** en `similarity` y `retrieval`, `es>en` R@5 mejora con al menos 6 casos netos y p < 0,05, **o** el R@5 global mejora con p < 0,05, **o** el R@5 del bloque a escala (`--background`) mejora con p < 0,05.
2. **Ninguna regresión:** `--fail-on-regression` no sale con 6 en `golden.json`, `golden-cross.json`, agent-view ni a escala, y `es>es` y `en>en` no pierden casos.
3. **Docs** (después del paso 1 de 20): Hit@3 no baja. No alcanza solo: son 15 preguntas.
4. **Costo:** memoria privada con el modelo cargado no mayor que hoy; piso con el modelo suelto no mayor que hoy (lo que mida 91 o la fase 0); `embed` del import no mayor que 2 veces el de hoy; archivos que entran en la descarga anónima con licencia Apache o MIT.
5. **Contrato:** se prefiere 384 (camino B de 17) salvo que un brazo de 768 gane por una diferencia que el de 384 no alcanza con significancia.

Ningún brazo que pase U1 es un resultado válido: cierra con la segunda búsqueda de 80 como la solución del cruce, ahora con evidencia.

### Bloque 3. Qué costaría adoptarlo (va a 17)

Nada de esto se hace en este cambio. Se escribe acá para que el umbral sepa qué compra y queda en 17 como su alcance si un brazo pasa:

- **H2, pooling:** una variante CLS de `pool_and_normalize` (tomar el primer token antes de normalizar), con el pooling en el id del contrato, como hoy `/mean` (`embedder.rs:146`).
- **H3, salida:** que el manifiesto declare la salida en vez de depender del orden de `pick_output`. Sólo urge con Gemma.
- **H4, descarga:** el manifiesto deja de ser fijo (`HF_BASE_URL`, `ALL_FILES`, el directorio `multilingual-e5-small` en `tokenizer_path`, `model_download.rs:27,92,126-128`). Con granite-97m no hace falta tocar el tope; con granite-311m o Gemma, sí, o el int8, y Gemma pide además leer datos externos.
- **H6, recalibración:** las seis constantes más `FEW_SHOWN`, con `rag-eval`, `conflict-eval` y `revision-eval`. Precedente: el paso de CLS a mean en 12 movió tres.
- **Re-embebido:** camino B de 17 para 384 (un `clawg reembed` sobre `reembed_all`, hoy llamado sólo por la mejora de pooling: `is_pooling_upgrade` y `verify_or_reembed`, `crates/clawg-core/src/memory/bootstrap.rs:641,655`), camino C para 768, con la decisión abierta F1/F2/F3 de 17 sobre la fila que no se puede embeber. El tiempo sobre un store como el de prod es la cifra de la fase de costo, no una estimación.
- **`e5_golden`:** fixture nueva con `CLAWG_BLESS=1` y el test renombrado.

### Bloque 4. Cuantizar

**H8 y Q. int8 de e5-small — Posponer.** Con el 81 % de los pesos en la tabla de embeddings, int8 baja sobre todo disco y RAM con el modelo cargado, y apenas el cálculo, que vive en ~21M de parámetros. No toca la brecha entre idiomas: son los mismos pesos con menos precisión. Su ahorro es el de 62 y 92, que miden memoria y CPU con sus puertas; si una de ellas muestra que la RAM del modelo cargado es el problema, se suma un brazo int8 a este mismo procedimiento (misma calidad medida, mismo umbral) y el re-embebido va por 17, como 85 dejó escrito.

**H7. Tolerancia de `e5_golden` — Posponer** con int8. Si se cuantiza, el test compara por coseno (por ejemplo ≥ 0,999 contra la fixture) y no por bits, porque el int8 cambia los vectores según la CPU.

**Q2. fp16 — Descartar.** El proveedor de CPU de ONNX Runtime no tiene kernels fp16 para todos los operadores y mete conversiones, así que sólo baja el disco; EmbeddingGemma declara que no lo soporta en las activaciones [S4].

**Q3. Truncar e5-small por Matryoshka — Descartar.** No se entrenó así, y 384 ya ocupa 1,5 KB por fila en vec0. Matryoshka sirve en la otra dirección: traer un modelo de 768 a 384 sin migrar, que es lo que hace C2.

## Lo que no se reabre

- **El prefijo `passage: `**: 21 lo midió dos veces con mean y pierde.
- **Un piso de coseno como solución del cruce**: 21 lo descartó.
- **Reranker y GPU**: descartados en 20 y en las ideas evaluadas; ningún candidato pide GPU.
- **Hilos, spinning y lotes del embedder**: son de 62, con su puerta.
- **La integridad de la descarga** (SHA-256 por archivo, tope, origen limitado): sirve igual para cualquier manifiesto.

## Orden

Corre en paralelo al orden de ejecución, porque no toca el producto, pero cada fase con el modelo real o con datos de prod pide autorización. La parte de docs espera el paso 1 de 20 y aprovecha la memo de 94. Si un brazo pasa, 17 sube de lugar: deja de ser «sin cambio de modelo planeado». Se coordina con 91: el resultado puede cambiar el tokenizer cuyo piso 91 quiere bajar, así que el paso 0 de 91 conviene antes de un cambio de modelo, no después.

## Decisiones abiertas

Son del usuario. Esta exploración no las decide.

- [ ] **¿Se lee prod para contar filas por idioma** (H1), sobre la copia de un snapshot y sin textos?
- [ ] **¿Worktree por brazo o un embedder candidato en `eval_api`?** Se recomienda el worktree.
- [ ] **¿Se acepta la licencia Gemma** para medir B3? Para medir no hay distribución; para adoptarlo habría que aceptarla en cada instalación, y la descarga de clawg es anónima.
- [ ] **¿B2 corre aunque su costo lo deje fuera del producto?** Sirve para saber si el techo de la familia cierra el cruce.
- [ ] **¿Se fija el umbral U tal como está,** en particular el tope de 2 veces el tiempo de `embed` y el piso con el modelo suelto?

## Cómo se mide

No hay tests de producto: el cambio no toca el producto. Lo que se escribe es la medición:

1. **Fase 0:** archivos de resultados de A0 en `docs/eval/results/` (fechados, nunca reemplazan uno existente) y el conteo por idioma, con números y sin textos.
2. **Por brazo:** el control de identidad (coseno ≥ 0,999), los mismos archivos de resultados con su `--tag`, el `--diff` contra A0 y la tabla de costo.
3. **Veredicto:** cada punto de U con su número, en el `verify.md` del cambio. Los worktrees y sus builds se borran al terminar.

## Fuentes

- [S1] Vera et al., *EmbeddingGemma: Powerful and Lightweight Text Representations*, arXiv:2509.20354v3, tablas 1, 5, 6 y 9 — https://arxiv.org/html/2509.20354v3 (cifras citadas por la investigación, no releídas)
- [S2] IBM Research, *Granite Embedding Multilingual R2 Models*, arXiv:2605.13521v2, el resumen, las tablas 5, 8, 9 y 11 — https://arxiv.org/html/2605.13521v2 (leído el 2026-10-31)
- [S3] Fichas y archivos: https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2 , https://huggingface.co/ibm-granite/granite-embedding-311m-multilingual-r2 (leídos el 2026-10-31)
- [S4] https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX y https://huggingface.co/google/embeddinggemma-300m (licencia `gemma`, acceso `manual`; leídos el 2026-10-31); términos en https://ai.google.dev/gemma/terms
- [S5] ONNX Runtime, *Quantize ONNX models* — https://onnxruntime.ai/docs/performance/model-optimizations/quantization.html
- Repositorio en `6afce36`: los `ruta:línea` de cada punto; [english-recall](../archive/2026-10-17-english-recall/proposal.md) (21), [language-preference](../archive/2026-10-23-language-preference/exploration.md) (78), [agent-language](../archive/2026-10-23-agent-language/verify.md) (80), [embed-throughput](../archive/2026-10-27-embed-throughput/verify.md) (85), [ranking-at-scale](../archive/2026-10-29-ranking-at-scale/proposal.md) (86), [ram-footprint-gates](../archive/2026-10-29-ram-footprint-gates/proposal.md) (92), [docs-index-cost](../docs-index-cost/proposal.md) (94)
