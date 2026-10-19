# session-handoff — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 5 | M | agent-lifecycle-hooks (25) y context-budget (23), archivados | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); lo que sigue está verificado otra vez contra `develop` (`4a213aa`).

1. **Ningún save sabe de qué sesión salió.** `memory.source_session_id` existe desde V1 (`crates/clawg-core/migrations/V1__init.sql:16`), pero el `INSERT` de producción no la nombra (`crates/clawg-core/src/memory/store_insert.rs:436-440`), `MemoryEvent` no tiene dónde llevarla (`crates/clawg-core/src/memory/events.rs:13-56`) y el puente no conoce la sesión del host. El visor la imprime (`crates/clawg-cli/src/memories.rs:194`) y siempre dice `-`.
2. **El último resumen de sesión no se muestra primero.** Un `session_summary` sólo llega al contexto si la consulta se le parece: ni `build_memory_prompt` ni `build_session_prompt` lo tratan distinto (`crates/clawg-core/src/memory/prompt_builder.rs:154`, `:268`). Una sesión nueva no sabe dónde quedó la anterior aunque el agente haya guardado el resumen que pide el protocolo (`crates/clawg-cli/src/mcp/protocol.md:107-117`).
3. **El único que conoce el id de sesión es el hook.** El host lo manda por stdin a cada hook, y `HookInput` lo ignora (`crates/clawg-cli/src/hook.rs:38-46`).

## Objetivo que sirve

1 (que lo guardado se use: el resumen que el agente escribió llega a la sesión siguiente) y 5 (la historia de una sesión queda atribuida, base de cualquier conclusión por sesión).

## Decisiones

Las de la exploración, cerradas con sus recomendaciones; el porqué está en el [diseño](design.md).

- **Origen del id: registro en el daemon (A).** Los dos hooks (`session-start` y `user-prompt-submit`) mandan `{agent_id, project_id, session_id}` a `POST /v1/sessions`; el daemon lo recuerda en memoria por agente y proyecto. El entorno del puente (B) queda fuera: no está en la documentación oficial del host.
- **Sesiones simultáneas: gana la última registrada.** Como también registra `user-prompt-submit`, la última es la sesión donde el usuario escribió por última vez. Es una limitación documentada: la consecuencia es sólo de atribución.
- **El id se sella al aceptar el save**, en el evento que va al outbox, así una reanudación después de un kill lo conserva. Sin registro (daemon recién arrancado, sin hooks, cliente viejo) queda `NULL`. El modelo nunca lo pasa: `save_memory` no gana argumentos.
- **Bloque `=== LAST SESSION ===`** en todo `POST /v1/context`, con consulta o sin ella: el `session_summary` vivo más nuevo del proyecto entre lo que el agente puede leer, propio o compartido por `sharing.json`, marcado `by <author>` cuando es de otro. Se muestra con su fecha, sin umbral de antigüedad. No registra accesos. No se repite en RECENT ni en LONG-TERM.
- **Presupuesto:** el bloque va después del workspace y se corta después de LONG-TERM y RECENT y antes que los pares de conflicto.
- **Sin cambios de esquema** (ni índice ni tabla); `last_modified_by`, la línea de tiempo de sesión y la captura de prompts quedan fuera.

## Alcance

1. `clawg-core`: `MemoryEvent::session_id` (opcional, `serde(default)`), `NewMemoryRecord::source_session_id` y el `INSERT`; el bloque LAST SESSION en `prompt_builder.rs`.
2. `clawg-cli`: el registro de sesiones del daemon y `POST /v1/sessions`; el sello en `save_memory`; `HookInput::session_id` y el registro desde los dos hooks.
3. `clawg-eval`: `rag-eval --agent-view` lee también los hits de LAST SESSION.
4. `protocol.md`, fuera de la apertura: que el resumen más nuevo abre la sesión siguiente.
5. Docs: `.claude/CLAUDE.md` y el mapa de `openspec/README.md`.

## Fuera de alcance

- Tabla `session`, índice nuevo, continuaciones o arrendamientos.
- `CLAUDE_CODE_SESSION_ID` o `_meta` como fuente del id.
- Escribir `last_modified_by`, consultar las memorias de una sesión, capturar prompts.
- Estructura obligatoria del resumen (ya la pide el protocolo).

## Cómo se mide el éxito

- Tests que fallan hoy (ver [spec](spec.md)): un save con sesión registrada escribe `source_session_id` y sin registro queda `NULL`; el id sobrevive al outbox; el registro separa agentes y proyectos y lo reemplaza un registro nuevo; `recall_context` y el contexto sin consulta traen LAST SESSION con el resumen más nuevo aunque no se parezca a la consulta; el sharing oculta o marca el resumen de otro agente; el bloque cede ante el presupuesto en el orden dicho; el hook registra la sesión contra un daemon real; `save_memory` sigue sin argumento de sesión (contrato del agente sin cambios).
- `rag-eval --real --agent-view` contra la línea base de `openspec/config.yaml` (`recall_context` 39/46 a 1.086,4 tokens de media), umbral fijado de antemano: ningún caso de `recall_context` que se pierda, y el aumento de tokens medio no mayor que el tamaño medio de un `session_summary` del corpus. Pide el modelo real y el golden set de `docs/eval/`, fuera de git.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding: escribe una columna que ya existe y que hasta ahora era siempre `NULL`. Las filas anteriores siguen con `NULL` y se leen, buscan y muestran como hoy (test de preservación sobre filas sin sesión). Un outbox escrito por una versión anterior no trae el campo y se guarda con `NULL`. Volver a la versión anterior no necesita nada: la versión anterior ignora la columna. Un hook nuevo contra un daemon anterior recibe 404 en el registro y sigue igual.
