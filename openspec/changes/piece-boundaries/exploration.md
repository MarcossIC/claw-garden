# piece-boundaries — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar y recuperar bien) | S el corte en límite de palabra; M si se adoptan los dos brazos medidos | ninguno (se coordina con import-linear, import-decision-pass, embed-throughput y ranking-at-scale; debe entrar antes de la importación grande) | exploración (borrador) |

**Tesis:** el divisor parte un guardado largo en ventanas de 96 tokens contando tokens y nada más, así que el borde de una pieza cae donde caiga el token: dentro de una palabra. Cada pieza que empieza o termina a mitad de palabra pierde dos veces: el índice léxico ve un término que no existe (`ies`, `vant`) y el vector promedia un fragmento sin sentido en sus extremos. El arreglo es elegir el borde en la última frontera de palabra que cabe bajo el tope, con la información que el tokenizador ya entrega (`word_ids` u offsets), sin tocar los topes 96/24. Debe aterrizar antes de importar un historial grande porque las filas conservan las piezas en que se partieron, una reimportación encuentra la nota presente y no cambia nada, y el contrato de embeddings no incluye ningún parámetro del divisor: ni un arreglo del borde ni una cabecera más corta dispararían un re-embebido. Dos brazos se miden aparte y se adoptan sólo si no pierden: (a) menos solape cuando el corte cae en fin de oración, que baja el número de piezas; (b) una cabecera de menos de 48 tokens, que baja los tokens embebidos por pieza. El cambio es pequeño en código y su costo real está en la medición.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| Las piezas se cortan por tokens y el corte puede caer dentro de una palabra | **Cierta.** `split_by_tokens` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:74-103`) toma `plan_windows(offsets.len(), max, overlap)` (`:153-175`), que trabaja sobre índices de token, y corta el texto en `offsets[start].0 .. offsets[end - 1].1` (`:96-99`). No mira si el token siguiente continúa la palabra. |
| `split_save` está en `chat.rs` alrededor de 88-125 | **Corregido.** Está en `chunker/chat.rs:114-122`. Corta tres veces con el mismo divisor: el texto entero a 480 (`chunk_with`, `:116`), la cabecera a `SAVE_HEAD_TOKENS` sin solape (`:120`) y las piezas a `PIECE_MAX_TOKENS` con `PIECE_OVERLAP_TOKENS` (`:121`). **La cabecera también se corta con el mismo divisor**, así que termina a mitad de palabra por la misma causa; el arreglo del divisor la alcanza sin código aparte. |
| El `word_tokenizer` de prueba nunca parte una palabra, así que un RED necesita un tokenizador de subpalabras | **Cierta.** `word_tokenizer` (`chunker/mod.rs:14-19`) es un `WordLevel` con `Whitespace`; `word_level_tokenizer` (`token_splitter.rs:716-730`) igual. No hay en el repo ningún fixture `WordPiece`, `BPE` ni `Unigram` (búsqueda sobre `crates/`). Hay que construir uno. |
| El respaldo por bytes (sin tokenizador) comparte el problema | **Parcial.** `split_by_bytes` (`token_splitter.rs:105-142`) corta a 4 bytes por token y ajusta sólo a frontera de carácter UTF-8; también puede partir una palabra. Pero `eval_api::fallback_pieces` (`crates/clawg-core/src/eval_api.rs:69-75`) usa ese camino para **dimensionar** el set `long` de `golden-mine synth`, y una prueba (`long_rows_do_not_depend_on_the_piece_cap`, `synth/mod.rs`) exige que ese archivo no cambie. Mover el respaldo cambiaría el set `--seed 7`. |
| Quién llama al divisor | **Verificado.** `split_save`: el worker (`crates/clawg-core/src/memory/worker.rs:760-765`) y la traducción de la importación (`portability/translate.rs:328`). `split` directo: el índice de docs (`project_docs/chunk.rs:127-141`, `emit`, con `CHUNK_MAX_TOKENS` menos la miga y `CHUNK_OVERLAP_TOKENS`; `project_docs/mod.rs:192`) y `EvalDocIndex` (`eval_api.rs:927`, `:955-958`). Un cambio dentro de `TokenSplitter::split` llega a los cuatro. |
| El `rag-eval --real` parte con el tokenizador real | **Cierta.** `EvalIngest::open` recibe `tokenizer_dir` y usa `resolve_tokenizer_and_chunker` (`eval_api.rs:42`, `:554`); `fallback_pieces` sólo dimensiona. Así el brazo se mide sobre el camino de producción. |
| Una fila conserva las piezas en que se partió | **Cierta.** `memory_piece` guarda el vínculo y `embedding_text` (`chunker/mod.rs:62-72`) rearma el texto con la cabecera guardada en `context`; el re-embebido usa el mismo contexto, no recalcula la cabecera. Una importación posterior a una nota presente "no cambia nada" (`.claude/CLAUDE.md`, `clawg import`). |
| Ningún campo del contrato de embeddings cubre parámetros del divisor | **Cierta.** `expected_contract` (`crates/clawg-core/src/memory/embedder.rs:145-152`) lleva `model_id`, `dim`, `query_prefix` y `passage_prefix`. Cambiar un tope o la cabecera no lo toca y no dispara re-embebido. |
| Reconstruir un guardado depende de que las piezas vecinas compartan texto | **Cierta, y limita el brazo (a).** `join_pieces` (`pieces.rs:62-93`) busca el solape como el sufijo más largo de la pieza anterior que la siguiente repite, con un mínimo de `MIN_OVERLAP_CHARS` = 16 (`:57`) y un máximo de 2048 bytes. Con solape 0, o menor de 16 caracteres, une con un salto de línea (`:74-77`) y `read_memory` mostraría ese salto donde no hay. El comentario de `:58-59` aún dice 64 tokens y está desfasado. |
| Un camino o un identificador es "una palabra" | **Depende del tokenizador, no verificable aquí.** El tokenizador real (e5, SentencePiece con `Metaspace`) no está en el repo ni se puede abrir sin el modelo; si su pre-tokenizador parte sólo por espacio, `crates/clawg-core/src/memory/…` es una palabra de 15 o más tokens y `word_ids` la agrupa. Con `Whitespace` (el fixture actual) los signos serían fronteras y daría otro resultado. El fixture nuevo debe usar `WhitespaceSplit`. Lo que no se sabe sin el modelo real: si el offset del primer token de una palabra incluye el espacio previo. Un test contra `tokenizer.json` real, marcado como requisito de modelo, lo fija. |
| CJK y otros sistemas sin espacios | **Sin frontera.** Con pre-tokenizado por espacio, una oración china es una sola "palabra" mucho mayor que 96 tokens: no hay frontera dentro de la ventana y el corte debe caer al token, como hoy. Es el comportamiento correcto del respaldo, no un defecto. |
| Tasa de filas por nota de la importación | **Cierta como dato:** 52.080 filas / 3.814 notas = 13,7 (medición de la importación real de `engram export --all`, perfil dev, compilación debug de la 0.56.0). No dice qué parte son guardados de una sola pieza. |

## Problema (verificado contra el código)

1. **El borde ignora las palabras.** `plan_windows` (`token_splitter.rs:153-175`) es aritmética de índices: `[s, s+96)`, siguiente inicio `s + 72`. El texto sale de los offsets del primer y último token de la ventana (`:96-99`). Con un tokenizador de subpalabras, el primer token de la siguiente ventana puede ser la continuación de una palabra (`ies`, `vant Files` en el corpus importado). Dos efectos: el texto almacenado de la pieza empieza o acaba con un fragmento, y su vector (y el término que va a `memory_fts`) se calcula sobre ese fragmento.
2. **La cabecera hereda el defecto.** `split(content, 48, 0).next()` (`chat.rs:120`) termina donde caiga el token 48, y ese texto se guarda en `memory_piece.context` y se antepone al texto embebido de cada pieza posterior (`mod.rs:62-72`).
3. **El solape existe para que una oración cortada en el borde quede entera en una pieza** (`mod.rs:52-55`): 24 de 96 tokens, un cuarto, fijo. El paso real es 72 tokens por pieza.
4. **Cada pieza posterior embebe ~147 tokens por 96 propios** (48 de cabecera, el separador `HEAD_SEPARATOR` y el texto): un tercio es cabecera (`openspec/changes/archive/2026-10-15-long-save-ranking/design.md`, D1, línea 15). La cabecera es costo de inferencia en cada fila.
5. **Ya medido antes:** con piezas de 96/24 contra 424 tokens, la respuesta visible en `search_memory` de los 12 casos `long` pasó de 0,000 a 0,917 (`archive/2026-10-15-long-save-ranking/verify.md`).

Esto se ve en el corpus importado (piezas que empiezan en `ies`, `vant Files`), y ahí se vuelve irreparable por el camino normal: las notas importadas conservan sus piezas, y reimportar encuentra la nota presente.

## Lo que ya existe y se reutiliza

- `tokenizers` 0.22.2 con la regex `onig` (`Cargo.toml:52`): `Encoding::get_word_ids()` y `get_offsets()` ya vienen en el `encode(text, false)` que `split_by_tokens` hace (`token_splitter.rs:81`).
- `plan_windows` pura y sus pruebas exhaustivas (`token_splitter.rs:205-353`): el modelo para una variante pura que reciba las fronteras como `&[bool]`.
- `TokenSplitter::count` (`token_splitter.rs:41-46`): la medida del tope, para comprobar que ninguna pieza lo pasa.
- `join_pieces` y su test `the_pieces_of_a_split_save_join_back_into_its_text` (`pieces.rs:359`): comprueban que el solape sigue siendo reconocible.
- `translate_*` tests con `ChatChunker::with_fallback()` (`translate.rs:845`, `:878`, `:971`, `:1001`) y `ChatChunker::with_tokenizer` (`chat.rs:88`, sólo `cfg(test)`): el punto de entrada para un test de importación con subpalabras.
- La infraestructura de medición: `golden-mine synth --seed 7`, `rag-eval --diff` (con `--agent-view`), `--lexical-only` (`openspec/config.yaml`, `.claude/CLAUDE.md`).

## Enfoques posibles

### A. Frontera de palabra con `word_ids` (recomendado)

En `split_by_tokens`, después de `encode`, calcular un vector `starts_word[i]` (el token `i` abre una palabra: `i == 0` o `word_ids[i] != word_ids[i-1]`) y pasarlo a una variante pura de `plan_windows`:

- El fin de cada ventana es el mayor `e <= s + max` con `starts_word[e]` (o `e == n`). Si no hay ninguno en `(s, s+max]`, la palabra es más larga que el tope y el fin es `s + max` (corte duro, como hoy).
- El siguiente inicio es la primera frontera de palabra `>= e - overlap`, siempre `> s` para que el inicio avance.
- Con todas las fronteras verdaderas (el `word_tokenizer`) reproduce `plan_windows` exacto, así que ninguna prueba actual cambia.

Efecto en el tamaño: cada corte pierde en promedio menos de un token de ventana, así que el paso baja de 72 a ~71 y las filas suben menos del 2 % (~+0 a +1.000 sobre 52.080). La cabecera queda corregida por el mismo código.

- **A favor:** no cambia los topes; es puro y testeable; no depende de heurísticas de carácter. Cubre mensajes, cabecera, importación y el índice de docs.
- **En contra:** depende de que el pre-tokenizador real marque `word_ids` como se espera (se verifica con un test contra el `tokenizer.json` real, no con el fixture). Una "palabra" más larga que 96 tokens (URL, base64) se corta igual al token.

### B. Frontera por caracteres sobre el texto ya cortado

Buscar hacia atrás el último espacio en el texto de cada ventana. Evita `word_ids`, pero cambia el recuento de tokens de la pieza (queda menos de 96, sin medirlo) y rompe la regla "el tope es del tokenizador". **Descartado:** menos exacto que A y obliga a recontar.

### C. Brazo (a): menos solape donde el corte es fin de oración

Hoy 24 de 96 tokens fijos. La idea: si dentro de una zona de búsqueda hacia atrás desde el tope hay un fin de oración (punto, `?`, `!`, salto de párrafo, seguido de espacio), cortar ahí y empezar la pieza siguiente sin solape o con uno corto, porque el solape se justificó para que una oración partida quede entera en una pieza (`mod.rs:52-55`) y si el corte coincide con el fin de oración no hay oración partida.

- **Efecto en filas:** las piezas de un guardado crecen como `1/paso`. Con solape 0 el paso sube de 72 a ~96 (hasta −25 % de piezas); si sólo la mitad de los cortes encuentra un fin de oración, ~−10 % (de 52.080 a ~47.000). Son cotas, no datos: la fracción real se mide.
- **Restricción dura:** `join_pieces` necesita ≥ 16 caracteres de solape (`pieces.rs:57`); con solape 0 `read_memory` une con un salto de línea. Aceptar eso, o fijar un mínimo de solape de ≥ 16 caracteres (≈ 4 tokens), es parte de la decisión.
- **Riesgo:** una oración larga que cruza dos piezas sin solape pierde el puente. La medición lo ve sólo si los casos `long` la contienen.

### D. Brazo (b): cabecera menor de 48 tokens

La cabecera es un tercio del texto embebido de cada pieza posterior (~147 tokens: 48 + separador + 96). Con 24 baja a ~123 (−16 % por fila), sin cambiar el número de filas. Reduce además el texto guardado en `memory_piece.context`.

- **Riesgo medido antes:** "la clave del save llega al vector de cada pieza" gracias a esa cabecera (`design.md` D1, línea 15). Una cabecera corta puede dejar fuera el sujeto. Los casos `long` abren con `{key}` en la primera frase (`golden-mine/synth/mod.rs:513`), lo que es favorable al brazo y puede sobrestimarlo.
- Se prueba un barrido: 48, 32, 24, 16.

### Recomendación

1. **A** como arreglo de base y como punto de partida de los brazos. Entra con sus tests aunque los brazos no se adopten.
2. **C y D** como dos brazos independientes, cada uno detrás de una constante y medido contra `A` solo: se adopta sólo el que no pierde en los 12 `long` ni en las 46 del ancla (`rag-eval --diff`, también `--agent-view`), y se mide también el recuento de filas.
3. El respaldo por bytes **no** cambia en este cambio, para no mover el set `--seed 7`.
4. Todo, antes de la importación grande.

## Descartado

| Opción | Razón técnica |
|---|---|
| Subir el tope por encima de 96 (350-450 tokens) | Ya medido: un hecho de una oración pierde contra memorias cortas en un vector de 424 tokens; la respuesta visible pasó de 0,000 a 0,917 al bajar a 96. |
| Cambiar el respaldo por bytes a frontera de palabra | Mueve `fallback_pieces` y con él el set `long` de `golden-mine synth`, que debe salir idéntico para ser comparable. Sin tokenizador el corte es un respaldo transitorio. |
| Recortar la pieza a la palabra con un `trim` de caracteres (B) | Ver B: deja la pieza bajo el tope sin medirla y desvía el recuento de tokens. |
| Re-partir las notas ya importadas después | Una reimportación encuentra la nota presente y no cambia nada; cambiarlo sería otro mecanismo (borrar y reimportar) y toca el snapshot y la deduplicación. Por eso este cambio va antes. |
| Cambiar `E5_PASSAGE_PREFIX` | Ya medido y rechazado. |
| Añadir un campo de chunker al contrato de embeddings | No hace falta: ningún brazo cambia los vectores existentes, y un campo dispararía un re-embebido de toda la base. |

## Interacciones

- **import-linear y import-decision-pass.** Tocan `portability/import.rs`; este cambio toca `chunker/` (y, si hace falta, el test de `translate.rs`). No chocan en archivos. El orden relevante: se mide el recuento de filas con la importación en seco, que hoy tarda 910 s con 52.080 filas; con esos dos cambios la medición de cada brazo es viable.
- **Importación grande.** Debe ir después de este cambio, no antes: es la razón de su lugar en la hoja de ruta (las piezas importadas no se re-parten).
- **embed-throughput.** La inferencia es proporcional a los tokens embebidos. La cabecera (b) recorta ~16 % por fila de pieza y el brazo (a) baja el número de filas; juntos, del orden de −20 a −25 % de tokens embebidos (estimación, no medida), que es más que cualquier ajuste de lote. embed-throughput debe partir del corpus ya partido con estas reglas.
- **ranking-at-scale.** Menos piezas por resumen de sesión baja la cuota de esas piezas en el top 3 (hoy 19 de 36 casillas, el 53 %, con el 11 % de las filas). No sustituye al ajuste de lectura, que ve el mismo corpus: lo reduce.
- **Índice de docs.** `emit` usa el mismo `split`; un archivo se vuelve a leer sólo si cambia su sha256 o el modelo, así que los pasajes existentes no cambian hasta entonces. Si se prefiere no tocarlos, el arreglo entra detrás de un parámetro (decisión abierta).
- **Specs:** `openspec/specs` de chunking/ingest (cómo se parte un guardado largo) y la de importación si menciona las piezas. No cambia el contrato del agente ni el esquema.

## Alcance candidato

1. Variante pura de `plan_windows` con fronteras de palabra, y su uso en `split_by_tokens`, que alcanza la cabecera y el índice de docs.
2. Un fixture de subpalabras (WordPiece con `WhitespaceSplit`) compartido por las pruebas del divisor, de `split_save` y de la traducción.
3. La medición del recuento de filas y de tokens por nota sobre la importación en seco.
4. Los dos brazos (a) y (b), cada uno tras su constante, medidos; sólo se adoptan si no pierden.
5. Actualizar el comentario desfasado de `pieces.rs:58-59`.

### No-alcance

- El respaldo por bytes y el set `golden-mine synth --seed 7`.
- Los topes 96/24 y los 480 del texto entero.
- Re-partir notas ya importadas o guardadas.
- Cambios de esquema, del contrato de embeddings o del prefijo E5.
- El ranking de lectura de los resúmenes de sesión.

## Decisiones abiertas

Son del usuario.

- [ ] **Índice de docs.** ¿El arreglo entra en `TokenSplitter::split` (alcanza docs; sus pasajes cambian sólo cuando el archivo cambia, recomendado por simplicidad) o detrás de un parámetro que sólo `split_save` activa?
- [ ] **Brazo (a).** ¿Se prueba con solape 0 en fin de oración o con un mínimo de ≥ 16 caracteres, para que `join_pieces` siga uniendo sin salto? Y cuánto se busca hacia atrás desde el tope.
- [ ] **Brazo (b).** Valores del barrido de la cabecera (48, 32, 24, 16) y el criterio de "no pierde": ¿ningún caso `long` peor y ningún peor en el ancla, o un umbral de `search_visible`?
- [ ] **Medida del recuento.** ¿Se acepta la importación en seco como medida del número de filas, o se escribe una herramienta de sólo conteo en `clawg-eval`?
- [ ] **Qué hacer si ningún brazo gana.** Cerrar con A solo (recomendado) o seguir el barrido.

## Cómo se mide

**Tests RED (fallan hoy)**, con un fixture `WordPiece` sobre `WhitespaceSplit` (vocabulario con raíces y continuaciones `##`), porque ni `word_tokenizer` ni `word_level_tokenizer` pueden partir una palabra:

