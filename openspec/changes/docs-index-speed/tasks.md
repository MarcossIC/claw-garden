# docs-index-speed — Tareas

Strict TDD: en cada grupo el test va primero y falla (RED), por la aserción o porque no compila; después el código (GREEN). Cada grupo es un commit convencional atómico, que se comprueba con las pruebas de los archivos que cambia: `cargo test -p <crate> --target-dir target-test <filtro>`. La suite completa (`cargo test --workspace --target-dir target-test`) y los dos clippy (`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -A clippy::all -D warnings`) corren una vez, sobre el último commit del cambio. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea, y sin atribución. `docs/migration-plan.md` no se commitea. Un cambio de conducta actualiza `docs/reference/` en el mismo commit.

Requisitos en la [spec](spec.md) (IS1 a IS5) y decisiones en el [diseño](design.md) (D1 a D7). Los nombres de test son los de la spec y el diseño.

## Orden elegido (y por qué difiere de lo previsto)

`passage_order`, `in_passage_order` y `TokenSplitter::tokenizer()` son `pub(crate)` o privados: sin llamador fuera de `#[cfg(test)]` serían código muerto (error de compilación, `tests/conventions.rs`). Por eso se commitean junto con `embed_passages`, que los llama (grupo 2). `eval_api::passage_order` es `pub` y va aparte (grupo 3).

El caso nuevo de `e5_golden` no puede pasar sin el bless, que necesita el modelo real: su código, el test sobre la fixture y la fixture se commitean juntos en un único commit que espera la Parte 2a (grupo 7). La suite completa y los dos clippy cierran sobre ese commit, que es el último.

Antes del primer test: `cargo clean --workspace --target-dir target-test`. Ningún build release, instalación ni acceso a `~/.clawg*` en la Parte 1.

## Parte 1 — código, sin modelo real (lo hace apply)

### 1. `longest_first` vive junto a `EMBED_BATCH` (D4)

- [x] Limpiar: `cargo clean --workspace --target-dir target-test`.
- [x] Mover `longest_first` de `crates/clawg-core/src/memory/portability/import.rs:2537` a `crates/clawg-core/src/memory/embedder.rs` (`pub(crate)`, junto a `EMBED_BATCH`, `:184`) con su test (`import.rs:5222`, `longest_first_puts_the_most_tokens_first_and_keeps_ties_in_order`). `import.rs` (`:2529`, `:5486`) y `eval_api::longest_first` (`eval_api.rs:1085`) la toman de ahí.
- [x] Comprobar: `cargo test -p clawg-core --target-dir target-test longest_first`.
- Commit: `refactor(embedder): keep longest_first next to the batch size`

### 2. El refresco manda un doc largo del más largo al más corto (IS1, IS2; D1 a D5)

- [x] RED puros en `crates/clawg-core/src/memory/project_docs/mod.rs`: `in_passage_order_undoes_longest_first` (vacío, empates, identidad) y `passage_order_is_none_up_to_one_batch_and_longest_first_above_it` (16 textos `None`, 17 un orden). Fallan por no compilar.
- [x] RED con `try_refresh` y `Some(word_tokenizer())`, embedder falso y `test_dir()`: `a_doc_over_a_batch_reaches_the_embedder_longest_first` (`Recording`, `mod.rs:1426`), `a_reordered_doc_keeps_each_passage_with_its_own_vector`, `a_reordered_doc_writes_what_an_unordered_one_wrote` (`dump`, `mod.rs:1470`), `reuse_decides_whether_a_doc_is_reordered`, `an_unusable_vector_in_a_reordered_doc_skips_only_that_doc`. Fijan lo que no cambia, y pasan hoy: `a_doc_of_one_batch_or_less_keeps_the_passage_order`, `without_a_tokenizer_a_doc_keeps_the_passage_order`.
- [x] GREEN: `TokenSplitter::tokenizer() -> Option<&Tokenizer>` (`token_splitter.rs`); `passage_order(Option<&Tokenizer>, &dyn Embedder, &[String]) -> Option<Vec<usize>>` (`pub(crate)`; prefijo de pasaje y ventana como `passage_lengths`) e `in_passage_order<T>` (privada, junto a `fill`); `embed_passages` (`mod.rs:645`) recibe el tokenizer del `splitter` de `refresh`, manda una sola llamada ordenada y devuelve los vectores en el orden de los pasajes antes de `to_unit`. Sin cambio de firma en `refresh` ni en sus llamadores.
- [x] Docs: una frase en `docs/reference/daemon.md`, en el párrafo del índice de docs, como la que `docs/reference/commands.md` dice de la importación; `embed` incluye contar y ordenar.
- [x] Comprobar: `cargo test -p clawg-core --target-dir target-test project_docs`.
- Commit: `perf(project-docs): embed a doc of several batches longest first`

