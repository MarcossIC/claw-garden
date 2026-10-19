# secret-coverage — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. Formatos con prefijo (R1)

- [ ] `one_of_each` suma `hf_`, `xapp-`, el webhook de Slack y `whsec_`; test `a_pgp_private_key_block_is_masked`.
- [ ] `huggingface_token` y `slack_webhook`; `xapp-`, `whsec_` y `PRIVATE KEY BLOCK` en los patrones existentes.
- Commit: `feat(core): mask Hugging Face, Slack app and webhook, Stripe webhook and PGP secrets`

## 2. Credenciales por clave (R1)

- [ ] Tests `a_literal_credential_is_masked_and_its_key_kept` y `code_and_placeholders_after_a_credential_key_are_left_alone`; `a_known_format_wins_over_its_context` y `masking_twice_finds_nothing_new` con el tipo nuevo.
- [ ] `SecretKind::Credential` y su patrón; `accepts` exige el valor literal y una letra.
- Commit: `feat(core): mask literal values after secret, token and API key names`

## 3. El `reason` de un veredicto (R7)

- [ ] Test `a_verdict_reason_is_stored_masked` (`dispute.rs`).
- [ ] `judge_pair` enmascara el `reason`.
- Commit: `fix(core): mask secrets in the reason of a verdict`

## 4. `last_error` (R9)

- [x] Test `a_failure_is_stored_masked` (`outbox.rs`).
- [x] `outbox_fail` y el evento que no se lee enmascaran su `last_error`.
- Commit: `fix(core): mask secrets in the error an outbox row records`

## 5. `clawg secrets` mira los veredictos (R4)

- [ ] Tests `the_scan_finds_reasons_that_hold_a_secret` y `the_secrets_report_lists_pairs_and_how_to_mask_them`.
- [ ] `scan` devuelve filas y pares; `report` los imprime.
- Commit: `feat: list verdict reasons that hold a secret in clawg secrets`

## 6. `clawg secrets --mask-reasons --yes` (R8)

- [ ] Tests `masking_reasons_rewrites_only_the_ones_with_a_secret`, `masking_reasons_refuses_while_the_daemon_holds_the_store`, `mask_reasons_needs_yes` y el parseo en `cli.rs`.
- [ ] `mask_reasons` en core, el flag y su salida en la CLI.
- Commit: `feat: mask the secrets of stored verdict reasons on request`

## 7. `purge` y el outbox (R5)

- [ ] Test `purge_keeps_other_agents_and_longer_saves_in_the_outbox`; `purge_removes_the_text_from_the_outbox` con el agente.
- [ ] Filtro por agente, igualdad para un save de un pedazo; `Purged.outbox`.
- Commit: `fix(core): purge only the outbox rows of the purged memory's own save`

## 8. `purge` y los pedazos (R5)

- [ ] Test `purging_the_first_piece_empties_the_head_of_its_save`.
- [ ] `context` vacío para el save del pedazo 1; `Purged.pieces`.
- Commit: `fix(core): purge the head of a long save with its first piece`

## 9. `purge` revive la historia del tema (R5)

- [ ] Test `purging_the_current_revision_brings_the_previous_one_back`.
- [ ] La revival corre en la transacción; `Purged.revived` y `Purged.hidden`.
- Commit: `fix(core): bring a topic's previous revision back when its current one is purged`

## 10. `purge` nombra los `workspace.md` (R6)

- [ ] Tests `the_sections_holding_a_text_are_told_apart` y `purge_names_the_workspaces_that_still_hold_the_text`.
- [ ] `sections_holding`, la búsqueda de archivos y `Purged.workspaces`.
- Commit: `feat(core): name the workspace files that still hold a purged text`

## 11. La salida de `clawg purge` (R5, R6)

- [ ] Test `the_purge_report_names_what_came_back_and_what_went`.
- [ ] `purge_report` con cada grupo.
- Commit: `feat(cli): tell what a purge brought back, removed and left behind`

## 12. Docs

- [ ] `.claude/CLAUDE.md`: los formatos y la clave nuevos, `--mask-reasons` y lo que hace `purge`.
- Commit: `docs: describe the new masking rules and what purge reaches`

## 13. Verificación

- [ ] `cargo test --workspace`, clippy con `-A clippy::all -D warnings`, y el corpus de prosa del repositorio antes y después en `verify.md`.
