# secret-coverage — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. Formatos con prefijo (R1)

- [x] `one_of_each` suma `hf_` y el webhook de Slack; tests `a_pgp_private_key_block_is_masked` y `the_slack_app_and_stripe_webhook_forms_are_masked`.
- [x] `huggingface_token` y `slack_webhook`; `xapp-`, `whsec_` y `PRIVATE KEY BLOCK` en los patrones existentes.
- Commit: `feat(core): mask Hugging Face, Slack app and webhook, Stripe webhook and PGP secrets`

## 2. Credenciales por clave (R1)

- [x] Tests `a_literal_credential_is_masked_and_its_key_kept` y `code_and_placeholders_after_a_credential_key_are_left_alone`; `a_known_format_wins_over_its_context` y `masking_twice_finds_nothing_new` con el tipo nuevo.
- [x] `SecretKind::Credential` y su patrón; `accepts` exige el valor literal y una letra.
- Commit: `feat(core): mask literal values after secret, token and API key names`

## 3. El `reason` de un veredicto (R7)

- [x] Test `a_verdict_reason_is_stored_masked` (`dispute.rs`).
- [x] `judge_pair` enmascara el `reason`.
- Commit: `fix(core): mask secrets in the reason of a verdict`

## 4. `last_error` (R9)

- [x] Test `a_failure_is_stored_masked` (`outbox.rs`).
- [x] `outbox_fail` y el evento que no se lee enmascaran su `last_error`.
- Commit: `fix(core): mask secrets in the error an outbox row records`

## 5. `clawg secrets` mira los veredictos (R4)

- [x] Tests `the_scan_finds_reasons_that_hold_a_secret`, `the_scan_of_a_store_from_before_the_pairs_lists_rows_only` y `the_secrets_report_lists_pairs_and_how_to_mask_them`.
- [x] `scan` devuelve filas y pares; `report` los imprime.
- Commit: `feat: list verdict reasons that hold a secret in clawg secrets`

## 6. `clawg secrets --mask-reasons --yes` (R8)

- [x] Tests `masking_reasons_rewrites_only_the_ones_with_a_secret`, `masking_reasons_refuses_while_the_daemon_holds_the_store`, `mask_reasons_needs_yes`, `the_mask_report_names_pairs_and_backups` y `secrets_takes_a_profile_and_masks_reasons_on_request` (`cli.rs`).
- [x] `mask_reasons` en core, el flag y su salida en la CLI.
- Commit: `feat: mask the secrets of stored verdict reasons on request`

## 7. `purge` y el outbox (R5)

- [x] Test `purge_keeps_other_agents_and_longer_saves_in_the_outbox`; `purge_removes_the_text_from_the_outbox` con el agente.
- [x] Filtro por agente, igualdad para un save de un pedazo; `Purged.outbox`.
- Commit: `fix(core): purge only the outbox rows of the purged memory's own save`

## 8. `purge` y los pedazos (R5)

- [x] Test `purging_the_first_piece_empties_the_head_of_its_save`.
- [x] `context` vacío para el save del pedazo 1; `Purged.pieces`.
- Commit: `fix(core): purge the head of a long save with its first piece`

## 9. `purge` revive la historia del tema (R5)

- [x] Test `purging_the_current_revision_brings_the_previous_one_back`.
- [x] La revival corre en la transacción; `Purged.revived` y `Purged.hidden`.
- Commit: `fix(core): bring a topic's previous revision back when its current one is purged`

## 10. `purge` nombra los `workspace.md` (R6)

- [x] Tests `the_sections_holding_a_text_are_told_apart` y `purge_names_the_workspaces_that_still_hold_the_text`.
- [x] `sections_holding`, la búsqueda de archivos y `Purged.workspaces`.
- Commit: `feat(core): name the workspace files that still hold a purged text`

## 11. La salida de `clawg purge` (R5, R6)

- [x] Test `the_purge_report_names_what_came_back_and_what_went`.
- [x] `purge_report` con cada grupo.
- Commit: `feat(cli): tell what a purge brought back, removed and left behind`

## 12. Docs

- [x] `.claude/CLAUDE.md`: los formatos y la clave nuevos, `--mask-reasons` y lo que hace `purge`.
- Commit: `docs: describe the new masking rules and what purge reaches`

## 13. Verificación

- [x] `cargo test --workspace`, clippy con `-A clippy::all -D warnings`, el corpus del repositorio y un daemon real en un home de prueba, en [`verify.md`](verify.md).
