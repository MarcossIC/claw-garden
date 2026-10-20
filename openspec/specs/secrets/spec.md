# secrets — Especificación

Fuente de verdad del comportamiento actual con secretos: el detector (formatos conocidos, patrones por contexto y la marca privada), el enmascarado al ingerir, en el `reason` de un veredicto y en el `last_error` del outbox, el campo `redacted` del save, `clawg secrets` (con `--mask-reasons`) y `clawg purge`. El resto del contrato de `POST /v1/memories` y de `save_memory` está en [`specs/ingest`](../ingest/spec.md). Qué hace una purga con los enlaces de procedencia de la fila (borra aquellos en los que es conclusión y deja los que la nombran como fuente) está en [`specs/synthesis`](../synthesis/spec.md), R6.

**Origen:**
- [secret-hygiene](../../changes/archive/2026-10-14-secret-hygiene/proposal.md), archivado el 2026-10-14. Los ids de requisito son los de esa spec.
- [ingest-outbox](../../changes/archive/2026-10-15-ingest-outbox/proposal.md), archivado el 2026-10-15 (R2 y R3: el enmascarado ocurre antes de escribir la fila del outbox; R5 y R6: `purge` y los backups cubren el outbox).
- [redaction-precision](../../changes/archive/2026-10-15-redaction-precision/proposal.md), archivado el 2026-10-15 (`sk-` con la forma de una clave, la marca privada, los patrones por contexto y cuatro formatos nuevos en R1; lo guardado antes de una regla nueva en R4).
- [conclusion-upkeep](../../changes/archive/2026-10-19-conclusion-upkeep/proposal.md), archivado el 2026-10-19 (R5: la purga deja los enlaces que nombran la fila como fuente).
- [secret-coverage](../../changes/archive/2026-10-19-secret-coverage/proposal.md), archivado el 2026-10-19 (R1: `credential`, `huggingface_token`, `slack_webhook` y las formas nuevas de Slack, Stripe y PGP; R4: los `reason` de los veredictos; R5: el outbox del propio save, la cabecera de un save largo y la revival; R6: los `workspace.md`; R7, R8 y R9 nuevos).

## Definiciones

- **Tipo de secreto:** uno de los de R1 (un formato conocido, un patrón por contexto o `private`), con su nombre en `snake_case` (`SecretKind` y `as_str`, `crates/clawg-core/src/memory/redact.rs:17` y `redact.rs:41`).
- **Placeholder:** `[REDACTED:<tipo>]`, el texto que reemplaza al secreto.
- **Marca privada:** el texto desde una apertura `<private>` (sin distinguir mayúsculas) hasta el primer cierre `</private>` que la sigue, o hasta el final del texto si no hay cierre.
- **Valor literal:** el valor de un patrón por contexto que cumple la regla de R1.
- **Fila con secreto:** una fila de `memory`, viva o borrada, cuyo `content` tiene al menos un match de R1.
- **Par con secreto:** una fila de `memory_conflict` cuyo `reason` tiene al menos un match de R1.
- **Purgar:** sacar una fila de `memory` de la base para siempre, con su vector y sus términos del FTS, de modo que su texto no quede en `memory.db` ni en `memory.db-wal`.

## Enmascarado

### R1. Detector puro de secretos

`find_secrets(&str)` (`redact.rs:204`) MUST devolver los rangos y tipos de cada secreto, sin I/O, y `redact(&str)` (`redact.rs:235`) MUST devolver el texto con cada rango reemplazado por su placeholder, más los tipos encontrados (sin repetir, en orden de aparición). Los patrones se compilan una vez (`PATTERNS`, `redact.rs:86`); `accepts` (`redact.rs:153`) descarta los matches que el regex solo no puede excluir. El detector MUST NOT usar entropía: un hash de commit, un UUID o un checksum también parecen aleatorios, y son memoria que vale la pena guardar.

#### Formatos conocidos

