# knowledge-synthesis — Verificación

**Estado:** verificado. Los tests pasan en Linux (commit por commit) y en Windows, `rag-eval --real` da diff vacío en retrieval, similarity y agent-view, y la prueba con el perfil dev en Claude Code cumple cada paso (ver [Windows](#windows-2026-10-13), [rag-eval](#rag-eval) y [Perfil dev](#perfil-dev-con-claude-code-2026-10-13)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, en cada commit de la rama.

| Commit | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, c01a0af, con visibility-policy) | 1.696 | 10 |
| `docs(openspec): propose knowledge synthesis driven by the agent` (5ccb77d) | 1.696 | 10 |
| `refactor(core): remove the unused LLM extraction pipeline` (675f574) | 1.563 | 10 |
| `feat(core): add tables for memory provenance and synthesis reviews` (8214a7d) | 1.564 | 10 |
| `feat(core): record which memories a saved memory was derived from` (b1e41d6) | 1.570 | 10 |
| `feat(core): show the sources of a derived memory` (71fa3a6) | 1.571 | 10 |
| `feat(core): list the memories no conclusion covers yet` (757f169) | 1.579 | 10 |
| `feat(daemon): accept memory sources and serve synthesis` (81bec0c) | 1.587 | 10 |
| `feat(mcp): let the agent draw conclusions from its memories` (631fccd) | 1.595 | 10 |
| `docs: list the synthesis endpoint and tool` (5b3ed48) | 1.595 | 10 |

Todos los commits compilan, y las 10 fallas son las mismas en todos.

Después se mergeó `main` (262f839, con mcp-protocol-upgrade y secret-hygiene) en la rama. Con el merge, `fix(core): drop a purged memory's provenance links` y los arreglos de la revisión de CodeRabbit encima: **1.637 pasan, 10 fallan**, las mismas 10. El merge pidió dos ajustes:

- `synthesize` toma el agente del `RequestContext` de cada pedido, como el resto de las herramientas desde rmcp 3.5.1.
- `clawg purge` borra la fila de verdad, y SQLite puede darle su id a una fila nueva. Por eso ahora también borra sus enlaces de `memory_derivation` (spec R6).

Las 10 fallas vienen del entorno:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

**Por qué baja el conteo:** la limpieza borra 133 tests. Todos probaban el código que se borra: `extractor`, `miner`, `resolver`, `episodic_writer` y `models/error.rs`, más los lectores sin llamador de `admin.rs` (histograma de fallas y tendencia de pasos) y `update_trajectory_root_cause` y `fetch_prior_trajectories` de `store_insert.rs`. Ningún test de lectura de `episodic_trajectory` ni `procedural_skill` se borró: son los que cubren R1 (ver la spec), y pasan antes y después. El cambio agrega 32 tests nuevos.

Los warnings de compilación bajan de 11 a 6. Los 6 que quedan ya estaban en la base: `insert_row_with_embedding` en los tests del worker, `ceiling` y `production_lexical_budget` en `rag-eval`, entre otros.

### Windows (2026-10-13)

`cargo test --workspace --no-fail-fast --target-dir target-test` sobre `main` en `1569ea3` (incluye el merge de este cambio, `c45b18a`): **1.655 pasan, 0 fallan**.

Los tests que nombran `spec.md` y `tasks.md` (procedencia en el worker, `synthesis.rs`, HTTP, MCP, `purging_drops_the_links_to_and_from_the_row` y `a_store_from_the_previous_version_opens_with_every_row_untouched`) aparecen uno por uno como `ok`.

Las 10 fallas de Linux no aparecen: las 7 de `doctor::tests` modelan justamente un PATH de Windows, y los 3 de `diff::tests` encuentran `docs/eval/results/` en esta máquina.

## rag-eval

### Medición en Windows (2026-10-13)

**Entorno:** Windows 11, binarios debug de `rag-eval`, modelo real (multilingual-e5-small) leído del caché de `~/.clawg-dev/models` sin descargar ni escribir; los resultados van a un directorio temporal, fuera de `docs/eval/results/`.

**Binarios**, cada uno compilado en su worktree y con su directorio de build:
- Base: `262f839` (`main` justo antes del merge de este cambio).
- Cambio: `c45b18a` (el merge de este cambio).

**Golden set:** `docs/eval/golden.json` (ancla, sha256 `d2d9d732…ed792a`, n = 46), el mismo que midió dispute-resolution.

**Corridas**, las mismas con los dos binarios:
- `rag-eval --real --golden docs/eval/golden.json --tag anchor`
- `rag-eval --real --agent-view --golden docs/eval/golden.json`

### Resultados

| Métrica | Base | Cambio |
|---|---|---|
| R@1 | 0,4348 | 0,4348 |
| R@5 | 0,7826 | 0,7826 |
| R@10 | 0,8913 | 0,8913 |
| MRR | 0,5979 | 0,5979 |
| nDCG@10 | 0,6693 | 0,6693 |

- `rag-eval --diff` del ancla, base contra cambio: 0 mejoran y 0 empeoran en todas las métricas, en retrieval y en similarity.
- `rag-eval --diff` del agent-view: igual, sin cambios.
- Agent-view, `search`: hit rate 0,8913 en las dos, tokens media 851,5 y máximo 1239 en las dos.
- Agent-view, `context`: hit rate 0,8913 en las dos, tokens media 3355,0 y máximo 4900 en las dos. El recordatorio va fuera de `build_memory_prompt` (diseño D6) y no suma tokens al prompt medido.

Es lo esperado: la búsqueda no cambia, y una conclusión es una fila de `memory` como cualquier otra.

## Perfil dev con Claude Code (2026-10-13)

**Entorno:** el MCP `clawg-dev` de Claude Code (agente `claude-code`, `--agent`) contra el daemon dev en `target\debug\clawg.exe`, compilado después del merge. Para no mezclar con las memorias del repo, la prueba usó un proyecto aparte en un directorio temporal, con memorias rotuladas como fixtures; al terminar se borraron todas (#12 a #32).

| Paso | Resultado |
|---|---|
| 19 memorias guardadas: `recall_context` no muestra el recordatorio | Cumple |
| La memoria 20: `recall_context` termina con `20 memories here have not been reviewed for conclusions yet. When the current task is done, call `synthesize`.` | Cumple |
| `clawg mcp --agent otro` (stdio, mismo proyecto): `synthesize` da `no memories are waiting for a conclusion` y `search_memory` no encuentra nada; el control con `--agent claude-code` y el mismo `project_path` sí las encuentra | Cumple |
| `synthesize` como `claude-code`: lista las 20 (`20 of 20`, más nuevas primero) con las instrucciones de cómo guardar | Cumple |
| `save_memory` con `derived_from: [17, 18]`: `list_memories` muestra `from=#17,#18` | Cumple |
| `recall_context` otra vez: la línea del recordatorio ya no está | Cumple |
| La conclusión no queda en disputa con sus fuentes (#17, #18) | Cumple (R4) |

**Calidad de las conclusiones:** no tiene métrica automática (propuesta, **Éxito**). Con la lista que devuelve `synthesize`, la conclusión que generaliza dos memorias (despliegue en un solo binario → trabajo en segundo plano dentro del proceso) sale directa, y la instrucción pide no repetir una sola memoria.

**Fuera de este cambio, anotado para context-density:** los fixtures compartían un prefijo idéntico, y el detector de disputas los emparejó entre sí (incluida la conclusión con fixtures que no son sus fuentes). Con eso, ACTIVE CONFLICTS listó 26 pares sin tope, cada uno con sus tres líneas de instrucción. El bloque no tiene límite de pares: es el costo que ya mide context-density.

## Datos guardados

- La migración V10 es aditiva (dos tablas nuevas). La prueba `a_store_from_the_previous_version_opens_with_every_row_untouched` abre una base en V9 con una memoria, una trayectoria y un skill, y comprueba que ninguna fila cambia.
- Ninguna tabla se borra. Las filas que dejó la app de escritorio siguen alcanzables (R1).

## Solapamiento con otros cambios

- **visibility-policy** (mergeado antes que éste): `synthesize` y `derived_from` siguen su regla. Un agente sólo revisa y cita sus propias filas, y una fila ajena cuenta como inexistente. La revisión que apaga el recordatorio es por agente.
- **Migraciones:** visibility-policy no agregó ninguna, así que V10 queda libre. Si otro cambio publica una V10 antes, éste renumera su archivo y su checksum.
- **context-density:** el recordatorio es una línea fuera del prompt. Si ese cambio reescribe `recall_context`, tiene que conservarla.
