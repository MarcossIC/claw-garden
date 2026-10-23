# preferences — Especificación

Fuente de verdad del comportamiento actual de las preferencias del usuario: el archivo `preferences.json` de un perfil, cómo se lee y se escribe, qué pasa sin él o con uno roto, y `clawg preferences`. La única preferencia es el idioma, `es` o `en`. Lo que el idioma cambia vive en las specs de cada parte: el idioma de cada fila en [`specs/storage`](../storage/spec.md) (LP4, LP5) y [`specs/portability`](../portability/spec.md) (LP6), el conteo del otro idioma en [`specs/retrieval`](../retrieval/spec.md) (LP7), el puente en [`specs/mcp`](../mcp/spec.md) (LP8 a LP11), `session-start` en [`specs/hooks`](../hooks/spec.md) (LP12), `setup` en [`specs/setup`](../setup/spec.md) (LP13), `doctor` en [`specs/doctor`](../doctor/spec.md) (LP14, CL4) y `rag-eval --lang-detect` en [`specs/eval`](../eval/spec.md) (LP15). Lo que el agente lee en el idioma preferido también: el protocolo, las descripciones de las herramientas y las respuestas del puente en [`specs/mcp`](../mcp/spec.md) (AL1 a AL4), las notas del contexto y el parser de `rag-eval` en [`specs/retrieval`](../retrieval/spec.md) (AL5, AL7), los hooks en [`specs/hooks`](../hooks/spec.md) (AL6) y el contrato en español en [`specs/quality`](../quality/spec.md). Lo que los comandos le imprimen a la persona está acá (CL1 a CL3 y CL5), y cada informe en la spec de su comando: [`specs/setup`](../setup/spec.md) (R7), [`specs/self-update`](../self-update/spec.md) (S11), [`specs/secrets`](../secrets/spec.md) (CL3) y [`specs/portability`](../portability/spec.md) (CL3).

**Origen:**
- [language-preference](../../changes/archive/2026-10-23-language-preference/proposal.md), archivado el 2026-10-23. Los ids de requisito son los de esa spec; acá están LP1 a LP3.
- [cli-language](../../changes/archive/2026-10-23-cli-language/proposal.md), archivado el 2026-10-23: lo que imprimen los comandos en el idioma preferido (CL1 a CL3 y CL5; CL4 está en [`specs/doctor`](../doctor/spec.md)).

## Definiciones

- **Preferencia:** `<data>/preferences.json` de un perfil (`paths::preferences_file`), leído con éxito y con `language` puesto. Un archivo ausente, inválido o sin `language` es **sin preferencia**.
- **Idioma preferido:** el `language` de la preferencia, `es` o `en` (`Lang`, `crates/clawg-core/src/preferences.rs:21`). El **otro idioma** es el que no es el preferido.
- **Idioma del comando:** el idioma preferido del perfil del comando, leído una vez al empezar (`language`, `crates/clawg-cli/src/main.rs`, sobre `language_in`); `en` sin preferencia o con un archivo que no se puede leer.
- **Frase del CLI:** un texto que clawg compone para la persona y un comando imprime: un informe, un aviso, la frase que enmarca un fallo o el `message` de un hallazgo de `doctor`.
- **Causa técnica:** el `Display` de un error tipado, un error de SQLite, del sistema, de HTTP o del servidor de releases. No es una frase del CLI aunque vaya dentro de una.

## LP1. La preferencia y su archivo

`preferences.json` MUST tener la forma `{"version": 1, "language": "es"|"en"}`, con `language` opcional (`Preferences`, `preferences.rs:89`). Leerlo (`load`, `preferences.rs:113`) MUST rechazar una versión distinta de 1, un campo desconocido y un idioma que no sea `es` ni `en`. Un archivo ausente MUST dar «sin preferencia» sin error. `language_in` (`preferences.rs:124`) MUST loguear un archivo inválido y leerlo como sin preferencia. Escribirlo (`save`, `preferences.rs:137`) MUST pasar por un archivo temporal y un `rename` dentro del directorio de datos, creado privado como el resto (`private_dir`). El archivo MUST NOT estar en `memory.db`, en un snapshot ni en un export.

#### Escenario: ida y vuelta
- WHEN se escribe la preferencia `es` y se vuelve a leer
- THEN da `es`, y el texto es `{"version": 1, "language": "es"}` con salto de línea final

