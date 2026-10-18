# secrets — Especificación

Fuente de verdad del comportamiento actual con secretos: el detector (formatos conocidos, patrones por contexto y la marca privada), el enmascarado al ingerir, el campo `redacted` del save, `clawg secrets` y `clawg purge`. El resto del contrato de `POST /v1/memories` y de `save_memory` está en [`specs/ingest`](../ingest/spec.md). Que una purga borre también los enlaces de procedencia de la fila está en [`specs/synthesis`](../synthesis/spec.md).

**Origen:**
- [secret-hygiene](../../changes/archive/2026-10-14-secret-hygiene/proposal.md), archivado el 2026-10-14. Los ids de requisito son los de esa spec.
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (R2 y R3: el enmascarado ocurre antes de escribir la fila del outbox; R5 y R6: `purge` y los backups cubren el outbox).
- [redaction-precision](../../changes/archive/2026-10-15-redaction-precision/proposal.md), archivado el 2026-10-15 (`sk-` con la forma de una clave, la marca privada, los patrones por contexto y cuatro formatos nuevos en R1; lo guardado antes de una regla nueva en R4).

## Definiciones

- **Tipo de secreto:** uno de los de R1 (un formato conocido, un patrón por contexto o `private`), con su nombre en `snake_case` (`SecretKind` y `as_str`, `crates/clawg-core/src/memory/redact.rs:17` y `redact.rs:38`).
- **Placeholder:** `[REDACTED:<tipo>]`, el texto que reemplaza al secreto.
- **Marca privada:** el texto desde una apertura `<private>` (sin distinguir mayúsculas) hasta el primer cierre `</private>` que la sigue, o hasta el final del texto si no hay cierre.
- **Valor literal:** el valor de un patrón por contexto que cumple la regla de R1.
- **Fila con secreto:** una fila de `memory`, viva o borrada, cuyo `content` tiene al menos un match de R1.
- **Purgar:** sacar una fila de `memory` de la base para siempre, con su vector y sus términos del FTS, de modo que su texto no quede en `memory.db` ni en `memory.db-wal`.

## Enmascarado

### R1. Detector puro de secretos

`find_secrets(&str)` (`redact.rs:179`) MUST devolver los rangos y tipos de cada secreto, sin I/O, y `redact(&str)` (`redact.rs:207`) MUST devolver el texto con cada rango reemplazado por su placeholder, más los tipos encontrados (sin repetir, en orden de aparición). Los patrones se compilan una vez (`PATTERNS`, `redact.rs:80`); `accepts` (`redact.rs:138`) descarta los matches que el regex solo no puede excluir. El detector MUST NOT usar entropía: un hash de commit, un UUID o un checksum también parecen aleatorios, y son memoria que vale la pena guardar.

#### Formatos conocidos

| Tipo | Forma |
|---|---|
| `aws_access_key` | `AKIA` o `ASIA` y 16 de `[A-Z0-9]`, como palabra entera |
| `github_token` | `ghp_`, `gho_`, `ghu_`, `ghs_` o `ghr_` y 36 de `[A-Za-z0-9]`; o `github_pat_` y 22 o más de `[A-Za-z0-9_]` |
| `anthropic_key` | `sk-ant-` y 20 o más de `[A-Za-z0-9_-]` |
| `openai_key` | `sk-`, opcionalmente `proj-`, `svcacct-` o `admin-`, y 20 o más de `[A-Za-z0-9_-]` con al menos una mayúscula, una minúscula y un dígito ASCII; nunca `sk-ant-` |
| `slack_token` | `xoxa-`, `xoxb-`, `xoxp-`, `xoxo-`, `xoxs-` o `xoxr-` y 10 o más de `[A-Za-z0-9-]` |
| `google_api_key` | `AIza` y 35 de `[A-Za-z0-9_-]` |
| `stripe_key` | `sk_live_` o `rk_live_` y 16 o más de `[A-Za-z0-9]` |
| `gitlab_token` | `glpat-` y 20 o más de `[A-Za-z0-9_-]` |
| `npm_token` | `npm_` y exactamente 36 de `[A-Za-z0-9]`, como palabra entera |
| `google_oauth_secret` | `GOCSPX-` y 28 de `[A-Za-z0-9_-]` |
| `sendgrid_key` | `SG.`, 22 de `[A-Za-z0-9_-]`, `.` y 43 de `[A-Za-z0-9_-]` |
| `private_key` | de `-----BEGIN … PRIVATE KEY-----` al `-----END … PRIVATE KEY-----` que lo cierra, o al final del texto si no se cierra |
| `jwt` | `eyJ` y 10 o más de `[A-Za-z0-9_-]`, `.`, `eyJ` y 10 o más, `.` y la firma (`[A-Za-z0-9_-]*`) |
| `url_password` | sólo la contraseña de `esquema://usuario:contraseña@`; se conservan el esquema, el usuario y el host. No se enmascara si empieza con `$`, `<`, `{`, `[` o `*` (una variable, un ejemplo o un placeholder) |

