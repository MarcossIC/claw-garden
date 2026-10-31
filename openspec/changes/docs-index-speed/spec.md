# docs-index-speed — Especificación

Modifica [`specs/project-docs`](../../specs/project-docs/spec.md) («Refresco incremental») con IS1 e IS2, e IC6 de [`specs/eval`](../../specs/eval/spec.md) («`rag-eval --docs`», bloque `index.tokens`) con IS3. IS4 e IS5 son requisitos de verificación, no de producción. Cubre sólo el paso A1 de la [propuesta](proposal.md); A2 se especifica en «Paso A2», al final (IS6 a IS17), y C, W y S están cerrados (ver [`verify.md`](verify.md)) y no piden nada. Todos los tests usan embedders falsos y no dejan nada en el temporal, salvo IS4 e IS5, que necesitan el modelo real y corren sólo con autorización, nunca en la suite normal.

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

# Paso A2

Ordenar entre docs con una cola acotada ([diseño](design.md), «Paso A2», D8 a D15). Modifica `specs/project-docs` («Refresco incremental») con IS7 a IS14 y `specs/eval` (IC6) con IS15. IS6, IS16 e IS17 son requisitos de medición, no de producción.

**Condicional.** IS7 a IS15 describen código de producción y **sólo entran en vigor si la regla de IS6 elige un tope c**; entonces `QUEUE_PASSAGES` vale c. Si ningún tope la cumple, A2 se cierra sin código de producción: sólo se cumple IS6, `verify.md` guarda la tabla, e IS7 a IS17 quedan sin implementar. Los tests de producción usan embedders falsos, el tokenizer de palabras y `crate::test_dir::test_dir()`, y no dejan nada en el temporal. IS6, IS16 e IS17 corren sólo con autorización, nunca en la suite normal.

## El tope (verificación, decide A2)

### IS6. El tope se elige con una cuenta fija de antemano

`padding_and_batches_by_queue_cap` (`crates/clawg-core/src/memory/project_docs/mod.rs`, ignorada) MUST recorrer el corpus en orden de ruta, contar los tokens como los recibe el modelo (`passage_token_counts`) y, para el orden de los pasajes, el de cada doc, la cola con c ∈ {64, 128, 256, 512, 1024, 2048, 4096} y todo junto, imprimir P (tokens rellenados), B (tandas), Q (filas × más largo al cuadrado, por tanda), los pasajes de la llamada más grande y H (bytes del vaciado más grande). Sólo necesita el tokenizer verificado, no una sesión de ONNX. La regla de D9 MUST aplicarse tal como está escrita: el menor c con H(c) ≤ 16 MiB y previsto(c) ≥ 0,18. La cuenta MUST reproducir P = 1 998 461 y 1 614 977 sobre el worktree de `0c9d49a` antes de que sus B valgan.

#### Escenario: un tope cumple
- GIVEN la tabla de la cuenta sobre este repositorio
- WHEN c = 256 es el menor con H ≤ 16 MiB y previsto ≥ 0,18
- THEN `QUEUE_PASSAGES` vale 256 y siguen IS7 a IS15

#### Escenario: ninguno cumple
- GIVEN una tabla donde todo c con H ≤ 16 MiB da previsto < 0,18
- WHEN se aplica la regla
- THEN A2 se cierra sin código de producción y `verify.md` guarda la tabla

**Test (ignorado, compila y no corre en apply):** `padding_and_batches_by_queue_cap`. La decisión de vaciado que usa es la pieza pura de IS7.

## La cola (`specs/project-docs`)

### IS7. El límite crece hasta el tope y la cola se vacía por tres causas

Los pasajes que la cola retiene, se reutilicen o no, MUST contar contra su límite. El límite MUST empezar en `EMBED_BATCH` y duplicarse con cada vaciado hasta `QUEUE_PASSAGES`. La cola MUST vaciarse antes de que entre un doc que la pasaría del límite (si no está vacía), cuando llega al límite, y al terminar el plan. Un doc sin pasajes MUST NOT mover ningún vaciado. La decisión es una función pura, que recorren `refresh` y `eval_api::queue_groups` (IS15).

#### Escenario: el límite crece
- GIVEN una cola con tope 64 y docs que llenan cada límite
- WHEN se suceden vaciados
- THEN los límites son 16, 32, 64, 64

#### Escenario: vaciar antes y al llegar
- GIVEN límite 16 y una cola con 10 pasajes
- WHEN llega un doc de 8, y luego uno de 6
- THEN se vacía antes del de 8; el de 8 entra solo; al llegar el de 6 la cola retiene 14 y no se vacía; con 2 más llega a 16 y se vacía

#### Escenario: un doc vacío
- GIVEN una cola con 10 pasajes y un doc sin pasajes
- WHEN entra
- THEN ni se vacía antes ni después