### 3. La puerta de evaluación al orden del refresco (D6, D7)

- [x] RED en `crates/clawg-core/src/eval_api.rs`: `passage_order_is_the_order_a_refresh_embeds_in` (la puerta da el mismo orden que `project_docs::passage_order`; con el contador falso, la identidad).
- [x] GREEN: `pub fn passage_order(embedder: &EvalEmbedder, counter: &EvalTokenCounter, texts: &[String]) -> Vec<usize>`, que delega en `project_docs::passage_order` con el tokenizer del contador (si lo tiene) y el prefijo del embedder, y da la identidad donde la de producción da `None`. `EvalTokenCounter` es el que `e5_golden` ya usa (`crates/clawg-core/tests/e5_golden.rs:18`, `:215`).
- [x] Comentarios de `EvalDocLengths` (`eval_api.rs:1368`) y `passage_lengths` (`:1442-1446`): los largos salen en el orden de los pasajes, no en el que se embebió. Renombrar `passage_lengths_follow_the_stored_passages_in_batch_order` (`:2887`) a «ordinal».
- [x] Comprobar: `cargo test -p clawg-core --target-dir target-test eval_api`.
- Nota: los tests de `eval_api` sólo compilan con `--features eval` (el comprobar de arriba da 0 tests sin él): `cargo test -p clawg-core --features eval --target-dir target-test --lib eval_api`; `--workspace` lo enciende.
- Commit: `feat(eval): expose the order a docs refresh embeds in`

### 4. El aviso compara contra el orden que se embebe (IS3; D6)

- [x] RED en `crates/clawg-eval/src/bin/rag-eval/index_cost.rs`: `the_padding_warning_compares_with_the_sorted_in_doc_padding` (`padded = 1000`, `padded_sorted_in_doc = 900`, `padded_ran = 900`: sin aviso), `the_padding_warning_names_both_numbers` (900 y 1000 en el mensaje), y se reescribe `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` sobre `padded_sorted_in_doc` (con `from_memo > 0` no avisa).
- [x] GREEN: `padding_warning` (`:151`) compara `padded_ran` con `padded_sorted_in_doc`. Los campos no cambian de nombre ni de tipo; `padded` queda en el orden del doc. Comentarios de `Tokens` (`:103-122`).
- [x] Docs: `docs/reference/testing.md` si describe el aviso.
- [x] Comprobar: `cargo test -p clawg-eval --target-dir target-test index_cost`.
- Nota: `docs/reference/testing.md` no describe el aviso; no se tocó.
- Commit: `fix(eval): warn when the padding that ran differs from the refresh order`

### 5. La prueba ignorada de los bits en docs reales (IS5; D7)

- [x] Escribir y compilar, sin correr, `docs_passages_keep_their_bits_in_either_order` (`#[ignore]`) en los tests de `crates/clawg-core/src/memory/project_docs/mod.rs`, según el modelo de `import.rs:5411-5597`. Necesita `CLAWG_E5_MODEL_DIR`; corpus de este repositorio desde `CARGO_MANIFEST_DIR/../..` (`corpus::walk`, `corpus::read`, `redact`, `passages`); docs de más de `EMBED_BATCH` pasajes, hasta `CLAWG_EMBED_PASSAGES` (5000); `CLAWG_EMBED_RUNS` (3), alternando el orden; no abre un store ni usa el temporal. Imprime «N de M con los mismos bits», la diferencia mayor y la mediana de cada orden.
- [x] Comprobar sólo que compila: `cargo test -p clawg-core --target-dir target-test --no-run`. NO correrla.
- Commit: `test(project-docs): report how many vectors move bits with the embed order`

### 6. El caso nuevo de `e5_golden` (IS4; D7) — espera la Parte 2a

