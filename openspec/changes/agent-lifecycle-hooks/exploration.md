# agent-lifecycle-hooks — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1, 5 | M | memory-protocol (22), context-budget (23), conflict-candidates (24) | exploración (borrador) |

**Tesis:** hoy la memoria sólo funciona si el agente se acuerda de llamarla. Una sesión empieza ciega salvo que el modelo decida llamar a `recall_context`; después de una compactación el cliente pierde lo que se había recordado; y la síntesis (objetivo 5) depende de que un recordatorio dentro de la respuesta de `recall_context` llegue a leerse. El host (Claude Code) ofrece hooks de ciclo de vida que corren **sin pasar por el modelo**. Un subcomando `clawg hook <evento>` puede usarlos para empujar la memoria en los momentos justos, sin depender de la disciplina del agente y sin scripts por plataforma.

## Problema (verificado contra el código)

### Qué empuja hoy al agente a usar la memoria

- Un único texto de instrucciones del servidor MCP, fijo, que el host muestra en `initialize` (`crates/clawg-cli/src/mcp/server.rs:108-119`). Dice cuándo guardar, buscar, recordar y sintetizar. Es una sugerencia: el modelo decide si la sigue.
- Un único recordatorio dentro de una respuesta: `recall_context` agrega «N memories here have not been reviewed… call `synthesize`» cuando `unreviewed >= 20` (`crates/clawg-cli/src/mcp/tools.rs:256-278`, umbral en `:259`). Sólo aparece **si** el modelo ya llamó a `recall_context`. Quien nunca lo llama nunca lo ve.
- No hay nada más. `rg "hook"` sobre `crates/clawg-cli/src` sólo da un comentario de `user_path.rs:9` sobre el hook del instalador. **No existe soporte de hooks de host.**

### No hay subcomando para un hook

- El enum `Command` tiene `Serve`, `Mcp`, `Status`, `Stop`, `Backup`, `Restore`, `Secrets`, `Purge`, `Sharing`, `Doctor`, `Memories` y `Path` (`crates/clawg-cli/src/cli.rs:31-126`). Ninguno lee JSON del host por stdin ni escribe una salida para el host.

### Cada sesión empieza ciega

- `recall_context` exige un `query` (`mcp/server.rs:168-170`; `ContextRequest { query, agent_id, project_id }`, `crates/clawg-cli/src/daemon/api.rs:340-345`). Al arrancar una sesión no hay consulta del usuario todavía. **No verifiqué** cómo se comporta `build_memory_prompt` con una consulta vacía (no hay ramas sobre `query` vacío en `prompt_builder.rs`). Es una pregunta abierta de diseño, no un hecho.
- `POST /v1/context` no tiene ningún parámetro de tamaño (`api.rs:340-345`). El texto que devuelve no tiene tope duro; eso lo resuelve context-budget (23). Inyectado en cada arranque, ese texto se paga en cada sesión.
- La respuesta de contexto ya se arma con bloques WORKSPACE, RECENT, LONG-TERM y ACTIVE CONFLICTS (`crates/clawg-core/src/memory/prompt_builder.rs:21-25`, ensamblado en `:236-250`).

### Los conflictos activos hoy llegarían a cada arranque

- `ACTIVE CONFLICTS (review needed)` forma parte del prompt (`prompt_builder.rs:24`), y las instrucciones del servidor le piden al modelo que le pregunte al usuario cuál memoria vale (`mcp/server.rs:112-113`). Inyectar ese bloque en cada inicio de sesión haría que el agente interrogue al usuario **en cada sesión** mientras el conflicto siga abierto. Por eso conflict-candidates (24) va antes: sólo los pares de alto riesgo deben llegar al usuario.

### El cliente HTTP ya sabe encontrar el daemon

