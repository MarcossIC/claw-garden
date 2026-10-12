# storage-safety-net — Verificación (partes B + C)

**Código:** rama `claude/storage-sdd-xv8bd2`, un commit por tarea (más la corrección 9b y un ajuste de tests).
**Modo:** Strict TDD (`openspec/config.yaml`: `strict_tdd: true`).
**Estado:** verificado en Linux, con pendientes en Windows. R1 a R8 tienen sus tests en verde y se probaron de punta a punta con el binario debug. R9 (instalador) y la prueba manual en Windows quedan pendientes: este entorno no tiene PowerShell ni Windows. El cambio queda en verify hasta que pasen; los pasos para archivarlo están en `tasks.md`.

## Suite

`cargo test --workspace --no-fail-fast` en Linux:

| | Pasan | Fallan |
|---|---|---|
| `main` (`deb5dc5`) | 1551 | 10 |
| rama (`82c0d7c`, última tarea) | 1588 | 10 |
| rama con `main` mergeado (`99d04c5`) | 1622 | 10 |

- **+37 tests**, todos de este cambio. Con 9b, 1589 (+38).
- **Con `main` mergeado** (`99d04c5`, que ya trae la V9 de project-identity, PR #9): 1622 pasan y fallan los mismos 10. Los tests que fijan las migraciones (`published_migration_checksums_are_pinned` con la fila de V9, y los de conteo de `db.rs`) pasan; los de este cambio leen la última versión del binario (`latest_known`) y también pasan con V9. Una base en V8 abierta con este binario hace su snapshot `pre-migration` antes de aplicar la V9, que es la red que project-identity pedía para su re-key.
- **Los 10 que fallan, fallan igual en `main`.** Son de plataforma o de archivos fuera de git: `doctor::tests::*` (7, sólo pasan en Windows) y `rag-eval` `diff::tests::*` (3, leen `docs/eval/results/`, que `.gitignore` deja afuera). Ninguno toca este cambio.
- **La base de 1566 de `.claude/CLAUDE.md`** es de Windows. En Linux la suite tiene esos 10 fallos de plataforma.
- **Warnings:** los mismos que en `main`. `cargo clippy -p clawg-core -p clawg-cli --all-targets` no da avisos en lo que agregó el cambio; los dos más cercanos (`db.rs:130`, `transmute` sin anotar, y `runtime.rs:348`) son anteriores.
- **`rag-eval`:** no aplica, no cambia ranking ni recuperación.

### Atomicidad

Cada commit se probó en un worktree limpio con `cargo test --workspace --no-fail-fast`:

| Commit | Pasan | Fallan |
|---|---|---|
| `b3325ab` docs(openspec): propose … | 1551 | 10 |
| `f2a6799` feat(core): decide the schema plan … | 1554 | 10 |
| `3aafd4e` feat(core): refuse a newer or diverged database … | 1556 | 10 |
| `ac4a02b` feat(core): write consistent snapshots … | 1561 | 10 |
| `4dd3f1a` feat(core): keep only the newest automatic snapshots | 1563 | 10 |
| `0ef7468` feat(core): snapshot the store before applying pending migrations | 1567 | 10 |
| `f47d90c` feat(core): back up a stopped store without migrating it | 1570 | 10 |
| `b33007b` feat(cli): add clawg backup | 1572 | 10 |
| `d360a13` feat(core): restore the store from a snapshot … | 1577 | 10 |
| `8b40e51` feat(cli): add clawg restore | 1581 | 10 |
| `669e793` feat(cli): show the store's schema state in clawg status … | 1587 | 10 |
| `a9efb65` fix(cli): point at clawg status when the daemon could not start | 1588 | 10 |
| `82c0d7c` fix(install): abort when the daemon is still running … | 1588 | 10 |
| `12d3738` fix(core): name a missing store and restore onto a new data directory | 1589 | 10 |
| `373724e` docs: document clawg backup, clawg restore … | 1589 | 10 |
| `f1e1edf` docs(openspec): record the storage-safety-net apply … | 1589 | 10 |
| `f5d47c2` test(core): derive the newest migration instead of assuming V8 | 1589 | 10 |

En todos, los 10 que fallan son los mismos 10 de `main`. El conteo nunca baja.

## Requisitos

| Req | Test(s) | Resultado |
|---|---|---|
| R1. Plan de esquema antes de migrar | `schema_plan_covers_every_history_shape`, `a_divergence_the_binary_knows_about_wins_over_newer`, `history_order_does_not_matter`, `a_newer_database_is_refused_untouched`, `a_diverged_database_is_refused_untouched` | ✓ |
| R2. Snapshot antes de migrar | `opening_an_older_database_snapshots_it_before_migrating`, `opening_a_current_database_takes_no_snapshot`, `a_new_database_takes_no_snapshot`, `a_failed_snapshot_stops_the_migration` | ✓ |
| R3. Snapshot completo | `a_snapshot_includes_writes_still_in_the_wal`, `a_snapshot_keeps_vectors_and_full_text_searchable`, `snapshot_names_sort_in_time_order`, `snapshot_names_parse_back_to_their_reason`, `a_failed_snapshot_leaves_no_file_behind` | ✓ |
| R4. Retención | `pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one`, `an_automatic_snapshot_prunes_older_automatic_ones` | ✓ |
| R5. `clawg backup` | `backup_endpoint_writes_a_manual_snapshot`, `backup_takes_only_a_profile`, `an_offline_backup_copies_without_migrating`, `an_offline_backup_refuses_a_held_store`, `an_offline_backup_of_nothing_says_so` | ✓ |
| R6. `clawg restore` | `restore_replaces_the_store_and_keeps_the_previous_one`, `restore_refuses_files_it_cannot_open_safely`, `restore_refuses_while_the_daemon_holds_the_store`, `a_restored_older_snapshot_migrates_on_the_next_open`, `restoring_the_oldest_kept_automatic_snapshot_survives_the_pruning`, `restore_onto_a_machine_without_a_data_directory`, `a_store_held_by_the_daemon_says_to_stop_it`, `restore_takes_a_file_and_a_profile` | ✓ |
| R7. `clawg status` sin daemon | `a_stopped_daemon_reports_the_store_schema`, `a_store_that_would_open_carries_no_message`, `probing_a_newer_store_reads_without_writing`, `probing_without_a_store_creates_none`, `probing_a_file_that_is_not_a_store_says_unreadable`, `immutable_uris_escape_what_a_uri_cannot_hold` | ✓ |
| R8. MCP nombra `clawg status` | `a_daemon_that_did_not_start_points_at_status`, `a_tool_call_when_the_daemon_never_comes_up_gives_the_readable_error` | ✓ |
| R9. Instalador | manual en Windows | **pendiente** |

## Prueba de punta a punta (Linux, binario debug)

Con `HOME` apuntando a un directorio temporal del scratchpad, perfil dev. No se tocó ningún `~/.clawg*` real. El modelo no se pudo descargar (el entorno no llega a Hugging Face), así que el daemon quedó con el embedder en `loading`; nada de esta prueba lo necesita.

| Paso | Resultado |
|---|---|
| `clawg backup` sin base | `clawg: there is no memory store at …/.clawg-dev/memory.db`, salida 1. La primera vez decía "memory store lock error … No such file or directory": de ahí sale la tarea 9b |
| `clawg status` sin base | `"store": {"state": "missing", "db_version": null, "binary_version": 8, "message": null}` |
| `clawg serve --port 0` y `clawg backup` | el backup pasó por el daemon: `…/backups/memory-2026-10-12T18-16-52.599Z-manual.db` |
| `clawg restore` con el daemon andando | `the memory store is in use by the clawg daemon (pid 4172); run \`clawg stop\` first, then restore`, salida 1 |
| `clawg stop`, `clawg backup` sin daemon | `…-manual.db` nuevo, sin migrar |
| `clawg restore <el primero>` | `restored …; the store it replaced is at …-pre-restore.db`. En el directorio sólo quedan `backups`, `memory.db`, `memory.lock` y `models`: ni temporales ni WAL |
| `clawg status` | `"state": "up_to_date"` |
| V9 agregada a mano al historial, `clawg status` | `"state": "newer"`, con el mensaje que nombra `clawg restore`. No aparecieron `-wal` ni `-shm` |
| `clawg serve` sobre esa base | `could not open the memory store: memory.db is at schema V9, newer than the V8 …`, salida 1. El sha1 de `memory.db` es el mismo antes y después |

## Lo que encontró la verificación

- **`clawg backup` sin directorio de datos** daba un error de lock que no decía qué pasaba, y `clawg restore` no podía mudar una base a una máquina nueva. Corregido en la tarea 9b, con tests.
- **El probe de `clawg status`**, en su primera versión con `SQLITE_OPEN_READ_ONLY`, dejaba `memory.db-wal` y `memory.db-shm` junto a una base parada. Lo detectó su test antes del commit; ahora abre con `immutable=1` (design D7).
- **Restaurar el automático más viejo** lo borraba la poda del `pre-restore` antes de copiarlo. Lo detectó la revisión del orden de D3; el archivo se copia antes del snapshot, y un test lo fija.
- **Coordinación con project-identity (PR #9, V9):** este cambio no agrega migraciones. Los tests nuevos leen la última versión del binario (`latest_known`) en vez de suponer V8, así que no se rompen cuando entre V9. La re-key de #9 queda cubierta por el snapshot `pre-migration` si va dentro de la migración.

## Pendiente (necesita Windows y autorización)

- [ ] **R9, instalador.** Con un daemon prod que no para (o con `clawg status` devolviendo 0), `scripts/install.ps1` tiene que abortar con "is still running after 'clawg stop', so the binary was not replaced" y dejar el binario anterior en su lugar. Y con el daemon parado, instalar como siempre.
- [ ] **Prueba manual en el perfil dev:** `clawg backup` con el daemon andando, `clawg stop`, `clawg restore` del archivo, arrancar y comparar el conteo de filas con `clawg memories stats`.
- [ ] **Suite en Windows** (`cargo test --workspace --target-dir <otro-dir>`), para actualizar la base de `.claude/CLAUDE.md`. En particular `immutable_uris_escape_what_a_uri_cannot_hold` y el probe sobre una ruta con letra de unidad.
