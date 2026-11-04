# hook-run-log — Especificación

`clawg hook session-start` deja un renglón por corrida en `<data>/hook-runs.jsonl`, y `clawg doctor` muestra las últimas. Al archivar, HR1 a HR5 se suman a [`specs/hooks`](../../specs/hooks/spec.md) y HR6 y HR7 a [`specs/doctor`](../../specs/doctor/spec.md). Lo que esas specs dicen de la salida del hook no cambia.

## Definiciones

- **Corrida:** una ejecución de `clawg hook session-start`, de `--client claude-code` o de `--client cursor`, que llegó a leer su entrada. `user-prompt-submit` no es una corrida, con ningún cliente.
- **Registro:** `<data>/hook-runs.jsonl`, un objeto JSON por línea. Al llegar a 64 KiB (65.536 bytes) pasa a `hook-runs.1.jsonl`.
- **Campos de una corrida:** `at_ms` (milisegundos Unix al empezar), `client` (`claude-code` o `cursor`), `agent`, `source` (el de la entrada, o `null`), `outcome`, `bytes` (los de la salida impresa en stdout, sin el salto de línea final; 0 si no se imprimió nada), `elapsed_ms`, `roots` (la cantidad de `workspace_roots`, o `null` si no se mandaron), `folder` (la carpeta cuyo proyecto se leyó, como llegó, o `null`), `cursor_version` (el texto de ese campo, o `null`) e `input_keys` (los nombres de los campos de primer nivel de la entrada, en su orden; vacío si la entrada no es un objeto JSON).
- **`outcome`:** uno de `injected`, `empty_context`, `several_folders`, `no_folder`, `no_daemon`, `project_refused`, `daemon_error`, `timed_out`, `bad_input`, `other_client`.

## HR1. Cada inicio de sesión deja un renglón

`clawg hook session-start`, con cualquiera de los dos clientes, MUST agregar una línea al registro con los campos de una corrida, y MUST NOT agregar ninguna en `user-prompt-submit`. `folder` MUST ser la ruta tal como llegó (`cwd`, la única raíz de Cursor), sin normalizar. `input_keys` MUST ser los nombres y no los valores.

#### Escenario: un `sessionStart` de Cursor
- GIVEN un directorio de datos que existe y una entrada de Cursor con una raíz, `user_email` y los campos de su documentación
- WHEN corre `clawg hook session-start --agent cursor --client cursor`
- THEN el registro tiene una línea con `client` `cursor`, `agent` `cursor`, `roots` 1, la raíz tal como llegó en `folder`, la versión en `cursor_version` y `input_keys` con los nombres de los campos de la entrada

**Tests:** `a_session_start_leaves_one_line_with_its_fields` (`hook.rs`), `a_cursor_session_start_leaves_its_line_and_no_input_value` (`tests/binary.rs`), `user_prompt_submit_leaves_no_run` (`hook.rs`), `a_run_is_one_json_line_with_every_field` e `input_keys_are_the_names_of_the_top_level_fields` (`hook_log.rs`).

## HR2. El motivo dice qué pasó con la memoria

`outcome` MUST salir de estos hechos, en este orden:

| `outcome` | Cuándo |
|---|---|
| `bad_input` | la entrada no se pudo leer entera ni es JSON |
| `other_client` | `--client claude-code` y la entrada trae `cursor_version` |
| `timed_out` | el plazo del hook venció leyendo la entrada o esperando al daemon |
| `several_folders` | `--client cursor` y `workspace_roots` tiene más de una raíz |
| `no_folder` | no hay carpeta (Cursor sin raíces, o el directorio actual ilegible) |
| `no_daemon` | no hay un daemon al que hablarle |
| `project_refused` | el daemon rechaza la raíz porque su id pertenece a otra |
| `daemon_error` | el daemon respondió con un error |
| `empty_context` | el daemon respondió y el texto del contexto está vacío |
| `injected` | el daemon respondió y el contexto tiene texto |

Con la línea del idioma, un aviso o cualquier otro texto impreso, `outcome` MUST conservar su motivo y `bytes` MUST mostrar lo impreso.

#### Escenario: sin daemon y con un idioma elegido
- GIVEN la preferencia `es` y ningún daemon
- WHEN corre `clawg hook session-start`
- THEN la línea dice `no_daemon` y `bytes` es el largo de lo que se imprimió: la línea del idioma

**Tests:** `every_session_start_names_why`, `the_language_line_alone_keeps_its_reason`, `an_empty_context_and_an_injected_one_differ`, `a_refused_root_is_project_refused` (`hook.rs`).

## HR3. Nada de la entrada, fuera de los campos

El registro MUST NOT contener el valor de `user_email`, el texto del contexto ni el valor de ningún campo de la entrada fuera de `source`, `cwd`, `workspace_roots` (una carpeta, o sólo la cuenta si son varias) y `cursor_version`. Con varias raíces, `roots` MUST ser la cuenta y `folder` MUST ser `null`.

