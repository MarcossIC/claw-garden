# core-use-cases — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (sirve al 1: guardar y clasificar) | M, por partes; este PR aplica la primera | memory-module-layout, aplicado en `develop` | ver [`state.yaml`](state.yaml) |

## Problema

Las reglas de qué es un guardado válido viven en el handler HTTP. Para probar una de ellas hay que levantar un `AppCtx` y pasar por el router, y los otros caminos del núcleo (`eval_api`, un puerto de almacenamiento futuro) no tienen dónde pedirlas. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código), revisada el 2026-11-12 sobre `develop`. Las líneas que siguen son las de `develop` al escribir esto:

1. **`save_memory` decide el dominio** (`crates/clawg-cli/src/daemon/api.rs:1420-1552`):
   - `user_stated` con `derived_from` se rechaza (`:1440-1444`);
   - las fuentes de una conclusión tienen que existir, estar al alcance del que guarda y no estar reemplazadas (`:1445-1466`), con `existing_memory_ids` y `replaced_memory_ids` (`crates/clawg-core/src/memory/upkeep/synthesis.rs:378`, `:345`), que sólo llama este handler;
   - con el embedder `failed` no se acepta el save (`:1467-1475`);
   - el proyecto se sigue hasta el vivo (`live_project`, `:1477`).
2. **El tipo se valida y se tira.** `raw.parse::<MemoryType>()` (`:1427-1429`) descarta el resultado; la métrica de la sesión vuelve a parsear y tira el error (`:1482`), y al evento viaja el `String` original (`:1495`).
3. **Las reglas de formato son funciones de `api.rs`:** `check_content` (`:703`), `check_operation_id` (`:722`), `distinct_sources` (`:331`), `missing_sources` (`:345`) y `replaced_list` (`:350`), con sus tests en el `mod tests` del daemon. El tope de contenido es del producto ([`specs/ingest`](../../specs/ingest/spec.md)), no de HTTP.

## Enfoque

El **A** de la exploración: un caso de uso por operación, empezando por el guardado. Las tres decisiones abiertas se toman en el [diseño](design.md):

1. **Los errores llevan el texto de hoy en su `Display` y los datos en sus campos** ([D3](design.md#d3-un-error-con-el-texto-de-hoy-y-sus-datos)). La API sólo habla inglés; `Lang` es de la CLI y del puente, que muestran el mensaje del daemon tal cual.
2. **`check_content` y `check_operation_id` son del dominio** ([D4](design.md#d4-el-formato-de-un-save-es-del-dominio)): el tope de contenido sale del outbox, y el `operation_id` es la identidad de una fila del outbox.
3. **`eval_api` no pasa a usar estos casos de uso** ([D6](design.md#d6-eval_api-no-cambia)).

Y de los otros handlers largos, en este PR no entra ninguno ([D7](design.md#d7-qué-handlers-entran)).

## Alcance

Este PR aplica la primera parte coherente, el guardado completo (`POST /v1/memories`):

1. **`memory::save`** (`crates/clawg-core/src/memory/ingest/save.rs`), puro:
   - `SaveInput`, el save como lo manda un cliente;
   - `NewSave::parse`, que comprueba el formato en el orden de hoy y guarda el `MemoryType` parseado;
   - `SaveRejected`, con `thiserror`, una variante por regla;
   - `check_operation_id`, que usan también las rutas del outbox;
   - `MAX_CONTENT_BYTES`, `MAX_OPERATION_ID_LEN` y `MAX_SOURCES`, que se mudan del daemon.
2. **`MemoryRuntime::admit`** (`crates/clawg-core/src/runtime.rs`): las reglas que leen el store y el embedder (fuentes vivas y al alcance, embedder en falla, proyecto vivo) y devuelve un `Admitted`, del que sale el `MemoryEvent`.
3. **`save_memory`** queda en leer el cuerpo, pedir la sesión al registro del daemon, llamar a `accept` y traducir cada `SaveRejected` a un estado.
4. Los tests puros del daemon que cubren esas reglas se mudan a core con su nombre y sus literales.

Las partes siguientes, cada una en su PR, están en [`tasks.md`](tasks.md#partes-siguientes).

## Fuera de alcance

- Cambiar códigos de estado, cuerpos de respuesta o mensajes de error. Ninguna respuesta existente cambia, ni con campos agregados.
- El contrato MCP (`agent_contract.json`, `agent_contract.es.json`) y los textos del puente.
- `PATCH /v1/memories/...`, que también parsea `memory_type`, y las demás rutas que escriben: van en las partes siguientes.
- `context`, `register_root` y `locate_project` ([D7](design.md#d7-qué-handlers-entran)).
- `eval_api` ([D6](design.md#d6-eval_api-no-cambia)).
- Partir `api.rs` en archivos.

Y lo que se hace en local después del merge: archivar el cambio, subir la versión, `openspec/ROADMAP.md` y consolidar las specs principales.

## Datos guardados

Ninguno. El evento que se guarda en `ingest_outbox` sigue igual: `MemoryEvent::memory_type` sigue siendo `Option<String>` y lleva el mismo texto ([D5](design.md#d5-el-tipo-se-parsea-y-el-evento-lleva-su-nombre)). No cambia el esquema ni ningún identificador. Una versión anterior abre la misma base y reproduce las mismas filas del outbox.

## Cómo se mide

- **Tests en rojo en core, sin router ni HTTP** ([spec](spec.md)): los de `memory/ingest/save.rs` no compilan antes del cambio (el módulo no existe), y los de `admit` en `runtime.rs` tampoco (el método no existe).
- **Caracterización primero:** `each_refused_save_keeps_its_status_and_body` (`mod tests` de `crates/clawg-cli/src/daemon/api.rs`) fija el estado y el cuerpo entero de cada rechazo de un save, pasa sobre el código de `develop` y sigue pasando sin tocarse ([R7](spec.md#r7-lo-que-ve-un-cliente-no-cambia)).
- **La suite pasa sin editar ningún test de respuestas:** `crates/clawg-cli/tests/api.rs` y los tests de `crates/clawg-cli/src/mcp/` no cambian, y los dos contratos del agente pasan sin `CLAWG_BLESS`.
- **`save_memory` no decide ninguna regla:** no nombra `MemoryType`, `EmbedderStatus`, `existing_memory_ids`, `replaced_memory_ids` ni `live_project`. `verify.md` lo registra con su largo antes y después.
- **No hay métrica de `rag-eval`:** el cambio no toca la recuperación.