- `DaemonClient::from_data_dir` lee `daemon.json` y `token` y arma una URL siempre loopback (`crates/clawg-cli/src/client.rs:429-444`). Como cada hook es un proceso nuevo, lee los dos archivos en cada evento: no le pasa lo del puente, que se queda con el token del primer daemon (bridge-reconnect, 39).
- `probe_running(base_dir, autostart)` decide si hay daemon, si hay que arrancarlo o si otro proceso tiene la base (`client.rs:937-966`). El puente MCP pasa `autostart = !no_autostart` (`crates/clawg-cli/src/main.rs:73-75`, `mcp/mod.rs:52`). **Confirmado:** sin `--no-autostart` el puente arranca el daemon (`client.rs:968-994`, espera hasta `AUTOSTART_TIMEOUT` de 60 s, `client.rs:40`). Con la bandera nunca lo lanza (test en `client.rs:1384-1398`).
- Salud con tope de 1 s: `HEALTH_TIMEOUT` (`client.rs:44`). `/health` informa `embedder: "loading" | "ready" | "failed"` (`api.rs:611-623`).
- La resolución de perfil sigue `--profile`, luego `CLAWG_PROFILE`, luego el tipo de build (`crates/clawg-core/src/paths.rs:73`; en `cli.rs:390` `ProfileArg::resolve`). Un hook que reuse `ProfileArg` hereda la regla de que un build debug rechaza `prod`.

### Qué datos tiene el daemon para «hace rato que no guardás»

- La tabla `memory` guarda `agent_id`, `project_id` y `created_at` por fila (`crates/clawg-core/migrations/V1__init.sql:8-13`).
- `GET /v1/memories` acepta `agent`, `project`, `memory_type` y `limit` (`api.rs:352-372`) y ordena por `created_at DESC` (`crates/clawg-core/src/memory/admin.rs:636`). `limit=1` con `agent` y `project` responde «cuándo guardó por última vez este agente en este proyecto». **No hace falta ningún estado nuevo** en disco ni en el daemon.

### Identidad del agente en un hook

- El puente fija el agente con `--agent`, o con `clientInfo.name`, o `mcp` (`mcp/server.rs:84-92`). Un hook **no recibe** `clientInfo`. Si el hook usara otro id, leería y mediría otro «agente» que el del puente. El hook necesita el mismo `--agent` que el puente (hoy sólo es fiable si ambos lo fijan; lo instalará client-setup, 26).

## Contrato del host (Claude Code), verificado

Fuente: https://code.claude.com/docs/en/hooks (leída en esta exploración; el mismo contenido vive bajo docs.claude.com).

