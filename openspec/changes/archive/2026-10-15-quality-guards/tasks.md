# quality-guards — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. Compuerta de código muerto (R4, D6) — el test, en rojo

- [x] Test `no_source_allows_dead_code` y el puro `hides_dead_code_recognises_the_attribute_forms` (`crates/clawg-cli/tests/conventions.rs`). Correrlo sobre `main`: falla con `clock.rs:99`, `chunker/mod.rs:97` y `golden.rs:167`.
- No se commitea todavía: entra en la tarea 5, cuando pasa.

Al correr `cargo test -p clawg-core` solo apareció otro warning, `search_all_memories_with_scoring` (`search.rs`), que sólo llama `eval_api` y queda sin uso sin la feature `eval`; en `--workspace` no se ve porque `clawg-eval` la activa. Va en su propio commit: `refactor(core): compile the search with a scoring config only for the eval feature`.

## 2. Sin el helper huérfano (R5)

- [x] Borrar `insert_row_with_embedding` (`crates/clawg-core/src/memory/worker.rs:1280`). `cargo test -p clawg-core` deja de dar el warning.
- Commit: `test(core): drop a helper no test calls`

## 3. Sin `age_days_from` (R5)

- [x] Borrar `TimestampMs::age_days_from` y sus tests (`clock.rs`); el comentario de `score.rs:247` deja de nombrarlo y conserva el porqué de la fórmula.
- Commit: `refactor(core): drop the unused day-age helper`

## 4. `id_of` sólo en tests y sin `token_count` (R5)

- [x] `RefMap::id_of` bajo `#[cfg(test)]` (`crates/clawg-eval/src/bin/rag-eval/golden.rs`).
- Commit: `refactor(eval): compile RefMap::id_of only for tests`
- [x] Borrar `Chunk::token_count` (`chunker/mod.rs`) y sus sitios de construcción (`token_splitter.rs`, `project_docs/mod.rs`, `store_insert.rs`, `worker.rs`); los tests de `token_splitter.rs` que lo leían siguen fijando el texto exacto de cada pedazo.
- Commit: `refactor(core): drop the token count no one reads`

## 5. La compuerta, en verde (R4)

- [x] El test de la tarea 1 pasa.
- Commit: `test: fail when a source hides dead code behind an allow`

## 6. Contrato del agente (R1, R2, D1–D4)

- [x] Tests puros `contract_diff_names_the_changed_tool`, `contract_diff_ignores_key_order` y `render_contract_is_stable_and_sorted` (`server.rs`).
- [x] Test `the_agent_contract_matches_its_file`: falla porque el archivo no existe.
- [x] `agent_contract`, `contract_diff` y `render_contract` en el `mod tests` de `server.rs`; el archivo inicial con `CLAWG_BLESS=1`, revisado a mano contra los `#[tool(` y los `*Args`.
- [x] A mano: cambiar una palabra de una descripción hace fallar el test y el mensaje nombra la herramienta; se revierte.
- Commit: `test(mcp): pin what the model sees in a versioned contract file`

## 7. Inventario de `.claude/CLAUDE.md` (R3, D5)

- [x] Test puro `tool_names_in_doc_reads_the_mcp_tools_line` y `the_claude_md_tool_list_is_the_registered_one` (`server.rs`). Pasa hoy; se comprueba en rojo quitando un nombre del documento.
- Commit: `test(mcp): check the tool list in CLAUDE.md against the registered tools`

## 8. Docs (R6, D7)

- [x] `AGENTS.md` como puntero a `.claude/CLAUDE.md`; `README.md:412` manda a `.claude/CLAUDE.md`.
- [x] `.claude/CLAUDE.md`, Testing: el comando de `clippy` de D7 y cómo bendecir el contrato.
- [x] Comentario de `AUTO_KEEP` con `pre-reembed`; `openspec/specs/storage/spec.md:225` con las líneas de hoy; `README.md:110` y `:143` sin número de versión.
- [x] `openspec/README.md`: fila 32 y los hallazgos de `clippy` en el backlog.
- Commit: `docs: point AGENTS.md at CLAUDE.md and document the quality checks`

## 9. Verificación

- [x] `cargo test --workspace` en cada commit; conteo contra la base y sin warnings del workspace.
- [x] El comando de `clippy` de D7 termina con código 0.
- [x] Bendecir a mano (R2): con `CLAWG_BLESS=1` reescribe y falla; la siguiente pasa; dos bendiciones dan los mismos bytes.
- [x] `verify.md` con los pasos locales: la suite y el `clippy` en Windows.
- [x] Correr esos pasos: 1.918 pasan y 0 fallan, `clippy` con código 0.

## 10. Archive

- [x] Spec nueva `openspec/specs/quality/spec.md` con R1 a R4 y R6, y la línea **Origen**.
- [x] `git mv` a `changes/archive/2026-10-15-quality-guards`; `state.yaml` archivado; fila 32 del `README.md` de openspec; base de tests de `.claude/CLAUDE.md` y `openspec/config.yaml` (ya en 1.918, que incluye este cambio); subir el minor.
