# conclusion-upkeep — Diseño

Las opciones de fondo (A a D) están en la [exploración](exploration.md#enfoques-posibles). Acá van las decisiones de implementación.

## D1. Dónde se calcula

La lista se calcula al leer, en la misma transacción de `synthesis_core`, sin columna ni tabla nueva. Restaurar una fuente o revivir una versión la saca de la lista sin que nadie limpie estado.

| Alternativa | Por qué no |
|---|---|
| Marcar la conclusión al borrar, reemplazar o purgar una fuente | Cuatro escritores (`delete_memory_core`, `revise_in`, `judge_pair`, `purge`) y sus inversos (restore, revival) tendrían que mantener la marca; una marca vieja miente. |
| Calcularlo en el puente con `derived_from` y un `GET` por fuente | Un pedido por fuente, y el estado de otra fila no es del puente. |

La condición SQL de «fuente perdida» es una sola constante (`LOST_SOURCE`): no existe una fila viva con ese id. La pura `SourceState::of` clasifica cada fuente de las conclusiones que entraron, para el texto; los tests de `synthesis.rs` cubren que las dos digan lo mismo.

## D2. Respuesta

`Synthesis` suma:

- `to_review: Vec<ConclusionReview>`, hasta `REVIEW_LIMIT = 10`, la más nueva primero. `ConclusionReview { conclusion: MemoryAdminRow, live: usize, lost: Vec<LostSource> }`, `LostSource { id, state, superseded_by }` (`superseded_by` omitido si no hay).
- `to_review_total: usize`.

El tope es la mitad de `CONCLUSIONS_LIMIT`: cada línea a revisar es más larga, y diez ya son una sesión de trabajo. El cliente los lee con `#[serde(default)]`: un daemon anterior no los manda.

## D3. El aviso

Un aviso que cuenta todas las conclusiones a revisar no se apaga nunca si el agente decide que una sigue valiendo. Se cuentan sólo las pérdidas posteriores a la última revisión del alcance (`synthesis_review.reviewed_at`), igual que las memorias sin revisar:

- borrada: `deleted_at > reviewed_at`;
- `superseded`: `last_modified_at > reviewed_at` (`SUPERSEDE_SQL` lo escribe al reemplazar, `crates/clawg-core/src/memory/dispute.rs:268-270`);
- purgada: sólo cuenta si el alcance nunca se revisó. No queda cuándo se purgó, y `purge` es una operación a mano del dueño de la base.

| Alternativa | Por qué no |
|---|---|
| Sumarlas al conteo de memorias sin revisar (umbral 20) | Una conclusión que perdió su base pesa más que una memoria nueva; con el umbral de 20 no avisaría casi nunca. |
| Línea propia contando todas las a revisar | No se apaga si la conclusión sigue valiendo. |
| Una columna «revisada» por conclusión | Esquema nuevo para lo que el `reviewed_at` del alcance ya resuelve. |

El umbral es 1: una conclusión sin base es más rara y más cara que una memoria sin revisar. `ContextBody.conclusions_to_review`; el conteo que falla se registra y da 0, como `unreviewed`.

## D4. B: fuentes reemplazadas

`replaced_memory_ids(store, ids, agent) -> Vec<(i64, Option<i64>)>`: las propias no borradas con `status = 'superseded'`, con su `superseded_by`. `save_memory` del daemon la llama después de rechazar las que faltan, así que un id ausente sigue respondiendo como hoy. Se valida antes de encolar, como lo demás de `derived_from`; una fila de la cola escrita antes sigue enlazando lo que nombra.

## D5. C: purge

`DELETE FROM memory_derivation WHERE memory_id = ?1`. El comentario actual dice que el id se puede dar a otra fila; no es así (`AUTOINCREMENT`, y `VACUUM` conserva `sqlite_sequence`), y el comentario nuevo dice por qué el vínculo de la fuente se queda: es lo único que le dice a la conclusión que perdió una fuente.

## D6. Texto

```
Conclusions to review (2 of 2, newest first): they rest on memories deleted, replaced or purged since they were drawn. …guía…
#12 user_preference origin=conclusion live=1/3 lost=#4 deleted,#5 replaced by #9,#6 purged The user prefers tabs
```

Las conclusiones de este bloque se sacan de «Conclusions already drawn». Sin pendientes, el texto sigue empezando con `no memories are waiting for a conclusion`.
