# repeated-errors — Especificación

Delta de [`specs/hooks`](../../specs/hooks/spec.md) (RE1 a RE6), [`specs/setup`](../../specs/setup/spec.md) (RE7, que cambia R3, R6 y SL2), [`specs/doctor`](../../specs/doctor/spec.md) (RE8) y [`specs/eval`](../../specs/eval/spec.md) (RE9). Origen: [proposal](proposal.md). Los tests de RE1 a RE8 corren con la suite, sin modelo real, red ni daemon salvo donde se dice. Los que corren el binario usan `CLAWG_TEST_HOME`, el modelo falso y `--port 0`, y no dejan nada en el temporal. Cada test nombrado falla antes del cambio, porque la aserción no se cumple o porque el código no compila.

## Definiciones

- **Firma de programa ausente:** una línea del texto de una llamada que termina en `: command not found` (las formas de `bash` y `sh`, con o sin `line N: ` delante del nombre, como `/usr/bin/bash: line 1: eza: command not found`), que tiene la forma `sh: N: X: not found` (`dash`) o la forma `zsh: command not found: X`. `X` es el programa.
- **Nombre de programa válido:** de 1 a 32 caracteres de `[A-Za-z0-9._+-]`, empezando por letra o número, en el que `redact` no encuentra ningún secreto.
- **Palabra de comando:** la primera palabra de cada segmento del comando de `tool_input.command`, partido por `|`, `||`, `&&`, `;`, `&`, salto de línea, `(` y `$(`, saltando asignaciones `NOMBRE=valor` y los prefijos `sudo`, `time`, `env`, `command`, `exec` y `xargs`. El comando no se interpreta más allá de esto. Un error de este corte sólo puede costar una observación, nunca inventar una.
- **Libro:** `<data>/tool-failures.jsonl`, un objeto JSON por línea. A los 64 KiB pasa a `tool-failures.1.jsonl`.
- **Línea del libro:** `{"at_ms": N, "event": "fail"|"ok", "tool": T, "program": P, "session": S}`. `tool` es el `tool_name` de la entrada (`Bash`). `session` es el `session_id` de la entrada, cortado a 64 caracteres.
- **Ventana:** los 30 días anteriores al momento en que se lee el libro.
- **Regla:** un par (`tool`, `program`) que, en la ventana, tiene líneas `fail` de dos o más `session` distintas y ninguna línea `ok`, y cuyo programa no está en el PATH del proceso del hook.

## RE1. Los eventos `post-tool-use` y `post-tool-use-failure`

`clawg hook post-tool-use|post-tool-use-failure --agent A [--profile P]` MUST leer de stdin la entrada de `PostToolUse` o de `PostToolUseFailure` de Claude Code: `tool_name`, `tool_input`, `tool_response` (cualquier valor JSON) o `error` (texto), y `session_id`. Los campos desconocidos y los ausentes se toleran. Con los dos eventos, el hook:

- MUST NOT conectarse a un daemon ni arrancarlo.
- MUST NOT abrir ni crear `memory.db`.
- MUST NOT dejar una línea en `hook-runs.jsonl`.
- MUST salir con 0 y terminar en `POST_TOOL_DEADLINE` (300 ms) como mucho.
- Lee la preferencia de idioma sólo cuando va a imprimir algo.
- Una entrada que no es JSON, que pasa de 1 MiB o que no trae `tool_name` MUST no imprimir nada ni anotar nada.
- Con `--client cursor`, o con una entrada que trae `cursor_version`, MUST no imprimir nada ni anotar nada.
- Sólo `tool_name` = `Bash` se mira. Cualquier otra herramienta MUST no imprimir nada ni anotar nada.

#### Escenario: sin daemon, igual responde
- GIVEN un home de prueba sin daemon y una entrada de `PostToolUseFailure` de `Bash` cuyo `error` es `Exit code 127\n/usr/bin/bash: line 1: eza: command not found` y cuyo comando es `eza -la src`
- WHEN corre `clawg hook post-tool-use-failure --agent claude-code --profile dev`
- THEN sale con 0, imprime un sobre con `hookEventName` `PostToolUseFailure`, y en el directorio de datos no hay `memory.db` ni línea nueva en `hook-runs.jsonl`

**Tests:** `hook_takes_the_post_tool_events` (`cli.rs`); `the_hook_input_reads_the_tool_fields` (`hook.rs`); `a_post_tool_hook_answers_without_a_daemon_and_leaves_the_store_alone` (`crates/clawg-cli/tests/binary.rs`); `cursor_and_other_tools_get_nothing_after_a_tool` (`hook.rs`).

