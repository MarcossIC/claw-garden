# release-and-self-update — Tareas

Strict TDD: en cada tarea el test va primero y falla (por la aserción o porque no compila); el código que lo hace pasar va en el mismo commit. Cada tarea es un commit convencional que compila y pasa la suite: `cargo test --workspace --target-dir target-test` y `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`. Las tareas **M** son manuales y no son commits.

Tres tandas, en orden, en la misma rama. Ninguna llega sola a `main`: el PR llega con el archivo y el bump en el mismo push (sección «Cierre»).

Reglas de todo el cambio:

- Los módulos nuevos de `clawg-cli` son `pub` dentro de la biblioteca `clawg_cli`; un `pub fn` de una biblioteca no dispara `dead_code`. Lo que sea privado entra con su primer llamador. Ningún `#[allow]`.
- Los tests de `.github/workflows/*.yml` leen el texto por líneas, sin dependencia YAML, como `testing_outside_dev_dependencies` en `crates/clawg-cli/tests/conventions.rs` (diseño, D8).
- Hasta la tarea 18 la lista de claves embebida está vacía: un build sin `testing` rechaza toda firma. Nunca se commitea una clave de relleno para llenarla antes.
- Dependencias de `Cargo.lock` nuevas (reqwest/rustls, `minisign-verify`, `minisign`): después, `cargo clean --target-dir target-test`. Al empezar a probar la versión de este cambio: `cargo clean --workspace --target-dir target-test`. Dentro de un ciclo TDD, el directorio se reutiliza.
- Nunca: builds de release locales, instalar, tocar `~/.clawg*`, matar `clawg.exe`. El build de release ocurre sólo en `release.yml`.

## Tanda 1. Pipeline e instalación

- [x] 1. Test `the_manifest_carries_the_publication_metadata` (`conventions.rs`). `LICENSE` MIT («The Claw Garden contributors») y `license`, `repository`, `authors` en `[workspace.package]` de `Cargo.toml`.
  Commit: `chore: add the MIT license and publication metadata`
- [x] 2. Test `reqwest_uses_rustls_with_the_system_roots` (`conventions.rs`). `reqwest` de `Cargo.toml` raíz con `default-features = false` y las features de D9; `Cargo.lock`. Después `cargo clean --target-dir target-test`.
  Commit: `build: use rustls with the system roots for reqwest`
- [x] M1. **Manual (un administrador del repositorio en GitHub, con `minisign`):** generar los dos pares minisign fuera del repositorio; la privada principal y su contraseña, como secretos del environment `release` (limitado a `main`, sin revisores); la privada de respaldo, fuera de GitHub. De aquí salen `minisign.pub` y `minisign-next.pub`. Pueden avanzar sin M1, sólo con claves de prueba, las tareas 1, 2, 4 a 17 y 19 a 38; dependen de M1 las tareas 3 y 18 (y la ejecución real de la tanda). Nunca se commitea una clave de relleno.
- [x] 3. **Depende de M1.** Test `the_release_keys_are_two_distinct_minisign_public_keys` (`conventions.rs`, por líneas: comentario `untrusted comment:`, segunda línea de 56 caracteres que empieza con `RW`, las dos distintas). Se commitean `release/minisign.pub` y `release/minisign-next.pub`.
  Commit: `chore: add the release signing public keys`
- [x] 4. Test `third_party_actions_are_pinned_to_a_commit` y `the_release_workflow_publishes_only_for_a_successful_push_to_main` (`conventions.rs`; fallan: no existe `release.yml`). `.github/workflows/release.yml` con el disparador `workflow_run` y el job `plan` (D8: `event`, `conclusion`, `head_branch`, `head_repository`, tag existente, `concurrency`); SHAs resueltos con `gh api` al escribirlo, el tag en un comentario.
  Commit: `ci: add the release workflow trigger and plan job`
