# topic-revisions — Diseño

Las alternativas de fondo (E1 a E3b) están en la [exploración](exploration.md#enfoques-posibles). Aquí van las decisiones de implementación.

## D1. Dónde se reemplaza: al terminar el save

**Elegido:** en la transacción que termina el save, la misma que enlaza sus piezas y, con outbox, borra su fila (`outbox_finish`, `crates/clawg-core/src/memory/outbox.rs:329-341`). Sin outbox, el enlace de `link_save` (`worker.rs:806-814`) pasa a hacer lo mismo en una transacción.

| Alternativa | Por qué no |
|---|---|
| En la transacción de la primera pieza | Un save que falla en la tercera pieza dejaría la versión vieja reemplazada y la nueva a medias: lo vivo pierde texto |
| En una transacción propia después de terminar | Un corte entre las dos deja dos versiones vivas sin outbox que lo arregle |

Un corte antes de terminar deja la outbox con todas las piezas anotadas; el arranque siguiente reanuda, no inserta nada y termina, reemplazando entonces. Un save que falla queda con las dos versiones vivas, como hoy cualquier save a medias.

## D2. Qué se reemplaza

```
replaced = filas vivas del alcance (agent_id, COALESCE(project_id,''), topic_key)
           que no son piezas de este save
shared   = las de replaced que memory_piece enlaza a un save
           cuya primera pieza es una fila viva con otra clave o sin clave
```

La primera pieza dice de quién es el save: un save con clave se la da a todas sus filas (abajo), así que su primera pieza la lleva aunque la dedup le haya sumado una fila de otra clave en el medio. Mirar todas las filas del save, en vez de la primera, desengancharía la versión vieja entera en ese caso.

`replaced − shared` pasa a `superseded` por la primera fila del save (`SUPERSEDE_SQL` de `dispute.rs`, más `release_challengers`); `shared` queda viva con `topic_key = NULL`. El índice parcial de V16 sirve a la primera consulta.

Las filas del save sin clave la toman (`UPDATE … SET topic_key = ? WHERE id IN (…) AND topic_key IS NULL`), así que un save absorbido por la dedup en una fila sin clave deja la clave en esa fila. Una fila con otra clave la conserva.

## D3. Herencia

- **Tipo e importancia:** antes de construir los registros, el worker lee la fila viva más antigua del alcance (`ORDER BY id LIMIT 1`) y completa en una copia del evento lo que el save no trae. En una reposición la versión vieja sigue viva, porque el reemplazo es lo último (D1), así que la lectura da lo mismo.
- **Tier y accesos:** en la transacción de D2, sobre las filas que el save insertó: `tier = MIN(tier, mejor tier reemplazado)` y `access_count = MAX(access_count, mayor reemplazado)`. El tier sólo sube (`tier.rs`), así que tomar el mejor no rompe esa regla.

## D4. Detector

Las filas vivas del alcance, leídas en D3, se suman a `siblings` (`worker.rs:747-748`): `detect_contradiction` ya deja fuera esos ids. Un par entre una versión y la siguiente nacería pendiente y quedaría muerto en la misma transacción.

## D5. El informe y la respuesta

`IngestReport` suma `replaced: Vec<Replaced>` (id y `excerpt`, la primera línea que ya usan las candidatas, `contradiction.rs:121`). `ChunkOutcome` no cambia: un puente anterior sigue leyendo `inserted` y `reinforced`. `SaveBody` suma `replaced` (omitido vacío) y `topic_key` (omitido sin clave). Un daemon anterior ignora el campo `topic_key` del pedido (`SaveRequest` no niega campos desconocidos, `crates/clawg-cli/src/daemon/api.rs:328-346`); el puente lo detecta porque la respuesta no lo trae.

## D6. Unicidad

Sin índice único: las piezas de un save comparten la clave. El worker es el único escritor de saves; `restore` comprueba en su transacción, como ya hace con el gemelo de la dedup (`RESTORE_TWIN`, `crates/clawg-core/src/memory/admin.rs:400-405`).

## D7. Validación

`topic.rs` en `clawg-core/src/memory`: `validate_topic_key(&str) -> Result<(), TopicKeyError>` pura, con `thiserror`. El daemon la llama antes de la outbox; el worker no la repite, porque un evento de la outbox ya pasó por ella.