Los prefijos MUST empezar en un borde de palabra. `sk-` solo es un prefijo común en identificadores, rutas y nombres de paquete, que casi nunca mezclan mayúsculas y minúsculas; una clave real es base62 o base64url aleatoria. Un identificador que sí las mezcla y tiene un dígito después de `sk-` se sigue enmascarando.

#### Patrones por contexto

Dos patrones MUST enmascarar sólo el valor y conservar la clave, el separador y las comillas (`redact.rs:103` y `redact.rs:107`):

- `password`: el valor de una clave que termina en `password` o `passwd` (con cualquier prefijo de palabra, como `DB_PASSWORD` o `PGPASSWORD`; sin distinguir mayúsculas; entre comillas o no), seguida de `=`, `:`, `:=` o `==`. Un valor nunca empieza con `:`, así que `password::rules::check` es un camino a través de un módulo y no una clave con su valor. `password_hash` o `passwordFile` no terminan en `password` y no cuentan.
- `auth_header`: el valor después de `Authorization:` o `Authorization=` (también con la clave entre comillas, como en JSON) y `Bearer`, `Basic` o `Token`.

El valor MUST enmascararse sólo si es un valor literal (`is_literal_secret`, `redact.rs:162`): al menos 6 caracteres; no empieza con `$`, `%` ni `*`; no tiene `( ) [ ] { } < >`; no es un camino de identificadores separados por puntos; y tiene un dígito o un símbolo distinto de `-`, `_` y `.`. Una contraseña de sólo letras y guiones queda sin enmascarar: después de esas claves es mucho más frecuente un tipo, una variable o un valor de ejemplo, y enmascararlo destruye la memoria.

#### La marca privada

`find_secrets` MUST reportar cada marca privada con el tipo `private` (`private_spans`, `redact.rs:121`); el rango cubre las dos marcas y lo que hay entre ellas, o va hasta el final del texto si no hay cierre: quien abrió la marca dijo que lo que sigue es secreto. Una apertura precedida inmediatamente por una comilla invertida MUST NOT abrir una marca, para poder escribir sobre la marca sin perder texto. El índice de docs enmascara cada archivo entero con la misma función antes de trocearlo (`crates/clawg-core/src/memory/project_docs/mod.rs:382`), así que un archivo que escribe la apertura sin comilla invertida delante pierde, en el índice, el tramo hasta el próximo cierre del archivo o hasta su final; se acepta porque el índice es derivado y el archivo no cambia.

#### Solapamientos e idempotencia

Dos matches que se superponen MUST resolverse a favor del que empieza antes y, si empiezan juntos, del más largo; en un empate exacto gana el primero en `PATTERNS`, donde los formatos conocidos van antes que los patrones por contexto. Así un formato conocido gana sobre su contexto, y una marca privada, que empieza antes que cualquier secreto de adentro, deja un solo placeholder. `redact` MUST ser idempotente: aplicada a su propia salida no encuentra nada, con todos los tipos.

#### Escenario: un ejemplo de cada formato
- GIVEN un texto con un ejemplo de cada formato conocido, rodeado de palabras
- WHEN se enmascara
- THEN cada ejemplo quedó como su placeholder, el resto del texto quedó igual, y los tipos son los catorce, en orden