#### Escenario: dos raíces y un correo
- GIVEN una entrada de Cursor con dos raíces, `user_email` `user@example.com` y un `transcript_path`
- WHEN corre el hook
- THEN la línea dice `several_folders`, `roots` 2, `folder` `null`, y ni el correo, ni las rutas ni el `transcript_path` aparecen en el archivo

**Tests:** `a_run_never_holds_an_input_value` (`hook_log.rs`), `a_cursor_session_start_leaves_its_line_and_no_input_value`, `two_roots_leave_their_count_and_not_their_paths` (`tests/binary.rs`).

## HR4. El registro está acotado y nunca molesta

La línea MUST armarse entera y agregarse con una sola escritura. Antes de agregar, con el registro en 64 KiB o más, MUST renombrarlo a `hook-runs.1.jsonl`, reemplazando al anterior, y empezar uno nuevo. MUST NOT crear el directorio de datos: si no existe, no registra. Cualquier falla (permisos, un directorio en lugar del archivo, un disco lleno) MUST tragarse sin nada en stdout ni en stderr y sin cambiar la salida ni el código de salida. El registro MUST escribirse antes de imprimir.

#### Escenario: el directorio de datos no existe
- GIVEN un home sin directorio de datos
- WHEN corre el hook
- THEN no hay directorio de datos después, y la salida es la de siempre

**Tests:** `a_line_is_appended`, `the_file_rolls_over_at_64_kib_and_replaces_the_old_one`, `concurrent_appends_keep_every_line`, `a_missing_data_directory_is_not_created`, `a_failed_append_is_swallowed` (`hook_log.rs`), `a_hook_does_not_create_the_data_directory_to_record` (`tests/binary.rs`).

## HR5. La salida del hook no cambia

`respond` MUST devolver lo que devolvía, con la misma firma, y `main` MUST imprimir lo mismo que antes y salir con el mismo código. Los tests de `clawg hook` que ya existen MUST pasar sin cambiar lo que esperan.

**Tests:** los de `hook.rs` y `tests/binary.rs` que ya había, y `respond_prints_what_the_run_counted` (`hook.rs`).

## HR6. `clawg doctor` muestra las últimas corridas

`clawg doctor` MUST sumar un hallazgo `hooks.runs` de severidad `info` cuando el registro tiene corridas (`reason_code` `recent`), con un `message` en el idioma del comando que lista las últimas 5, la más nueva primero: su antigüedad, el cliente, la última parte de `folder` (o `(N carpetas)` con varias, o `(sin carpeta)`) y qué pasó, en palabras. `evidence` MUST traer las últimas 20 completas, la más nueva primero. Lee `hook-runs.jsonl` y, si hace falta, `hook-runs.1.jsonl`, ignora las líneas que no son una corrida, MUST NOT abrir el store para esto y MUST NOT cambiar el código de salida ni la severidad por ningún `outcome`. El JSON de `clawg doctor` es la salida completa, y es lo que se pega.

#### Escenario: corridas de dos clientes
- GIVEN siete corridas en el registro
- WHEN corre `clawg doctor`
- THEN `hooks.runs` es `info`, su `message` lista las cinco últimas y su `evidence.runs` las siete

**Tests:** `recent_runs_are_listed_newest_first_five_in_the_message_and_twenty_in_the_evidence`, `every_outcome_has_words_and_none_raises_the_severity`, `a_folder_is_named_by_its_last_part`, `ages_are_counted_in_words` (`checks.rs`), `doctor_lists_the_hook_runs_and_still_exits_0` (`tests/binary.rs`).

## HR7. El hook instalado que nadie corrió

Con el hook de Cursor del perfil instalado en `~/.cursor/hooks.json` (la regla de `setup cursor`) y sin corridas, `clawg doctor` MUST dar un hallazgo `hooks.runs` `info` con `reason_code` `not_run`, que dice que el hook está instalado y todavía ningún chat lo corrió, y que el paso siguiente es abrir un chat nuevo y correr `clawg doctor` otra vez. Sin hook instalado y sin corridas MUST NOT dar ningún hallazgo `hooks.runs`.

#### Escenario: instalado y sin corridas
- GIVEN `clawg setup cursor` aplicado y ningún registro
- WHEN corre `clawg doctor`
- THEN `hooks.runs` es `info` `not_run`; y sin `setup cursor` no hay ningún `hooks.runs`

**Tests:** `an_installed_hook_with_no_runs_says_so_and_none_installed_says_nothing` (`checks.rs`), `the_hook_of_this_profile_is_found_in_hooks_json` (`setup/cursor.rs`), `doctor_says_when_the_installed_hook_never_ran` (`tests/binary.rs`).
