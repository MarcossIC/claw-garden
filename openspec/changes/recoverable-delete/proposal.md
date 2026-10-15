# recoverable-delete — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (todo lo previo está archivado; el scope sale de visibility-policy y agent-sharing) | ver [`state.yaml`](state.yaml) |

## Problema

Borrar una memoria es el único error que hoy no tiene vuelta atrás. Detalle en la [exploración](exploration.md), revisada contra `main` (9aa1012) el 2026-10-15 ([qué cambió](exploration.md#revisión-contra-main-2026-10-15)):

1. **El borrado deja la fila sin vector y sin índice léxico.** `delete_memory_core` marca `deleted_at` y borra el vector en la misma transacción (`crates/clawg-core/src/memory/admin.rs:300-348`). El trigger de V8 resta sus términos del FTS en la transición viva → borrada (`crates/clawg-core/migrations/V8__memory_fts.sql:68-78`). El texto, la importancia, el tier, los contadores, la procedencia y el vínculo de pedazo quedan.
2. **Nada la vuelve a poner.** No hay función que restaure (`rg restore crates/clawg-core/src/memory` sólo encuentra el restore de snapshots). `update_memory` contesta 404 a una borrada (`admin.rs:189`). Re-guardar el mismo texto no la resucita: el dedup excluye las borradas (`crates/clawg-core/src/memory/store_insert.rs:236-243`) y crea otra fila, con otro id y sin su historia. Poner `deleted_at = NULL` a mano deja la fila sin vector ni FTS: la búsqueda no la encuentra, y un segundo borrado restaría del FTS términos que nunca se sumaron (`V8__memory_fts.sql:46-51`).
3. **`clawg restore` no sirve para una fila.** Reemplaza toda la base por un snapshot y pierde lo guardado después ([`specs/storage`](../../specs/storage/spec.md)).
4. **No se encuentra una borrada entre sesiones.** `list_memories` y `GET /v1/memories` filtran siempre `deleted_at IS NULL` (`admin.rs:633, 697, 752`). `GET /v1/memories/{kind}/{id}` sí la devuelve, pero hay que tener el id.

## Decisiones (cierran las abiertas de la exploración)

Tarjeta en el hilo para la superficie; el resto son los valores por defecto que la exploración recomendaba o que siguen la regla de cambios anteriores. Mientras Marcos no elija otra cosa, se sigue con éstos.

- **Superficie: el agente y el usuario.** Herramienta MCP `restore_memory` (la duodécima) con el agente que llama, más `clawg undelete <ID>` y `clawg memories list --deleted` para el usuario. El caso común es "me equivoqué, deshacelo" en la misma conversación, y el agente tiene el id: lo acaba de pasar a `delete_memory`. Restaurar se deshace volviendo a borrar, así que no necesita la barrera de `purge` (sólo CLI, con el daemon parado).
- **Sólo lo propio.** Con scope, una fila de otro agente es "no existe", como `delete_memory` ([`specs/visibility`](../../specs/visibility/spec.md) R3 y R4; [`specs/sharing`](../../specs/sharing/spec.md) R5: lo compartido es de sólo lectura). `clawg undelete` es del usuario y no lleva scope.
- **Sólo la tabla `memory`.** Es la única que se escribe en producción: `insert_episodic_trajectory` y `promote_to_procedural_skill` sólo se llaman desde tests (`rg` sobre `crates`). `episodic` y `procedural` no tienen FTS, ni dedup, ni pedazos; restaurarlas sumaría dos textos de embedding y sus tests sin un caso real. Un `kind` distinto de `memory` es 400 con un mensaje que lo dice.
- **El FTS se reinserta en la transacción del restore, sin migración.** Un trigger de la transición borrada → viva (V13) cubriría cualquier camino futuro, pero un binario anterior rechaza una base de una versión más nueva (`specs/storage`): volver atrás obligaría a restaurar el snapshot pre-migración y perder lo guardado después. Con la reinserción manual no hay migración y el rollback es cambiar de binario. El invariante de V8 (cada fila viva sumada exactamente una vez) se mantiene porque la reinserción corre sólo en la transición borrada → viva, en la misma transacción.
- **Dedup: no restaura si ya hay una viva igual.** Si se guardó el mismo texto después del borrado, hay una fila viva con el mismo `(agent, proyecto, hash)`. Restaurar daría dos filas vivas iguales, justo lo que el dedup evita. Responde 409 nombrando la viva. Fundir (sumar contadores a la viva) cambia filas que nadie pidió tocar.
- **Contradicciones: no se re-detectan.** Es la regla de memory-update-tool: el detector corre al guardar. La fila restaurada recalcula su propio estado de reposo (`disputed` sólo si nombra una fila viva) en la misma transacción. Las disputas que se liberaron al borrar siguen liberadas ([`specs/disputes`](../../specs/disputes/spec.md) R4).
- **Un secreto la deja borrada.** Una fila anterior al enmascarado puede tener un secreto en claro. Borrada no se busca ni se comparte; viva viaja a otros agentes por la búsqueda (sharing por defecto). Si `redact` encuentra un secreto de formato conocido en el texto, el restore se rechaza, nombra los tipos y apunta a `clawg purge`. La fila sigue borrada y `clawg secrets` la sigue listando.
- **`superseded` se restaura y se avisa.** `resolve_conflict` sí escribe `superseded` (`crates/clawg-core/src/memory/dispute.rs:192`). La fila vuelve con su estado, y la respuesta dice que sigue reemplazada por `#N` y que la búsqueda no la muestra; `read_memory` y `list_memories` sí.
- **Recencia y tier: se conservan.** Restaurar no es un acceso: no suma `access_count` ni mueve `last_accessed_at` (la regla de retrieval-hygiene es que sólo cuenta lo que el agente lee). Sí escribe `last_modified_at` y suma 1 a `update_count`, como `update_memory`. No toca el ranking, así que `rag-eval --diff` no es requisito.
- **Un pedazo se restaura solo, con su contexto.** `delete_memory` borra una fila, y un save largo es una fila por pedazo ([`specs/pieces`](../../specs/pieces/spec.md)). Restaurar un pedazo lo embebe detrás de la cabecera guardada en su vínculo, como el re-embed (`crates/clawg-core/src/memory/maintenance.rs:635-652`), y `read_memory` deja de nombrarlo como faltante.
- **Descubrimiento por MCP: no.** `list_memories` no aprende `deleted`. El agente restaura ids que ya tiene; el usuario lista las borradas con `clawg memories list --deleted`.
- **Sin papelera con retención.** Una borrada se queda hasta `purge`, como hoy. Vaciar sola la papelera borraría datos del usuario sin que lo pida.
- **Borrar un proyecto entero: otro cambio, después de éste.** Necesita esto para poder deshacerse.

## Alcance

1. **Core** (`admin.rs`): `restore_memory_core(store, embedder, id, owner)`. Calcula el vector fuera de la transacción y, dentro, vuelve a leer la fila, chequea dedup, pone `deleted_at = NULL`, reinserta vector y FTS, recalcula su reposo, escribe `last_modified_at` y `update_count`. Errores tipados para "no existe", duplicado, secreto y embedder. El listado acepta `deleted`.
2. **HTTP**: `POST /v1/memories/{kind}/{id}/restore?agent=`, y `GET /v1/memories?deleted=true` ordenado por `deleted_at` descendente.
3. **MCP**: herramienta `restore_memory(kind, id)` con el agente que llama. La respuesta y la descripción de `delete_memory` la nombran.
4. **CLI**: `clawg undelete <ID> [--profile P]` por el daemon, y `clawg memories list --deleted`.
5. **Docs**: `.claude/CLAUDE.md` y `README.md`.

## Fuera de alcance

- Restaurar `episodic_trajectory` y `procedural_skill`.
- Re-detectar contradicciones al restaurar, y restaurar disputas liberadas.
- Restaurar filas purgadas: ya no existen ([`specs/secrets`](../../specs/secrets/spec.md) R5). Responden como un id inexistente.
- Importar una fila desde un snapshot.
- Restaurar un save entero de un golpe: se restaura pedazo por pedazo.
- Papelera con retención y borrado por proyecto.
- Que el decaimiento salte las filas borradas (`demote_sql` no filtra `deleted_at`, `maintenance.rs:42-49`): una borrada vuelve con el tier que le dejó el decaimiento, que es el que tendría si nunca se hubiera borrado y nadie la hubiera leído.

## Éxito

`cargo test --workspace` sin fallas nuevas contra la base, con tests que fallan hoy:

- restaurar una borrada la deja viva, con vector, y la encuentran la búsqueda por vector y la léxica;
- borrar, restaurar y volver a borrar deja el FTS como un solo borrado (`integrity-check` del FTS y conteo de términos);
- una fila ajena (con scope), viva, inexistente o purgada da el mismo "no existe" sin escribir nada;
- una borrada con una viva igual no se restaura y el error nombra la viva;
- una con un secreto no se restaura y el error nombra los tipos;
- un pedazo restaurado se embebe con su contexto y `read_memory` deja de nombrarlo como faltante;
- un vector huérfano preexistente no hace fallar el restore, y un embedder no listo deja la fila borrada;
- la restaurada recalcula su reposo, y una conclusión restaurada vuelve a cubrir a sus fuentes;
- `deleted=true` lista sólo borradas por `deleted_at` descendente;
- `restore_memory` restaura lo propio y falla como "no existe" con lo ajeno; `tools/list` trae doce herramientas;
- filas sembradas como las deja el delete de la versión anterior (sin vector, FTS restado) se restauran y quedan alcanzables.

No toca el ranking ni el prompt. `rag-eval --real --diff` en la verificación debería dar diff vacío; no es requisito.

## Datos guardados

- **Esquema:** no cambia. No hay migración.
- **Estados e identificadores:** no cambian. El restore sólo escribe `deleted_at`, `status` (el reposo), `last_modified_at` y `update_count` de la fila pedida, su vector y su entrada FTS, y sólo cuando se pide.
- **Datos de la versión anterior:** un test siembra con SQL filas borradas por el `delete_memory_core` de hoy, con el vector ya podado y el FTS restado, y prueba que se restauran y que la búsqueda las encuentra.
- **Rollback:** volver al binario anterior no necesita nada; la base es la misma. Lo restaurado queda vivo, como si nunca se hubiera borrado; un binario anterior lo ve y lo puede volver a borrar.

## Riesgos

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Doble resta o doble suma en el FTS, que corrompe el índice en silencio | Media | Reinserción sólo en la transición borrada → viva, dentro de la transacción; test de borrar-restaurar-borrar con `integrity-check` |
| "Restaurada" pero invisible: duplicado o `superseded` | Media | 409 por dedup; la respuesta de `superseded` lo dice |
| Un secreto viejo vuelve a la búsqueda y a otros agentes | Baja | Se rechaza y se apunta a `clawg purge` |
| Costo de contexto de la duodécima herramienta | Baja | Descripción de una línea; el repo no mide el costo de las herramientas |
| Un daemon viejo recibe el `POST .../restore` | Baja | Responde 404 o 405 de ruta y el puente lo informa como error; se corrige al reiniciar el daemon |
