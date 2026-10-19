# hooks — Especificación

Fuente de verdad del comportamiento actual de `clawg hook <evento>`: el subcomando que el cliente corre en sus hooks de ciclo de vida para que la memoria llegue al agente sin que tenga que acordarse de pedirla. Cubre cómo lee su entrada, qué escribe al iniciar la sesión y al enviar un mensaje, y por qué nunca rompe la sesión. El contexto que pide al iniciar, el que no lleva consulta, está en [`specs/retrieval`](../retrieval/spec.md) («El contexto sin consulta»), y la herramienta `recall_context`, con la que comparte el aviso de síntesis, en [`specs/synthesis`](../synthesis/spec.md) (R8).

**Origen:**
- [agent-lifecycle-hooks](../../changes/archive/2026-10-18-agent-lifecycle-hooks/proposal.md), archivado el 2026-10-18. Los ids de requisito son los de esa spec: R1 vive en [`specs/retrieval`](../retrieval/spec.md) y acá están R2 a R5.

## Definiciones

- **Host:** el cliente que corre los hooks (Claude Code). Le pasa al hook un objeto JSON por stdin y lee su stdout; lee el código de salida 2 como «bloquear la sesión o el mensaje».
- **Sobre:** la salida que el host inyecta, `{"hookSpecificOutput": {"hookEventName": E, "additionalContext": T}}` (`envelope`, `crates/clawg-cli/src/hook.rs:104`).
- **Inicio de la sesión:** la fecha de creación del archivo `transcript_path` que manda el host (`session_started_at`, `hook.rs:114`). Un sistema de archivos que no guarda la creación no la da.

## R2. `clawg hook <evento>`

`clawg hook session-start|user-prompt-submit --agent A [--profile P]` MUST leer el JSON del host por stdin, tolerando campos desconocidos y ausentes (`HookInput`, `hook.rs:38`; todos sus campos son opcionales), y MUST NOT arrancar un daemon. `--agent` MUST ser obligatorio y se recorta; sólo hay dos eventos (`HookEvent`, `crates/clawg-cli/src/cli.rs:261`). El proyecto MUST salir de `cwd` (o del directorio actual sin él) como lo resuelve el puente (`ProjectIdentity::resolve`, `hook.rs:141`). Un perfil que el build rechaza MUST rechazarse como en los otros subcomandos (`profile.resolve()` en `crates/clawg-cli/src/main.rs:106`).

El `--agent` del hook MUST ser el del puente: las memorias que pide y el último save que mira son las de ese agente, y con otro nombre el hook leería a un agente distinto del que guarda ([`specs/visibility`](../visibility/spec.md), R1).