- [x] RED (escrito en el árbol, sin commitear): en `crates/clawg-core/tests/e5_golden.rs`, el caso `a batch of 17, longest first` después de `a batch of 17` (`:206`): los mismos 17 textos (`:176-183`), una sola llamada, en el orden de `eval_api::passage_order`. `the_vectors_are_the_ones_fastembed_gave` (ignorado) lo incluye. Test nuevo, que corre con la suite: `a_batch_longest_first_stays_within_the_tolerance_of_the_batch_in_order` (lee sólo la fixture; por texto, diferencia mayor ≤ 1e-6, nombra texto y diferencia; imprime cuántos dan los mismos bits). Falla mientras la fixture no tenga el caso.
- [x] Este grupo NO se commitea en la Parte 1: el test no pasa sin el bless y la suite se rompería. Se deja el árbol con estos cambios (sin `git add`) y se pasa a la Parte 2a, o se guarda como parche fuera del repo.
- [x] Docs: `docs/reference/testing.md`, si habla de los casos de `e5_golden`. No los describe, así que no cambia.
- Commit (después de 2a y 2b): `test(embedder): pin the vectors of a batch embedded longest first`, con `crates/clawg-core/tests/e5_golden.rs`, `crates/clawg-core/tests/fixtures/e5_golden.json` y la doc.

### Cierre de la Parte 1

Los grupos 1 a 5 se commitean y se comprueban cada uno con sus archivos. La suite completa y los dos clippy corren sobre el commit del grupo 6, el último de la serie, cuando ya trae la fixture blessed (Parte 2a). Si el orquestador pide verlos antes, se corren con el grupo 6 fuera del árbol, y se repiten tras la Parte 2a.

## Parte 2 — necesita el modelo real y autorización explícita (la pide el orquestador; apply no la corre)

Una a la vez, sin otra compilación ni prueba en marcha. Usan el modelo del perfil dev (`~/.clawg-dev/models`, sólo lectura), nunca `~/.clawg`. No matar ningún `clawg.exe`. Cada ítem marca qué necesita.

- [x] 2a. Bless de `e5_golden` (modelo real, autorización, máquina de la fixture: Windows `x86_64` con AVX2, AVX-512F y FMA): `CLAWG_BLESS=1 CLAWG_E5_MODEL_DIR=<data dir dev>/models/multilingual-e5-small cargo test -p clawg-core --features eval --target-dir target-test --test e5_golden -- --ignored the_vectors_are_the_ones_fastembed_gave` (el target pide `eval`, `e5_golden.rs:14`). Antes, comprobar que la huella de la fixture es la de la máquina. Revisar `git diff` de `crates/clawg-core/tests/fixtures/e5_golden.json`: sólo debe agregar el caso nuevo. Correr sin `CLAWG_BLESS`: `CLAWG_E5_MODEL_DIR=… cargo test -p clawg-core --features eval --target-dir target-test --test e5_golden -- --include-ignored` (incluye el test sobre la fixture). Commitear el grupo 6.
  - Si algún texto difiere más de 1e-6 entre `a batch of 17` y el caso nuevo, el paso cae (propuesta, decisión 4). En ese caso:
    - Se corre igual 2b y sus números se anotan en `verify.md`.
    - No se commitea el grupo 6 y se descartan sus cambios.
    - Se revierten los grupos 2 a 5: el aviso del grupo 4 y la prueba del grupo 5 suponen el orden nuevo. El grupo 1 queda, porque es un movimiento sin cambio de conducta.
    - La tolerancia no se agranda, y el tema pasa a 17 model-reembed.
