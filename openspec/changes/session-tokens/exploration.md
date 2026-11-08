# session-tokens — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| medición (cuánto cuesta una sesión, por proyecto y por agente; la pregunta de fondo es si una sesión con clawg gasta más o menos) | M–L (el receptor es M; el lector del transcript es S pero frágil) | [session-usage](../session-usage/exploration.md) (121), que da la fila de sesión a la que se suman los tokens; contrasta con [agent-eval](../agent-eval/proposal.md) (96), que mide costo en corridas controladas | exploración (borrador) |

**Tesis.** clawg no ve ningún token: se gastan entre el cliente y el proveedor del modelo, y a clawg sólo le llegan llamadas a herramientas. Para contarlos alguien tiene que decírselos. En Claude Code hay una sola fuente documentada, su telemetría OpenTelemetry, que hay que encender a mano. Hay otra sin documentar, el transcript, y Anthropic avisa que su formato cambia entre versiones. Cursor no expone tokens en ningún hook. Este cambio decide si vale la pena y por cuál camino, y no se empieza antes de que exista el registro de sesiones de 121.

## El problema verificado contra el código

1. **Nada en `clawg-cli` ni en `clawg-core` lee tokens.** Las apariciones de `token` en esos crates son el bearer del daemon (`crates/clawg-cli/src/daemon/lifecycle.rs:80`) y los tokens del embedder.
2. **El hook lee del transcript sólo su fecha de creación** (`session_started_at`, `crates/clawg-cli/src/hook.rs:239`), para el aviso de guardado. `HookInput` no parsea `context_tokens` ni `model`.
3. **El único lector de uso está en `agent-eval`.** `Usage` tiene `input_tokens`, `output_tokens`, `cache_creation_input_tokens` y `cache_read_input_tokens` (`crates/clawg-eval/src/bin/agent-eval/stream.rs:27-31`). Lee el `stream-json` de una corrida de `claude -p`, no el transcript en disco, y es un módulo de un binario de evaluación que producción no enlaza.
4. **El token del daemon cambia en cada arranque.** «a restarted daemon has a new token and maybe a new port» (`crates/clawg-cli/src/mcp/server.rs:740`), y todo `/v1` pide `Authorization: Bearer` (`crates/clawg-cli/src/daemon/api.rs:13`). Un cliente configurado con una variable de entorno fija no puede llevar ese token.

## Qué da el cliente (documentación leída el 2026-11-08)