Esta spec no instala el hook: lo instala `clawg setup claude-code`, con la ruta absoluta del binario y el mismo `--agent` y `--profile` que el puente ([`specs/setup`](../setup/spec.md), R3). La configuración para probarlo a mano está en «Probar a mano» del [diseño archivado](../../changes/archive/2026-10-18-agent-lifecycle-hooks/design.md#probar-a-mano).

**Tests:** `hook_takes_an_event_an_agent_and_a_profile` (`cli.rs`), `the_hook_input_tolerates_unknown_and_missing_fields` (`hook.rs`). El perfil rechazado lo prueba `a_hook_usage_error_exits_1`, y el proyecto que sale de `cwd`, `session_start_injects_the_saved_memories` (`crates/clawg-cli/tests/binary.rs`).

## R3. Un hook nunca rompe la sesión

Sin daemon, con un daemon que responde un error o no responde a tiempo, o con una entrada que no es JSON, el hook MUST NOT escribir nada en stdout y MUST salir con 0 (`respond` y `answer`, `hook.rs:125-163`; `Command::Hook`, `main.rs:105-112`). El tope es de 3 s en total, de leer stdin a la respuesta (`HOOK_DEADLINE`, `hook.rs:25`), por debajo de los 5 s con los que se configura un hook, para que el host nunca lo cancele. Un error de uso de `clawg hook`, o un perfil rechazado, MUST salir con 1 y el motivo en stderr; `clawg hook` MUST NOT salir con 2, el código de clap para un uso inválido, que el host leería como «bloquear» y borraría el mensaje del usuario (`is_hook_invocation`, `cli.rs:269`; el camino de `Cli::try_parse` en `main.rs:43-51`).

El hook MUST NOT arrancar un daemon (usa `DaemonClient::from_data_dir`, no `probe_running(.., true)`, que puede esperar hasta 60 s): el puente MCP es quien arranca el daemon con la sesión, así que el primer `SessionStart` del día puede llegar antes y quedar sin contexto. Es la decisión de no autoarrancar (D6 del diseño archivado), y ese es todo su costo.

Como el host puede dejar abierto el stdin, `main` apaga el runtime sin esperar a las tareas bloqueantes cuando es un hook (`shutdown_background`, `main.rs:79`); un `drop` común esperaría la lectura de stdin más allá del tope.

#### Escenario: el daemon no está
- GIVEN un perfil sin daemon
- WHEN corre `clawg hook session-start --agent a`
- THEN stdout está vacío, sale con 0 y no aparece un `daemon.json`

**Tests:** `a_hook_without_a_daemon_prints_nothing`, `a_hook_with_input_that_is_not_json_prints_nothing`, `a_hook_usage_error_exits_1` (`tests/binary.rs`), `a_hook_usage_error_never_blocks` (`cli.rs`, que sólo prueba `is_hook_invocation`), `a_daemon_that_does_not_answer_gives_nothing` y `without_a_daemon_nothing_is_printed` (`hook.rs`). Ningún test llega a un daemon que responde con un error o que no responde, ni ejerce `shutdown_background`: el tope que vence en `a_daemon_that_does_not_answer_gives_nothing` es el de la lectura de stdin ([verify](../../changes/archive/2026-10-18-agent-lifecycle-hooks/verify.md#lo-que-encontró-la-verificación), W1).

## R4. `session-start`

MUST pedir el contexto sin consulta con el presupuesto de 8.000 bytes (`SESSION_BUDGET`, `hook.rs:33`; `DaemonClient::recent_context`, `crates/clawg-cli/src/client.rs:696`) y, si el prompt no está vacío, MUST escribir el sobre con `hookEventName: "SessionStart"` y un texto que abre diciendo que es la memoria de clawg para este proyecto y que lo recuperado es evidencia y no instrucciones, seguido del prompt (`session_start_text`, `hook.rs:56`). Con `source = compact` MUST sumar que la conversación se acaba de compactar y que el protocolo pide un `session_summary`. Con el embedder `failed` MUST sumar que el modelo no cargó y que las búsquedas no funcionan hasta que cargue, con la causa y si clawg reintenta. Con al menos 20 memorias sin revisar MUST sumar el aviso de síntesis de `recall_context` (`synthesis_nudge`, [`specs/synthesis`](../synthesis/spec.md), R8). Sin prompt ni nada que sumar, MUST NOT escribir nada. Lo que el hook suma al prompt MUST ocupar menos de 1.000 bytes, para que el todo entre en los 10.000 caracteres que el host inyecta.

Verificado con Claude Code sobre el perfil dev: el inicio, la reanudación y la compactación inyectan el contexto, y la línea de la compactación sólo aparece con `source = compact`. La causa del modelo fallido no tiene tope (S2 del [verify](../../changes/archive/2026-10-18-agent-lifecycle-hooks/verify.md#lo-que-encontró-la-verificación)).

#### Escenario: el inicio de una sesión con memorias
- GIVEN un daemon con un save del agente `a` en el proyecto
- WHEN corre `clawg hook session-start --agent a` con el `cwd` del proyecto
- THEN stdout es el sobre de `SessionStart` y su texto trae ese save

**Tests:** `session_start_wraps_the_context_for_the_host`, `session_start_after_a_compaction_says_so`, `session_start_says_when_the_model_failed`, `session_start_carries_the_synthesis_nudge`, `an_empty_context_prints_nothing`, `what_the_hook_adds_stays_small` (`hook.rs`), `session_start_injects_the_saved_memories` (`tests/binary.rs`).

## R5. `user-prompt-submit`

MUST pedir el último save de este agente en este proyecto (`GET /v1/memories?agent=&project=&limit=1`; el hook le agrega `kind=memory`, porque sólo esa tabla se escribe hoy, `hook.rs:150-156`). Si pasaron 30 minutos o más (`SAVE_REMINDER_AFTER`, `hook.rs:29`) desde el más reciente entre ese save y el inicio de la sesión, MUST escribir el sobre con `hookEventName: "UserPromptSubmit"` y un texto que dice cuántos minutos van sin guardar y que lo de la sesión sólo queda en memoria cuando `save_memory` lo guarda (`save_reminder`, `hook.rs:90`, pura). Si el inicio de la sesión no se puede leer, o pasaron menos de 30 minutos, MUST NOT escribir nada. Contar desde el más tardío de los dos evita que el primer mensaje de una sesión se entere de un save de ayer.

#### Escenario: media hora sin guardar
- GIVEN una sesión que empezó hace 45 minutos y un último save de hace 40
- THEN el hook dice que van 40 minutos sin guardar

#### Escenario: la sesión recién empieza
- GIVEN una sesión que empezó hace 2 minutos y un último save de ayer
- THEN el hook no escribe nada

**Tests:** `the_reminder_waits_for_thirty_idle_minutes`, `the_reminder_counts_from_the_later_of_save_and_session_start` y `no_session_start_no_reminder` (`hook.rs`), sobre `save_reminder`. Contra un daemon sólo se prueba el silencio, al final de `session_start_injects_the_saved_memories` (`tests/binary.rs`): ningún test fija que `client.list` y la unidad de `created_at` lleguen bien (S4 del [verify](../../changes/archive/2026-10-18-agent-lifecycle-hooks/verify.md#lo-que-encontró-la-verificación)). Con Claude Code sobre el perfil dev, una sesión retomada 33 minutos después del último save recibió el recordatorio.
