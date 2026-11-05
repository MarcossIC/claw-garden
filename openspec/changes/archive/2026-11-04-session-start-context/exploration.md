# session-start-context — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | S–M | — | exploración; decisiones cerradas, lista para aplicar |

**Tesis:** instalar los hooks en todos los proyectos hoy empeora el arranque de cada sesión. Las preferencias ocupan todo el presupuesto, el resumen de la última sesión queda afuera y el aviso de `synthesize` sale en cada proyecto importado. Este cambio arregla el reparto y el aviso. Qué preferencias merecen entrar lo decide [preference-model](../../preference-model/exploration.md).

## Problema (verificado)

- **Las preferencias se comen el presupuesto.** `session-start` pide 8.000 bytes (`SESSION_BUDGET`, `crates/clawg-cli/src/hook.rs:34`). En un store real con hooks sin instalar, el workspace de un agente mide 14.560 bytes, y 14.400 son 18 preferencias.
- **Eso deja el arranque sin traspaso.** `specs/retrieval`, «Un workspace más grande que el presupuesto se entrega y se dice», manda llevar el workspace entero y ningún hit. Medido el 2026-11-03: un `recall_context` de 12.000 bytes devolvió sólo el workspace y dejó afuera el resumen de la última sesión.
- **Pasa en todos los proyectos.** `fetch_auto_rows` filtra las preferencias por agente y no por proyecto (`crates/clawg-core/src/memory/workspace.rs:360`), así que el mismo bloque entra en cada repositorio.
- **El aviso de `synthesize` cuenta lo importado.** `PENDING` (`crates/clawg-core/src/memory/synthesis.rs:34`) no mira `source`. En un proyecto con 1.839 filas propias da 13.552 pendientes, y con 20 o más el hook lo dice en cada arranque.

## Decisiones (cerradas)

1. **Lo que escribió la persona no se corta.** Su parte de `workspace.md` entra siempre; lo único que se recorta es la sección AUTO.
2. **La sección AUTO usa como mucho la mitad del presupuesto** cuando hay un resumen o hits esperando. Sin nada esperando, puede usar todo.
3. **El corte va por importancia descendente y, a igual importancia, lo más nuevo primero.** Lo cortado se nombra por id en el bloque `BUDGET`, como ya se hace con los hits, para que `read_memory` lo traiga.
4. **El resumen de la última sesión tiene prioridad** sobre RECENT y LONG-TERM, como hoy, una vez hecho el lugar del punto 2.
5. **El aviso no cuenta filas de `source = 'import'`.** `synthesize` sigue listándolas si se lo llama; lo que cambia es el aviso, no lo que se puede revisar.
6. **Vale para las dos rutas que arman contexto:** `build_session_prompt` y `build_memory_prompt`, porque comparten `assemble`.

## Alcance

**Dentro:** `crates/clawg-core/src/memory/prompt_builder.rs`, el conteo de `synthesis.rs`, `workspace_for_prompt` si hace falta, `specs/retrieval`, `specs/synthesis`, y `docs/reference/daemon.md` si dice lo contrario.

**Fuera:** qué filas son elegibles para el workspace, el alcance por proyecto, `crates/clawg-cli/src/hook.rs` (lo toca client-setup-cursor en paralelo), el contrato del agente, subir la versión y archivar.

## Cómo se mide

Tests en rojo primero, sin modelo, porque el contexto sin consulta no embebe:

- un workspace de 14 KB y un resumen de sesión con presupuesto de 8 KB: el resumen está, y las preferencias cortadas se nombran por id;
- la parte de la persona más grande que el presupuesto: se entrega entera y se dice, como hoy;
- sin resumen ni hits: las preferencias usan todo el presupuesto;
- 25 filas importadas y 3 propias: `unreviewed_count` para el aviso da 3, y `synthesize` sigue listando las 28.

`rag-eval --diff` no aplica: no cambia el ranking.

## Por qué va en la nube

Se prueba entero en Linux con el embedder falso, y no necesita datos reales.
