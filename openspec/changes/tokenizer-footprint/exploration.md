# tokenizer-footprint — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M–L (el paso 0 es S; un `Model` propio es L) | ram-footprint-gates (92), que mide el piso en uso real con lo que dejó ram-footprint (90); se coordina con 62 y 17 | exploración (borrador) |

**Tesis:** con el modelo suelto, el daemon ocupa ~284 MiB en prod, y casi todo eso es el tokenizer de `multilingual-e5-small`: un `tokenizer.json` de 17 MB se convierte en ~255 MB residentes, en 1,18 millones de asignaciones chicas. Se parsea una vez y queda toda la vida del proceso. No se lo puede soltar junto con el modelo, porque en Windows reparsearlo fragmenta el heap. El sospechoso es la estructura del modelo Unigram de `tokenizers`, no los datos. El cambio es de **medición primero**: saber de qué están hechos esos 255 MB antes de decidir entre una representación compacta propia, un arreglo upstream o cerrar con lo medido.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El piso del daemon con el modelo suelto es el tokenizer | **Cierta, medida.** Con `CLAWG_TEST_EMBEDDER=fake`, el daemon ocupa 6 a 9 MB privados sin `tokenizer.json` y 262 a 265 MB con él (`changes/archive/2026-10-17-embedder-idle-unload/verify.md:82-85`). En prod, la mínima de una sesión fue 283,9 MiB (ram-footprint, «Lo medido en prod»). |
| Se lo puede soltar junto con el modelo | **Falsa en Windows.** Parsear y soltar sólo el tokenizer 40 veces llevó el proceso de 12,5 a 481 MB, con saltos (`verify.md:70`, fila T). Por eso D5 de 46 eligió un tokenizer por proceso, parseado una vez. |
| La causa es la estructura y no el tamaño del vocabulario | **Probable, sin medir.** El modelo Unigram guarda un `Trie<u8>` en el que **cada nodo tiene su propio `AHashMap<u8, Node>`** (`tokenizers-0.22.2/src/models/unigram/trie.rs:79-81`), más `vocab: Vec<(String, f64)>` y `token_to_ids`, un mapa con claves `String` propias (`src/models/unigram/model.rs:19-23`), todo sobre unas 250.000 piezas. Un mapa por nodo, en un trie de bytes, explica tanto el millón largo de asignaciones como un múltiplo de 15× sobre el archivo. Cuánto pesa cada parte no está medido. |
| Un `Model` propio obliga a reescribir todo el tokenizer | **Falsa.** `Model` es un trait público (`tokenizers-0.22.2/src/tokenizer/mod.rs:70-88`) y `TokenizerImpl<M, N, PT, PP, D>` es genérico en el modelo (`:513`). `Tokenizer` es un newtype sobre `TokenizerImpl<ModelWrapper, …>` (`:410`). Un Unigram compacto puede ir con el normalizador `Precompiled`, el pre-tokenizador `Metaspace` y el post-procesador del archivo tal como `tokenizers` los arma. |
| Cambiar el tipo toca poco código | **Cierta a medias.** El tokenizer compartido aparece en once archivos de `clawg-core` (`runtime.rs`, `bootstrap.rs`, `model_download.rs`, `embedder.rs`, `chunker/`, `pieces.rs`, `project_docs/mod.rs`, `worker.rs`, `eval_api.rs`). Usan `encode`, `encode_batch`, `post_process`, `get_post_processor`, ids, offsets, máscaras y el `pad_id`. Todo eso lo da `TokenizerImpl` con cualquier `M`, pero la firma `Arc<Tokenizer>` cambia en cada uno. |

## Problema

1. **Un costo fijo que no se puede soltar.** Son ~255 MB todo el tiempo, con el modelo cargado o no. Después de ram-footprint (90), que deja el modelo suelto más tiempo, este piso pasa a ser lo que más pesa en un daemon ocioso.
2. **No se sabe qué parte es evitable.** Si el trie es la mayor parte, una representación compacta (los hijos en un arreglo ordenado, o un double-array) baja el piso a decenas de MB. Si lo que pesa son las `String` del vocabulario y del mapa, la ganancia es otra y el camino también.

## Lo que ya existe y se reutiliza

- El procedimiento de 46 para medir el daemon con `CLAWG_TEST_EMBEDDER=fake` y sólo `tokenizer.json` (`verify.md:102`): es la línea de base de este cambio.
- `e5_golden` (`crates/clawg-core/tests/e5_golden.rs`): 32 vectores que no se pueden mover. Un tokenizer que diera otro id cambiaría un vector.
- Los tests del splitter que fijan offsets e inicios de palabra (`crates/clawg-core/src/memory/chunker/token_splitter.rs`), y los de `window_encodings` contra `encode_batch` (`crates/clawg-core/src/memory/embedder.rs:891-935`).
- El corpus de `docs/eval/` (fuera de git) y los textos de los tests, como entradas para comparar dos tokenizers.

## Paso 0: de qué están hechos los 255 MB (sin modelo, sin puerta)

Un test ignorado, o un binario de `clawg-eval`, con un asignador que cuenta bytes y asignaciones. Son unas líneas de `GlobalAlloc` sobre el del sistema; no hace falta una dependencia nueva. Mide:

| Medición | Qué decide |
|---|---|
| El `Tokenizer` entero parseado desde `tokenizer.json` | La línea de base, comparada con los 255 MB de 46 |
| `vocab` como `Vec<(String, f64)>` y `token_to_ids` como `AHashMap<String, u32>`, armados aparte con las mismas piezas | Lo que pesa el vocabulario en cualquier representación con `String` |
| La cantidad de nodos del trie (los prefijos de bytes distintos de todas las piezas) y el resto: total menos vocabulario y mapa | Lo que pesa el trie. Si es la mayor parte, B o C tienen sentido |
| Lo mismo en Linux y en Windows | El asignador de cada sistema cambia el costo por asignación |

