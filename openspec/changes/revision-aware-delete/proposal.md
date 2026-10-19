# revision-aware-delete — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | recoverable-delete, conflict-candidates y topic-revisions (archivados) | ver [`state.yaml`](state.yaml) |

## Problema

Borrar y restaurar se escribieron antes que los pares (24) y las revisiones por clave (27), y siguen tratando cada fila como independiente. Detalle verificado en la [exploración](exploration.md#premisas-que-se-verificaron):

1. **Borrar la versión vigente de un tema esconde el tema entero.** `delete_memory_core` sólo estampa `deleted_at`, borra el vector y libera a las retadoras (`crates/clawg-core/src/memory/admin.rs:331-338`); la versión anterior sigue `superseded` por una fila borrada, y el KNN, el índice de texto y la síntesis dejan afuera lo `superseded`.
2. **El callejón.** Esa versión anterior no se puede restaurar (no está borrada), ni juzgar (`SELECT_PAIR_ROW` no ve la borrada, `dispute.rs:218`), y si se guardó otra versión de la clave, restaurar la borrada da `TopicTaken`.
3. **Restaurar no compara.** El detector corre en la transacción del insert (`store_insert.rs:484`); `restore_memory_core` no lo llama, así que lo guardado mientras la fila estuvo borrada nunca se empareja con ella.
4. **La causa del reemplazo no se guarda.** Una fila `superseded` puede venir de una revisión por clave (`topic.rs:131`) o de un veredicto `supersedes` (`dispute.rs:354`). Lo único que hoy las distingue es que la clave sea la misma, y no alcanza: deshacer una revisión con `resolve_conflict(keep = v1, other = v2, supersedes)` (`check_pair` lo acepta por `superseded_by`, `dispute.rs:135-155`) deja a v2 reemplazada **por veredicto** y con la misma clave que v1.

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-abiertas):

- **Borrar la vigente revive a la anterior, con registro (A2).** En la transacción del borrado, la versión que la vigente reemplazó por revisión vuelve a estar viva y queda anotada como revivida por la borrada. Restaurar la borrada la vuelve a reemplazar, exacto. No se elige A1 (deducir por orden de id): adivina lo que se puede registrar.
- **La causa del reemplazo se registra.** Además de `revived_from`, V17 agrega `supersede_cause` (`revision` | `verdict`), que se escribe donde se reemplaza. La migración la completa de forma determinista para las filas que ya existen, así que ninguna regla en tiempo de ejecución adivina por la clave. Alternativas en el [diseño](design.md#d1-qué-se-registra).
- **Varias reemplazadas por la misma:** se revive el save de id mayor (todas sus piezas) y las demás se reapuntan a su primera fila, para que la clave tenga un solo save vivo.
- **Un veredicto `supersedes` cuyo ganador se borra:** la perdedora no revive; la respuesta del borrado y la del restore lo dicen.
- **Barrido de mantenimiento** para lo que dejó un binario anterior, con la misma regla.
- **Redetección al restaurar** (ya decidida): sólo contra las filas vivas guardadas mientras estuvo borrada; la guardada después (C) es `new_id` del par y la que carga la disputa si escala.

## Alcance

1. V17: `memory.supersede_cause` y `memory.revived_from`, con la causa completada para las filas `superseded`.
2. `revise_in` y `judge_pair` escriben la causa; revivir la borra.
3. Regla pura `plan_revival` y su tabla de casos.
4. El borrado revive (`DeletedBody.revived`) y nombra lo que queda escondido (`DeletedBody.hidden`); `delete_memory` lo dice.
5. Restore: vuelve a reemplazar lo que su borrado revivió, revive a una fila cuyo reemplazante por revisión está borrado, y redetecta. `RestoredBody` suma `topic_key`, `replaced`, `superseded_by_deleted` y `related`; `restore_memory` y `clawg undelete` lo muestran.
6. Barrido en `run_maintenance_once_at`.
7. Descripciones de `delete_memory` y `restore_memory` (contrato bendecido), `.claude/CLAUDE.md` y las specs `restore`, `disputes`, `storage`, `ingest` y `mcp`.

## Fuera de alcance

- Revivir por veredicto.
- Cambios de ranking o del detector.
- Que restaurar una fila `superseded` de un tema vivo deje de dar `TopicTaken` (comportamiento de 27 que no cambia).
- Un save largo cuya primera pieza se borra con las demás vivas: no revive nada (la clave sigue viva en las otras piezas).

## Datos guardados

- **Esquema:** dos columnas nullable y un índice parcial en `memory`. Antes de migrar se escribe la copia `pre-migration`.
- **Migración:** sólo escribe `supersede_cause` en las filas `superseded`: `revision` cuando su reemplazante es del mismo agente, proyecto y clave y ningún par `supersedes` juzgado une a las dos; `verdict` en cualquier otro caso (antes de V16 sólo un veredicto reemplazaba). Ninguna otra columna cambia.
- **Rollback:** un binario anterior rechaza la base en V17 sin tocarla (`SchemaNewer`). Se vuelve con `clawg restore` del snapshot `pre-migration`, o con `DROP INDEX idx_memory_revived; ALTER TABLE memory DROP COLUMN revived_from; ALTER TABLE memory DROP COLUMN supersede_cause; DELETE FROM refinery_schema_history WHERE version = 17;`. Una fila revivida por un borrado queda viva, que es lo que un binario anterior ve de cualquier fila viva.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)): los ocho de la [exploración](exploration.md#cómo-se-mide), más la base sembrada en V16 que abre con la causa completada y el resto igual, y el reemplazo deshecho por veredicto que no revive al borrar. `rag-eval` no aplica: el ranking no cambia y el golden set no tiene claves ni borrados.
