# conclusion-upkeep — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 5 | S–M | — (gana sentido con 27 y 18; usa 41 si existe; ver «Interacciones») | exploración cerrada; ver [propuesta](proposal.md) |

**Tesis:** una conclusión (objetivo 5) se guarda una vez y después nadie la vuelve a mirar. Si sus fuentes se borran, quedan reemplazadas o se purgan, `synthesize` la sigue listando como válida y nada se lo dice al agente. Además, `derived_from` acepta como fuente una memoria que ya fue reemplazada. A medida que la memoria evoluciona, con las revisiones de 27 y los borrados y restauraciones de 18, las conclusiones se separan de su evidencia en silencio. El objetivo 5 pide extraer conclusiones **con el tiempo**, y eso incluye revisarlas cuando la base cambia.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| El vínculo conclusión → fuente existe y no se pierde al borrar | **Cierta.** `memory_derivation(memory_id, source_id, created_at)`, sin claves foráneas (`crates/clawg-core/migrations/V10__synthesis.sql:8-13`). El comentario (`:5-7`) dice que un vínculo a una fila borrada es historia que vale la pena conservar. El borrado lógico no toca la tabla (`crates/clawg-core/src/memory/admin.rs:286-338`; `openspec/specs/synthesis/spec.md:72`). |
| `synthesize` no mira el estado de las fuentes | **Cierta.** `CONCLUSION` lista las filas vivas, no `superseded`, que tienen vínculos (`crates/clawg-core/src/memory/synthesis.rs:38-39`), y devuelve sus `derived_from` (`admin.rs:573`). Ninguna consulta mira si esas fuentes siguen vivas. Una conclusión cuyas tres fuentes se borraron sale igual que una recién hecha. |
| Las fuentes borradas desaparecen de «pendientes» sin dejar rastro en la conclusión | **Cierta.** `PENDING` sólo considera filas vivas (`synthesis.rs:32-36`). Una fuente borrada sale de pendientes, y la conclusión no se entera. |
| `purge` borra el vínculo | **Cierta.** `DELETE FROM memory_derivation WHERE memory_id = ?1 OR source_id = ?1` (`crates/clawg-core/src/memory/secrets.rs:105-107`). La conclusión pierde la fuente sin rastro. |
| `derived_from` acepta fuentes reemplazadas | **Cierta.** La validación sólo exige que sean propias y no borradas (`existing_memory_ids`, `synthesis.rs:189-215`, SQL en `:200-201`; llamada en `crates/clawg-cli/src/daemon/api.rs:692-704`). Una fuente `superseded` o `disputed` se acepta. |
| Volver a derivar la misma conclusión suma fuentes pero no cambia nada visible | **Cierta.** El texto idéntico refuerza la fila (`access_count + 1`, tier recalculado; `crates/clawg-core/src/memory/store_insert.rs:280-325`) y agrega las fuentes nuevas con `INSERT OR IGNORE` (`store_insert.rs:198-212`, `:318`). La respuesta del save es la de siempre (`specs/synthesis`). |
| No hay columna de confianza | **Cierta.** Ni `confidence` ni `strength` en las migraciones. El puntaje no trata distinto a las conclusiones (`score.rs`, `tier.rs`). |
| El aviso de síntesis sólo cuenta pendientes nuevas | **Cierta.** `unreviewed_count` cuenta pendientes posteriores a la última revisión (`synthesis.rs:168-184`). El aviso de `recall_context` sale con 20 o más (`crates/clawg-cli/src/mcp/tools.rs:259-278`). Una conclusión con sus fuentes muertas nunca lo dispara. |

## Problema (verificado contra el código)

- **Una conclusión sin evidencia sigue pesando.** Se recupera como cualquier fila, con su tipo y su importancia. Si el usuario borró las memorias de las que salió, porque eran falsas o viejas, la conclusión sigue diciendo lo que ellas decían.
- **Una conclusión sacada de lo reemplazado.** Si una decisión se revisó (27 la deja `superseded`), la conclusión que se apoyaba en la versión vieja no se entera. Peor: hoy un agente puede derivar una conclusión **nueva** de esa versión vieja, porque `derived_from` la acepta.
- **Nadie lo mira.** `synthesize` es el único lugar donde el agente revisa conclusiones, y no muestra el estado de las fuentes. El aviso no cuenta este caso.

## Lo que ya existe y se reutiliza

- `memory_derivation` conserva los vínculos a filas borradas a propósito. El dato para saber «esta fuente ya no está» existe, salvo después de un `purge`.
- `synthesis_core` (`synthesis.rs:137-164`) y su salida: se le agrega una lista, sin herramienta nueva.
- `unreviewed_count` y el aviso de `recall_context`: se le suma un conteo.
- El refuerzo por texto idéntico, que ya agrega fuentes.

## Enfoques posibles

### A. `synthesize` muestra las conclusiones a revisar (recomendado)

Una lista acotada de conclusiones vivas con alguna fuente borrada, `superseded` o, si se decide, `disputed`. Por cada una: cuántas fuentes quedan vivas, de cuántas.

