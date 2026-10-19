# revision-aware-delete — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | 18 (recoverable-delete), 24 y 27, archivados | decidida; sigue en la [propuesta](proposal.md) |

**Tesis:** 18 se implementó antes que 24 y 27, aunque el plan decía que restaurar «tiene que conocer las candidatas y las revisiones». Borrar y restaurar siguen escritos para un mundo donde una fila es independiente de las demás. Hoy borrar la versión vigente de un tema esconde el tema entero, restaurar una fila reemplazada por una borrada deja un callejón sin salida, y una fila restaurada nunca se compara con lo que se guardó mientras estaba borrada. Este cambio hace que borrar y restaurar respeten las revisiones y los pares.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| Borrar la versión vigente deja a la anterior `superseded` apuntando a una fila borrada | **Cierta.** `delete_memory_core` sólo estampa `deleted_at`, borra el vector y libera a las retadoras (`crates/clawg-core/src/memory/admin.rs:331-338`). Ninguna sentencia toca `superseded_by`. Una revisión por clave deja a la vieja con `SUPERSEDE_SQL` y no escribe par (`topic.rs:131`, `dispute.rs:265`). |
| El tema desaparece de las lecturas | **Cierta.** El KNN exige `status IN ('active', 'disputed')` y `deleted_at IS NULL` (`search.rs:209-210`); `PENDING` y `CONCLUSION` de síntesis excluyen `superseded` (`synthesis.rs:32`, `:38`). La vieja sigue en `list_memories` y `read_memory`, que no filtran. |
| La vieja no se puede restaurar ni juzgar | **Cierta.** `RESTORE_READ` exige `deleted_at IS NOT NULL` (`admin.rs:395-398`) y la vieja no lo está. `judge_pair` lee ambas filas con `SELECT_PAIR_ROW`, que exige `deleted_at IS NULL` (`dispute.rs:218-219`, `:329-330`): con la vigente borrada devuelve `NotFound`. |
| Lo único que la trae de vuelta es restaurar la vigente | **Cierta**, y sólo mientras nadie guarde otra versión de la clave: `RESTORE_TOPIC_HOLDER` (`admin.rs:412-419`) da `TopicTaken` (409). Es la sesión C observada: se borró #70, #69 quedó `superseded by=#70`, se guardó #71 con la misma clave y no reemplazó nada (`openspec/changes/archive/2026-10-18-topic-revisions/verify.md:59`). |
| Restaurar una fila reemplazada por una borrada la deja `superseded` | **Cierta.** `settle_sql` sólo escribe filas `active` o `disputed` (`dispute.rs:182`), así que `settle_row` no la toca (`admin.rs:511`). `RestoredRow` devuelve `status` y `superseded_by` (`admin.rs:386-390`). |
| `format_restore` manda a `resolve_conflict` | **Cierta y no funciona con un reemplazante borrado.** El texto dice «until resolve_conflict keeps it» (`crates/clawg-cli/src/mcp/tools.rs:206-215`) y `read_pair_row` no ve la fila borrada. `clawg undelete` dice lo mismo sin la sugerencia (`crates/clawg-cli/src/undelete.rs:9-16`). |
| El detector nunca compara una fila restaurada | **Cierta.** Los pares se escriben en la transacción del insert (`note_candidates`, `store_insert.rs:244-259`, llamada en `:480`), con candidatas que sólo vienen de `search_all_memories`, que excluye lo borrado (`search.rs:210`). `restore_memory_core` no llama al detector y no escribe pares (`admin.rs:487-518`). Una fila C guardada mientras A estaba borrada no se empareja con A al volver. |
| `format_restore` no muestra `topic=K` | **Cierta.** `format_hits`, `format_save` y `format_rows` sí (`tools.rs:110`, `:180`, `:251`); `Restored` ni siquiera lleva la clave (`client.rs:410-415`). |

Fuera de alcance: `RESTORE_TOPIC_HOLDER` cuenta como dueña de la clave a una pieza hermana del mismo save con clave. Se arregla dentro de 18. Importa acá porque cualquier regla nueva sobre «quién tiene la clave» se apoya en esa consulta.

## Problema (verificado contra el código)

- **Borrar es más destructivo de lo que parece.** El usuario borra «la versión nueva» y el tema entero deja de aparecer en `search_memory` y `recall_context`. La respuesta de `delete_memory` es `deleted 1; restore_memory ... undoes it` (`mcp/server.rs:825`): no dice que había una versión anterior escondida.
- **El callejón.** La vieja no se puede ver en búsquedas, restaurar ni juzgar. Si además se guardó otra versión, ni siquiera deshacer el borrado sirve.
- **Restaurar no compara.** El detector mide cercanía al guardar. Una fila que vuelve de entre los borrados se salta esa comparación, y la contradicción con lo guardado en su ausencia queda sin anotar.

