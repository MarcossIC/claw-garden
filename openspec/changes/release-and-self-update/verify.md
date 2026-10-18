# release-and-self-update — Verificación

## Windows (2026-10-18)

Sobre `c9f1eb4` (0.20.0), en `target-test`, que se había limpiado y reconstruido antes y se reusó sin limpiar:

- `cargo test --workspace --target-dir target-test`: pasa, ningún test falla. El único ignorado es `the_vectors_are_the_ones_fastembed_gave` (`crates/clawg-core/tests/e5_golden.rs`), que pide el modelo real y no es de este cambio.
- `cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`: sale con 0.
- `the_agent_contract_matches_its_file` pasa sin `CLAWG_BLESS`. Ninguna tarea del cambio toca `agent_contract.json` ni `protocol.md`.
- Después de la suite no queda vivo ningún `clawg.exe` de `target-test` ni de un home temporal. Los únicos que corren son los del binario instalado, que no se tocaron.
- Las 38 tareas y M1 están marcadas, y cada test que nombran la [spec](spec.md) y las [tareas](tasks.md) existe y pasa.

Además, sobre el mismo árbol:

- `cargo test -p clawg-core --lib --target-dir target-test build_info`: pasa, con la variante sin `testing` de `update_overrides_are_read_only_in_a_test_build`, que la suite no compila (W3).
- `cargo check -p clawg-cli --target-dir target-test` compila las features que publica `release.yml`, sin `testing` ni `eval`. Sólo da el warning de `ImportanceFactor::spanning` (`crates/clawg-core/src/memory/score.rs:90`), que ya estaba antes de este cambio (lo anotó el [verify de topic-revisions](../archive/2026-10-18-topic-revisions/verify.md#linux-2026-10-18)). A partir de ahora va a salir en el log de cada release.
- `cargo tree -p clawg-cli --target x86_64-unknown-linux-gnu -e normal -i openssl-sys` (también con `native-tls` y `openssl`) no imprime nada. `rustls-native-certs` entra por `hyper-rustls` y `reqwest`, y `-e features -i clawg-core` sólo muestra `default`.
- En `crates/*/src`, cada `CLAWG_TEST_` es un `const` bajo `cfg(any(test, feature = "testing"))` (`paths.rs:169`, `build_info.rs:44` y `:49`, `memory/bootstrap.rs:295`). El humo `grep -aq 'CLAWG_TEST_'` de `release.yml` no tiene qué encontrar.

## Contra GitHub, sin escribir nada (2026-10-18)

- Cada acción fijada en `release.yml` resuelve al SHA que dice su comentario (`git ls-remote`): checkout v7.0.1, upload-artifact v7.0.1, download-artifact v8.0.0, attest-build-provenance v4.1.0 y la rama `stable` de dtolnay/rust-toolchain.
- El tarball de minisign 0.12 tiene el sha256 de `release.yml:157` y trae `minisign-linux/x86_64/minisign`, la ruta que el paso agrega al PATH.
- El environment `release` existe, limitado a `main` por una política de ramas, que es su única regla: no pide revisores. Sin autenticarse no se pueden listar sus secretos.
- `GET /repos/MarcossIC/claw-garden/releases?per_page=30` contesta `200` con `[]`.
- Una descarga de `github.com/…/releases/download/…` con el `Accept: application/vnd.github+json` que manda `ReleaseClient::open` (`crates/clawg-cli/src/update/client.rs:168-175`) redirige con un 302 y baja el archivo. Se probó con el tarball de minisign.
- Las dos claves de `release/` se decodificaron: el algoritmo es `Ed` y el key id coincide con el de su comentario (`351FCDDC90C0D50D`, `6F9927FDCEFE2FCB`).

| Requisito | Cómo quedó |
|---|---|
| R1 | `the_manifest_carries_the_publication_metadata` |
| R2 | `reqwest_uses_rustls_with_the_system_roots` y el `cargo tree` sin OpenSSL; el `ldd`, tras el merge |
| R3 | `the_release_workflow_builds_only_the_cli_with_the_dist_mark`; el CLI se compila sin `testing` ni `eval` |
| R4 | `the_release_workflow_publishes_only_for_a_successful_push_to_main`; los casos del tag existente y del fork, tras el merge |
| R5 | `the_release_workflow_publishes_last`; los cinco archivos y la atestación, tras el merge |
| R6 | `the_embedded_key_list_has_the_primary_and_backup_keys_and_not_the_test_key`, `the_release_keys_are_two_distinct_minisign_public_keys` y `the_signing_step_reads_the_key_password_from_stdin_without_echo`; `minisign -Vm`, tras el merge |
| R7 | `third_party_actions_are_pinned_to_a_commit` y los SHA comprobados contra sus tags; el humo `--version`, tras el merge |
| R8, R9 | A mano en un Windows limpio, pendiente |
| R10 | `the_readme_documents_the_install_and_update_path`; `.claude/CLAUDE.md` describe `update` en Commands, el release en Versioning y el sello en Testing |
| S1 | `version_parse_and_order_cases` y `check_outcome_cases` |
| S2 | `update_eligibility_cases` (S1 de las sugerencias) |
| S3 | `release_choice_and_asset_cases` |
| S4 | `sha256sums_parse_cases` y `trusted_comment_cases` |
| S5 | `signature_cases`, con el juego firmado por el CLI de minisign; `a_signature_by_an_untrusted_key_never_reaches_the_binary` y `hashes_signed_for_another_version_never_reach_the_binary` (W2) |
| S6 | `update_check_reports_a_newer_version_and_an_up_to_date_one`, `requests_carry_a_user_agent`, `the_listing_is_asked_with_the_page_size_and_returned_as_text` y `update_takes_check_and_a_profile`. Sólo el brazo `Update` de `main.rs` llega a `update::` |
| S7 | `a_hash_mismatch_never_touches_the_installed_binary`, `a_failed_download_or_signature_leaves_everything_as_it_was`, `a_binary_that_does_not_match_its_hash_leaves_no_file`, `the_signed_hashes_come_first_and_the_binary_last` y `an_unwritable_install_dir_fails_before_downloading` |
| S8 | `plan_order_with_and_without_daemon`, `update_without_a_daemon_does_not_start_one`, `two_updates_do_not_swap_at_once`, `a_full_update_with_a_daemon_replaces_the_binary_and_a_bridge_reaches_the_new_daemon` y `watch_cases` (`judge`, `leftovers` y los nombres) (W4, S3) |
| S9 | `a_new_binary_that_does_not_start_brings_the_old_one_and_its_daemon_back`, `a_new_binary_that_migrated_and_failed_gets_its_pre_migration_restored`, `each_failed_step_undoes_what_it_did`, `a_store_the_old_binary_cannot_open_is_restored_before_the_old_binary_returns`, `a_failed_restore_leaves_the_new_binary_and_explains_the_way_back` y `restoring_over_a_store_migrated_by_a_newer_version_keeps_it_as_pre_restore` (W1) |
| S10 | `an_update_over_a_pending_migration_leaves_a_pre_migration_snapshot_and_the_data` (base V15 sembrada por SQL), `an_update_whose_candidate_refuses_the_store_leaves_the_old_binary_and_its_daemon`, `a_cap_hides_the_migrations_past_it` y `a_runner_with_a_cap_migrates_no_further_than_it` |
| S11 | `the_update_report_names_versions_snapshot_and_restart_notice` y los demás tests de `report.rs` |
| S12 | `only_dev_dependencies_turn_on_the_test_stand_ins`, `update_overrides_are_read_only_in_a_test_build` (W3), `a_test_build_reads_its_version_from_the_stamp`, `update_refuses_a_binary_without_a_stamp` y `trailing_bytes_after_the_executable_image_do_not_change_execution`, en PE; en ELF, en el CI de Linux |
| I1 | `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms`; las líneas que cita (`ci.yml:5-26`, `:39`, `:41`, `:53`, `:56`) son las de hoy |

## Lo que encontró la verificación

### CRITICAL

Ninguno.

### WARNING

- **W1. Para volver a levantar el daemon viejo, `update` espera la versión del daemon que paró, no la del binario que arranca.** `Running.version` sale del `/health` del daemon que corría (`crates/clawg-cli/src/update/mod.rs:136`), y `Undo::StartOldDaemon` lanza el binario instalado esperando esa versión (`crates/clawg-cli/src/update/effects.rs:326-330`). Si las dos difieren, `judge` (`plan.rs:202-220`) no da `Ready`: el hijo contesta con su pid y otra versión, tiene el store y entonces le toca `PREPARE_WAIT`. Así, `update` espera 600 s, se rinde con `the daemon did not answer within 600 s` y el informe dice `Its daemon is not running` con el daemon corriendo, porque ese hijo no se termina. Las dos versiones difieren, por ejemplo, en Linux después del `install -m 755` del README, que reemplaza el archivo con el daemon andando. El problema aparece sólo si además la actualización falla después de parar el daemon. Ningún test cubre ese caso: en todos, el daemon y el binario instalado tienen la misma versión. Se arregla esperando la versión del binario que corre `update` (`installed` en `mod.rs:102`).
- **W2. `a_legacy_signature_is_rejected` no detecta un cambio de `allow_legacy`.** El test cambia `ED` por `Ed` en una firma *prehashed* y sólo exige `Rejected(_)` (`crates/clawg-cli/src/update/signature.rs:112-120`). En minisign-verify 0.2.5, con `allow_legacy = false` esa firma falla como `UnexpectedAlgorithm` (`src/lib.rs:365-366`), y con `true` falla igual, como `InvalidSignature`, porque verifica ed25519 sobre el contenido. Ningún test fija el `false` de `signature.rs:48`. El escenario de S5 «una firma legada del mismo archivo falla» queda cubierto a medias. Alcanza con exigir el texto de `UnexpectedAlgorithm` («Unexpected signature algorithm»).
- **W3. La suite y el CI nunca compilan el lado de producción de `build_info`.** Con `cargo test --workspace` y `cargo clippy --workspace --all-targets`, la dev-dependency de clawg-cli enciende `testing` en todo el build. Por eso la variante `cfg(not(feature = "testing"))` de `update_overrides_are_read_only_in_a_test_build` (`crates/clawg-core/src/build_info.rs:295-306`), el escenario «build de producción» de S12, sólo corre con `cargo test -p clawg-core`: los dos binarios de test de clawg-core tienen hashes distintos y la corrida de workspace tiene más tests. Tampoco se compilan en CI los cuerpos sin `testing` de `build_info`, ni clawg-cli contra ellos. El primer lugar que lo hace es `release.yml`. Un error que sólo exista en esa configuración pasaría el CI y frenaría el release, sin publicar nada. Hoy compila (`cargo check -p clawg-cli`, arriba).
- **W4. La rama de D4 que para a otro daemon y reintenta no tiene test.** `start_and_wait` (`effects.rs:154-182`) termina al hijo perdedor, para con `stop_in` al que ganó el store y reintenta una vez. `f1cb486` cambió ese orden sin test de regresión: la carrera necesita un daemon arrancado desde afuera a mitad de la actualización, y la suite no tiene dónde provocarlo. Sólo se prueba el veredicto `Other` de `judge` (`another_version_answering_is_not_the_child`). El escenario de S8 «un editor abre un puente entre la parada y el arranque» queda cubierto sólo en su parte pura.

### SUGGESTION

- **S1.** En `run`, `debug = cfg!(debug_assertions) && build_info::dist_target().is_none()` (`mod.rs:94`) relaja la negativa por build debug para cualquier build con marca, no sólo para el sello de prueba. Un build debug sin `testing` compilado a mano con `CLAWG_DIST_TARGET` pasaría la primera negativa de S2 y, fuera de un target de cargo, actualizaría el perfil `dev`. `release.yml` no llega a ese caso. Un `build_info::stamped()` limitaría la excepción al sello.
- **S2.** Ningún test decodifica las claves embebidas con `PublicKey::decode`: `the_release_keys_are_two_distinct_minisign_public_keys` revisa el formato como texto. `verify` decodifica todas las claves antes de mirar la firma (`signature.rs:41-44`), así que con una sola clave embebida mal formada cada actualización fallaría con `BadKey`. El workflow verifica con el CLI de minisign y no lo notaría. Las dos claves de hoy decodifican bien (arriba).
- **S3.** Dos caminos de `Installation` sólo tienen la prueba pura: la actualización sin daemon (S8, `update_without_a_daemon_does_not_start_one`) y el borrado de `.old-*` y `.new-*` de corridas anteriores (`remove_leftovers`, `mod.rs:146-156`, probado por `leftovers`). Ningún escenario de `binary.rs` corre dos actualizaciones seguidas ni una sin daemon.
- **S4.** Cuando otro daemon gana el store por segunda vez, el mensaje dice «run `clawg stop` and then any clawg command» (`effects.rs:171-177`), pero el plan deshace el intercambio y el binario vuelve a ser el viejo. Eso es lo que pide S9; D4 decía que quedaba el nuevo. Después de la vuelta atrás, ese consejo levanta otra vez el daemon viejo. Queda mejor decir que se cierre el editor y se repita `clawg update`.
- **S5.** `install.ps1` pide el binario y `SHA256SUMS` en dos pedidos separados a `releases/latest/download/` (`scripts/install.ps1:168-169`). Si se publica un release entre los dos, el hash no coincide y el script aborta sin tocar nada, que es el lado seguro. Resolver el tag una vez evitaría ese falso error.

## Diferencias con el diseño

- **D4, segundo `Other`.** El código deshace el intercambio en vez de dejar el binario nuevo con un daemon viejo. Sigue S9, que manda volver al binario viejo cuando el nuevo no contesta con su versión (S4 de las sugerencias).
- **D5.** `apply` es genérico (`E: Effects`), no `&mut dyn Effects`: el rasgo devuelve `impl Future`, y eso no admite `dyn`.
- **D3.** La línea de progreso dice `starting the daemon (pid N)`, no `starting the new daemon (pid N)`, porque sirve también para volver a levantar el viejo. En los informes de falla, «starting the new daemon» es el nombre del paso (`report.rs:150`).
- **Flujo.** La elegibilidad va antes del cerrojo, así que un binario rechazado no crea `update.lock`. `--check` no toma el cerrojo, como dice `.claude/CLAUDE.md`. Los restos de corridas anteriores se borran sólo cuando hay algo que instalar, como en el diagrama del diseño.
- **Archivos.** `update/lock.rs` y `update/testkit.rs` caben en `src/update/*.rs`. `.gitattributes` no estaba en la tabla: fija `release/*.pub` en LF para que `include_str!` dé lo mismo en Windows y deja `src/update/testdata/*` sin normalizar, porque son los bytes firmados.

## Pendiente

Según «Cierre» de las [tareas](tasks.md), lo de GitHub se anota después del merge, en un commit `docs(openspec)` aparte:

- Linux: la suite y clippy en el CI del PR (`test (ubuntu-latest)`). Es también la primera corrida de `trailing_bytes_after_the_executable_image_do_not_change_execution` sobre un ELF, la premisa de D11.
- La ejecución real de `release.yml`: que `--version` sea el tag, el `ldd` sin `libssl` y la glibc más alta que pide, `minisign -Vm SHA256SUMS` con `release/minisign.pub`, los cinco archivos y `gh attestation verify`.
- A mano, en un Windows limpio: `irm … | iex` (que también prueba el `param()` y el scope hijo de `616d017`), el aviso del VC++ Redistributable cuando falta, y `clawg update --check` diciendo «al día».
- A mano, un administrador de GitHub: revisar el primer release y los secretos del environment `release`.

No se puede comprobar después que cada test haya fallado antes de su código: cada test va en el mismo commit que el código que lo hace pasar, como piden las tareas.

## Veredicto

Verificado en Windows, contra el código, la spec, el diseño y las tareas. No hay ningún CRITICAL. Quedan cuatro WARNING (W1 es un error real en un camino de falla poco frecuente) y cinco SUGGESTION. Linux y los controles del release real se hacen después del merge.
