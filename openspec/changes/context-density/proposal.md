# context-density — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S–M | — (se suma a dispute-resolution) | propuesta |

## Problema

`recall_context` repite lo que el mismo prompt ya mostró y le manda ruido al modelo. Datos de `docs/eval/results/2026-10-12-real-lines-conflicts-shown-agent-view.json` (46 casos):

- ACTIVE CONFLICTS es el **65 %** de los tokens: 2.112 de media cuando aparece (45/46). El total es 3.196 de media y 4.625 de máximo.
- Cada par copia el texto completo de las dos filas y una NOTE (`prompt_builder.rs:309-320`), aunque el lado mostrado ya esté arriba. Una fila nueva que choca con varias viejas repite su texto en cada par (`:394-395`).
- La consulta no tiene `ORDER BY` ni `LIMIT` (`:390-401`). Es el único bloque sin techo, porque los hits se cortan en `K_FINAL = 10` (`:32`, `:157`).
- Los hits no llevan id (`:300-306`), así que el agente no puede relacionar un conflicto con lo que leyó.
- El comentario de `:109` dice que se sacan los marcadores, pero `strip_empty_workspace_markers` devuelve el archivo entero apenas hay contenido (`:345-370`). Por eso llegan al modelo los `<!-- … -->` y los encabezados vacíos (`workspace.rs:388-407`).

## Alcance

1. **Conflictos.** El lado ya mostrado como hit se cita por `#id`, y sólo el lado no mostrado lleva el texto completo. Los pares que comparten la fila nueva se agrupan, así que su texto aparece una vez. La NOTE va una sola vez por bloque. El orden es determinista: primero van los pares cuyo lado mostrado aparece más arriba entre los hits, y al final los que sólo tocan una fila del workspace. Así un conflicto sobre una preferencia no ocupa el tope en cada llamada, sea cual sea la pregunta. Hay un tope de 5 pares; si quedan pares afuera, una línea final dice cuántos.
2. **Hits en una línea:** `[YYYY-MM-DD #id tipo] texto`. El kind se muestra sólo si no es `memory`.
3. **Workspace:** se sacan los marcadores, la sección USER vacía y los encabezados sin bullets **sólo en el prompt**. El archivo no cambia, porque la regeneración usa los marcadores para encontrar y preservar la sección USER (`split_existing` en `workspace.rs:416`, `assemble_file` en `:475`).
4. **Una fila del workspace que está en un conflicto** cuenta como no mostrada y lleva su texto, porque su bullet no tiene id (`workspace.rs:394`).
5. **Parser del eval:** `context_segments` e `is_markup` (`crates/clawg-eval/src/bin/rag-eval/agent_view/parse.rs:78-137`) se actualizan en el mismo cambio. `found` mantiene su definición: cuenta los hits y los bullets del workspace, y el texto de un conflicto no cuenta como visible.
6. **`delete_memory`:** la doc del argumento `id` (`server.rs:128-129`) pasa a decir que también sirven los ids de `recall_context`, y que un hit sin kind explícito es `memory`. La tool exige `kind` (`server.rs:131-134`).

**Enfoque:** el orden depende de la posición de cada fila en el prompt, que la consulta no conoce. El orden y el tope se calculan con una función pura sobre los pares leídos, que se puede testear sin base.

## Fuera de alcance

- Resumir o reemplazar originales.
- Pasar el formato a JSON o YAML: el agente ya recibe texto plano (`server.rs:205-207`).
- `search_memory`, que ya recorta y corta por relevancia (`tools.rs:23`, `:57`).
- Bajar los falsos `disputed`: es dispute-resolution (9). Los dos cambios se complementan.
- Un presupuesto global de tokens.

## Éxito (umbrales comprometidos)

Se mide con `rag-eval --real --agent-view --diff` contra `2026-10-12-real-lines-conflicts-shown`:

| Métrica | Hoy | Umbral |
|---|---|---|
| `found` | 41/46 | 41/46, sin bajar |
| `search_memory`, media | 852 | sin cambios |
| ACTIVE CONFLICTS, media cuando aparece | 2.112 | ≤ 800 |
| Total de `recall_context`, media / máximo | 3.196 / 4.625 | ≤ 2.100 / ≤ 3.000 |
| RECENT CONTEXT, media | 1.130 | no crece |

`cargo test --workspace` tiene que quedar en verde contra la base de 1.304. Los tests nuevos fallan hoy y verifican que:

- el bloque no repite una fila ya mostrada;
- el prompt no contiene `<!-- `;
- un encabezado sin bullets no se emite;
- el bloque respeta el tope y su orden es estable.

## Datos guardados

No cambia ningún dato: el cambio sólo toca cómo se arma el texto.

- `fetch_active_conflicts` es un `SELECT` (`prompt_builder.rs:390-401`).
- La lectura del workspace no escribe (`:110-119`).
- Las escrituras que ya hay en el camino de `recall_context` quedan iguales: la regeneración del workspace (`crates/clawg-cli/src/daemon/api.rs:521-527`) y el registro de accesos (`prompt_builder.rs:168-175`).

No hace falta migración. El rollback es revertir los commits.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Algo depende del formato `CONTENT:` / `OLD:` / `NEW:` | Baja | En el repo sólo lo lee el parser del eval (`parse.rs:99`), que entra en el alcance |
| El texto de un conflicto se parece a un hit y el parser lo cuenta, lo que infla `found` | Media | Los hits se segmentan sólo en RECENT y LONG-TERM, y los bullets sólo en WORKSPACE. Se adapta el test `parse.rs:260-264` |
| `delete_memory` exige `kind` (`server.rs:131-134`), y el hit lo omite cuando es `memory` | Media | Alcance 6: la doc del `id` lo dice explícitamente |
| Ids visibles para el agente | Baja | `search_memory` ya los muestra (`tools.rs:94`) |

## Design

No hace falta `design.md`. Las alternativas (opciones a/b/c, ids en los hits, tope, filas del workspace) ya se cerraron en la exploración, y lo que queda es formato y orden, que la spec fija con escenarios.