#### Escenario: no son secretos
- GIVEN un hash de commit de 40 caracteres, un UUID, un SHA-256 en hex, `MAX_LEXICAL_CANDIDATES_PER_QUERY_WINDOW`, `scikit-learn`, `sk-learn-is-not-a-key-at-all`, `sk-learn-pipeline-for-text-classification-v2-2026-model`, `scripts/sk-build-2026-release-candidate-01`, `AKIA` suelto, `ghp_short`, `postgres://app:${DB_PASSWORD}@db:5432/app` y `https://user:<password>@example.com`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

#### Escenario: claves `sk-` de cada forma
- GIVEN una clave `sk-` de base62 y una `sk-proj-`, `sk-svcacct-` y `sk-admin-` de base64url, sintéticas
- THEN cada una queda `[REDACTED:openai_key]`

#### Escenario: contraseña en una URL
- GIVEN `postgres://app:s3cr3t-pass@db.internal:5432/app`
- THEN queda `postgres://app:[REDACTED:url_password]@db.internal:5432/app`

#### Escenario: clave privada sin cierre
- GIVEN `config:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk...` sin línea `END`
- THEN queda `config:\n[REDACTED:private_key]`

#### Escenario: valores literales
- GIVEN `password=hunter2`, `export DB_PASSWORD="s3cr3t!x"`, `{"password": "p@ssw0rd"}`, `password==s3cr3t9` y `curl -H "Authorization: Bearer a1b2c3d4e5f6g7h8"`
- WHEN se enmascaran
- THEN quedan `password=[REDACTED:password]`, `export DB_PASSWORD="[REDACTED:password]"`, `{"password": "[REDACTED:password]"}`, `password==[REDACTED:password]` y `curl -H "Authorization: Bearer [REDACTED:auth_header]"`

#### Escenario: código y placeholders después de la clave
- GIVEN `password=<tu-clave>`, `password: string`, `password: Option<String>`, `password = os.getenv("DB")`, `password: cfg.db.password`, `PGPASSWORD=$PGPASS`, `password: change-me`, `password_hash: "$2b$12$abcdefghijk"`, `PWD=/home/user/app2`, `use crate::auth::password::rules::check;`, `algo: Password::Argon2id` y `Authorization: Bearer $TOKEN`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

#### Escenario: un formato conocido gana
- GIVEN `Authorization: Bearer ghp_<36>` y `OPENAI_PASSWORD=sk-proj-<192>`
- THEN quedan `Authorization: Bearer [REDACTED:github_token]` y `OPENAI_PASSWORD=[REDACTED:openai_key]`

#### Escenario: marcas cerradas
- GIVEN `user is … and ….` con `<private>Ana, 34</private>` en el primer hueco y `<PRIVATE>vive en Rosario</Private>` en el segundo
- THEN queda `user is [REDACTED:private] and [REDACTED:private].` con el tipo `private` una vez

#### Escenario: marca sin cerrar
- GIVEN `keep this ` seguido de `<private>card 4111 and everything after`, sin cierre, y una línea más
- THEN queda `keep this [REDACTED:private]`

#### Escenario: escribir sobre la marca
- GIVEN ``wrap secrets in `<private>…</private>`; an open `<private>` masks to the end``
- THEN el texto no cambia

#### Escenario: un secreto dentro de la marca
- GIVEN `<private>token ghp_<36></private> rest`
- THEN queda `[REDACTED:private] rest`, sólo con el tipo `private`

#### Escenario: dos veces
- GIVEN la salida de `redact` sobre un texto con un ejemplo de cada formato, una contraseña en una URL, los dos patrones por contexto y una marca cerrada y otra sin cerrar
- THEN `redact` sobre esa salida la deja igual y no encuentra tipos

**Tests:** `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_hyphenated_identifier_after_sk_is_left_alone`, `every_form_of_an_openai_key_is_masked`, `an_anthropic_key_is_never_reported_as_an_openai_one`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `a_literal_password_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_password_key_are_left_alone`, `an_authorization_header_masks_only_its_credential`, `a_known_format_wins_over_its_context`, `a_private_span_is_masked_with_its_marks`, `an_open_private_mark_masks_to_the_end`, `a_mark_quoted_in_backticks_is_text`, `a_private_span_covers_the_secrets_inside_it`, `masking_twice_finds_nothing_new` y `a_kind_found_twice_is_reported_once` (`redact.rs`, mod tests).

### R2. La ingesta enmascara antes de trocear

