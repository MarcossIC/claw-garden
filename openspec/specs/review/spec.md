# review — Especificación

Fuente de verdad del comportamiento actual de `clawg review`: el CLI de Cursor revisa un cambio con la memoria del perfil, puede leer pero no escribir, y la corrida queda registrada como una sesión (`crates/clawg-cli/src/review.rs`). Quien la lanza, el orquestador, lee el informe y decide qué hacer con él.

**Origen:**
- [cursor-review](../../changes/archive/2026-11-26-cursor-review/plan.md), archivado el 2026-11-26, sale en 0.96.0.

## Definiciones

- **CLI:** el CLI de Cursor (`agent`). `--cursor` lo nombra. Sin `--cursor`, es `%LOCALAPPDATA%\cursor-agent\agent.cmd` en Windows y `agent` en el `PATH` en los demás sistemas.
- **Carpeta del revisor:** `<data>/review/cursor`, el home que el CLI recibe en `USERPROFILE` y `HOME`. Su `.cursor/` se escribe en cada corrida, y ahí quedan los chats del CLI.
- **Stream:** lo que el CLI escribe en stdout con `--output-format stream-json`, una línea JSON por evento. La línea `init` (`type` `system`, `subtype` `init`) trae el `session_id`. La línea `result` trae el informe (`result`), `is_error` y `usage` (`inputTokens`, `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`, `reasoningTokens`), que el CLI no documenta.

## RV1. La entrada

`clawg review --instructions FILE [--model MODEL] [--resume SESSION] [--allow COMMAND]... [--agent NAME] [--cursor PATH] [--profile P]` MUST leer el mensaje de stdin. Con un mensaje vacío, o un stdin que es una terminal, MUST salir con 2 sin lanzar nada.

El prompt MUST ser el texto de `FILE` sin el frontmatter de su principio (`---` … `---`), una línea en blanco y el mensaje, y llegarle al CLI por stdin.

`--agent` es por defecto `cursor-review`, y `--allow` es por defecto `git diff`, `git log` y `git show`. Un `--allow` dado reemplaza ese valor por defecto.

**Tests:** `the_prompt_is_the_instructions_without_frontmatter_and_the_message` en `review.rs`; `review_requires_its_instructions`, `review_takes_its_model_resume_agent_and_cursor`, `review_allows_git_diff_log_and_show_by_default` en `cli.rs`; `a_review_without_a_message_is_refused` en `crates/clawg-cli/tests/binary.rs`.

## RV2. La carpeta del revisor

Antes de lanzar el CLI, `clawg review` MUST crear la carpeta del revisor y su `.cursor/` como directorios privados (`ensure_private_dir`), y escribir:

- `mcp.json`, con sólo el servidor del perfil (`clawg` o `clawg-dev`). Corre este clawg como `mcp --profile P --agent NAME`, con `env` `HOME` y `USERPROFILE` en el home del usuario. Un build con la feature `testing` también le pasa `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER` cuando están puestas (`clawg_core::paths::test_env`).
- `cli-config.json`, que:
  - permite `Mcp(<servidor>:<herramienta>)` sólo para las herramientas que leen (`search_memory`, `recall_context`, `read_memory`, `list_memories`, `search_project`, `memory_status`);
  - permite un `Shell(<comando>)` por cada `--allow`;
  - niega `Write(**)` y `Shell(* --output*)`: con `--output=<archivo>`, en cualquier lugar del comando, `git diff`, `git log` y `git show` truncan y escriben el archivo que nombra.
- `hooks.json`, con `"version": 1` y ningún hook.

**Tests:** `the_cursor_mcp_file_pins_the_bridge`, `the_cursor_config_allows_only_reading`, `the_cursor_hooks_file_has_no_hook` en `review.rs`; `a_review_runs_the_cursor_cli_and_records_its_session` en `binary.rs`.

## RV3. La corrida

El CLI MUST correr con `-p --trust --approve-mcps --output-format stream-json --workspace <carpeta> --model <modelo>`, más `--resume` cuando se da, y nunca con `--force`. Corre en la carpeta donde corre `clawg review`, con `USERPROFILE` y `HOME` en la carpeta del revisor, y con `PSModuleAnalysisCachePath` dentro de ella. El shell donde el CLI corre los comandos no recibe `LOCALAPPDATA`, y sin esa variable PowerShell escribe su caché en `Microsoft/Windows/PowerShell/` de la carpeta revisada.