#### Escenario: lo que se rechaza
- WHEN el archivo dice `"version": 2`, `"lenguaje": "es"` o `"language": "fr"`
- THEN la lectura falla con un error que nombra el archivo y la causa, y quien lo lea como idioma obtiene «sin preferencia»

**Test:** `a_missing_file_is_no_preference`, `a_preference_round_trips`, `an_unknown_version_field_or_language_is_refused`, `an_invalid_file_reads_as_no_preference` y `saving_goes_through_a_temporary_file` (`preferences.rs`).

## LP2. Sin preferencia, lo de antes

Sin preferencia, las instrucciones del servidor, la lista de herramientas (salvo la descripción de LP10, [`specs/mcp`](../mcp/spec.md)), las respuestas de las herramientas, el contexto, lo que imprimen los hooks, el bloque que escribe `setup` y los hallazgos de `doctor` MUST ser los de antes de las preferencias. Un archivo inválido MUST NOT hacer fallar ningún comando, herramienta ni hook: el puente, `session-start`, `setup` y cada comando de CL3 lo leen con `language_in`, y sólo `doctor` lo nombra (LP14).

#### Escenario: nada cambia
- GIVEN un perfil sin `preferences.json`
- THEN `get_info().instructions` es la apertura de `protocol.md`, `session_start_text` da lo de antes, `setup` deja el bloque instalado `Unchanged` y `doctor` no tiene hallazgos `preferences` ni `protocol_block`

#### Escenario: un archivo roto
- GIVEN un `preferences.json` que no es JSON
- THEN el puente arranca y sus instrucciones son las de sin preferencia, `session-start` imprime lo de sin preferencia y `doctor` lo marca

**Test:** `without_a_preference_the_instructions_are_the_opening` (`server.rs`); `without_a_preference_session_start_says_what_it_said` (`hook.rs`); `an_invalid_file_reads_as_no_preference` (`preferences.rs`), la lectura que hacen todos; los tests de `server.rs`, `hook.rs`, `setup.rs`, `prompt_builder.rs` y `the_agent_contract_matches_its_file` con sus textos esperados de antes. Ningún test arranca el puente con un archivo roto.

## LP3. `clawg preferences`

`clawg preferences [--profile P]` MUST mostrar la preferencia o decir que no hay y que clawg usa el inglés. `clawg preferences set language es|en` MUST escribirla y `clawg preferences unset language` MUST quitarla, y las dos MUST decir que se aplica a las sesiones del cliente que se abran después y que `clawg setup claude-code` reescribe el bloque del protocolo. Un archivo inválido MUST hacer fallar `set` y `unset` con salida 1, nombrando la causa, sin tocarlo. Las frases del comando MUST salir en el idioma preferido después del cambio (en inglés sin preferencia), un `match` sobre `Lang` por frase (`crates/clawg-cli/src/preferences.rs`). MUST NOT necesitar el daemon ni abrir el store.

#### Escenario: fijar y quitar
- WHEN se corre `clawg preferences set language es --profile dev`
- THEN `preferences.json` del perfil dev dice `es`, la salida está en español y nombra la sesión nueva y `clawg setup claude-code`
- WHEN se corre después `clawg preferences unset language --profile dev`
- THEN el archivo no tiene `language` y la salida está en inglés

**Test:** `preferences_parses_set_and_unset` (`cli.rs`); `the_preferences_command_speaks_the_language_it_sets` (`preferences.rs` de `clawg-cli`); `preferences_set_and_unset_write_the_profiles_file` (`tests/binary.rs`).

## CL1. El idioma del comando

Cada comando de la lista de CL3 MUST leer el idioma del comando del directorio de datos de su perfil antes de imprimir, una sola vez. Un `preferences.json` inválido MUST dar `en` y MUST NOT cambiar el código de salida del comando.

#### Escenario: un archivo roto
- GIVEN un perfil con `preferences.json` que no es JSON
- WHEN se corre `clawg doctor`
- THEN los `message` salen en inglés, el hallazgo `preferences` es `warn` `invalid` y el código de salida es el que da el resto de los hallazgos

**Test:** `doctor_names_a_broken_preferences_file` y `stop_speaks_the_preferred_language`, con un `preferences.json` roto (`tests/binary.rs`).

## CL2. Sin preferencia, lo de antes

Con el idioma del comando `en`, cada frase del CLI MUST ser idéntica, byte a byte, a la de antes de las preferencias.

