# docs-index-speed — Diseño (paso A1)

Sólo A1, ordenar por tokens dentro de un doc. El contrato, los umbrales y «se queda si» están en la [propuesta](proposal.md) (decisiones 2 y 4, paso A1), y qué puertas abrieron, en [verify.md](verify.md). A2 se diseña con A1 medido.

## Enfoque

`embed_passages` (`crates/clawg-core/src/memory/project_docs/mod.rs:645`) recibe también el tokenizer del refresco. Si un doc tiene más de `EMBED_BATCH` textos para embeber (`crates/clawg-core/src/memory/embedder.rs:184`), los cuenta como los recibe el modelo y los manda en una sola llamada, del más largo al más corto. Cuando vuelven los vectores y la cuenta da bien (`mod.rs:657`), los pone otra vez en el orden de los pasajes. `to_unit`, `fill` (`mod.rs:601`) y `replace_doc` no cambian.

```
lookup ─ to_embed ─┬─ ≤ EMBED_BATCH, sin tokenizer o conteo fallido ─→ embed_batch(to_embed)
                   └─ passage_order → embed_batch(ordenados) → in_passage_order
                   ─→ cuenta, to_unit, fill                     (todo dentro de la fase embed)
```

**Dónde (D1).** El orden vive en el refresco, nunca en `E5Embedder::embed_batch` (`embedder.rs:434`). Ponerlo en el embedder ordenaría también los guardados, la importación, las consultas y todos los casos de `e5_golden`, y la propuesta los deja fuera (decisión 2 y no-alcance).

## D2. La clave y el tokenizer

