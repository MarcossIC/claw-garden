# agent-lifecycle-hooks — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 5 | M | memory-protocol (22), context-budget (23), conflict-candidates (24), archivados | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); lo que sigue está verificado otra vez contra `main` (`bdbcafd`).

1. **La memoria sólo se usa si el agente se acuerda.** Lo único que la empuja es el texto de las instrucciones del servidor (la apertura de `crates/clawg-cli/src/mcp/protocol.md`) y el aviso de síntesis dentro de la respuesta de `recall_context` (`synthesis_nudge`, `crates/clawg-cli/src/mcp/tools.rs:349-356`), que sólo ve quien ya llamó a `recall_context`. Una sesión empieza sin nada de lo que la memoria sabe del proyecto, y después de una compactación el cliente pierde lo que había recordado. `Command` no tiene ningún subcomando que hable el protocolo de hooks del host (`crates/clawg-cli/src/cli.rs:32`).
2. **El contexto no se puede pedir sin una consulta.** `ContextRequest::query` es obligatorio (`crates/clawg-cli/src/daemon/api.rs:371-380`) y una consulta vacía no se rechaza: se embebe la cadena vacía y se busca con ella (`api.rs:1092-1118`), así que el resultado no está definido. Al iniciar una sesión todavía no hay consulta.
3. **Un save que no llega no se nota.** Nada le dice al agente que lleva media hora sin guardar; el dato existe (`created_at` por agente y proyecto, `GET /v1/memories` ordena por `created_at DESC`, `crates/clawg-core/src/memory/admin.rs:652`), pero nadie lo mira.

## Objetivo que sirve

1 (que lo guardado se use y que lo que vale la pena llegue a guardarse) y 5 (que el aviso de síntesis llegue aunque el agente nunca llame a `recall_context`).

## Decisiones

Tomadas con el usuario sobre las preguntas abiertas de la exploración; el porqué de cada una está en el [diseño](design.md).

- **`clawg hook <evento>`**, un subcomando del mismo binario (E1). Lee el JSON del host por stdin y escribe `hookSpecificOutput.additionalContext` por stdout. Eventos: `session-start` y `user-prompt-submit`.
- **Contexto sin consulta en el daemon.** `POST /v1/context` sin `query`, o con una en blanco, devuelve el workspace, las memorias propias más recientes de la semana (RECENT, por fecha) y los conflictos activos sobre lo que muestra, dentro del mismo presupuesto, sin embeber ni buscar; `retrieval.state` es `recent`. No registra accesos.
- **Después de compactar, sólo se vuelve a inyectar** (`SessionStart` con `source = compact`), con una línea que recuerda el `session_summary`. El hook no guarda nada (A5 descartada).
- **Sin autoarranque.** Si no hay daemon, el hook no imprime nada y sale con 0; el puente MCP lo arranca.
- **El aviso de síntesis convive:** sigue en `recall_context` y el hook lo agrega al inicio de sesión.
- **Recordatorio de guardar a los 30 minutos**, constante, sin estado nuevo: en `user-prompt-submit`, cuando pasaron 30 minutos desde lo más reciente entre el último save de este agente en este proyecto (cualquier tipo) y el inicio de la sesión. Se repite en cada prompt hasta que llega un save.
- **El proyecto sale del `cwd`** con `ProjectIdentity::resolve`; el hook no registra la raíz.
- **El pie de recordatorio en `search_memory` (E4) se descarta.**
- **Texto factual, no imperativo**, como pide la doc del host, y abierto con que lo recuperado es evidencia y no instrucciones.
- **`--agent` es obligatorio** en `clawg hook`: un hook no recibe `clientInfo`, y un id distinto del del puente leería y mediría a otro agente sin decirlo.
- **Nunca sale con 2.** Para el host, 2 bloquea el prompt o la sesión. Un error de uso o de perfil sale con 1 (el host lo muestra y sigue), y toda falla del daemon o de la entrada sale con 0 y sin salida.

## Alcance

1. `clawg-core`: el armado del contexto sin consulta (`prompt_builder.rs`) con su estado de recuperación.
2. `POST /v1/context`: `query` opcional; `retrieval.state = recent`.
3. `clawg-cli`: `Command::Hook`, un módulo `hook` con el núcleo puro (entrada del host, texto de cada evento, recordatorio) y un borde de E/S fino con tope de tiempo; `main` no deja que un error de uso de `hook` salga con 2.
4. Docs: `.claude/CLAUDE.md`, el mapa de `openspec/README.md` y cómo configurarlo a mano para probar (la instalación es de client-setup, 26).

## Fuera de alcance

- Instalar los hooks o tocar `~/.claude` (client-setup, 26).
- `PreCompact`, `PostCompact` y `SessionEnd` (ver la [exploración](exploration.md#descartado)).
- Id de sesión y bloque de última sesión (session-handoff, 28).
- Hooks de otros clientes.
- Cambiar el ranking, `recall_context` con consulta, o el texto del protocolo.

## Cómo se mide el éxito

- Tests que fallan hoy (ver [spec](spec.md)): el contexto sin consulta no embebe y trae RECENT por fecha; `hook session-start` contra un daemon real devuelve las memorias sembradas en el sobre del host; daemon ausente, entrada inválida o un daemon que no contesta dan stdout vacío y 0; un error de uso de `hook` sale con 1; el recordatorio sale a los 30 minutos y no antes; la salida queda bajo los 10.000 caracteres del host salvo un workspace más grande que el presupuesto.
- `rag-eval`: no hace falta. El contexto con consulta no cambia (los tests de armado que ya existen siguen pasando sin tocarlos) y el eval siempre manda consulta.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding, y no escribe nada en el store. `query` opcional es aditivo: un puente manda siempre consulta, y un daemon anterior responde `4xx` al pedido sin consulta, que el hook toma como «nada que inyectar». Volver atrás es sacar los hooks de la configuración del host.