## Lo que ya existe y se reutiliza

- `REVIVE_SQL` (`dispute.rs:262`): deja `active` a una fila `superseded` por `?2`. Hoy la usa `judge_pair`.
- `settle_row` y `settle_challengers` (`dispute.rs:200-209`): llevan al reposo la fila restaurada y a las que la nombran.
- `detect_contradiction` (`contradiction.rs:181`): recibe el vector, el agente, el proyecto, el tipo y una lista `siblings` que deja afuera. `note_candidates` (`store_insert.rs:244`) es `INSERT OR IGNORE`, así que un par ya anotado, con su veredicto, no se pisa.
- `needs_user` (`contradiction.rs:103`): es simétrica en tipos e identificadores, sirve igual con el par al revés.
- `RelatedMemory`, `related_text` y el campo `related` de la respuesta del save (`daemon/api.rs:850`, `mcp/server.rs:325`).
- `RankedHit.created_at` (`search.rs:85`) y `memory.deleted_at`: dan la ventana.
- `revise_in` (`topic.rs:111`) para volver a reemplazar.

## Enfoques posibles

### 1. Qué pasa con la versión anterior al borrar la vigente (decisión abierta)

Alcance común: el mismo agente y proyecto de la clave, que es lo que ya fija `SCOPE_ROWS_SQL` (`topic.rs:83`). Un agente sólo borra filas propias (`admin.rs:326-330`), así que no cruza agentes.

#### A. Revivir la anterior en la transacción del borrado

`status = 'active'`, `superseded_by = NULL` (`REVIVE_SQL`), y `settle_row` para que el estado de reposo sea el correcto.

- **Qué filas.** Las `superseded` con `superseded_by = D` **y la misma `topic_key` que D**. Ninguna columna dice «reemplazada por una revisión» frente a «reemplazada por un veredicto `supersedes`»; la clave igual es el único marcador sin esquema nuevo (`adopt` copia la clave a la cabeza, `topic.rs:145-148`). Una importación (44) tiene que conservar esa igualdad.
- **Sólo si D era la vigente y no queda otra viva con la clave.** Si la cadena es v1←v2←v3 y se borra v2 (ya `superseded`), v1 no se revive: v3 sigue vigente. Si se borra v3, v2 se revive y v1 sigue `superseded by v2`.
- **Varias reemplazadas por D.** Pueden ser varias (duplicados viejos, un save largo anterior con piezas). Se revive la de id mayor y las otras se reapuntan a ella, para que la clave tenga una sola viva. Alternativa: revivirlas todas, y la clave vuelve a tener varias filas vivas.
- **Save largo con piezas.** Borrar es por fila. Borrar sólo la primera pieza de la vigente deja las otras vivas con la clave, así que «no queda otra viva» no se cumple y no se revive. Es un caso raro; queda dicho en la respuesta.
- **Veredicto `supersedes` (no es tema).** El ganador K borrado deja a la perdedora L `superseded by K`. Es una decisión juzgada, no un historial. Recomendación: no revivir, pero decirlo en la respuesta (ver C).
- **Deshacer.** Restaurar D choca con `TopicTaken`, porque la anterior volvió a estar viva. Sin columna que registre «revivida por el borrado de D», dos caminos:
  - **A1 (sin esquema).** Restaurar puede quedarse con la clave cuando la dueña viva tiene id **menor** que la restaurada: los ids de una clave crecen con las versiones, así que la restaurada es la más nueva y `revise_in` reemplaza a la dueña. Con id mayor sigue el 409 de hoy (el caso de la sesión C). Se pierde la distinción «la revivió el borrado» frente a «volvió a estar viva por otro camino»; en la práctica sólo un borrado o un save pueden dejar viva una fila con la clave, y un save nuevo tiene id mayor.
  - **A2 (con esquema).** Una columna `revived_by` en `memory`. Es exacta, pero es una migración V17: un binario anterior rechaza la base con `SchemaNewer` y la vuelta atrás es el snapshot `pre-migration` (`specs/storage`, líneas 233 y 244).
- **A favor:** el tema no desaparece, el estado final es coherente y no hay herramienta nueva.
- **En contra:** borrar resucita algo que el usuario quizá consideraba obsoleto. Si lo que quería era olvidar el tema, ahora tiene que borrar también la anterior. La respuesta del borrado lo nombra para que se vea.