## RE2. La firma de un programa ausente

La decisión de qué observa una llamada MUST ser una función pura de (`tool_name`, comando, textos de la salida, el evento). Los textos de la salida son `error` en `post-tool-use-failure` y todos los textos que contiene `tool_response` en `post-tool-use`, a cualquier profundidad. Esa función:

- MUST dar un fallo por cada programa distinto que aparece en una firma, es una palabra de comando del comando y tiene un nombre válido.
- MUST NOT dar un fallo por un programa que no es una palabra de comando, como la dependencia de un script que el comando lanzó.
- MUST NOT dar nada por un `Exit code 127` sin firma, ni por las formas `is not recognized` de cmd o PowerShell.
- Da un éxito de un programa P sólo en `post-tool-use`, si P es una palabra de comando, la salida no trae una firma de P y el comando no tiene `2>`, `&>` ni `||`.

#### Escenario: un pipe que esconde el código
- GIVEN un `PostToolUse` de `Bash` con el comando `eza -la | head` y un `tool_response` cuyo `stderr` es `/usr/bin/bash: line 1: eza: command not found`
- WHEN se clasifica la llamada
- THEN da un fallo de `eza` y ningún éxito de `head`

#### Escenario: un script que llama a otro programa
- GIVEN el comando `npm test` y una salida con `sh: 1: eslint: not found`
- WHEN se clasifica la llamada
- THEN no da ningún fallo

**Tests:** `a_missing_program_is_read_from_each_shell_form`, `a_failure_needs_the_program_in_command_position`, `a_pipe_with_exit_zero_still_shows_the_failure`, `a_redirected_or_chained_command_is_no_success`, `a_secret_or_an_odd_name_is_never_a_program`, `exit_127_alone_and_other_shells_say_nothing` (`tool_failures.rs`).

## RE3. El aviso en el mismo turno

Cuando la llamada da un fallo de P y el libro no tiene una línea `ok` de P en la ventana, el hook MUST imprimir el sobre del evento con `additionalContext`. El texto es una línea en el idioma preferido (inglés sin preferencia) que:

- nombra P entre comillas invertidas;
- dice que no existe en el Bash de esta máquina;
- pide no volver a llamarlo en la sesión;
- ofrece Glob, Grep, Read o Edit, u otro programa instalado.

Con varios programas, una línea los nombra a todos. Con una línea `ok` de P en la ventana, MUST NOT imprimir nada por P, porque el fallo es probablemente transitorio. Sin ningún fallo, MUST NOT imprimir nada.

#### Escenario: el primer fallo de `eza`
- GIVEN la preferencia `es` y un libro vacío
- WHEN llega un `PostToolUseFailure` con la firma de `eza`
- THEN `additionalContext` es la línea en español que nombra `` `eza` ``

#### Escenario: un `rg` que suele andar
- GIVEN un libro con una línea `ok` de `rg` de ayer
- WHEN llega un `PostToolUse` con la firma de `rg`
- THEN no se imprime nada

**Tests:** `the_same_turn_line_names_the_program_in_the_users_language`, `a_program_that_worked_recently_gets_no_line`, `a_call_without_a_signature_prints_nothing` (`hook.rs`).

## RE4. El libro

- Cada fallo MUST agregar una línea `fail` al libro.
- Un éxito de P MUST agregar una línea `ok` sólo si la línea más nueva de P en el libro es `fail`. Así el libro crece con los fallos y no con las llamadas que andan.
- Cada línea MUST armarse entera y agregarse con una sola escritura en modo append.
- Con el libro en 64 KiB o más, antes de agregar MUST renombrarlo a `tool-failures.1.jsonl`, reemplazando al anterior, sin leer ni reescribir nada.
- MUST NOT crear el directorio de datos.
- Cualquier falla MUST tragarse sin cambiar la salida ni el código de salida.
- Una línea MUST NOT tener el comando, la salida, una ruta, `cwd`, `transcript_path` ni ningún otro valor de la entrada fuera de los de su definición.
- Al leer, una línea dañada MUST costar sólo esa línea.

#### Escenario: lo que queda de un fallo
- GIVEN un `PostToolUseFailure` con el comando `cd /home/u/secreto && eza --token=abc` y la firma de `eza`
- WHEN se anota
- THEN la línea nueva tiene sólo `at_ms`, `event` `fail`, `tool` `Bash`, `program` `eza` y `session`, y el libro no contiene `secreto` ni `abc`

