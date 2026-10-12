# secret-hygiene — Tareas

Cada tarea es un commit atómico (conventional commits) que compila y deja `cargo test --workspace` sin fallas nuevas. Dentro de cada una, primero el test que falla (RED), después el código (GREEN). En Windows: `cargo test --workspace --target-dir <otro-dir>`.

Los mensajes de commit son sugeridos. No mencionan planes, fases ni specs.

## Enmascarado

### 1. Detector puro — R1
- [ ] RED: `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `masking_twice_finds_nothing_new` y `an_anthropic_key_is_never_reported_as_an_openai_one` en un módulo nuevo, `crates/clawg-core/src/memory/redact.rs`.
- [ ] GREEN: `regex` como dependencia directa de `clawg-core` (`workspace = true`); `enum SecretKind` con `as_str`; `find_secrets(&str) -> Vec<Found>` y `redact(&str) -> Redacted { text, kinds }`, puras, con los patrones compilados una vez.
- [ ] Commit: `feat(core): mask known secret formats in text`

### 2. La ingesta enmascara — R2
- [ ] RED: `a_saved_secret_is_stored_masked_and_not_searchable`, `saves_that_differ_only_in_a_masked_secret_are_one_memory` y `the_report_names_what_was_masked_even_when_the_save_fails` en `worker.rs`.
- [ ] GREEN: `process_event` aplica `redact` antes de `select_chunker`; `IngestReport.redacted: Vec<SecretKind>`, en todos los caminos de salida.
- [ ] Commit: `feat(core): mask secrets before a save is chunked, hashed or embedded`

### 3. El save avisa — R3
- [ ] RED: `a_save_reports_the_secrets_it_masked` en `crates/clawg-cli/tests/api.rs`, los unit de `save_response` con y sin tipos, y `a_masked_save_tells_the_agent_what_was_masked` en `mcp/server.rs`.
- [ ] GREEN: `SaveBody.redacted` (omitido si está vacío); `SaveOutcome::Saved.redacted` en `client.rs` (con `#[serde(default)]`, para un daemon anterior); `save_outcome_text` lo nombra.
- [ ] Commit: `feat(cli): tell the saver which secrets were masked`

## Lo ya guardado

### 4. Escaneo — R4
- [ ] RED: `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_reads_while_another_process_holds_the_store` y `the_scan_of_a_missing_store_creates_nothing` en un módulo nuevo, `crates/clawg-core/src/memory/secrets.rs`.
- [ ] GREEN: `scan(base_dir) -> Result<Vec<Flagged>, MemoryError>` con `open_for_probe` (pasa a `pub(crate)`); `Flagged { id, kinds, deleted }`.
- [ ] Commit: `feat(core): find stored memories that hold a secret`

### 5. `clawg secrets` — R4
- [ ] RED: `secrets_takes_only_a_profile` en `cli.rs`, y `the_secrets_report_lists_rows_and_never_their_content` en un módulo nuevo, `crates/clawg-cli/src/secrets.rs`.
- [ ] GREEN: el comando en `main.rs` y el reporte puro.
- [ ] Commit: `feat(cli): add clawg secrets`

### 6. Purga — R5, R6
- [ ] RED: `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched`, `purge_refuses_a_store_with_pending_migrations_untouched` y `purge_names_the_backups_that_still_hold_the_row` en `secrets.rs`.
- [ ] GREEN: `purge(base_dir, id) -> Result<Purged, MemoryError>` con los pasos de R5 (D4, D5); `PurgeError` con `thiserror` y `MemoryError::Purge`; `Purged { backups }` (D6). `StoreHolder::Maintenance` nombra la purga.
- [ ] Commit: `feat(core): purge one memory for good, with its vector and index terms`

### 7. `clawg purge` — R5, R6
- [ ] RED: `purge_takes_an_id_a_yes_and_a_profile` en `cli.rs`; `purge_needs_yes_before_it_opens_anything`, `a_purge_refused_by_the_daemon_says_to_stop_it` y `the_purge_report_names_backups_to_delete_by_hand` en `crates/clawg-cli/src/secrets.rs`.
- [ ] GREEN: el comando en `main.rs`; la confirmación y los mensajes, puros.
- [ ] Commit: `feat(cli): add clawg purge`

### 8. Documentación
- [ ] `.claude/CLAUDE.md`: `clawg secrets` y `clawg purge` en "Commands". `README.md` si lista los comandos.
- [ ] Commit: `docs: document clawg secrets and clawg purge`

## Verificación

- [ ] `cargo test --workspace --no-fail-fast` en Linux: conteo contra la base, y cada commit en un worktree limpio. Ver `verify.md`.
- [ ] Prueba de punta a punta en Linux con el binario debug y un `HOME` temporal (nunca `~/.clawg*`): un save con una key, `clawg secrets`, `clawg stop`, `clawg purge` sin y con `--yes`, y `grep` del secreto en el directorio de datos.
- [ ] Suite en Windows, `rag-eval --real` y prueba en el perfil dev: pendientes en la máquina del usuario (ver `verify.md`).

## Archivo (después de los pendientes de Windows y `rag-eval`)

- [ ] Marcar los pendientes de `verify.md` con su resultado; si cambia la base de tests, actualizarla en `.claude/CLAUDE.md`.
- [ ] `git mv openspec/changes/secret-hygiene openspec/changes/archive/AAAA-MM-DD-secret-hygiene`.
- [ ] Crear `openspec/specs/secrets/spec.md` con R1 a R6 y las rutas y líneas del código en ese momento, con la línea **Origen**. En `openspec/specs/ingest/spec.md`, agregar el campo `redacted` al contrato del save, apuntando a `specs/secrets`.
- [ ] Corregir los enlaces relativos de los archivos movidos (un nivel más: `../../../specs/`).
- [ ] `state.yaml`: `phase: archived`, `verify: done`, `archive: done`, `main_spec: openspec/specs/secrets/spec.md`.
- [ ] `openspec/README.md`: fila 13 como archivada, con enlace a `specs/secrets`.
- [ ] Commit: `docs(openspec): archive secret-hygiene`