#### B. Dejarla escondida y decirlo

El borrado no toca nada; su respuesta, y las de `restore_memory` y `read_memory` de la vieja, dicen «#69 queda reemplazada por #70, que está borrada».

- **A favor:** el borrado hace exactamente lo pedido. Sin sorpresas, sin esquema.
- **En contra:** el tema sigue invisible para búsquedas y `recall_context`. Hay que dar un camino de vuelta, que hoy no existe (C).

#### C. Camino de vuelta explícito

`read_pair_row` y `judge_pair` aceptan un reemplazante borrado cuando la fila está unida a él por `superseded_by`. El agente revive la vieja con `resolve_conflict(keep = vieja, other = borrada, compatible)`.

- **A favor:** decide una persona o el agente, y sirve también para el caso del veredicto.
- **En contra:** es forzar una herramienta pensada para conflictos. Escribe un par `judged` (`RECORD_PAIR_SQL`, `dispute.rs:281`) entre una fila viva y una borrada. Se vuelve a ver al restaurar la borrada. Y el agente tiene que enterarse para usarla.

**Recomendación (a decidir):** A en su forma A1 para lo que es revisión por clave, B para el veredicto `supersedes`. La respuesta del borrado nombra lo revivido y lo que queda escondido. C queda como lo que se hace con las filas que dejó un binario anterior, en un `maintenance` que aplique la misma regla que el borrado.

### 2. Restaurar una fila cuyo reemplazante está borrado (misma familia)

Con A, una fila nueva en ese estado sólo se da por un veredicto. Se cubren tres casos con la misma regla pura `orphaned_supersession(fila, reemplazante)`, que se usa en el borrado, en el restore y en un barrido de mantenimiento:

- **Restore de la reemplazada** con reemplazante borrado y la clave sin dueña viva: revivir (la fila vuelve vigente).
- **Barrido de mantenimiento** para lo que dejaron versiones anteriores (como `release_orphaned_disputes`, `maintenance.rs:216`). Es el mismo SQL que `doctor-checks` (31) mostraría como evidencia.
- **Veredicto con reemplazante borrado:** `format_restore` y `undelete` dicen la verdad. `Restored` suma `superseded_by_deleted: bool`, con `#[serde(default)]`; el texto deja de mandar a `resolve_conflict` y dice «#D está borrada; restaurá #D, o guardá de nuevo el contenido».

### 3. Redetección al restaurar (decidido en su ventana)

**Decidido.** Al restaurar A se compara con las filas **vivas guardadas mientras A estaba borrada** (`created_at` posterior a su `deleted_at`). Los pares se anotan como los de un save: `pending`, con las reglas de `needs_user`, y la respuesta los devuelve como `related` para que el agente los juzgue. Los pares y veredictos que ya están en `memory_conflict` sobreviven al borrado y al restore y no se vuelven a preguntar.

A explorar, con lo que sugiere el código:

- **Punto de entrada.** `detect_contradiction` tal cual, llamada **antes** de la transacción con el vector que `restore_memory_core` ya calcula (`admin.rs:471-478`). A no es viva todavía, así que el KNN no puede devolverla (no hay autoemparejamiento). Hay que leer también `memory_type`, `project_id` y `deleted_at` en `RESTORE_READ`, que hoy sólo lee `content` y el contexto. Un tipo que no afirma nada (`is_claim_bearing`) no consulta.
- **Ventana.** El detector descarta `created_at`; el filtro `created_at > deleted_at` se aplica **dentro de la transacción**, que además vuelve a comprobar que cada candidata sigue viva y sin par con A (el mismo patrón que `RESTORE_TWIN`). Un `INSERT OR IGNORE` protege de repetir.
- **El pool de 5.** `CANDIDATE_POOL` corta antes de filtrar (`contradiction.rs:197`): las filas cercanas más viejas, que ya tienen su par con A, podrían ocupar los cinco lugares. Se pasan como `siblings` las filas que ya forman un par con A (en cualquier dirección), y el pool crece por su cantidad, como con las hermanas de un save. Las piezas hermanas de A caen por la ventana, no por un par.
- **Dirección del par.** `new_id` = C (guardada después, id mayor) y `old_id` = A. Cumple `new_id > old_id` (`specs/disputes`, «Par») y `judge_pair` ya calcula `(new, old) = (max, min)` (`dispute.rs:323`), así que el agente juzga con el orden que quiera.
- **Quién queda `disputed`.** El que carga `disputed_against` es el **nuevo** del par, es decir C, no la restaurada. Un par escalado deja a C `disputed` y con `disputed_against = [A]`, y `settle_challengers(A)` (`admin.rs:512`) ya lo deja en su estado de reposo. Hace falta un helper que sume la disputa (hoy `add_dispute_sql` es privado de `dispute.rs:269`). A puede quedar `disputed` sólo por pares viejos, como hoy.
- **Regla de las revisiones.** «El detector no empareja un save con lo que reemplaza» sólo se aplica si A reemplaza algo. Con la opción A1 de arriba, restaurar A puede reemplazar a la dueña de la clave, y esa fila va como `siblings`. Sin A1, A no reemplaza nada (el 409 lo impide).
- **Respuesta.** `RestoredBody` suma `related` (`daemon/api.rs:665`), omitido si está vacío, igual que el save. `Restored` lo lee con `#[serde(default)]`: un daemon anterior no lo manda. `format_restore` suma la línea con `#C "excerpt"` y, si escaló, «asks the user». El texto de `related_text` dice «keep = the new id», que acá no vale: la guardada después es C. Se escribe una variante.
- **`clawg undelete`** imprime lo mismo en una línea por candidata y no juzga (`undelete.rs`).
- **Costo.** Un KNN más por restore, sobre un camino que ya paga un embedding. Un fallo del detector se registra y no aborta (`contradiction.rs:199-205`), pero a diferencia de un save, nadie lo reintenta: esos pares no se anotan nunca. Se acepta, igual que en un save.
- **Alternativa descartada.** Hacerlo en el worker de forma asíncrona: el restore responde antes de saber, y se pierde el `related` en la respuesta que el agente lee con la conversación a mano.

### 4. `topic=K` en la respuesta del restore

`Restored` lleva `topic_key` (`RestoredRow` lo lee en la misma sentencia) y `format_restore` lo muestra con el formato de `read_memory`. La descripción de `restore_memory` nombra la gemela y el secreto pero no el rechazo por clave, que hoy llega sólo por el mensaje del daemon; nombrarlo cambia lo que ve el modelo y se bendice en `agent_contract.json`. Sin decisión abierta.

## Descartado

| Opción | Razón técnica |
|---|---|
| Que borrar siga sin tocar nada (el estado actual) | Es el defecto que se describe. |
| Escribir un par al reemplazar por clave | Cambia lo que 27 decidió («el detector no empareja con lo que reemplaza»), llena `memory_conflict` y no resuelve el borrado. |
| Revivir también las reemplazadas por veredicto | Deshace una decisión juzgada por una persona sin que lo pida. |
| Redetectar todo el store en cada restore | Un KNN por fila viva; la ventana ya acota lo que puede haber cambiado. |
| Redetectar con filas anteriores al borrado | Ya tienen su par desde que se guardaron; la ventana decidida las deja afuera. |

## Interacciones

- **recoverable-delete (18), archivado ([`specs/restore`](../../specs/restore/spec.md)).** Se apoya en su `restore_memory_core`. 18 se cerró con la consulta de la dueña de la clave ya arreglada (no cuenta a las piezas hermanas), y su spec deja nombrados como límites conocidos los dos huecos que toma este cambio.
- **conflict-candidates (24) y `specs/disputes`.** R4 («Borrar MUST NOT tocar los pares de X») sigue: el borrado no toca pares, sólo la fila anterior. «Restaurar X ... lleva a su reposo» se amplía con la redetección. Se actualizan R4 y su escenario de la clave.
- **topic-revisions (27) y `specs/ingest`/`specs/storage`.** La unicidad de la clave la mantienen el worker y restore (`specs/storage:231`); este cambio suma el borrado como tercero que la respeta (no deja dos vivas y no deja ninguna cuando había una anterior).
- **conclusion-upkeep (45).** Revivir una fuente la vuelve a cubrir y a hacer pendiente; una conclusión derivada de la borrada se ve «a revisar». 45 calcula al vuelo, no hay estado que limpiar. Su opción B (rechazar fuentes `superseded`) no cambia.
- **memory-portability (43) e history-import (44).** El export tiene que llevar `deleted_at`, `topic_key` y `superseded_by`. La regla A depende de que una importación conserve la clave de la cabeza y de la reemplazada; 44 lo tiene en cuenta (el 75 % de sus notas tienen clave). Un import no corre la redetección.
- **revision-eval (48).** Puede contar, entre sus sesiones, cuántas veces se borra una versión vigente. No depende de este cambio.
- **doctor-checks (31).** Un chequeo «reemplazada por una fila borrada, de la misma clave, sin dueña viva» sale del mismo SQL del barrido. Aporta evidencia, sin ruido.
- **knn-prefilter (37).** Los filtros de agente y proyecto del detector corren después del corte de vec0; la redetección hereda esa limitación. No hay nada que medir hoy.
- **Esquema y `rag-eval`.** A1, B, C y la redetección no necesitan migración. A2 es V17 y deja a un binario anterior sin poder abrir la base. El ranking no cambia, y el golden set no tiene claves ni borrados, así que `rag-eval` no aplica. `conflict-eval` mide el detector sobre saves y tampoco cambia.
- **Docs.** `specs/disputes` (R4), `specs/mcp` (respuesta de `delete_memory` y de `restore_memory`), `.claude/CLAUDE.md` y `agent_contract.json` (se bendice con `CLAWG_BLESS=1`) si cambia el texto del borrado.