| Tipo | Forma |
|---|---|
| `aws_access_key` | `AKIA` o `ASIA` y 16 de `[A-Z0-9]`, como palabra entera |
| `github_token` | `ghp_`, `gho_`, `ghu_`, `ghs_` o `ghr_` y 36 de `[A-Za-z0-9]`; o `github_pat_` y 22 o más de `[A-Za-z0-9_]` |
| `anthropic_key` | `sk-ant-` y 20 o más de `[A-Za-z0-9_-]` |
| `openai_key` | `sk-`, opcionalmente `proj-`, `svcacct-` o `admin-`, y 20 o más de `[A-Za-z0-9_-]` con al menos una mayúscula, una minúscula y un dígito ASCII; nunca `sk-ant-` |
| `slack_token` | `xoxa-`, `xoxb-`, `xoxp-`, `xoxo-`, `xoxs-`, `xoxr-` o `xapp-` y 10 o más de `[A-Za-z0-9-]` |
| `google_api_key` | `AIza` y 35 de `[A-Za-z0-9_-]` |
| `stripe_key` | `sk_live_` o `rk_live_` y 16 o más de `[A-Za-z0-9]`; o `whsec_` y 24 o más de `[A-Za-z0-9]` |
| `gitlab_token` | `glpat-` y 20 o más de `[A-Za-z0-9_-]` |
| `npm_token` | `npm_` y exactamente 36 de `[A-Za-z0-9]`, como palabra entera |
| `google_oauth_secret` | `GOCSPX-` y 28 de `[A-Za-z0-9_-]` |
| `sendgrid_key` | `SG.`, 22 de `[A-Za-z0-9_-]`, `.` y 43 de `[A-Za-z0-9_-]` |
| `huggingface_token` | `hf_` y exactamente 34 de `[A-Za-z0-9]`, como palabra entera, con al menos una mayúscula, una minúscula y un dígito ASCII |
| `slack_webhook` | `https://hooks.slack.com/services/`, `T` y 6 o más de `[A-Z0-9]`, `/B` y 6 o más, `/` y 20 o más de `[A-Za-z0-9]`; la URL entera |
| `private_key` | de `-----BEGIN … PRIVATE KEY-----` (o `PRIVATE KEY BLOCK-----`, la cabecera PGP) al `-----END …` que lo cierra, o al final del texto si no se cierra |
| `jwt` | `eyJ` y 10 o más de `[A-Za-z0-9_-]`, `.`, `eyJ` y 10 o más, `.` y la firma (`[A-Za-z0-9_-]*`) |
| `url_password` | sólo la contraseña de `esquema://usuario:contraseña@`; se conservan el esquema, el usuario y el host. No se enmascara si empieza con `$`, `<`, `{`, `[` o `*` (una variable, un ejemplo o un placeholder) |

Los prefijos MUST empezar en un borde de palabra. `sk-` solo es un prefijo común en identificadores, rutas y nombres de paquete, que casi nunca mezclan mayúsculas y minúsculas; una clave real es base62 o base64url aleatoria. Un identificador que sí las mezcla y tiene un dígito después de `sk-` se sigue enmascarando. Con `hf_` pasa lo mismo: 34 letras pueden ser un nombre de clase (`AutoModelForSequenceClassification`), que no tiene dígitos.

#### Patrones por contexto

Tres patrones MUST enmascarar sólo el valor y conservar la clave, el separador y las comillas (`redact.rs:113`, `redact.rs:117` y `redact.rs:121`):

- `password`: el valor de una clave que termina en `password` o `passwd` (con cualquier prefijo de palabra, como `DB_PASSWORD` o `PGPASSWORD`; sin distinguir mayúsculas; entre comillas o no), seguida de `=`, `:`, `:=` o `==`. Un valor nunca empieza con `:`, así que `password::rules::check` es un camino a través de un módulo y no una clave con su valor. `password_hash` o `passwordFile` no terminan en `password` y no cuentan.
- `auth_header`: el valor después de `Authorization:` o `Authorization=` (también con la clave entre comillas, como en JSON) y `Bearer`, `Basic` o `Token`.
- `credential`: el valor de una clave que termina en `secret`, `token`, `api_key`, `apikey`, `api-key`, `access_key`, `secret_key`, `account_key` (o `accountkey`) o `auth_key`, sin distinguir mayúsculas y con cualquier prefijo de letras, dígitos, `_` o `-` (`OPENROUTER_API_KEY`, `AWS_SECRET_ACCESS_KEY`, `client_secret`, `X-Api-Key`, `AccountKey`, `githubToken`), entre comillas o no, seguida de `=`, `:`, `:=` o `==`. Una clave que sigue (`tokenizer`, `max_tokens`, `secret_name`) no cuenta.