- [x] 2b. Prueba de IS5 (modelo real, autorización): `CLAWG_E5_MODEL_DIR=<data dir dev>/models/multilingual-e5-small cargo test -p clawg-core --lib --target-dir target-test docs_passages_keep_their_bits_in_either_order -- --ignored --nocapture`. Anotar en `verify.md`: «N de M con los mismos bits», la diferencia mayor y la mediana de cada orden.
- [x] 2c. Si el paso se adopta: vaciar `docs/eval/cache/` (`rm -rf docs/eval/cache` y comprobar con `test ! -e docs/eval/cache`); los vectores guardados son de otros bits (decisión 4). **No hace falta:** 2a y 2b no movieron ningún bit en esta CPU, así que la memoria sigue siendo exacta (`verify.md`).
- [x] 2d. «Se queda si» (modelo real, build release, autorización). Corridas alternadas base/paso, `rag-eval --real --docs --no-cache` de release en `target-test`, con el mismo `--root` (`cli.rs:235`) y `CLAWG_PROFILE=dev`, mínimo 3 pares. La base es un worktree descartable del commit anterior al grupo 2, con su build dir; se borran ambos al terminar. Orden: base, paso, base, paso, base, paso. Sin otra compilación ni corrida en paralelo.
  - Comando: `CLAWG_PROFILE=dev cargo run -p clawg-eval --release --target-dir target-test --bin rag-eval -- --real --docs --no-cache --root <raíz> --tag <etiqueta>` (en el worktree, con su `--target-dir` propio dentro del worktree para la base).
  - Comprobar: `phases.embed` baja ≥ 10 % (sobre las corridas repetidas y alternadas); `tokens.padded_ran == tokens.padded_sorted_in_doc` dentro de cada corrida del paso; `--diff` no mueve ninguna pregunta: `cargo run -p clawg-eval --target-dir target-test --bin rag-eval -- --diff docs/eval/results/<base>.json docs/eval/results/<paso>.json`; la decisión 4 se cumple (2a y 2b).
  - Cómo se corrió: los valores por omisión de `rag-eval` (`--root`, `--golden`, `--out`) salen de `CARGO_MANIFEST_DIR` al compilar. Por eso las dos ramas los recibieron explícitos y apuntando a un corpus congelado en un worktree de `eafcb8a`. Los binarios se llamaron directamente después de compilarlos, para que ningún build cayera dentro de una corrida.
- [x] 2e. Escribir `openspec/changes/docs-index-speed/verify.md` con los números de 2b y 2d (por corrida: `index_seconds`, `phases.*`, `tokens.*`, `by_area`) y el veredicto de cada condición. Commit: `docs(openspec): record the docs index ordering measurement`.
- [x] 2f. Borrar el worktree de la base y su directorio de build. La tarea termina sin ningún directorio de build que no sea `target` y `target-test`.

## Parte 3 — después de medir

- [ ] A2 tiene su propia actualización de spec, diseño y tareas cuando A1 esté medido. No se hace aquí.
- [ ] Archivar y subir la versión (minor) según `openspec/config.yaml` y `.claude/CLAUDE.md` sólo cuando el cambio se cierre, no ahora. (Ver Parte 8.)

# Paso A2

Requisitos IS6 a IS17 en la [spec](spec.md), «Paso A2», y decisiones D8 a D15 en el [diseño](design.md), «Paso A2». Las reglas de commit, TDD y comprobación son las de arriba. Antes del primer test de la Parte 4: `cargo clean --workspace --target-dir target-test` (otra versión de las fuentes).

## Orden elegido

La pieza pura de D8 sólo la llama la cuenta ignorada hasta que `refresh` la use, y sin llamador de producción sería código muerto (`tests/conventions.rs`, avisos como errores). Por eso vive en el módulo `#[cfg(test)]` en la Parte 4 y se promueve a producción en el mismo commit que `refresh` con la cola (Parte 6, grupo 1). Las Partes 6 y 7a sólo existen si la Parte 5 elige un tope.

## Parte 4 — la cuenta del tope, sin modelo (lo hace apply)

### 7. La cuenta ignorada y la decisión de vaciado (IS6, IS7; D8, D9)