1. **Ninguna pieza empieza ni termina dentro de una palabra:** el byte inicial es 0 o sigue a un espacio, y el final es el fin del texto o precede a un espacio. Hoy falla.
2. **El tope se respeta:** `TokenSplitter::count(pieza) <= 96` para toda pieza.
3. **Una palabra más larga que el tope** se corta al token, termina y no pierde bytes.
4. **El solape sigue uniendo:** `join_pieces` rearma el texto original de un guardado largo (`pieces.rs:359` sobre el fixture).
5. **La cabecera** de `split_save` termina en frontera de palabra (hoy a mitad).
6. **Aritmética pura:** variante de `plan_windows` con fronteras: con todas verdaderas coincide con `plan_windows`; barrido exhaustivo de terminación y de cobertura sin huecos (como `token_splitter.rs:237-300`).
7. **Importación:** `translate` con `ChatChunker::with_tokenizer(fixture)` no produce una pieza que empiece a mitad de palabra.
8. **Sin cambio:** las pruebas del respaldo por bytes siguen verdes, y `golden-mine synth --seed 7` da un archivo idéntico byte a byte (R4 de `archive/2026-10-15-long-save-ranking`).

**Medición:**

- Recuento: filas y tokens embebidos por nota con la importación en seco de la exportación completa, antes y con cada brazo.
- Calidad: `rag-eval --real --agent-view --golden <set>` con el set de `golden-mine synth --seed 7` sobre `docs/eval/golden.json` (ignorado por git; requiere el modelo real), leyendo `long`: `answers.search_visible`, `answers.context_visible`, `search.hit_rate`; y `rag-eval --diff` contra el ancla de 46 casos, con y sin `--agent-view`. Con 12 casos `long`, una conclusión necesita ≥ 6 moviéndose en el mismo sentido: menos es "no distinguible", no "sin diferencia". Se elige el valor con `by_split.calibration` y se informa `by_split.test`.
- Un test contra el `tokenizer.json` real (requisito: el modelo descargado) fija si el offset del primer token de palabra incluye el espacio previo.

## Riesgos

- **El offset del primer token real incluya el espacio previo.** Entonces la pieza empezaría con un espacio; se recorta en el borde o se ajusta el inicio. Lo detecta el test contra el tokenizador real.
- **Un pre-tokenizador distinto al supuesto** haría que `word_ids` agrupe más o menos de lo esperado (caminos, identificadores): el efecto sería menos frontera disponible, no un corte incorrecto, y la medición del recuento lo muestra.
- **El brazo (a) pierda el puente de una oración larga** o rompa `read_memory` por solape bajo el mínimo de `join_pieces`. Se mide en `long` y con un test de rearme.
- **El brazo (b) pierda la clave del guardado** en el vector de las piezas posteriores. Los casos `long` abren con la clave en la primera frase y pueden sobrestimar la cabecera corta.
- **Doce casos `long` es poca potencia.** Un empate no prueba igualdad; se declara como "no distinguible".
- **Corpus cuya mayoría son guardados de una pieza:** el efecto en filas sería menor que el estimado; la medición con la importación en seco lo da.
- **Aterrizar tarde.** Si la importación grande ocurre antes, sus piezas quedan con los bordes actuales sin vía de reparación.