- [x] 5. Test `the_release_workflow_builds_only_the_cli_with_the_dist_mark` (R3). Job `build` (matriz `windows-2022` / `ubuntu-22.04`, `CLAWG_DIST_TARGET`, `cargo build --release --locked -p clawg-cli --target`), humo `--version`, ausencia de `CLAWG_TEST_`, `ldd` sin `libssl`/`libcrypto`.
  Commit: `ci: build and smoke-test the release binaries`
- [x] 6. Tests `the_release_workflow_publishes_last` (el `publish` depende de `plan` y `build`; `gh release create` sólo ahí; `environment: release` sólo ahí) y `the_signing_step_reads_the_key_password_from_stdin_without_echo` (`printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S …`, sin `set -x` ni `echo` del secreto, sin `-W`, `secrets.MINISIGN_*` sólo dentro de ese paso). Job `publish`: `SHA256SUMS`, firma, `minisign -Vm`, atestación, `gh release create`.
  Commit: `ci: sign and publish the release as the last step`
- [x] 7. Test `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` (I1; pasa hoy: fija el estado actual de `ci.yml`, sin secretos del environment `release`). Sin cambios a `ci.yml` salvo que el test lo exija.
  Commit: `test(cli): pin the CI workflow paths and platforms`
- [x] 8. `scripts/install.ps1` como bootstrap (R8, R9): baja el binario del último release, compara con `SHA256SUMS`, avisa del VC++ Redistributable sin instalar nada; con `clawg.exe` local instala ese. Sin test automático (no hay tests de `scripts/`); se verifica a mano en el checklist.
  Commit: `feat(scripts): install the latest release from install.ps1`
- [x] 9. Test `the_readme_documents_the_install_and_update_path` (`conventions.rs`) con la URL de `install.ps1`, la glibc mínima, SmartScreen, clave comprometida y verificación con `minisign`. `README.md` y `.claude/CLAUDE.md` (Versioning y CI con el release).
  Commit: `docs: document installing from a release`

## Tanda 2. `clawg update --check`

- [x] 10. **Primer RED, riesgo de D11.** Test `trailing_bytes_after_the_executable_image_do_not_change_execution` (`binary.rs`): una copia del binario con `0x00 CLAWG-TEST-STAMP {json}` al final se comporta igual (PE y ELF; corre en las dos plataformas de CI). Pasa en cuanto existe; es el control de la premisa. Si falla en una plataforma, se detiene la tanda y vuelve al diseño (D11).
  Commit: `test(cli): pin that bytes after the executable do not change execution`
- [x] 11. **Primer RED, riesgo de D1.** Dependencias `minisign-verify = "=0.2.5"`, `sha2` y `fs2` del workspace, dev `minisign = "=0.9.1"`; `cargo clean --target-dir target-test`. Test `signature_cases` (caso «prehashed del firmante de pruebas» con `allow_legacy = false`) en `src/update/signature.rs`, y `verify`. Si el firmante de pruebas firma en modo legado, se detiene y vuelve al diseño (D1, alternativas).
  Commit: `feat(cli): verify minisign signatures against a key list`
- [x] 12. Resto de `signature_cases`: segunda clave de la lista, clave fuera, archivo alterado, `.minisig` ajeno, legada rechazada, y el juego fijo firmado por el CLI de minisign (`src/update/testdata/`).
  Commit: `test(cli): cover signature rejection and the minisign CLI fixture`
- [x] 13. Test `version_parse_and_order_cases`. `src/update/version.rs`.
  Commit: `feat(cli): parse and order release versions`
- [x] 14. Test `release_choice_and_asset_cases` (JSON fijo: borrador, prerelease, tag mal formado, falta el asset y el error nombra el triple). `src/update/release.rs`.
  Commit: `feat(cli): choose the release and asset for a target`
- [x] 15. Tests `sha256sums_parse_cases` y el del comentario confiable de otra versión. `SHA256SUMS` y comentario en `release.rs`.
  Commit: `feat(cli): parse SHA256SUMS and bind its signature to a version`
