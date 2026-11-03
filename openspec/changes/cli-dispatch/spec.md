# cli-dispatch — Especificación

Cómo se organiza por dentro el despacho de los comandos de `clawg`. Ningún comando cambia de conducta: lo que dice está en las specs de cada dominio ([`specs/doctor`](../../specs/doctor/spec.md), [`specs/portability`](../../specs/portability/spec.md), [`specs/secrets`](../../specs/secrets/spec.md), [`specs/preferences`](../../specs/preferences/spec.md) y las demás), y esas specs no cambian. Al archivar, estos requisitos se suman a [`specs/quality`](../../specs/quality/spec.md).

## Definiciones

- **Lo común:** lo que un comando necesita antes de hacer lo suyo y que comparte con otros: su perfil, su directorio de datos, su idioma y, si lo necesita, el daemon corriendo.
- **Función del comando:** la función de su módulo en `clawg_cli` a la que `dispatch` le pasa lo común y los argumentos del comando, y que devuelve lo que se imprime o el error.
- **Frase compuesta:** un texto que clawg arma para la persona, en el idioma del comando ([`specs/preferences`](../../specs/preferences/spec.md)): la que enmarca un fallo, una negativa o un informe.

## CD1. El texto de un error es el de antes

`dispatch` MUST devolver `Result<ExitCode, CliError>`, y `CliError` MUST derivar `thiserror::Error`. Para un error tipado que hoy se muestra con su `Display`, el `Display` de `CliError` MUST ser exactamente el de la fuente. Una frase compuesta MUST mostrarse tal como se compuso, y MUST entrar en `CliError` de forma explícita: `CliError` MUST NOT implementar `From<String>`. `main` MUST imprimir `clawg: ` y el error en stderr y salir con 1, como hoy.

#### Escenario: un perfil inválido
- GIVEN `CLAWG_PROFILE=bogus`
- WHEN se corre cualquier comando con perfil
- THEN stderr dice `clawg: ` seguido del mismo texto que `ProfileArg::resolve` da hoy, y el código es 1

#### Escenario: una frase en español
- GIVEN una negativa compuesta en español
- WHEN llega a `main`
- THEN se imprime tal cual, sin pasar por ninguna otra traducción

**Test:** `a_typed_error_reads_as_its_source` y `a_composed_sentence_reads_as_it_was_written` (`crates/clawg-cli/src/error.rs`). Fallan hoy: no hay `CliError`.

## CD2. La frase que enmarca un fallo la compone el módulo del comando

Cuando un comando dice un fallo con palabras propias, la frase MUST componerla una función pura de su módulo que recibe el idioma del comando, y MUST NOT componerla `main` ni `dispatch`. La causa técnica MUST ir tal cual dentro de la frase, como pide [`specs/preferences`](../../specs/preferences/spec.md). Los 13 comandos que necesitan el daemon y no lo arrancan (`import`, `undelete`, `outbox`, `projects` y `memories`) MUST decir que no lo encontraron con `memories::explain_unavailable`, a través de un solo lugar.

#### Escenario: `stop` sin daemon
- GIVEN un directorio de datos sin daemon
- WHEN se corre `clawg stop` con el idioma `en`, y después con `es`
- THEN dice `no clawg daemon is running for profile dev`, y después `no hay ningún daemon de clawg corriendo para el perfil dev`, con código 1

#### Escenario: un comando que necesita el daemon
- GIVEN un directorio de datos sin daemon
- WHEN un comando busca el daemon con el idioma `es`
- THEN el error dice `no hay ningún daemon de clawg corriendo para el perfil dev; arráncalo con 'clawg serve --profile dev'`

**Test:** `a_missing_daemon_is_named_in_both_languages` y `the_stop_line_speaks_both_languages` (`crates/clawg-cli/src/stop.rs`); `a_missing_daemon_says_how_to_start_it_in_both_languages` y `a_published_daemon_is_found` (`crates/clawg-cli/src/running.rs`). Fallan hoy: la frase de `stop` está en `main.rs` y no hay un lugar para buscar el daemon.

## CD3. `status` se prueba sin `main`

La función de `clawg status` MUST recibir el perfil y el directorio de datos y devolver el JSON que se imprime y el código de salida: 0 si `/health` contesta, 1 si no. Con un daemon, el informe MUST llevar su `health` y MUST NOT sondear el store; sin daemon, MUST llevar lo que dice el store.

#### Escenario: sin daemon
- GIVEN un directorio de datos vacío
- WHEN se llama la función de `status`
- THEN el JSON dice `"running": false` y `"store": {"state": "missing", …}`, y el código es 1

#### Escenario: con un daemon
- GIVEN un daemon en proceso que publicó su estado en el directorio
- WHEN se llama la función de `status`
- THEN el JSON dice `"running": true`, lleva el `health` del daemon y `"store": null`, y el código es 0

**Test:** `status_without_a_daemon_reports_the_store_and_exits_1` y `status_with_a_daemon_reports_its_health_and_exits_0` (`crates/clawg-cli/src/status.rs`). Fallan hoy: la lógica está en el brazo de `main.rs` y no hay función que llamar.

## CD4. Cada comando con lógica propia se prueba sin `main`

Un comando cuyo brazo hacía algo además de resolver lo común, llamar una función y mostrar lo que devuelve MUST tener una función del comando en su módulo, y lo que decidía el brazo MUST estar probado ahí:

| Comando | Lo que decide | Test |
|---|---|---|
| `backup` | el snapshot lo saca el daemon si corre; sin daemon, la copia sin lock, y su error tal cual | `a_running_daemon_takes_the_snapshot`, `without_a_daemon_the_missing_store_is_said_as_it_came` (`backup.rs`) |
| `restore` | un store que tiene el daemon se niega diciendo cómo liberarlo, en el idioma del comando | `a_store_the_daemon_holds_is_refused_in_the_commands_language` (`restore.rs`) |
| `export` | `--project` se resuelve contra el store antes de escribir nada | `a_project_without_a_store_is_refused_before_anything_is_written` (`export.rs`) |
| `import` | importar un archivo o deshacer una importación, después de encontrar el daemon | `neither_a_file_nor_an_undo_is_refused_once_the_daemon_answers` (`import.rs`) |
| `secrets`, `purge` | sin `--yes` se niega antes de abrir el store; `purge` pide una memoria o un borrado | `a_purge_without_yes_is_refused_before_the_store_is_opened`, `a_purge_names_a_memory_or_a_delete`, `a_scan_without_a_store_fails_as_the_store_says` (`secrets.rs`) |
| `metrics report` | el informe de una métrica o los eventos | `the_flags_choose_the_report_or_the_events` (`metrics.rs`) |
| `projects` | la acción, su negativa y su código de salida | `each_projects_action_names_its_profile` (`cli.rs`), `a_listing_and_a_refused_rename_go_through_the_running_daemon` (`projects.rs`) |
| `path` | fuera de Windows, la negativa | `outside_windows_the_path_command_says_what_to_do_instead` (`user_path.rs`) |

**Test:** los de la tabla. Fallan hoy: no hay función del comando que llamar.

## Invariantes

- Ningún comando cambia sus argumentos, su ayuda, lo que imprime en stdout y en stderr, ni su código de salida. `crates/clawg-cli/tests/binary.rs` pasa sin editarse, y los tests que ya fijan informes y negativas pasan sin cambiar el texto que esperan.
- El idioma se lee en el mismo punto que antes: un `preferences.json` inválido escribe su línea en stderr en los mismos casos.
- Lo que ve el modelo no cambia: `agent_contract.json` y `agent_contract.es.json` pasan sin bendecir.
