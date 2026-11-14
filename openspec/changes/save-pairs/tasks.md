# save-pairs — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla por la aserción o por no compilar. Cada tarea es un commit convencional, comprobado con los tests de los archivos que cambia. La suite entera y los dos clippy se corren una sola vez, en el último commit (tarea 4).

## 1. La guarda de punta a punta (R3)

- [x] `a_long_save_is_judged_through_the_first_id_it_answers` en `crates/clawg-cli/src/mcp/server.rs` (mod tests, `bridge_with_store`). Los pasos:
  1. guardar el texto largo donde el save del test no lo alcanza, en otro proyecto, o borrarlo y purgarlo después;
  2. leer el vector de su última pieza;
  3. guardar la vieja y ubicar su vector a un ángulo de 0,3 de ese, como `a_restore_answers_its_key_and_its_new_pairs` (`crates/clawg-cli/tests/api.rs`);
  4. guardar el texto con `save_memory`;
  5. leer el primer id de `saved #…` y comprobar que el par es de otra pieza;
  6. llamar a `resolve_conflict_impl` con `supersedes`.
- [x] Hoy pasa, así que se commitea como guarda: `test(mcp): judge a long save through the first id it answers`.

## 2. Un save reanudado se empareja una vez (R1)

- [x] Tests primero, en `crates/clawg-core/src/memory/ingest/worker.rs`. Se arman con `record`, `replay_pending` y `NanAtEmbedder` para cortar la primera toma, y con un embedder que ubica cada pieza por su texto. Así las dos tomas dan el mismo vector a la misma pieza.
  - `a_resumed_save_pairs_once_with_an_earlier_save`: hoy falla, porque quedan dos pares y el juicio con la primera pieza da `SeveralPairs`.
  - `a_resumed_save_pairs_once_with_a_save_made_while_it_waited`: hoy falla, con dos pares entre el save y X.
  - `a_resumed_absorbed_piece_does_not_seed_the_pairs`: guarda.
  - `the_saves_a_resumed_save_already_paired_with`, en `crates/clawg-core/src/memory/revision/contradiction.rs`: la función pura que pasa de los pares y los vínculos a los `SaveKey`, con la misma regla que `detect_contradiction` (`contradiction.rs:235-238`).
- [x] `gate_pieces` devuelve en `Gated` los ids que este save insertó en una toma anterior. `store_pieces` siembra `paired` antes del bucle con una lectura de `memory_conflict` de esos ids, en los dos lados, y de `save_links` de las otras filas.
- [x] Commit: `fix(ingest): pair a resumed save once with each earlier save`.

## 3. Un cierre que falla no nombra pares (R2)

- [x] Test primero: `a_save_whose_finish_fails_names_no_pairs` (`worker.rs`).
  - Un `TEMP TRIGGER` sobre `memory_piece` que hace `RAISE(ABORT)`.
  - El save con fila en el outbox (`Track`) y la vieja en la banda de su segunda pieza.
  - Primero se comprueban el informe y la fila. Después se quita el trigger, se corre `replay_pending`, y se comprueban el enlace y el juicio con la primera pieza.
  - Hoy falla, porque `related` nombra a la vieja.
- [x] `process_tracked` deja `related`, `replaced` y `revision_candidate` vacíos cuando `outbox_finish` o `finish_save` fallan. Para eso, `finish_save` devuelve el `Result` en lugar de tragarlo. El log no cambia.
- [x] Commit: `fix(ingest): name no pairs for a save whose finish failed`.

## 4. Verificación

- [ ] `cargo clean --workspace --target-dir target-test` antes de la primera corrida del cambio.
- [ ] `cargo test --workspace --target-dir target-test` y `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings`, en Windows, sobre el último commit.
- [ ] El CI en verde en Linux y Windows.
- [ ] `verify.md`: que la suite pasa, y qué test cubre cada requisito. Este cambio no toca el ranking, así que no corre `rag-eval`.

## 5. Archive

- [ ] Llevar los requisitos a las specs:
  - `specs/disputes`, «El detector anota candidatas», con R1;
  - `specs/pieces` R3, el párrafo «Si escribir el vínculo falla…», con R2;
  - `specs/storage`, «Un replay retoma donde quedó», con la siembra de R1;
  - `specs/mcp` o `specs/disputes` R6, con el test de R3.
- [ ] En `docs/reference/daemon.md`, la línea del outbox: qué responde un save cuyo cierre falló.
- [ ] `git mv` a `changes/archive/AAAA-MM-DD-save-pairs`, y `state.yaml` archivado.
- [ ] La fila 130 del `README.md` de openspec y la línea del `ROADMAP.md`.
- [ ] Subir el minor, con `cargo update --workspace`, en un commit `chore: bump version to X.Y.Z`.