- [x] RED en el módulo `tests` de `crates/clawg-core/src/memory/project_docs/mod.rs`, sobre una función pura `queue_flush` (nombre libre) definida dentro de ese mismo módulo, no en producción: `the_queue_limit_starts_at_one_batch_and_doubles_up_to_the_cap`, `the_queue_flushes_before_a_doc_that_would_pass_its_limit_and_when_it_reaches_it`, `a_doc_over_the_limit_is_a_flush_of_its_own`, `a_doc_with_no_passages_moves_no_flush`. Fallan por no compilar.
- [x] GREEN: la función recibe el tope c (parámetro) y lleva como único estado lo que retiene la cola y cuántos vaciados se hicieron; con el tamaño del doc que llega dice si vaciar antes y si vaciar después. Límite inicial `EMBED_BATCH`, se duplica por vaciado hasta c.
- [x] Escribir y compilar, sin correr, `padding_and_batches_by_queue_cap` (`#[ignore]`), junto a `sorting_across_docs_against_sorting_in_each_doc` (`mod.rs:2450`): sólo `load_verified_tokenizer` (como `real_model`, `:2339`) y los prefijos de `expected_contract()` (`embedder.rs:157-164`), sin sesión de ONNX; `CLAWG_DOCS_ROOT` cambia el árbol (por omisión, este repositorio); parte como `repo_docs` (`:2353`), en orden de ruta, con `passage_token_counts` (`embedder.rs:268-279`). Imprime P, B, Q, los pasajes de la llamada más grande y H (Σ 3 × bytes del texto + 1536, máximo sobre los vaciados) para: orden de los pasajes, en cada doc, la cola con c ∈ {64, 128, 256, 512, 1024, 2048, 4096} y todo junto; y P, B y Q en cada doc y todo junto sobre la muestra de `sorting_across_docs_…` (`:2455-2458`). No abre un store ni usa el temporal.
- [x] Comprobar: `cargo test -p clawg-core --target-dir target-test --lib the_queue` y `... a_doc_with_no_passages_moves_no_flush` pasan; `cargo test -p clawg-core --target-dir target-test --no-run` compila. NO correr la cuenta.
- Commit: `test(project-docs): count the padding and batches of a bounded queue`

## Parte 5 — correr la cuenta (sólo el tokenizer; autorización; la pide el orquestador, apply no la corre)

Usa `CLAWG_E5_MODEL_DIR` del perfil dev (`<data dir dev>/models/multilingual-e5-small`, sólo lectura), nunca `~/.clawg`. Sin otra compilación en marcha; no matar ningún `clawg.exe`.

- [ ] 5a. Sobre HEAD y este repositorio: `CLAWG_E5_MODEL_DIR=… cargo test -p clawg-core --lib --target-dir target-test padding_and_batches_by_queue_cap -- --ignored --nocapture`. Guardar la salida (P, B, Q, pasajes, H por orden y por c).
- [ ] 5b. Las tandas «en cada doc» del techo: worktree descartable de `0c9d49a` y `CLAWG_DOCS_ROOT=<worktree>`; la prueba se lleva al worktree (como parche fuera del repo) con su propio `target-test` dentro. Validar que reproduce P = 1 998 461 (en cada doc) y 1 614 977 (todo junto) sobre la muestra; si no, B no vale y la Parte 5 se detiene y se informa. Tomar B de cada doc de la muestra (197 docs, Σ ⌈n/16⌉) y B todo junto (329). Borrar el worktree y su build: en Windows, `cmd //c 'rd /s /q "\\?\<ruta>"'` y `git worktree prune`.
- [ ] 5c. Aplicar la regla de D9 tal cual: ρ_X, R_X(c), previsto(c) = 0,210 × min(1, min_X ρ_X(corpus) / ρ_X(muestra)) × min_X R_X(c), y elegir el menor c con H(c) ≤ 16 MiB y previsto(c) ≥ 0,18. Escribir en `verify.md` («A2: la cuenta del tope») la tabla (P, B, Q, H, R_X y previsto por c, y la muestra) y la decisión. Commit: `docs(openspec): record the queue cap count`.
- [ ] 5d. **Si ningún c cumple:** A2 se cierra sin código de producción. Se saltan la Parte 6, la Parte 7 completa (incluida 7a) y el chequeo de pico; IS7 a IS17 quedan sin implementar salvo IS6. Se marcan como no aplicables, el cambio va a archivo junto con A1 (Parte 8) y no hay commit de producción.
- [ ] 5e. **Si uno cumple:** `QUEUE_PASSAGES` = ese c y sigue la Parte 6.

## Parte 6 — código de producción (sólo si la Parte 5 eligió un tope; lo hace apply)

### 8. `refresh` embebe en vaciados de una cola (IS7 a IS14; D8 a D12)

