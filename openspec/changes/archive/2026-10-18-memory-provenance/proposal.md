# memory-provenance — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 4, 5 | S | — | ver [`state.yaml`](state.yaml) |

## Problema

Cada memoria guardada por MCP queda sellada como dicha por el usuario, aunque el agente la haya inferido o sea una conclusión de `synthesize`. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código). Lo que sigue se volvió a comprobar contra `main` (`880aa07`):

1. **El handler atribuye todo save al usuario.** Arma el evento con `role: Role::User` y `source: MemorySource::Explicit` (`crates/clawg-cli/src/daemon/api.rs:901-917`), y el insert fija `CreatedByType::User` para todo `Explicit` (`crates/clawg-core/src/memory/store_insert.rs:156-183`).
2. **Una conclusión no se distingue de un hecho.** Un save con `derived_from` queda `explicit` / `user` como cualquier otro; sólo sus filas en `memory_derivation` lo marcan (`crates/clawg-core/src/memory/synthesis.rs:32-39`).
3. **El agente no puede declarar el origen.** `SaveMemoryArgs` tiene `content`, `memory_type`, `importance`, `project_path` y `derived_from` (`crates/clawg-cli/src/mcp/server.rs:151-177`).
4. **Nadie lee el origen.** Ni los pares que juzga el agente (`format_open_pairs`, `crates/clawg-cli/src/mcp/tools.rs:124-143`), ni ACTIVE CONFLICTS (`format_conflict_block`, `crates/clawg-core/src/memory/prompt_builder.rs:557-585`), ni `synthesize` (`synthesis_line`, `tools.rs:234-242`) lo muestran. Sólo el visor imprime las columnas (`crates/clawg-cli/src/memories.rs:212-224`).

## Decisiones

Tomadas sobre las [decisiones abiertas](exploration.md#decisiones-abiertas):

- **Enfoque B.** El daemon sella lo que sabe, y `save_memory` suma un argumento opcional para declarar que el contenido es lo que dijo el usuario.
- **La declaración es un booleano, `user_stated`.** Sin él, el save es del agente.
- **`source` suma valores.** Un save con `derived_from` queda `source = 'synthesis'`. `import` queda reservado para history-import (44), que lo escribe; este cambio sólo lo lee.
- **Usos que entran:** 1 (los pares) y 2 (`synthesize`). La marca en las líneas de búsqueda (3) y el export (4) quedan fuera.
- **Filas anteriores:** no se tocan. Su origen no es confiable, y por eso ningún lector afirma «lo dijo el usuario» (ver [diseño](design.md#d1-se-marca-lo-inferido-no-lo-dicho)).

## Alcance

1. Una regla pura de sello en el núcleo: (origen del evento, rol, si trae `derived_from`, `user_stated`) → (`source`, `created_by_type`).
2. `user_stated` en `save_memory`, `SaveRequest`, `SaveArgs` y `MemoryEvent`, con `#[serde(default)]`. `POST /v1/memories` rechaza con 400 `user_stated` junto a `derived_from`, antes del outbox.
3. Una regla pura de marca que lee (`source`, `created_by_type`) de una fila: `conclusion`, `imported`, `inferred` o nada.
4. Uso 1: `memory_status` nombra primero, en cada par pendiente, el lado sin marca, y marca el otro; ACTIVE CONFLICTS marca cada lado. Ningún par cambia de estado por su origen.
5. Uso 2: `synthesize` marca cada memoria pendiente y cada conclusión con su origen.
6. `protocol.md` dice cuándo declarar `user_stated`; el contrato del agente se bendice y su diff se revisa.
7. Docs: `.claude/CLAUDE.md` y la spec nueva `specs/provenance` al archivar.

## Fuera de alcance

- Reescribir el origen de las filas existentes.
- Usar el origen en `needs_user`, en el ranking o para resolver un par.
- La marca en `search_memory` y `recall_context` fuera de ACTIVE CONFLICTS (uso 3), y el export (uso 4).
- Las candidatas que lista la respuesta del save (`related`): nacen de un hit de KNN, que no trae el origen.
- Cambiar el origen de una fila al reforzarla con un save idéntico: el refuerzo no escribe una fila.
- `last_modified_by` y `source_session_id`.

## Datos guardados

- **Esquema:** sin migración. `source` y `created_by_type` son `TEXT` sin `CHECK` (`crates/clawg-core/migrations/V1__init.sql:12-14`).
- **Valores nuevos:** `source = 'synthesis'`; `created_by_type = 'agent'` en un save por MCP sin `user_stated`.
- **Una fila del outbox escrita por la versión anterior** no trae `user_stated` y se sella con la regla nueva: `agent`. No falla.
- **Rollback:** una versión anterior abre la base sin cambios. Lee las columnas como texto y sólo las muestra (`crates/clawg-core/src/memory/admin.rs:757`), así que imprime `synthesis` tal cual. Un save que haga después vuelve a sellarse `explicit` / `user`.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- un save por MCP sin `user_stated` deja `created_by_type = 'agent'` (hoy `user`);
- un save con `derived_from` queda `source = 'synthesis'`, `created_by_type = 'agent'`;
- un save con `user_stated` queda `explicit` / `user`, y el esquema de `save_memory` nombra el argumento;
- `user_stated` con `derived_from` responde 400 y no escribe nada;
- un evento sin el campo se ingiere sin fallar;
- una base sembrada con filas de la versión anterior abre con todas sus filas y su origen sin cambios, y ninguna lleva marca;
- en `memory_status`, el lado sin marca va primero y el inferido lleva `inferred`, sin que el par cambie de estado;
- `synthesize` marca una conclusión y una memoria inferida.

`rag-eval`: el ranking y las líneas de búsqueda no cambian, así que no hace falta `--diff`. La suite y el contrato fijado lo cubren.
