# secret-coverage — Especificación (delta sobre `specs/secrets`)

Los ids siguen a los de [`specs/secrets`](../../../specs/secrets/spec.md). R1, R4, R5 y R6 se modifican; R7, R8 y R9 son nuevos.

## R1 (modifica). Formatos y patrones nuevos

Se suman a la tabla de formatos conocidos:

| Tipo | Forma |
|---|---|
| `huggingface_token` | `hf_` y exactamente 34 de `[A-Za-z0-9]`, como palabra entera, con al menos una mayúscula, una minúscula y un dígito |
| `slack_token` | además, `xapp-` y 10 o más de `[A-Za-z0-9-]` |
| `slack_webhook` | `https://hooks.slack.com/services/T…/B…/…`: `T` y 6 o más de `[A-Z0-9]`, `/B` y 6 o más, `/` y 20 o más de `[A-Za-z0-9]`; la URL entera |
| `stripe_key` | además, `whsec_` y 24 o más de `[A-Za-z0-9]` |
| `private_key` | además, la cabecera PGP: de `-----BEGIN … PRIVATE KEY BLOCK-----` al `-----END … PRIVATE KEY BLOCK-----`, o al final del texto |

Y un tercer patrón por contexto, que MUST enmascarar sólo el valor y conservar la clave, el separador y las comillas:

- `credential`: el valor de una clave que termina en `secret`, `token`, `api_key`, `apikey`, `api-key`, `access_key`, `secret_key`, `account_key`, `accountkey` o `auth_key` (sin distinguir mayúsculas, separadas por `_` o `-` o pegadas; con cualquier prefijo de letras, dígitos, `_` o `-`, como `OPENROUTER_API_KEY`, `client_secret` o `X-Api-Key`; entre comillas o no), seguida de `=`, `:`, `:=` o `==`. El valor MUST ser un valor literal (la regla de `password`) y MUST tener al menos una letra.

Un formato conocido MUST seguir ganando sobre su contexto (`HF_TOKEN=hf_…` queda `HF_TOKEN=[REDACTED:huggingface_token]`), y `redact` MUST seguir siendo idempotente con todos los tipos.

#### Escenario: un ejemplo de cada formato
- GIVEN un texto con un ejemplo sintético de cada formato conocido, los nuevos incluidos
- WHEN se enmascara
- THEN cada uno quedó como su placeholder y los tipos van en orden

#### Escenario: claves de configuración
- GIVEN `AWS_SECRET_ACCESS_KEY=<40 base64>`, `OPENROUTER_API_KEY=sk-or-v1-<64 hex>`, `"client_secret": "<34>"`, `X-Api-Key: <32>`, `AccountKey=<base64>==`, `GITHUB_TOKEN: a1b2c3d4e5f6` y `secret = "zebra-quokka-7741"`
- WHEN se enmascaran
- THEN cada valor queda `[REDACTED:credential]` y la clave queda

#### Escenario: código y prosa con las mismas palabras
- GIVEN `token = next_token()`, `secret: string`, `api_key: Option<String>`, `let token = parser.token;`, `max_token=100000`, `max_tokens: 4096`, `GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`, `api_key=$OPENAI_API_KEY`, `secret_name: my-app`, `tokenizer: bert-base-2`, `api_key: YOUR_API_KEY` y `AutoModelForSequenceClassification`
- WHEN se enmascara
- THEN el texto no cambia y no hay tipos

**Tests:** `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_literal_credential_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_credential_key_are_left_alone`, `a_known_format_wins_over_its_context`, `a_pgp_private_key_block_is_masked`, `the_slack_app_and_stripe_webhook_forms_are_masked`, `masking_twice_finds_nothing_new` (`redact.rs`).

## R4 (modifica). `clawg secrets` también mira los veredictos

`scan` MUST recorrer también `memory_conflict.reason` y reportar cada par con secreto como `pair <new_id> <old_id> <tipos>`, sin el texto. Sin filas ni pares con secreto MUST decir `no stored memory holds a secret of a known format`. Si hay pares, la última línea MUST decir cómo limpiarlos: `clawg stop`, luego `clawg secrets --mask-reasons --yes`.

#### Escenario
- GIVEN un par juzgado con `reason` `rotated ghp_<36>` escrito directo en la tabla, y una fila de `memory` con un JWT
- WHEN se escanea
- THEN el reporte lista la fila y el par, con sus tipos, y ningún carácter del token

**Tests:** `the_scan_finds_reasons_that_hold_a_secret` y `the_scan_of_a_store_from_before_the_pairs_lists_rows_only` (`crates/clawg-core/src/memory/secrets.rs`), `the_secrets_report_lists_pairs_and_how_to_mask_them` (`crates/clawg-cli/src/secrets.rs`).

## R5 (modifica). Lo que `purge` borra y lo que devuelve

El paso 6 de R5 pasa a ser, en una transacción:

1. si la fila está viva, marcarla borrada y aplicar la misma revival que un `delete` (`revive_after_delete_of_piece`): la revisión más nueva de su tema vuelve a estar viva, las anteriores la apuntan, y lo que reemplazó un veredicto queda escondido;
2. si la fila es la posición 1 de un save largo, vaciar el `context` de todos los vínculos de ese save; el vínculo de la fila queda;
3. borrar su vector, la fila, sus enlaces de `memory_derivation` como conclusión y sus pares de `memory_conflict`;
4. borrar las filas del outbox de su agente (`agent_id` igual al de la fila; una fila sin agente no filtra) cuyo `content` es igual al de la fila, si la fila no es un pedazo de un save largo, o lo contiene, si lo es;
5. si estaba viva, liberar las filas que la disputaban.

