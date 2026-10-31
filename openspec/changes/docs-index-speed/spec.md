# docs-index-speed — Especificación

Modifica [`specs/project-docs`](../../specs/project-docs/spec.md) («Refresco incremental») con IS1 e IS2, e IC6 de [`specs/eval`](../../specs/eval/spec.md) («`rag-eval --docs`», bloque `index.tokens`) con IS3. IS4 e IS5 son requisitos de verificación, no de producción. Cubre sólo el paso A1 de la [propuesta](proposal.md); A2 se especifica cuando A1 esté medido, y C, W y S están cerrados (ver [`verify.md`](verify.md)) y no piden nada. Todos los tests usan embedders falsos y no dejan nada en el temporal, salvo IS4 e IS5, que necesitan el modelo real y corren sólo con autorización, nunca en la suite normal.

## Orden del refresco (`specs/project-docs`)

### IS1. Un doc de más de una tanda se embebe del más largo al más corto

`refresh` (`crates/clawg-core/src/memory/project_docs/mod.rs:486`) cuenta los textos a embeber de cada doc después de reusar vectores (`reuse_plan`, `mod.rs:587`; `to_embed`, `mod.rs:538`), no sus pasajes. Hoy los entrega en el orden del doc, en una llamada (`embed_passages`, `mod.rs:645-656`), que el embedder corta en tandas de `EMBED_BATCH` = 16 (`crates/clawg-core/src/memory/embedder.rs:184`).

Cuando esa cuenta supera `EMBED_BATCH`, el refresco MUST entregarlos al embedder en **una sola llamada**, ordenados de más tokens a menos, con los empates en el orden de los pasajes. Los tokens se cuentan como los recibe el modelo: con el prefijo de pasaje del embedder, cortados a su ventana (como `passage_lengths`, `crates/clawg-core/src/eval_api.rs:1447-1481`). Cada pasaje MUST recibir el vector de su propio texto: `fill` (`mod.rs:601`) los devuelve en el orden de los pasajes. Con un embedder cuyo vector depende sólo del texto, lo que se escribe (`RefreshReport`, pasajes, ordinales, qué vector va con qué texto) MUST ser lo mismo que antes. Con el modelo real sólo pueden moverse los últimos bits de un vector, porque cambia con qué textos comparte su tanda (IS4, IS5). Un vector inutilizable MUST omitir sólo su doc (`mod.rs:545-549`).

Con `EMBED_BATCH` textos a embeber o menos, el refresco MUST entregarlos en el orden de los pasajes, como hoy, y MUST NOT contar sus tokens.

#### Escenario: un doc largo llega ordenado
- GIVEN un doc de 20 pasajes sin vectores guardados, con largos conocidos y un embedder falso que anota cada llamada
- WHEN se refresca
- THEN el embedder recibe una llamada de 20 textos, del más largo al más corto, y los empates en el orden de los pasajes

#### Escenario: cada pasaje conserva su vector
- GIVEN el mismo doc, con un embedder falso cuyo vector depende del texto
- WHEN se refresca
- THEN el pasaje *i* guarda el vector de su texto, y el índice es igual al que escribe un refresco que no ordena

#### Escenario: la reutilización decide
- GIVEN un doc de 20 pasajes del que 10 tienen vector guardado
- WHEN se refresca
- THEN los 10 textos restantes llegan en el orden de los pasajes, sin contar tokens

#### Escenario: una tanda o menos
- GIVEN un doc de 16 pasajes nuevos
- WHEN se refresca
- THEN llegan en el orden de los pasajes y no se tokeniza nada

#### Escenario: un vector inutilizable
- GIVEN dos docs de 20 pasajes y un embedder que devuelve un vector no finito para un texto del primero
- WHEN se refresca
- THEN el primero va a `failed` y el segundo se indexa

#### Escenario: relleno menor
- GIVEN un doc de 20 pasajes que alternan 10 y 100 tokens
- WHEN se refresca
- THEN el relleno de las tandas que corre el modelo es 16 × 100 + 4 × 10 = 1640, no 2000

**Tests (fallan hoy):** `a_doc_over_a_batch_reaches_the_embedder_longest_first`, `a_reordered_doc_keeps_each_passage_with_its_own_vector`, `a_reordered_doc_writes_what_an_unordered_one_wrote`, `reuse_decides_whether_a_doc_is_reordered` y `an_unusable_vector_in_a_reordered_doc_skips_only_that_doc` en `crates/clawg-core/src/memory/project_docs/mod.rs`. Hay dos puros, que fallan porque las funciones no existen: `in_passage_order_undoes_longest_first` (vacío, empates, identidad) y `passage_order_is_none_up_to_one_batch_and_longest_first_above_it` (16 textos dan `None` y 17, un orden). `a_doc_of_one_batch_or_less_keeps_the_passage_order` ya pasa y fija lo que no cambia.

