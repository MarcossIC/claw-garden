# redaction-precision — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. `sk-` exige la forma de una clave (R1, D1)

- [x] Tests `a_hyphenated_identifier_after_sk_is_left_alone` y `every_form_of_an_openai_key_is_masked` (`redact.rs`).
- [x] `accepts(OpenaiKey)` exige mayúscula, minúscula y dígito.
- Commit: `fix(core): stop masking hyphenated identifiers that start with sk-`

## 2. La marca privada (R2, D2)

- [x] Tests `a_private_span_is_masked_with_its_marks`, `an_open_private_mark_masks_to_the_end`, `a_mark_quoted_in_backticks_is_text` y `a_private_span_covers_the_secrets_inside_it` (`redact.rs`).
- [x] `SecretKind::Private`, `private_spans` y su entrada en `find_secrets`.
- Commit: `feat(core): mask text the writer marks as private`

## 3. Formatos nuevos (R3, D4)

- [x] `one_of_each` suma un ejemplo de cada tipo nuevo; `every_known_secret_format_is_masked_in_place` falla.
- [x] `gitlab_token`, `npm_token`, `google_oauth_secret` y `sendgrid_key` en `SecretKind` y `PATTERNS`.
- Commit: `feat(core): mask GitLab, npm, Google OAuth and SendGrid secrets`

## 4. Contraseñas y cabeceras de autorización (R4, D3)

- [x] Tests `a_literal_password_is_masked_and_its_key_kept`, `code_and_placeholders_after_a_password_key_are_left_alone`, `an_authorization_header_masks_only_its_credential` y `a_known_format_wins_over_its_context` (`redact.rs`).
- [x] `password` y `auth_header` en `SecretKind` y `PATTERNS`; `is_literal_secret`; sólo el valor se enmascara.
- Commit: `feat(core): mask literal passwords and authorization credentials`

## 5. Idempotencia y lo ya guardado (R5, R6)

- [x] `masking_twice_finds_nothing_new` con la marca y los dos patrones por contexto.
- [x] Test `the_scan_flags_rows_saved_before_a_new_rule` (`secrets.rs`).
- Commit: `test(core): pin that the new masking rules are idempotent and reach stored rows`

## 6. Docs

- [x] `.claude/CLAUDE.md`: la marca `<private>`, los patrones por contexto y los formatos nuevos.
- Commit: `docs: describe the private mark and the new masking rules`

## 7. Verificación

- [x] `cargo test --workspace` sobre el último commit, clippy con `-A clippy::all -D warnings`, `verify.md` con el conteo contra la base.
- [ ] Pendiente en Windows (máquina de Marcos): la suite, clippy y `clawg secrets` sobre el perfil dev.
