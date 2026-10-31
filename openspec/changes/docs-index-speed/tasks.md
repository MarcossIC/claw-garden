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
- [ ] Este grupo NO se commitea en la Parte 1: el test no pasa sin el bless y la suite se rompería. Se deja el árbol con estos cambios (sin `git add`) y se pasa a la Parte 2a, o se guarda como parche fuera del repo.
- [ ] Docs: `docs/reference/testing.md`, si habla de los casos de `e5_golden`.
- Commit (después de 2a y 2b): `test(embedder): pin the vectors of a batch embedded longest first`, con `crates/clawg-core/tests/e5_golden.rs`, `crates/clawg-core/tests/fixtures/e5_golden.json` y la doc.

### Cierre de la Parte 1

Los grupos 1 a 5 se commitean y se comprueban cada uno con sus archivos. La suite completa y los dos clippy corren sobre el commit del grupo 6, el último de la serie, cuando ya trae la fixture blessed (Parte 2a). Si el orquestador pide verlos antes, se corren con el grupo 6 fuera del árbol, y se repiten tras la Parte 2a.

## Parte 2 — necesita el modelo real y autorización explícita (la pide el orquestador; apply no la corre)

Una a la vez, sin otra compilación ni prueba en marcha. Usan el modelo del perfil dev (`~/.clawg-dev/models`, sólo lectura), nunca `~/.clawg`. No matar ningún `clawg.exe`. Cada ítem marca qué necesita.

- [ ] 2a. Bless de `e5_golden` (modelo real, autorización, máquina de la fixture: Windows `x86_64` con AVX2, AVX-512F y FMA): `CLAWG_BLESS=1 cargo test -p clawg-core --target-dir target-test --test e5_golden -- --ignored the_vectors_are_the_ones_fastembed_gave`. Revisar `git diff` de `crates/clawg-core/tests/fixtures/e5_golden.json`: sólo debe agregar el caso nuevo. Correr sin `CLAWG_BLESS`: `cargo test -p clawg-core --target-dir target-test --test e5_golden` (incluye el test sobre la fixture). Commitear el grupo 6.
  - Si algún texto difiere más de 1e-6 entre `a batch of 17` y el caso nuevo, el paso cae (propuesta, decisión 4). En ese caso:
    - Se corre igual 2b y sus números se anotan en `verify.md`.
    - No se commitea el grupo 6 y se descartan sus cambios.
    - Se revierten los grupos 2 a 5: el aviso del grupo 4 y la prueba del grupo 5 suponen el orden nuevo. El grupo 1 queda, porque es un movimiento sin cambio de conducta.
    - La tolerancia no se agranda, y el tema pasa a 17 model-reembed.
- [ ] 2b. Prueba de IS5 (modelo real, autorización): `CLAWG_E5_MODEL_DIR=<data dir dev>/models/multilingual-e5-small cargo test -p clawg-core --target-dir target-test docs_passages_keep_their_bits_in_either_order -- --ignored --nocapture`. Anotar en `verify.md`: «N de M con los mismos bits», la diferencia mayor y la mediana de cada orden.
- [ ] 2c. Si el paso se adopta: vaciar `docs/eval/cache/` (`rm -rf docs/eval/cache` y comprobar con `test ! -e docs/eval/cache`); los vectores guardados son de otros bits (decisión 4).
- [ ] 2d. «Se queda si» (modelo real, build release, autorización). Corridas alternadas base/paso, `rag-eval --real --docs --no-cache` de release en `target-test`, con el mismo `--root` (`cli.rs:235`) y `CLAWG_PROFILE=dev`, mínimo 3 pares. La base es un worktree descartable del commit anterior al grupo 2, con su build dir; se borran ambos al terminar. Orden: base, paso, base, paso, base, paso. Sin otra compilación ni corrida en paralelo.
  - Comando: `CLAWG_PROFILE=dev cargo run -p clawg-eval --release --target-dir target-test --bin rag-eval -- --real --docs --no-cache --root <raíz> --tag <etiqueta>` (en el worktree, con su `--target-dir` propio dentro del worktree para la base).
  - Comprobar: `phases.embed` baja ≥ 10 % (sobre las corridas repetidas y alternadas); `tokens.padded_ran == tokens.padded_sorted_in_doc` dentro de cada corrida del paso; `--diff` no mueve ninguna pregunta: `cargo run -p clawg-eval --target-dir target-test --bin rag-eval -- --diff docs/eval/results/<base>.json docs/eval/results/<paso>.json`; la decisión 4 se cumple (2a y 2b).
- [ ] 2e. Escribir `openspec/changes/docs-index-speed/verify.md` con los números de 2b y 2d (por corrida: `index_seconds`, `phases.*`, `tokens.*`, `by_area`) y el veredicto de cada condición. Commit: `docs(openspec): record the docs index ordering measurement`.
- [ ] 2f. Borrar el worktree de la base y su directorio de build. La tarea termina sin ningún directorio de build que no sea `target` y `target-test`.

## Parte 3 — después de medir

- [ ] A2 tiene su propia actualización de spec, diseño y tareas cuando A1 esté medido. No se hace aquí.
- [ ] Archivar y subir la versión (minor) según `openspec/config.yaml` y `.claude/CLAUDE.md` sólo cuando el cambio se cierre, no ahora.