**Tests:** `a_line_keeps_only_the_program_the_tool_and_the_session`, `only_the_first_success_after_a_failure_is_written`, `the_ledger_rolls_at_64_kib_without_rewriting`, `no_data_directory_no_ledger`, `a_damaged_line_costs_only_itself` (`tool_failures.rs`).

## RE5. Cuándo un fallo es una regla

Las reglas MUST salir de una función pura de las líneas de los dos archivos del libro, el momento actual y una función `on_path(program) -> bool`. MUST dar las reglas de la definición, cada una con su cantidad de sesiones y la hora de su último fallo, de la más reciente a la más vieja y como mucho 5. Dos fallos de una misma `session` MUST contar como una sola sesión. En el hook, `on_path` busca el programa, y en Windows también con las extensiones de `PATHEXT`, en las carpetas del `PATH` del proceso, como `doctor::analyze` busca `clawg`.

#### Escenario: lo que dice A2
- GIVEN fallos de `eza` en dos sesiones; fallos de `rg` en tres sesiones, cada uno seguido de un `ok`; fallos de `gh` en dos sesiones con `gh` presente en el PATH; y fallos de `fd` en dos sesiones de hace 31 días
- WHEN se calculan las reglas
- THEN la única regla es `eza`, con 2 sesiones

**Tests:** `two_sessions_make_a_rule_and_one_does_not`, `a_success_in_the_window_cancels_the_rule`, `a_program_on_the_path_is_no_rule`, `failures_older_than_thirty_days_are_forgotten`, `at_most_five_rules_newest_first` (`tool_failures.rs`).

## RE6. Las reglas al iniciar la sesión

Con `--client claude-code`, `session-start` MUST leer el libro y calcular las reglas en un hilo propio, esperado como mucho `LANGUAGE_WAIT`. Con alguna regla:

- MUST imprimir un bloque después de la línea del idioma (R7) y antes del contexto que se presenta como evidencia.
- El bloque abre con `clawg:`, nombra cada programa con sus sesiones y su último fallo, ofrece las herramientas del cliente y dice que, si una instrucción pide esos programas, se le avise a la persona.
- El bloque MUST imprimirse aunque no haya daemon, o aunque el daemon falle o no responda a tiempo, como la línea del idioma.
- El contexto MUST pedirse con `SESSION_BUDGET` menos los bytes del bloque, así lo que el hook agrega sigue debajo de 1500 bytes y el total debajo de 10.000.

Sin reglas, la salida MUST ser byte a byte la de antes. `--client cursor` MUST NOT leer el libro.

#### Escenario: dos sesiones y la tercera
- GIVEN un home de prueba sin daemon y con la preferencia `es`
- WHEN `clawg hook post-tool-use-failure` recibe la firma de `eza` en la sesión `s1` y después en `s2`, y luego corre `clawg hook session-start` para `s3`
- THEN la salida trae la línea del idioma y, debajo, el bloque que nombra `` `eza` `` con 2 sesiones

#### Escenario: una sola sesión no alcanza
- GIVEN dos fallos de `eza`, los dos en la sesión `s1`
- WHEN corre `session-start`
- THEN la salida es sólo la línea del idioma

**Tests:** `session_start_names_the_learned_rules_before_the_evidence`, `the_rules_come_out_of_the_context_budget`, `without_rules_session_start_says_what_it_said`, `what_the_hook_adds_stays_small` (ampliado con 5 reglas de nombre máximo) (`hook.rs`); `two_sessions_teach_the_third` y `one_session_teaches_nothing` (`crates/clawg-cli/tests/binary.rs`).

## RE7. `clawg setup` instala los dos hooks

El plan de `clawg setup claude-code`, con o sin `--local`, MUST sumar dos eventos a los tres de R3: `PostToolUse` y `PostToolUseFailure`, cada uno en un grupo con matcher `Bash` y el hook exec del plan (`hook post-tool-use` y `hook post-tool-use-failure`, con el mismo `--agent` y `--profile`). Las reglas de R3 valen igual para ellos: el hook igual al del plan queda, otro hook de clawg del evento y perfil es un conflicto, y los hooks ajenos se conservan en su orden. Sobre una instalación de los tres eventos de antes, MUST agregar sólo los dos nuevos y dejar el resto del archivo byte a byte. `--remove` MUST sacarlos con los demás. `clawg setup cursor` no cambia.