**Puerta:** si una representación compacta no baja el piso en al menos un umbral (decisión abierta; candidato: 150 MB), se cierra con lo medido (D).

## Enfoques posibles

### A. Paso 0 y nada más, si la medición no da

Es un resultado válido: el piso queda medido y explicado.

### B. Un `Model` Unigram compacto propio (recomendado si el trie es la mayor parte)

Un tipo que implementa `tokenizers::Model` con el vocabulario en un solo buffer de bytes con sus offsets, los puntajes en un `Vec<f64>`, el trie como arreglo (hijos ordenados en un vector plano, o double-array) y el mismo Viterbi, desempates incluidos. Va enchufado a `TokenizerImpl` junto con el normalizador, el pre-tokenizador y el post-procesador que `tokenizers` arma desde el mismo archivo. **A favor:** se reemplaza sólo lo que pesa, y lo demás sigue siendo de `tokenizers`. **En contra:** el Viterbi de Unigram tiene detalles que hay que copiar exactos: el `unk`, el byte fallback si el modelo lo usa, los desempates y la caché. Cualquier diferencia mueve ids, y con ellos vectores y cortes de pieza. La prueba es una comparación exhaustiva: los mismos ids, offsets y máscaras que `tokenizers` sobre todo el corpus disponible, más `e5_golden` sin cambios.

### C. Arreglo upstream en `tokenizers`

Un PR que haga compacto el `Trie` de Unigram (por ejemplo, hijos en un `Vec<(u8, Node)>` ordenado). **A favor:** no hay código propio que mantener, y se beneficia todo el que usa Unigram. **En contra:** no se controlan los tiempos. Además se gana menos que con B: sigue habiendo una asignación por nodo con hijos, aunque mucho más chica que un mapa. Se puede proponer en paralelo a B.

### D. Cerrar con lo medido

Si el paso 0 muestra que lo evitable es poco.

## Descartado

| Opción | Razón técnica |
|---|---|
| Soltar el tokenizer con el modelo | Fragmenta el heap de Windows (46, fila T: de 12,5 a 481 MB en 40 vueltas) |
| Un heap propio para el tokenizer (`HeapCreate`/`HeapDestroy`) | Enrutar las asignaciones de una estructura a otro heap exige un asignador global con estado por hilo o por ámbito: `unsafe`, frágil y sólo para Windows |
| mimalloc como asignador | 46 lo midió plano pero entre 280 y 430 MB (`verify.md:64`): no baja el piso, sólo evita que crezca |
| Otro modelo con un vocabulario chico | Es un cambio de contrato y de calidad entre idiomas (17, 21) |
| Cargar el tokenizer sólo cuando hace falta | El chunker lo usa en cada save y el presupuesto de la consulta en cada contexto, así que estaría cargado casi siempre y pagaría la fragmentación de reparsearlo |

## Interacciones

- **ram-footprint (90) y ram-footprint-gates (92).** 90 dejó la serie por minuto (M1), y 92 mide con ella el piso en uso real (su pregunta 5). Cuanto menos tiempo pasa el modelo cargado, más pesa este cambio.
- **embedder-footprint (62).** Su brazo C (`set_parallelism(false)`) toca el mismo tokenizer, en otro eje (hilos). Si llega antes, la comparación de B se hace con el paralelismo como quede.
- **model-reembed (17).** Un cambio de modelo podría traer otro tokenizer y volver inútil este cambio. No hay ninguno planeado.
- **embed-throughput (85).** INT8 no cambia el tokenizer. El costo de tokenizar que mida 85 es la referencia para que B no sea más lento.
- **piece-boundaries (83, archivado).** Los cortes por inicio de palabra dependen de los offsets: B tiene que dar los mismos.

## Alcance candidato

1. El paso 0, con su registro en `verify.md`.
2. Según la puerta, B (con la comparación exhaustiva como test, `e5_golden` y los tests del splitter sin cambios), C en paralelo, o D.

### No-alcance

- Cambiar el modelo, el vocabulario o el contrato de embedding.
- La política de carga y descarga del modelo (90).

## Decisiones abiertas

Son del usuario.

- [ ] **Umbral de la puerta.** ¿Cuántos MB de piso justifican mantener un `Model` propio? Candidato: 150.
- [ ] **B y C.** ¿Se propone el arreglo upstream aunque se haga B?
- [ ] **Corpus de la comparación.** ¿Alcanza con el golden set, los docs del repo y los textos de los tests, o se suma un corpus multilingüe además del español y el inglés del store?

## Cómo se mide

1. **Paso 0.** Sin sesión de ONNX y sin perfil, sólo con `tokenizer.json`, leído de `CLAWG_E5_MODEL_DIR` como lo lee `e5_golden`.
2. **B.** Una comparación exhaustiva contra `tokenizers` (ids, offsets, máscaras, `post_process`), `e5_golden` con los mismos bits, los tests del splitter y del embedder sin cambios, y el daemon con `CLAWG_TEST_EMBEDDER=fake` y sólo `tokenizer.json`, medido como en 46, en Linux y Windows.

## Riesgos

- **Que B dé un id distinto en un caso que el corpus no cubre.** Se mitiga con un corpus amplio y multilingüe. En el peor caso, un texto raro tokeniza distinto: su vector cambia y su corte de pieza también.
- **Mantener un tokenizer propio.** Una versión nueva de `tokenizers` puede cambiar el comportamiento de Unigram; la comparación exhaustiva corre en la suite y lo detecta.
- **Que el paso 0 diga que el trie no es la mayor parte.** Entonces es D, o un enfoque sobre las `String`, y este documento se revisa.