La clave es la que contó docs-index-cost. El texto lleva el prefijo de pasaje (`apply_role_prefix`, `embedder.rs:196`), se tokeniza y se corta a la ventana (`window_encodings`, `:240`), y cuenta su `Encoding::len`, lo que rellena `pad_right` (`:259-261`). `EvalDocIndex::passage_lengths` arma los largos con esas mismas tres piezas (`crates/clawg-core/src/eval_api.rs:1470`, `:1399`). Por eso el orden de producción es el de `padded_sorted` (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:96`), y `padded_ran` tiene que dar `padded_sorted_in_doc`.

| Clave | A favor | En contra |
|---|---|---|
| **Largo en la ventana, con prefijo** (elegida) | Iguala por construcción lo que mide el aviso (D6). El relleno depende sólo de la secuencia de largos, así que dos órdenes no crecientes en esta clave rellenan lo mismo aunque desempaten distinto | Los docs de más de una tanda se tokenizan dos veces, porque el embedder tokeniza otra vez en su hilo (`embedder.rs:505`). Cuesta a lo sumo `count_seconds`, 1,9 s en R |
| `TokenSplitter::count_all` del `indexed_text` (`crates/clawg-core/src/memory/chunker/token_splitter.rs:51`), como la importación (`crates/clawg-core/src/memory/portability/import.rs:2527`) | Ya existe | No lleva prefijo ni ventana. El prefijo puede tokenizar distinto en la frontera, y todo lo que pasa de la ventana queda en 512. El orden deja de ser monótono en la clave del modelo, y la igualdad de «se queda si» puede fallar en algunas tandas |
| Bytes | Gratis | No es lo que el modelo rellena, así que la igualdad no se cumple |

**El tokenizer.** `refresh` ya recibe `splitter: &TokenSplitter` (`mod.rs:489`), y ese tokenizer es el del embedder. En el daemon es la única copia del proceso (`crates/clawg-core/src/runtime.rs:144-151`; `refresh_docs` la pasa en `crates/clawg-cli/src/daemon/api.rs:3017-3019`). En la evaluación, `EvalDocIndex::open` arma el splitter con el mismo tokenizer con el que después cuenta (`eval_api.rs:1427-1432`).

| Opción | A favor | En contra |
|---|---|---|
| **`TokenSplitter::tokenizer() -> Option<&Tokenizer>`** (elegida) | Un método en un tipo `pub(crate)`. Ni `refresh` ni sus llamadores cambian de firma | El splitter expone lo que envuelve, que hoy sólo ven los tests (`tokenizer_ptr`, `token_splitter.rs:34-37`) |
| Un parámetro `Option<&Tokenizer>` en `refresh` | Ninguna visibilidad nueva | Cambian las dos llamadas y cada test que llama `refresh`, y el splitter y el tokenizer podrían venir de copias distintas |
| Que el splitter calcule los largos de ventana | Encapsula | Mete la ventana y el prefijo del embedder en el partidor |

**Sin tokenizer o si contar falla, el doc va en el orden de sus pasajes, como hoy.** Así pasa con `TokenSplitter::fallback()`, en los tests del refresco y con `CLAWG_TEST_EMBEDDER=fake` sin tokenizer. El conteo de respaldo (bytes / 4, `token_splitter.rs:42-47`) no es lo que el modelo rellena, y sin tokenizer no hay modelo real que rellene. `window_encodings` es determinista, así que si contar falla, el embedder falla igual con los mismos textos: da `DocIndexError::Embed`, que corta el refresco como hoy (`mod.rs:550`). Un vector inutilizable sigue salteando sólo su doc (`mod.rs:545-549`), porque una llamada sigue siendo un doc.

## D3. Contar sólo cuando importa

Si `to_embed.len() ≤ EMBED_BATCH`, no se cuenta ni se reordena: una tanda sola rellena igual en cualquier orden (`one_batch_pads_the_same_in_any_order`, `index_cost.rs:355`). Lo que se mide es `to_embed`, no el total de pasajes: un doc cambiado de 40 pasajes al que le quedan 5 por embeber (RM5) no cuenta nada.

## D4. Las piezas puras

- **`longest_first` se mueve de `import.rs:2537` a `embedder.rs`**, al lado de `EMBED_BATCH`, junto con su test (`import.rs:5222`). La importación (`:2529`, `:5486`) y `eval_api::longest_first` (`eval_api.rs:1085`) la toman de ahí. La alternativa era llamarla en `import`, pero eso ata el índice de docs al módulo de la importación por una función que trata de cómo se arman las tandas.
- **`in_passage_order<T>(order: &[usize], sorted: Vec<T>) -> Vec<T>`** es nueva y privada, en `project_docs/mod.rs`, al lado de `fill`. Pone `sorted[j]` en la posición `order[j]`, después de comprobar la cuenta y antes de `to_unit`.
- **`passage_order(tokenizer: Option<&Tokenizer>, embedder: &dyn Embedder, texts: &[String]) -> Option<Vec<usize>>`** es `pub(crate)`. Da `None` con `EMBED_BATCH` textos o menos, sin tokenizer o si contar falla. Si no, da el `longest_first` de los largos de D2, con los empates en el orden de los pasajes. Tokenizar es trabajo de CPU, sin disco ni red, y la regla y las permutaciones se prueban sin el modelo.

## D5. Dónde cae el costo de contar

| Opción | A favor | En contra |
|---|---|---|
| **Dentro de `embed_passages`: en la fase `embed`, en el mismo hilo** (elegida) | `phases` sigue sumando `index_seconds` (`lap`, `mod.rs:570`), y «se queda si» lee un `embed` neto del costo de contar. `passages` (`mod.rs:529`) ya tokeniza el mismo doc en ese hilo y cuesta más (`split` 12,0 s contra 1,9 s en R) | Ocupa el hilo del runtime unos milisegundos por cada doc grande |
| En `lookup`, o en una fase `order` nueva | Se vería aparte | `embed` bajaría por el ahorro bruto, o cambiarían `DocPhases`, la línea del daemon (IC2) y el bloque `index` |
| En una tarea bloqueante, como la importación (`import.rs:2522`) | No ocupa el runtime | Pide copias `'static` de los textos y del tokenizer para un costo menor que el del partido, que sigue en el hilo |

## D6. El aviso de `rag-eval`

`padding_warning` (`index_cost.rs:151`) compara `padded_ran` con `padded_sorted_in_doc` cuando `from_memo == 0`, que es el orden en que embebe el refresco. `padded` sigue en el orden de los pasajes: es el contrafáctico que muestra lo que ahorró A1. Ningún campo cambia de nombre. Se corrigen los comentarios de `Tokens` (`index_cost.rs:103-122`), de `EvalDocLengths` (`eval_api.rs:1368`) y de `passage_lengths` (`:1442-1446`), porque los largos salen en el orden de los pasajes y no en el que se embebieron. Por lo mismo, `passage_lengths_follow_the_stored_passages_in_batch_order` (`eval_api.rs:2887`) pasa a decir «ordinal».

| Opción | A favor | En contra |
|---|---|---|
| **Los mismos nombres, y el aviso mira `padded_sorted_in_doc`** (elegida) | Las fórmulas de las puertas (la de A1, y la de A2, que divide por `padded`) dan lo mismo en un archivo viejo y en uno nuevo | `padded` deja de ser «lo que corre el modelo» |
| `padded` pasa al orden de producción y un campo nuevo guarda el de los pasajes | `padded` sigue siendo lo que corre | Un mismo nombre con dos sentidos según la fecha del archivo; en uno nuevo, la puerta de A1 daría 0 |

`--diff` no decide entre las dos, porque `DocsFile` (`crates/clawg-eval/src/bin/rag-eval/docs.rs:411`) no lee el bloque `index`.

## D7. Los bits (propuesta, decisión 4)

- **`e5_golden`** (`crates/clawg-core/tests/e5_golden.rs`) gana el caso `a batch of 17, longest first`, que va después de `a batch of 17` (`:206`). Son los mismos 17 textos (`:176-183`), en una sola llamada y en el orden que da `eval_api::passage_order(embedder: &EvalEmbedder, counter: &EvalTokenCounter, texts: &[String]) -> Vec<usize>`, con el `EvalTokenCounter` que `e5_golden` ya usa (`e5_golden.rs:18`, `:215`). Esa puerta nueva delega en `project_docs::passage_order`, y sin tokenizer da la identidad. Así la fixture fija el orden del refresco, no una copia escrita en el test. El caso se escribe con `CLAWG_BLESS=1` (`:13-14`) y sólo en la máquina cuya huella nombra la fixture, hoy Windows `x86_64` con AVX2, AVX-512F y FMA; el diff tiene que mostrar sólo la línea nueva. En otra CPU se compara con 1e-6 (`:26`, `:34-40`).
- **`a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order`** lee sólo la fixture. Por cada texto, la diferencia mayor entre los dos casos MUST ser ≤ 1e-6, y el test imprime cuántos textos dan los mismos bits. Así la regla de aceptación se puede ejecutar y corre con la suite, como `the_fixture_holds_one_vector_of_the_model_size_for_every_text` (`:111`). El commit que lo agrega trae la fixture ya escrita.
- **La prueba ignorada** va en los tests de `project_docs/mod.rs` y sigue el modelo de la importación (`import.rs:5411-5597`).
  - **Qué necesita:** `CLAWG_E5_MODEL_DIR` (`<data dir>/models/multilingual-e5-small`), el tokenizer verificado y el modelo en caché, sin descargar nada (`import.rs:5451`, `:5515-5517`).
  - **El corpus:** este repositorio, desde `CARGO_MANIFEST_DIR/../..`, recorrido y partido como lo hace el refresco (`corpus::walk`, `corpus::read`, `redact`, `passages`). No abre un store ni usa el temporal.
  - **Qué hace:** toma docs de más de `EMBED_BATCH` pasajes, enteros y repartidos por ruta, hasta `CLAWG_EMBED_PASSAGES` pasajes (5000 por omisión). Llama `embed_passages` con tokenizer y sin él, `CLAWG_EMBED_RUNS` veces (3 por omisión), y alterna cuál va primero.
  - **Qué imprime:** cuántos blobs, lo que el índice guarda, dan los mismos bits; la diferencia mayor; y la mediana de cada orden.

El bless y la prueba ignorada necesitan el modelo real del perfil dev. Corren sólo con autorización y nunca en la corrida normal de la suite ni en CI.

## Medición

«Se queda si» se lee en corridas `rag-eval --real --docs --no-cache` de release en `target-test`. La base y el paso se alternan, y las dos usan el mismo `--root` (`crates/clawg-eval/src/bin/rag-eval/cli.rs:235`), así indexan el mismo corpus aunque el de R haya crecido. Mientras tanto no corre otra compilación ni otra corrida ([verify.md](verify.md), «Antes de leer el tiempo de A1»). La base es un worktree descartable que se borra junto con su directorio de build. `padded_ran == padded_sorted_in_doc` se compara dentro de cada corrida (D6).

## Deltas

- **`specs/project-docs`, «Refresco incremental»:** describe el orden (D2 y D3), con sus escenarios y tests. En IC1, `embed` pasa a incluir contar y ordenar.
- **`specs/eval`, IC6:**
  - `padded` es el orden de los pasajes, y `padded_sorted_in_doc` el del refresco.
  - El aviso y su escenario comparan `padded_ran` con `padded_sorted_in_doc`.
  - `longest_first` se cita en `embedder.rs`.
- **`specs/storage`, «Los mismos vectores que fastembed» (`:877`):** el caso nuevo, el test sobre la fixture y la tolerancia entre órdenes.
- **`docs/reference/daemon.md`:** una frase en el párrafo del índice de docs, como la que `commands.md` dice de la importación.

| Archivo | Cambio |
|---|---|
| `crates/clawg-core/src/memory/embedder.rs`, `memory/portability/import.rs` | `longest_first` y su test se mudan |
| `crates/clawg-core/src/memory/chunker/token_splitter.rs` | `tokenizer()` |
| `crates/clawg-core/src/memory/project_docs/mod.rs` | `passage_order`, `in_passage_order`, `embed_passages` con tokenizer, sus tests y la prueba ignorada |
| `crates/clawg-core/src/eval_api.rs` | `passage_order`; `longest_first` desde `embedder`; comentarios y el nombre de un test |
| `crates/clawg-core/tests/e5_golden.rs`, `tests/fixtures/e5_golden.json` | El caso nuevo, el test sobre la fixture y la fixture |
| `crates/clawg-eval/src/bin/rag-eval/index_cost.rs` | `padding_warning`, los comentarios de `Tokens` y su test |

## Tests que hoy fallan

Los nombres son los de la [spec](spec.md), IS1 a IS5. Usan embedders falsos, el tokenizer de palabras (`crates/clawg-core/src/memory/chunker/mod.rs:14`) por `try_refresh` con `Some(word_tokenizer())`, y `crate::test_dir::test_dir()`. Hay dos que la spec no nombra porque son piezas del diseño:

- `a_reordered_doc_writes_what_an_unordered_one_wrote` compara el `dump` (`mod.rs:1470`) byte a byte con el de un refresco sin tokenizer. `FakeEmbedder` saca el vector del texto, así que un vector devuelto al pasaje equivocado cambia el dump. `a_doc_over_a_batch_reaches_the_embedder_longest_first` usa `Recording` (`mod.rs:1426`).
- `passage_order_is_the_order_a_refresh_embeds_in` (`eval_api.rs`): la puerta de `e5_golden` da el mismo orden que el refresco.

## A2, después

A2 tiene su diseño en [«Paso A2»](#paso-a2), al final de este archivo. Usa sin cambios `passage_order`, `in_passage_order` y el conteo dentro de `embed`.

## Fuera de alcance

Con una última tanda corta, ordenar de más largo a más corto rellena más que no ordenar: en `sorting_can_pad_more_when_the_last_batch_is_short` (`index_cost.rs:368`) da 1610 contra 260. Mandar primero la tanda incompleta, una partición óptima o elegir en cada doc el orden que rellena menos ahorrarían algo más. Pero la puerta y «se queda si» se midieron con `longest_first`, así que cualquiera de esas ideas sería otro cambio, con su propio campo en `index.tokens`.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| El orden mueve bits | D7. Si la diferencia mayor es ≤ 1e-6, se reescribe la fixture y se vacía `docs/eval/cache/`; si pasa de 1e-6, el paso cae y el tema pasa a 17 (propuesta, decisión 4) |
| El test sobre la fixture falla hasta el bless | Su commit espera la corrida autorizada en la máquina de la fixture |
| La varianza del reloj (+26,8 % entre D y R) supera el −10 % que se pide | Corridas alternadas sobre el mismo `--root` |
| Algún doc rellena más ordenado | Se ve en `by_area`; arreglarlo queda fuera de alcance |

# Paso A2

Ordenar entre docs con una cola acotada. El umbral, el costo y «se queda si» están en la [propuesta](proposal.md) (paso A2, decisiones 2 y 4). La base contra la que se compara es la corrida de A1, y el techo es el que midió [verify.md](verify.md) en «A2: el techo, antes del código»: −21 % de `embed` con −19,2 % de tokens rellenados. Las decisiones siguen la numeración de A1.

## Enfoque

Hoy `refresh` embebe y escribe un doc antes de leer el siguiente (`crates/clawg-core/src/memory/project_docs/mod.rs:515-563`), así que cada doc de 16 textos o menos corre su propia tanda incompleta.

Con A2:
1. Cada doc del plan se lee, se enmascara, se parte y busca sus vectores como hoy (`:521-542`), y entra a una cola.
2. Al vaciarse la cola, los textos de sus docs van en el orden del plan al embedder, en una sola llamada y ordenados por `passage_order` (`:688-694`) sobre la lista entera.
3. Vuelven con `in_passage_order` (`:698-704`) y se reparten por doc.
4. Cada doc pasa por `to_unit`, `fill` (`:603`) y `replace_doc` (`:759`) como hoy, en el orden del plan.

`E5Embedder` no cambia (D1).

```
doc del plan: read → mask → split → lookup → ¿vaciar antes? → entra → ¿llegó al límite? → vaciar
vaciar: textos de la cola → passage_order → embed_batch → in_passage_order
        → por doc, en orden: to_unit (inutilizable → failed) → fill → replace_doc
fin del plan: vaciar; después, remove_docs
```

## D8. La cola

- **Qué cuenta el tope:** los pasajes que la cola retiene, se reutilicen o no.
- **Cuándo se vacía:**
  - antes de que entre un doc que la pasaría de su límite, si no está vacía;
  - cuando llega al límite;
  - al terminar el plan.
- **El límite** empieza en `EMBED_BATCH` y se duplica con cada vaciado hasta el tope, `QUEUE_PASSAGES` (D11).
- **Un doc más grande que el límite** se vacía solo: para él, A2 es A1. La excepción es la rampa: si la cola no estaba vacía, el vaciado de antes duplica el límite, y un doc de entre límite + 1 y 2 × límite − 1 pasajes queda por debajo del nuevo y los docs que siguen se le suman. En el tope un doc de más de `QUEUE_PASSAGES` va siempre solo. La cuenta de D9 usa la misma pieza, así que la tabla ya lo incluye.
- **Los vectores vuelven por posición**, como hoy. Los textos se concatenan doc tras doc, `in_passage_order` deja los vectores en el orden de esa concatenación, y cada doc toma los suyos según cuántos textos tiene.

La decisión la toma una pieza pura. Su único estado es lo que retiene la cola y cuántos vaciados se hicieron, y con el tamaño del doc que llega dice si vaciar antes y si vaciar después. La recorren `refresh` y `eval_api::queue_groups` (D14), así que la evaluación agrupa como el refresco por construcción.

| Opción | A favor | En contra |
|---|---|---|
| **Tope en pasajes retenidos** (elegida) | Acota siempre la memoria (D13). En una primera indexación, los pasajes son los textos a embeber, así que la evaluación lo reproduce | En un refresco incremental, un pasaje reutilizado ocupa lugar sin embeberse |
| Tope en textos a embeber | Sigue el trabajo del modelo | Un doc con todos sus pasajes reutilizados no cuenta nada. Tras un cambio de espacios en todo el árbol, la cola retendría el árbol entero, con sus blobs |
| Tope en tokens o en bytes | Sigue el relleno o la memoria | En tokens, hay que contar antes de decidir, y sin tokenizer no habría tope. En bytes, no es lo que rellena el modelo ni lo que mide la tabla de D9 |
| **Los docs de más de una tanda entran si caben** (elegida) | El techo se midió ordenando todos los pasajes | La cola retiene más |
| Los docs de más de una tanda van solos, como en A1 | Menos memoria | Recupera las tandas incompletas de los docs chicos, pero no el relleno entre docs grandes |

## D9. Cómo se elige el tope, antes de ver los números

El tiempo no es una fracción fija de los tokens rellenados. Con A1 bajó 0,75 de lo que bajaron los tokens (verify.md:111), y ordenando entre docs, 1,09 (21,0 % frente a 19,2 %, :291). La causa probable, sin medir, son las tandas (:292). Por eso el tope se elige con tres cantidades que no necesitan el modelo y con una regla que se fija acá.

**La cuenta.** `padding_and_batches_by_queue_cap` es una prueba ignorada de `project_docs/mod.rs`, junto a `sorting_across_docs_against_sorting_in_each_doc` (`:2450`).
- **Qué necesita:** sólo el tokenizer verificado de `CLAWG_E5_MODEL_DIR` (`load_verified_tokenizer`, como `real_model`, `:2345`) y los prefijos de `expected_contract()` (`crates/clawg-core/src/memory/embedder.rs:157-164`). No abre una sesión de ONNX. `CLAWG_DOCS_ROOT` cambia el árbol que recorre; por omisión, es este repositorio.
- **Cómo recorre:** parte como `repo_docs` (`mod.rs:2353-2366`), en orden de ruta, que es el plan de una primera indexación (`corpus.rs:110-112`; `plan_refresh` conserva el orden del disco, `mod.rs:194-198`). Cuenta con `passage_token_counts` (`embedder.rs:268-279`).
- **Qué órdenes compara:** el de los pasajes; en cada doc, como A1; la cola con tope c ∈ {64, 128, 256, 512, 1024, 2048, 4096}, armada por la pieza de D8; y todo junto.
- **Qué imprime de cada orden:**
  - **P**, los tokens rellenados: filas × el más largo, por tanda.
  - **B**, las tandas: ⌈n/16⌉ por llamada.
  - **Q**, filas × el más largo al cuadrado, por tanda, porque la atención crece con textos × tokens² (`embedder.rs:180-181`).
  - Los pasajes de la llamada más grande.
  - **H**, los bytes que retiene el vaciado más grande (D13).
- **La muestra del techo:** imprime también P, B y Q en cada doc y todo junto sobre la muestra que toma `sorting_across_docs_…` (`mod.rs:2455-2458`).

**Las tandas del techo, que su corrida no imprimió.**
- **Todo junto:** es una llamada de 5261 textos, así que da ⌈5261/16⌉ = 329 tandas (`embedder.rs:460`), sin contar nada.
- **En cada doc:** es Σ ⌈n/16⌉ sobre sus 197 docs. Lo da la cuenta con `CLAWG_DOCS_ROOT` en un worktree descartable de `0c9d49a`, el commit de esa corrida. El número vale sólo si la cuenta reproduce P = 1 998 461 y 1 614 977 (verify.md:280). El worktree se borra al terminar.

**La regla.** Con X ∈ {P, B, Q}:
- ρ_X = (X_en_cada_doc − X_todo) / X_en_cada_doc, en el corpus y en la muestra.
- R_X(c) = (X_en_cada_doc − X(c)) / (X_en_cada_doc − X_todo): la parte de lo que recupera el techo que recupera la cola. Vale 1 si el denominador no es positivo.
- previsto(c) = 0,210 × min(1, min_X ρ_X(corpus) / ρ_X(muestra)) × min_X R_X(c), donde 0,210 = 1 − 0,790, la mediana del techo (verify.md:284).
- **Se elige el menor c con H(c) ≤ 16 MiB y previsto(c) ≥ 0,18.** Si ninguno cumple, A2 se cierra sin código de producción y `verify.md` guarda la tabla.

**Por qué así:**
- **Los pesos no hacen falta.** Si el tiempo de embeber es α·P + β·B + γ·Q, con pesos no negativos, más lo que el orden no toca, lo que ahorra c es al menos min_X R_X(c) veces lo que ahorra ordenar todo, sean cuales sean α, β y γ.
- **El segundo factor lleva el 21 % de la muestra al corpus.** La muestra salió representativa en tokens (19,2 % frente a 19,7 %, verify.md:290), pero en tandas no se midió.
- **0,18 es el 15 % de «se queda si» más 3 puntos.** Es algo menos que lo que erró en A1 la proyección sólo por tokens (16,9 % frente a 12,65 %), porque esta regla ya cuenta las tandas.
- **El menor c**, porque con el tope crecen la memoria, el trabajo que se pierde al parar (D10) y el escalón con que los docs se vuelven buscables (D11).

| Opción | A favor | En contra |
|---|---|---|
| **Las tres reducciones, sin ajustar pesos** (elegida) | No depende de un ajuste, y vale para cualquier mezcla de relleno, tandas y atención | Pide de más si un término no pesa |
| Ajustar k = β/α con los dos órdenes del techo | Da una predicción puntual | Despejando, k = (P₂ − 0,790·P₁) / (0,790·B₁ − 329), con P₁ y B₁ en cada doc y P₂ todo junto. Hay 120 docs de una tanda y 77 que reparten a lo sumo 5141 pasajes, así que B₁ ≤ 518 y el denominador vale a lo sumo 80 tandas; sólo es positivo con B₁ > 416. Un error chico en B₁ multiplica k. A1 no separa los pesos: sus dos ramas corren las mismas tandas |
| Sólo tokens | Ya existe (`padded_sorted_all`) | Esa proyección falló en los dos sentidos (verify.md:267, :291) |
| Un campo de `rag-eval` | Queda en cada archivo | `rag-eval` cuenta los largos desde el store, después de indexar con el modelo (`crates/clawg-core/src/eval_api.rs:1471-1497`). Además, una lista de topes hipotéticos no describe una corrida |

## D10. Fallos

| Fallo | Qué pasa |
|---|---|
| Un vector inutilizable | El vaciado devuelve, por doc, sus blobs o su `DocIndexError::Vector`. Ese doc va a `failed` y los demás del vaciado se escriben, como hoy (`mod.rs:545-551`) |
| El embedder falla o devuelve otra cantidad (`Embed`, `VectorCount`, `:663-669`) | El refresco termina, como hoy. `E5Embedder::embed_batch` es todo o nada: vuelve con el error de la primera tanda que falla (`embedder.rs:472`). Ningún doc del vaciado queda escrito; los de vaciados anteriores, sí |
| Falla el store en `replace_doc` (`:559`) | El refresco termina. Los docs del vaciado escritos antes quedan, y los demás siguen como estaban |
| La tarea se cancela, como al apagar | Se pierden los vectores que volvieron y no se escribieron: a lo sumo un vaciado, que al ritmo de A1 (D11) son tope / (10,7 a 17,3) segundos de embeber. `replace_doc` corre entero en el hilo de la conexión, así que cada doc queda viejo o nuevo (`:757-758`) |

Lo que no se escribió conserva su huella anterior, o no tiene ninguna, así que el refresco siguiente lo vuelve a planear (`plan_refresh`, `:190-203`): el índice se deriva del disco.

`Embed` y `VectorCount` conservan su forma y nombran el primer doc del vaciado, y `VectorCount.passages` cuenta los textos del vaciado. Así siguen valiendo `a_failed_embedding_keeps_the_doc_as_it_was` (`:1221`) y `a_short_batch_still_aborts_the_refresh` (`:1314-1325`). El error es del embedder, no del doc. Había dos alternativas:
- Nombrar todos los docs: el mensaje no tendría cota y llega al `warning` de cada búsqueda (`crates/clawg-cli/src/daemon/api.rs:3184`).
- Agregar una cuenta de docs: cambia el mensaje.

**`RefreshReport` sale igual para las mismas entradas.** Sus campos son sumas, y `failed` sigue el orden del plan, porque los vaciados van en ese orden y los docs de cada vaciado también. Con un error, el resultado es un `Err` en los dos casos. Lo que cambia es qué docs quedaron escritos antes del error, y eso no está en el informe.

## D11. Escritura y latencia

- **Una transacción por doc**, en el orden del plan, después de su vaciado. Es la misma secuencia de escrituras de hoy, así que los ids de fila salen iguales. Como `fuse` desempata por id (`mod.rs:392`), los empates del ranking también se ordenan igual.
  - Una transacción por vaciado no ganaría nada, porque escribir es el 0,15 % del total (verify.md:17), y un fallo del store perdería el vaciado entero.
- **Cuándo un doc se vuelve buscable:** hoy, después de embeberlo; con A2, después de su vaciado.
- **La primera indexación.**
  - Con el índice vacío, una búsqueda espera hasta `FIRST_INDEX_WAIT` (20 s, `api.rs:3084`, `:3167-3177`) y responde con lo que haya. El comentario de `:3164-3166` dice por qué espera: una respuesta vacía se lee como «los docs no dicen nada».
  - Las corridas de A1 embebieron 18 885 pasajes en 1088,8 a 1760,6 s (verify.md:77, :82-87). Son de 10,7 a 17,3 pasajes/s, o sea, de 214 a 347 pasajes en 20 s.
  - Con un tope fijo de 256 o más, el primer vaciado puede no terminar en 20 s, y la primera búsqueda de un repositorio grande no encuentra nada.
  - Con el límite que se duplica (16, 32, 64, 128…), el primer doc se escribe después de una tanda. En 20 s quedan escritos de 112 a 240 pasajes, entre la mitad y dos tercios de lo de hoy.
  - Que el límite crezca tiene un costo: los primeros vaciados ordenan en ventanas chicas. La tabla de D9 ya lo incluye, porque usa la misma pieza.
- **Un refresco incremental chico** (`REFRESH_GRACE`, `api.rs:3090`) cabe en el primer vaciado, que cierra al terminar el plan: queda como hoy.

| Opción | En contra |
|---|---|
| **Un límite que se duplica hasta el tope** (elegida) | Unas líneas más en la pieza pura, y su prueba |
| Un tope fijo | La primera respuesta vacía de arriba |
| Un tope acotado por la latencia | Ata el ahorro a `FIRST_INDEX_WAIT`, que no es lo que se mide |
| Vaciar por tiempo | No es determinista: la evaluación no podría reproducir los grupos, y el aviso no podría compararlos (D14) |

## D12. Tiempos

- `read`, `mask`, `split`, `lookup` y `write` se miden por doc con `lap` (`mod.rs:572-575`), como hoy.
- El `lap` de un vaciado, que incluye contar y ordenar (D5), se reparte entre sus docs según la parte de los textos que aporta cada uno.
  - El reparto es en nanosegundos enteros, y el resto va al último doc.
  - Así `RefreshTiming::phases` (`:144-148`) sigue sumando todo `index_seconds`, y la línea del daemon (`api.rs:3029-3042`, IC2) no cambia de forma.
- Un `DocTiming` se agrega cuando su doc se escribe o falla. El de un doc que no se pudo leer se agrega en el momento. Así, `timing.docs` sigue el orden en que terminan los docs. Ningún consumidor depende de ese orden: las pruebas ordenan (`mod.rs:1123`, `:1305`), y `Phases::of` y `ByArea::of` suman (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:22-35`, `:245`).

| Opción | A favor | En contra |
|---|---|---|
| **Por textos** (elegida) | Una sola regla, que no necesita largos. Dentro de una tanda todas las filas se rellenan al mismo largo, así que cuestan lo mismo (`pad_right`, `embedder.rs:283-303`). La puerta lee sólo el total. Como las áreas son tramos de rutas (`area_of`, `index_cost.rs:175-183`), un vaciado mezcla áreas sólo en los bordes y donde un manifiesto o un README queda entre fuentes | Entre las tandas de un vaciado, un doc de pasajes largos paga el promedio |
| Por tokens rellenados | Más fiel entre tandas | Los largos existen sólo con tokenizer y más de 16 textos, así que harían falta dos reglas |
| Todo al doc que dispara el vaciado | Trivial | `by_area` deja de significar algo |
| Un campo de vaciados en `RefreshTiming` | Exacto | Cambian `DocPhases`, IC2 y el bloque `index` |

## D13. Memoria

- **Cada doc en cola retiene:**
  - su ruta, su tipo y su hash;
  - sus `passages`, con breadcrumb y cuerpo;
  - el plan de reutilización, con los blobs reutilizados (1536 B cada uno: 384 f32);
  - los textos a embeber, otra copia de cada texto;
  - sus fases.
- **El vaciado suma:**
  - los vectores f32, 1536 B cada uno;
  - mientras cuenta, las copias con prefijo y las codificaciones del tokenizer de `passage_token_counts`, que cuenta en tandas de `EMBED_BATCH`: coexisten las de una tanda (unos 70 a 80 B por token, a lo sumo 512 tokens por texto, o sea unos 40 KB por texto y 640 KB por tanda), no las de la llamada.

  Los textos se ordenan moviéndolos, sin copiarlos: hoy se clonan (`mod.rs:660`), y el vaciado es su dueño.
- **La cota:** H es el máximo, sobre los vaciados, de Σ (3 × bytes del texto + 1536). Cuenta el pasaje, el texto a embeber, la copia del conteo y el vector, y es una cota porque la copia y el vector no coexisten. Los vectores f32 se consumen al armar los blobs (`into_iter`), así que no viven junto a ellos. Lo que H no cuenta es el máximo de una tanda de codificaciones y el blob que se arma, que son del orden de 1 MiB. IS17 sigue siendo la puerta.
  - Los tamaños tienen tope: un pasaje tiene a lo sumo 480 tokens (`crates/clawg-core/src/memory/chunker/mod.rs:26`); un pedazo de código, 1200 caracteres (`project_docs/code.rs:13`) y un breadcrumb de hasta 256 B (`chunk.rs:12`); un doc, 256 KiB (`corpus.rs:14`).
  - La cuenta de D9 mide H en este corpus en lugar de suponer bytes por token.
- **La cota en pasajes:** hoy es el doc más grande; con A2, el mayor entre el tope y el doc más grande.
- **Por qué 16 MiB:** el 5 % de la menor memoria privada estable medida con el modelo cargado es 37 MB (739 MB, [exploración de embedder-footprint](../../embedder-footprint/exploration.md), línea 25). La mitad deja lugar a lo que el asignador no devuelve.

**El pico (+5 %), con autorización.**
1. Se usa el perfil dev, sin otro daemon dev en marcha, porque el store tiene un solo dueño.
2. Los builds son release, como en A1: la base en un worktree descartable con su propio directorio de build, y el paso en `target-test`.
3. Por rama, se arranca un `clawg serve --profile dev` nuevo, porque el pico es el de la vida del proceso.
4. Se registra una copia congelada del corpus en una ruta que el perfil nunca indexó (`clawg projects add`). Si el registro no lanza el índice, lo lanza una búsqueda (`api.rs:3167-3168`).
5. Se espera la línea `docs refresh of …: done` y un minuto más, porque la muestra es por minuto.
6. Se lee `clawg metrics report --metric process.private_peak_bytes --by minute --since 1h --profile dev`.
7. Se alternan base, paso, base y paso, y se comparan las corridas vecinas. Al final se borran las copias, sus proyectos y el worktree.

La carga del modelo puede fijar ese pico antes del refresco, como pasó en embed-throughput (su verify, línea 43). Por eso se lee también el máximo por minuto de `process.private_bytes` durante el refresco. Si en las dos ramas el pico ya tiene su valor antes de que el refresco empiece, el ≤ 5 % se lee sobre ese máximo. Queda fijado acá.

## D14. La evaluación

- **Un campo nuevo, `tokens.padded_sorted_queue`:**
  - Los docs van en orden de ruta (`passage_lengths`, `eval_api.rs:1479`), que es el del plan.
  - Los agrupa `eval_api::queue_groups`, pública y sólo con `eval`. Delega en la pieza de D8 con el tope de producción, como `eval_api::passage_order` (`:1196-1202`).
  - Cada grupo se rellena con `padded_sorted` (`index_cost.rs:96-99`).
- **El aviso:** `padding_warning` (`:154-161`) compara `padded_ran` con `padded_sorted_queue` cuando `from_memo == 0`.
- **Los archivos viejos siguen comparables.** `padded` y `padded_sorted_in_doc` no cambian de nombre ni de sentido: son lo que se rellenaría sin ordenar y con A1. Las puertas de A1 y de A2 dividen por `padded`, así que dan lo mismo en un archivo viejo y en uno nuevo. Nada vuelve a leer el bloque `index` (D6).
- **Por qué coinciden los dos números:**
  - Sin memoria de vectores, los grupos dependen sólo de cuántos pasajes tiene cada doc, y en una primera indexación son los que quedan guardados.
  - Un doc sin pasajes no mueve ningún vaciado (D8).
  - Un doc que falla hace fallar la corrida de `rag-eval` (`eval_api.rs:1509-1511`).
  - Con memoria, `from_memo > 0`, y no hay aviso.
- **En «se queda si»,** cada corrida del paso comprueba que `padded_ran` = `padded_sorted_queue`, sin aviso, como A1 comprobó su igualdad. No es un umbral nuevo.
- **Las alternativas:** agrupar de nuevo en clawg-eval deja dos copias que pueden divergir sin que nada falle, y comparar contra `padded_sorted_all` avisaría en cada corrida.

## D15. Bits (decisión 4)

- **A2 no cambia cómo se corta ni cómo se ordena una llamada.**
  - Usa el mismo `passage_order` sobre los textos del vaciado, con la clave `passage_token_counts` y `longest_first`, y `embed_batch` sigue cortando de a 16 (`embedder.rs:460`).
  - Cambia qué textos comparten llamada y tanda, y con eso el largo al que se rellena cada fila.
  - Eso ya lo ejercita el caso de A1 `a batch of 17, longest first` (`crates/clawg-core/tests/e5_golden.rs:126`, en el orden de `passage_order`, `:211`) frente a `a batch of 17`: los mismos textos cambian de compañeros y de largo rellenado.
  - `e5_golden` no sabe de docs: un caso «de varios docs» tendría la misma forma con otros textos.
  - **Por eso no hay caso nuevo.** En otra CPU se sigue comparando con 1e-6.
- **Lo que falta comprobar en esta CPU.** El techo, que ordena todo junto, dio los mismos bits en 5261 de 5261 vectores (verify.md:286), pero la cola arma otras tandas.
  - `sorting_across_docs_against_sorting_in_each_doc` gana un tercer orden: la cola con el tope elegido, por la función de vaciado de producción.
  - Informa cuántos vectores dan los mismos bits que en cada doc y la diferencia mayor.
  - De paso, mide en debug el tiempo de la cola sobre la muestra. Es una vista previa y no decide nada.
- **Si se mueve un bit, vale la decisión 4 tal cual.** Si no se mueve ninguno, nada guardado cambia, y volver atrás es revertir los commits (propuesta, «Datos y vuelta atrás»).

## Medición

«Se queda si» se lee como en A1 (Medición, y verify.md:73-99):
- Son corridas `rag-eval --real --docs --no-cache` en release, alternadas, sobre el mismo `--root` congelado.
- La base es el commit anterior al cambio de conducta de A2, que ya incluye A1.
- Se comparan corridas vecinas del mismo régimen de la máquina.

## Tests que hoy fallan

Están en `project_docs/mod.rs`, salvo donde se indica. Usan `Batches` (`:2031-2069`), el tokenizer de palabras y `crate::test_dir::test_dir()`.

| Qué | Test |
|---|---|
| La pieza pura (fallan porque no existe) | `the_queue_limit_starts_at_one_batch_and_doubles_up_to_the_cap`, `the_queue_flushes_before_a_doc_that_would_pass_its_limit_and_when_it_reaches_it`, `a_doc_over_the_limit_is_a_flush_of_its_own`, `a_doc_with_no_passages_moves_no_flush` |
| Los docs chicos comparten llamada | `small_docs_share_one_call_to_the_embedder` |
| La cola respeta su tope | `no_call_holds_more_passages_than_the_cap_unless_it_is_one_doc`, con docs que suman más que `QUEUE_PASSAGES` |
| Los vectores vuelven a su doc y a su pasaje | `each_passage_of_a_shared_call_keeps_its_own_vector`: una sola llamada, y cada blob es el de `FakeEmbedder` para su texto |
| Un vector inutilizable | `an_unusable_vector_in_a_shared_call_skips_only_its_doc`: tres docs en una llamada, y sólo el del medio en `failed` |
| El orden de escritura | `a_shared_call_writes_its_docs_in_plan_order`: los ids crecen con la ruta |
| Un error del embedder | `an_embedder_error_keeps_the_docs_of_its_call_as_they_were`: los del vaciado anterior quedan escritos, y el refresco siguiente indexa el resto |
| Los tiempos | `embed_shares_split_the_time_by_texts_and_add_up_to_it` (pura) y `every_doc_of_a_shared_call_has_embed_time` |
| La evaluación | `queue_groups_are_the_calls_a_refresh_makes` (`eval_api.rs`); `the_padding_warning_compares_with_the_queue_padding` y `padded_sorted_queue_sorts_each_group_of_docs_together` (`index_cost.rs`) |

Las pruebas de refresco afirman también que los docs compartieron una llamada. Por eso hoy fallan, aunque una regla que fijan, como que un vector inutilizable omite sólo su doc, ya valga doc por doc.

Siguen pasando sin cambios:
- `a_doc_over_a_batch_reaches_the_embedder_longest_first`, que cubre el doc grande: sus 40 pasajes se vacían solos con el límite de 16.
- `a_failed_embedding_keeps_the_doc_as_it_was`, `a_short_batch_still_aborts_the_refresh`, `an_unusable_vector_skips_its_doc_and_the_rest_go_on` y `a_refresh_times_a_doc_whose_vector_is_unusable`.
- Las dos de la memoria de vectores (`crates/clawg-core/src/eval_api/vector_memo.rs:889-904`, `:907-930`): `big.md` se vacía solo y `small.md` después.

Las mediciones ignoradas son `padding_and_batches_by_queue_cap` (D9) y el tercer orden de `sorting_across_docs_…` (D15). Corren sólo con autorización.

## Archivos

| Archivo | Cambio |
|---|---|
| `crates/clawg-core/src/memory/project_docs/mod.rs` | La cola en `refresh`; la pieza pura y `QUEUE_PASSAGES`; `embed_passages` pasa a embeber un vaciado y a devolver el resultado de cada doc; `embed_shares`; los comentarios de los errores y el de «one changed doc» (`:518-520`); los tests y las dos mediciones |
| `crates/clawg-core/src/eval_api.rs` | `queue_groups` y su test |
| `crates/clawg-eval/src/bin/rag-eval/index_cost.rs` | `padded_sorted_queue`, `padding_warning`, `Tokens::line` y sus tests |
| `docs/reference/daemon.md` | La cola, en el párrafo del índice de docs |
| `specs/project-docs` («Refresco incremental») y `specs/eval` (IC6) | La cola, su tope, el límite que crece, los fallos y el campo nuevo. Los requisitos de A2 los escribe la spec |

## Riesgos

| Riesgo | Mitigación |
|---|---|
| Ningún tope cumple D9 | A2 se cierra sin código, como permite la propuesta |
| El tiempo tiene un término que no es P, B ni Q | Decide «se queda si», con corridas alternadas como las de A1 |
| La memoria | H ≤ 16 MiB por construcción, y el pico en el daemon (D13) |
| La primera búsqueda en un repositorio grande | El límite que crece (D11) |
| El trabajo que se pierde al apagar | A lo sumo un vaciado; por eso se elige el menor tope |
| Una CPU mueve bits | `e5_golden` con 1e-6, y la decisión 4 |