`Purged` MUST nombrar las filas revividas, las que quedan escondidas por veredicto, los otros pedazos del save (ids) y las filas del outbox borradas (`operation_id`, agente y estado), y la salida de `clawg purge` MUST decirlo en una línea por grupo, sólo cuando el grupo no está vacío.

#### Escenario: otro agente con el mismo texto
- GIVEN una fila de `agent-s` con `use pnpm`, y en el outbox una fila de `agent-o` con `use pnpm`, una de `agent-s` con `use pnpm` y otra de `agent-s` con `use pnpm in the monorepo`
- WHEN se purga la fila
- THEN el outbox conserva la de `agent-o` y la más larga de `agent-s`, y `Purged` nombra la borrada

#### Escenario: un pedazo de un save largo
- GIVEN un save de tres pedazos cuyos vínculos 2 y 3 llevan la cabecera del primero en `context`, y en el outbox una fila del mismo agente con el save entero
- WHEN se purga el pedazo 1
- THEN los tres vínculos quedan, con `context` vacío, la fila del outbox se borra, y `Purged` nombra los pedazos 2 y 3

#### Escenario: la versión vigente de un tema
- GIVEN tres revisiones de `deploy/target` (la vigente con un secreto) y una fila que un veredicto `supersedes` hizo reemplazar por la vigente
- WHEN se purga la vigente
- THEN la revisión del medio vuelve a estar viva, la primera la apunta, la del veredicto sigue `superseded`, y `Purged` las nombra

**Tests:** `purge_keeps_other_agents_and_longer_saves_in_the_outbox`, `purging_the_first_piece_empties_the_head_of_its_save`, `purging_the_current_revision_brings_the_previous_one_back` (`crates/clawg-core/src/memory/secrets.rs`); `the_purge_report_names_what_came_back_and_what_went` (`crates/clawg-cli/src/secrets.rs`).

## R6 (modifica). La purga nombra los `workspace.md` con el texto

Además de los backups, después de purgar MUST leer cada `workspaces/<agente>/<proyecto>/workspace.md` del directorio de datos y devolver los que tienen el texto de la fila, cada uno con si está en la sección AUTO, en la USER o en las dos, ordenados por ruta. MUST NOT escribir ningún archivo. La salida MUST decir que la sección AUTO se regenera en el próximo `recall_context` de ese agente en ese proyecto y que la USER se edita a mano. La decisión de en qué sección está MUST ser pura (`sections_holding`).

#### Escenario
- GIVEN un `workspace.md` con el texto en AUTO, otro con el texto en USER y otro sin él
- WHEN se purga la fila
- THEN `Purged` nombra los dos primeros con su sección y los tres archivos quedan byte a byte iguales

**Tests:** `the_sections_holding_a_text_are_told_apart` (`workspace.rs`), `purge_names_the_workspaces_that_still_hold_the_text` (`secrets.rs`).

## R7 (nuevo). El `reason` de un veredicto se guarda enmascarado

`judge_pair` MUST pasar el `reason` por `redact` antes de guardarlo en `memory_conflict.reason`. Un `reason` que queda vacío después de recortarlo sigue siendo `None`.

#### Escenario
- GIVEN un par pendiente
- WHEN se juzga `scoped` con `reason` `the staging key AKIA<16> is only for ci`
- THEN `memory_conflict.reason` es `the staging key [REDACTED:aws_access_key] is only for ci`

**Tests:** `a_verdict_reason_is_stored_masked` (`dispute.rs`).

## R8 (nuevo). `clawg secrets --mask-reasons --yes`

MUST, en este orden: sin `--yes`, negarse sin abrir nada; sin `memory.db`, fallar; tomar el lock (`Maintenance`) y fallar si lo tiene el daemon, diciendo que se pare; exigir el esquema `UpToDate`; en una transacción, reemplazar cada `reason` con secreto por `redact(reason)`; `VACUUM` y `wal_checkpoint(TRUNCATE)`, como `purge`. MUST NOT hacer snapshot. MUST devolver los pares que cambió y los backups que todavía tienen alguno de esos `reason`.

#### Escenario
- GIVEN un par con `reason` `rotated ghp_<36>` y otro sin secreto
- WHEN se corre con `--yes`
- THEN el primero queda `rotated [REDACTED:github_token]`, el otro igual, y los bytes del token no están en `memory.db` ni en el WAL

**Tests:** `masking_reasons_rewrites_only_the_ones_with_a_secret`, `masking_reasons_refuses_while_the_daemon_holds_the_store` (`secrets.rs`); `mask_reasons_needs_yes` y `the_mask_report_names_pairs_and_backups` (`crates/clawg-cli/src/secrets.rs`); `secrets_takes_a_profile_and_masks_reasons_on_request` (`cli.rs`).

## R9 (nuevo). `last_error` se guarda enmascarado

Todo `last_error` que lleva el texto de un error MUST pasar por `redact` antes de escribirse en `ingest_outbox`: el de `outbox_fail` y el del evento que no se lee, cuyo error de serde repite valores del evento. El abandono tras tres arranques es un mensaje fijo.

#### Escenario
- GIVEN una fila del outbox
- WHEN falla con `embed failed: token ghp_<36>`
- THEN `last_error` es `embed failed: token [REDACTED:github_token]`

- GIVEN una fila del outbox cuyo evento lleva `"importance": "ghp_<36>"`
- WHEN se cargan las pendientes
- THEN queda `failed` y su `last_error` no tiene el token

**Tests:** `a_failure_is_stored_masked` (`outbox.rs`).
