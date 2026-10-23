# cli-language — Especificación

Suma requisitos a [`specs/preferences`](../../specs/preferences/spec.md), [`specs/doctor`](../../specs/doctor/spec.md), [`specs/setup`](../../specs/setup/spec.md), [`specs/self-update`](../../specs/self-update/spec.md), [`specs/secrets`](../../specs/secrets/spec.md) y [`specs/portability`](../../specs/portability/spec.md). Lo que no se nombra aquí sigue como está.

## Definiciones

- **Idioma del comando:** el idioma preferido del perfil del comando ([`specs/preferences`](../../specs/preferences/spec.md)), leído una vez al empezar; `en` sin preferencia o con un archivo que no se puede leer.
- **Frase del CLI:** un texto que clawg compone para la persona y un comando imprime: un informe, un aviso, la frase que enmarca un fallo o el `message` de un hallazgo de `doctor`.
- **Causa técnica:** el `Display` de un error tipado, un error de SQLite, del sistema, de HTTP o del servidor de releases. No es una frase del CLI aunque vaya dentro de una.

## CL1. El idioma del comando

Cada comando de la lista de CL3 MUST leer el idioma del comando del directorio de datos de su perfil antes de imprimir, una sola vez. Un `preferences.json` inválido MUST dar `en` y MUST NOT cambiar el código de salida del comando.

#### Escenario: un archivo roto
- GIVEN un perfil con `preferences.json` que no es JSON
- WHEN se corre `clawg doctor`
- THEN los `message` salen en inglés, el hallazgo `preferences` es `warn` `invalid` y el código de salida es el que da el resto de los hallazgos

**Test:** `doctor_names_a_broken_preferences_file` (`tests/binary.rs`), que ya existe y sigue pasando con sus mensajes en inglés.

## CL2. Sin preferencia, lo de hoy

Con el idioma del comando `en`, cada frase del CLI MUST ser idéntica, byte a byte, a la de la versión anterior.

#### Escenario: nada cambia
- GIVEN un perfil sin `preferences.json`
- THEN los informes de `setup`, `update`, `secrets`, `purge`, `export`, `import`, `restore`, `undelete`, `outbox`, `projects`, `memories`, `stop` y los `message` de `doctor` son los de antes

**Test:** los tests actuales de cada informe pasan con `Lang::En` sin cambiar el texto que esperan.

## CL3. Las frases en español

Con el idioma del comando `es`, cada frase del CLI de estos comandos MUST estar en español: `doctor` (cada `message`), `update` (el informe de éxito, el de fallo, el de `--check` y la frase de un directorio que no se puede escribir), `setup` (cada línea del informe), `secrets` y `purge` (los informes y las negativas sin `--yes`), `export`, `import`, `restore`, `undelete`, `outbox retry|discard`, `projects add|move`, `sharing` (la frase que enmarca un archivo inválido), `metrics report` (la falta del archivo o de la métrica), `memories` (los avisos de lista vacía, el pie de `list --deleted`, la falta de daemon y el aviso de versión), `stop` y `path`. Una causa técnica MUST ir tal cual dentro de la frase. Los comandos entre comillas invertidas, los ids, las rutas y los nombres de memoria MUST NOT cambiar.

#### Escenario: el informe de un purge
- GIVEN el idioma del comando `es`
- WHEN `clawg purge 7 --yes` borra la fila y revive la #3
- THEN la salida empieza por `se purgó la memoria #7` y nombra `#3`, con los mismos ids que en inglés

#### Escenario: una causa técnica
- GIVEN el idioma del comando `es` y una actualización que falla al bajar el binario por `the download does not match SHA256SUMS`
- THEN la primera línea está en español y contiene `the download does not match SHA256SUMS` tal cual

**Test:** un test por informe con `Lang::Es`: `the_secrets_report_speaks_spanish`, `the_purge_report_speaks_spanish`, `the_update_reports_speak_spanish`, `the_setup_report_speaks_spanish`, `the_export_and_import_reports_speak_spanish`, `the_move_report_speaks_spanish`, y los de `restore`, `undelete`, `outbox` y `memories`; `stop_speaks_the_preferred_language` (`tests/binary.rs`).

## CL4. Los identificadores de `doctor` no cambian

Los hallazgos de `doctor` de unos mismos hechos MUST tener, en `en` y en `es`, el mismo `check_id`, `severity`, `reason_code`, `evidence`, `safe_next_step` y `requires_confirmation`; sólo `message` cambia. Las claves y los demás valores del JSON de `doctor` MUST NOT cambiar.

#### Escenario: mismos hechos, dos idiomas
- GIVEN los hechos que hacen saltar cada `reason_code` de `checks.rs` y `mcp.rs`
- WHEN se corren los chequeos con `en` y con `es`
- THEN las dos listas de hallazgos son iguales salvo en `message`, y cada `message` de `es` es distinto del de `en`

**Test:** `the_findings_are_the_same_in_both_languages_but_their_message` (`doctor/checks.rs`) y `the_mcp_findings_are_the_same_in_both_languages_but_their_message` (`doctor/mcp.rs`); `doctor_speaks_the_preferred_language` (`tests/binary.rs`).

## CL5. Concordancia

Una frase del CLI con un número MUST concordar con él en cada idioma: singular con uno, plural con otro número.

#### Escenario: una memoria y varias
- WHEN `clawg secrets` encuentra una fila con un secreto, y después dos
- THEN en `es` dice `1 memoria guarda un secreto` y `2 memorias guardan un secreto`

**Test:** `the_secrets_report_speaks_spanish` (`secrets.rs`) y `the_export_and_import_reports_speak_spanish` (`export.rs`, `import.rs`) con uno y con varios.
