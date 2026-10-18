# conflict-candidates — Diseño

Las alternativas de fondo (A a E) están en la [exploración](exploration.md#enfoques-posibles). Aquí van las decisiones de implementación.

## D1. Escalado ⇔ `disputed_against`

**Elegido:** el detector anota **todas** sus candidatas en `memory_conflict`, pero en `disputed_against` sólo escribe las escaladas. Un par `escalated` es exactamente un id en el `disputed_against` de la fila más nueva. Por eso `REST_STATUS`, `release_challengers`, el mantenimiento, `fetch_active_conflicts` y el filtro `status=disputed` siguen igual: ya leen «los escalados».

| Alternativa | Por qué no |
|---|---|
| Que la tabla sea la única fuente y `memory.status` se calcule desde ella | Cambia todas las lecturas de `disputed` y deja a un binario anterior leyendo filas sin aviso |
| Que la migración vuelva las filas a `active` (como proponía la exploración) | Esconde a la versión anterior las disputas que hoy ve, y obliga a reescribir el bloque de conflictos |
| Guardar el veredicto en el JSON de `disputed_against` (D de la exploración) | Una versión anterior leería objetos como ids inexistentes |

## D2. La tabla

```sql
memory_conflict(new_id, old_id, distance REAL NULL, state, verdict NULL, reason NULL, created_at, judged_at NULL)
PRIMARY KEY (new_id, old_id) WITHOUT ROWID
```

`new_id > old_id` siempre: el detector compara una fila nueva con filas que ya estaban, y `memory.id` es `AUTOINCREMENT`. Un veredicto sobre un par sin fila (dos filas enlazadas sólo por `superseded_by`, de antes de la tabla) lo crea con `new_id = max`. `state` ∈ `pending | escalated | judged`, y `verdict` ∈ `supersedes | compatible | scoped` sólo cuando el estado es `judged`. Sin claves foráneas, igual que `memory_piece`: las filas de `memory` no se borran salvo con purge, que borra también sus pares.

## D3. `needs_user`

Función pura en `contradiction.rs`:

```
needs_user(new_type, old_type, distance: Option<f64>, shares_identifier) -> bool
  = distance is None
    || (distance < ESCALATE_BELOW && (protected(new_type) || protected(old_type) || shares_identifier))
```

`protected` es `user_preference` o `architectural_decision`. `ESCALATE_BELOW = 0,40` sale de la calibración de `contradiction.rs` (contradicciones hasta 0,350, compatibles desde 0,469) y se recalibra con `conflict-eval`. `shares_identifier` usa `identifier_tokens` (`search.rs`) sobre los dos textos. Dos `semantic_fact` sin un identificador en común no escalan: los ve el agente en la respuesta del save y en `memory_status`.

Sin confianza en el veredicto: el agente que no está seguro usa `conflicts`, que es un argumento menos en cada sesión.

## D4. Colapso por save

Dentro de un pedazo, el detector se queda con la candidata más cercana de cada save (clave `memory_piece.save_id`, o el id de la fila si no es un pedazo). Entre los pedazos de un mismo save nuevo, gana el primero que encuentra a ese save: el worker lleva el conjunto de saves ya emparejados. Elegir el pedazo más cercano de todo el save obligaría a reescribir un par ya escrito en la transacción de otro pedazo.

## D5. Veredictos en el núcleo

`judge_pair(store, keep, other, verdict, reason, owner)` corre en una transacción sobre la lógica de `resolve_dispute_core`, que pasa a ser `judge_pair` con `supersedes` o `compatible`:

| Veredicto | Filas | Par |
|---|---|---|
| `supersedes` | `other` queda `superseded` por `keep` (R1) | `judged` |
| `compatible`, `scoped` | las dos en reposo (R2) | `judged`, con motivo |
| `conflicts` | la nueva suma a la vieja en `disputed_against` y queda `disputed` | `escalated` |

`check_pair` también enlaza dos filas que tienen un par en la tabla. `scoped` sin motivo es un 400. Un `supersedes` sobre un par `pending` en el que algún lado es protegido es un 409 que pide `conflicts`.

## D6. Contrato HTTP entre versiones

- `POST /v1/disputes/resolve` se queda como está (`keep_both`), para los puentes anteriores, y ahora guarda el veredicto.
- `POST /v1/disputes/judge` es la ruta nueva, con `verdict` y `reason`. Un puente nuevo contra un daemon anterior recibe 404, y el puente le dice al modelo que reinicie el daemon, como ya hace con `search_project`. Mandar `verdict` por la ruta vieja no sirve: un daemon anterior ignoraría el campo y trataría `conflicts` como «una gana».
- `GET /v1/disputes?agent_id=` devuelve el conteo de pendientes y escalados y hasta 5 pares pendientes, los más viejos primero. `memory_status` omite esa línea si el daemon contesta 404.
- `related` en la respuesta del save es opcional, con `skip_serializing_if`, y el puente lo lee con `#[serde(default)]`.

## D7. Undelete y purge

Restaurar una fila vuelve a asentar las filas cuyo `disputed_against` la nombra, así que el par escalado vuelve con ella (cambia R4). Purge borra los pares que nombran la fila. El mantenimiento borra los pares de una fila que ya no existe, que sólo puede dejar un purge de una versión anterior.
