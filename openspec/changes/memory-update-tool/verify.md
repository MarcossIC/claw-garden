# memory-update-tool — Verificación

**Estado:** tests en verde en Linux, commit por commit. Faltan la suite en Windows, `rag-eval --real --diff` y la prueba con el perfil dev en Claude Code ([pendiente](#pendiente-máquina-de-marcos)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama.

| Commit | Pasan | Fallan |
|---|---|---|
| Base (`main`, 35e448c) | 1.639 | 10 |
| `docs(openspec): propose correcting a memory's importance and type` (36250fd) | 1.639 | 10 |
| `feat(core): let a memory's importance and type be corrected` (e1d3b70) | 1.654 | 10 |
| `feat(daemon): accept a memory type in the update endpoint` (5beee5e) | 1.656 | 10 |
| `feat(mcp): let the agent correct a memory's importance and type` (6e9e826) | 1.659 | 10 |
| `docs: list the update_memory tool` (475ec12) | 1.659 | 10 |

Todos los commits compilan, y las 10 fallas son las mismas en todos y vienen del entorno, como en los cambios anteriores:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

**Conteo:** 20 tests nuevos. En `tier.rs`, 8 de `retier`; en `admin.rs`, 4 de la validación y 3 del cambio (`an_update_writes_what_was_asked_and_counts_itself`, `an_update_leaves_status_accesses_and_disputes_alone`, `rows_from_the_previous_version_can_be_updated`); 2 HTTP, 2 del puente y 1 de `tools.rs`. Ningún test se borró: los 6 de `set_importance_core` siguen, renombrados, sobre `update_memory_core`. El que fijaba la regla vieja (`set_importance_lowering_never_demotes_tier_1`) ahora fija la nueva (`lowering_importance_takes_away_the_tier_it_gave`).

Los warnings de compilación no cambian: ninguno es nuevo.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1 validación | 4 en `admin.rs`; `an_invalid_patch_changes_nothing` (HTTP); el caso de importancia 11 en `update_memory_changes_an_own_memory_and_says_how_it_ended` (MCP) |
| R2 tier | 8 en `tier.rs`; `lowering_importance_takes_away_the_tier_it_gave` con base |
| R3 escritura | `an_update_writes_what_was_asked_and_counts_itself`, `an_update_leaves_status_accesses_and_disputes_alone`, los de fila inexistente, borrada y ajena |
| R4 HTTP | `patch_changes_importance_and_type_and_reports_the_row`, `an_invalid_patch_changes_nothing`, `patch_with_another_agent_is_404_and_keeps_the_importance` |
| R5 MCP | `update_memory_changes_an_own_memory_and_says_how_it_ended`, `an_agent_cannot_update_another_agents_memory`, `format_update_names_the_fields_and_the_tier`, el refuerzo en `save_outcome_text_covers_every_daemon_answer`, los conteos de herramientas |
| R6 datos anteriores | `rows_from_the_previous_version_can_be_updated` |

## rag-eval

No corrido en Linux: necesita el modelo real. El cambio no toca la búsqueda, el puntaje ni el prompt, y el golden set no usa `PATCH`, así que se espera diff vacío.

## Pendiente (máquina de Marcos)

1. `cargo test --workspace --target-dir <otro-dir>` en Windows. Se espera 1.675 (la base de Windows, 1.655, más los 20 nuevos), sin fallas.
2. `rag-eval --real --diff` contra el baseline: diff vacío, ninguna métrica baja.
3. Con el perfil dev en Claude Code (reiniciá el daemon dev para que tome el binario nuevo):
   - guardá una memoria con importancia 9,5 y confirmá con `list_memories` que está en `t1` y que `recall_context` la muestra en el workspace;
   - pedile al agente que la baje a 2 con `update_memory`: la respuesta dice `tier 3` y la memoria sale del workspace;
   - cambiale el tipo a `user_preference`: vuelve a `t1`;
   - re-guardá el mismo texto: la respuesta nombra `update_memory`;
   - con otro agente (`--agent otro`), intentá cambiar ese id: falla como si no existiera.