| Hecho | Estado |
|---|---|
| Evento `SessionStart`, matchers `startup`, `resume`, `clear`, `compact`, `fork` | verificado |
| Campos comunes por stdin: `session_id`, `transcript_path`, `cwd`, `hook_event_name` (también `prompt_id`, `permission_mode`, …) | verificado |
| `SessionStart` agrega `source` y puede traer `model`; `startup` también corre con `claude --init-only` | verificado |
| `SessionStart` admite sólo hooks `command` y `mcp_tool` | verificado |
| Salida: `hookSpecificOutput.additionalContext` (con `hookEventName`) y, en `SessionStart` y `UserPromptSubmit`, el stdout en texto plano también entra al contexto | verificado |
| `additionalContext`: tope de 10 000 caracteres; más largo se escribe a un archivo y Claude recibe la ruta y 2 000 caracteres de vista previa | verificado |
| La doc recomienda afirmaciones factuales en vez de órdenes imperativas tipo sistema (riesgo de defensa contra inyección) | verificado |
| Códigos de salida: 0 éxito (stdout se parsea como JSON si es un objeto), 2 error bloqueante, otro = error no bloqueante que sólo muestra la primera línea de stderr | verificado |
| `timeout` por hook en segundos. Por defecto 600 para `command`; **30** para `UserPromptSubmit`. Un hook que vence se cancela y su salida se descarta | verificado |
| `UserPromptSubmit`: sin matcher, recibe `prompt`; puede inyectar `additionalContext` o bloquear | verificado |
| `PreCompact`: matchers `manual`/`auto`; recibe `trigger` y `custom_instructions`; puede **bloquear** la compactación; la doc **no** lista `additionalContext` para este evento; descarta `systemMessage` y `continue` | verificado |
| `PostCompact`: recibe `trigger` y `compact_summary`; **sin control de decisión**; descarta `systemMessage` y `continue` | verificado |
| `SessionEnd`: matchers `clear`, `resume`, `logout`, `prompt_input_exit`, `other`; presupuesto de 1,5 s por defecto compartido | verificado |
| Dónde se configuran: `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`, política gestionada, `hooks/hooks.json` de un plugin, frontmatter de skills y subagentes; las entradas se **combinan** entre niveles | verificado |
| Los hooks `mcp_tool` de `SessionStart` al lanzar se saltan (los servidores MCP aún no están disponibles); los de `/clear` o compactación sí corren | verificado |
| Hooks `http` (POST del JSON a una URL) y `command` (JSON por stdin) | verificado en el índice de tipos; los campos de configuración de `http` **sin verificar** |
| Variable de entorno `CLAUDE_CODE_SESSION_ID` en subprocesos de hooks | **sin verificar en la doc oficial** (la mencionan sólo reportes de terceros) |

Consecuencia: **no existe forma documentada de inyectar contexto antes de compactar.** Para «volver a inyectar después de compactar» el evento es `SessionStart` con `source = compact`. `PreCompact` sólo puede bloquear; `PostCompact` sólo puede ejecutar efectos laterales.

## Lo que ya existe y se reutiliza

- `DaemonClient` y el descubrimiento por `daemon.json` + `token` (`client.rs:429-444`), `probe_running` y `ensure_running` (`client.rs:904-966`) y los métodos `context`, `list`, `save`, `health` (`client.rs:490-560`, `:595`).
- `ProfileArg` y la resolución de perfil (`cli.rs:390`, `paths.rs:73`).
- `ProjectIdentity::resolve(&cwd)` para derivar el proyecto desde el `cwd` del hook (`crates/clawg-core/src/memory/identity.rs:29`; el mismo uso ya está en `memories.rs:349`).
- `format_recall` y `synthesis_nudge` (`mcp/tools.rs:259-278`): el texto del recordatorio y su umbral, hoy atados al MCP.
- `unreviewed_count` ya calculado por `POST /v1/context` (`api.rs:862-867`).
- Estado del embedder en `/health` para decir «cargando» con honestidad (`api.rs:611-623`).
- El patrón de lógica pura separada de E/S (`cli.rs`, `Profile::resolve`).

## Enfoques posibles

### E1. `clawg hook <evento>` en Rust (recomendado)

Un subcomando por evento (`session-start`, `user-prompt-submit`; `post-compact` según decisión). Lee el JSON del host por stdin, resuelve perfil, llama al daemon por HTTP con el `DaemonClient` y escribe la salida del host por stdout.

- **Núcleo puro:** una función `decide(evento, entrada, hechos_del_daemon) -> salida` sin E/S. Los hechos son: estado del embedder, texto de contexto, hora del último guardado, `unreviewed`. El borde de E/S sólo reúne esos hechos y escribe.
- **Fail-open:** cualquier error (sin daemon, timeout, 401, JSON inválido) imprime nada y sale con 0. Un hook nunca debe romper ni retrasar la sesión.
- **A favor:** un solo binario en todas las plataformas, JSON parseado con serde, la misma resolución de perfil y token que el puente, testeable sin red.
- **En contra:** el proceso arranca en cada evento (costo de arranque de un binario Rust; sin medir). El host debe tener `clawg` en el PATH o usar ruta absoluta.

### E2. Hooks `http` contra el daemon

