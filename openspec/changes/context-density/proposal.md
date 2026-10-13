# context-density — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | — (se suma a dispute-resolution) | spec |

## Problema

`recall_context` repite lo que el mismo prompt ya mostró y le manda ruido al modelo. Datos de `docs/eval/results/2026-10-13-real-lexical-fusion-main-agent-view.json` (`main` en `d28fe90`, 46 casos):

- ACTIVE CONFLICTS es el **66 %** de los tokens: 2.274 de media cuando aparece (45/46). El total es 3.355 de media y 4.900 de máximo.
- Cada par copia el texto completo de las dos filas y una NOTE de tres líneas (`format_conflict_pair`, `crates/clawg-core/src/memory/prompt_builder.rs:299-311`), aunque el lado mostrado ya esté arriba. Una fila nueva que choca con varias viejas repite su texto en cada par (el join en `:385-386`).
- Desde `aa52e3b`, `fetch_active_conflicts` (`:368-420`) sólo trae pares con un lado mostrado: un hit `memory` o una fila del workspace (`shown`, `:187-193`). Pero la consulta sigue sin `ORDER BY` ni `LIMIT` (`:381-393`): una fila mostrada que choca con muchas trae todos sus pares. En el golden set el filtro no bajó ni un token: `197186d`, que ya lo tiene, da los mismos 3.196 que `3b7b13f`, que no lo tiene. Es el único bloque sin techo, porque los hits se cortan en `K_FINAL = 10` (`:32`, `:156`).
- La NOTE se repite en cada par. Desde que existe `resolve_conflict` tiene una línea más (`Con su respuesta, llamá a resolve_conflict …`), y sólo eso suma unos 25 tokens por par: es lo que llevó la media de 3.196 a 3.355.
- Los hits no llevan id (`format_hit_line`, `:288-296`), así que el agente no puede relacionar un conflicto con lo que leyó.
- El comentario de `:107-108` dice que se sacan los marcadores, pero `strip_empty_workspace_markers` devuelve el archivo entero apenas hay contenido (`:331-361`). Por eso llegan al modelo los `<!-- … -->` y los encabezados vacíos (`render_rows`, `workspace.rs:445-464`).

## Alcance

