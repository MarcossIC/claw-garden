# secret-hygiene — Especificación

**Dominio destino:** `specs/secrets/spec.md`, nuevo. Al archivar, R1 a R6 pasan ahí con las rutas y líneas del código en ese momento. El contrato de `POST /v1/memories` de [`specs/ingest`](../../specs/ingest/spec.md) gana un campo (R3); al archivar se agrega una línea en su sección «Un save confirmado devuelve los ids» que apunta a R3.

**Alcance:** el enmascarado al ingerir, el campo `redacted` del save, `clawg secrets` y `clawg purge`. No cambia el esquema ni toca filas existentes.

## Definiciones

- **Tipo de secreto:** uno de los de la tabla de R1, con su nombre en `snake_case`.
- **Placeholder:** `[REDACTED:<tipo>]`, el texto que reemplaza al secreto.
- **Fila con secreto:** una fila de `memory`, viva o borrada, cuyo `content` tiene al menos un match de R1.
- **Purgar:** sacar una fila de `memory` de la base para siempre, con su vector y sus términos del FTS, de modo que su texto no quede en `memory.db` ni en `memory.db-wal`.

## Enmascarado

### R1. Detector puro de secretos de formato conocido

`find_secrets(&str)` MUST devolver los rangos y tipos de cada secreto, sin I/O, y `redact(&str)` MUST devolver el texto con cada rango reemplazado por su placeholder, más los tipos encontrados (sin repetir, en orden de aparición).

| Tipo | Forma |
|---|---|
| `aws_access_key` | `AKIA` o `ASIA` y 16 de `[A-Z0-9]`, como palabra entera |
| `github_token` | `ghp_`, `gho_`, `ghu_`, `ghs_` o `ghr_` y 36 de `[A-Za-z0-9]`; o `github_pat_` y 22 o más de `[A-Za-z0-9_]` |
| `anthropic_key` | `sk-ant-` y 20 o más de `[A-Za-z0-9_-]` |
| `openai_key` | `sk-`, opcionalmente `proj-`, `svcacct-` o `admin-`, y 20 o más de `[A-Za-z0-9_-]` que tengan al menos una letra y un dígito; nunca `sk-ant-` |
| `slack_token` | `xoxa-`, `xoxb-`, `xoxp-`, `xoxo-`, `xoxs-` o `xoxr-` y 10 o más de `[A-Za-z0-9-]` |
| `google_api_key` | `AIza` y 35 de `[A-Za-z0-9_-]` |
| `stripe_key` | `sk_live_` o `rk_live_` y 16 o más de `[A-Za-z0-9]` |
| `private_key` | de `-----BEGIN … PRIVATE KEY-----` al `-----END … PRIVATE KEY-----` que lo cierra, o al final del texto si no se cierra |
| `jwt` | `eyJ` y 10 o más de `[A-Za-z0-9_-]`, `.`, `eyJ` y 10 o más, `.` y la firma (`[A-Za-z0-9_-]*`) |
| `url_password` | sólo la contraseña de `esquema://usuario:contraseña@`; se conservan el esquema, el usuario y el host. No se enmascara si empieza con `$`, `<`, `{`, `[` o `*` (una variable, un ejemplo o un placeholder) |

Los prefijos de las keys MUST empezar en un borde de palabra. Dos matches que se superponen MUST resolverse a favor del que empieza antes y, si empiezan juntos, del más largo. `redact` MUST ser idempotente: aplicada a su propia salida no encuentra nada. El detector MUST NOT usar entropía.

#### Escenario: una key de cada tipo
- GIVEN un texto con un ejemplo de cada tipo, rodeado de palabras
- WHEN se enmascara
- THEN cada ejemplo quedó como su placeholder, el resto del texto quedó igual, y los tipos son los diez, en orden

#### Escenario: no son secretos
- GIVEN un hash de commit de 40 caracteres, un UUID, un SHA-256 en hex, un identificador como `MAX_LEXICAL_CANDIDATES_PER_QUERY_WINDOW`, `scikit-learn`, `sk-learn-is-not-a-key-at-all`, `AKIA` suelto y `postgres://app:${DB_PASSWORD}@db:5432/app`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

#### Escenario: contraseña en una URL
- GIVEN `postgres://app:s3cr3t-pass@db.internal:5432/app`
- THEN queda `postgres://app:[REDACTED:url_password]@db.internal:5432/app`

#### Escenario: clave privada sin cierre
- GIVEN `config:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk...` sin línea `END`
- THEN queda `config:\n[REDACTED:private_key]`