El host haría `POST` directo al daemon.

- **A favor:** sin proceso intermedio.
- **En contra:** el daemon exige bearer token (`api.rs:538-558`) y escucha en un puerto que cambia por perfil; el archivo de configuración del host tendría que contener el token y el puerto. Los campos de configuración de `http` están **sin verificar**. Descartado como primera opción.

### E3. Hook `mcp_tool` que llama a una herramienta del propio servidor

Evita un proceso nuevo, pero: se salta en el `startup` (los servidores MCP aún no están listos) y la herramienta tendría que existir en la lista que el modelo ve (costo de contexto fijo por sesión). No sirve para el caso principal.

### E4. Pie de recordatorio en las respuestas de `search_memory` / `recall_context`

Para clientes sin hooks. `format_recall` ya agrega una línea (`tools.rs:272-278`); se podría ampliar a `search_memory`.

- **A favor:** funciona con cualquier cliente MCP.
- **En contra:** sólo aparece cuando el modelo ya usa la memoria, que es justo el caso que no hace falta empujar; suma tokens a cada respuesta. Es un complemento acotado, no un reemplazo.

### Eventos: qué hace cada uno

| Evento | Qué inyecta | Notas |
|---|---|---|
| `SessionStart` `startup`/`resume`/`clear` | Protocolo corto (22) + contexto con tope (23) | Si el embedder está `loading`, decirlo. Con `failed`, decirlo y no inyectar contexto. |
| `SessionStart` `compact` | Lo mismo, más la pauta de dejar el estado actual guardado | La compactación ya ocurrió: el modelo conserva el resumen del host, no el detalle. Pedirle «guardá un `session_summary` de lo compactado» llega tarde; lo que ve es lo que el resumen conservó. |
| `UserPromptSubmit` | Un recordatorio, sólo si pasaron N minutos sin guardado de este agente en este proyecto y hay enfriamiento | En el primer prompt de la sesión, nada, salvo que haga falta (por ejemplo `failed`). Plazo por defecto del host: 30 s. |
| `PostCompact` | Nada al modelo (no hay `additionalContext`). Como efecto lateral, el hook podría guardar `compact_summary` como `session_summary` | Opción A5 abajo. |
| `SessionEnd` | Nada. Presupuesto de 1,5 s | Se descarta: no puede inyectar y el presupuesto es demasiado corto para una llamada al daemon fiable. |

### Opción A5. Guardar `compact_summary` desde el hook

`PostCompact` recibe el resumen generado por el host. Un hook que lo guarde como `session_summary` conserva el hilo **sin depender del modelo**.

- **A favor:** no depende de que el agente obedezca una instrucción; el resumen existe justo en ese momento.
- **En contra:** es texto que el modelo no escribió pensando en la memoria, puede ser largo y puede traer secretos del chat (el enmascarado de `crates/clawg-core/src/memory/redact.rs` corre en el ingest, pero el volumen y la calidad son un riesgo). Es una escritura que el usuario no pidió. Decisión del usuario; aquí queda como opción, no como recomendación.

### Autoarranque del daemon en el hook

- **Con autoarranque** (`probe_running(.., true)`): el inicio de sesión puede bloquearse hasta 60 s (`client.rs:40`) si el daemon tarda. Inaceptable para un hook síncrono.
- **Sin autoarranque** (recomendado): si no hay daemon, el hook no imprime nada. El puente MCP, que arranca junto con la sesión, sí lo arranca (`main.rs:73-75`), de modo que a partir de la primera herramienta el daemon existe. Costo: el primer `SessionStart` puede llegar antes que el daemon y quedar sin contexto. Alternativa intermedia: autoarrancar sin esperar (lanzar y salir), a decidir.
- Con el embedder en `loading` el daemon ya responde (`/health` lo informa, `api.rs:411-419`); el hook lo dice en vez de callar.

## Descartado

