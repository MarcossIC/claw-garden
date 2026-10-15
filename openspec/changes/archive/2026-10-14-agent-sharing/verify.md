# agent-sharing — Verificación

**Estado:** verificado. Tests en verde en Linux (commit por commit) y en Windows (`main` 9bbc78f, 1.863 pasan, 0 fallan); `rag-eval --real` con diff vacío en el ancla y en agent-view; la prueba con dos agentes en el perfil dev pasó entera. Archivado el 2026-10-14.

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

### Windows

`cargo test --workspace --no-fail-fast --target-dir target-test` sobre `main` 9bbc78f (el merge de agent-sharing, que ya trae project-indexing): **1.863 pasan, 0 fallan**, en 13 binarios de test. El único warning es el de `insert_row_with_embedding`.

El conteo cierra con la base de `.claude/CLAUDE.md`: 1.737 (long-save-context) + 77 de project-indexing (sale `chunker/code.rs` con 10, entran 87) + 49 de agent-sharing = 1.863.

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

A/B en Windows entre `01eec91` (`main` con project-indexing y sin agent-sharing) y `9bbc78f` (el mismo `main` con el merge de agent-sharing), binarios debug de `rag-eval`, el de la base en un worktree aparte. Golden set `docs/eval/golden.json` (sha256 `d2d9d732…`, 46 casos, 28 pares de contradicción, corpus de 48 filas), el mismo en las dos corridas.

| | R@1 | R@5 | R@10 | MRR | nDCG@10 |
|---|---|---|---|---|---|
| Base `01eec91` | 0,4348 | 0,6522 | 0,8478 | 0,5524 | 0,6215 |
| Cambio `9bbc78f` | 0,4348 | 0,6522 | 0,8478 | 0,5524 | 0,6215 |

- `rag-eval --diff` sobre el ancla: 0 mejoran, 0 empeoran, neto 0 en cada métrica. Sobre agent-view también 0/0/0.
- Agent-view comparado campo por campo (incluye `per_case` y `by_subset`, sin `date`, `tag`, `git_sha` ni latencias): ninguna diferencia. `search_memory` encuentra 30/46 (media 454,67 tokens, máximo 1.081) y `recall_context` 39/46 (media 1.086,39, máximo 1.408) en las dos.
- Lo único que cambia es la latencia: p50 de búsqueda 16,7 → 17,1 ms, de contexto 20,1 → 20,2 ms.
- Resultados en `docs/eval/results/2026-10-15-real-agent-sharing-{base,tip}.json` y `…-agent-view.json`.

Es lo esperado: rag-eval busca con `SearchScope::all_default()`, que queda en *sólo lo propio*, y el golden set es de un solo agente.

Las métricas del ancla ya no son las del baseline de `openspec/config.yaml` (R@5 0,7826, MRR 0,5979): la baja viene de project-indexing, que cambió el pooling de CLS a media, y es igual en la base y en el cambio. Le toca a la verificación de project-indexing, que tiene pendiente la calibración sobre el golden set y su propio `--diff`.

## Perfil dev

Con autorización de Marcos, el 2026-10-14, con el `clawg.exe` de `main` 9bbc78f copiado al scratchpad (para que el daemon no bloquee `target-test`) y con un proyecto aparte como directorio de trabajo. Los bridges se manejaron por JSON-RPC sobre stdio.

1. `clawg backup --profile dev` → `memory-2026-10-15T01-37-58.694Z-manual.db`. No había `sharing.json`. `clawg stop --profile dev` frenó el daemon del editor, que corría un binario anterior a project-indexing y a agent-sharing.
2. El primer bridge (`--agent a`) levantó el daemon nuevo: el store estaba en V11, así que dejó los snapshots `pre-migration` y `pre-reembed`, migró a V12 y re-embebió por media. Embedder `ready` a los ~15 s.
3. `a` guardó `#42` y `b` guardó `#43`, dos fixtures.
4. Sin `sharing.json`, `b`: `search_memory` trae `#42 [… by a]` con la nota al pie; `recall_context` la pone en RECENT CONTEXT con `by a` y agrega el bloque `SHARED`; `memory_status` termina en `shared: reads every agent`. `delete_memory` y `update_memory` sobre `#42` dan `daemon returned 404: no Memory row with id 42`, y la fila sigue apareciendo.
5. `clawg sharing hide b a --profile dev` (escribe `{"version":1,"hides":{"b":["a"]}}`), sin reiniciar el daemon (mismo pid): `b` ya no ve `#42` ni en la búsqueda ni en el contexto, y `memory_status` dice `shared: reads every agent but a`. `a` sigue viendo `#43 [… by b]` y su estado dice `shared: reads every agent`.
6. `clawg sharing unhide b a --profile dev`: `b` vuelve a ver `#42` con `by a`.
7. Limpieza: `a` borró `#42`, `b` borró `#43`, y la búsqueda queda vacía; se borró el `sharing.json` que dejó la prueba.

## Lo que encontró el archive

La spec del cambio y el código coinciden en R1–R9: no hubo diferencias. Al consolidar en `specs/sharing` aparecieron dos huecos que el cambio no cubre:

- **Un save largo compartido no se puede leer entero.** La búsqueda muestra el mejor pedazo de un save ajeno (`piece=K/N by a`), pero `read_memory` sobre ese id falla como si no existiera, porque `GET /v1/memories/{kind}/{id}/save` sigue con el scope de `?agent=` (`an_agent_cannot_read_another_agents_save`, `crates/clawg-cli/src/mcp/server.rs:2160-2168`). Es lo que dice `specs/visibility`. Leer no es modificar, así que abrir `read_memory` a lo que la política comparte es una decisión pendiente.
- **El parser de `rag-eval` no reconoce ` by <autor>` en el encabezado de un hit** (`[fecha #id (tipo)?]`, en `specs/retrieval`). Hoy no importa, porque `rag-eval` busca con `Reach::Own` y nunca recibe líneas ajenas, pero un golden set con varios agentes lo necesitaría.

## Nota sobre el binario del editor

El bridge del editor, que es binario viejo, sigue funcionando contra el daemon nuevo, porque el embedder vive en el daemon. Si un binario anterior a project-indexing vuelve a levantar el daemon dev, el contrato de embeddings ya no coincide (`BootstrapError::Mismatch`, sin reintento): hay que recompilar `target\debug\clawg.exe` desde `main`.