**Tests:** `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `masking_twice_finds_nothing_new` y `an_anthropic_key_is_never_reported_as_an_openai_one` (`crates/clawg-core/src/memory/redact.rs`).

### R2. La ingesta enmascara antes de trocear

El worker MUST aplicar `redact` al contenido de cada evento, de cualquier fuente, antes de elegir el chunker y de trocear. El hash de dedup, el embedding, el FTS y `memory.content` MUST calcularse sobre el texto enmascarado. `IngestReport` MUST llevar los tipos enmascarados del evento, también cuando el evento falla después.

#### Escenario: el secreto no llega a la base
- GIVEN un save `deploy uses ghp_<36 caracteres>`
- WHEN el worker lo procesa
- THEN `memory.content` es `deploy uses [REDACTED:github_token]`, una consulta FTS por los 36 caracteres no devuelve filas, y el reporte dice `github_token`

#### Escenario: dos saves que sólo difieren en el secreto
- GIVEN un save `the staging key is AKIA<16>` ya guardado
- WHEN se guarda el mismo texto con otra key AWS
- THEN el segundo resultado es `reinforced`, con el id del primero, y la tabla tiene una fila

**Tests:** `a_saved_secret_is_stored_masked_and_not_searchable`, `saves_that_differ_only_in_a_masked_secret_are_one_memory` y `the_report_names_what_was_masked_even_when_the_save_fails` (`worker.rs`, mod tests).

### R3. El save dice qué se enmascaró

Una respuesta `201` de `POST /v1/memories` MUST incluir `redacted`, la lista de tipos del reporte, cuando no está vacía, y MUST omitirlo cuando lo está:

```json
{"status": "saved", "memories": [{"kind": "memory", "id": 57, "outcome": "inserted"}], "redacted": ["github_token"]}
```

`save_memory` MUST agregar al texto de éxito que se enmascararon secretos y de qué tipos. Las respuestas `202` no lo llevan: el worker todavía no vio el evento.

#### Escenario
- GIVEN un daemon con el embedder listo
- WHEN se guarda un texto con una key de Slack
- THEN la respuesta es `201` con `redacted: ["slack_token"]`, y el texto de `save_memory` nombra `slack_token`

**Tests:** `a_save_reports_the_secrets_it_masked` (`crates/clawg-cli/tests/api.rs`), los unit de `save_response` (`api.rs`) y `a_masked_save_tells_the_agent_what_was_masked` (`crates/clawg-cli/src/mcp/server.rs`).

## Lo ya guardado

### R4. `clawg secrets` reporta sin mostrar el secreto

`clawg secrets [--profile P]` MUST recorrer todas las filas de `memory`, vivas y borradas, e imprimir una línea por fila con secreto: `memory <id> <tipos> <live|deleted>`. La salida MUST NOT contener ningún carácter de un secreto (sólo tipos, ids y estado). Sin filas con secreto MUST decirlo. MUST terminar con 0 en los dos casos.

La lectura MUST hacerse en sólo lectura, sin el lock y sin migrar, de modo que funciona con el daemon andando. Sin `memory.db` MUST fallar diciendo que no hay base, sin crearla. El armado de la salida MUST ser puro.

#### Escenario: con el daemon andando
- GIVEN una base con una fila viva con una key AWS, una fila borrada con un JWT y una fila sin secretos, y el lock tomado por otro proceso
- WHEN se escanea
- THEN el reporte tiene las dos primeras filas, con sus tipos y estados, y ningún carácter de la key ni del JWT

**Tests:** `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_reads_while_another_process_holds_the_store` y `the_scan_of_a_missing_store_creates_nothing` (`crates/clawg-core/src/memory/secrets.rs`); `the_secrets_report_lists_rows_and_never_their_content` y `secrets_takes_only_a_profile` (`clawg-cli`).

### R5. `clawg purge` saca una fila para siempre

`clawg purge <ID> --yes [--profile P]` MUST, en este orden:

1. sin `--yes`, negarse sin abrir nada, con un mensaje que dice que no hay vuelta atrás y cómo confirmarlo;
2. si no existe `memory.db`, fallar diciendo que no hay base;
3. tomar el lock del store (titular `Maintenance`); si lo tiene el daemon, fallar diciendo que se pare con `clawg stop`;
4. exigir que el plan de esquema sea `UpToDate`; con `Pending`, fallar diciendo que se arranque el daemon una vez para migrar; con `Newer` o `Diverged`, los errores de siempre;
5. si la fila no existe, fallar con un error que nombra el id;
6. con `secure_delete` activo, en una transacción: si la fila está viva, liberar las filas que la disputaban (como el delete); borrar su vector y la fila (el trigger del FTS resta sus términos si estaba viva);
7. compactar el índice FTS (`optimize`), para que ni la fila ni sus marcas de borrado queden en ningún segmento;
8. `wal_checkpoint(TRUNCATE)`; si no se completa, fallar diciendo que la base está ocupada.

MUST NOT hacer snapshot previo. MUST NOT estar disponible como herramienta MCP ni por la API HTTP.

#### Escenario: fila viva
- GIVEN una base con 300 filas y una viva con `the vault passphrase is zebra-quokka-7741`
- WHEN se purga
- THEN la fila, su vector y su entrada del FTS no existen, una búsqueda FTS por `quokka` no devuelve nada, y los bytes `quokka` no están en `memory.db` ni en `memory.db-wal`

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

**Tests:** `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched` y `purge_refuses_a_store_with_pending_migrations_untouched` (`secrets.rs`); `purge_needs_yes_before_it_opens_anything`, `purge_takes_an_id_a_yes_and_a_profile` y `a_purge_refused_by_the_daemon_says_to_stop_it` (`clawg-cli`).

### R6. La purga nombra los backups que todavía tienen la fila

Después de purgar, MUST revisar cada snapshot de `<data>/backups/` (los archivos con nombre de snapshot) y devolver los que tienen una fila con ese id y el mismo `content`, ordenados por nombre. Uno que no se puede leer MUST listarse también: puede tenerla. MUST NOT modificar ni borrar ningún backup. La salida MUST decir que esos archivos todavía tienen el texto y que borrarlos es decisión del usuario.

#### Escenario
- GIVEN un backup manual tomado con la fila y otro tomado después de borrarla con el delete
- WHEN se purga la fila
- THEN el resultado lista los dos (el delete lógico deja el texto) y los dos archivos quedan byte a byte iguales

- GIVEN un backup tomado antes de guardar la fila
- THEN no se lista

**Tests:** `purge_names_the_backups_that_still_hold_the_row` (`secrets.rs`) y `the_purge_report_names_backups_to_delete_by_hand` (`clawg-cli`).