- **Lógica de hook duplicada en scripts por plataforma (bash y PowerShell) con parseo de JSON a mano.** Hay que mantener dos implementaciones, el parseo de JSON en shell es frágil con comillas, saltos de línea y Unicode, y la resolución de perfil y token se reescribiría fuera del código probado. Un binario Rust cubre ambas plataformas con un solo camino.
- **Archivos de estado por hook** (última hora de guardado, enfriamiento). El daemon ya tiene el dato (`created_at` por agente y proyecto, `admin.rs:636`); un archivo aparte puede divergir, hay que limpiarlo, y no soporta sesiones simultáneas. El enfriamiento puede salir del mismo dato: si el último guardado es reciente, no se recuerda.
- **`PreCompact` como punto de inyección.** La doc no lista `additionalContext` para él; sólo puede bloquear. Bloquear la compactación del usuario para forzar un guardado es una interferencia desproporcionada.
- **`SessionEnd`.** Sin control de decisión y con 1,5 s de presupuesto compartido.
- **Inyectar el contexto en cada `UserPromptSubmit`.** Paga el tamaño de 23 en cada turno, y lo que agrega después del primero es casi nada, porque el contexto de la sesión ya lo tiene. El recordatorio con enfriamiento cubre el caso sin ese costo.

## Alcance candidato

1. Subcomando `clawg hook <evento>` en `crates/clawg-cli` (parser en `cli.rs` con `ProfileArg` y `--agent`), con un núcleo puro `decide` y un borde de E/S fino.
2. `session-start`: protocolo (22) + contexto con tope (23); mensaje explícito si el embedder está `loading` o `failed`; sin autoarranque.
3. Distinguir `source`: `startup`/`resume`/`clear` frente a `compact`.
4. `user-prompt-submit`: recordatorio por tiempo sin guardado con enfriamiento, y reemplazo oportuno del aviso de síntesis cuando `unreviewed` supere el umbral. Primer prompt sin salida.
5. Fail-open con un tope de tiempo corto (el tope propio del hook, menor que el del host).
6. Documentar cómo configurarlo **a mano para probar** (abajo, marcado).
7. Pruebas del núcleo puro y de integración contra un daemon efímero (`--port 0`).
8. `CLAUDE.md` del repo: el comando nuevo.

### Configuración manual para pruebas (NO es la instalación)

Fragmento de `.claude/settings.local.json` (no se commitea). Sólo para probar a mano; la instalación real es de client-setup (26). El nombre y las banderas dependen de lo que se implemente.

```json
{
  "hooks": {
    "SessionStart": [
      { "matcher": "startup|resume|clear|compact",
        "hooks": [ { "type": "command", "command": "clawg hook session-start --agent claude-code", "timeout": 5 } ] }
    ],
    "UserPromptSubmit": [
      { "hooks": [ { "type": "command", "command": "clawg hook user-prompt-submit --agent claude-code", "timeout": 5 } ] }
    ]
  }
}
```

El valor de `--agent` debe coincidir con el del puente. En una compilación de desarrollo hay que pasar `--profile dev`.

### No-alcance

- Instalar los hooks, tocar `~/.claude` o registrar el servidor MCP (client-setup, 26).
- El texto del protocolo (22) y el presupuesto de contexto (23): este cambio los consume.
- La detección de conflictos en sí (24).
- Hooks de otros clientes distintos de Claude Code.
- Captura de prompts del usuario (ver session-handoff, 28).
- Estado por sesión (ver session-handoff).

## Decisiones abiertas

Son del usuario.