El valor MUST enmascararse sólo si es un valor literal (`is_literal_secret`, `redact.rs:187`): al menos 6 caracteres; no empieza con `$`, `%` ni `*`; no tiene `( ) [ ] { } < >`; no es un camino de identificadores separados por puntos; y tiene un dígito o un símbolo distinto de `-`, `_` y `.`. Una contraseña de sólo letras y guiones queda sin enmascarar: después de esas claves es mucho más frecuente un tipo, una variable o un valor de ejemplo, y enmascararlo destruye la memoria. Para `credential`, además, el valor MUST tener una letra y MUST NOT tener `::`: después de `max_token` o `token` un número es un límite y `Token::Ident` es un camino.

#### La marca privada

`find_secrets` MUST reportar cada marca privada con el tipo `private` (`private_spans`, `redact.rs:136`); el rango cubre las dos marcas y lo que hay entre ellas, o va hasta el final del texto si no hay cierre: quien abrió la marca dijo que lo que sigue es secreto. Una apertura precedida inmediatamente por una comilla invertida MUST NOT abrir una marca, para poder escribir sobre la marca sin perder texto. El índice de docs enmascara cada archivo entero con la misma función antes de trocearlo (`crates/clawg-core/src/memory/project_docs/mod.rs:382`), así que un archivo que escribe la apertura sin comilla invertida delante pierde, en el índice, el tramo hasta el próximo cierre del archivo o hasta su final; se acepta porque el índice es derivado y el archivo no cambia.

#### Solapamientos e idempotencia

Dos matches que se superponen MUST resolverse a favor del que empieza antes y, si empiezan juntos, del más largo; en un empate exacto gana el primero en `PATTERNS`, donde los formatos conocidos van antes que los patrones por contexto. Así un formato conocido gana sobre su contexto, y una marca privada, que empieza antes que cualquier secreto de adentro, deja un solo placeholder. `redact` MUST ser idempotente: aplicada a su propia salida no encuentra nada, con todos los tipos.

#### Escenario: un ejemplo de cada formato
- GIVEN un texto con un ejemplo de cada formato conocido, rodeado de palabras
- WHEN se enmascara
- THEN cada ejemplo quedó como su placeholder, el resto del texto quedó igual, y los tipos son los dieciséis, en orden

#### Escenario: no son secretos
- GIVEN un hash de commit de 40 caracteres, un UUID, un SHA-256 en hex, `MAX_LEXICAL_CANDIDATES_PER_QUERY_WINDOW`, `scikit-learn`, `sk-learn-is-not-a-key-at-all`, `sk-learn-pipeline-for-text-classification-v2-2026-model`, `scripts/sk-build-2026-release-candidate-01`, `AKIA` suelto, `ghp_short`, `postgres://app:${DB_PASSWORD}@db:5432/app`, `https://user:<password>@example.com`, `hf_` seguido de `AutoModelForSequenceClassification`, `hf_hub_download` y `https://hooks.slack.com/services/your-webhook-here`
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

#### Escenario: claves de configuración
- GIVEN `AWS_SECRET_ACCESS_KEY=<40 base64>`, `OPENROUTER_API_KEY=sk-or-v1-<hex>`, `{"client_secret": "<34>"}`, `X-Api-Key: <32>`, `AccountKey=<base64>==;`, `GITHUB_TOKEN: a1b2c3d4e5f6`, `secret = "zebra-quokka-7741"` y `githubToken := "q9W-e8R7t"`
- WHEN se enmascaran
- THEN cada valor queda `[REDACTED:credential]` y la clave, el separador y las comillas quedan

#### Escenario: código y prosa después de una clave de credencial
- GIVEN `token = next_token()`, `secret: string`, `api_key: Option<String>`, `let token = parser.token;`, `max_token=100000`, `max_tokens: 4096`, `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`, `api_key=$OPENAI_API_KEY`, `secret_name: my-app`, `tokenizer: bert-base-2`, `api_key: YOUR_API_KEY`, `access_key: <your-access-key>`, `token: Token::Ident`, `the secret: keep it out of the repo`, `csrf_token: true` e `id_token: [REDACTED:jwt]`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

#### Escenario: un formato conocido gana
- GIVEN `Authorization: Bearer ghp_<36>`, `OPENAI_PASSWORD=sk-proj-<192>`, `HF_TOKEN=hf_<34>` y `aws_access_key: AKIA<16>`
- THEN quedan `Authorization: Bearer [REDACTED:github_token]`, `OPENAI_PASSWORD=[REDACTED:openai_key]`, `HF_TOKEN=[REDACTED:huggingface_token]` y `aws_access_key: [REDACTED:aws_access_key]`