El daemon MUST enmascarar el contenido de un save antes de escribir su fila en el outbox (`MemoryRuntime::accept`, `crates/clawg-core/src/runtime.rs:220`): la fila vive en `memory.db` y en cada snapshot mientras el save espera o queda `failed`, y guarda sólo el texto ya enmascarado ([`specs/ingest`](../ingest/spec.md), «Aceptar es durable»). El volcado de una versión anterior, que sí guardaba texto crudo, se enmascara al importarlo (`import_spill`, `runtime.rs:447`).

El worker MUST seguir aplicando `redact` al contenido de cada evento, de cualquier fuente, antes de elegir el chunker y de trocear (`ingest_pieces`, `crates/clawg-core/src/memory/worker.rs:621`): sobre el texto de una fila del outbox ya no encuentra nada, porque `redact` es idempotente (R1), y es la pasada que enmascara un evento que ninguna fila respalda (los caminos en proceso de tests y evaluación). El hash de dedup, el embedding, el FTS y `memory.content` MUST calcularse sobre el texto enmascarado. `IngestReport.redacted` (`worker.rs:85`) MUST llevar los tipos que el worker enmascaró del evento en todos los caminos de salida, también cuando el evento falla después.

#### Escenario: el secreto no llega a la base
- GIVEN un save `deploy uses ghp_<36 caracteres>`
- WHEN el worker lo procesa
- THEN `memory.content` es `deploy uses [REDACTED:github_token]`, una consulta FTS por los 36 caracteres no devuelve filas, y el reporte dice `github_token`

#### Escenario: dos saves que sólo difieren en el secreto
- GIVEN un save `the staging key is AKIA<16>` ya guardado
- WHEN se guarda el mismo texto con otra key AWS
- THEN el segundo resultado es `reinforced`, con el id del primero, y la tabla tiene una fila

#### Escenario: el secreto no llega al outbox
- GIVEN un save `the token is ghp_<36 caracteres>`
- WHEN el daemon lo acepta
- THEN la fila del outbox tiene `content = the token is [REDACTED:github_token]` y el evento encolado lleva el mismo texto

**Tests:** `a_saved_secret_is_stored_masked_and_not_searchable`, `saves_that_differ_only_in_a_masked_secret_are_one_memory` y `the_report_names_what_was_masked_even_when_the_save_fails` (`worker.rs`, mod tests); `accept_writes_the_row_before_it_queues_the_event` (`runtime.rs`, mod tests); `an_old_spill_is_imported_masked_and_removed` (`crates/clawg-core/tests/shutdown.rs`).

### R3. El save dice qué se enmascaró