- [ ] RED en `project_docs/mod.rs`, con `Batches` (`:2031`), el tokenizer de palabras, `try_refresh` y `test_dir()`: `small_docs_share_one_call_to_the_embedder`, `no_call_holds_more_passages_than_the_cap_unless_it_is_one_doc` (docs que suman más que `QUEUE_PASSAGES`), `each_passage_of_a_shared_call_keeps_its_own_vector`, `an_unusable_vector_in_a_shared_call_skips_only_its_doc`, `a_shared_call_writes_its_docs_in_plan_order` (también compara `RefreshReport`), `an_embedder_error_keeps_the_docs_of_its_call_as_they_were`, `embed_shares_split_the_time_by_texts_and_add_up_to_it` (pura) y `every_doc_of_a_shared_call_has_embed_time`. Siguen pasando: `a_doc_over_a_batch_reaches_the_embedder_longest_first`, `a_failed_embedding_keeps_the_doc_as_it_was`, `a_short_batch_still_aborts_the_refresh`, `an_unusable_vector_skips_its_doc_and_the_rest_go_on`, `a_refresh_times_a_doc_whose_vector_is_unusable`, las dos de `vector_memo.rs:889-930` y `the_docs_timing_line_names_every_phase` (`api.rs:3207`).
- [ ] GREEN: promover la pieza de la Parte 4 a producción (`const QUEUE_PASSAGES` = c elegido, la prueba de la Parte 4 llama ahora a la de producción); `refresh` (`:515-563`) lee, enmascara, parte y busca vectores de cada doc y lo encola; al vaciar manda los textos en el orden del plan por `passage_order` sobre la lista entera, vuelve con `in_passage_order` y reparte por doc; cada doc pasa por `to_unit` (inutilizable a `failed`), `fill` y `replace_doc` en el orden del plan; vaciar al terminar el plan antes de `remove_docs`. `embed_passages` pasa a embeber un vaciado y devolver el resultado de cada doc; `Embed` y `VectorCount` nombran el primer doc, con `passages` = textos del vaciado. Los textos se ordenan moviéndolos, sin clonarlos (D13). `embed_shares` reparte el `lap` en nanosegundos enteros con el resto al último doc; un `DocTiming` se agrega al escribirse o fallar el doc. Actualizar los comentarios de los errores y el de «one changed doc» (`:518-520`).
- [ ] Docs: `docs/reference/daemon.md`, en el párrafo del índice de docs: la cola, su tope, el límite que crece, los fallos y que `embed` incluye contar, ordenar y repartir.
- [ ] Comprobar: `cargo test -p clawg-core --target-dir target-test project_docs` y `cargo test -p clawg-cli --target-dir target-test docs_timing`.
- Commit: `perf(project-docs): embed the docs of a refresh in shared calls`

### 9. La evaluación agrupa como el refresco (IS15; D14)

- [ ] RED en `crates/clawg-core/src/eval_api.rs`: `queue_groups_are_the_calls_a_refresh_makes` (con `--features eval`: los grupos que da `queue_groups` son las llamadas que registra un refresco con `Recording`).
- [ ] GREEN: `pub fn queue_groups(passages_per_doc: &[usize]) -> Vec<Vec<usize>>` (nombre y forma libres), que delega en la pieza de producción con `QUEUE_PASSAGES`, como `eval_api::passage_order` (`:1196`).
- [ ] Comprobar: `cargo test -p clawg-core --features eval --target-dir target-test --lib eval_api`.
- Commit: `feat(eval): expose how a docs refresh groups its docs into calls`

### 10. `padded_sorted_queue` y el aviso (IS15; D14)

- [ ] RED en `crates/clawg-eval/src/bin/rag-eval/index_cost.rs`: `padded_sorted_queue_sorts_each_group_of_docs_together`, `the_padding_warning_compares_with_the_queue_padding` (`padded_sorted_queue = 800`, `padded_ran = 800`: sin aviso; con 900 avisa y nombra 800 y 900; con `from_memo > 0`, nunca) y reescribir `the_padding_warning_names_both_numbers` y `a_mismatch_between_computed_and_ran_padding_warns_only_when_nothing_came_from_the_memo` sobre ese campo.
- [ ] GREEN: `tokens.padded_sorted_queue` (grupos de `queue_groups` sobre los docs en orden de ruta, cada uno con `padded_sorted`, `:96`); `padding_warning` (`:154`) compara contra él; `Tokens::line` y los comentarios de `Tokens`. `padded` y `padded_sorted_in_doc` no cambian de nombre ni de sentido.
- [ ] Docs: `docs/reference/testing.md` si describe el aviso.
- [ ] Comprobar: `cargo test -p clawg-eval --target-dir target-test index_cost`.
- Commit: `feat(eval): warn when the padding that ran differs from the queue order`

