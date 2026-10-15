# agent-sharing — Verificación

**Estado:** tests en verde en Linux, commit por commit. Faltan la suite en Windows, `rag-eval --real --diff` y la prueba con dos agentes en el perfil dev (ver [Pendiente](#pendiente-máquina-de-marcos)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama, cada uno en un worktree limpio.

| Commit | Pasan | Fallan |
|---|---|---|
| Base (`main`, 9c3ff19) | 1.674 | 10 |
| `docs(openspec): propose sharing memories between agents on the user's grant` (af3d016) | 1.674 | 10 |
| `refactor(core): drop the unused Scope enum` (46a9e89) | 1.674 | 10 |
| `feat(core): read and write the sharing policy` (a80a4d2) | 1.688 | 10 |
| `feat(core): let a search reach the memories of granted agents` (6077ea9) | 1.696 | 10 |
| `feat(core): name the author of a shared memory in the context` (f1aa217) | 1.700 | 10 |
| `feat(daemon): apply the sharing policy to search and context` (ab69db9) | 1.704 | 10 |
| `feat(mcp): mark shared memories and say whom the agent reads` (04f1eda) | 1.713 | 10 |
| `feat(cli): manage the sharing policy with clawg sharing` (eda95cc) | 1.718 | 10 |
| `docs: describe sharing memories between agents` (fd29778) | 1.718 | 10 |
| `docs(openspec): align the agent-sharing spec with the tests that cover it` (e4ad917) | 1.718 | 10 |
| `feat!: share every agent's memories unless the user hides them` (1226774) | 1.722 | 10 |
| `docs: describe sharing by default and how to hide an agent` (8de8dc6) | 1.722 | 10 |
| `fix(cli): honour a --profile given before the sharing action` (9556225) | 1.723 | 10 |
| `docs: say a renamed agent still finds its old memories but no longer owns them` (3852e14) | 1.723 | 10 |

Todos los commits compilan, y las 10 fallas son las mismas en todos y vienen del entorno, como en los cambios anteriores:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

**Conteo:** 49 tests nuevos sobre la base. En el commit de "todo compartido" se reescribieron los tests de la política, de la búsqueda, del daemon, del puente y de la CLI para la semántica final, y salió `the_eligible_count_still_rides_the_agent_index`: con el predicado de exclusiones el conteo recorre el índice parcial en lugar de buscar por agente, y eso es lo esperado (design D3).

El único warning es el de `insert_row_with_embedding`, el mismo de `main`.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1 política | 9 en `memory/sharing.rs` (incluye `a_missing_policy_shares_everything` y `every_agent_stands_for_either_side`); `search_and_context_follow_the_sharing_policy_from_the_next_call` y `an_unreadable_policy_shares_nothing_over_http` (HTTP) |
| R2 búsqueda | 7 en `search.rs` (dos canales, oculto, dirección, lo propio nunca se oculta, proyecto, conteo de elegibles, sin cambios con sólo lo propio); `contradiction_probe_ignores_shared_authors` |
| R3 autor | `a_hit_carries_its_author`, `search_returns_the_author_of_each_hit`, `a_hit_from_an_old_daemon_reads_as_own` |
| R4 formato | 3 en `prompt_builder.rs`, 2 de `format_search`, `search_memory_shows_another_agents_memory_with_its_author`, `recall_context_shows_another_agents_memory_with_its_author` |
| R5 lo propio | `a_shared_memory_is_read_only`, `tool_descriptions_say_shared_memories_are_read_only`, `the_workspace_and_conflicts_stay_own` |
| R6 `memory_status` | `format_status_says_whom_the_agent_reads`, `memory_status_reports_the_sharing_of_the_agent` |
| R7 CLI | 4 en `cli.rs` (incluye `sharing_takes_the_profile_from_either_side_of_the_action`), 4 en `clawg-cli/src/sharing.rs`, 6 de `hide`/`unhide`/`save` en `memory/sharing.rs` |
| R8 datos anteriores | `rows_of_the_previous_version_are_shared_untouched` |
| R9 `enum Scope` | compila y pasa la suite sin él |

## rag-eval

No corrido en Linux: necesita el modelo real. Se espera diff vacío en el ancla y en agent-view. rag-eval busca con `SearchScope::all_default()`, que queda en *sólo lo propio*, y el golden set es de un solo agente, así que la consulta trae las mismas filas.

## Pendiente (máquina de Marcos)

1. `cargo test --workspace --no-fail-fast --target-dir target-test` en Windows. Lo esperado: 1.675 + 49 = **1.724 pasan, 0 fallan**.
2. `rag-eval --real` con el binario de `main` y con el de esta rama (ancla y `--agent-view`), y `rag-eval --diff` entre los dos. Lo esperado: 0 mejoran, 0 empeoran y el bloque `agent_view` igual campo por campo.
3. Perfil dev, con autorización y un `clawg backup --profile dev` antes, en un proyecto aparte. El backup no incluye `sharing.json`: si ya existe en `~/.clawg-dev`, copialo aparte y sacalo para la prueba.
   - `clawg mcp --profile dev --agent a` guarda una memoria rotulada como fixture.
   - Con `--agent b`, sin `sharing.json`: `search_memory` y `recall_context` la traen con `by a` y la nota al final; `memory_status` dice `shared: reads every agent`.
   - `b` no la puede borrar ni cambiar (`delete_memory` y `update_memory` dan error).
   - `clawg sharing hide b a --profile dev`, sin reiniciar el daemon: `b` deja de verla y `memory_status` dice `shared: reads every agent but a`. `a` sigue viendo lo de `b`.
   - `clawg sharing unhide b a --profile dev`: `b` la vuelve a ver.
   - Borrar las filas de prueba con `--agent a`. Si había un `sharing.json` antes, devolver la copia; si no, borrar el que dejó la prueba.
