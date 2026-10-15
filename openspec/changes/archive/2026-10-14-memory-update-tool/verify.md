# memory-update-tool — Verificación

**Estado:** verificado. Tests en verde en Linux (commit por commit) y en Windows; `rag-eval --real` da diff vacío en el ancla y en agent-view; en el perfil dev, bajar la importancia saca una decisión del workspace por el tier, y otro agente no puede tocarla (ver [Windows](#windows-2026-10-14), [rag-eval](#rag-eval) y [Perfil dev](#perfil-dev-2026-10-14)).

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

### Windows (2026-10-14)

`cargo test --workspace --no-fail-fast --target-dir target-test` sobre `main` en `e759d18` (el merge de este cambio, #19): **1.675 pasan, 0 fallan**. Es la base de Windows (1.655) más los 20 nuevos.

- Los 20 tests de este cambio aparecen uno por uno como `ok`: los 8 de `retier` en `tier.rs`, los 7 de `admin.rs`, los 2 HTTP, los 2 del puente y el de `tools.rs`. También pasan `patch_with_another_agent_is_404_and_keeps_the_importance` y `save_outcome_text_covers_every_daemon_answer`, con el refuerzo nuevo.
- Las 10 fallas de Linux no aparecen: las 7 de `doctor::tests` modelan un PATH de Windows, y las 3 de `diff::tests` encuentran `docs/eval/results/` en esta máquina.
- El único warning es el de `insert_row_with_embedding`, el mismo de `main` (está en el backlog del README).

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

**Entorno:** Windows 11, binarios debug de `rag-eval`, modelo real (multilingual-e5-small) leído del caché de `~/.clawg-dev/models` (`--check-only` lo confirmó con los dos binarios antes de correr, sin descargar ni escribir); los resultados van a un directorio temporal, fuera de `docs/eval/results/`.

**Binarios**, cada uno compilado en su worktree:
- Base: `31b9341` (`main` justo antes del merge de este cambio).
- Cambio: `e759d18` (el merge de este cambio).

**Golden set:** `docs/eval/golden.json` (ancla, sha256 `d2d9d732…ed792a`, n = 46).

**Corridas**, las mismas con los dos binarios:
- `rag-eval --real --golden docs/eval/golden.json --tag anchor`
- `rag-eval --real --agent-view --golden docs/eval/golden.json`

| Métrica | Base | Cambio |
|---|---|---|
| R@1 | 0,4348 | 0,4348 |
| R@5 | 0,7826 | 0,7826 |
| R@10 | 0,8913 | 0,8913 |
| MRR | 0,5979 | 0,5979 |
| nDCG@10 | 0,6693 | 0,6693 |

- `rag-eval --diff` base contra cambio: 0 mejoran y 0 empeoran en todas las métricas, en retrieval y en similarity, en el ancla y en agent-view.
- `rag-eval --diff` no compara el bloque `agent_view`, así que los dos JSON se compararon campo por campo, sin fecha, tag, sha ni latencias: **0 diferencias**, en el ancla y en agent-view.
- Agent-view, en las dos: 48 filas guardadas para un corpus de 48; `search` con hit rate 0,8913 y tokens media 851,5 y máximo 1239; `context` con hit rate 0,8913 y tokens media 3355,0 y máximo 4900. Son los mismos números que midió secret-hygiene.

## Perfil dev (2026-10-14)

**Entorno:** `target\debug\clawg.exe` compilado sobre `main` en `e759d18`, contra el `~/.clawg-dev` real, con autorización del usuario. Antes de tocar la base: `clawg backup --profile dev` por el daemon viejo (`memory-2026-10-14T22-55-50.803Z-manual.db`) y `clawg stop --profile dev`. El daemon nuevo (pid 28684) lo levantó el autostart del primer puente.

**Por qué no fue con el cliente MCP de Claude Code:** el puente `clawg-dev` de la sesión tenía tomado `target\debug\clawg.exe` (un binario del 2026-10-12, sin `update_memory`) y no se puede matar. Se renombró el ejecutable en uso, se compiló el nuevo en su lugar y los pasos se corrieron con un `clawg mcp --profile dev --agent claude-code` nuevo, manejado por stdio con JSON-RPC: es el mismo protocolo y el mismo agente que usa Claude Code. El handshake negocia `2025-06-18`, y `tools/list` da 9 herramientas con la descripción y el esquema de `update_memory` del diseño. Claude Code va a ver la herramienta cuando reinicie el puente. Las filas de prueba se guardaron en un proyecto aparte (un directorio temporal), rotuladas como fixture, y se borraron al final.

| Paso | Resultado |
|---|---|
| `save_memory` de una `architectural_decision` con importancia 9,5 | `t1`; `recall_context` la muestra bajo `=== WORKSPACE ===`, en "Architectural Decisions" |
| `update_memory` con importancia 8,5 | `updated #35 memory: architectural_decision, importance 8.5, tier 2`; `recall_context` ya no la muestra en el workspace sino en `RECENT CONTEXT`. Con 8,5 sigue pasando el filtro de importancia del workspace (≥ 8), así que sale sólo por el tier (R2); con la regla vieja se quedaba en `t1` |
| `update_memory` con tipo `user_preference` | `tier 1`; vuelve al workspace, bajo "User Preferences" |
| `save_memory` de un texto ya guardado | `already stored as #34; reinforced it instead of saving a copy (update_memory changes its importance or type)` (R5) |
| `update_memory` de #34 desde `--agent otro` | `daemon returned 404: no Memory row with id 34`, igual que un id que no existe (`999999`); `list_memories` de `otro` no la ve y la fila sigue con su importancia y su tipo (R3, R4) |
| `semantic_fact` con 9,5 bajada a 2 | `updated #34 memory: semantic_fact, importance 2.0, tier 3`, y `list_memories` da `t3` |
| `delete_memory` de las dos filas | `deleted 1` en cada una |

## Lo que encontró la verificación

- **El paso del workspace estaba mal planteado.** Decía "guardá una memoria con importancia 9,5 y bajala a 2". Pero el AUTO del workspace sólo toma `user_preference` y `architectural_decision` con importancia ≥ 8 **y** tier 1 (`workspace.rs:255-292`). Un `semantic_fact`, que es el tipo por defecto, no entra nunca, y una memoria bajada a 2 sale por la importancia con la regla vieja o con la nueva. La prueba que aísla R2 es la de la tabla: una decisión de 9,5 bajada a 8,5. La proposal dice lo mismo para "bajarla a 2"; la regla vale, pero lo que se ve en el workspace es la importancia y no el tier.