Una respuesta `201` de `POST /v1/memories` MUST incluir `redacted`, cada tipo una vez y en el orden en que se enmascaró, cuando no está vacía, y MUST omitirlo cuando lo está (`SaveBody.redacted`, `crates/clawg-cli/src/daemon/api.rs:491`; `save_response`, `api.rs:821`). La lista une lo que el daemon enmascaró al aceptar el save con lo que el reporte del worker agrega (`masked_kinds`, `api.rs:868`; sólo un evento sin fila puede sumar algo en el worker):

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}], "redacted": ["github_token"]}
```

`DaemonClient` MUST leerlo en `SaveOutcome::Saved.redacted` (`crates/clawg-cli/src/client.rs:434`), vacío si un daemon anterior no lo manda. `save_memory` MUST agregar al texto de éxito `; masked <tipos> before storing it` (`save_outcome_text`, `crates/clawg-cli/src/mcp/server.rs:563`). Las respuestas `202` no lo llevan, aunque el daemon ya haya enmascarado: su cuerpo es `status`, `reason` y `operation_id` (`SaveBody::queued`, `api.rs:507`).

#### Escenario
- GIVEN un daemon con el embedder listo
- WHEN se guarda un texto con una key de Slack
- THEN la respuesta es `201` con `redacted: ["slack_token"]`, y el texto de `save_memory` nombra `slack_token`

**Tests:** `a_save_reports_the_secrets_it_masked` (`crates/clawg-cli/tests/api.rs`), `a_confirmed_save_lists_the_kinds_it_masked` (`api.rs`, mod tests) `a_masked_save_tells_the_agent_what_was_masked` (`server.rs`, mod tests) y `a_confirmed_save_names_its_operation_and_what_was_masked_before_it_was_recorded` (`api.rs`, mod tests).

## Lo ya guardado

### R4. `clawg secrets` reporta sin mostrar el secreto

`clawg secrets [--profile P]` (`Command::Secrets`, `crates/clawg-cli/src/cli.rs:82`; `main.rs:152`) MUST recorrer todas las filas de `memory`, vivas y borradas, e imprimir una línea por fila con secreto: `memory <id> <tipos> <live|deleted>`. La salida MUST NOT contener ningún carácter de un secreto (sólo tipos, ids y estado): `Flagged` (`crates/clawg-core/src/memory/secrets.rs:24`) no lleva el contenido. Sin filas con secreto MUST decir `no stored memory holds a secret of a known format`. MUST terminar con 0 en los dos casos.

La lectura MUST hacerse en sólo lectura, sin el lock y sin migrar (`scan`, `secrets.rs:34`, con `open_for_probe`), de modo que funciona con el daemon andando. Sin `memory.db` MUST fallar diciendo que no hay base, sin crearla. El armado de la salida MUST ser puro (`report`, `crates/clawg-cli/src/secrets.rs:11`).

`scan` clasifica cada fila con `redact` (`secrets.rs:46`), así que una regla nueva del detector alcanza a lo ya guardado: una fila guardada antes de la regla, con un secreto que sólo esa regla reconoce, MUST listarse con el tipo nuevo.

#### Escenario: con el daemon andando
- GIVEN una base con una fila viva con una key AWS, una fila borrada con un JWT y una fila sin secretos, y el lock tomado por otro proceso
- WHEN se escanea
- THEN el reporte tiene las dos primeras filas, con sus tipos y estados, y ningún carácter de la key ni del JWT

#### Escenario: filas guardadas antes de una regla nueva
- GIVEN filas escritas directo en `memory`, sin pasar por la ingesta, con `db password=hunter2` y con una marca privada cerrada
- WHEN se escanea
- THEN el reporte las lista con los tipos `password` y `private`

**Tests:** `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_flags_rows_saved_before_a_new_rule`, `the_scan_reads_while_another_process_holds_the_store` y `the_scan_of_a_missing_store_creates_nothing` (`crates/clawg-core/src/memory/secrets.rs`); `the_secrets_report_lists_rows_and_never_their_content` (`crates/clawg-cli/src/secrets.rs`) y `secrets_takes_only_a_profile` (`cli.rs`).

### R5. `clawg purge` saca una fila para siempre

`clawg purge <ID> --yes [--profile P]` (`Command::Purge`, `cli.rs:89`; `main.rs:160`; `purge`, `crates/clawg-core/src/memory/secrets.rs:82`) MUST, en este orden:

1. sin `--yes`, negarse sin abrir nada, con un mensaje que dice que no hay vuelta atrás y cómo confirmarlo (`confirm`, `crates/clawg-cli/src/secrets.rs:42`);
2. si no existe `memory.db`, fallar diciendo que no hay base;
3. tomar el lock del store (titular `StoreHolder::Maintenance`, `crates/clawg-core/src/memory/store_lock.rs:59`); si lo tiene el daemon, fallar diciendo que se pare con `clawg stop` (`explain`, `crates/clawg-cli/src/secrets.rs:66`);
4. exigir que el plan de esquema sea `UpToDate`; con `Pending`, fallar diciendo que se arranque el daemon una vez para migrar; con `Newer` o `Diverged`, los errores de siempre;
5. si la fila no existe, fallar con un error que nombra el id (`PurgeError::NotFound`, `secrets.rs:58`);
6. en una transacción: borrar su vector y la fila (el trigger del FTS resta sus términos si estaba viva), sus enlaces de `memory_derivation`, las filas del outbox cuyo `content` contiene el texto de la fila (`instr(content, ?) > 0`) y, si estaba viva, liberar las filas que la disputaban, después del `DELETE` para que la fila ya no cuente como viva (`secrets.rs:103-115`). Un save que todavía no terminó de guardarse, o que quedó `failed`, tiene su texto en el outbox, y una fila guardada de un save largo es una de sus piezas: sin este paso el texto seguiría en `memory.db` y en el WAL;
7. compactar el índice FTS (`optimize`, `secrets.rs:120`), para que ni la fila ni sus marcas de borrado queden en ningún segmento;
8. `VACUUM` (`secrets.rs:126`), para que ninguna página libre conserve una copia vieja. `secure_delete` no alcanza: sólo pone en cero las páginas que se liberan de ahí en más, y un merge del FTS anterior puede haber dejado el texto en una página libre;
9. `wal_checkpoint(TRUNCATE)` (`secrets.rs:128`); si no se completa, fallar diciendo que la base está ocupada (`PurgeError::Busy`).

MUST NOT hacer snapshot previo: sería una copia más de lo que se quiere sacar. MUST NOT estar disponible como herramienta MCP ni por la API HTTP.

#### Escenario: fila viva
- GIVEN una base con 300 filas y una viva con `the vault passphrase is zebra-quokka-7741`
- WHEN se purga
- THEN la fila, su vector y su entrada del FTS no existen, una búsqueda FTS por `quokka` no devuelve nada, los bytes `quokka` no están en `memory.db` ni en `memory.db-wal`, y una búsqueda KNN sobre otra fila la sigue encontrando

#### Escenario: fila ya borrada
- GIVEN la misma fila, borrada antes con el delete
- WHEN se purga
- THEN lo mismo

#### Escenario: rechazos
- GIVEN el lock del store tomado por el daemon, un id que no existe y una base en V8 con un binario en V9
- WHEN se intenta purgar
- THEN cada uno falla con su mensaje y la base queda byte a byte igual

#### Escenario: una fila disputada por la purgada
- GIVEN una fila B `disputed` contra la fila A, viva
- WHEN se purga A
- THEN B queda `active`

#### Escenario: el texto está también en el outbox
- GIVEN una fila de `memory` y una fila del outbox con el mismo texto, y otra fila del outbox con otro texto
- WHEN se purga la de `memory`
- THEN el outbox conserva sólo la fila con el otro texto

**Tests:** `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched`, `purge_refuses_a_store_with_pending_migrations_untouched` y `purge_removes_the_text_from_the_outbox` (`crates/clawg-core/src/memory/secrets.rs`); `purge_needs_yes_before_it_opens_anything` y `a_purge_refused_by_the_daemon_says_to_stop_it` (`crates/clawg-cli/src/secrets.rs`) y `purge_takes_an_id_a_yes_and_a_profile` (`cli.rs`).

### R6. La purga nombra los backups que todavía tienen la fila

Después de purgar, MUST revisar cada snapshot de `<data>/backups/` (los archivos con nombre de snapshot) y devolver los que tienen una fila con ese id y el mismo `content`, o una fila del outbox que contiene ese texto, ordenados por nombre (`backups_holding`, `secrets.rs:139`; `Purged.backups`, `secrets.rs:72`). Un snapshot anterior a V14 no tiene la tabla del outbox, y eso MUST NOT contar como «puede tenerla» (`outbox_holds`, `secrets.rs:162`). Uno que no se puede leer MUST listarse también: puede tenerla. MUST NOT modificar ni borrar ningún backup. La salida MUST decir que esos archivos todavía tienen el texto y que borrarlos es decisión del usuario.

#### Escenario
- GIVEN un backup manual tomado con la fila y otro tomado después de borrarla con el delete
- WHEN se purga la fila
- THEN el resultado lista los dos (el delete lógico deja el texto) y los dos archivos quedan byte a byte iguales

- GIVEN un backup tomado antes de guardar la fila
- THEN no se lista

- GIVEN un backup tomado con el texto en una fila del outbox, sin fila de `memory` con ese id
- THEN se lista

- GIVEN un backup tomado antes de que existiera el outbox
- THEN no se lista por no tener la tabla

**Tests:** `purge_names_the_backups_that_still_hold_the_row`, `purge_removes_the_text_from_the_outbox` (el backup que sólo tiene el texto en el outbox) y `a_backup_from_before_the_outbox_is_not_named_for_it` (`crates/clawg-core/src/memory/secrets.rs`) y `the_purge_report_names_backups_to_delete_by_hand` (`crates/clawg-cli/src/secrets.rs`).
