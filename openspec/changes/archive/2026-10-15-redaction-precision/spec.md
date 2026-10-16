# redaction-precision — Especificación

Precisión del enmascarado y nuevas formas de decir qué es secreto. Al archivar, R1–R5 reemplazan la tabla y los escenarios de R1 en [`specs/secrets`](../../specs/secrets/spec.md) y R6 se suma a su R4. R2 y R3 de ese archivo (la ingesta enmascara antes de trocear, el save dice qué se enmascaró) no cambian: las reglas nuevas pasan por la misma función.

## Definiciones

- **Tipo de secreto, placeholder, fila con secreto:** como en [`specs/secrets`](../../specs/secrets/spec.md#definiciones).
- **Marca privada:** el texto desde una apertura `<private>` (sin distinguir mayúsculas) hasta el primer cierre `</private>` que la sigue, o hasta el final del texto si no hay cierre.
- **Valor literal:** el valor de un patrón por contexto que cumple la regla de R4.

## R1. `sk-` exige la forma de una clave

`accepts` (`crates/clawg-core/src/memory/redact.rs:86`) MUST aceptar un match de `openai_key` sólo si la parte capturada no empieza con `ant-` y tiene al menos una letra mayúscula, una minúscula y un dígito ASCII.

#### Escenario: identificador con guiones y dígitos
- GIVEN `the sk-learn-pipeline-for-text-classification-v2-2026-model and scripts/sk-build-2026-release-candidate-01`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

#### Escenario: claves de cada forma
- GIVEN una clave `sk-` de 48 caracteres de base62 y una `sk-proj-`, `sk-svcacct-` y `sk-admin-` de base64url, sintéticas
- WHEN se enmascaran
- THEN cada una queda `[REDACTED:openai_key]`

**Tests:** `a_hyphenated_identifier_after_sk_is_left_alone` y `every_form_of_an_openai_key_is_masked` (`redact.rs`, mod tests).

## R2. La marca privada

`find_secrets` MUST reportar cada marca privada con el tipo `private`, cuyo rango cubre las dos marcas y lo que hay entre ellas, o va hasta el final del texto si no hay cierre. Una apertura precedida inmediatamente por una comilla invertida MUST NOT abrir una marca. Una marca privada MUST competir con los demás matches con la regla de solapamiento de siempre.

#### Escenario: marcas cerradas
- GIVEN `user is <private>Ana, 34</private> and <PRIVATE>vive en Rosario</Private>.`
- WHEN se enmascara
- THEN queda `user is [REDACTED:private] and [REDACTED:private].` con el tipo `private` una vez

#### Escenario: marca sin cerrar
- GIVEN `keep this <private>card 4111 and everything after`
- THEN queda `keep this [REDACTED:private]`

#### Escenario: escribir sobre la marca
- GIVEN ``wrap secrets in `<private>…</private>`; an open `<private>` masks to the end``
- THEN el texto no cambia

#### Escenario: un secreto dentro de la marca
- GIVEN `<private>token ghp_<36></private> rest`
- THEN queda `[REDACTED:private] rest`, sólo con el tipo `private`

**Tests:** `a_private_span_is_masked_with_its_marks`, `an_open_private_mark_masks_to_the_end`, `a_mark_quoted_in_backticks_is_text` y `a_private_span_covers_the_secrets_inside_it` (`redact.rs`, mod tests).

## R3. Formatos nuevos

`find_secrets` MUST reconocer, además de los de hoy:

| Tipo | Forma |
|---|---|
| `gitlab_token` | `glpat-` y 20 o más de `[A-Za-z0-9_-]` |
| `npm_token` | `npm_` y exactamente 36 de `[A-Za-z0-9]`, como palabra entera |
| `google_oauth_secret` | `GOCSPX-` y 28 de `[A-Za-z0-9_-]` |
| `sendgrid_key` | `SG.`, 22 de `[A-Za-z0-9_-]`, `.` y 43 de `[A-Za-z0-9_-]` |

Los prefijos MUST empezar en un borde de palabra.

#### Escenario
- GIVEN un texto con una clave sintética de cada tipo nuevo, rodeada de palabras
- WHEN se enmascara
- THEN cada una quedó como su placeholder y el resto del texto quedó igual

**Tests:** `every_known_secret_format_is_masked_in_place` (con los tipos nuevos en `one_of_each`) (`redact.rs`, mod tests).

## R4. Contraseñas y cabeceras de autorización

`find_secrets` MUST reconocer dos patrones por contexto y MUST enmascarar sólo el valor, conservando la clave y el separador:

- `password`: el valor de una clave que termina en `password` o `passwd` (cualquier prefijo de palabra, sin distinguir mayúsculas, entre comillas o no), seguida de `=`, `:`, `:=` o `==`;
- `auth_header`: el valor después de `Authorization:` o `Authorization=` y `Bearer`, `Basic` o `Token`.

El valor MUST enmascararse sólo si es un valor literal: al menos 6 caracteres; no empieza con `$`, `%` ni `*`; no tiene `( ) [ ] { } < >`; no es un camino de identificadores separados por puntos; y tiene un dígito o un símbolo distinto de `-`, `_` y `.`.

#### Escenario: valores literales
- GIVEN `password=hunter2`, `export DB_PASSWORD="s3cr3t!x"`, `{"password": "p@ssw0rd"}` y `curl -H "Authorization: Bearer a1b2c3d4e5f6g7h8"`
- WHEN se enmascaran
- THEN quedan `password=[REDACTED:password]`, `export DB_PASSWORD="[REDACTED:password]"`, `{"password": "[REDACTED:password]"}` y `curl -H "Authorization: Bearer [REDACTED:auth_header]"`

#### Escenario: no son secretos
- GIVEN `password=<tu-clave>`, `password: string`, `password: Option<String>`, `password = os.getenv("DB")`, `password: cfg.db.password`, `PGPASSWORD=$PGPASS`, `password: change-me`, `password_hash: "$2b$12$abc"`, `PWD=/home/user` y `Authorization: Bearer $TOKEN`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

#### Escenario: un formato conocido gana
- GIVEN `Authorization: Bearer ghp_<36>`
- THEN queda `Authorization: Bearer [REDACTED:github_token]`

**Tests:** `a_literal_password_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_password_key_are_left_alone`, `an_authorization_header_masks_only_its_credential` y `a_known_format_wins_over_its_context` (`redact.rs`, mod tests).

## R5. Sigue siendo idempotente

`redact` aplicada a su propia salida MUST NOT encontrar nada, con todos los tipos, incluidas la marca privada y los patrones por contexto.

**Tests:** `masking_twice_finds_nothing_new`, con un ejemplo de cada tipo (`redact.rs`, mod tests).

## R6. `clawg secrets` ve las reglas nuevas en lo ya guardado

`scan` (`crates/clawg-core/src/memory/secrets.rs:34`) MUST listar una fila guardada antes del cambio que tenga un secreto de un tipo nuevo, con ese tipo. No hace falta código nuevo: clasifica con `redact` (`secrets.rs:46`). El test lo fija.

#### Escenario
- GIVEN una fila escrita directo en `memory` con `db password=hunter2`
- WHEN se escanea
- THEN el reporte la lista con el tipo `password`

**Tests:** `the_scan_flags_rows_saved_before_a_new_rule` (`crates/clawg-core/src/memory/secrets.rs`).
