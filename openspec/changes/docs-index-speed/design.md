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

A1 no le cierra nada a A2. `passage_order` e `in_passage_order` trabajan sobre cualquier lista de textos, no sobre un doc; el orden queda fuera de `E5Embedder`; y contar sigue dentro de `embed`, así que A2 compara su `phases.embed` contra el de la corrida de A1. Este diseño no decide lo que le toca a A2: devolver un vector inutilizable a su doc, el tope de la cola y el campo contra el que avisa.

## Fuera de alcance

Con una última tanda corta, ordenar de más largo a más corto rellena más que no ordenar: en `sorting_can_pad_more_when_the_last_batch_is_short` (`index_cost.rs:368`) da 1610 contra 260. Mandar primero la tanda incompleta, una partición óptima o elegir en cada doc el orden que rellena menos ahorrarían algo más. Pero la puerta y «se queda si» se midieron con `longest_first`, así que cualquiera de esas ideas sería otro cambio, con su propio campo en `index.tokens`.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| El orden mueve bits | D7. Si la diferencia mayor es ≤ 1e-6, se reescribe la fixture y se vacía `docs/eval/cache/`; si pasa de 1e-6, el paso cae y el tema pasa a 17 (propuesta, decisión 4) |
| El test sobre la fixture falla hasta el bless | Su commit espera la corrida autorizada en la máquina de la fixture |
| La varianza del reloj (+26,8 % entre D y R) supera el −10 % que se pide | Corridas alternadas sobre el mismo `--root` |
| Algún doc rellena más ordenado | Se ve en `by_area`; arreglarlo queda fuera de alcance |