#### Escenario: clave privada PGP
- GIVEN un bloque de `-----BEGIN PGP PRIVATE KEY BLOCK-----` a `-----END PGP PRIVATE KEY BLOCK-----` entre dos líneas
- THEN queda `[REDACTED:private_key]` y las dos líneas quedan

#### Escenario: Slack `xapp-` y el secreto de webhook de Stripe
- GIVEN `xapp-1-<…>` y `whsec_<32>`
- THEN quedan `[REDACTED:slack_token]` y `[REDACTED:stripe_key]`

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
- GIVEN la salida de `redact` sobre un texto con un ejemplo de cada formato, una contraseña en una URL, los tres patrones por contexto y una marca cerrada y otra sin cerrar
- THEN `redact` sobre esa salida la deja igual y no encuentra tipos

**Tests:** `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_hyphenated_identifier_after_sk_is_left_alone`, `every_form_of_an_openai_key_is_masked`, `an_anthropic_key_is_never_reported_as_an_openai_one`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `a_literal_password_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_password_key_are_left_alone`, `an_authorization_header_masks_only_its_credential`, `a_literal_credential_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_credential_key_are_left_alone`, `a_known_format_wins_over_its_context`, `a_pgp_private_key_block_is_masked`, `the_slack_app_and_stripe_webhook_forms_are_masked`, `a_private_span_is_masked_with_its_marks`, `an_open_private_mark_masks_to_the_end`, `a_mark_quoted_in_backticks_is_text`, `a_private_span_covers_the_secrets_inside_it`, `masking_twice_finds_nothing_new` y `a_kind_found_twice_is_reported_once` (`redact.rs`, mod tests).

### R2. La ingesta enmascara antes de trocear

El daemon MUST enmascarar el contenido de un save antes de escribir su fila en el outbox (`MemoryRuntime::accept`, `crates/clawg-core/src/runtime.rs:238`): la fila vive en `memory.db` y en cada snapshot mientras el save espera o queda `failed`, y guarda sólo el texto ya enmascarado ([`specs/ingest`](../ingest/spec.md), «Aceptar es durable»). El volcado de una versión anterior, que sí guardaba texto crudo, se enmascara al importarlo (`import_spill`, `runtime.rs:497`).

El worker MUST seguir aplicando `redact` al contenido de cada evento, de cualquier fuente, antes de elegir el chunker y de trocear (`ingest_pieces`, `crates/clawg-core/src/memory/worker.rs:730`): sobre el texto de una fila del outbox ya no encuentra nada, porque `redact` es idempotente (R1), y es la pasada que enmascara un evento que ninguna fila respalda (los caminos en proceso de tests y evaluación). El hash de dedup, el embedding, el FTS y `memory.content` MUST calcularse sobre el texto enmascarado. `IngestReport.redacted` (`worker.rs:85`) MUST llevar los tipos que el worker enmascaró del evento en todos los caminos de salida, también cuando el evento falla después.

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