#### Escenario: nada cambia
- GIVEN un perfil sin `preferences.json`
- THEN los informes de `setup`, `update`, `secrets`, `purge`, `export`, `import`, `restore`, `undelete`, `outbox`, `projects`, `memories`, `stop` y los `message` de `doctor` son los de antes

**Test:** los tests de cada informe anteriores a las preferencias pasan con `Lang::En` sin cambiar el texto que esperan.

## CL3. Las frases en el idioma del comando

Con el idioma del comando `es`, cada frase del CLI de estos comandos MUST estar en español, un `match` sobre `Lang` por frase en la función que compone el informe: `doctor` (cada `message`, [`specs/doctor`](../doctor/spec.md) CL4), `update` (el informe de éxito, el de fallo y el de `--check`, [`specs/self-update`](../self-update/spec.md) S11), `setup` (cada línea del informe, [`specs/setup`](../setup/spec.md) R7), `secrets` y `purge` (los informes y las negativas sin `--yes`, [`specs/secrets`](../secrets/spec.md) CL3), `export` e `import` ([`specs/portability`](../portability/spec.md) CL3), `restore`, `undelete`, `outbox retry|discard`, `projects add|move`, `sharing` (la política que imprime, que compone `SharingPolicy::render` en `clawg-core`, la negativa de ocultar a un agente de sí mismo y la frase que enmarca un archivo inválido), `metrics report` (la falta del archivo o de la métrica), `memories` (las listas vacías, el pie de cada listado, el resumen de `stats`, la falta de daemon y el aviso de versión) y `stop`. El español es neutro, con imperativo de tú en los pasos («ejecuta `clawg stop`»).

Una causa técnica MUST ir tal cual dentro de la frase. Los identificadores (`check_id`, `reason_code`, `severity`, `evidence`, `safe_next_step`, las claves de cada JSON, los valores de `state`, los tipos de memoria), los comandos entre comillas invertidas, los ids, las rutas, los códigos de salida y lo que un comando lista como datos (el texto de una fila, las columnas de `memories list`, las etiquetas de `memories show`) MUST NOT cambiar. La ayuda y los errores de `clap`, los logs, `clawg serve`, `clawg path` (que no tiene perfil) y `scripts/install.ps1` quedan en inglés.

#### Escenario: el informe de un purge
- GIVEN el idioma del comando `es`
- WHEN `clawg purge 7 --yes` borra la fila y revive la #3
- THEN la salida empieza por `se purgó la memoria #7` y nombra `#3`, con los mismos ids que en inglés

#### Escenario: una causa técnica
- GIVEN el idioma del comando `es` y una actualización que falla al bajar el binario por `the download does not match SHA256SUMS`
- THEN la primera línea está en español y contiene `the download does not match SHA256SUMS` tal cual

**Test:** `the_secrets_report_speaks_spanish`, `the_purge_report_speaks_spanish` (`secrets.rs`), `the_update_reports_speak_spanish` (`update/report.rs`), `the_report_names_the_versions` (`update/mod.rs`), `the_setup_report_speaks_spanish` (`setup.rs`), `the_export_report_agrees_with_its_numbers_in_both_languages` (`export.rs`), `the_import_report_speaks_spanish` (`import.rs`), `the_move_report_speaks_spanish` (`projects.rs`), `the_restore_lines_speak_spanish` (`restore.rs`), `the_undelete_report_speaks_spanish` (`undelete.rs`), `the_outbox_reports_speak_spanish` (`outbox.rs`), `the_sharing_command_speaks_spanish` (`sharing.rs`), `a_missing_file_or_metric_is_named_in_spanish` (`metrics.rs`), `the_viewer_speaks_spanish` (`memories.rs`), `render_lists_one_exclusion_per_line` (`clawg-core`, `memory/sharing.rs`); `stop_speaks_the_preferred_language` (`tests/binary.rs`).

## CL5. Concordancia

Una frase del CLI con un número MUST concordar con él en cada idioma: singular con uno, plural con otro número. Donde el español pide una concordancia que el inglés no, la rama `Lang::Es` la escribe aparte.

#### Escenario: una memoria y varias
- WHEN `clawg secrets` encuentra una fila con un secreto, y después dos
- THEN en `es` dice `1 memoria guarda un secreto` y `2 memorias guardan un secreto`

**Test:** `the_secrets_report_speaks_spanish` (`secrets.rs`), `the_export_report_agrees_with_its_numbers_in_both_languages` (`export.rs`) y `a_finding_in_spanish_agrees_with_its_number` (`doctor/checks.rs`), con uno y con varios.