1. **Conflictos.** El lado ya mostrado como hit se cita por `#id`, y sólo el lado no mostrado lleva el texto completo. Los dos ids de cada par se imprimen siempre, porque son los que `resolve_conflict` recibe como `keep` y `other` (`ResolveConflictArgs`, `crates/clawg-cli/src/mcp/server.rs:219-232`). Los pares que comparten la fila nueva se agrupan, así que su texto aparece una vez. La NOTE va una sola vez por bloque, con sus tres líneas. El orden es determinista: primero van los pares cuyo lado mostrado aparece más arriba entre los hits, y al final los que sólo tocan una fila del workspace. Así un conflicto sobre una preferencia no ocupa el tope en cada llamada, sea cual sea la pregunta. Hay un tope de 5 pares; si quedan pares afuera, una línea final dice cuántos.
2. **Hits en una línea:** `[YYYY-MM-DD #id tipo] texto`. El kind se muestra sólo si no es `memory`.
3. **Workspace:** se sacan los marcadores, la sección USER vacía y los encabezados sin bullets **sólo en el prompt**. El archivo no cambia, porque la regeneración usa los marcadores para encontrar y preservar la sección USER (`split_existing` en `workspace.rs:492`, `assemble_file` en `:551`).
4. **Una fila del workspace que está en un conflicto** cuenta como no mostrada y lleva su texto, porque su bullet no tiene id (`workspace.rs:451`, `:455`).
5. **Parser del eval:** `context_segments` e `is_markup` (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs:78-137`) se actualizan en el mismo cambio. `found` mantiene su definición: cuenta los hits y los bullets del workspace, y el texto de un conflicto no cuenta como visible. Hoy el parser recorre todo el texto sin mirar la sección; pasa a segmentar por sección.
6. **`delete_memory`:** la doc del argumento `id` (`server.rs:209-211`) y la descripción de la tool (`:727-728`) pasan a decir que también sirven los ids de `recall_context`, y que un hit sin kind explícito es `memory`. La tool exige `kind` (`:213-216`).

**Enfoque:** el orden depende de la posición de cada fila en el prompt, que la consulta no conoce. El orden y el tope se calculan con una función pura sobre los pares leídos, que se puede testear sin base.

## Fuera de alcance

- Resumir o reemplazar originales.
- Pasar el formato a JSON o YAML: el agente ya recibe texto plano (`ok`, `server.rs:328`).
- `search_memory`, que ya recorta y corta por relevancia (`tools.rs:26`, `:57`).
- El aviso de `synthesize` que el puente MCP agrega después del prompt (`format_recall`, `tools.rs:205-211`): no es parte del prompt, no cambia y el agent-view no lo mide.
- El título `ACTIVE CONFLICTS (review needed)`: no cambia, porque las instrucciones del servidor (`server.rs:111`) y la doc de `resolve_conflict` (`:221`) lo nombran.
- Bajar los falsos `disputed`: es dispute-resolution (9). Los dos cambios se complementan.
- Un presupuesto global de tokens.

## Éxito (umbrales comprometidos)

Se mide con `rag-eval --real --agent-view --diff` contra `2026-10-13-real-lexical-fusion-main-agent-view.json`. Los umbrales se fijaron sobre la medición de `3b7b13f` y no se mueven. Desde entonces el bloque creció por la línea de `resolve_conflict` (`main` con `lexical_bonus = 0` da los mismos 3.355, y `197186d`, sin esa línea, 3.196).

| Métrica | Al proponer (`3b7b13f`) | Hoy (`d28fe90`) | Umbral |
|---|---|---|---|
| `found` | 41/46 | 41/46 | 41/46, sin bajar |
| `search_memory`, media | 852 | 852 | sin cambios |
| ACTIVE CONFLICTS, media cuando aparece | 2.112 | 2.274 | ≤ 800 |
| Total de `recall_context`, media / máximo | 3.196 / 4.625 | 3.355 / 4.900 | ≤ 2.100 / ≤ 3.000 |
| RECENT CONTEXT, media | 1.130 | 1.130 | no crece |

`cargo test --workspace` tiene que quedar en verde contra la base de 1.655 (Windows). Los tests nuevos fallan hoy y verifican que:

- el bloque no repite una fila ya mostrada;
- el bloque imprime los dos ids de cada par;
- el prompt no contiene `<!-- `;
- un encabezado sin bullets no se emite;
- el bloque respeta el tope y su orden es estable.

## Datos guardados

No cambia ningún dato: el cambio sólo toca cómo se arma el texto.

- `fetch_active_conflicts` es un `SELECT` (`prompt_builder.rs:381-393`).
- La lectura del workspace no escribe (`:109-118`).
- Las escrituras que ya hay en el camino de `recall_context` quedan iguales: la regeneración del workspace (`crates/clawg-cli/src/daemon/api.rs:800-810`) y el registro de accesos (`prompt_builder.rs:159-165`).

No hace falta migración. El rollback es revertir los commits.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Algo depende del formato `CONTENT:` / `OLD:` / `NEW:` | Baja | En el repo lo leen el parser del eval (`parse.rs:99`) y los tests de `prompt_builder.rs` (ver spec, R10); los dos entran en el alcance |
| El agente no encuentra los ids para `resolve_conflict` | Media | R2: los dos ids de cada par se imprimen siempre, aunque el texto se omita. La NOTE nombra `keep` y `other` |
| El texto de un conflicto se parece a un hit y el parser lo cuenta, lo que infla `found` | Media | Los hits se segmentan sólo en RECENT y LONG-TERM, y los bullets sólo en WORKSPACE. Se adapta el test `parse.rs:260-264` |
| `delete_memory` exige `kind` (`server.rs:213-216`), y el hit lo omite cuando es `memory` | Media | Alcance 6: la doc del `id` y la de la tool lo dicen explícitamente |
| Ids visibles para el agente | Baja | `search_memory` ya los muestra (`tools.rs:79`) |
| La fila vieja de un par es de otro agente | Nula | La consulta filtra por agente sólo la fila nueva, pero el detector busca contradicciones dentro del mismo `agent_id` (`contradiction.rs:139`), así que la vieja es del mismo agente |

## Design

No hace falta `design.md`. Las alternativas (opciones a/b/c, ids en los hits, tope, filas del workspace) ya se cerraron en la exploración, y lo que queda es formato y orden, que la spec fija con escenarios.
