# dispute-resolution — Verificación

**Estado:** verificado. Los tests pasan en Linux y en Windows, la hipótesis de los pedazos hermanos quedó confirmada con el modelo real, y `rag-eval --real` en Windows da diff vacío en retrieval y similarity, sin bajar ninguna métrica del ancla (ver [rag-eval](#rag-eval)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 197186d) | 1.551 | 10 |
| Con el cambio (3cc85ac) | 1.588 | 10 |

Son 37 tests nuevos y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

Cada commit del cambio se probó por separado, con el mismo comando: todos compilan y ninguno agrega fallas (1.569, 1.572, 1.575, 1.576, 1.578, 1.584 y 1.588 pasan; 10 fallan en todos).

### Windows (2026-10-13)

`cargo test --workspace --no-fail-fast --target-dir target-test` sobre `main` en `a1ba363` (incluye el cambio y el scope por agente que le sumó visibility-policy en `8f0eef2` y `9f954f8`): **1.655 pasan, 0 fallan**.

Los 24 tests que nombra `spec.md` (R1 a R8 y los datos de la versión anterior) aparecen uno por uno como `ok` en la salida.

Las 10 fallas de Linux no aparecen: las 7 de `doctor::tests` modelan justamente un PATH de Windows, y los 3 de `diff::tests` encuentran `docs/eval/results/` en esta máquina.

## Hipótesis de los pedazos hermanos (con el modelo real)

**Confirmada, y es mucho más grave de lo que suponía la exploración.**

Prueba hecha con multilingual-e5-small real (descargado a un directorio temporal, sin tocar `~/.clawg*`) a través de `EvalIngest`, que usa el mismo `memory_worker` que el daemon. Cada uno de los 40 `.md` de `openspec/changes/` se guardó como **un** save explícito `semantic_fact`, cada uno con su propio `agent_id`, así que el detector sólo podía emparejar pedazos del mismo save. El programa de la prueba no se commiteó.

| Corrida | Saves | Pedazos | Pedazos `disputed` | Saves con alguna disputa |
|---|---|---|---|---|
| `origin/main` (197186d) | 40 | 263 | 220 | 40 |
| Con el cambio (3cc85ac) | 40 | 263 | 0 | 0 |

En `main`, casi todos los pedazos después del primero quedan `disputed` contra sus hermanos: 220 de los 223 posibles (263 pedazos menos el primero de cada save). Los pedazos vecinos comparten el solapamiento del chunker y el mismo tema, y caen en la banda del detector. Con el cambio no queda ninguno.

Esto también importa para long-save-context, que esperaba esta respuesta: la causa común (pedazos de un mismo save que se tratan como afirmaciones independientes) queda confirmada, y la parte de las disputas queda resuelta acá.

## rag-eval

### Medición en Windows (2026-10-13)

**Entorno:** Windows 11, binarios debug de `rag-eval`, modelo real (multilingual-e5-small) leído del caché de `~/.clawg-dev/models` sin descargar ni escribir; los stores de eval van a `%TEMP%` y los resultados a un directorio temporal, fuera de `docs/eval/results/`.

**Binarios**, cada uno compilado en su worktree y con su directorio de build:
- Base: `48b0094` (`main` justo antes del merge de este cambio).
- Cambio: `b950c1a` (el merge de este cambio).

Así el diff aísla este cambio, sin lo que entró después en `main`.

**Golden set:** `docs/eval/golden.json` (ancla, sha256 `d2d9d732…ed792a`, n = 46).

**Corridas**, las mismas con los dos binarios:
- `rag-eval --real --golden docs/eval/golden.json --tag anchor`
- `rag-eval --real --agent-view --golden docs/eval/golden.json`

### Resultados

| Métrica | Línea de base (`proposal.md`) | Base | Cambio |
|---|---|---|---|
| R@1 | 0,4348 | 0,4348 | 0,4348 |
| R@5 | 0,7826 | 0,7826 | 0,7826 |
| R@10 | 0,8913 | 0,8913 | 0,8913 |
| MRR | 0,5979 | 0,5979 | 0,5979 |
| nDCG@10 | 0,6693 | 0,6693 | 0,6693 |

- `rag-eval --diff` del ancla, base contra cambio: 0 mejoran y 0 empeoran en todas las métricas, en retrieval y en similarity.
- `rag-eval --diff` del agent-view: igual, sin cambios.
- El barrido de umbral del detector (pares de contradicción, global y por idioma) da lo mismo en las dos corridas. Era lo esperado: esos pares son saves de un solo pedazo, y para ellos el detector no cambia.
- Agent-view, `search`: hit rate 0,8913 en las dos, tokens media 851,5 y máximo 1239 en las dos; los 46 casos tienen el mismo rango y los mismos tokens.
- Agent-view, `context`: hit rate 0,8913 en las dos, con el mismo rango en los 46 casos. Los tokens suben de 3196,3 a 3355,0 de media (máximo de 4625 a 4900).

### Los tokens de `recall_context`

Toda la suba está en el bloque ACTIVE CONFLICTS; RECENT CONTEXT da los mismos tokens caso por caso. La suba va en múltiplos exactos de 25 (45 de 46 casos, de 25 a 275; el caso que no sube no tiene conflictos): cada par suma la línea `Con su respuesta, llamá a resolve_conflict (keep = la que vale, other = la otra).`, que es lo que pide R6. La cantidad de conflictos no cambia, porque el ancla tiene saves de un solo pedazo y no hay pedazos hermanos que dejen de disputarse.

Esto también aclara lo que dejó anotado lexical-fusion en su verify (la suba de 3196,3 a 3355,0 de `context`, "lo traen otros cambios"): es esta línea. El costo del bloque lo baja context-density, que ya conserva la instrucción.

### Umbrales

| Umbral (`proposal.md`) | Medido | Resultado |
|---|---|---|
| R@1 / R@5 / R@10 sin bajar ninguna | iguales | Cumple |
| MRR / nDCG@10 sin bajar ninguna | iguales | Cumple |
| Diff vacío en retrieval y similarity (`--real` y `--agent-view`) | vacío | Cumple |

## Solapamiento con otros cambios en curso

- **storage (data-preservation B–E):** un undelete futuro devuelve filas cuyas disputas ya se liberaron al borrarlas (`delete_memory_core` ahora libera en su transacción). La disputa no vuelve sola. Está en los riesgos de la propuesta.
- **project-identity:** el detector compara dentro del mismo `project_id`. Un re-key cambia qué filas se ven entre sí, pero no toca este código.
- **lexical-fusion:** toca `search.rs`. Este cambio no lo modifica; el detector sigue con `lexical: None`.
- **context-density:** el bloque ACTIVE CONFLICTS ganó una línea (la que nombra `resolve_conflict`). Si ese cambio reescribe el bloque, tiene que conservar esa instrucción.
- **Migraciones:** este cambio no agrega ninguna, así que no compite por V9.

## Backlog del mapa

El comentario desactualizado de `worker.rs` que figura en el backlog de `openspec/README.md` quedó corregido en `fix(core): stop chunks of one save from disputing each other`. La línea del backlog se saca al archivar.
