# memory-protocol — Verificación

## Linux (2026-10-17)

Sobre `28530eb`, en `target`:

- `cargo test --workspace`: la suite pasa, sin warnings del workspace. Corrió en cada commit del cambio.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: termina con código 0.
- El contrato del agente se bendijo dos veces a propósito. El diff se revisó a mano:
  - en `ef760c0`, `memory_type` de `save_memory` y de `update_memory` suma `enum` con los siete tipos y `null`, y su descripción deja de repetir la lista;
  - en `0412da0`, `instructions` pasa a ser la apertura de `protocol.md` (1.699 caracteres) y la descripción de `save_memory` suma la primera línea y el "dónde".

| Requisito | Cómo quedó |
|---|---|
| R1 | `the_instructions_are_the_opening_of_the_protocol` y el puro `opening_joins_the_lines_of_each_paragraph`, que también cubre `\r\n` (un checkout de Windows con `autocrlf`) |
| R2 | `the_instructions_fit_older_clients`: 1.699 de 2.048 |
| R3 | `the_instructions_carry_the_saving_habits` y `the_protocol_describes_the_session_summary`. Los tests de instrucciones de antes siguen verdes |
| R4 | `the_protocol_names_only_what_exists` y el puro `backticked_names_reads_each_code_span`. En rojo, con `save_memories` en el texto, el test nombra `save_memories` |
| R5 | `every_listed_type_parses_back_to_itself` y `an_unknown_type_names_the_accepted_ones`. `PatchError::UnknownType` envuelve el mismo error |
| R6 | `a_save_with_an_unknown_memory_type_is_refused_before_the_outbox`, `save_and_patch_accept_the_same_memory_types` y `save_memory_says_an_unknown_type_was_not_saved`. Antes del cambio, los dos primeros fallaban porque el save contestaba 201, y el tercero porque decía `saved #1`. La reproducción del outbox sigue fijada por `explicit_source_with_an_unknown_type_override_falls_back_to_semantic_fact` |
| R7 | `the_save_memory_schema_names_the_accepted_memory_types` compara el enum de las dos herramientas con `MemoryType::ALL` |
| R8 | `save_memory_asks_for_a_title_and_where` |
| R9 | `the_conflict_note_is_in_english` y los tests de la nota adaptados |

`cargo build -p clawg-cli`, sin la feature `eval` de `clawg-core`, da un warning ya presente en `af6fef4`: `ImportanceFactor::spanning` (`crates/clawg-core/src/memory/score.rs:88`) sin uso. `--workspace` no lo muestra porque `clawg-eval` activa la feature. No es de este cambio y queda para un arreglo suelto.

## Windows (2026-10-17)

Sobre `b302fd7` (main en 0.16.0, con context-budget ya mezclado), en `target-test` después de `cargo clean --workspace --target-dir target-test`:

- `cargo test --workspace --target-dir target-test`: pasan 2.175, no falla ninguno y queda 1 ignorado.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: termina con código 0.
- El checkout tiene `core.autocrlf = true`, así que `protocol.md` está con `\r\n` en disco (`git ls-files --eol`: `i/lf w/crlf`). Los tests de R1 a R4 leen ese archivo y pasan. La apertura que arma el binario mide 1.699 caracteres y no lleva ningún `\r`.
- `cargo build -p clawg-cli` sigue dando el warning de `ImportanceFactor::spanning` anotado arriba (antes se lo nombraba `ScoringConfig::spanning`). No es de este cambio y quedó en el backlog del README.

## Perfil dev con Claude Code (2026-10-17)

Con `target\debug\clawg.exe` compilado desde `b302fd7` y registrado en Claude Code como `clawg-dev` (`mcp --profile dev --agent claude-code`), contra un daemon dev 0.16.0:

- Las instrucciones llegaron enteras al host: los tres párrafos de la apertura, cada uno en una sola línea, hasta "leads to.", sin cortes ni `\r`.
- El schema de `save_memory` que vio el agente trae en `memory_type` el `enum` con los siete tipos y `null`, y la descripción pide la primera línea y el "dónde".
- El agente guardó una decisión (`architectural_decision`) y un bug con su causa raíz como pide el protocolo: en inglés, con una primera línea que nombra el tema, y con rutas e identificadores exactos. Una búsqueda en lenguaje natural trajo la decisión primero.
- Un save con `memory_type: "bug_fix"` volvió como error: `not saved: unknown memory_type "bug_fix"; expected one of semantic_fact, user_preference, architectural_decision, session_summary, code_block, chat_message, tool_error`. Claude Code no valida el `enum` antes de llamar, así que el valor llegó al daemon, como pide R7. El save rechazado no dejó ninguna fila (los ids pasaron de #57 a #58), y el reintento con `semantic_fact` se guardó.
- Las tres memorias de prueba se borraron.

### Hallazgo: qué tipo lleva un bug arreglado

El protocolo pide guardar un bug arreglado con su causa raíz («When to save»), pero «Type and importance» no dice qué tipo le corresponde. En la sesión, el agente lo guardó como `tool_error`, que el protocolo describe como "a tool failure" y del que dice "They fade fast". Un arreglo con su causa raíz es un hecho que tiene que durar. No bloquea el archivo. Queda como arreglo suelto en el backlog del [README](../../../README.md#backlog-sin-spec-arreglos-sueltos): que «Type and importance» diga que un bug arreglado se guarda como `semantic_fact` (o `architectural_decision` si fija una regla) y que `tool_error` es una falla de herramienta sin resolver.

## `rag-eval`

No aplica: el cambio no toca ranking, recuperación ni contexto. El efecto de los saves escritos según el protocolo se va a leer en el corte por pareja de idiomas cuando exista un golden con filas nuevas.

## Al archivar

- Las líneas que citan `spec.md` y `design.md` son las de antes de aplicar. Las specs principales citan las de `b302fd7`.
- Se volvieron a revisar las citas de los archivos que tocó el cambio (`daemon/api.rs`, `mcp/server.rs`, `admin.rs`, `prompt_builder.rs`, `store_insert.rs`, `agent_view/parse.rs` y `.claude/CLAUDE.md`) en todas las specs principales. Las corridas se movieron a HEAD en `mcp`, `update`, `quality`, `retrieval`, `pieces`, `project`, `project-docs`, `sharing` y `storage`. `quality` además dejó de citar `INSTRUCTIONS`, que ya no existe: las instrucciones son `opening(PROTOCOL)`.
- `specs/mcp` (R3) y `specs/update` (R5) siguen contando once herramientas sin `restore_memory`, que trajo recoverable-delete (18), todavía sin archivar. No es de este cambio: lo corrige el archivo de 18.

## Veredicto

Verificado en Linux, en Windows y con Claude Code sobre el perfil dev. Se archivó el 2026-10-17 en [`specs/mcp`](../../../specs/mcp/spec.md) (protocolo, instrucciones, `enum` y descripción de `save_memory`), [`specs/ingest`](../../../specs/ingest/spec.md) (el save rechaza un tipo desconocido) y [`specs/retrieval`](../../../specs/retrieval/spec.md) (la nota de conflictos en inglés), con ajustes en [`specs/update`](../../../specs/update/spec.md) y [`specs/quality`](../../../specs/quality/spec.md).
