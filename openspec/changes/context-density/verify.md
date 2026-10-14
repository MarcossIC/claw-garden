# context-density — Verificación

**Código:** rama `claude/project-thread-hsnti2`, un commit por tarea, sobre `main` en `e759d18`.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`). Los tests de la tarea 1 se vieron fallar contra el código de `main` (el módulo de tests nuevo copiado sobre `e759d18`); los de las tareas 2, 3 y 5 fallaron antes de su código por no compilar o por la aserción, y los de la 4 por la aserción.
**Estado:** R1 a R10 pasan en Linux, commit por commit. Faltan la suite en Windows, la medición de R11 y la prueba con el perfil dev, que necesitan la máquina de Marcos ([pendiente](#pendiente-máquina-de-marcos)).

## Suite

`cargo test --workspace --no-fail-fast` en Linux, cada commit en un worktree limpio con su propio directorio de build:

| Commit | Pasan | Fallan |
|---|---|---|
| `e759d18` (base, `main`) | 1659 | 10 |
| `dfaa1a5` docs(openspec): revise context-density against main and plan its tasks | 1659 | 10 |
| `4ffc8d0` feat(core): print each recall hit on one line with its id | 1664 | 10 |
| `f92142d` feat(core): cap the conflicts block and order it by what the prompt shows first | 1667 | 10 |
| `88af38f` feat(core): cite conflict rows the prompt already shows by id | 1670 | 10 |
| `2e970f1` feat(core): hand the model the workspace without its markers or empty headings | 1673 | 10 |
| `41a0873` docs(mcp): say that recall_context ids work in delete_memory and update_memory | 1674 | 10 |

- **+15 tests.** El conteo nunca baja: los tests que fijaban el formato viejo se reescribieron en el mismo commit que lo cambia.
- **Los 10 que fallan son los mismos 10 de `main`** en todos los commits (comparados por nombre): `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera).
- **Warnings y clippy:** el único warning del build es el de `main` (`insert_row_with_embedding`). `cargo clippy -p clawg-core -p clawg-cli -p clawg-eval --all-targets` no da avisos nuevos; el de `too_many_arguments` en `build_memory_prompt` ya estaba.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Línea de hit | `hit_line_is_one_header_with_id_and_text`, `format_hit_line_drops_the_old_labels` | ✓ |
| R2. Referencia por id | `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest`, `e2e_full_4_block_output` (la fila mostrada aparece una vez y el bloque la cita como `OLD #id`) | ✓ |
| R3. Agrupación | `a_new_row_disputing_several_old_rows_prints_its_text_once` | ✓ |
| R4. NOTE una vez | `conflict_block_has_one_note`, `the_conflict_note_names_the_tool` | ✓ |
| R5. Orden determinista | `conflict_pairs_rank_by_best_hit_position_workspace_only_last`, `conflict_pair_order_ignores_input_order` | ✓ |
| R6. Tope de 5 | `conflict_block_keeps_five_pairs_and_counts_the_rest` | ✓ |
| R7. Workspace | `workspace_in_prompt_has_no_markers_or_empty_headings`, `workspace_user_section_content_is_kept_without_markers`, `workspace_file_bytes_are_unchanged_by_rendering_the_prompt`; el archivo plano sigue en `embedder_none_emits_workspace_only_if_present` | ✓ |
| R8. Parser del eval | `a_hit_line_is_one_segment`, `a_hit_of_another_kind_is_one_segment`, `a_multi_line_hit_runs_until_the_next_header`, `a_hit_ends_at_the_next_section_separator`, `a_hit_may_open_with_a_dash`, `conflict_text_yields_no_segments`, `workspace_markers_end_a_bullet`, `read_context_finds_rows_in_hit_lines_and_workspace_bullets`; el punta a punta del agent-view (`runner.rs`) sigue encontrando la fila en `recall_context` | ✓ |
| R9. Doc de `delete_memory` y `update_memory` | `memory_id_docs_name_recall_context_and_the_default_kind` | ✓ |
| R10. Sin cambios | la suite de `tools.rs` y `server.rs` (`no context yet`, el aviso de `synthesize`, `search_memory`), `all_empty_returns_empty_string`, `separator_strings_match_exactly` y los tests de `fetch_active_conflicts` | ✓ |
| R11. Medición | — | pendiente ([abajo](#pendiente-máquina-de-marcos)) |

## Cómo queda el prompt

Un `recall_context` con una preferencia en el workspace, dos hits y una fila nueva que disputa al primero:

```
=== WORKSPACE ===
## User Preferences (always active)
- answer in spanish

=== RECENT CONTEXT (Last 7 Days) ===
[2026-10-14 #57 semantic_fact] usar sqlite para el store
[2026-10-13 #41 architectural_decision] el daemon escucha sólo en loopback

=== ACTIVE CONFLICTS (review needed) ===
NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
Con su respuesta, llamá a resolve_conflict (keep = la que vale, other = la otra).
NEW [2026-10-14 #63 disputed] usar postgres para el store
OLD #57
```

Antes, el mismo prompt llevaba los cuatro marcadores `<!-- … -->` y los dos encabezados vacíos del workspace, cada hit en dos líneas (`SOURCE:`/`TYPE:` y `CONTENT:`) sin id, y el par copiaba otra vez el texto de #57 con `importance` y `tier`.

## Decisiones tomadas al implementar

- **La poda de encabezados sólo mira la sección AUTO.** La sección USER es texto del usuario y se entrega como la escribió, encabezados incluidos. La spec lo dice así desde la revisión del 2026-10-14.
- **Un archivo con AUTO pero sin `AUTO-END`** se toma como AUTO hasta el final. Antes se entregaba entero si tenía algún bullet.
- **El texto antes de `AUTO-START`** (si el usuario escribió algo ahí) se entrega. Antes se perdía cuando no había bullets ni texto en USER.

## Pendiente (máquina de Marcos)

1. **Suite en Windows:** `cargo test --workspace --target-dir <otro-dir>`. Espero la base de Windows más los 15 tests nuevos (1.670 si la base sigue en 1.655) y 0 fallas.
2. **R11:** `rag-eval --real --agent-view --diff docs/eval/results/2026-10-13-real-lexical-fusion-main-agent-view.json`. Los umbrales no se movieron:

   | Métrica | Base | Umbral |
   |---|---|---|
   | `found` | 41/46 | 41/46 |
   | `search_memory`, media | 852 | sin cambios |
   | ACTIVE CONFLICTS, media cuando aparece | 2.274 | ≤ 800 |
   | `recall_context`, media / máximo | 3.355 / 4.900 | ≤ 2.100 / ≤ 3.000 |
   | RECENT CONTEXT, media | 1.130 | ≤ 1.130 |

   RECENT tendría que bajar un poco: cada hit pierde `SOURCE: Memory | TYPE:` y `CONTENT:`, y gana `#id`. Si algún umbral falla, el cambio no se da por bueno.
3. **Perfil dev con Claude Code:** un `recall_context` real muestra los hits con `#id` y el workspace sin marcadores; si hay un conflicto, `resolve_conflict` acepta los dos ids del bloque, y `delete_memory` o `update_memory` aceptan un id de un hit con `kind: memory`.

Cuando estén, se archiva con los pasos de `tasks.md`.