Antes de lanzarlo, `clawg review` MUST arrancar el daemon del perfil si no corre y registrar el proyecto de esa carpeta. Al llegar la línea `init`, MUST registrar la sesión: su agente, ese proyecto, cliente `cursor` y `source` `startup`, o `resume` con `--resume` ([`specs/metrics`](../metrics/spec.md), SU13). Las llamadas del puente del revisor se cuentan así en esa fila: el CLI no declara `roots`, y el puente lee la carpeta donde arrancó, que es la del workspace.

Al terminar, MUST cerrar la sesión con:

- `completed` si el CLI salió con 0 y su `result` no trae `is_error`, y `error` en otro caso;
- los tokens del `usage`, cero los que faltan;
- el modelo, que se le pasa siempre al CLI: `--model` en minúsculas, si no el que eligió el proyecto de la carpeta, si no el del perfil ([`specs/preferences`](../preferences/spec.md), RM1), y si no `gpt-5.3-codex-high`;
- el veredicto.

El veredicto es la última línea no vacía del informe cuando es `CLEAN`, `WARNINGS` o `BLOCKED` (sin importar mayúsculas, con o sin `` ` `` o `**`). Si no, es `none`.

Un `review.json` que no se puede leer MUST hacer salir con 1, nombrándolo, sin lanzar el CLI: no se corre un modelo que nadie eligió.

Un CLI que termina sin escribir la línea `init` MUST NOT dejar ninguna fila. `clawg review` muestra entonces su stderr y sale con 1.

**Tests:** `the_cursor_arguments_follow_the_options`, `the_cursor_runs_with_the_review_folder_as_its_home_and_powershell_cache`, `the_init_line_gives_the_session`, `the_result_line_gives_the_report_and_the_usage`, `a_missing_usage_reads_as_zero`, `other_lines_are_skipped`, `a_verdict_is_read_from_the_last_line`, `a_report_without_a_verdict_line_reads_as_none`, `the_model_is_the_flag_then_the_project_then_the_profile_then_the_default` (`review/prefs.rs`), `the_cursor_cli_is_found_where_its_installer_puts_it`, `a_missing_cursor_cli_names_the_option` en `review.rs`; `a_review_runs_the_cursor_cli_and_records_its_session`, `a_review_whose_cursor_fails_ends_its_session_as_error`, `a_review_whose_cursor_fails_before_starting_leaves_no_row`, `a_resumed_review_starts_as_resume`, `a_review_uses_the_preferred_model`, `a_review_with_a_broken_review_file_is_refused` en `binary.rs`.

## RV4. La salida

`clawg review` MUST imprimir en stdout el informe, una línea en blanco y la línea `clawg review: session <id> · model <modelo> · verdict <VEREDICTO> · <s> s · tokens <n> in, <n> out, <n> cache read · next round: --resume <id>`, en el idioma de la preferencia del perfil.

MUST salir con 0 cuando la sesión cerró `completed`. Con `error`, MUST salir con 1 y mostrar el stderr del CLI.

**Test:** `a_review_runs_the_cursor_cli_and_records_its_session` en `binary.rs`.

## Límites conocidos

- Un comando de `--allow` que escribe por otro camino que `--output` no se niega: lo que se permite es lo que pasa el orquestador.
- `Shell(* --output*)` también niega un comando permitido que use un flag que empieza con `--output`, como `--output-format`.
- La sesión se registra al llegar la línea `init`, mientras el CLI sigue corriendo: una llamada del puente que llegara al daemon antes de ese registro no se contaría. La primera llamada espera una respuesta del modelo, y no se vio pasar.
- Si el daemon no da un proyecto para la carpeta, la fila queda sin proyecto, y las llamadas del puente, que sí lo tienen, no se cuentan en ella.
- Dos `clawg review` a la vez en el mismo perfil escriben los mismos archivos de la carpeta del revisor: con `--allow` o `--agent` distintos, uno puede correr con los del otro.
- Un `.cursor/mcp.json` o `.cursor/hooks.json` del repo revisado también se carga. Un servidor suyo con el nombre del del perfil podría tomar el lugar del puente fijado.
- Si quien lanza `clawg review` lo corta, la fila queda abierta hasta que cierra con `idle`, sin tokens, y los procesos del CLI siguen vivos.
- El login del CLI con otro home sólo se comprobó en Windows.
- `usage` no está en la documentación del CLI: si deja de venir, los tokens se cuentan como cero.
- Si Cursor deja de ofrecer `gpt-5.3-codex-high`, una revisión sin preferencia falla como un CLI que no arranca.
- La preferencia de un proyecto está bajo su id: después de unir o borrar el proyecto ya no aplica.