- [x] 16. Tests de `build_info` en `clawg-core` (lectura del sello, `parse_stamp` pura) y `update_overrides_are_read_only_in_a_test_build` (`cfg`), más en `binary.rs` `a_test_build_reads_its_version_from_the_stamp`. `crates/clawg-core/src/build_info.rs` (`version()`, `dist_target()`, sello, sustituciones; dos cuerpos según `testing`); los siete sitios de `env!("CARGO_PKG_VERSION")` de D10 pasan a `version()`.
  Commit: `feat(core): read the build version and distribution mark from one place`
- [x] 17. Test `update_eligibility_cases` (cada negativa con su motivo; release marcado en `prod` pasa). `src/update/eligibility.rs` y la lectura de `.cargo-lock`.
  Commit: `feat(cli): decide whether a binary can update itself`
- [x] 18. **Depende de 3.** Test `the_embedded_key_list_has_the_primary_and_backup_keys_and_not_the_test_key` (`conventions.rs`, vía `clawg_cli::update::signature`). `include_str!` de las dos claves de `release/` y sustitución por `CLAWG_TEST_RELEASE_KEYS` sólo en `testing`.
  Commit: `feat(cli): embed the release public keys`
- [x] 19. Test del cliente contra un servidor axum falso: `requests_carry_a_user_agent`, topes de tamaño, orden (firma antes del binario). `src/update/client.rs`.
  Commit: `feat(cli): query the release server on request`
- [x] 20. Test de `cli.rs` para `update [--check]` y test del mapeo a salida `0`/`100`/`1` (puro). `Command::Update`, su brazo en `main.rs`, `update/mod.rs` con `--check`.
  Commit: `feat(cli): add clawg update --check`
- [x] 21. Tests `update_check_reports_a_newer_version_and_an_up_to_date_one` y `update_refuses_a_binary_without_a_stamp` (`binary.rs`, con el arnés: copia con sello en `<home>/bin`, servidor falso firmado al vuelo, `CLAWG_TEST_RELEASE_URL`/`CLAWG_TEST_RELEASE_KEYS`).
  Commit: `test(cli): drive update --check through the real binary`
- [x] 22. `.claude/CLAUDE.md`: variables de `testing` y sello de prueba (Testing). Se comprueba que `agent_contract.json` no cambia (`cargo test -p clawg-cli the_agent_contract_matches_its_file` sin bendecir).
  Commit: `docs: describe the update test stand-ins`

## Tanda 3. `clawg update`

- [x] 23. Tests de `paths::update_lock_file` y `StoreLock::recorded_owner` (no prueba el cerrojo). Código en `clawg-core`.
  Commit: `feat(core): expose the update lock path and the recorded store owner`
- [x] 24. Test `restoring_over_a_store_migrated_by_a_newer_version_keeps_it_as_pre_restore` (`backup.rs`; fija un contrato existente, puede pasar de entrada).
  Commit: `test(core): pin restoring over a store migrated by a newer version`
- [x] 25. Tests `plan_order_with_and_without_daemon` y `update_without_a_daemon_does_not_start_one`. `src/update/plan.rs`: `Step`, `Undo`, `plan`.
  Commit: `feat(cli): describe the update as a plan`
- [x] 26. Tests `each_failed_step_undoes_what_it_did`, `a_hash_mismatch_never_touches_the_installed_binary`, `a_failed_download_or_signature_leaves_everything_as_it_was`, el de `StopTimeout` sin renombrar. `apply` y `Effects` con falsos.
  Commit: `feat(cli): run the update plan and undo on failure`
- [x] 27. Test `a_failed_restore_leaves_the_new_binary_and_explains_the_way_back` y el de restore exitoso (D2).
  Commit: `feat(cli): restore the pre-migration snapshot when the new daemon migrated and failed`
- [x] 28. Tests de `judge` (cada señal y plazo, `Other`), `leftovers` y del diff de snapshots.
  Commit: `feat(cli): judge a starting daemon and pick leftovers`
- [x] 29. Tests `an_unwritable_install_dir_fails_before_downloading` y `the_update_report_names_versions_snapshot_and_restart_notice`. `src/update/report.rs`.
  Commit: `feat(cli): report the outcome of an update`
