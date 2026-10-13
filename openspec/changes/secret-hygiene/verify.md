# secret-hygiene — Verificación

**Código:** rama `claude/secret-hygiene-sdd-feo162`, un commit por tarea.
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`). Cada test se vio fallar antes de su código (por no compilar o por la aserción).
**Estado:** verificado en Linux; R1 a R6 pasan. Quedan pendientes en la máquina del usuario la suite de Windows, `rag-eval --real` y la prueba en el perfil dev (ver «Pendiente»). No se archiva hasta entonces.

## Suite

`cargo test --workspace --no-fail-fast` en Linux, cada commit en un worktree limpio:

| Commit | Pasan | Fallan |
|---|---|---|
| `1127b50` (base, `main`) | 1675 | 10 |
| `c3b93ec` docs(openspec): propose secret-hygiene | 1675 | 10 |
| `caf0cff` feat(core): mask known secret formats in text | 1682 | 10 |
| `2dce41f` feat(core): mask secrets before a save is chunked, hashed or embedded | 1685 | 10 |
| `584b83f` feat(cli): tell the saver which secrets were masked | 1688 | 10 |
| `a2272c7` feat(core): find stored memories that hold a secret | 1691 | 10 |
| `4b417c8` feat(cli): add clawg secrets | 1693 | 10 |
| `bfdd4a8` feat(core): purge one memory for good, with its vector and index terms | 1700 | 10 |
| `6e2e05b` feat(cli): add clawg purge | 1704 | 10 |
| `80c4a0c` docs: document clawg secrets and clawg purge | 1704 | 10 |

- **+29 tests**, todos de este cambio. El conteo nunca baja.
- **Los 10 que fallan son los mismos 10 de `main`** en todos los commits (comparados por nombre): `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera).
- **La base de 1691 de `.claude/CLAUDE.md`** es de Windows; en Linux la suite tiene esos 10 fallos de plataforma.
- **Warnings y clippy:** `cargo clippy -p clawg-core -p clawg-cli --all-targets` no da avisos en el código nuevo. Los warnings del build son los mismos de `main`.
- **`regex`:** pasa a dependencia directa de `clawg-core` con la versión que ya estaba en `Cargo.lock` (1.12.3); el lock sólo suma `regex` a las dependencias de `clawg-core`, sin paquetes nuevos.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Detector puro | `every_known_secret_format_is_masked_in_place`, `ordinary_identifiers_and_hashes_are_left_alone`, `a_url_keeps_everything_but_its_password`, `a_private_key_without_its_end_line_is_masked_to_the_end`, `masking_twice_finds_nothing_new`, `an_anthropic_key_is_never_reported_as_an_openai_one`, `a_kind_found_twice_is_reported_once` | ✓ |
| R2. La ingesta enmascara | `a_saved_secret_is_stored_masked_and_not_searchable`, `saves_that_differ_only_in_a_masked_secret_are_one_memory`, `the_report_names_what_was_masked_even_when_the_save_fails` | ✓ |
| R3. El save avisa | `a_save_reports_the_secrets_it_masked` (`tests/api.rs`, por el router HTTP real), `a_confirmed_save_lists_the_kinds_it_masked`, `a_masked_save_tells_the_agent_what_was_masked` | ✓ |
| R4. `clawg secrets` | `the_scan_finds_live_and_deleted_rows_without_their_secrets`, `the_scan_reads_while_another_process_holds_the_store`, `the_scan_of_a_missing_store_creates_nothing`, `the_secrets_report_lists_rows_and_never_their_content`, `secrets_takes_only_a_profile` | ✓ |
| R5. `clawg purge` | `purging_a_live_row_leaves_no_trace_of_its_text`, `purging_a_deleted_row_leaves_no_trace_of_its_text`, `purging_releases_the_rows_that_disputed_it`, `purge_refuses_while_the_daemon_holds_the_store`, `purge_refuses_an_unknown_id_untouched`, `purge_refuses_a_store_with_pending_migrations_untouched`, `purge_needs_yes_before_it_opens_anything`, `purge_takes_an_id_a_yes_and_a_profile`, `a_purge_refused_by_the_daemon_says_to_stop_it` | ✓ |
| R6. Backups que tienen la fila | `purge_names_the_backups_that_still_hold_the_row`, `the_purge_report_names_backups_to_delete_by_hand` | ✓ |

