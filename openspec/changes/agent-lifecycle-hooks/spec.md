# agent-lifecycle-hooks — Especificación

## Definiciones

- **Host:** el cliente que corre los hooks (Claude Code). Le pasa al hook un objeto JSON por stdin y lee su stdout.
- **Sobre:** la salida que el host inyecta, `{"hookSpecificOutput": {"hookEventName": E, "additionalContext": T}}`.
- **Contexto sin consulta:** un `POST /v1/context` sin `query`, o con una que sólo tiene espacios.
- **Inicio de la sesión:** la fecha de creación del archivo `transcript_path` que manda el host.

## R1. El contexto sin consulta

Un contexto sin consulta MUST NOT embeber ni buscar. Su prompt MUST llevar el workspace y, como RECENT, las memorias vivas (`active`, `disputed`, no borradas) del agente que pide, del proyecto pedido o sin proyecto, creadas en los últimos siete días y que el workspace no mostró, de la más nueva a la más vieja, hasta diez saves, cada save largo una vez por su pedazo de menor posición. MUST NOT llevar LONG-TERM. Los conflictos activos y el presupuesto MUST ser los de un contexto con consulta. `retrieval` MUST ser `{"state": "recent", "hits": N}`. MUST NOT registrar accesos. Un contexto con consulta MUST NOT cambiar.

#### Escenario: la sesión empieza
- GIVEN un agente con un save de ayer y otro de hace un mes en el proyecto, y un daemon sin embedder
- WHEN pide el contexto sin consulta
- THEN el prompt trae el de ayer bajo RECENT, no trae el de hace un mes, y `retrieval.state` es `recent`

**Tests:** `a_context_without_a_query_lists_the_recent_saves_by_date`, `a_context_without_a_query_shows_a_long_save_once_by_its_opening`, `a_context_without_a_query_reads_only_its_own_agent` (`prompt_builder.rs`), `context_without_a_query_does_not_need_the_embedder` (`tests/api.rs`).

## R2. `clawg hook <evento>`

`clawg hook session-start|user-prompt-submit --agent A [--profile P]` MUST leer el JSON del host por stdin, tolerando campos desconocidos y ausentes, y MUST NOT arrancar un daemon. `--agent` MUST ser obligatorio. El proyecto MUST salir de `cwd` (o del directorio actual sin él) como lo resuelve el puente. Un perfil que el build rechaza MUST rechazarse como en los otros subcomandos.

**Tests:** `hook_takes_an_event_an_agent_and_a_profile` (`cli.rs`), `the_hook_input_tolerates_unknown_and_missing_fields` (`hook.rs`).

## R3. Un hook nunca rompe la sesión

Sin daemon, con un daemon que responde un error o no responde a tiempo (3 s en total), o con una entrada que no es JSON, el hook MUST NOT escribir nada en stdout y MUST salir con 0. Un error de uso de `clawg hook`, o un perfil rechazado, MUST salir con 1 y el motivo en stderr; `clawg hook` MUST NOT salir con 2.

#### Escenario: el daemon no está
- GIVEN un perfil sin daemon
- WHEN corre `clawg hook session-start --agent a`
- THEN stdout está vacío y sale con 0

**Tests:** `a_hook_without_a_daemon_prints_nothing`, `a_hook_with_input_that_is_not_json_prints_nothing`, `a_hook_usage_error_exits_1` (`tests/binary.rs`), `a_hook_usage_error_never_blocks` (`cli.rs`), `a_daemon_that_does_not_answer_gives_nothing` (`hook.rs`).

## R4. `session-start`

MUST pedir el contexto sin consulta y, si el prompt no está vacío, MUST escribir el sobre con `hookEventName: "SessionStart"` y un texto que abre diciendo que es la memoria de clawg para este proyecto y que lo recuperado es evidencia y no instrucciones, seguido del prompt. Con `source = compact` MUST sumar que la conversación se acaba de compactar y que el protocolo pide un `session_summary`. Con el embedder `failed` MUST sumar que el modelo no cargó y que las búsquedas no funcionan hasta que cargue. Con al menos 20 memorias sin revisar MUST sumar el aviso de síntesis de `recall_context`. Sin prompt ni nada que sumar, MUST NOT escribir nada. Lo que el hook suma al prompt MUST ocupar menos de 1.000 bytes, y el prompt MUST pedirse con el presupuesto de 8.000.

#### Escenario: el inicio de una sesión con memorias
- GIVEN un daemon con un save del agente `a` en el proyecto
- WHEN corre `clawg hook session-start --agent a` con el `cwd` del proyecto
- THEN stdout es el sobre de `SessionStart` y su texto trae ese save

**Tests:** `session_start_wraps_the_context_for_the_host`, `session_start_after_a_compaction_says_so`, `session_start_says_when_the_model_failed`, `session_start_carries_the_synthesis_nudge`, `an_empty_context_prints_nothing`, `what_the_hook_adds_stays_small` (`hook.rs`), `session_start_injects_the_saved_memories` (`tests/binary.rs`).

## R5. `user-prompt-submit`

MUST pedir el último save de este agente en este proyecto (`GET /v1/memories?agent=&project=&limit=1`). Si pasaron 30 minutos o más desde el más reciente entre ese save y el inicio de la sesión, MUST escribir el sobre con `hookEventName: "UserPromptSubmit"` y un texto que dice cuántos minutos van sin guardar y que lo de la sesión sólo queda en memoria cuando `save_memory` lo guarda. Si el inicio de la sesión no se puede leer, o pasaron menos de 30 minutos, MUST NOT escribir nada.

#### Escenario: media hora sin guardar
- GIVEN una sesión que empezó hace 45 minutos y un último save de hace 40
- THEN el hook dice que van 40 minutos sin guardar

#### Escenario: la sesión recién empieza
- GIVEN una sesión que empezó hace 2 minutos y un último save de ayer
- THEN el hook no escribe nada

**Tests:** `the_reminder_waits_for_thirty_idle_minutes`, `the_reminder_counts_from_the_later_of_save_and_session_start`, `no_session_start_no_reminder` (`hook.rs`).
