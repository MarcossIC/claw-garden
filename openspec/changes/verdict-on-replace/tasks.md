# verdict-on-replace — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia.

## 1. La respuesta del save y la de restore (R1, R2)

- [ ] Tests primero: `save_outcome_text_names_related_memories` (`server.rs`) y `format_restore_names_key_replacements_and_pairs` (`tools.rs`) con la línea nueva, en inglés y en español donde el test ya cubre los dos.
- [ ] `related_text` (`server.rs`) y la línea de las cercanas de un restore (`tools.rs`).

## 2. El protocolo (R3)

- [ ] Tests primero: `the_instructions_carry_the_saving_habits` y `the_spanish_instructions_carry_the_saving_habits` (`server.rs`) con la frase nueva.
- [ ] `protocol.md` y `protocol.es.md`. Regenerar `agent_contract.json` y `agent_contract.es.json` y revisar su diff: sólo cambia la apertura.

## 3. `memory_status` (R4)

- [ ] Tests primero: `memory_status_counts_only_the_pairs_that_wait_for_the_user` (`server.rs`), `pending_pairs_are_not_put_to_the_agent` y `format_status_ends_with_the_open_pairs` (`tools.rs`).
- [ ] `format_open_pairs` (`tools.rs`) y lo que el cliente lee de `GET /v1/disputes` (`client.rs`), sin dejar código sin uso.

## 4. Las descripciones (R5)

- [ ] Las descripciones de `resolve_conflict` y `restore_memory` en `server.rs` y `tools_es.rs`. Regenerar los dos archivos del contrato y revisar su diff: sólo cambian esas dos descripciones.

## 5. Verificación

- [ ] `cargo clean --workspace --target-dir target-test`, y después la suite y clippy de `.claude/CLAUDE.md` en Windows (`target-test`), una vez, sobre el último commit.
- [ ] El CI en verde en Linux y Windows.
- [ ] Pendiente en local, con uso real: correr `clawg setup` otra vez para que el bloque del protocolo del usuario tome la frase nueva, y con los transcripts de algunas semanas medir los pedidos que sólo juzgan por save con cercanas (umbral fijado: menos de 0,15; hoy 0,79) y los `supersedes` por cada cien saves con cercanas (hoy 3,3; se informa).

## 6. Al archivar

- [ ] Consolidar R1 a R6 en `specs/disputes`, `specs/mcp` y `specs/provenance`, como dice la [spec](spec.md#specs-que-se-consolidan-al-archivar), y corregir en `docs/reference/` lo que describa estos textos.