#### Escenario: quien ya instaló
- GIVEN un `settings.json` con los hooks de `SessionStart`, `UserPromptSubmit` y `SessionEnd` del plan
- WHEN corre `clawg setup claude-code`
- THEN el reporte dice que agrega `PostToolUse` y `PostToolUseFailure` y que los otros tres ya estaban, y lo que había queda igual byte a byte

**Tests:** `a_run_over_an_install_without_the_tool_hooks_adds_only_them`, `the_tool_hooks_match_bash_only`, `an_empty_state_plans_the_server_the_hooks_and_the_block` (ampliado), `remove_takes_the_tool_hooks_too` (`setup.rs`); los de `--local` en `setup/local.rs`, ampliados a cinco eventos.

## RE8. `clawg doctor`

- Con alguna regla (RE5, con el PATH del proceso de `doctor`), `clawg doctor` MUST sumar un hallazgo `tool_rules`, de severidad `info`, `reason_code` `learned`.
  - Su `message`, en el idioma del comando, lista cada programa con sus sesiones y la antigüedad de su último fallo, y dice que borrar `tool-failures.jsonl` y `tool-failures.1.jsonl` olvida todo.
  - Cuando el `CLAUDE.md` del usuario, o el `CLAUDE.md` o `.claude/CLAUDE.md` de la carpeta actual, nombra un programa como palabra entera, el `message` MUST decir qué archivo lo pide y que conviene instalarlo o sacarlo de ahí.
  - `evidence` trae `program`, `tool`, `sessions`, `last_failure_ms` y `named_in` (las rutas).
  - `safe_next_step` es nulo.
- Sin reglas, MUST NOT haber hallazgo.
- Cuando el `settings.json` del usuario, o el `.claude/settings.local.json` de la carpeta actual, tiene el hook de `session-start` de este perfil y le falta el de `PostToolUse` o el de `PostToolUseFailure`, MUST sumar un hallazgo `tool_hooks`, de severidad `info`, `reason_code` `not_installed`, con `clawg setup claude-code` (o con `--local`) como `safe_next_step`.
- Ninguno de los dos hallazgos cambia el código de salida.
- `doctor` MUST NOT escribir el libro.

#### Escenario: el `CLAUDE.md` que pide `eza`
- GIVEN un libro con la regla `eza` y un `CLAUDE.md` del usuario que dice «usar `eza` en lugar de `ls`»
- WHEN corre `clawg doctor`
- THEN hay un hallazgo `tool_rules` cuyo `named_in` es la ruta de ese `CLAUDE.md`, y el código de salida no cambia

**Tests:** `the_learned_rules_are_listed_with_the_files_that_ask_for_them`, `no_rules_no_finding`, `a_session_start_hook_without_the_tool_hooks_is_reported` (`doctor/checks.rs`).

## RE9. La medición en agent-eval

Se construye cuando [agent-eval-sessions](../agent-eval-sessions/proposal.md) (129) esté aplicado. Ese cambio da las tareas de varias sesiones (`sessions`), el shell acotado (`shell` con `missing_commands`), el check `tool_failed_count` y la tarea `shelf-eza-missing`, que es la línea base de este cambio ([129, «Lugar para 131 y 132»](../agent-eval-sessions/proposal.md#lugar-para-131-y-132)). Este cambio no redefine nada de eso y sólo suma un brazo.

Los brazos con hooks toman los de `setup::plan` (`crates/clawg-eval/src/bin/agent-eval/arm.rs:243-249`), así que, desde RE7, `a3` corre con los hooks nuevos. El `a3` que mide 129 antes de este cambio queda como un brazo nuevo:

- El arnés MUST tener un brazo `a3-notools`, igual a `a3` salvo por los grupos de `PostToolUse` y `PostToolUseFailure` de clawg, que no tiene.
- Todo lo demás de `a3-notools` MUST ser igual a `a3`: el servidor, los otros tres hooks, el bloque del protocolo y la siembra.

La corrida en seco no corre los hooks del cliente (129), así que lo que el hook entrega lo prueban RE3 y RE6 contra el binario, no el arnés.

#### Escenario: el brazo de antes
- GIVEN el plan de `a3` y el de `a3-notools` para una misma tarea
- WHEN se comparan sus `settings.json`
- THEN difieren sólo en que `a3-notools` no tiene las claves `PostToolUse` y `PostToolUseFailure`

**Tests:** `a3_notools_is_a3_without_the_tool_hooks`, `a3_notools_reads_and_prints_by_its_name` (`arm.rs`).