**Tests (fallan hoy, funciones puras):** `the_queue_limit_starts_at_one_batch_and_doubles_up_to_the_cap`, `the_queue_flushes_before_a_doc_that_would_pass_its_limit_and_when_it_reaches_it`, `a_doc_with_no_passages_moves_no_flush`.

### IS8. Un doc más grande que el límite va solo

Un doc con más pasajes que el límite MUST vaciarse solo: la cola se vacía antes de él si no está vacía, y él es su propio vaciado. Ninguna llamada al embedder MUST retener más de `QUEUE_PASSAGES` pasajes, salvo que sea un solo doc.

#### Escenario: un doc grande
- GIVEN límite 16, un doc de 40 pasajes y uno de 3 antes que él
- WHEN se refresca
- THEN el de 3 se vacía solo, el de 40 va en otra llamada de 40 y el límite crece entre ambas

#### Escenario: el tope se respeta
- GIVEN docs de 30 pasajes que suman más que `QUEUE_PASSAGES`
- WHEN se refresca
- THEN ninguna llamada pasa de `QUEUE_PASSAGES` pasajes

**Tests:** `a_doc_over_the_limit_is_a_flush_of_its_own` (pura, falla hoy) y `no_call_holds_more_passages_than_the_cap_unless_it_is_one_doc`. `a_doc_over_a_batch_reaches_the_embedder_longest_first` sigue pasando sin cambios.

### IS9. Los docs de un vaciado comparten una llamada, ordenada

Los textos a embeber de los docs de un vaciado MUST llegar al embedder en **una sola llamada**, concatenados en el orden del plan y ordenados por `passage_order` sobre la lista entera. Con más de `EMBED_BATCH` textos y sin tokenizer o si contar falla, rigen IS2 y el orden del plan.

#### Escenario: docs chicos
- GIVEN cuatro docs de 5 pasajes nuevos y `Recording`
- WHEN se refresca
- THEN el embedder recibe una llamada de 20 textos, del más largo al más corto

**Test (falla hoy):** `small_docs_share_one_call_to_the_embedder`.

### IS10. Cada vector vuelve a su doc y a su pasaje

Con un embedder cuyo vector depende sólo del texto, cada pasaje MUST guardar el vector de su propio texto, sea de su doc o de otro del vaciado. Cada doc MUST tomar los suyos según cuántos textos tenía.

#### Escenario: una llamada, tres docs
- GIVEN tres docs de largos distintos en una llamada y `FakeEmbedder`
- WHEN se refresca
- THEN cada blob es el de `FakeEmbedder` para el texto de su pasaje

**Test (falla hoy):** `each_passage_of_a_shared_call_keeps_its_own_vector`.

### IS11. Un vector inutilizable omite sólo su doc

Cuando el vector de un texto es inutilizable, el doc dueño de ese texto MUST ir a `failed` con su motivo y los demás docs del vaciado MUST escribirse.

#### Escenario: el del medio
- GIVEN tres docs en una llamada y un vector no finito para un texto del segundo
- WHEN se refresca
- THEN sólo el segundo va a `failed`; el primero y el tercero se indexan

**Test (falla hoy):** `an_unusable_vector_in_a_shared_call_skips_only_its_doc`. Siguen pasando: `an_unusable_vector_skips_its_doc_and_the_rest_go_on` y `a_refresh_times_a_doc_whose_vector_is_unusable`.

### IS12. Un error del embedder termina el refresco

Si el embedder falla o devuelve otra cantidad, el refresco MUST terminar con `Embed` o `VectorCount`, que nombran el primer doc del vaciado, con `VectorCount.passages` igual a los textos del vaciado. Ningún doc del vaciado que falló MUST quedar escrito; los de vaciados anteriores MUST seguir escritos, y el refresco siguiente MUST planear el resto.

#### Escenario: falla el segundo vaciado
- GIVEN dos vaciados y un embedder que falla en la segunda llamada
- WHEN se refresca, y después otra vez con un embedder sano
- THEN el primero termina en `Err` con los docs del primer vaciado escritos y los del segundo como estaban; el segundo indexa los que faltan

**Test (falla hoy):** `an_embedder_error_keeps_the_docs_of_its_call_as_they_were`. Siguen pasando: `a_failed_embedding_keeps_the_doc_as_it_was` y `a_short_batch_still_aborts_the_refresh`.

### IS13. Se escribe en el orden del plan y el informe no cambia

Después de su vaciado, cada doc MUST escribirse en una transacción propia (`replace_doc`), en el orden del plan, de modo que los ids de fila salgan como hoy. `RefreshReport` MUST ser igual para las mismas entradas (sus campos son sumas y `failed` sigue el orden del plan).

#### Escenario: ids por ruta
- GIVEN cuatro docs que comparten una llamada
- WHEN se refresca
- THEN los ids de fila crecen con la ruta, y el informe es el de un refresco doc por doc