| Fuente | Qué trae | Contra |
|---|---|---|
| **OpenTelemetry de Claude Code** (https://code.claude.com/docs/en/monitoring-usage) | `claude_code.token.usage` por tipo (`input`, `output`, `cacheRead`, `cacheCreation`), `claude_code.cost.usage` en USD, y eventos `api_request` con modelo, tokens, costo y duración, con el atributo `session.id`. Exporta OTLP por `http/json` a localhost | Hay que encenderlo (`CLAUDE_CODE_ENABLE_TELEMETRY=1`, exportador y endpoint). Un repositorio no puede encenderlo desde `.claude/settings.json`: hacen falta variables de entorno o la configuración de usuario. Un usuario que ya manda su telemetría a otro lado tendría que elegir. El `session.id` lo controla `OTEL_METRICS_INCLUDE_SESSION_ID`, cuyo valor por omisión está sin verificar. Que sea el mismo `session_id` de los hooks: sin verificar |
| **El transcript** (`~/.claude/projects/<proyecto>/<id>.jsonl`, https://code.claude.com/docs/en/sessions) | Según fuentes de terceros, cada respuesta del asistente lleva `message.usage` con los cuatro campos | La documentación dice que el formato es interno y «changes between versions, so scripts that parse these files directly can break on any release». Hay entradas repetidas que se deduplican por id de mensaje, y los subagentes escriben en archivos aparte. El transcript «may lag the in-memory conversation» |
| **La `statusline`** (https://code.claude.com/docs/en/statusline) | `cost.total_cost_usd` acumulado, que vuelve a 0 con `/clear` | Los tokens que trae son los de la última respuesta, no los acumulados de la sesión. Además, es una sola configuración del usuario, que puede tener la suya |
| **`context_tokens` en `SessionStart`** | Lo que la primera petición de una sesión retomada reenvía | Sólo con `resume` o `fork`. No es un contador de uso |
| **Cursor** (https://cursor.com/docs/agent/hooks) | Ningún campo de tokens ni de uso en los hooks (según el resumen de la página; sin verificar campo por campo) | — |

## Caminos

### A. Un receptor OTLP en el daemon (Recomendado si se hace; M)

El daemon acepta OTLP por `http/json` en una ruta propia, por ejemplo `/otlp/v1/logs` y `/otlp/v1/metrics`. Así no choca con `GET /v1/metrics` (`api.rs:1141`), y Claude Code admite un endpoint por señal. De lo que llega guarda sólo `session.id`, modelo, los cuatro tipos de tokens, el costo y la cantidad de peticiones, y los suma a la fila de sesión de 121. Descarta todo lo demás al recibirlo: el texto de prompts, el email, el uuid de la cuenta y los nombres de herramientas.

Lo que tiene que resolver la propuesta:

- **Autenticación.** El token cambia en cada arranque (punto 4). Hay dos salidas:
  - un secreto propio de la ingesta OTLP, estable y guardado en el directorio de datos;
  - o aceptar esa ruta sin token, sólo desde loopback, sabiendo que otro proceso local podría escribir uso falso. El daño es bajo, pero rompe la regla del guard.
- **El puerto.** El daemon puede arrancar en otro puerto si el suyo está ocupado (mismo comentario en `server.rs:740`). Con el endpoint fijo en una variable, esa telemetría se pierde.
- **Quién lo enciende.** Puede ser un `clawg setup claude-code --telemetry` que escriba las variables en la configuración de usuario de Claude Code, que es una escritura a nivel usuario con su `--remove`. O una guía para hacerlo a mano.
- **`http/json`** evita la dependencia de protobuf. Si Claude Code manda bien ese protocolo a un endpoint por señal se comprueba en local.

### B. Leer el transcript al final o al empezar la siguiente (Descartado salvo que A no se pueda; S)

`session-start` ya conoce `transcript_path`. Al terminar una sesión, o al empezar la siguiente, se lee el archivo y se suma `message.usage`, con un lector tolerante como el de `stream.rs`. No pide configurar nada. Pero se rompe con cualquier versión del cliente sin que nadie se entere, y leer un archivo grande no entra en el plazo de un hook. Si se hace, tiene que ser en segundo plano en el daemon y no en el hook, y cada sesión tiene que quedar marcada con «formato desconocido» cuando el lector no reconoce lo que lee.

### C. No hacerlo y usar 96 (Alternativa)

agent-eval ya mide el costo de cada corrida con y sin clawg (`total_cost_usd` y `Usage` en `stream.rs`). Si la pregunta es «¿clawg ahorra?», 96 la contesta mejor, porque compara la misma tarea con y sin clawg. En el uso real no hay contrafactual: una sesión cara puede serlo por la tarea y no por clawg. El uso real contesta otra pregunta: cuánto gasta cada proyecto.

## Decisiones abiertas

- **Si se hace.** Depende de para qué se quieren los tokens. Para «cuánto gasta cada proyecto», va A. Para «¿clawg ahorra?», alcanza 96.
- **La autenticación del receptor**, si se hace A.
- **Si `setup` enciende la telemetría** o se documenta a mano.

## Esfuerzo e impacto

| Camino | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Recomendado si se hace | Medio: costo y tokens por sesión y proyecto, sólo en Claude Code | Medio (configuración del usuario, autenticación, puerto) | Medio |
| B | Descartado salvo que A no se pueda | Medio | Alto (formato sin documentar) | Bajo |
| C | Alternativa | Alto para la pregunta del ahorro | Bajo | Ya hecho |

Va en local: necesita Claude Code instalado con la telemetría encendida contra un daemon real. El receptor y su filtro se prueban en la nube con cuerpos OTLP escritos a mano.

## Fuentes

- Código de `develop` en `5fb9a5f`, con las rutas citadas.
- https://code.claude.com/docs/en/monitoring-usage, leída el 2026-11-08: métricas, eventos, atributos estándar y exportadores.
- https://code.claude.com/docs/en/sessions, «Where transcripts are stored», leída el 2026-11-08.
- https://code.claude.com/docs/en/statusline, «Available data», y https://code.claude.com/docs/en/hooks, «SessionStart», leídas el 2026-11-08.
- https://cursor.com/docs/agent/hooks, leída el 2026-11-08, a través de un resumen.