### Cierre de la Parte 6

- [ ] Sobre el último commit: `cargo test --workspace --target-dir target-test` y los dos clippy de arriba. Ambos deben pasar sin avisos.

## Parte 7 — medir (modelo real, release, autorización; la pide el orquestador, apply no la corre)

- [ ] 7a. Escribir y compilar, sin correr: el tercer orden de `sorting_across_docs_against_sorting_in_each_doc` (D15), la cola con `QUEUE_PASSAGES` por la función de vaciado de producción; informa cuántos vectores dan los mismos bits que en cada doc, la diferencia mayor y, en debug, el tiempo de la cola (vista previa, no decide). No hay caso nuevo en `e5_golden` ni cambia su tolerancia. Comprobar con `--no-run` y los dos clippy. Commit: `test(project-docs): report whether the queue order moves bits`.
- [ ] 7b. Bits (modelo real): `CLAWG_E5_MODEL_DIR=… CLAWG_EMBED_PASSAGES=5000 CLAWG_EMBED_RUNS=3 cargo test -p clawg-core --lib --target-dir target-test sorting_across_docs_against_sorting_in_each_doc -- --ignored --nocapture`. Si algún bit se mueve, vale la decisión 4 (≤ 1e-6 y `--diff` sin cambios; si no, el paso cae). Si no se mueve ninguno, nada guardado cambia.
- [ ] 7c. «Se queda si»: base = último commit de código de A1 (`eafcb8a`, o el que el orquestador fije), paso = HEAD. Corridas alternadas `rag-eval --real --docs --no-cache` de release, 3 pares (base, paso, base, paso, base, paso), con `CLAWG_PROFILE=dev`, el mismo worktree de corpus congelado y `--root`, `--golden` y `--out` explícitos. La base se compila en un worktree descartable con su propio build; el paso en `target-test`. Se llaman los binarios directamente después de compilar, sin otra compilación ni corrida en paralelo. Se comparan corridas vecinas del mismo régimen de la máquina, con `phases.split` como indicador (como `verify.md`, «A1: «se queda si»»); un par que cruza un cambio de régimen no se usa. Condiciones: `phases.embed` baja ≥ 15 %; `tokens.padded_ran == tokens.padded_sorted_queue` y sin aviso en cada corrida del paso; `--diff` base-n contra paso-n no mueve ninguna pregunta; decisión 4.
- [ ] 7d. Pico de memoria (D13, con autorización): perfil dev sin otro daemon dev, builds release, un `clawg serve --profile dev` nuevo por rama, copia congelada del corpus registrada con `clawg projects add`, esperar `docs refresh of …: done` y un minuto más, leer `clawg metrics report --metric process.private_peak_bytes --by minute --since 1h --profile dev`; alternar base, paso, base, paso y comparar vecinas. Si el pico ya está fijado antes del refresco en las dos ramas, leer el máximo por minuto de `process.private_bytes` durante el refresco. Condición: ≤ 5 %.
- [ ] 7e. Registrar en `verify.md` («A2: «se queda si»») por corrida `index_seconds`, `phases.*`, `tokens.*` y `by_area`, el pico y los bits, con el veredicto de cada condición. Si A2 no se queda, se revierten los commits 8 a 10 (y 7a) y se cierra con la tabla de la Parte 5. Commit: `docs(openspec): record the queue ordering measurement`.
- [ ] 7f. Borrar los worktrees (corpus y base), sus directorios de build, las copias del corpus y sus proyectos del perfil dev. Si el paso se adopta y algún bit se movió, vaciar `docs/eval/cache/`. La tarea termina sin ningún build dir que no sea `target` y `target-test`.

## Parte 8 — cierre

- [ ] Archivar el cambio (A1 y, si se adoptó, A2) y subir la versión (minor) según `openspec/config.yaml` y `.claude/CLAUDE.md`, con `cargo update --workspace` y `chore: bump version to X.Y.Z`; sólo cuando el cambio se cierre. Si A2 cerró en 5d, se archiva con A1 sin código de A2.