**Test (falla hoy):** `a_shared_call_writes_its_docs_in_plan_order`, que también compara el informe.

### IS14. Los tiempos siguen sumando `index_seconds`

`read`, `mask`, `split`, `lookup` y `write` MUST medirse por doc. El tiempo de un vaciado MUST repartirse entre sus docs según la parte de textos que aporta cada uno, en nanosegundos enteros y con el resto al último. `RefreshTiming::phases` MUST seguir sumando todo `index_seconds`. La línea de tiempos del daemon (IC2) MUST NOT cambiar de forma.

#### Escenario: el reparto suma
- GIVEN un vaciado de 3, 1 y 6 textos y un `lap` de 10 ns
- WHEN se reparte
- THEN los docs reciben 3, 1 y 6 ns, y en general la suma es siempre el `lap`

#### Escenario: todo doc con textos tiene `embed`
- GIVEN tres docs en una llamada
- WHEN se refresca
- THEN cada uno tiene `phases.embed` > 0

**Tests:** `embed_shares_split_the_time_by_texts_and_add_up_to_it` (pura) y `every_doc_of_a_shared_call_has_embed_time` fallan hoy. `the_docs_timing_line_names_every_phase` (`crates/clawg-cli/src/daemon/api.rs`) pasa hoy y fija la forma.

## Medir el índice (`specs/eval`, IC6)

### IS15. La evaluación agrupa como el refresco y el aviso lo compara

`eval_api::queue_groups` (pública, sólo con `eval`) MUST agrupar los docs, en orden de ruta, con la misma pieza de IS7 y el tope de producción. `tokens.padded_sorted_queue` MUST ser el relleno de `padded_sorted` de cada grupo, sumado. `padding_warning` MUST comparar `padded_ran` con `padded_sorted_queue` (no con `padded_sorted_in_doc`) cuando `from_memo == 0`, MUST NOT avisar con `from_memo > 0`, y su mensaje MUST nombrar los dos números. `padded` y `padded_sorted_in_doc` MUST NOT cambiar de nombre ni de sentido, y un archivo viejo MUST seguir legible con `--diff`.

#### Escenario: coinciden
- GIVEN `from_memo = 0`, `padded_sorted_queue = 800` y `padded_ran = 800`
- WHEN se evalúa el aviso
- THEN no hay aviso

#### Escenario: difieren
- GIVEN `from_memo = 0`, `padded_sorted_queue = 800` y `padded_ran = 900`
- WHEN se evalúa el aviso
- THEN el aviso nombra 800 y 900

#### Escenario: los grupos son las llamadas
- GIVEN docs con pasajes `[3, 20, 5, 0, 5]` y tope 64
- WHEN `queue_groups` y un refresco agrupan
- THEN dan los mismos grupos

**Tests (fallan hoy):** `queue_groups_are_the_calls_a_refresh_makes` (`eval_api.rs`); `the_padding_warning_compares_with_the_queue_padding` y `padded_sorted_queue_sorts_each_group_of_docs_together` (`index_cost.rs`); `the_padding_warning_names_both_numbers` se reescribe sobre `padded_sorted_queue`.

## Bits y memoria (verificación)

### IS16. El tercer orden de la prueba de bits

`sorting_across_docs_against_sorting_in_each_doc` MUST ganar un tercer orden: la cola con `QUEUE_PASSAGES`, armada por la función de vaciado de producción. MUST informar cuántos vectores dan los mismos bits que en cada doc y la diferencia mayor. Si algún bit se mueve, vale la decisión 4 de la propuesta; la tolerancia de `e5_golden` MUST NOT agrandarse y MUST NOT haber un caso nuevo.

#### Escenario: el informe
- GIVEN la muestra de docs reales y el modelo real
- WHEN corre
- THEN imprime «N de M con los mismos bits» y la diferencia mayor para la cola

**Test (ignorado):** `sorting_across_docs_against_sorting_in_each_doc`, con el tercer orden.

### IS17. El pico de memoria del daemon

El pico privado de un daemon dev que indexa un corpus congelado MUST subir ≤ 5 % con el paso frente a la base (D13), alternando base, paso, base y paso con `process.private_peak_bytes`. Si el pico ya tiene su valor antes de que el refresco empiece en las dos ramas, se lee el máximo por minuto de `process.private_bytes` durante el refresco. H ≤ 16 MiB lo fija IS6 por construcción; esto lo comprueba en el proceso.

#### Escenario: el pico
- GIVEN dos daemons dev, base y paso, con el mismo corpus congelado
- WHEN terminan `docs refresh of …: done` y pasa un minuto
- THEN el pico del paso es ≤ 1,05 × el de la base

**Verificación con autorización, sin test:** se registra en `verify.md`.
