# revision-aware-delete — Diseño

Las opciones de fondo (A, B, C) están en la [exploración](exploration.md#enfoques-posibles). Acá van las decisiones de implementación.

## D1. Qué se registra

Hacen falta dos hechos que hoy no están en ninguna columna: **por qué** una fila quedó `superseded` (para saber si un borrado la revive) y **quién** la revivió (para que restaurar la borrada lo deshaga exacto).

| Alternativa | Por qué no |
|---|---|
| A1: deducir por la clave y el orden de ids, sin esquema | Adivina las dos cosas. La clave igual no distingue una revisión de un veredicto que la deshizo (`check_pair` acepta dos filas unidas por `superseded_by`, `dispute.rs:135-155`): borrar v1 reviviría a v2, la perdedora juzgada. Y el orden de ids no distingue «la revivió el borrado» de «volvió a estar viva por otro camino». |
| Sólo `revived_by` (A2 de la exploración) | Deshace exacto, pero sigue decidiendo qué revivir por la clave: el mismo error de arriba. |
| Calcular al leer que una fila cuyo reemplazante está borrado está vigente, sin escribir nada al borrar | Deshacer sería gratis, pero cada lector del estado (KNN, texto, síntesis, pares, `SCOPE_ROWS_SQL`, `settle_sql`) tendría que cambiar de filtro, y una cadena de borradas pide recursión. Es el cambio con más superficie y más riesgo para las lecturas. |
| Una tabla de historial de reemplazos | Equivale a las dos columnas con un join en cada consulta; `superseded_by` y `disputed_against` ya viven en `memory`, y no hay otro uso que la pida. |

**Elegido:** V17 agrega a `memory`:

- `supersede_cause TEXT CHECK (supersede_cause IN ('revision', 'verdict'))`: se escribe junto con `superseded_by`. `NULL` en una fila que no está reemplazada.
- `revived_from INTEGER`: el id de la fila borrada cuyo borrado revivió a esta. `NULL` en cualquier otra. Índice parcial `idx_memory_revived` sobre las no nulas.

Los dos escritores de `superseded` son `revise_in` (`topic.rs:131`, causa `revision`) y `judge_pair` (`dispute.rs:354`, causa `verdict`); `SUPERSEDE_SQL` toma la causa como parámetro. `REVIVE_SQL` borra `supersede_cause`. Volver a reemplazar una fila revivida borra su `revived_from`.

## D2. La causa de las filas que ya existen

La migración completa `supersede_cause` de cada fila `superseded`, para que ninguna regla tenga que adivinar en tiempo de ejecución:

- `revision` si su reemplazante es del mismo agente, del mismo proyecto (`COALESCE`), tiene la misma `topic_key` no nula, y ningún par de `memory_conflict` entre las dos tiene `verdict = 'supersedes'`;
- `verdict` en cualquier otro caso. Antes de V16 sólo un veredicto reemplazaba, y desde V16 una revisión nunca escribe par (`dispute.rs:265`), así que un par `supersedes` juzgado sólo puede venir de un veredicto.

Ninguna otra columna cambia.

## D3. La regla

`revival.rs`, pura:

```
plan_revival(was_current, key_held, replaced: [(id, cause, save)]) -> { revive, repoint, hidden }
hidden  = las de causa verdict
si !was_current o key_held: revive = repoint = []
si no:  top = la de causa revision con id mayor
        revive  = las de causa revision del mismo save que top (sólo top si no tiene save)
        repoint = las demás de causa revision, a la primera de revive
```

- `was_current`: la borrada estaba `active` o `disputed`. El borrado no toca `status`, así que se lee igual en el borrado, en el restore y en el barrido.
- `key_held`: queda otra fila viva del alcance de la clave (`SCOPE_ROWS_SQL`) que no está en `revive`. Una pieza viva del mismo save de la borrada la cuenta, así que borrar sólo la primera pieza de un save largo no revive nada.
- `save`: el `save_id` de `memory_piece`. Revivir por save mantiene enteras las piezas de un save largo, que una revisión reemplaza juntas.

Revivir es `REVIVE_SQL` + `revived_from = D`, y después `settle_row` y `settle_challengers` (las retadoras que su reemplazo había liberado vuelven a disputarla). Reapuntar sólo cambia `superseded_by`.

## D4. Dónde corre

`revive_after_delete(c, d, now)` aplica la regla a la fila borrada `d` dentro de la transacción del llamador:

1. **El borrado**, cuando la UPDATE afectó la fila (un segundo borrado no revive). Si la fila es una pieza de un save cuya primera pieza ya está borrada, la regla corre también para esa primera pieza: las filas que un save largo reemplazó apuntan a ella, y borrarla antes que las otras no revive nada mientras ellas tengan la clave. Lo revivido así queda con `revived_from` = la primera pieza, y es restaurarla a ella lo que lo vuelve a reemplazar; restaurar antes otra pieza da `TopicTaken` con la revivida.
2. **El restore de una fila reemplazada por `d`**, ya borrada: la fila vuelve viva y `superseded`, y la regla decide si revive (por revisión, sin dueña viva de la clave) o queda escondida (`superseded_by_deleted`).
3. **El barrido de mantenimiento**, para cada borrada que está `active` o `disputed` y a la que todavía apunta una fila viva de causa `revision`. Es idempotente: después de revivir no queda ninguna.

## D5. Restaurar la borrada

Dentro de la transacción de `restore_memory_core`:

1. La consulta de la dueña de la clave (`RESTORE_TOPIC_HOLDER`) deja afuera las filas con `revived_from = id`.
2. Después de reponer fila, vector e índice de texto, si la fila no está `superseded`, cada fila viva con `revived_from = id` vuelve a quedar `superseded` por ella con causa `revision` y sin `revived_from`, y se liberan sus retadoras. La respuesta las lista en `replaced`.
3. Las filas que el borrado había reapuntado quedan apuntando a la revivida, que ahora está reemplazada por la restaurada: la cadena es coherente y ningún lector distingue la diferencia.

## D6. Redetección

- **Antes de la transacción**, con el vector que el restore ya calcula: `detect_contradiction` con el tipo, agente y proyecto de la fila (que `RESTORE_READ` ahora lee), y como `siblings` las filas que ya forman un par con ella y las que va a volver a reemplazar (D5). El pool crece por su cantidad.
- **Dentro**, cada candidata se escribe sólo si sigue viva, no está `superseded` y su `created_at` es posterior al `deleted_at` de la restaurada; con `INSERT OR IGNORE`, un par que ya existe no se pisa. El par es `(new_id = C, old_id = A)`. Si escala, `A` se suma al `disputed_against` de `C` (`add_dispute_in`, hasta ahora privado de `dispute.rs`) y `settle_challengers(A)` deja a `C` `disputed`.
- **Si la restaurada vuelve `superseded`**, no se escribe ningún par: no está en ninguna lectura.
- Un fallo del detector se registra y no aborta, como en un save.

## D7. Respuestas

- `DeletedBody`: `deleted`, `revived` (id y primera línea, omitido vacío) y `hidden` (ids que siguen reemplazados por la borrada por un veredicto, omitido vacío). `DaemonClient::delete` devuelve `Deleted` con los dos con `#[serde(default)]`: un daemon anterior no los manda.
- `RestoredRow`: suma `topic_key`, `replaced`, `superseded_by_deleted` y `related` (los tres últimos omitidos vacíos o falsos). `Restored` los lee con `#[serde(default)]`.
- `format_restore` deja de mandar a `resolve_conflict` cuando el reemplazante está borrado: «#D is deleted; restore_memory #D, or save the content again». Los pares nuevos van como `#C "excerpt"`, diciendo que la guardada después es `keep` para un `supersedes`.