## Alcance candidato

1. Regla pura `orphaned_supersession` y su tabla de casos (cadena, varias reemplazadas, piezas, veredicto).
2. El borrado revive a la anterior (A) y devuelve qué revivió; la respuesta de `delete_memory` lo dice.
3. Restore: revive al restaurar, toma la clave si la dueña tiene id menor (A1) y redetecta (3).
4. `RestoredBody` suma `related` y `topic_key`; `Restored`, `format_restore` y `undelete` los muestran.
5. Barrido de mantenimiento para lo que dejaron versiones anteriores.
6. Docs y contrato.

### No-alcance

- Una columna `revived_by` (A2), salvo que la decisión lo pida.
- Revivir por veredicto.
- Cambios de ranking.
- La consulta `RESTORE_TOPIC_HOLDER` con piezas hermanas (la arregla 18).

## Decisiones abiertas

Decididas; el detalle está en la [propuesta](proposal.md#decisiones).

- [x] **Qué pasa con la anterior al borrar la vigente:** revivirla, con registro (A2), y además con la causa de cada reemplazo, para no decidir por la clave ([diseño](design.md#d1-qué-se-registra)).
- [x] **Si la cadena tiene varias reemplazadas por la misma:** revivir el save de id mayor y reapuntar las otras.
- [x] **Un veredicto `supersedes` cuyo ganador se borra:** no revivir; decirlo en la respuesta.
- [x] **Restaurar una vigente con la anterior revivida:** por `revived_from`, con la migración V17.
- [x] **Barrido de mantenimiento** para las filas que dejó un binario anterior, con la misma regla.
- [x] **Redetección al restaurar:** sólo las vivas guardadas mientras estuvo borrada, como candidatas `pending` con las reglas de `needs_user`, devueltas en `related`.
- [x] **La dirección del par y quién queda `disputed`:** C (la nueva) como `new_id` y como la que carga la disputa.

## Cómo se mide

Tests RED primero, en `admin.rs` y `crates/clawg-cli/tests/api.rs`:

1. Con v1 reemplazada por v2, borrar v2 deja v1 viva y la búsqueda la encuentra. Hoy no.
2. Cadena v1←v2←v3: borrar v3 revive v2 y deja v1 reemplazada por v2; borrar v2 (ya reemplazada) no revive v1.
3. Un veredicto `supersedes`: borrar al ganador no revive a la perdedora, y la respuesta lo dice.
4. Restaurar v2 con v1 revivida reemplaza a v1; con una v3 guardada después sigue el 409.
5. Restaurar una fila reemplazada por una borrada: nunca queda `superseded` sin salida.
6. Restaurar A con una C cercana guardada durante el borrado anota el par (`new_id = C`, `old_id = A`), lo devuelve en `related`, y un par escalado deja a C `disputed` contra A.
7. Una C guardada **antes** de borrar A, o ya emparejada o juzgada, no se vuelve a anotar ni a preguntar.
8. Otro agente no ve ni revive las filas ajenas.

`rag-eval` no aplica: no cambia el ranking. Windows: el suite corre en `target-test`.

## Riesgos

- **Sorpresa al borrar.** Se mitiga con la respuesta que nombra lo revivido.
- **A1 por orden de id** asume que una clave crece con sus versiones. Una importación (44) que reasigne ids debe conservar el orden.
- **Ruido de la redetección.** Una restauración de una fila vieja en un proyecto activo puede traer varias candidatas. Se acota con el pool del detector y se mide con `conflict-eval` si hace falta.
- **El detector falla en silencio** (se registra, no se reintenta): los pares de ese restore no se anotan.
