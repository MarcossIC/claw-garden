# context-density — Verificación

**Código:** rama `claude/project-thread-hsnti2`, un commit por tarea, sobre `main` en `e759d18`; mergeada en `main` (#20, `8432d57`). El fix que encontró la verificación es `d175348`, directo en `main`.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`). Los tests de la tarea 1 se vieron fallar contra el código de `main` (el módulo de tests nuevo copiado sobre `e759d18`); los de las tareas 2, 3 y 5 fallaron antes de su código por no compilar o por la aserción, los de la 4 por la aserción, y el del fix (tarea 6) por la aserción, con el mismo texto doble que mostró el perfil dev.
**Estado:** verificado. R1 a R10 pasan en Linux, commit por commit, y en Windows. R11 cumple los cinco umbrales con el modelo real. En el perfil dev, el prompt sale con el formato nuevo y los ids del bloque sirven en `resolve_conflict`, `update_memory` y `delete_memory`. La prueba encontró una violación de R3 en las cadenas de disputas, que se arregló en `d175348` ([abajo](#lo-que-encontró-la-verificación)).

## Suite

### Linux

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

### Windows (2026-10-14)

`cargo test --workspace --no-fail-fast --target-dir target-test`:

| Commit | Pasan | Fallan |
|---|---|---|
| `9c3ff19` (`main` con el merge de este cambio y el archivo de memory-update-tool) | 1.690 | 0 |
| `d175348` fix(core): print a conflict row's text once when it sits in several pairs | 1.691 | 0 |

- 1.690 es la base de Windows (1.675) más los 15 de este cambio, y 1.691 suma el del fix. Los 23 tests que nombra la spec, los renombrados incluidos, aparecen uno por uno como `ok`.
- Las 10 fallas de Linux no aparecen: las 7 de `doctor::tests` modelan un PATH de Windows, y las 3 de `diff::tests` encuentran `docs/eval/results/` en esta máquina.
- El único warning de `cargo test` es el de `insert_row_with_embedding`, el de `main`. `cargo build -p clawg-cli` da además `spanning` y `search_all_memories_with_scoring` sin uso fuera del eval; vienen de `57119d3` y este cambio no toca esos archivos.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Línea de hit | `hit_line_is_one_header_with_id_and_text`, `format_hit_line_drops_the_old_labels` | ✓ |
| R2. Referencia por id | `conflict_block_cites_shown_hits_by_id_and_prints_text_only_for_the_rest`, `a_row_in_several_pairs_prints_its_text_once`, `e2e_full_4_block_output` (la fila mostrada aparece una vez y el bloque la cita como `OLD #id`) | ✓ (con el fix) |
| R3. Agrupación | `a_new_row_disputing_several_old_rows_prints_its_text_once`, `a_row_in_several_pairs_prints_its_text_once` | ✓ (con el fix) |
| R4. NOTE una vez | `conflict_block_has_one_note`, `the_conflict_note_names_the_tool` | ✓ |
| R5. Orden determinista | `conflict_pairs_rank_by_best_hit_position_workspace_only_last`, `conflict_pair_order_ignores_input_order` | ✓ |
| R6. Tope de 5 | `conflict_block_keeps_five_pairs_and_counts_the_rest` | ✓ |
| R7. Workspace | `workspace_in_prompt_has_no_markers_or_empty_headings`, `workspace_user_section_content_is_kept_without_markers`, `workspace_file_bytes_are_unchanged_by_rendering_the_prompt`; el archivo plano sigue en `embedder_none_emits_workspace_only_if_present` | ✓ |
| R8. Parser del eval | `a_hit_line_is_one_segment`, `a_hit_of_another_kind_is_one_segment`, `a_multi_line_hit_runs_until_the_next_header`, `a_hit_ends_at_the_next_section_separator`, `a_hit_may_open_with_a_dash`, `conflict_text_yields_no_segments`, `workspace_markers_end_a_bullet`, `read_context_finds_rows_in_hit_lines_and_workspace_bullets`; el punta a punta del agent-view (`runner.rs`) sigue encontrando la fila en `recall_context` | ✓ |
| R9. Doc de `delete_memory` y `update_memory` | `memory_id_docs_name_recall_context_and_the_default_kind`; en el perfil dev, `tools/list` da la doc nueva | ✓ |
| R10. Sin cambios | la suite de `tools.rs` y `server.rs` (`no context yet`, el aviso de `synthesize`, `search_memory`), `all_empty_returns_empty_string`, `separator_strings_match_exactly` y los tests de `fetch_active_conflicts`; `search_memory` da los mismos tokens en los 46 casos de `rag-eval` | ✓ |
| R11. Medición | `rag-eval --real --agent-view` ([abajo](#r11-rag-eval)) | ✓ |

## R11 (rag-eval)

**Entorno:** Windows 11, binario debug de `rag-eval` compilado por la suite en `target-test`, modelo real (multilingual-e5-small) leído del caché de `~/.clawg-dev/models` (`--check-only` lo confirmó antes de correr, sin descargar ni escribir), con autorización del usuario. Golden set `docs/eval/golden.json` (sha256 `d2d9d732…ed792a`, n = 46). Base: `docs/eval/results/2026-10-13-real-lexical-fusion-main-agent-view.json` (`d28fe90`).

**Corridas:** `rag-eval --real --agent-view --golden docs/eval/golden.json` y `rag-eval --real --golden docs/eval/golden.json` (ancla), con `--tag context-density-main` sobre `9c3ff19` y `--tag context-density-fix` sobre `d175348`. Los resultados quedan en `docs/eval/results/` (fuera del repo) como base de los próximos cambios: `2026-10-15-real-context-density-fix-agent-view.json` y `-anchor.json` (la fecha del archivo es UTC).

| Métrica | Base (`d28fe90`) | Merge (`9c3ff19`) | Fix (`d175348`) | Umbral |
|---|---|---|---|---|
| `found` | 41/46 | 41/46 | 41/46 | 41/46 |
| `search_memory`, media / máximo | 851,5 / 1.239 | 851,5 / 1.239 | 851,5 / 1.239 | sin cambios |
| ACTIVE CONFLICTS, media cuando aparece (45 casos) | 2.273,8 | 407,7 | 403,7 | ≤ 800 |
| `recall_context`, media / máximo | 3.355,0 / 4.900 | 1.408,9 / 1.738 | 1.405,0 / 1.738 | ≤ 2.100 / ≤ 3.000 |
| RECENT CONTEXT, media | 1.129,6 | 1.009,0 | 1.009,0 | ≤ 1.130 |

- **El contexto cuesta 58 % menos** (media) y el máximo baja de 4.900 a 1.738. ACTIVE CONFLICTS baja 82 %. RECENT baja 11 %: cada hit pierde `SOURCE: Memory | TYPE:` y `CONTENT:`, y gana `#id`.
- **Nada de lo que ve el agente se pierde:** en los 46 casos, `found`, las referencias encontradas en `recall_context`, el rango y los tokens de `search_memory` son los de la base.
- **El fix cambia 2 de los 46 casos** (89 y 91 tokens menos en cada uno): la cadena de disputas que encontró el perfil dev también está en el golden set.
- `rag-eval --diff` base contra merge y base contra fix: 0 mejoran y 0 empeoran en todas las métricas, en retrieval y en similarity, en el ancla y en agent-view. El ancla del fix es idéntica a la de la base campo por campo, sin fecha, tag, sha ni latencias: R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG@10 0,6693.

## Perfil dev (2026-10-14)

**Entorno:** `target\debug\clawg.exe` compilado sobre `main`, contra el `~/.clawg-dev` real, con autorización del usuario. Antes de tocar la base: `clawg backup --profile dev` por el daemon viejo (`memory-2026-10-15T00-00-36.032Z-manual.db`) y `clawg stop --profile dev`.

**Por qué no fue con el cliente MCP de Claude Code:** el daemon dev que corría era del binario de memory-update-tool (compilado antes del merge de este cambio), y los dos puentes `clawg-dev` abiertos tenían tomado `target\debug\clawg.exe` y no se pueden matar. Se renombró el ejecutable en uso y se compiló el nuevo en su lugar. Un puente ya abierto guarda la URL y el token del daemon al arrancar, y el daemon nuevo genera otro token, así que los pasos se corrieron con un `clawg mcp --profile dev --agent claude-code` nuevo, manejado por stdio con JSON-RPC: es el mismo protocolo y el mismo agente que usa Claude Code. El handshake negocia `2025-06-18`, y `tools/list` da la doc nueva de `delete_memory` y `update_memory` (R9). Claude Code va a ver el formato nuevo cuando reinicie sus puentes. Las filas de prueba se guardaron en un proyecto aparte (un directorio temporal), rotuladas como fixture, y se borraron al final.

**Fixtures:** #36 "el store … guarda los datos en sqlite" (`semantic_fact`), #37 "el daemon … escucha solamente en loopback" (`architectural_decision`, 9,5, para que entre al workspace) y #38 "el store … guarda los datos en postgres". El detector marcó tres pares: 37 contra 36, 38 contra 36 y 38 contra 37. El último es un falso positivo del detector por proximidad: las tres fixtures comparten un prefijo largo.

Con el binario de `9c3ff19` (antes del fix), `recall_context` dio:

```
=== WORKSPACE ===
## Architectural Decisions
- Fixture …: el daemon del proyecto de prueba escucha solamente en loopback.

=== RECENT CONTEXT (Last 7 Days) ===
[2026-10-15 #36 semantic_fact] Fixture …: el store del proyecto de prueba guarda los datos en sqlite.
[2026-10-15 #38 semantic_fact] Fixture …: el store del proyecto de prueba guarda los datos en postgres.

=== ACTIVE CONFLICTS (review needed) ===
NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
Con su respuesta, llamá a resolve_conflict (keep = la que vale, other = la otra).
NEW #38
OLD #36
OLD [2026-10-15 #37] Fixture …: el daemon del proyecto de prueba escucha solamente en loopback.
NEW [2026-10-15 #37 disputed] Fixture …: el daemon del proyecto de prueba escucha solamente en loopback.
OLD #36
```

El texto de #37 sale dos veces en el bloque: viola R3 ([abajo](#lo-que-encontró-la-verificación)). Con el binario de `d175348` (daemon reiniciado igual, con `clawg stop`), las dos últimas líneas de grupo pasan a `NEW #37` y `OLD #36`; el resto del prompt es el mismo.

| Paso (binario de `d175348`) | Resultado |
|---|---|
| `recall_context` | Hits en una línea con `#id` (R1); workspace sin `<!--` y sólo con el encabezado que tiene bullet (R7); una NOTE (R4); los hits citados por `#id` y el texto de #37 una sola vez en el bloque (R2, R3); grupos en el orden de R5 |
| `resolve_conflict` con `keep = 38`, `other = 36`, los ids tal como los imprime el bloque | `#38 disputed` / `#36 superseded by #38` (38 sigue en disputa con 37) |
| `resolve_conflict` con `keep = 38`, `other = 37`, `keep_both = true` (el falso positivo) | `#38 active` / `#37 active` |
| `recall_context` otra vez | #36 ya no aparece, y sin conflictos no hay bloque ACTIVE CONFLICTS |
| `update_memory` con `id = 38` (un hit de `recall_context`) y `kind = memory` | `updated #38 memory: semantic_fact, importance 7.0, tier 3` |
| `delete_memory` de 36, 37 y 38 con `kind = memory` | `deleted 1` en cada una; `list_memories` ya no las muestra |

## Lo que encontró la verificación

- **El texto de una fila salía dos veces en una cadena de disputas.** R3 dice que el texto de la nueva aparece a lo sumo una vez en el bloque. Pero el bloque decidía entre `#id` y texto mirando sólo si la fila era un hit, sin recordar lo que ya había impreso. En la cadena 36 ← 37 ← 38, con 37 fuera de los hits, 37 salía con texto como `OLD` del grupo de 38 y otra vez como `NEW` de su grupo. Lo mismo pasaba con una vieja que dos grupos disputan; ese caso no violaba R2 tal como estaba escrita, pero era el mismo desperdicio. Los tests no lo veían: ninguno tenía una fila en dos pares. **Fix** (`d175348`, TDD): `a_row_in_several_pairs_prints_its_text_once` primero en rojo, después `format_conflict_block` cita por `#id` toda mención después de la primera. R2 lo dice así desde la verificación, con el escenario de la cadena. En el golden set el fix cambia 2 de 46 casos.
- **El texto de una fila del workspace sigue saliendo dos veces en el prompt:** en su bullet y en el bloque. Lo pide R2, porque el bullet no tiene id al que apuntar. Darle id al bullet no entra en este cambio: el formato del archivo del workspace no cambia.

## Decisiones tomadas al implementar

- **La poda de encabezados sólo mira la sección AUTO.** La sección USER es texto del usuario y se entrega como la escribió, encabezados incluidos. La spec lo dice así desde la revisión del 2026-10-14.
- **Un archivo con AUTO pero sin `AUTO-END`** se toma como AUTO hasta el final. Antes se entregaba entero si tenía algún bullet.
- **El texto antes de `AUTO-START`** (si el usuario escribió algo ahí) se entrega. Antes se perdía cuando no había bullets ni texto en USER.