- **Decisión pura:** `needs_review(estados_de_las_fuentes) -> bool`.
- El aviso de `recall_context` suma estas conclusiones al conteo, o lleva una línea propia.
- **A favor:** es la misma herramienta y la misma consulta, más un `JOIN`. No hay esquema nuevo.
- **En contra:** más texto en la salida de `synthesize`. Se acota con un tope.

### B. `derived_from` rechaza fuentes `superseded` (recomendado, chico)

Una conclusión nueva no puede apoyarse en algo que el usuario ya reemplazó. El 400 nombra la fila vigente (`superseded_by`).

- **En contra:** un agente que listó pendientes hace un rato y guarda tarde puede chocar con un reemplazo intermedio. El mensaje le dice cuál es la vigente.

### C. Rastro de `purge`

Que `purge` deje el vínculo, apuntando a un id que ya no existe, para que la conclusión sepa que perdió una fuente. El id no se reusa (`AUTOINCREMENT`).

- **A favor:** A también ve las fuentes purgadas.
- **En contra:** `purge` promete que el texto no queda. Un id suelto no es texto, pero conviene confirmarlo contra `specs/secrets`.

### D. Una confianza que se refuerza y decae

Una columna nueva, con una regla de refuerzo al volver a derivar y de decaimiento con el tiempo.

- **En contra:** constantes sin medir, un cambio de ranking que exige `rag-eval --diff`, y una migración. El refuerzo que ya existe (accesos y tier) más la cantidad de fuentes vivas dan una señal sin inventar números. Se descarta por ahora.

## Descartado

| Opción | Razón técnica |
|---|---|
| Borrar sola una conclusión que perdió sus fuentes | Destruye algo que el agente o el usuario guardaron. La regla del proyecto lo impide. Se muestra para revisar y decide quien la mira. |
| Guardar el hash de cada fuente al derivar | Detecta «la fuente cambió». Pero con topic-revisions en su forma recomendada (E2) el contenido nunca cambia en el lugar: una revisión es una fila nueva y la vieja queda `superseded`, que A ya ve. Se reabre sólo si 27 elige reemplazar en el lugar (E1). |
| D (confianza con constantes) | Ver arriba. |

## Interacciones

- **topic-revisions (27).** Con 27, «una fuente cambió» pasa a ser «una fuente quedó `superseded`». Es lo que hace que A tenga casos reales.
- **recoverable-delete (18).** Restaurar una fuente saca la conclusión de la lista de revisión. A lo calcula al vuelo, así que no hay estado que limpiar.
- **conflict-candidates (24).** Si una fuente tiene un par escalado, ¿la conclusión entra en revisión? Es decisión, y depende del vocabulario de veredictos de 24.
- **memory-provenance (41).** Si las conclusiones quedan selladas como tales, `search_memory` y `recall_context` pueden marcarlas. Este cambio no lo necesita para funcionar.
- **context-budget (23) y agent-lifecycle-hooks (25).** El aviso con el conteo nuevo se paga dentro del presupuesto de 23 y lo puede entregar el hook de 25.

## Alcance candidato

1. `needs_review` pura, con su tabla de casos.
2. La lista de conclusiones a revisar en `POST /v1/synthesis` y en el texto de `synthesize`, con un tope.
3. El conteo en el aviso de `recall_context`.
4. `derived_from` rechaza fuentes `superseded` (B).
5. Si se decide, el rastro de `purge` (C).
6. Docs: `specs/synthesis` y `.claude/CLAUDE.md`.

### No-alcance

- Columnas de confianza (D).
- Borrar o reescribir conclusiones solas.
- Cambios de ranking.

## Decisiones abiertas

Son del usuario.

- [x] **¿Qué estados de una fuente mandan a revisión?** Borrada, `superseded` y purgada; `disputed` no.
- [x] **¿Desde cuántas fuentes perdidas?** Una.
- [x] **B.** Sí, con 400 que nombra el reemplazante.
- [x] **C.** Sí, el vínculo de la fila como fuente.
- [x] **Aviso.** Línea propia, contando las pérdidas posteriores a la última revisión ([diseño](design.md#d3-el-aviso)).

## Cómo se mide

Tests RED primero:

1. Una conclusión con una fuente borrada aparece en la lista a revisar de `synthesize`. Hoy no aparece.
2. Con todas sus fuentes vivas, no aparece.
3. Restaurar la fuente la saca de la lista.
4. Un `derived_from` con una fuente `superseded` se rechaza nombrando la vigente (si entra B).
5. El aviso de `recall_context` cuenta las conclusiones a revisar.
6. Otro agente no ve en su lista las conclusiones ajenas (mismo alcance que `synthesize` hoy).

`rag-eval` no aplica: no cambia el ranking ni lo que `recall_context` recupera, sólo el aviso. Si el aviso cambia de forma, se reportan los tokens de agent-view. Sin esquema nuevo, salvo que entre C, que no cambia el esquema sino un `DELETE`.

## Riesgos

- **Ruido.** Si casi toda conclusión tiene alguna fuente borrada, la lista no se lee. El umbral (una contra la mayoría) se decide mirando datos reales.
- **Costo de la consulta.** Un `JOIN` contra los estados de las fuentes, con el índice por fuente ya existente (`V10__synthesis.sql:16`). Se mide si el volumen crece.
- **B rechaza un save legítimo** que llegó tarde. El mensaje nombra la vigente para que se pueda reintentar.
