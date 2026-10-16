# quality-guards — Verificación

**Estado:** verificado en Linux, commit por commit. Falta la suite y el `clippy` en Windows ([pendiente](#pendiente-máquina-de-marcos)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama, en un worktree aparte.

| Commit | Pasan | Fallan | Warnings del workspace |
|---|---|---|---|
| Base (`main`, 638812b) | 1.886 | 10 | 1 (`insert_row_with_embedding`) |
| `docs(openspec): propose pinning the agent contract and failing on hidden dead code` (c71a8ce) | 1.886 | 10 | 1 |
| `test(core): drop a helper no test calls` (535bac2) | 1.886 | 10 | 0 |
| `refactor(core): drop the unused day-age helper` (cf004be) | 1.884 | 10 | 0 |
| `refactor(core): compile the search with a scoring config only for the eval feature` (1e45169) | 1.884 | 10 | 0 |
| `refactor(eval): compile RefMap::id_of only for tests` (6b482fe) | 1.884 | 10 | 0 |
| `refactor(core): drop the token count no one reads` (c5c1a96) | 1.884 | 10 | 0 |
| `test: fail when a source hides dead code behind an allow` (7aa2c9d) | 1.886 | 10 | 0 |
| `test(mcp): pin what the model sees in a versioned contract file` (02754f1) | 1.890 | 10 | 0 |
| `test(mcp): check the tool list in CLAUDE.md against the registered tools` (2f2341a) | 1.892 | 10 | 0 |
| `docs: point AGENTS.md at CLAUDE.md and document the quality checks` (f8f8690) | 1.892 | 10 | 0 |

Todos los commits compilan. Las 10 fallas son las mismas en todos y en `main`, y vienen del entorno: 7 de `doctor::tests` (modelan un PATH de Windows con `clawg.exe`) y 3 de `diff::tests` en `rag-eval` (leen `docs/eval/results/`, que está en `.gitignore`).

**Conteo:** 8 tests nuevos y 2 borrados con `age_days_from` (`ninety_days_apart_is_ninety_days`, `an_instant_after_now_reports_a_negative_age`); `now_reads_a_plausible_present_instant` se quedó y ahora compara milisegundos. Nuevos: 2 en `crates/clawg-cli/tests/conventions.rs`, 4 del contrato y 2 del inventario en `server.rs`. Neto: +6.

### Qué cubre cada requisito

| Requisito | Tests o comprobación |
|---|---|
| R1 contrato fijado | `the_agent_contract_matches_its_file`, `contract_diff_names_the_changed_tool`, `contract_diff_ignores_key_order`. A mano: cambiar «Delete» por «Remove» en la descripción de `delete_memory` da `delete_memory changed: description` |
| R2 bendecir | `render_contract_is_stable_and_sorted`. A mano: sin el archivo, el test falla y dice cómo crearlo; con `CLAWG_BLESS=1` lo escribe y falla; dos bendiciones seguidas dan el mismo md5; la corrida siguiente pasa |
| R3 inventario | `tool_names_in_doc_reads_the_mcp_tools_line`, `the_claude_md_tool_list_is_the_registered_one`. A mano: quitar `restore_memory` y agregar `forget_all` en `.claude/CLAUDE.md` da `missing ["restore_memory"], not registered ["forget_all"]` |
| R4 compuerta | `hides_dead_code_recognises_the_attribute_forms`, `no_source_allows_dead_code`. Sobre `main` falló con `golden.rs:167`, `chunker/mod.rs:97` y `clock.rs:99` |
| R5 código sin llamador | La tabla de arriba: 0 warnings desde el segundo commit. Además `cargo test -p clawg-core` solo, sin la feature `eval`, ya no da el warning de `search_all_memories_with_scoring` |
| R6 docs | Revisión del diff; R3 cubre la lista de herramientas |

El archivo inicial del contrato (`crates/clawg-cli/src/mcp/agent_contract.json`, 18 KB) tiene las 12 herramientas, cada una con sólo `name`, `description` e `inputSchema`, y el texto de `INSTRUCTIONS`. Se revisó contra los `#[tool(` y los `*Args` de `server.rs`.

## clippy

- `cargo clippy --workspace --all-targets -- -D warnings` en `main`: no pasa. Sin `-D` da 30 hallazgos, 29 de `clippy` y uno de `rustc`.
- En la punta, sin `-D`: 28 de `clippy` (uno estaba en código borrado), anotados en el backlog del README de openspec.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`, el comando documentado: **código 0** en la punta.

## rag-eval

No aplica: no cambia búsqueda, puntaje ni prompt. Lo que el modelo ve es el mismo texto que en `main`, ahora fijado.

## Pendiente (máquina de Marcos)

1. **Suite en Windows:** `cargo test --workspace --no-fail-fast --target-dir target-test` en la punta. Se esperan 1.902 + 6 = **1.908 pasan, 0 fallan**. Si `the_agent_contract_matches_its_file` falla en Windows, el mensaje dice qué herramienta difiere; lo esperable es que no, porque las descripciones vienen de los doc comments y no dependen del sistema.
2. **`clippy` en Windows:** `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings` termina con código 0. Puede haber un warning sólo de Windows en código con `cfg(windows)`, que Linux no compila.
