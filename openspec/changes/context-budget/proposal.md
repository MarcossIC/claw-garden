# context-budget — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | — (24 y 28 suman bloques cuando existan; 25 lo necesita) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md#premisas-que-se-verificaron-y-cuáles-no-eran-del-todo-ciertas); lo que sigue está verificado otra vez contra `main` (`af6fef4`).

1. **`recall_context` no tiene tope de tamaño.** `ContextRequest` sólo lleva `query`, `agent_id` y `project_id` (`crates/clawg-cli/src/daemon/api.rs:353-359`), y `RecallContextArgs` sólo `query` y `project_path` (`crates/clawg-cli/src/mcp/server.rs:172-181`). `build_memory_prompt` imprime hasta `K_FINAL = 10` hits con su texto completo (`crates/clawg-core/src/memory/prompt_builder.rs:42,179,344`), hasta cinco pares de conflictos con texto (`:44,234`) y el workspace, cuyas preferencias y sección USER no tienen tope (`crates/clawg-core/src/memory/workspace.rs:392-402`; `prompt_builder.rs:430-446`). Una fila llega a 480 tokens; diez, a unos 4.800. Claude Code advierte arriba de 10.000 tokens de una herramienta MCP, y el stdout de un hook `SessionStart` se corta a 10.000 caracteres.
2. **Sin embedder, el contexto calla que faltan los recuerdos.** Con el embedder ausente (`loading` o `failed`), con `embed` fallando o con la búsqueda fallando, `build_memory_prompt` sólo registra un `warn` y devuelve el workspace (`prompt_builder.rs:144-177`). `ContextBody` sólo lleva `prompt` y `unreviewed` (`api.rs:522-528`), así que el puente no puede avisar, y un prompt vacío se lee `no context yet` (`crates/clawg-cli/src/mcp/tools.rs:297-299`): un agente concluye que no hay memorias.

## Objetivo que sirve

1: guardar y recuperar memorias con un costo predecible, y sin que la recuperación mienta sobre lo que hay.

## Decisiones

Defaults tomados donde la exploración dejaba la pregunta abierta. Se pueden cambiar sin rehacer el diseño ([diseño](design.md)).

- **Unidad: bytes**, documentados como «unos 4 por token». Es la unidad del tope AUTO (`workspace.rs:18-19`) y la de los topes del cliente (caracteres); contar tokens en el daemon usaría un tokenizador que no es el de Claude.
- **Default 8.000 bytes, máximo 24.000.** El default queda bajo los 10.000 caracteres del hook y por encima del máximo medido (1.408 tokens, unos 5.600 bytes). Un `budget` mayor que el máximo se lleva al máximo; `0` se rechaza con `400`.
- **El workspace entra entero, siempre.** Las preferencias y la sección USER no se recortan, y las decisiones y skills del AUTO tampoco: el workspace ya las reportó como entregadas (`EmittedIds`), y la búsqueda las saltea, así que recortarlas en el prompt las dejaría sin ningún canal. Si el workspace solo supera el presupuesto, la respuesta lo dice con cuántos bytes haría falta.
- **Conflictos antes que hits.** Se buscan sobre las filas candidatas (como hoy, antes del recorte) y tienen prioridad sobre los hits; un lado cuyo hit quedó afuera se imprime con su texto, nunca por un id que no se imprimió. La prioridad «alto riesgo» llega con 24.
- **Prioridad del resto:** RECENT antes que LONG-TERM; dentro de cada bloque, el orden de hoy. Se corta desde el final; el primer hit de un bloque se recorta en lugar de saltarse.
- **Lo cortado se nombra por id** en un bloque `=== BUDGET ===` al final, con cuántos bytes ocuparía el contexto entero y `read_memory` como forma de pedirlo.
- **El aviso de estado cubre los cuatro silencios:** modelo cargando, modelo que no cargó (con causa y si reintenta), `embed` que falla y búsqueda que falla. Los cuatro hacen que un agente lea «no hay memorias».
- **El aviso lo escribe el puente**, fuera del prompt (como el recordatorio de `synthesize`), y en inglés, como las herramientas. Un daemon viejo no manda el campo y el puente no inventa un aviso.
- **El resumen del historial (E3) queda fuera:** necesita títulos (22) y la última sesión (28).

## Alcance

1. `prompt_builder.rs`: el presupuesto con prioridad (pura), el bloque `BUDGET`, el resultado de la búsqueda, y el registro de accesos sólo de lo que se imprimió.
2. `POST /v1/context`: `budget` opcional en la petición; `retrieval` en la respuesta, leído de la misma fuente que `ready_embedder`.
3. `DaemonClient::context` con `budget`; `Context::retrieval`, ausente en un daemon viejo.
4. `recall_context`: argumento `budget`, el aviso de estado y su descripción.
5. Docs: `.claude/CLAUDE.md` y el mapa de `openspec/README.md`.

## Fuera de alcance

- Resumen o compresión del historial (E3).
- Cambiar `K_FINAL`, el ranking, la ventana de siete días o el tope AUTO.
- Presupuestar `search_memory` o `search_project`.
- Conflictos de alto riesgo (24) y última sesión (28): el orden deja lugar para ellos.
- Tope de la sección USER.
- Un `--context-budget` en `rag-eval`: con el default alto el eval no cambia; la prueba del recorte real es el test de la base con filas largas.

## Cómo se mide el éxito

- Tests que fallan hoy (ver [spec](spec.md)): el prompt nunca supera el presupuesto salvo por el workspace, que lo dice; el corte cae en un carácter UTF-8 y se marca; lo cortado se nombra por id y `read_memory` lo devuelve; un conflicto nunca cita un id que no se imprimió; `recall_context` empieza con el aviso en los cuatro silencios; un daemon viejo no produce aviso.
- `rag-eval --real --agent-view` antes y después, con el default: `found` de `recall_context` sin cambios (39/46) y tokens sin aumento. Necesita el modelo real y el golden set (`docs/eval/`, fuera de git).

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding. Los dos campos nuevos son aditivos: un daemon viejo ignora `budget` y no manda `retrieval`, y un puente viejo no manda `budget` (el daemon usa el default) e ignora `retrieval`. Volver atrás es revertir; un `budget` de 24.000 deja el contexto casi como hoy.
