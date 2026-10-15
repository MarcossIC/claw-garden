# secret-hygiene — Tareas

Cada tarea es un commit atómico (conventional commits) que compila y deja `cargo test --workspace` sin fallas nuevas. Dentro de cada una, primero el test que falla (RED), después el código (GREEN). En Windows: `cargo test --workspace --target-dir <otro-dir>`.

Los mensajes de commit son sugeridos. No mencionan planes, fases ni specs.

## Enmascarado

### 1. Detector puro — R1
- [x] RED: `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `masking_twice_finds_nothing_new` y `an_anthropic_key_is_never_reported_as_an_openai_one` en un módulo nuevo, `crates/clawg-core/src/memory/redact.rs`.
- [x] GREEN: `regex` como dependencia directa de `clawg-core` (`workspace = true`); `enum SecretKind` con `as_str`; `find_secrets(&str) -> Vec<Found>` y `redact(&str) -> Redacted { text, kinds }`, puras, con los patrones compilados una vez.
- [x] Commit: `feat(core): mask known secret formats in text` (`caf0cff`)

### 2. La ingesta enmascara — R2
- [x] RED: `a_saved_secret_is_stored_masked_and_not_searchable`, `saves_that_differ_only_in_a_masked_secret_are_one_memory` y `the_report_names_what_was_masked_even_when_the_save_fails` en `worker.rs`.
- [x] GREEN: `process_event` aplica `redact` antes de `select_chunker`; `IngestReport.redacted: Vec<SecretKind>`, en todos los caminos de salida.
- [x] Commit: `feat(core): mask secrets before a save is chunked, hashed or embedded` (`2dce41f`)

### 3. El save avisa — R3
- [x] RED: `a_save_reports_the_secrets_it_masked` en `crates/clawg-cli/tests/api.rs`, los unit de `save_response` con y sin tipos, y `a_masked_save_tells_the_agent_what_was_masked` en `mcp/server.rs`.
- [x] GREEN: `SaveBody.redacted` (omitido si está vacío); `SaveOutcome::Saved.redacted` en `client.rs` (con `#[serde(default)]`, para un daemon anterior); `save_outcome_text` lo nombra.
- [x] Commit: `feat(cli): tell the saver which secrets were masked` (`584b83f`)

## Lo ya guardado

### 4. Escaneo — R4
- [x] RED: `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_reads_while_another_process_holds_the_store` y `the_scan_of_a_missing_store_creates_nothing` en un módulo nuevo, `crates/clawg-core/src/memory/secrets.rs`.
- [x] GREEN: `scan(base_dir) -> Result<Vec<Flagged>, MemoryError>` con `open_for_probe` (pasa a `pub(crate)`); `Flagged { id, kinds, deleted }`.
- [x] Commit: `feat(core): find stored memories that hold a secret` (`a2272c7`)

### 5. `clawg secrets` — R4
- [x] RED: `secrets_takes_only_a_profile` en `cli.rs`, y `the_secrets_report_lists_rows_and_never_their_content` en un módulo nuevo, `crates/clawg-cli/src/secrets.rs`.
- [x] GREEN: el comando en `main.rs` y el reporte puro.
- [x] Commit: `feat(cli): add clawg secrets` (`4b417c8`)

### 6. Purga — R5, R6
- [x] RED: `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched`, `purge_refuses_a_store_with_pending_migrations_untouched` y `purge_names_the_backups_that_still_hold_the_row` en `secrets.rs`.
- [x] GREEN: `purge(base_dir, id) -> Result<Purged, MemoryError>` con los pasos de R5 (D4, D5); `PurgeError` con `thiserror` y `MemoryError::Purge`; `Purged { backups }` (D6). `StoreHolder::Maintenance` nombra la purga.
- [x] Lo que encontraron los tests antes del commit: `secure_delete` no alcanzaba (una página libre de antes tenía el texto) y pasó a `VACUUM`; `release_challengers` va después del `DELETE`. Detalle en `design.md`, D5.
- [x] Commit: `feat(core): purge one memory for good, with its vector and index terms` (`bfdd4a8`)

### 7. `clawg purge` — R5, R6
- [x] RED: `purge_takes_an_id_a_yes_and_a_profile` en `cli.rs`; `purge_needs_yes_before_it_opens_anything`, `a_purge_refused_by_the_daemon_says_to_stop_it` y `the_purge_report_names_backups_to_delete_by_hand` en `crates/clawg-cli/src/secrets.rs`.
- [x] GREEN: el comando en `main.rs`; la confirmación y los mensajes, puros.
- [x] Commit: `feat(cli): add clawg purge` (`6e2e05b`)

### 8. Documentación
- [x] `.claude/CLAUDE.md`: `clawg secrets` y `clawg purge` en "Commands", y el enmascarado junto a la API. `README.md` no documenta los comandos de mantenimiento (tampoco `backup` ni `restore`), así que no cambia.
- [x] Commit: `docs: document clawg secrets and clawg purge` (`80c4a0c`)

## Verificación

- [x] `cargo test --workspace --no-fail-fast` en Linux: conteo contra la base, y cada commit en un worktree limpio. Ver `verify.md`.
- [x] Prueba de punta a punta en Linux con el binario debug y un `HOME` temporal (nunca `~/.clawg*`): `clawg secrets` con el daemon andando, `clawg stop`, `clawg purge` sin y con `--yes`, y `grep` del secreto en el directorio de datos. El save con una key no se pudo hacer con el binario (el modelo no se descarga desde este entorno); queda en los pendientes del perfil dev.
- [x] `cargo test --workspace --target-dir <otro-dir>` en Windows: 1.655 pasan, 0 fallan ([`verify.md`](verify.md#windows-2026-10-14)).
- [x] `rag-eval --real --diff` y `--agent-view` contra el baseline, en Windows ([`verify.md`](verify.md#rag-eval)).
- [x] Prueba en el perfil dev con Claude Code: el save con una key falsa avisa `masked github_token`, y `clawg secrets` sobre la base real no encuentra filas, así que no hubo nada que purgar ([`verify.md`](verify.md#perfil-dev-con-claude-code-2026-10-14)).

## Archivo

- [x] Pendientes de `verify.md` marcados con su resultado. La base de tests de `.claude/CLAUDE.md` (1.655) no cambia.
- [x] `git mv openspec/changes/secret-hygiene openspec/changes/archive/2026-10-14-secret-hygiene`.
- [x] `openspec/specs/secrets/spec.md` con R1 a R6, las rutas y líneas del código y la línea **Origen**. En `openspec/specs/ingest/spec.md`, el campo `redacted` en «Un save confirmado devuelve los ids», apuntando a `specs/secrets`.
- [x] Enlaces relativos de los archivos movidos corregidos (`../../../specs/`).
- [x] `state.yaml`: `phase: archived`, `verify: done`, `archive: done`, `main_spec: openspec/specs/secrets/spec.md`.
- [x] `openspec/README.md`: fila 13 como archivada, con enlace a `specs/secrets`.
- Commit: `docs(openspec): archive secret-hygiene after its Windows, rag-eval and dev profile checks`
