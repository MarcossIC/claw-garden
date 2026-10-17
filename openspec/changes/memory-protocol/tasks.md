# memory-protocol — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. Una sola lista de tipos (R5, D4)

- [ ] Tests `every_listed_type_parses_back_to_itself` y `an_unknown_type_names_the_accepted_ones` (`store_insert.rs`): no compilan sin `MemoryType::ALL` ni `FromStr`.
- [ ] `MemoryType::ALL`, `as_str` público, `FromStr` con `UnknownMemoryType`. `PatchError::UnknownType` lo envuelve, y los tests de `admin.rs` se adaptan.
- Commit: `refactor(core): keep the memory types in one list with one parse error`

## 2. El save rechaza un tipo desconocido (R6, D3)

- [ ] Tests `a_save_with_an_unknown_memory_type_is_refused_before_the_outbox` y `save_and_patch_accept_the_same_memory_types` (`crates/clawg-cli/tests/api.rs`): hoy el save contesta 201.
- [ ] Test `save_memory_says_an_unknown_type_was_not_saved` (`server.rs`): hoy dice `daemon returned 400`.
- [ ] Validación en `save_memory` (`api.rs`) y la rama `not saved:` de `save_outcome_text`.
- Commit: `feat(cli): refuse a save whose memory_type is unknown`

## 3. El enum en el schema (R7, D2)

- [ ] `the_save_memory_schema_names_the_accepted_memory_types` compara el enum de `save_memory` y de `update_memory` con `MemoryType::ALL`, y su comentario se corrige. Hoy falla porque el schema no trae `enum`.
- [ ] `memory_type_enum` como `transform` en los dos campos. El contrato se bendice y el diff se revisa.
- Commit: `feat(mcp): declare the accepted memory types in the tool schemas`

## 4. El protocolo y las instrucciones (R1–R4, R8, D1, D5)

- [ ] Tests puros `opening_joins_the_lines_of_each_paragraph` y `backticked_names_reads_each_code_span`.
- [ ] Tests `the_instructions_are_the_opening_of_the_protocol`, `the_instructions_fit_older_clients`, `the_instructions_carry_the_saving_habits`, `the_protocol_describes_the_session_summary`, `the_protocol_names_only_what_exists` y `save_memory_asks_for_a_title_and_where`.
- [ ] `protocol.md`, `opening` y `get_info` con la apertura. La descripción de `save_memory` pide título y "dónde". El contrato se bendice y el diff se revisa.
- Commit: `feat(mcp): send the opening of a compiled memory protocol as the server instructions`

## 5. La nota de conflictos en inglés (R9)

- [ ] Test `the_conflict_note_is_in_english`, y los de `prompt_builder.rs:745-790` adaptados al texto nuevo. El fixture de `agent_view/parse.rs:318` pasa a usar la nota nueva.
- [ ] `CONFLICT_NOTE` y la línea de omitidos en inglés. `specs/retrieval` toma el texto nuevo cuando se archiva el cambio.
- Commit: `feat(core): write the conflict note in English`

## 6. Docs

- [ ] `.claude/CLAUDE.md`: dónde vive el protocolo, que su apertura son las instrucciones y que el save rechaza un tipo desconocido.
- Commit: `docs: describe the memory protocol and the memory_type check on save`

## 7. Verificación

- [ ] `cargo test --workspace` y el `clippy` de `.claude/CLAUDE.md` en Linux.
- [ ] Lo mismo en Windows, en `target-test`.
- [ ] Una sesión de Claude Code sobre el perfil dev, con autorización: el agente guarda una decisión y un bug según el protocolo, y un save con un tipo inventado vuelve con la lista.
- [ ] `verify.md`.

## 8. Archive

- [ ] Specs nuevas o actualizadas: `specs/mcp` (instrucciones, protocolo, enum), `specs/ingest` (el rechazo del tipo) y `specs/retrieval` (la nota en inglés). Cada una con su línea **Origen**.
- [ ] `git mv` a `changes/archive/AAAA-MM-DD-memory-protocol`, `state.yaml` archivado, fila 22 del `README.md` de openspec y orden de ejecución. Subir el minor.
