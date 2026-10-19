# local-isolation — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El ayudante (R6, D2, D3)

- [x] Tests primero (Unix): `a_missing_directory_and_its_parents_are_created_private`, `an_open_directory_is_tightened_and_the_rest_left_alone`, `a_private_directory_is_left_as_it_is`, `only_group_and_other_bits_are_taken`.
- [x] `ensure_private_dir` y `tightened` en `private_dir.rs`.
- Commit: `feat(core): add a helper that keeps a directory private to its owner`

## 2. Los lugares de `clawg-core` (R6, D3)

- [ ] Tests primero (Unix): `opening_a_store_makes_its_directory_private`, `a_snapshot_directory_is_created_private`, `restore_creates_a_private_data_directory`, `save_creates_a_private_data_directory`, `workspace_directories_are_created_private`, `the_model_cache_is_created_private`.
- [ ] `MemoryStore::open`, `snapshot_into`, `restore`, `sharing::save`, `workspace.rs` y `model_download.rs` usan el ayudante; `open` devuelve el error al crear.
- Commit: `fix(core): create the data directory and its subdirectories private to the owner`

## 3. Los lugares de `clawg-cli` (R6, D4)

- [ ] Tests primero (Unix): `the_update_lock_makes_the_data_directory_private`, `setup_copies_go_in_a_private_directory`; en `tests/binary.rs`, `a_bridge_on_a_fresh_home_leaves_a_private_data_directory` y `an_open_data_directory_is_tightened_and_keeps_its_rows`.
- [ ] `serve`, `start_daemon`, `spawn_serve_at`, `UpdateLock::acquire` y `setup` usan el ayudante.
- Commit: `fix(cli): create the data directory private to the owner from every entry point`

## 4. Docs

- [ ] README y `.claude/CLAUDE.md`; `state.yaml`.
- Commit: `docs: say the data directory is private to its owner on Unix`

## 5. Verificación

- [ ] Suite y clippy en Linux sobre el último commit.
- [ ] Con dos cuentas en Linux: la otra no lista el directorio, no lee `memory.db` y no abre `memory.lock`; antes del cambio sí.
- [ ] `verify.md`.

## 6. Archive

- [ ] `specs/daemon-access` con R6, el `Límite` y R5; `git mv` a `changes/archive/AAAA-MM-DD-local-isolation`; `state.yaml` archivado; fila 50 y orden de ejecución del `README.md` de openspec; subir el minor.
