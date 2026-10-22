# context-hit-room — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | 23 (context-budget) | exploración |

**Tesis:** en un store maduro, `recall_context` entrega el workspace, el último resumen y los conflictos, y ninguna memoria de la consulta. La regla de recorte de 23 saca primero los hits y nunca el workspace, y se midió con el golden set, que no tiene workspace ni resúmenes largos: ahí ningún prompt se cortaba. Con un workspace bien provisto, un resumen de sesión largo y un par en disputa, lo que queda del presupuesto para los hits es cero, aunque lo único que la llamada pidió fueron esos hits.

## Evidencia

- `revision-eval chains --real --background` con un export de prod (829 memorias): la última versión de una cadena no aparece en `recall_context` en ninguna de las 40 cadenas, en ningún brazo. El prompt medio pasa de 3.780 a 7.357 bytes y ningún hit queda con cabecera ([revision-recheck](../archive/2026-10-22-revision-recheck/tasks.md), §2).
- Un `recall_context` real de este proyecto, el 2026-10-22: «Cut to fit 8000 bytes; the full context is 19134 bytes», con los 10 hits afuera, los de la consulta incluidos, y el contexto hecho del workspace, LAST SESSION recortado y un conflicto (misma fuente).
- `search_memory` no se ve afectado: no tiene presupuesto ni workspace.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| El recorte saca primero los hits | **Cierta.** `fit_to_budget` elige el bloque con `next_to_cut` sobre RECENT y LONG-TERM, y sólo cuando no queda ninguno pasa a LAST SESSION y después a los conflictos (`crates/clawg-core/src/memory/prompt_builder.rs:417-421`, `:423-430`). |
| El workspace nunca se corta | **Cierta, por diseño.** `fit_to_budget` no tiene un paso para él (`prompt_builder.rs:399-403`); las filas que imprimió están en `EmittedIds` y la búsqueda las saltea (`:217`), así que cortarlas las dejaría sin canal (D2 de [context-budget](../archive/2026-10-17-context-budget/design.md)). |
| El workspace puede ocupar casi todo el presupuesto | **Cierta.** La sección AUTO tiene un tope de 6.000 bytes (`AUTO_BYTE_CAP`, `crates/clawg-core/src/memory/workspace.rs:19-20`) que nunca recorta las preferencias (`workspace.rs:395-397`), y la sección USER no tiene tope. Con el default de 8.000, un AUTO lleno deja 2.000 bytes para todo lo demás. |
| LAST SESSION puede ser largo | **Cierta.** El bloque muestra el save entero, sus pedazos unidos (`with_last_session`, `prompt_builder.rs:813-820`), y un resumen sigue el protocolo hasta «Next steps» y «Relevant files». |
| Los conflictos ocupan texto aunque su hit se haya cortado | **Cierta.** Un lado cuyo hit se cortó se imprime con su texto (`format_conflict_block`, `prompt_builder.rs:745-751`), hasta cinco pares (`MAX_CONFLICT_PAIRS`, `:49`). Cortar los hits primero hace crecer el bloque de conflictos. |
| El hook `session-start` pasa por el mismo armado | **Cierta.** `build_session_prompt` llama al mismo `assemble` (`prompt_builder.rs:288-289`) con `SESSION_BUDGET = DEFAULT_BUDGET` (`crates/clawg-cli/src/hook.rs:33`). Sus hits son los saves más nuevos, todos con score 0, y no una respuesta a una consulta. |
| 23 se midió sin recortes | **Cierta.** Con el default ningún prompt del golden se cortaba y ningún caso se movió (`docs/eval/results/2026-10-17-real-context-budget-tip-agent-view.json`, citado en `openspec/config.yaml`). El golden no tiene workspace y su único resumen es la línea de 87 tokens. |

## Enfoques

| | A favor | En contra |
|---|---|---|
| **A. Reservarles a los hits una parte del presupuesto** | Puro y local a `fit_to_budget`; los mejores hits sobreviven a cualquier tamaño de store; el orden de 23 sigue para el resto | Si el workspace solo se come el presupuesto, la reserva no alcanza: el workspace sigue sin cortarse |
| B. Ponerle un tope al workspace | Ataca el bloque que no se corta | Las filas que el workspace imprimió están en `EmittedIds`: cortarlas en el prompt las deja sin canal, salvo que el corte se decida antes de buscar y se les devuelva el canal. Las preferencias son «always active» y la sección USER es del usuario. Es otro cambio, más grande |
| C. Cortar LAST SESSION antes que los hits | Una línea | Pierde la continuidad de sesión (session-handoff) en cada `recall_context` de un store maduro, y no alcanza si el workspace y los conflictos llenan el resto: los conflictos siguen después |
| D. Subir el presupuesto por defecto | Ninguna regla cambia | Un store sigue creciendo: el caso real ya ocupaba 19.134 bytes, y cualquier default fijo se pasa. El hook comparte el default y su salida se corta a 10.000 caracteres. Encarece cada llamada |

**Elegido: A**, sólo para el contexto con consulta. B queda fuera y se abre como cambio propio si una medición real muestra que el workspace solo llena el presupuesto. C y D no arreglan la estructura.

## Decisiones abiertas

1. **Cuánto se reserva.** Un cuarto del presupuesto: 2.000 bytes con el default, unos cinco hits del golden (media de unos 400 bytes por hit) o uno de 480 tokens. Deja tres cuartos al workspace, al resumen y a los conflictos, que con el default es lo que el AUTO puede llenar.
2. **Qué cede ante la reserva.** El resto de los hits primero (como hoy), después LAST SESSION, después los conflictos, y recién entonces los hits de la reserva.
3. **El contexto sin consulta.** Sin reserva: al iniciar una sesión LAST SESSION es el traspaso y RECENT son sólo los saves más nuevos, no la respuesta a nada.
4. **Cómo se mide.** Un test con un store de esa forma, que falla hoy; y, con autorización, el mismo `recall_context` real y `revision-eval chains --real --background`, que piden el modelo real y un export del store.
