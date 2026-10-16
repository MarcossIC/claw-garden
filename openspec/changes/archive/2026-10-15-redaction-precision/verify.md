# redaction-precision — Verificación

**Estado:** verificado en Linux y en Windows ([Windows](#windows)). Al archivar apareció un falso positivo de R4: el separador `[:=]{1,2}` también tomaba `::`, y un camino de Rust a través de un módulo `password` perdía texto; arreglado en `54478a1` ([el separador `::`](#el-separador-)). `clawg secrets --profile dev` no lista ninguna fila ([perfil dev](#clawg-secrets-sobre-el-perfil-dev)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, una vez sobre el último commit de la rama.

| | Pasan | Fallan |
|---|---|---|
| Base (`main`, 210c692) | 1.971 | 7 |
| Rama (`docs: describe the private mark and the new masking rules`) | 1.982 | 7 |

Las 7 fallas son las mismas en `main` y en la rama, y vienen del entorno: `doctor::tests` modela un PATH de Windows con `clawg.exe`.

`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina con código 0.

**Conteo:** 11 tests nuevos, ninguno reemplazado.

### Windows

`cargo test --workspace --no-fail-fast --target-dir target-test`:

| Commit | Pasan | Fallan | Ignorados | Warnings del workspace |
|---|---|---|---|---|
| `main` en `2b1b08a` (este cambio mergeado) | 1.995 | 0 | 0 | 0 |
| `f7ce973` (ingest-outbox mergeado) con el arreglo de `::`, que es `54478a1` | 2.034 | 0 | 0 | 0 |

1.995 es la base de 1.984 (`8312e42`) más los 11 de este cambio. Entre las dos corridas `main` avanzó a `f7ce973` con ingest-outbox, que suma 39 tests (su delta en Linux): 1.995 + 39 = 2.034. El arreglo no agrega tests, suma casos a dos que ya estaban. Los 7 de `doctor::tests` que fallan en Linux pasan en Windows.

`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: **código 0** en las dos, sin ningún warning.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1. `sk-` exige la forma de una clave | `a_hyphenated_identifier_after_sk_is_left_alone`, `every_form_of_an_openai_key_is_masked` |
| R2. La marca privada | `a_private_span_is_masked_with_its_marks`, `an_open_private_mark_masks_to_the_end`, `a_mark_quoted_in_backticks_is_text`, `a_private_span_covers_the_secrets_inside_it` |
| R3. Formatos nuevos | `every_known_secret_format_is_masked_in_place` |
| R4. Contraseñas y cabeceras | `a_literal_password_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_password_key_are_left_alone` (con los caminos `::` desde `54478a1`), `an_authorization_header_masks_only_its_credential`, `a_known_format_wins_over_its_context` |
| R5. Idempotencia | `masking_twice_finds_nothing_new` |
| R6. `clawg secrets` sobre lo ya guardado | `the_scan_flags_rows_saved_before_a_new_rule` |

## El separador `::`

Al consolidar R4 se pasó el patrón `password` por casos que la tabla de tests no tenía, con `rg`, que usa el mismo crate `regex`. El diseño ([D3](design.md#d3-los-patrones-por-contexto)) decía que `[:=]{1,2}` cubre `=`, `:`, `:=` y `==`, pero también cubre `::` y `=:`. Con `::`, el valor capturado es lo que sigue en un camino de Rust o C++, e `is_literal_secret` lo aceptaba si tenía un dígito o un `:`:

| Texto | Valor capturado | Antes de `54478a1` |
|---|---|---|
| `use crate::auth::password::rules::check;` | `rules::check` | `use crate::auth::password::[REDACTED:password];` |
| `use crate::password::hash_v2;` | `hash_v2` | `use crate::password::[REDACTED:password];` |
| `algo: Password::Argon2id` | `Argon2id` | `algo: Password::[REDACTED:password]` |
| `password::Hasher::new()` | `Hasher::new(` | sin cambios, por el `(` |

R4 nombraba sólo los cuatro separadores, así que el código estaba fuera del spec. `54478a1` escribe el separador como `(?:==|:=|=|:)` y exige que el valor no empiece con `:`. Los tres primeros casos pasan a `code_and_placeholders_after_a_password_key_are_left_alone`, y `password==s3cr3t9` a `a_literal_password_is_masked_and_its_key_kept`, para fijar que `==` sigue enmascarando. Con el código anterior el test fallaba por la aserción (`password::[REDACTED:password];`). Afectaba también al índice de docs, que pasa cada archivo de código por `redact`; el repositorio no tiene un módulo `password`, y por eso la pasada del repo entero no lo vio.

## El repositorio por las dos versiones de `redact`

Se pasaron los 345 archivos de texto del repositorio (`.md`, `.rs`, `.toml`, `.json`, `.yaml`, `.txt`, sin `target` ni `.git`) por el `redact` de `main` y por el de la rama. 339 dan lo mismo. Los 6 que cambian son los de este cambio (`redact.rs`, `secrets.rs` y los documentos de `changes/redaction-precision/`), donde los casos de prueba aparecen como texto:

- con `main`, `exploration.md`, `proposal.md` y `spec.md` perdían `sk-learn-pipeline-for-text-classification-v2-2026-model` y `sk-build-2026-release-candidate-01`, que eran falsos positivos; con la rama quedan enteros;
- con la rama, los ejemplos de `password=`, `Authorization` y `<private>` de esos documentos se enmascaran, como se espera. `redact.rs` pierde, en el índice de docs, el tramo entre `Regex::new(r"(?i)<private>")` y el cierre de la línea siguiente: es código que escribe la marca sin comillas invertidas, y el índice es derivado ([diseño, D2](design.md#d2-la-marca)).

Ningún otro archivo, ni `.claude/CLAUDE.md` ni el resto de `openspec/`, cambia: no hay falsos positivos nuevos en el repositorio.

## `clawg secrets` sobre el perfil dev

Con el binario de `target-test` (0.9.0, con `54478a1`), en sólo lectura y sin el lock: `no stored memory holds a secret of a known format`, código 0. Ninguna fila guardada en claro tiene un secreto que sólo las reglas nuevas reconozcan, y no hay falsos positivos nuevos.

No se compiló el binario de `210c692` para comparar: sólo mostraría filas que desaparecen, un `sk-` guardado en claro antes de secret-hygiene que la versión anterior tomaba por clave, y lo que se busca es que la nueva no liste nada. Tampoco se contó cuántas filas tiene la base: pedía leer `~/.clawg-dev` con otra herramienta, y no se hizo; `clawg memories stats --profile dev` lo dice con el daemon dev andando. No se tocó `~/.clawg`.

## Contrato del agente

No cambia: `agent_contract.json` no se tocó y `the_agent_contract_matches_its_file` pasa en las dos corridas de Windows.

## rag-eval

No aplica: el cambio no toca ranking ni recuperación.

## Diferencias entre el spec y el código

Al consolidar en [`specs/secrets`](../../../specs/secrets/spec.md) se revisó contra `main` (`54478a1`) cada `ruta:línea` de R1 y R4 y cada test citado. Los 17 tests de `redact.rs` y el de `secrets.rs` existen. Las diferencias:

1. **Arreglada.** El separador `::` de R4 ([arriba](#el-separador-)).
2. D2 dice que una marca sin cerrar en un bloque de código «enmascara hasta el final del texto o del pasaje». El índice de docs enmascara el archivo entero antes de trocearlo (`crates/clawg-core/src/memory/project_docs/mod.rs:382`), así que una apertura sin comilla invertida delante enmascara hasta el próximo cierre del archivo o hasta su final, no del pasaje. `specs/secrets` y `specs/project-docs` lo dicen así, y los escenarios de la marca en `specs/secrets` ponen cada apertura detrás de una comilla invertida para que el índice no pierda texto de la spec.
3. D1 dice que `sk-Learn-Pipeline-V2-2026-model` «queda fijado como caso conocido en la tabla». Ningún test lo fija; `specs/secrets` lo dice como regla: un identificador que mezcla mayúsculas y minúsculas y tiene un dígito después de `sk-` se sigue enmascarando.
4. R4 dice `Authorization:` o `Authorization=`. El patrón acepta también la clave entre comillas (`"Authorization": "Bearer …"`), la forma de JSON (`redact.rs:107`). Ningún test la enmascara: `an_authorization_header_masks_only_its_credential` sólo la usa con un valor de plantilla (`f"Bearer {token}"`), que queda igual. `specs/secrets` lo dice.
5. La lista de llamadores de `redact` de `design.md` cita `worker.rs:430`; hoy es `worker.rs:591`, dentro de `ingest_pieces` (`worker.rs:578`), porque ingest-outbox lo movió.
6. Conocido y fuera del spec: `password: !vault |` (Ansible) enmascara la etiqueta `!vault`, porque `!` cuenta como símbolo; el valor cifrado, en las líneas siguientes, no se toca. Se pierde la etiqueta que dice cómo está cifrado el valor, y queda así hasta que aparezca en una memoria real.

Citas viejas que no vienen de este cambio y quedan como estaban en `specs/secrets`, porque ingest-outbox, todavía sin archivar, reescribe R2, R3 y R5: R2 cita `process_event`, `worker.rs:432` (hoy el enmascarado está en `ingest_pieces`, `worker.rs:591`) e `IngestReport.redacted`, `worker.rs:78` (hoy `:83`); R3 cita `SaveBody.redacted`, `api.rs:434` (hoy `:466`), `save_response`, `api.rs:624` (hoy `:701`), `client.rs:298` (hoy `:370`) y `save_outcome_text`, `server.rs:391` (hoy `:495`); R5 cita `cli.rs:88` y `main.rs:157` (hoy `:89` y `:160`), `PurgeError::NotFound`, `secrets.rs:56` (hoy `:58`), la transacción en `secrets.rs:102` (hoy `:103-115`, que además borra el texto del outbox), `optimize` en `:116`, `VACUUM` en `:122` y `wal_checkpoint` en `:124` (hoy `:120`, `:126` y `:128`); R6 cita `Purged.backups`, `secrets.rs:69` (hoy `:72`).
