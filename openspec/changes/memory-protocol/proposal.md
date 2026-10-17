# memory-protocol — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — | ver [`state.yaml`](state.yaml) |

## Problema

El servidor MCP le dice al agente **cuándo** llamar a cada herramienta, pero casi nada de **qué** guardar ni **cómo** escribirlo, y el tipo de memoria que el agente elige no se valida al guardar. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código). Lo que sigue se volvió a comprobar contra `main` (`af6fef4`):

1. **Las instrucciones dicen poco.** Son la constante `INSTRUCTIONS`, de 807 caracteres (`crates/clawg-cli/src/mcp/server.rs:114-125`), y se envían en `get_info` (`server.rs:1132`) y en `discover` (`server.rs:1138-1144`). Nombran un solo disparador para guardar ("a durable fact, preference or decision"). No piden la causa raíz de un bug, el "dónde" con identificadores exactos, ni un título en la primera línea. Tampoco dicen en qué idioma guardar, qué dejar afuera ni que lo recuperado es evidencia y no instrucciones. Eso último hoy sólo se dice de las filas de otro agente (`SHARED_NOTE`, `crates/clawg-core/src/memory/prompt_builder.rs:30-31`).
2. **El idioma del save decide si se encuentra.** english-recall (21) midió con `rag-eval --real` que una fila en inglés buscada en español acierta R@5 en 6 de 19 casos, contra 18/20 de español con español. Con cualquiera de los dos lados traducido llega a 19/19 y 18/19 ([`specs/eval`](../../specs/eval/spec.md#por-pareja)). Hoy nada le pide al agente un idioma.
3. **Un tipo mal escrito cae en `semantic_fact` sin aviso.** `SaveMemoryArgs.memory_type` es un `Option<String>` libre (`server.rs:139`), y los siete valores sólo aparecen en su comentario. El daemon lo pasa tal cual (`crates/clawg-cli/src/daemon/api.rs:813`), y el worker reemplaza un valor desconocido por `semantic_fact` (`crates/clawg-core/src/memory/store_insert.rs:141-145`). `PATCH` sí rechaza el mismo valor con 400 (`crates/clawg-core/src/memory/admin.rs:143`, `:173`; `api.rs:1054-1055`). El comentario de `the_save_memory_schema_names_the_accepted_memory_types` dice que un tipo inventado «is stored verbatim» (`server.rs:1826`), y el código dice lo contrario. La lista de los siete se escribe en cuatro lugares: los comentarios de `SaveMemoryArgs` y `UpdateMemoryArgs`, el mensaje de `PatchError::UnknownType` y `MemoryType::from_db_str`.
4. **`CONFLICT_NOTE` está en español** (`prompt_builder.rs:348-351`), igual que la línea de los pares omitidos (`prompt_builder.rs:402`). Todo lo demás que lee el agente está en inglés.

## Decisiones

Están cerradas en la [exploración](exploration.md#decisiones):

- **E2.** El protocolo es un único archivo, `crates/clawg-cli/src/mcp/protocol.md`, compilado con `include_str!`. Sus párrafos de apertura, antes de la primera sección, son las instrucciones del servidor. El texto completo queda en el binario para el hook de sesión (25) y el setup del cliente (26).
- **Tope de 2.048 caracteres** para las instrucciones, fijado por un test. Ese tope no corta el texto en clientes de Claude Code anteriores a la v2.1.296.
- **Inglés** para el protocolo, para los saves y para las búsquedas.
- **T2 + T1.** El schema de `save_memory` y de `update_memory` declara `memory_type` como enum. El daemon contesta 400 a un save con un tipo desconocido, con los siete valores en el mensaje, antes de registrarlo en el outbox.
- **`CONFLICT_NOTE` en inglés**, con la línea de los omitidos.
- **El resumen de sesión** tiene seis secciones: Goal, Instructions, Discoveries, Accomplished, Next steps y Relevant files.

## Alcance

1. `protocol.md` con la apertura (las instrucciones) y las secciones largas. Una función pura extrae la apertura y une las líneas cortadas de cada párrafo.
2. `get_info` y `discover` mandan esa apertura. Hay tests de tope, de coherencia con las herramientas registradas y con los tipos, y de los hábitos que la apertura tiene que nombrar.
3. Una sola lista de tipos en el núcleo (`MemoryType::ALL`). La usan el mensaje de error, la validación del save, el enum del schema y el test del protocolo.
4. El save rechaza un tipo desconocido con 400, y el puente lo informa como "not saved". Un evento que ya está en el outbox con un tipo desconocido (escrito por una versión anterior) sigue guardándose como `semantic_fact`.
5. El enum en los schemas de `save_memory` y `update_memory`, y el contrato del agente bendecido a propósito.
6. La descripción de `save_memory` pide el título y el "dónde".
7. `CONFLICT_NOTE` y la línea de omitidos en inglés. `specs/retrieval` se actualiza con el texto nuevo.
8. `.claude/CLAUDE.md` dice dónde vive el protocolo.

## Fuera de alcance

- Entregar el texto largo: el hook de sesión (25) y el setup del cliente (26).
- Prompts y recursos MCP ([exploración, Descartado](exploration.md#descartado)).
- Ranking, esquema, importancia por defecto, y agregar o quitar tipos.
- Traducir lo ya guardado.
- Declarar como dicho por el usuario lo que es una cita (memory-provenance, 41). El protocolo lo suma cuando 41 cierre.
- El filtro `memory_type` de `list_memories`: un valor desconocido no escribe nada y devuelve una lista vacía.

## Datos guardados

No cambian el esquema, los ids, los estados ni el contrato del embedding. El único cambio sobre lo que se escribe es que un save nuevo con un tipo desconocido se rechaza y no llega al outbox. Lo que ya está en el outbox se reproduce como antes: el reemplazo por `semantic_fact` de `store_insert.rs:141-145` se queda, y lo fija `explicit_source_with_an_unknown_type_override_falls_back_to_semantic_fact` (`store_insert.rs:997`). Rollback: revertir los commits. No hay nada que migrar.

## Éxito

Tests que fallan hoy (detalle en la [spec](spec.md)):

- las instrucciones son la apertura de `protocol.md`, miden 2.048 caracteres o menos y nombran los hábitos de R3;
- cada herramienta registrada aparece en el protocolo, y cada nombre entre comillas invertidas del protocolo existe;
- un save con `memory_type: "preference"` contesta 400 con los siete valores y no deja una fila en el outbox; un tipo válido o ausente se guarda como hoy;
- el schema de `save_memory` declara el enum con los siete valores;
- `CONFLICT_NOTE` está en inglés.

`rag-eval` no aplica: el cambio no toca ranking, recuperación ni contexto, y el golden set parte de un corpus fijo. El efecto de saves escritos según el protocolo (identificadores, inglés) se lee en el corte por pareja de un golden con filas nuevas. Eso queda pendiente hasta que exista ese golden. La sesión de prueba sobre el perfil dev necesita autorización y está en [`verify.md`](verify.md).