Una respuesta `201` de `POST /v1/memories` MUST incluir `redacted`, cada tipo una vez y en el orden en que se enmascaró, cuando no está vacía, y MUST omitirlo cuando lo está (`SaveBody.redacted`, `crates/clawg-cli/src/daemon/api.rs:519`; `save_response`, `api.rs:994`). La lista une lo que el daemon enmascaró al aceptar el save con lo que el reporte del worker agrega (`masked_kinds`, `api.rs:1062`; sólo un evento sin fila puede sumar algo en el worker):

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}], "redacted": ["github_token"]}
```

`DaemonClient` MUST leerlo en `SaveOutcome::Saved.redacted` (`crates/clawg-cli/src/client.rs:529`), vacío si un daemon anterior no lo manda. `save_memory` MUST agregar al texto de éxito `; masked <tipos> before storing it` (`save_outcome_text`, `crates/clawg-cli/src/mcp/server.rs:564`). Las respuestas `202` no lo llevan, aunque el daemon ya haya enmascarado: su cuerpo es `status`, `reason` y `operation_id` (`SaveBody::queued`, `api.rs:538`).

#### Escenario
- GIVEN un daemon con el embedder listo
- WHEN se guarda un texto con una key de Slack
- THEN la respuesta es `201` con `redacted: ["slack_token"]`, y el texto de `save_memory` nombra `slack_token`

**Tests:** `a_save_reports_the_secrets_it_masked` (`crates/clawg-cli/tests/api.rs`), `a_confirmed_save_lists_the_kinds_it_masked` (`api.rs`, mod tests) `a_masked_save_tells_the_agent_what_was_masked` (`server.rs`, mod tests) y `a_confirmed_save_names_its_operation_and_what_was_masked_before_it_was_recorded` (`api.rs`, mod tests).

## Lo ya guardado

### R4. `clawg secrets` reporta sin mostrar el secreto

`clawg secrets [--profile P]` (`Command::Secrets`, `crates/clawg-cli/src/cli.rs:83`; `main.rs:201`) MUST recorrer todas las filas de `memory`, vivas y borradas, y todos los `reason` de `memory_conflict`, e imprimir una línea por fila con secreto, `memory <id> <tipos> <live|deleted>`, y después una por par con secreto, `pair <new_id> <old_id> <tipos>`. La salida MUST NOT contener ningún carácter de un secreto (sólo tipos, ids y estado): `Flagged` y `FlaggedPair` (`crates/clawg-core/src/memory/secrets.rs:26` y `secrets.rs:35`) no llevan el texto. Después de las filas MUST decir cómo purgarlas, y después de los pares, cómo enmascararlos (`clawg stop`, luego `clawg secrets --mask-reasons --yes`, R8). Sin filas ni pares con secreto MUST decir `no stored memory holds a secret of a known format`. MUST terminar con 0 en todos los casos. Una base anterior a V15 no tiene la tabla de pares, y eso MUST NOT ser un error.

La lectura MUST hacerse en sólo lectura, sin el lock y sin migrar (`scan`, `secrets.rs:53`, con `open_for_probe`), de modo que funciona con el daemon andando. Sin `memory.db` MUST fallar diciendo que no hay base, sin crearla. El armado de la salida MUST ser puro (`report`, `crates/clawg-cli/src/secrets.rs:13`).

`scan` clasifica cada fila y cada `reason` con `redact`, así que una regla nueva del detector alcanza a lo ya guardado: una fila guardada antes de la regla, con un secreto que sólo esa regla reconoce, MUST listarse con el tipo nuevo. Por la misma razón, `clawg undelete` se niega a restaurar una fila borrada que una regla nueva reconoce (`RestoreError::HoldsSecret`, `crates/clawg-core/src/memory/admin.rs:397`); no se pierde nada: la fila sigue borrada y se lista acá.

#### Escenario: con el daemon andando
- GIVEN una base con una fila viva con una key AWS, una fila borrada con un JWT y una fila sin secretos, y el lock tomado por otro proceso
- WHEN se escanea
- THEN el reporte tiene las dos primeras filas, con sus tipos y estados, y ningún carácter de la key ni del JWT

#### Escenario: filas guardadas antes de una regla nueva
- GIVEN filas escritas directo en `memory`, sin pasar por la ingesta, con `db password=hunter2` y con una marca privada cerrada
- WHEN se escanea
- THEN el reporte las lista con los tipos `password` y `private`

#### Escenario: un veredicto con un secreto
- GIVEN un par juzgado con `reason` `rotated ghp_<36>` escrito directo en la tabla, otro con un `reason` sin secreto y otro sin `reason`
- WHEN se escanea
- THEN el reporte lista sólo el primero, `pair <new> <old>  github_token`, sin ningún carácter del token

**Tests:** `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_flags_rows_saved_before_a_new_rule`, `the_scan_finds_reasons_that_hold_a_secret`, `the_scan_of_a_store_from_before_the_pairs_lists_rows_only`, `the_scan_reads_while_another_process_holds_the_store` y `the_scan_of_a_missing_store_creates_nothing` (`crates/clawg-core/src/memory/secrets.rs`); `the_secrets_report_lists_rows_and_never_their_content` y `the_secrets_report_lists_pairs_and_how_to_mask_them` (`crates/clawg-cli/src/secrets.rs`) y `secrets_takes_a_profile_and_masks_reasons_on_request` (`cli.rs`).

### R5. `clawg purge` saca una fila para siempre

`clawg purge <ID> --yes [--profile P]` (`Command::Purge`, `cli.rs:97`; `main.rs:209`; `purge`, `crates/clawg-core/src/memory/secrets.rs:181`) MUST, en este orden:

1. sin `--yes`, negarse sin abrir nada, con un mensaje que dice que no hay vuelta atrás y cómo confirmarlo (`confirm`, `crates/clawg-cli/src/secrets.rs:59`);
2. si no existe `memory.db`, fallar diciendo que no hay base;
3. tomar el lock del store (titular `StoreHolder::Maintenance`, `crates/clawg-core/src/memory/store_lock.rs:59`; `open_to_rewrite`, `secrets.rs:309`); si lo tiene el daemon, fallar diciendo que se pare con `clawg stop` (`explain`, `crates/clawg-cli/src/secrets.rs:149`);
4. exigir que el plan de esquema sea `UpToDate`; con `Pending`, fallar diciendo que se arranque el daemon una vez para migrar; con `Newer` o `Diverged`, los errores de siempre;
5. si la fila no existe, fallar con un error que nombra el id (`PurgeError::NotFound`, `secrets.rs:105`);
6. en una transacción (`secrets.rs:196-249`):
   1. si la fila está viva, marcarla borrada y aplicar la misma revival que un `delete` (`revive_after_delete_of_piece`, `secrets.rs:204`; [`specs/restore`](../restore/spec.md)): la revisión más nueva que reemplazó como versión vigente de su tema vuelve a estar viva y las anteriores la apuntan; lo que reemplazó un veredicto queda reemplazado. Sin este paso la historia del tema queda `superseded` por un id que ya no existe, escondida para siempre;
   2. si la fila es la posición 1 de un save largo, vaciar el `context` de todos los vínculos de `memory_piece` de ese save (`secrets.rs:221`): cada pedazo siguiente se embebió detrás de la cabecera del save y el vínculo la guardaba como texto. El vínculo de la fila queda: guarda un id, que `AUTOINCREMENT` no reusa, y marca el pedazo como faltante;
   3. borrar su vector y la fila (el trigger del FTS resta sus términos), sus enlaces de `memory_derivation` en los que es conclusión (los que la nombran como fuente quedan, y la conclusión que sale de ella queda a revisar con esa fuente `purged`: [`specs/synthesis`](../synthesis/spec.md), R6) y sus pares de `memory_conflict`;
   4. borrar las filas del outbox del save de la fila (`OUTBOX_MATCH`, `secrets.rs:165`): las de su agente (una fila sin agente no filtra por agente) cuyo `content` es igual al de la fila si la fila no tiene vínculo en `memory_piece`, porque entonces su save fue sólo ella, o lo contiene si es un pedazo de un save largo. Un save que todavía no terminó de guardarse, o que quedó `failed`, tiene su texto en el outbox; el de otro agente que dice lo mismo es de ese agente, y fue aceptado;
   5. si estaba viva, liberar las filas que la disputaban, después del `DELETE` para que la fila ya no cuente como viva;
7. compactar el índice FTS (`optimize`, `secrets.rs:253`), para que ni la fila ni sus marcas de borrado queden en ningún segmento;
8. `VACUUM` (`leave_no_old_copy`, `secrets.rs:332`), para que ninguna página libre conserve una copia vieja. `secure_delete` no alcanza: sólo pone en cero las páginas que se liberan de ahí en más, y un merge del FTS anterior puede haber dejado el texto en una página libre;
9. `wal_checkpoint(TRUNCATE)` (`secrets.rs:334`); si no se completa, fallar diciendo que la base está ocupada (`PurgeError::Busy`).

`Purged` (`secrets.rs:116`) MUST nombrar las filas revividas, las que un veredicto deja reemplazadas, los otros pedazos del save (comparten con la purgada los tokens del solape, así que cada uno tiene un poco del texto), las filas del outbox borradas (`operation_id`, agente y estado), los backups de R6 y los `workspace.md` de R6. La salida de `clawg purge` (`purge_report`, `crates/clawg-cli/src/secrets.rs:110`) MUST decir cada grupo en su línea, sólo cuando no está vacío.

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

#### Escenario: la fuente de una conclusión
- GIVEN una conclusión C sacada de la fila A
- WHEN se purga A
- THEN el enlace `(C, A)` de `memory_derivation` sigue, y los de A como conclusión no

#### Escenario: el texto está también en el outbox
- GIVEN una fila de `memory` y una fila del outbox con el mismo texto, y otra fila del outbox con otro texto
- WHEN se purga la de `memory`
- THEN el outbox conserva sólo la fila con el otro texto

#### Escenario: otro agente con el mismo texto
- GIVEN una fila de `agent-s` con `use pnpm`, y en el outbox una fila de `agent-o` con `use pnpm`, una de `agent-s` con `use pnpm` y otra de `agent-s` con `use pnpm in the monorepo`
- WHEN se purga la fila
- THEN el outbox conserva la de `agent-o` y la más larga de `agent-s`, y `Purged` nombra la borrada

#### Escenario: el primer pedazo de un save largo
- GIVEN un save de tres pedazos cuyos vínculos 2 y 3 llevan en `context` el texto del primero, y en el outbox una fila del mismo agente con el save entero
- WHEN se purga el pedazo 1
- THEN los tres vínculos quedan con `context` vacío, la fila del outbox se borra, `Purged` nombra los pedazos 2 y 3, y los bytes del texto no están en disco

#### Escenario: la versión vigente de un tema
- GIVEN tres revisiones de `deploy/target`, la vigente con un secreto, y una fila que un veredicto hizo reemplazar por la vigente
- WHEN se purga la vigente
- THEN la revisión del medio vuelve a estar `active`, la primera la sigue apuntando, la del veredicto sigue `superseded`, y `Purged` nombra la revivida y la escondida

**Tests:** `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched`, `purge_refuses_a_store_with_pending_migrations_untouched`, `purge_removes_the_text_from_the_outbox`, `purge_keeps_other_agents_and_longer_saves_in_the_outbox`, `purging_the_first_piece_empties_the_head_of_its_save`, `purging_the_current_revision_brings_the_previous_one_back` y `purging_keeps_the_links_that_name_the_row_as_a_source` (`crates/clawg-core/src/memory/secrets.rs`); `purge_needs_yes_before_it_opens_anything`, `a_purge_refused_by_the_daemon_says_to_stop_it` y `the_purge_report_names_what_came_back_and_what_went` (`crates/clawg-cli/src/secrets.rs`) y `purge_takes_an_id_a_yes_and_a_profile` (`cli.rs`).

### R6. La purga nombra los backups y los `workspace.md` que todavía tienen el texto

Después de purgar, MUST revisar cada snapshot de `<data>/backups/` (los archivos con nombre de snapshot) y devolver los que tienen una fila con ese id y el mismo `content`, o una fila del outbox que R5 borraría, ordenados por nombre (`backups_holding`, `secrets.rs:377`). Un snapshot anterior a V14 no tiene la tabla del outbox, y eso MUST NOT contar como «puede tenerla» (`outbox_holds`, `secrets.rs:398`). Uno que no se puede leer MUST listarse también: puede tenerla. MUST NOT modificar ni borrar ningún backup. La salida MUST decir que esos archivos todavía tienen el texto y que borrarlos es decisión del usuario.

También MUST leer cada `workspaces/<agente>/<proyecto>/workspace.md` del directorio de datos y devolver, ordenados por ruta, los que tienen el texto de la fila, cada uno con si está en la sección AUTO, fuera de ella o en los dos lugares (`workspaces_holding` y `sections_holding`, `crates/clawg-core/src/memory/workspace.rs:515` y `workspace.rs:499`; la decisión es pura). La sección AUTO copia el `content` entero de cada fila que elige (`render_rows`), y lo de fuera es del usuario. MUST NOT escribir ningún archivo: la salida MUST decir que la sección AUTO se reescribe en el próximo `recall_context` de ese agente en ese proyecto y que lo de fuera se edita a mano.

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

- GIVEN un `workspace.md` con el texto en AUTO, otro con el texto en su sección USER y otro sin él
- WHEN se purga la fila
- THEN el resultado nombra los dos primeros, cada uno con su sección, y los tres archivos quedan byte a byte iguales

**Tests:** `purge_names_the_backups_that_still_hold_the_row`, `purge_removes_the_text_from_the_outbox` (el backup que sólo tiene el texto en el outbox), `a_backup_from_before_the_outbox_is_not_named_for_it` y `purge_names_the_workspaces_that_still_hold_the_text` (`crates/clawg-core/src/memory/secrets.rs`); `the_sections_holding_a_text_are_told_apart` (`crates/clawg-core/src/memory/workspace.rs`); `the_purge_report_names_backups_to_delete_by_hand` y `the_purge_report_names_what_came_back_and_what_went` (`crates/clawg-cli/src/secrets.rs`).

## Texto libre fuera de `memory`

### R7. El `reason` de un veredicto se guarda enmascarado

`judge_pair` (`crates/clawg-core/src/memory/dispute.rs:317`) MUST pasar el `reason`, ya recortado, por `redact` antes de guardarlo en `memory_conflict.reason` (`dispute.rs:329`): el par vive en `memory.db` y en cada snapshot, y lo llevaría un export. Un `reason` vacío después de recortarlo sigue siendo `None`.

#### Escenario
- GIVEN un par pendiente
- WHEN se juzga `scoped` con `reason` `the staging key AKIA<16> is only for ci`
- THEN `memory_conflict.reason` es `the staging key [REDACTED:aws_access_key] is only for ci`

**Tests:** `a_verdict_reason_is_stored_masked` (`dispute.rs`, mod tests).

### R8. `clawg secrets --mask-reasons --yes`

`clawg secrets --mask-reasons --yes [--profile P]` (`main.rs:192`; `mask_reasons`, `crates/clawg-core/src/memory/secrets.rs:285`) limpia los `reason` guardados antes de R7. MUST, en este orden: sin `--yes`, negarse sin abrir nada (`confirm_mask_reasons`, `crates/clawg-cli/src/secrets.rs:70`; `--yes` sin `--mask-reasons` es un error de uso); sin `memory.db`, fallar; tomar el lock (`Maintenance`) y, si lo tiene el daemon, fallar diciendo que se pare; exigir el esquema `UpToDate` (una base nunca migrada no tiene nada que limpiar); en una transacción, reemplazar cada `reason` con secreto por `redact(reason)`, sin tocar ningún otro; `VACUUM` y `wal_checkpoint(TRUNCATE)`, como R5. MUST NOT hacer snapshot. MUST devolver los pares que cambió, con sus tipos, y los snapshots que todavía tienen alguno de esos `reason` como estaba (`MaskedReasons`, `secrets.rs:271`), y la salida (`mask_report`, `crates/clawg-cli/src/secrets.rs:81`) MUST nombrarlos sin el texto.

#### Escenario
- GIVEN un par con `reason` `rotated ghp_<36>`, otro con un `reason` sin secreto, otro sin `reason` y un backup tomado antes
- WHEN se corre con `--yes`
- THEN el primero queda `rotated [REDACTED:github_token]`, los otros quedan iguales, los bytes del token no están en `memory.db` ni en el WAL, el backup se nombra y una segunda corrida no encuentra nada

#### Escenario: con el daemon andando
- GIVEN el lock tomado por el daemon
- WHEN se corre
- THEN falla con `StoreInUse` y la base queda byte a byte igual

**Tests:** `masking_reasons_rewrites_only_the_ones_with_a_secret` y `masking_reasons_refuses_while_the_daemon_holds_the_store` (`crates/clawg-core/src/memory/secrets.rs`); `mask_reasons_needs_yes` y `the_mask_report_names_pairs_and_backups` (`crates/clawg-cli/src/secrets.rs`); `secrets_takes_a_profile_and_masks_reasons_on_request` (`cli.rs`).

### R9. `last_error` se guarda enmascarado

Todo `last_error` del outbox que lleva el texto de un error MUST pasar por `redact` antes de escribirse: el de `outbox_fail` (`crates/clawg-core/src/memory/outbox.rs:346`) y el de un evento que no se lee (`outbox_pending`, `outbox.rs:486`), cuyo error de serde repite valores del evento. La columna sale en `/health` y en `memory_status`, y nada impide que una librería cite su entrada. El abandono tras tres arranques es un mensaje fijo.

#### Escenario
- GIVEN una fila del outbox
- WHEN falla con `embed failed: token ghp_<36>`
- THEN `last_error` es `embed failed: token [REDACTED:github_token]`

- GIVEN una fila del outbox cuyo evento lleva `"importance": "ghp_<36>"`
- WHEN se cargan las pendientes
- THEN queda `failed` y su `last_error` no tiene el token

**Tests:** `a_failure_is_stored_masked` (`outbox.rs`, mod tests).
