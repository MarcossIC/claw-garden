# cursor-review — Plan

Tamaño: grande (guarda valores nuevos en `metrics.db` y hay más de un enfoque razonable).

Corre en: local. La suite usa un CLI de Cursor falso; la comprobación con el CLI real necesita Cursor instalado y con sesión iniciada, y corre contra un home temporal. · Necesita a la persona: no

## Problema

La revisión de un cambio la hace hoy un agente de Claude (`change-reviewer`, `.claude/agents/change-reviewer.md:5`, `model: opus`). Un cambio mediano lleva uno y un cambio grande dos en paralelo (`.claude/skills/change-workflow/SKILL.md:76`). Los modelos que trae una suscripción de Cursor (GPT, Gemini, Grok, Codex) no entran en ningún paso del flujo, y clawg no tiene forma de lanzar a otro agente ni de registrar la sesión de un revisor.

Lo que clawg registra hoy de una sesión:

- una fila por sesión en `metrics.db` (`SessionRow`, `crates/clawg-core/src/metrics/sessions.rs:80-96`);
- contadores por tipo y nombre: `start`, `call`, `failed_call` y `save` (`CountKind`, `sessions.rs:51-76`).

No hay tokens, modelo ni veredicto. El cliente es `claude-code` o `cursor` (`CLIENTS`, `sessions.rs:37`).

Cómo se registra y se cuenta una sesión:

- `POST /v1/sessions` la abre y `POST /v1/sessions/end` la cierra (`crates/clawg-cli/src/daemon/api/sessions.rs:80-110`).
- Las llamadas del puente se cuentan para la sesión registrada de su agente y del proyecto de su carpeta abierta (`count_tool_call`, `api/sessions.rs:124-131`; `SessionRegistry::current`, `crates/clawg-cli/src/daemon/sessions.rs:84-87`).
- Esa carpeta sale de los `roots` del cliente. Si el cliente no declara ninguno, sale del directorio donde arrancó el puente, salvo que sea el home (`open_folders`, `crates/clawg-cli/src/mcp/roots.rs:131-149`; `default_project`, `crates/clawg-cli/src/mcp/server.rs:578-580`).
- El lector de `metrics.db` salta un tipo de contador que no conoce (`crates/clawg-core/src/metrics/store.rs:404-407`), y ningún test lo fija.

Lo que se comprobó del CLI de Cursor (`agent 2026.11.22-e9042b2`, Windows), en corridas con `USERPROFILE` y `HOME` en una carpeta de prueba:

- El login sigue valiendo, y el CLI lee y escribe el `.cursor` de esa carpeta.
- `agent -p --output-format stream-json` escribe una línea JSON por evento:
  - la primera, `{"type":"system","subtype":"init","session_id":…,"model":"Gemini 3.8 Flash Low",…}`;
  - la última, `{"type":"result","is_error":false,"duration_ms":…,"result":"<texto>","session_id":…,"usage":{"inputTokens":…,"outputTokens":…,"cacheReadTokens":…,"cacheWriteTokens":…}}`, a veces con `reasoningTokens`.

  `usage` no está en la documentación de Cursor (https://cursor.com/docs/cli/reference/output-format) y es de esa corrida: una ronda con `--resume` dio 6123 tokens de entrada frente a 22038 de la primera.
- El prompt se puede pasar por stdin y llega entero, con comillas, `&`, `|`, `%`, `^` y saltos de línea. En Windows el CLI es `%LOCALAPPDATA%\cursor-agent\agent.cmd`, que pasa sus argumentos por `cmd` y PowerShell.
- El proceso MCP de `mcp.json`:
  - arranca con su directorio de trabajo en la carpeta del workspace, no en el home movido;
  - recibe un `initialize` con `clientInfo` `{"name":"Cursor","version":"1.0.0"}` y sin la capacidad `roots`;
  - a un pedido `roots/list` le llega `{"code":-32601,"message":"Method not found"}`;
  - recibe un entorno filtrado, con el `USERPROFILE` movido. Una variable que `mcp.json` declara en `env` le llega; una que no declara, no.
- `--mode=ask` y `--mode=plan` no dejan correr ningún comando, así que el revisor no podría leer `git diff`.
- En el modo normal, sin `--force`, `.cursor/cli-config.json` con `permissions.allow` = `Shell(git log)` y `Shell(git diff)` y `deny` = `Write(**)`:
  - deja correr `git log` y `git diff`;
  - rechaza `git status`, `git commit` y `git log … && echo … > x`;
  - niega una edición con `writePermissionDenied`.
- Una herramienta MCP se rechaza en `-p` salvo que `permissions.allow` la nombre como `Mcp(<servidor>:<herramienta>)` (se probó `Mcp(<servidor>:*)`). `--approve-mcps` sólo arranca el servidor.
- `--resume <session_id>` retoma el mismo chat con su contexto, con el mismo `session_id`.
- El CLI corre los hooks de `.cursor/hooks.json`. Lanzado desde Git Bash, corre con bash los comandos que arma para PowerShell, y fallan.

## Objetivo y fuera de alcance

Objetivo 4: varios agentes leen y escriben el mismo cerebro. Un revisor de Cursor revisa un cambio leyendo la memoria del proyecto, y su sesión queda registrada como la de cualquier agente, con sus llamadas, su modelo, sus tokens y su veredicto. El orquestador, la sesión que planifica e implementa, lo lanza con `clawg review`, lee el informe y decide qué hacer con él.

Fuera de alcance:

- La configuración de Cursor del usuario (`~/.cursor/mcp.json`, `hooks.json`, `cli-config.json`) y `clawg setup cursor` no cambian.
- Contar los tokens de Claude Code: va en un cambio aparte que retoma `openspec/changes/archive/2026-11-22-session-tokens/exploration.md`.
- Otros CLIs de agentes (Codex CLI, Gemini CLI).
- Un plazo para la corrida de Cursor: cortar `agent.cmd` deja vivos los procesos que lanzó. La skill dice que la revisión corre en segundo plano.
- La revisión del plan sigue con `change-reviewer`. Sólo la revisión del cambio terminado pasa a Cursor.
- `memory.db`, el ranking, el contrato MCP fijado y el puente no cambian.

## Hecho

Las instrucciones:

- H1. Dado un archivo de instrucciones con frontmatter (`---` … `---`) y un mensaje, cuando se arma el prompt, entonces MUST ser el cuerpo sin el frontmatter, una línea en blanco y el mensaje → `review::tests::the_prompt_is_the_instructions_without_frontmatter_and_the_message`

El veredicto:

- H2. Dado un informe cuya última línea no vacía es `CLEAN`, `WARNINGS` o `BLOCKED` (sin importar mayúsculas, con o sin `` ` `` o `**` alrededor), cuando se lee su veredicto, entonces MUST ser `clean`, `warnings` o `blocked` → `review::tests::a_verdict_is_read_from_the_last_line`
- H3. Dado un informe que no termina así, su veredicto MUST ser `none` → `review::tests::a_report_without_a_verdict_line_reads_as_none`

La lectura del stream:

- H4. Dadas las líneas de `stream-json`, el `init` MUST dar el `session_id` → `review::tests::the_init_line_gives_the_session`
- H5. El `result` MUST dar el texto, `is_error` y el `usage` → `review::tests::the_result_line_gives_the_report_and_the_usage`
- H6. Un `usage` ausente, o un campo suyo que falta, MUST leerse como cero → `review::tests::a_missing_usage_reads_as_zero`
- H7. Una línea que no es JSON, o de otro tipo, MUST saltarse → `review::tests::other_lines_are_skipped`

La carpeta de Cursor del revisor:

- H8. `mcp.json` MUST tener un solo servidor, el del perfil, que corre `<exe> mcp --profile P --agent A`. Su `env` MUST llevar `HOME` y `USERPROFILE` en el home real, y las variables de prueba que da `clawg_core::paths::test_env` (`CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER` cuando están puestas, sólo en un build con la feature `testing`; en uno sin ella, ninguna) → `review::tests::the_cursor_mcp_file_pins_the_bridge`
- H9. `cli-config.json` MUST permitir sólo:
  - las herramientas de lectura del puente (`search_memory`, `recall_context`, `read_memory`, `list_memories`, `search_project`, `memory_status`), como `Mcp(<servidor>:<herramienta>)`;
  - un `Shell(<cmd>)` por cada comando permitido.

  Y MUST negar `Write(**)`. → `review::tests::the_cursor_config_allows_only_reading`
- H10. `hooks.json` MUST tener `"version": 1` y ningún hook → `review::tests::the_cursor_hooks_file_has_no_hook`
- H11. La carpeta MUST ser `<data>/review/cursor` y crearse con `ensure_private_dir` (`crates/clawg-core/src/private_dir.rs:21`), que en Unix la deja `0700` → `binary::a_review_runs_the_cursor_cli_and_records_its_session`, que en Unix también mira el modo

Los argumentos y la búsqueda del CLI:

- H12. Los argumentos del CLI MUST ser `-p --trust --approve-mcps --output-format stream-json --workspace <carpeta>`, más `--model M` y `--resume S` cuando se dan, y nunca `--force` → `review::tests::the_cursor_arguments_follow_the_options`
- H13. Sin `--cursor`, el CLI MUST buscarse en `%LOCALAPPDATA%\cursor-agent\agent.cmd` en Windows y como `agent` en el `PATH` en los demás sistemas → `review::tests::the_cursor_cli_is_found_where_its_installer_puts_it`
- H14. Si el CLI no está, el error MUST nombrar `--cursor` → `review::tests::a_missing_cursor_cli_names_the_option`
- H15. `clawg review` MUST tomar `--instructions FILE`, obligatorio → `cli::tests::review_requires_its_instructions`
- H16. MUST tomar `--model`, `--resume`, `--agent` (por defecto `cursor-review`), `--cursor PATH` y `--profile` → `cli::tests::review_takes_its_model_resume_agent_and_cursor`
- H17. MUST tomar `--allow CMD`, repetible, por defecto `git diff`, `git log` y `git show` → `cli::tests::review_allows_git_diff_log_and_show_by_default`

El modelo que se guarda:

- H18. Se guarda el `--model` pasado, en minúsculas, y `auto`, el valor por defecto de Cursor, cuando no se pasó ninguno → `review::tests::the_model_kept_is_the_one_passed_or_auto`

El registro de la sesión:

- H19. Dada una fila abierta, cuando su fin llega con uso, la fila MUST contar `tokens` por `input`, `output`, `cache_read`, `cache_write` y `reasoning`, sólo los mayores que cero → `metrics::sessions::tests::an_end_with_usage_counts_its_tokens`
- H20. Ese fin MUST contar `model` con el modelo y `verdict` con el veredicto, y cerrar la fila con su motivo → `metrics::sessions::tests::an_end_with_usage_counts_its_model_and_verdict`
- H21. `POST /v1/sessions/end` con `usage`, `model` y `verdict` MUST contarlos (H19 y H20) → `api::a_session_end_counts_the_usage_model_and_verdict`
- H22. El daemon MUST pasar `model` a minúsculas: `GPT-5.3-Codex` se guarda `gpt-5.3-codex`. Uno que después no es de 1 a 64 caracteres de `[a-z0-9._-]` MUST guardarse como `other` → `api::a_session_end_keeps_a_model_only_as_an_id`
- H23. Un `verdict` fuera de `clean`, `warnings`, `blocked` y `none` MUST guardarse como `other` → `api::a_session_end_keeps_a_verdict_of_its_set`
- H24. Un fin sin esos campos MUST cerrar la fila como hoy: lo prueba `a_session_end_closes_its_row_with_the_reason`, sin cambios.
- H25. Ni un `model` ni un `verdict` con texto o una ruta MUST llegar a la fila → `no_text_path_or_user_reaches_a_session_row`, con esos dos campos agregados.

Lo guardado y el reporte:

- H26. Una fila con contadores de `tokens`, `model` y `verdict` MUST volver igual de `metrics.db` → `metrics::store::tests::review_counts_are_written_and_read_back`
- H27. Un contador de un tipo que este clawg no conoce, sembrado en un archivo del esquema 3, MUST saltarse al leer, sin error y con las demás filas y contadores intactos. Es lo que hace un clawg anterior con los tipos nuevos → `metrics::store::tests::a_count_of_a_kind_this_clawg_does_not_know_is_skipped`
- H28. Cada fila y cada grupo del reporte MUST traer `tokens`, `models` y `verdicts` sumados, vacíos en una fila que no tiene ninguno → `metrics::session_report::tests::a_report_sums_the_tokens_models_and_verdicts`

De punta a punta, con un CLI falso (un `.cmd` en Windows, un script `sh` en los demás sistemas) que guarda su stdin y escribe un stream:

- H29. El CLI falso MUST recibir el prompt de H1 por stdin → `binary::a_review_runs_the_cursor_cli_and_records_its_session`
- H30. Con un `result` cuyo informe termina en `WARNINGS`, `clawg review` MUST imprimir el informe y una línea con la sesión, el modelo, el veredicto y los tokens, y salir con 0 → la misma
- H31. `clawg metrics sessions --session <id>` MUST dar una fila de agente `cursor-review`, cliente `cursor`, con el proyecto de la carpeta, `starts` `startup`, `end_reason` `completed`, los tokens, el modelo y el veredicto `warnings` → la misma
- H32. Con un `result` que trae `is_error: true`, `clawg review` MUST salir con 1, y la fila MUST cerrarse con `error` → `binary::a_review_whose_cursor_fails_ends_its_session_as_error`
- H33. Con un CLI que sale distinto de 0 antes de su `init` (sin sesión iniciada, por ejemplo), MUST salir con 1 mostrando lo que el CLI escribió en stderr, y MUST NOT dejar ninguna fila → `binary::a_review_whose_cursor_fails_before_starting_leaves_no_row`
- H34. Con `--resume S`, la fila MUST abrirse con `starts` `resume` → `binary::a_resumed_review_starts_as_resume`
- H35. Con un stdin vacío, `clawg review` MUST salir con 2 sin lanzar el CLI → `binary::a_review_without_a_message_is_refused`

Con el CLI real:

- H36. Un `clawg review` del build de test, contra un home temporal (`CLAWG_TEST_HOME`, `CLAWG_TEST_EMBEDDER=fake`, perfil `dev`), en una carpeta de prueba con un repo git y un mensaje que pide llamar a `memory_status` y a `search_memory`, MUST dejar esas dos llamadas contadas en la fila de la revisión, con el proyecto de la carpeta. No toca `~/.clawg` ni `~/.clawg-dev`. → comprobación local, en `Verificación`

La skill:

- H37. El párrafo **Review** de `.claude/skills/change-workflow/SKILL.md` MUST decir que:
  - un cambio mediano lo revisa `clawg review` y uno grande `change-reviewer` y `clawg review` en paralelo;
  - el orquestador decide sobre cada informe;
  - la revisión corre en segundo plano;
  - una segunda ronda usa `--resume`;
  - sin el CLI de Cursor (en la nube, por ejemplo) se usa `change-reviewer` en su lugar.

  → `rg -n "clawg review|segundo plano|--resume|sin el CLI de Cursor|el orquestador decide" .claude/skills/change-workflow/SKILL.md` muestra una línea de esa sección por cada parte (la skill está en inglés: las frases son `clawg review`, `in the background`, `--resume`, `without the Cursor CLI` y `the orchestrator decides`).

Lo que no cambia:

- No cambia nada fuera de:
  - `crates/clawg-cli/src/{review.rs,cli.rs,main.rs,lib.rs,hook.rs,daemon/api/sessions.rs,client.rs}`;
  - `crates/clawg-cli/tests/{api.rs,binary.rs}`;
  - `crates/clawg-core/src/metrics/{sessions.rs,store.rs,session_report.rs}` y `crates/clawg-core/src/paths.rs` (`test_env`);
  - los documentos del paso 4.

  De `hook.rs` sólo se hace visible `project_of`. `Cargo.toml` no cambia: el CLI se lanza con `std::process` en un hilo de bloqueo, porque el `tokio` de clawg-cli no tiene la feature `process` (`crates/clawg-cli/Cargo.toml:44`).
- No se afloja ni se borra ningún test existente. `the_agent_contract_matches_its_file` y su par en español pasan sin regenerar los archivos.

## Decisión

**Dónde lee Cursor su configuración: una carpeta propia de clawg (elegida).** `clawg review` escribe `<data>/review/cursor/.cursor/{mcp.json,cli-config.json,hooks.json}` en cada corrida y lanza el CLI con `USERPROFILE` y `HOME` en `<data>/review/cursor`. Así:

- el puente del revisor queda fijado al agente `cursor-review` y al perfil de quien lo lanza;
- los permisos se escriben ahí;
- los hooks del usuario no corren, ni duplican la sesión ni fallan bajo bash.

El puente arranca en la carpeta del workspace, que es el repo revisado. Como el CLI no declara `roots`, esa carpeta es la que el puente lee, y es la misma de la que `clawg review` registra el proyecto (Problema). Su home sale de `dirs::home_dir`, que en Windows no mira `USERPROFILE` (`crates/clawg-core/src/paths.rs:162-166`), y `mcp.json` además le devuelve el home real por `env`, para Linux y macOS. Los chats de `--resume` quedan en esa carpeta, la misma en cada corrida del perfil.

Alternativas descartadas:

- **El `~/.cursor` del usuario, más un bloque `env` en `mcp.json` (por ejemplo `CLAWG_AGENT`) y los permisos en su `cli-config.json`.** Obliga a cambiar `setup cursor` y a migrar las instalaciones que ya existen: el servidor instalado dejaría de coincidir con el plan (`matches_server`, `crates/clawg-cli/src/setup/cursor.rs:256`). Además, los hooks del usuario correrían en cada revisión.
- **`<repo>/.cursor/cli.json`.** Toca el repositorio revisado y también vale para el editor.

El login en otra carpeta sólo se comprobó en Windows. En Linux y macOS, `CURSOR_API_KEY` en el entorno de quien lanza llega al CLI, que lo hereda. Queda como pendiente en `Verificación`.

**Sólo lectura: modo normal con permisos (elegido), no `--mode=ask` ni `--mode=plan`.** Esos dos modos no dejan correr `git diff`, que es lo que el revisor necesita. Los permisos niegan escribir, permiten sólo los comandos de `--allow` por subcomando y rechazan las cadenas con `&&`. Sin `--force`, lo que no está permitido se rechaza. Del puente sólo se permiten las herramientas de lectura: el revisor no guarda, no borra, no cambia memorias y no resuelve conflictos. Lo que sabe vuelve en su informe, y el orquestador decide qué guardar.

**Dónde se guardan tokens, modelo y veredicto: tres tipos nuevos de contador en `session_count` (elegido).** El esquema de `metrics.db` no cambia (`user_version` sigue en 3), y un clawg anterior salta esos tipos al leer (`store.rs:404-407`; H27). Una fila nueva o una columna pedirían el esquema 4, con su migración y su vuelta atrás.

El modelo es el único valor de esas tablas que no sale de un conjunto cerrado del código: es el id que `clawg review` le pasó al CLI, en minúsculas, o `auto`. Se acota a `[a-z0-9._-]`, de 1 a 64 caracteres, y el invariante de `specs/metrics` lo nombra como un cuarto identificador.

No se guarda el nombre que muestra el `init` («Gemini 3.8 Flash Low»): es un texto para mostrar, cambia con la versión del CLI, y con `auto` no dice qué modelo eligió Cursor detrás.

**El id de la fila es el `session_id` de Cursor.** `--resume` reusa ese id, y una ronda que vuelve sobre una fila cerrada abre otra del mismo id (`specs/metrics`, SU3). Como `usage` es de cada corrida, sumar las filas no cuenta dos veces. La sesión se registra al llegar el `init`, que el CLI escribe antes de la primera llamada a una herramienta, así las llamadas del puente del revisor encuentran su sesión.

**El veredicto sale de la última línea del informe,** en el formato que pide `change-reviewer.md:32` (`CLEAN`, `WARNINGS` o `BLOCKED`). Con otras instrucciones queda `none`, que también se guarda.

**Qué le pasa el orquestador.** Las instrucciones son `.claude/agents/change-reviewer.md`. El mensaje lleva la ruta del plan, el rango de commits y el resultado de la suite y de clippy que el orquestador ya corrió. El revisor de Cursor no corre tests: con los comandos por defecto sólo lee el diff y el código. Si hace falta, el orquestador agrega otro comando con `--allow`.

## Datos y vuelta atrás

`memory.db` no cambia. `metrics.db` sigue en el esquema 3 y gana filas de `session_count` con tres valores nuevos de `kind` (`tokens`, `model`, `verdict`).

Un clawg anterior abre el archivo, lo lee sin esos contadores y no lo rechaza (H27). Si retoma al arrancar una fila abierta que los tenía, al reescribirla los pierde (`store.rs:329`). Las filas de una revisión se cierran cuando la corrida termina, así que eso pasa sólo con una revisión que quedó a medias.

Volver atrás es instalar la versión anterior: sus reportes no muestran esos contadores y lo demás sigue igual. `<data>/review/cursor` se puede borrar sin perder nada que no sean los chats para `--resume`.

## Límites conocidos

- `git diff`, `git log` y `git show` aceptan `--output=<archivo>`, que trunca y sobrescribe el archivo que nombra. `Write(**)` no lo cubre, porque es un efecto del comando. La revisión del cambio llevó a negarlo con `Shell(* --output*)` (abajo, en `Verificación`). Un comando de `--allow` que escriba por otro camino no se niega, y por eso la skill sigue pidiendo `git status` después de cada revisión.
- La sesión se registra al llegar el `init`, mientras el CLI sigue corriendo: una llamada del puente anterior a ese registro no se contaría.
- Si el daemon no da un proyecto para la carpeta, la fila queda sin proyecto y no cuenta las llamadas del puente.
- Dos revisiones a la vez en el mismo perfil escriben los mismos archivos de la carpeta del revisor.
- Un `.cursor/mcp.json` o `.cursor/hooks.json` del repo revisado también se carga, y un servidor suyo con el nombre del del perfil (`clawg` o `clawg-dev`) podría tomar el lugar del puente fijado.
- Si la herramienta del orquestador corta `clawg review` por tiempo:
  - la fila queda abierta y cierra a las 6 horas con `idle`, sin tokens;
  - los procesos del CLI siguen vivos.

  Por eso la skill dice que corre en segundo plano.

## Pasos

1. `an_end_with_usage_counts_its_tokens`, `an_end_with_usage_counts_its_model_and_verdict`, `review_counts_are_written_and_read_back`, `a_count_of_a_kind_this_clawg_does_not_know_is_skipped` y `a_report_sums_the_tokens_models_and_verdicts` fallan → `CountKind::{Tokens, Model, Verdict}`, los conjuntos `TOKEN_KINDS` y `VERDICTS`, `SessionLog::end_with`, y `tokens`, `models` y `verdicts` en `RowView` y `Group`. Va en un solo paso porque el `match` de `RowView::of` sobre `CountKind` es exhaustivo (`crates/clawg-core/src/metrics/session_report.rs:84-89`). Commit `feat(metrics): count and report a session's tokens, model and verdict`.
2. `a_session_end_counts_the_usage_model_and_verdict`, `a_session_end_keeps_a_model_only_as_an_id`, `a_session_end_keeps_a_verdict_of_its_set` y los campos nuevos de `no_text_path_or_user_reaches_a_session_row` fallan → `SessionEndRequest` con `usage`, `model` y `verdict`, `model_label`, y `DaemonClient::end_session_with`. Commit `feat(daemon): take a session's usage, model and verdict at its end`.
3. Los tests de `review.rs` (H1 a H10, H12 a H14, H18), los de `cli.rs` (H15 a H17) y los de `binary.rs` (H11, H29 a H35) fallan. El cambio:
   - `review.rs`, con las funciones puras y `review::run`, que lee stdin, asegura el daemon, registra el proyecto, escribe la carpeta, lanza el CLI, registra la sesión con el `init`, la cierra con el `result` e imprime;
   - `Command::Review` y su brazo en `main.rs`, en el mismo commit, porque el `match` de `crates/clawg-cli/src/main.rs:121` es exhaustivo.

   Commit `feat(review): run a cursor review and record its session`.
4. Documentos:
   - `openspec/specs/review/spec.md`, nuevo;
   - `openspec/specs/metrics/spec.md`: SU1, SU2, SU6, SU11 y los invariantes;
   - `openspec/specs/daemon-access/spec.md`: SU6;
   - `docs/reference/commands.md` y `docs/reference/daemon.md`: `POST /v1/sessions/end`;
   - `docs/reference/data-profiles.md`: los tipos de `session_count`, el cuarto identificador y `<data>/review/cursor`;
   - `.claude/skills/change-workflow/SKILL.md`, Review (H37);
   - `openspec/ROADMAP.md`.

   Commit `docs: describe clawg review and how a change is reviewed with it`.

La suite completa y los dos clippy corren una vez, en el último commit.

## Parte 2: el modelo del revisor como preferencia

Se agregó después de la revisión del cambio, antes de archivar, a pedido de la persona: «me gustaria poder setearlo el modelo de cursor de forma global o por proyecto en las preferencias. Que por defecto utilice ese mismo que usaste para esta prueba [...] el comando para setear el modelo sea capaz de extraer la lista de modelos disponibles para poder elegir entre esa lista».

### Problema

Hoy nada fija el modelo:

- `clawg review` lo toma de `--model`, sin valor por defecto (`ReviewOpts::model`, `crates/clawg-cli/src/cli.rs:341-344`);
- sin `--model` no se le pasa ninguno al CLI, que usa su `auto` (`kept_model`, `crates/clawg-cli/src/review.rs:136-139`; `AUTO_MODEL`, `:42`);
- la skill sólo dice «one of the Cursor plan's models» (`.claude/skills/change-workflow/SKILL.md:85`).

Cada sesión elige distinto, o cae en `auto`, y `auto` no dice qué modelo revisó. La revisión de este cambio usó `gpt-5.3-codex-high` porque lo eligió la sesión en el momento.

`clawg preferences` maneja hoy una sola preferencia, el idioma. Vive en `<data>/preferences.json` con `deny_unknown_fields` (`crates/clawg-core/src/preferences.rs:76-85`), y `set language` toma el valor con un `value_parser` de clap (`SetPreference` y `parse_lang`, `crates/clawg-cli/src/cli.rs:885-897`). La spec dice que `clawg preferences` no necesita el daemon ni abre el store (`openspec/specs/preferences/spec.md:48`).

`agent --list-models` imprime `Available models`, una línea en blanco y una línea por modelo, `<id> - <nombre>` (por ejemplo `gpt-5.3-codex-high - Codex 5.3 High`). Se comprobó con `agent 2026.11.22-e9042b2`.

### Hecho

- H38. Sin `--model`, `clawg review` MUST usar el modelo de la preferencia del proyecto de la carpeta, si no la del perfil, y si no `gpt-5.3-codex-high`. Con `--model`, ése → `review::prefs::tests::the_model_is_the_flag_then_the_project_then_the_profile_then_the_default`
- H38b. Con un `review.json` que no se puede leer, `clawg review` MUST salir con 1 nombrando el archivo, sin lanzar el CLI: no corre con un modelo que nadie eligió → `binary::a_review_with_a_broken_review_file_is_refused`
- H39. La sesión MUST guardarse con el modelo que se usó, que ahora siempre se le pasa al CLI: nunca `auto` sin `--model`. Reemplaza a H18 → `binary::a_review_uses_the_preferred_model`
- H40. `<data>/review.json` MUST guardar `{"version": 1, "model": …, "projects": {"<project_id>": …}}`, con `deny_unknown_fields`, escrito con un archivo temporal y un rename, como `preferences.json`. Un archivo ausente MUST leerse como sin preferencia. Uno roto, o de otra `version`, MUST rechazarse y MUST NOT sobrescribirse → `review::prefs::tests::the_review_preferences_round_trip`, `review::prefs::tests::a_broken_or_newer_file_is_refused`, `binary::a_review_model_set_on_a_broken_file_changes_nothing`
- H40b. Un `MODEL` MUST guardarse en minúsculas y sólo si es de 1 a 64 caracteres de `[a-z0-9._-]`, la regla de H22 → `review::prefs::tests::a_model_is_kept_only_as_an_id`
- H41. `clawg preferences set review-model MODEL` MUST guardar `MODEL` para el perfil, y con `--project` para el proyecto de la carpeta, que da el daemon (lo arranca si no corre). `unset review-model [--project]` MUST quitarlo. Sólo `--project` necesita el daemon; el resto de `clawg preferences` sigue sin él → `binary::the_review_model_is_set_and_unset_for_the_profile_and_the_project`
- H41b. `set review-model` MUST tomar `--cursor PATH`, como `clawg review`, y sin él buscar el CLI como `clawg review` (H13). Los tests del binario lo apuntan a un CLI falso, o a una ruta que no existe, y nunca al CLI instalado → `cli::tests::preferences_parses_set_and_unset`
- H42. Al fijarlo, cuando el CLI de Cursor responde a `--list-models`, un `MODEL` que no está en su lista MUST rechazarse con 1 sin cambiar nada, nombrando cómo ver la lista. Si el CLI no está o falla, MUST guardarse igual, avisando que no se pudo comprobar → `binary::a_review_model_outside_the_list_is_refused`, `binary::a_review_model_is_kept_when_the_list_cannot_be_read`
- H43. `clawg preferences set review-model` sin `MODEL`:
  - MUST pedirle la lista al CLI e imprimirla numerada, marcando el modelo vigente;
  - con una terminal, MUST leer un número o un id y guardarlo;
  - sin una terminal, o sin lista, MUST salir con 2 sin cambiar nada, diciendo que se pase `MODEL`.

  → `review::prefs::tests::the_model_list_is_read_from_the_cursor_output`, `review::prefs::tests::a_choice_is_a_number_or_an_id_of_the_list`, `binary::choosing_a_review_model_without_a_terminal_lists_them_and_exits_2`
- H44. `clawg preferences` sin acción MUST mostrar además el modelo del revisor del perfil (o el valor por defecto) y los de los proyectos → `preferences::tests::the_preferences_show_the_review_model`
- H44b. Cada frase nueva (lo que se muestra, lo que confirma un cambio, la cabecera de la lista, el rechazo, el aviso de que la lista no se pudo leer, la pregunta y la salida con 2) MUST estar en inglés y en español, un `match` sobre `Lang` por frase, en el idioma de la preferencia → `review::prefs::tests::every_sentence_is_in_both_languages`
- H45. `clawg preferences set language es` y `unset language` MUST seguir igual, y `set language fr` MUST seguir siendo un error de uso → `cli::tests::preferences_parses_set_and_unset`, adaptado sólo en la forma en que se arma el valor esperado
- H46. El párrafo **Review** de la skill MUST dejar de pedir `--model` y decir que el modelo sale de la preferencia (`clawg preferences set review-model`) → `rg -n "review-model" .claude/skills/change-workflow/SKILL.md` muestra la línea, y `rg -n "^- .--model" .claude/skills/change-workflow/SKILL.md` no muestra ninguna.

Tests existentes que cambian, y cómo:

- `review::tests::the_model_kept_is_the_one_passed_or_auto` (`review.rs:584-589`) sale con `AUTO_MODEL` y `kept_model`. Lo reemplaza `the_model_is_the_flag_then_the_project_then_the_profile_then_the_default`, que comprueba lo mismo para `--model` (en minúsculas) y el valor por defecto nuevo en lugar de `auto`.
- En `binary::a_review_runs_the_cursor_cli_and_records_its_session`, `model auto` y `"models": {"auto": 1}` pasan a `gpt-5.3-codex-high`. El test sigue comprobando lo mismo, con el modelo por defecto nuevo.
- `binary::preferences_set_and_unset_write_the_profiles_file` compara la salida entera de `clawg preferences`: sus dos comparaciones suman la línea del modelo del revisor (H44), en inglés y en español.
- Los ayudantes `set` y `unset` de `crates/clawg-cli/src/preferences.rs:70-76` y el valor esperado de `cli::tests::preferences_parses_set_and_unset` se arman con la forma nueva de `SetPreference` y `UnsetPreference`. Ninguna aserción se saca.

No cambia nada fuera de:

- `crates/clawg-cli/src/{review.rs,review/prefs.rs,cli.rs,main.rs,preferences.rs}`;
- `crates/clawg-cli/tests/binary.rs`;
- los documentos del paso 9.

`preferences.json`, su formato y su lectura (`clawg_core::preferences`), el store y el contrato MCP no cambian. No se afloja ningún test: los de arriba cambian el valor esperado por el nuevo comportamiento, y ninguno pierde una aserción.

### Decisión

**Dónde se guarda: `<data>/review.json` (elegido), no `preferences.json`.** `preferences.json` rechaza los campos que no conoce (`deny_unknown_fields`). Un clawg anterior leería entonces un `preferences.json` con el modelo como roto, y quien vuelve atrás perdería también la preferencia de idioma. Con un archivo aparte, un clawg anterior no lo ve y el idioma sigue. El comando sigue siendo `clawg preferences`.

**Por proyecto, con el id que da el daemon.** `clawg review` busca la preferencia del proyecto por el mismo id con que registra su sesión (`project_of`). Un id derivado de la carpeta sin el daemon podría no coincidir para un clon reconocido por su remoto o una subcarpeta declarada (`specs/metrics`, «Límites conocidos»).

**Elegir de la lista sin una dependencia nueva.** La lista se lee de `agent --list-models` y se elige escribiendo un número o un id en una línea. Un selector con flechas pediría una biblioteca de terminal que el workspace no tiene. Sin terminal, por ejemplo desde un agente, se imprime la lista y se pide `MODEL`, así un agente puede elegir en dos pasos.

**`--project` es la única parte de `clawg preferences` que usa el daemon.** El id del proyecto lo da el daemon, como en `clawg review`, y lo arranca si no corre. La spec de preferencias (`specs/preferences`, LP3) dice hoy que el comando no lo necesita: el paso 9 la cambia para decir que vale salvo para `--project`.

**Un `review.json` roto detiene la revisión.** Seguir con el valor por defecto correría un modelo que nadie eligió, sin avisar, que es lo que el `deny_unknown_fields` de `preferences.json` existe para evitar (`crates/clawg-core/src/preferences.rs:76-78`).

**`set language` pasa a ser un subcomando de `set`.** Hoy `set` toma `KEY VALUE` con el valor tipado como idioma. Con dos claves de valores distintos, cada una tiene su subcomando: `set language es|en` y `set review-model [MODEL] [--project]`. Lo que se escribe en la línea de comandos para el idioma no cambia.

### Límites conocidos de la parte 2

- Las preferencias son del perfil: `dev` y `prod` tienen cada uno la suya.
- La clave de un proyecto es su id. Después de `clawg projects merge` o `delete`, la preferencia del id que se fue ya no aplica, y nada lo dice.
- Si Cursor deja de ofrecer `gpt-5.3-codex-high`, una revisión sin preferencia falla como un CLI que no arranca (H33). `clawg preferences set review-model` muestra la lista para elegir otro.
- Una clave nueva en `review.json` pide subir su `version`, y un clawg anterior rechazaría entonces el archivo. Sólo se pierde el modelo elegido, nunca el idioma.

### Pasos

7. Los tests de `review::prefs` (H38, H40, H40b, H43, H44b) fallan → `crates/clawg-cli/src/review/prefs.rs`: el archivo, la regla del modelo, la lectura de la lista, la elección, la resolución del modelo y sus frases. Commit `feat(review): keep the reviewer's model per profile and per project`.
8. Los tests de `cli.rs` (H41b, H45), `preferences.rs` (H44) y `binary.rs` (H38b, H39, H40, H41 a H43) fallan → `set` y `unset` con un subcomando por clave, `review-model` en `clawg preferences`, la lista desde el CLI, y `clawg review` usando la preferencia. Commit `feat(preferences): set the review model, choosing from the Cursor CLI's list`.
9. Documentos: `docs/reference/commands.md` y `docs/reference/data-profiles.md` (`review.json`), `openspec/specs/review/spec.md` (H18 pasa a H38/H39), `openspec/specs/preferences/spec.md` (LP3, salvo `--project`), la skill (H46). Commit `docs: describe the review model preference`.

## Registro

- La revisión del plan no pasa a Cursor: la persona decidió el reparto para la revisión de un cambio, y la lectura más estrecha deja el plan como está.
- Guardar el modelo y el veredicto sale de «etc igual que hacemos con las sesiones normales»: a una sesión normal se le cuentan sus llamadas y sus saves. El modelo es la única forma de comparar revisores.
- «Memoria compartida» se lee como lectura: el revisor sólo usa las herramientas de lectura del puente, y lo que sabe vuelve en su informe para que el orquestador decida qué guardar. Un revisor que corre solo no debería borrar, cambiar ni resolver memorias.
- Sin el CLI de Cursor (una sesión en la nube, por ejemplo) la revisión vuelve a `change-reviewer`: la persona eligió Cursor para lo que tiene a mano, no dejar un cambio sin revisar.
- Un stdin que es una terminal también se rechaza con 2, para que `clawg review` no se quede esperando. No tiene test: la suite no tiene una terminal.
- H8, H9, H20, H30 y H31 dicen más de una cosa cada uno, pero cada uno tiene un solo test que comprueba todas sus partes; se dejan así.
- El permiso por herramienta, `Mcp(<servidor>:<herramienta>)`, sólo lo prueba H36. Si H36 muestra que no funciona, el cambio se detiene ahí, sin volver a `Mcp(<servidor>:*)`, que dejaría al revisor escribir memoria.
- La parte 2 fue aprobada por las dos revisiones de plan en su primera ronda, con `WARNINGS`, que se cerraron en este plan antes de implementarla. «Global» se lee como del perfil: cada perfil tiene su `review.json`.
- Plan aprobado en la segunda ronda por las dos revisiones de plan, con `WARNINGS`. Sus avisos se cerraron en este plan antes de implementar.
- Pasos 1 a 4 en commits `3271aa0a`, `b7fb0789`, `e7991d5a` y `8daa39dc`.
- Desvío en el orden TDD:
  - En el paso 1, los tests se corrieron y fallaron antes del código, salvo `a_count_of_a_kind_this_clawg_does_not_know_is_skipped`, que fija una conducta que ya existía (`store.rs:404-407`) y no podía fallar.
  - En los pasos 2 y 3, los tests y el código se escribieron juntos y se corrieron después: se vieron pasar, no fallar antes. Cada uno comprueba algo que antes no existía (un campo, una ruta, un comando), así que no podía pasar sin el cambio.
- Parte 2, pasos 7 a 9, en `8e4e4bfb`, `cace2e34` y `d948f4d6`:
  - Los tests y el código de los pasos 7 y 8 se escribieron juntos: se vieron pasar, no fallar antes, como en los pasos 2 y 3.
  - Un test existente que el plan no nombraba también cambió, `preferences_set_and_unset_write_the_profiles_file`, ya agregado a la lista de arriba.
  - Con el CLI real, `set review-model` sin modelo listó los modelos de la cuenta con `gpt-5.3-codex-high` marcado, y `set review-model gpt-5.6-sol-high` se comprobó contra esa lista y se guardó.
- H36 se corrió y salió como se esperaba (abajo): el permiso por herramienta funciona, así que el plan sigue sin cambios.
- `clawg_core::paths::test_env` se agregó a `paths.rs` porque `TEST_EMBEDDER_VAR` es `pub(crate)` en `memory::upkeep::bootstrap`. Está en la lista de lo que cambia.
- `e574a095` es un commit de arreglos de la revisión de la parte 2, fuera de los pasos 7 a 9:
  - El test de S7 se escribió con el código.
  - Los tres de W4 no podían fallar antes, porque cubren ramas que ya existían.
  - S2 (volver a leer `review.json` antes de guardar) no tiene test: haría falta otro proceso que rompa el archivo entre las dos lecturas, mientras el primero espera la lista o la terminal.
- La ronda 2 de la parte 2 dejó tres commits más, fuera de los pasos:
  - `e210010e`: el test `the_review_model_of_a_project_starts_the_daemon` toma un `StopOnDrop`.
  - `c2baa119`: el sufijo «no se cambió nada» de un `review.json` ilegible sale de `prefs::says`, en los dos idiomas. Su caso en `every_sentence_is_in_both_languages` se vio fallar antes del código.
  - `af466a3f`: los documentos de S4, S7 y `--model`.
- La suite y los dos clippy corrieron en el último commit de código, `af466a3f`: lo que vino después es sólo de documentos y la versión.
- El cambio se archiva el 2026-11-26 y sale en 0.96.0, no en 0.95.0 como decía este plan: esa versión la tomó `suite-memory`. Toma el número 144 del mapa.

## Verificación

### H36, con el CLI real

Corrió en Windows con `agent 2026.11.22-e9042b2`:

1. Una copia de `target-test\debug\clawg.exe` en una carpeta temporal.
2. `clawg serve --profile dev --port 0` con `CLAWG_TEST_HOME` en un home temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`.
3. En un repo git de prueba, el mensaje por stdin a `clawg review --profile dev --instructions <archivo> --model gemini-3.8-flash-low`. El mensaje pedía llamar a `memory_status` y a `search_memory` y terminar con `CLEAN`.
4. `clawg stop --profile dev` y `clawg metrics sessions --profile dev --session <id>`.

Resultado:

- `clawg review` salió con 0 en 20 s, e imprimió el informe y la línea `clawg review: session 270ad2cb-… · model gemini-3.8-flash-low · verdict CLEAN · 20 s · tokens 17811 in, 221 out, 31099 cache read · next round: --resume 270ad2cb-…`.
- La fila:
  - agente `cursor-review`, cliente `cursor`, proyecto `repo-7142`, el de la carpeta;
  - `starts` `startup` y `end_reason` `completed`;
  - `calls` con `memory_status` 1 y `search_memory` 1, y `empty_searches` 1;
  - `tokens` `input` 17811, `output` 221 y `cache_read` 31099;
  - `models` `gemini-3.8-flash-low` 1 y `verdicts` `clean` 1.

No se tocó `~/.clawg` ni `~/.clawg-dev`.

### Revisión del cambio, ronda 1

Corrieron en paralelo `change-reviewer` (`WARNINGS`) y `clawg review` con `gpt-5.3-codex-high` (`BLOCKED`, 352 s, 176.570 tokens de entrada). `clawg review` corrió con el build de test contra un home temporal, así que el revisor de Cursor no vio la memoria real.

- **`--output` (CRITICAL de Cursor).** `git diff|log|show --output=<archivo>` sobrescribe archivos. Se niega con `Shell(* --output*)`. En una corrida del CLI real, `git log -1 --output=…` y `git show HEAD --output=…` dieron `permissionDenied`, y `git show --stat` y `git diff --stat` pasaron. Commit `aabc0374`; test `the_cursor_config_allows_only_reading`.
- **`Microsoft/` en la carpeta revisada (W5 de `change-reviewer`).** Lo confirmó la primera revisión, que dejó `Microsoft/Windows/PowerShell/ModuleAnalysisCache` en la raíz del repo. El shell del CLI corre PowerShell sin `LOCALAPPDATA`. `clawg review` le pasa ahora `PSModuleAnalysisCachePath` dentro de la carpeta del revisor. Una corrida del CLI real con el arreglo no dejó nada en el repo de prueba, y la caché quedó en `<data>/review/cursor/powershell`. Commit `aabc0374`; test `the_cursor_runs_with_the_review_folder_as_its_home_and_powershell_cache`. La carpeta que dejó la primera revisión se borró.
- **Carácter de control en `commands.md` (W4, y SUGGESTION de Cursor).** Un `\a` del script de edición quedó como BEL. Corregido en `aabc0374`.
- **W1, el test sólo miraba que el proyecto fuera un texto.** Ahora lo compara con el que da el daemon a un hook en la misma carpeta (`a_review_runs_the_cursor_cli_and_records_its_session`).
- **W3, el orden TDD de los pasos 2 y 3.** Ya estaba en `Registro`; queda así.
- **S1.** Anotado en `Registro`.
- **S3.** El mensaje de error y el texto del test ya no nombran un plan.
- **S4, S5 y S6.** Pasaron a `Límites conocidos`, aquí y en la spec.
- **W2, la suite completa y los dos clippy.** No corrieron: la primera corrida de la suite se cortó por falta de memoria del sistema. Quedan pendientes antes de archivar.
- **S2.** El ROADMAP y la spec ya enlazan la carpeta archivada, que se crea al archivar.

### Revisión del cambio, ronda 2

Revisó `aabc0374`, con el revisor de Cursor retomado por `--resume` (109 s).

- `change-reviewer`: `WARNINGS`. Da por resueltos W1, W4, W5, S1 y S3. Sus dos sugerencias nuevas, S7 (la caché de PowerShell en `data-profiles.md`) y S8 (`--output-format` también se niega), quedaron en `ab1fb83c`.
- `clawg review`: `WARNINGS`. Da por resueltos su CRITICAL y su SUGGESTION.

Lo único abierto en las dos es W2: la suite completa y los dos clippy no corrieron.

### Revisión de la parte 2

Revisó `ab1fb83c..d948f4d6` en paralelo:

- `change-reviewer`: `WARNINGS`.
- `clawg review`: `WARNINGS`, sin `--model`, con lo que corrió con el valor por defecto `gpt-5.3-codex-high` (289 s, 119.120 tokens de entrada).

Las dos cumplen todos los criterios y dejan abierto W2, la suite y clippy. De `change-reviewer`, en `e574a095`:

- **W1, `ModelSource` sin uso fuera de los tests.** `resolve` devuelve sólo el modelo.
- **W4, ramas sin test.** Tres tests nuevos:
  - `choosing_a_review_model_without_a_list_exits_2`;
  - `a_review_model_is_kept_when_the_cursor_cli_fails_to_list`;
  - `the_review_model_of_a_project_starts_the_daemon`.

  Leer una respuesta de una terminal sigue sin test: la suite no tiene una.
- **S1.** El `replace` de saltos de línea literales usa ahora los escapes.
- **S2.** `set` vuelve a leer `review.json` antes de guardar.
- **S3.** Una elección que no está en la lista dice `NotInList`.
- **S4.** Un `review.json` roto hace salir a `clawg preferences` con 1.
- **S6.** `cursor_args` toma siempre un modelo.
- **S7.** Un modelo editado a mano que no es un id hace inválido el archivo.
- **Quedan como están:**
  - **S5.** `--list-models` no tiene un plazo, igual que la revisión.
  - **S8.** Anterior al cambio: el sufijo «nothing was changed» sólo en inglés.

El único hallazgo del revisor de Cursor en esa ronda fue W2.

### Revisión de la parte 2, ronda 2

Revisó `e574a095`. `change-reviewer` y `clawg review` corrieron en paralelo, sin ver uno el informe del otro. `clawg review` corrió con el build de test contra un home temporal, como en H36, con el modelo por defecto `gpt-5.3-codex-high`, en 185 s.

- `change-reviewer`: `WARNINGS`. Da por resueltos W1, W4, S1, S2, S3, S4, S6 y S7. Encontró:
  - **W1, un daemon que puede quedar vivo.** `the_review_model_of_a_project_starts_the_daemon` sólo lo paraba con su `clawg stop` final. Ahora toma un `StopOnDrop`, como los demás tests cuyo daemon no es hijo del test. Commit `e210010e`.
  - **W2, S4 y S7 sin documentar.** La spec de preferencias (RM1), `commands.md` y `data-profiles.md` dicen ahora dos cosas: un modelo que no es un id hace inválido `review.json`, y un archivo inválido también hace fallar `clawg preferences` sin acción. Commit `af466a3f`.
  - **W3, `e574a095` sin nombrar en `Registro`.** Agregado.
  - **W4, `--model` «cuando se da».** La spec de review (RV3) y `commands.md` dicen ahora que se pasa siempre. Commit `af466a3f`.
  - **W5, la versión, la fecha y `state.yaml`.** Se corrigen al archivar: 0.96.0, 2026-11-26.
  - **S8, la razón inexacta.** Las tres frases eran de este cambio, y `sharing.rs` ya traduce ese sufijo. En lugar de cambiar la razón, se tradujo. Commit `c2baa119`.
  - **S2, no copiar el número de tests.** Hecho.
- `clawg review`: `WARNINGS`. Da por resueltos los mismos arreglos. Sus dos sugerencias eran S5, que sigue fuera de alcance, y S8, la misma de arriba.
- El primer intento de `clawg review` salió con «timed out after 60s waiting for the daemon to start», porque con `CLAWG_TEST_DEFAULT_PORT=off` no hay un puerto por defecto donde arrancarlo. Se levantó el daemon de prueba a mano con `--port 0`, como en H36. Es del entorno de prueba: con un perfil real, el daemon arranca en su puerto.

### Revisión de la parte 2, ronda 3

Revisó `c58b61b1..af466a3f`. Sólo la corrió `clawg review`, retomado con `--resume`, en 133 s: los arreglos eran de los hallazgos de las dos revisiones de la ronda 2.

- Veredicto: `WARNINGS`. Da por resuelto S8, y no encontró nada nuevo.
- Sigue mencionando S5, el plazo de `--list-models` y de la corrida, que queda fuera de alcance (lo explica «Fuera de alcance»).

Ninguna revisión dejó archivos en el repo.

### Cierre

W2 queda cerrado:

- La suite completa (`cargo test --workspace --target-dir target-test`, sin `-j`) pasó dos veces sobre `d9d73c89`, y otra vez sobre `af466a3f`, el último commit de código.
- Los dos clippy (`cargo clippy --locked --workspace --all-targets -- -D warnings` y `cargo clippy --locked -p clawg-cli -- -D warnings`) pasaron sobre `c58b61b1` y sobre `af466a3f`.

### Pendiente en local

- El login del CLI con otro home en Linux y macOS.
- La primera revisión con el perfil `prod`, después de publicar 0.96.0, y `clawg preferences set review-model` en ese perfil.