- [x] 30. Test `two_updates_do_not_swap_at_once` (cerrojo `fs2` sobre `update.lock`).
  Commit: `feat(cli): serialize updates with an exclusive lock`
- [x] 31. Extraer de `ProcessSpawner::spawn_serve` (`client.rs`) el arranque por ruta y puerto; el net son los tests existentes (`binary.rs`, `client.rs`).
  Commit: `refactor(cli): start the daemon from an explicit binary path`
- [x] 32. Tests: `migrations_up_to` filtra `known_migrations`/`set_target` bajo `testing`, y `serve_fault` `before_open`/`after_open` (unitarios en `build_info`/`db.rs`). Código en `build_info.rs`, `backup.rs`, `db.rs`, `daemon/serve.rs`.
  Commit: `feat(core): let a test build cap its migrations and fail on purpose`
- [x] 33. **RED por el binario real.** Test `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon`. `effects.rs` de producción y el cableado de `update/mod.rs`.
  Commit: `feat(cli): add clawg update`
- [x] 34. Test `a_new_binary_that_does_not_start_brings_the_old_one_and_its_daemon_back`.
  Commit: `feat(cli): bring the old daemon back when the new one does not start`
- [x] 35. **Base sembrada en un esquema anterior** (instalado con `migrations_up_to` fijo en 15, la versión anterior a V16; su daemon crea el store, se detiene y las filas se insertan por SQL con `rusqlite`, porque un daemon con tope usa el código de inserción actual y no puede guardarlas): `an_update_over_a_pending_migration_leaves_a_pre_migration_snapshot_and_the_data`.
  Commit: `test(cli): update over a store with a pending migration`
- [x] 36. Misma siembra: `a_new_binary_that_migrated_and_failed_gets_its_pre_migration_restored` (`serve_fault = after_open`); `Installation::restore_store` sondea el store con las migraciones del binario que corre y restaura el `pre-migration` de ese arranque.
  Commit: `feat(cli): restore the store when the new daemon migrated and failed`
- [x] 37. Siembra al día y candidato con `migrations_up_to = última - 1`: `an_update_whose_candidate_refuses_the_store_leaves_the_old_binary_and_its_daemon`.
  Commit: `test(cli): update with a candidate that refuses the store`
- [x] 38. `README.md` («Actualizar», vuelta atrás) y ampliación de `the_readme_documents_the_install_and_update_path`; `.claude/CLAUDE.md` (Commands `clawg update [--check]`, Testing). `MCP tools` y `agent_contract.json` sin tocar.
  Commit: `docs: document clawg update`

## Verificación (checklist, no tareas de apply)

- [x] Suite y clippy en Windows (`target-test`); `the_agent_contract_matches_its_file` sin cambios.
- [ ] Lo mismo en Linux (CI del PR).
- [ ] Tras el merge (el workflow sólo corre desde `main`): ejecución de `release.yml`; `--version` = tag; `ldd` sin `libssl`; `minisign -Vm SHA256SUMS` con la clave del repositorio; cinco archivos y atestación (`gh attestation verify`).
- [ ] **Manual (Windows limpio):** `irm … | iex`, aviso de VC++ sin él, `clawg update --check` dice «al día».
- [ ] **Manual (administrador de GitHub):** revisar el primer release y el environment `release`.
- [x] `verify.md` (con lo pendiente).

## Cierre

- [x] Archivar: specs `release`, `self-update` e `install` (I1 reemplaza «CI»); `git mv` al archivo; subir el minor (0.21.0).
  Commits: `docs(openspec): archive release-and-self-update` y `chore: bump version to 0.21.0`
- [x] Commit `chore: bump version to X.Y.Z` (`Cargo.toml` y `cargo update --workspace`). El PR llega a `main` con el archivo y el bump en el mismo push, nunca antes: ese commit dispara `release.yml`.
- [ ] Tras el merge: los controles del release real y el bootstrap manual en Windows se anotan en el `verify.md` archivado, en un commit `docs(openspec)` aparte. Si alguno falla, se arregla con un fix y un bump de patch, que publica el release siguiente.