- [ ] **Forma de la salida:** JSON con `hookSpecificOutput.additionalContext` o texto plano por stdout. Ambos entran al contexto en `SessionStart` y `UserPromptSubmit`; el JSON permite además `hookEventName` explícito.
- [ ] **Autoarranque:** ninguno (recomendado), lanzar sin esperar, o esperar con tope.
- [ ] **Consulta de `session-start`:** `POST /v1/context` exige `query`. ¿Se envía el nombre del proyecto, una cadena fija, o se agrega un modo sin consulta en el daemon? Verificar antes el comportamiento con consulta vacía.
- [ ] **Tras compactar:** ¿sólo re-inyectar con `SessionStart compact`, o además guardar `compact_summary` desde `PostCompact` (A5)?
- [ ] **Recordatorio por tiempo:** N minutos (valor inicial a medir), enfriamiento y qué cuenta como «guardado» (¿cualquier `memory_type`?).
- [ ] **Aviso de síntesis:** ¿se mueve al hook y se saca de `format_recall`, o conviven?
- [ ] **Pie de recordatorio (E4):** ¿se agrega para clientes sin hooks o se descarta?
- [ ] **Registro de proyecto:** ¿el hook registra la raíz (`POST /v1/projects`, que también dispara el índice en segundo plano) o sólo la deriva de `cwd`?
- [ ] **Texto imperativo o factual:** la doc del host recomienda afirmaciones factuales. ¿Se redacta el recordatorio así?

## Cómo se mide

Tests RED primero:

1. Núcleo puro: para cada combinación de evento y `source`, con y sin daemon, con el embedder `loading`/`ready`/`failed`, `decide` devuelve la salida esperada (tabla de casos).
2. Fail-open: daemon ausente, 401, timeout y JSON de entrada inválido producen stdout vacío y código 0.
3. Parseo de la entrada: `session_id`, `cwd`, `source`, `prompt` presentes; campos desconocidos no rompen el parseo (el host agrega campos con el tiempo).
4. `UserPromptSubmit`: sin guardados recientes y fuera de enfriamiento, recuerda; con un guardado reciente, no; en el primer prompt, no.
5. La salida de `session-start` no supera el tope de 23 (y es menor que los 10 000 caracteres del host).
6. Con `failed`, el texto lo dice y no hay contexto inyectado.
7. Perfil: un build debug rechaza `--profile prod` también en `hook` (igual que el puente).
8. Integración: un daemon efímero con memorias sembradas; `hook session-start` devuelve el contexto esperado.
9. Los conflictos activos no se inyectan con el comportamiento previo a 24 (test que fija la dependencia).

`rag-eval`: sólo si se toca el contenido o la selección de `recall_context` o `/v1/context` (por ejemplo un modo sin consulta). Si el hook sólo consume lo que 23 ya entrega, no es requisito. Si se agrega un modo sin consulta, correr `rag-eval --diff` contra la línea base de `openspec/config.yaml`.

Esquema, ids, estados y contrato de embedding: **no cambian** en este cambio, por lo que no hace falta base sembrada en versión previa.

## Riesgos

- **Costo de contexto en cada sesión:** lo inyectado se paga siempre. Depende del tope de 23; sin él, este cambio empeora el consumo.
- **Retraso o rotura del inicio de sesión:** un hook lento bloquea. Mitigación: sin autoarranque, tope corto, fail-open.
- **Interrogatorio por conflictos:** inyectar `ACTIVE CONFLICTS` antes de 24 hace que el agente pregunte en cada sesión.
- **Agente distinto del puente:** si el `--agent` del hook no coincide con el del puente, el hook lee y mide otro agente y el recordatorio sale mal. Se cubre en 26.
- **Instrucciones imperativas filtradas como inyección:** la doc advierte que órdenes tipo sistema pueden activar defensas del modelo. Redactar de forma factual.
- **Compactación:** no hay hook documentado que permita al modelo guardar antes de perder detalle.
- **Variable `CLAUDE_CODE_SESSION_ID`:** no confirmada en la doc oficial; no apoyarse en ella sin verificar.
- **Cambio de contrato del host:** los eventos y campos pueden cambiar. La entrada se parsea de forma tolerante y un campo ausente no debe romper el hook.
