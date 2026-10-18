# memory-provenance — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. Sello y marca en el núcleo (P1, P3, P6)

- [x] Tests puros `seal_cases` y `mark_cases`, y `rows_saved_before_provenance_keep_their_origin_and_no_mark`, en `provenance.rs`; `an_event_without_user_stated_reads_as_undeclared` en `events.rs`.
- [x] `provenance.rs` con `seal` y `mark`; `MemorySource::Synthesis`; `MemoryEvent.user_stated`; `from_event_and_chunk` sella con `seal`.
- Commit: `feat(core): seal each save with who stated it`

## 2. La declaración por HTTP y MCP (P2)

- [x] Tests en `tests/api.rs` y `server.rs`.
- [x] `user_stated` en `SaveRequest`, `SaveArgs` y `SaveMemoryArgs`; el 400 con `derived_from`; `protocol.md`; el contrato se bendice y el diff se revisa.
- Commit: `feat(cli): let save_memory declare what the user stated`

## 3. Los pares (P4)

- [x] Tests en `dispute.rs`, `prompt_builder.rs` y `tools.rs`.
- [x] `OpenPair` y `ConflictPair` llevan la marca de cada lado; `format_open_pairs` ordena y marca; `format_conflict_block` marca.
- Commit: `feat: show which side of a conflict pair was inferred`

## 4. Síntesis (P5)

- [x] Test en `tools.rs`.
- [x] `synthesis_line` escribe la marca.
- Commit: `feat(cli): mark conclusions and inferred memories in synthesize`

## 5. Docs

- [x] `.claude/CLAUDE.md`: `user_stated`, el sello y las marcas.
- Commit: `docs: describe memory provenance`

## 6. Verificación

- [x] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [x] Lo mismo en Windows, en `target-test`.
- [x] Perfil dev con Claude Code: un save declarado, uno sin declarar y una conclusión, vistos en `clawg memories show` y en `memory_status`.
- [x] `verify.md`.
