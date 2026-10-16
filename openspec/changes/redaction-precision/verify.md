# redaction-precision — Verificación

**Estado:** verificado en Linux. Falta la suite y el `clippy` en Windows, y `clawg secrets` sobre el perfil dev ([pendiente](#pendiente-máquina-de-marcos)).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux, una vez sobre el último commit de la rama.

| | Pasan | Fallan |
|---|---|---|
| Base (`main`, 210c692) | 1.971 | 7 |
| Rama (`docs: describe the private mark and the new masking rules`) | 1.982 | 7 |

Las 7 fallas son las mismas en `main` y en la rama, y vienen del entorno: `doctor::tests` modela un PATH de Windows con `clawg.exe`.

`cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` termina con código 0.

**Conteo:** 11 tests nuevos, ninguno reemplazado. En Windows se espera la base de `main` más 11, sin fallas.

### Qué cubre cada requisito

| Requisito | Tests |
|---|---|
| R1. `sk-` exige la forma de una clave | `a_hyphenated_identifier_after_sk_is_left_alone`, `every_form_of_an_openai_key_is_masked` |
| R2. La marca privada | `a_private_span_is_masked_with_its_marks`, `an_open_private_mark_masks_to_the_end`, `a_mark_quoted_in_backticks_is_text`, `a_private_span_covers_the_secrets_inside_it` |
| R3. Formatos nuevos | `every_known_secret_format_is_masked_in_place` |
| R4. Contraseñas y cabeceras | `a_literal_password_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_password_key_are_left_alone`, `an_authorization_header_masks_only_its_credential`, `a_known_format_wins_over_its_context` |
| R5. Idempotencia | `masking_twice_finds_nothing_new` |
| R6. `clawg secrets` sobre lo ya guardado | `the_scan_flags_rows_saved_before_a_new_rule` |

## El repositorio por las dos versiones de `redact`

Se pasaron los 345 archivos de texto del repositorio (`.md`, `.rs`, `.toml`, `.json`, `.yaml`, `.txt`, sin `target` ni `.git`) por el `redact` de `main` y por el de la rama. 339 dan lo mismo. Los 6 que cambian son los de este cambio (`redact.rs`, `secrets.rs` y los documentos de `changes/redaction-precision/`), donde los casos de prueba aparecen como texto:

- con `main`, `exploration.md`, `proposal.md` y `spec.md` perdían `sk-learn-pipeline-for-text-classification-v2-2026-model` y `sk-build-2026-release-candidate-01`, que eran falsos positivos; con la rama quedan enteros;
- con la rama, los ejemplos de `password=`, `Authorization` y `<private>` de esos documentos se enmascaran, como se espera. `redact.rs` pierde, en el índice de docs, el tramo entre `Regex::new(r"(?i)<private>")` y el cierre de la línea siguiente: es código que escribe la marca sin comillas invertidas, y el índice es derivado ([diseño, D2](design.md#d2-la-marca)).

Ningún otro archivo, ni `.claude/CLAUDE.md` ni el resto de `openspec/`, cambia: no hay falsos positivos nuevos en el repositorio.

## Contrato del agente

No cambia: `agent_contract.json` no se tocó y `the_agent_contract_matches_its_file` pasa.

## rag-eval

No aplica: el cambio no toca ranking ni recuperación.

## Pendiente (máquina de Marcos)

1. `cargo test --workspace --target-dir <otro-dir>`: se espera la base de `main` más 11 tests y 0 fallas.
2. `cargo clippy --workspace --all-targets --target-dir <otro-dir> -- -A clippy::all -D warnings`: código 0.
3. `clawg secrets --profile dev` con el binario de `main` y con el de la rama (sólo lectura, funciona con el daemon andando). Cada fila que aparece sólo con la rama es un secreto que se había guardado en claro (`password`, `auth_header`, `private` o un formato nuevo) o un falso positivo nuevo que hay que mirar; cada fila que desaparece es un `sk-` que ya no se toma por clave.
