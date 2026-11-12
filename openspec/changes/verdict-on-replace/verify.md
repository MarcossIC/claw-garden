# verdict-on-replace — Verificación

## Windows (2026-11-11)

En Windows x86_64, build debug, toolchain 1.99.1, en `target-test` después de `cargo clean --workspace --target-dir target-test`, sobre `2bf6b66` (el último commit de código es `1d09862`):

- `cargo test --workspace` pasa entero.
- `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings` salen con 0, sin ningún warning.

Cada requisito tiene sus tests:

| Requisito | Tests |
|---|---|
| R1 | `save_outcome_text_names_related_memories` (`server.rs`), en inglés y en español |
| R2 | `format_restore_names_key_replacements_and_pairs` (`tools.rs`), en inglés y en español |
| R3 | `the_instructions_carry_the_saving_habits`, `the_spanish_instructions_carry_the_saving_habits` y `the_instructions_fit_older_clients` (`server.rs`) |
| R4 | `memory_status_counts_only_the_pairs_that_wait_for_the_user` y `memory_status_counts_every_project_with_several_folders` (`server.rs`); `pending_pairs_are_not_put_to_the_agent` y `format_status_ends_with_the_open_pairs` (`tools.rs`) |
| R5 | `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` (`server.rs`) |
| R6 | sin cambios: `a_confirmed_save_names_its_candidates` (`worker.rs`), `compatible_and_scoped_close_without_touching_the_ranking` y `open_pairs_carry_each_sides_mark` (`dispute.rs`), `the_conflict_note_names_the_tool` (`prompt_builder.rs`) |

Los tests de R1 a R4 se vieron fallar por su aserción antes del cambio. R5 no tiene un test anterior al texto: el contrato fijado compara las descripciones del servidor con sus dos archivos, que se regeneraron y cuyo diff sólo trae la apertura, las descripciones de `resolve_conflict` y `restore_memory`, y la de `keep`.

Lo que aplicar el cambio movió respecto de la spec inicial, ya anotado en ella:

- La apertura en inglés quedaba en 2.051 caracteres con la frase nueva. Cede la palabra `own` de otra frase y entra en los 2.048.
- El puente lee `GET /v1/disputes` con el tipo que el daemon serializa, así que no se le quitaron campos: imprime sólo `escalated`.
- La descripción de `keep` dejó de nombrar `memory_status`, que ya no muestra ids.
- `memory_status_counts_every_project_with_several_folders` fijaba «1 for you to judge» sobre un par pendiente; ahora usa un par escalado y sigue probando el alcance por proyecto.

Nada cambió en `crates/clawg-core` ni en `crates/clawg-cli/src/daemon`.

## Pendiente en el CI

El CI en Linux y Windows, que corre con el pull request.

## Pendiente en prod

Con la versión instalada:

1. Correr `clawg setup` otra vez en cada cliente, para que el bloque del protocolo del archivo de instrucciones tome la frase nueva. Hasta entonces el bloque viejo sigue pidiendo juzgar cada cercana, aunque la respuesta del save ya no lo pida.
2. Con los transcripts de algunas semanas de uso, repetir la pasada de la [exploración](exploration.md#los-transcripts-de-claude-code-de-este-repositorio):
   - los pedidos que sólo juzgan por cada save con cercanas, con el umbral fijado antes de mirar: menos de 0,15 (antes 0,79);
   - los `supersedes` por cada cien saves con cercanas (antes 3,3), que se informan y no deciden.