## Prueba de punta a punta (Linux, binario debug)

Con `HOME` en un directorio temporal del scratchpad, perfil dev. No se tocó ningún `~/.clawg*` real. El modelo no se puede descargar desde este entorno, así que el daemon quedó con el embedder en `loading`: el enmascarado de un save no se pudo probar con el binario (lo cubre `a_save_reports_the_secrets_it_masked` con el router HTTP real y un embedder de prueba). Las filas con secretos se escribieron con SQL directo sobre la base del daemon, como quedarían de una versión anterior: 200 filas comunes, una viva con una key AWS (#201) y una borrada con un JWT (#202).

| Paso | Resultado |
|---|---|
| `clawg secrets` con el daemon andando | `memory 201  aws_access_key  live` y `memory 202  jwt  deleted`, sin caracteres de los secretos; salida 0 |
| `clawg purge 201` | "clawg purge removes memory #201 for good … Run it again with --yes to do it", salida 1 |
| `clawg purge 201 --yes` con el daemon | "in use by the clawg daemon (pid 6888); run `clawg stop` first, then purge", salida 1 |
| `clawg backup` | snapshot `manual` por el daemon |
| `clawg stop` | ver «Lo que encontró la verificación»; el proceso salió unos segundos después |
| `grep` de la key en `memory.db` | 1 |
| `clawg purge 201 --yes` | `purged memory #201` y el backup manual como que todavía la tiene; salida 0 |
| `clawg purge 999 --yes` | `there is no memory #999`, salida 1 |
| `grep` de la key después | 0 en `memory.db` (no quedan `-wal` ni `-shm`), 1 en el backup, que no cambió |
| `clawg purge 202 --yes` (la borrada) | purgada, con el mismo backup listado; 0 ocurrencias del JWT en `memory.db` |
| `clawg secrets` | "no stored memory holds a secret of a known format" |
| Base después | 200 filas, `integrity_check` = `ok`, el FTS encuentra las 200 |
| `clawg serve` de nuevo y `clawg memories stats` | arranca; `200 live rows … 0 deleted` |

## Lo que encontró la verificación

- **`secure_delete` no alcanzaba para la purga.** El test de bytes sobre una base real encontró el secreto en una página de la lista libre, liberada por un merge del FTS mucho antes de purgar. La purga hace `VACUUM` (design D5). Sin `optimize` o sin `VACUUM`, los dos tests de bytes fallan.
- **`release_challengers` antes del `DELETE` no liberaba nada:** la fila todavía contaba como viva. Lo encontró `purging_releases_the_rows_that_disputed_it`; va después.
- **`clawg stop` con el modelo cargando** salió con 1 a los 15 s, y el proceso terminó unos segundos después. Es el problema conocido del backlog del README (la carga del embedder en `spawn_blocking`); no lo toca este cambio.
- **`rag-eval`:** el enmascarado sólo cambia textos con un patrón de secreto. El golden set no tiene ninguno, así que no debería moverse ninguna métrica; falta medirlo.

## Pendiente (máquina del usuario)

- [ ] `cargo test --workspace --target-dir <otro-dir>` en Windows: sin fallas nuevas contra la base de `.claude/CLAUDE.md` (los 5 de `project_root.rs` ya están arreglados). Los tests de purga usan `VACUUM` y comparan bytes de `memory.db`; conviene confirmarlos ahí.
- [ ] `rag-eval --real --diff` contra el baseline: R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979 y nDCG@10 0,6693 idénticos.
- [ ] Perfil dev, **con autorización** y con una copia de `memory.db` antes: un `save_memory` desde Claude Code con una key falsa (por ejemplo `ghp_` y 36 caracteres) dice `masked github_token`; `clawg secrets` sobre la base real; si hay filas, `clawg stop` y `clawg purge <ID> --yes` de una, y `clawg memories stats` igual salvo esa fila.