### IS2. Sin tokenizer

Los largos los cuenta el tokenizer del partidor que recibe el refresco (`TokenSplitter::tokenizer()`, [diseño](design.md) D2). Cuando el partidor no tiene tokenizer (`TokenSplitter::fallback()`, los tests del refresco, `CLAWG_TEST_EMBEDDER=fake` sin tokenizer), el doc MUST ir en el orden de sus pasajes, como hoy, y el refresco MUST NOT fallar por eso. Si contar falla, el doc también MUST ir en el orden de sus pasajes, y lo que pase después es lo que hace hoy el embedder con esos textos.

#### Escenario: sin largos
- GIVEN un doc de 20 pasajes y un refresco cuyo partidor no tiene tokenizer
- WHEN se refresca
- THEN llega en el orden de los pasajes y el resultado es el de hoy

**Test (pasa hoy, fija lo que no cambia):** `without_a_tokenizer_a_doc_keeps_the_passage_order`.

## Medir el índice (`specs/eval`, IC6)

### IS3. El aviso compara contra el orden que se embebe

`padded_sorted_in_doc` (`crates/clawg-eval/src/bin/rag-eval/index_cost.rs:112`, calculado en `:129`) MUST ser el relleno del orden en que el refresco embebe ahora. `padded` (`:108`) MUST seguir en el orden del doc, como contrafactual. `padding_warning` (`:151`) MUST comparar `padded_ran` con `padded_sorted_in_doc` y no con `padded`; MUST seguir sin avisar con `from_memo > 0`. El mensaje MUST nombrar los dos números. Los nombres y tipos de los campos MUST NOT cambiar, y `--diff` contra un archivo viejo MUST seguir andando.

#### Escenario: coinciden
- GIVEN `from_memo = 0`, `padded = 1000`, `padded_sorted_in_doc = 900` y `padded_ran = 900`
- WHEN se evalúa el aviso
- THEN no hay aviso

#### Escenario: difieren
- GIVEN `from_memo = 0`, `padded_sorted_in_doc = 900` y `padded_ran = 1000`
- WHEN se evalúa el aviso
- THEN el aviso nombra 900 y 1000

#### Escenario: algo salió de la memoria
- GIVEN `from_memo > 0` y los dos números distintos
- WHEN se evalúa el aviso
- THEN no hay aviso

**Tests (fallan hoy):** `the_padding_warning_compares_with_the_sorted_in_doc_padding` y `the_padding_warning_names_both_numbers`; `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` se reescribe sobre `padded_sorted_in_doc`. En `index_cost.rs`.

## Bits (verificación)

### IS4. `e5_golden` ve el orden nuevo

`crates/clawg-core/tests/e5_golden.rs` MUST ganar un caso con los 17 textos de «a batch of 17» (`e5_golden.rs:176-183`, `:206`), en una sola llamada y en el orden de IS1. El orden lo da `eval_api::passage_order`, que delega en el del refresco, así que la fixture fija el orden de producción y no una copia escrita en el test ([diseño](design.md) D7). En la máquina cuya huella nombra la fixture se compara exacto (se escribe con `CLAWG_BLESS=1`, `:14`, `:294`); en otra, con 1e-6. La tolerancia MUST NOT agrandarse.

Por cada uno de los 17 textos, la diferencia mayor entre su vector en «a batch of 17» y en el caso nuevo MUST ser ≤ 1e-6. Esa es la regla de aceptación de la decisión 4 de la propuesta. Un test que lee sólo la fixture la comprueba e informa cuántos textos dan los mismos bits. No necesita el modelo y corre con la suite.

#### Escenario: el caso falta
- GIVEN la fixture sin el caso
- WHEN corre `the_vectors_are_the_ones_fastembed_gave` (`:291`)
- THEN falla por el caso que no está

#### Escenario: el orden mueve un vector más de la cota
- GIVEN una fixture cuyo caso nuevo difiere en más de 1e-6 de «a batch of 17» en algún texto
- WHEN corre el test sobre la fixture
- THEN falla y nombra el texto y la diferencia

**Tests (fallan hoy):** `the_vectors_are_the_ones_fastembed_gave` (ignorado, necesita el modelo), con el caso nuevo; y `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order` (corre con la suite), que falla mientras la fixture no tenga el caso. Su commit trae la fixture ya escrita por el bless.

### IS5. Cuántos vectores mueven bits

Una prueba ignorada MUST embeber pasajes reales de docs en el orden del doc y en el de IS1, y MUST informar cuántos vectores tienen los mismos bits y la diferencia mayor.

#### Escenario: el informe
- GIVEN pasajes reales de docs
- WHEN corre
- THEN imprime «N de M con los mismos bits» y la diferencia mayor

**Test (ignorado):** `docs_passages_keep_their_bits_in_either_order`.
