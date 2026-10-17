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

`cargo build -p clawg-cli`, sin la feature `eval` de `clawg-core`, da un warning ya presente en `af6fef4`: `ScoringConfig::spanning` (`crates/clawg-core/src/memory/score.rs:88`) sin uso. `--workspace` no lo muestra porque `clawg-eval` activa la feature. No es de este cambio y queda para un arreglo suelto.

## Pendiente

- **Windows:** `cargo test --workspace --target-dir target-test` y el `clippy` en `target-test`. Lo que más interesa es `opening` sobre un checkout con `\r\n`, que el test puro cubre pero no sobre el archivo real.
- **Perfil dev con Claude Code (necesita autorización):** registrar un `clawg mcp` de debug, comprobar que las instrucciones llegan enteras, guardar una decisión y un bug como pide el protocolo, y hacer un save con un tipo inventado: tiene que volver `not saved: unknown memory_type …` con los siete valores, y el agente tiene que reintentarlo con uno válido.
- **`rag-eval`:** no aplica a este cambio, que no toca ranking, recuperación ni contexto. El efecto de los saves escritos según el protocolo se va a leer en el corte por pareja cuando exista un golden de filas nuevas.
