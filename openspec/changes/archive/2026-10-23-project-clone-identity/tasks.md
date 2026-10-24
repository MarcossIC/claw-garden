# project-clone-identity — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. La clave de una raíz (CI1)

- [x] Tests puros de `normalize_remote`, `origin_url` y `git_config_dir` (`identity.rs`), y los de `project_root.rs` con un worktree, un submódulo y una raíz sin `origin`.
- [x] `origin_key` y sus piezas.
- Commit: `feat(core): read the origin a project root was cloned from` (`75186b4`)

## 2. V24 (CI10)

- [x] `a_database_from_the_previous_version_opens_with_no_remote_recorded` (`db.rs`), y el checksum fijado.
- [x] `V24__project_remote.sql`.
- Commit: junto con la tarea 3, su primer uso.

## 3. Unir y deshacer (CI5, CI6)

- [x] Tests puros de `reconcile` y de `decide_merge`; tests de `merge_in` y del deshacer sobre `MemoryStore::new_for_test()`.
- [x] `merge_in`, `reconcile_in`, `memory_move_supersede`, `undo_move_core` que revierte.
- Commit: junto con la tarea 4, porque `merge_in` sólo tiene uso desde el registro.

## 4. El registro asigna (CI2, CI3, CI4, CI7)

- [x] Tests de `project.rs` y de `tests/api.rs`.
- [x] `register_project` con `assigned` y `remote`, `Reason::PreRekey`, `RegisterRequest.assigned`, `RegisteredBody.root_id`.
- Commit: `feat(core): give each root an id of its own and join the clones of one remote` (`7d8bb18`, con las tareas 2 y 3). El número del movimiento en el log llegó después, en `feat(daemon): name the move that undoes a registration's join` (`dfa090f`).

## 5. Los clientes (CI8)

- [x] Tests de `server.rs`, `import.rs` y `export.rs`; el hook, por el binario real (tarea 7).
- [x] `Registered` en el cliente, `project_for` con dos ids, los hooks, `projects`, `export`, el import.
- Commit: `feat: use the project id the daemon assigns` (`0d728a3`). `memories --here`, que la tarea no nombraba, se encontró al revisar el diff: `feat(cli): read the project of memories --here as the store records it` (`2b6fb5b`).

## 6. `doctor` (CI9)

- [x] `a_project_id_assigned_to_its_root_is_ok`.
- [x] `is_candidate_for`.
- Commit: `feat(doctor): accept a project id assigned to its root` (`fa680e9`)

## 7. El binario real

- [x] `two_clones_share_their_memories_through_a_real_daemon` (`tests/binary.rs`).
- Commit: `test(cli): two clones of one remote through a real daemon` (`33ce5b9`)

## 8. Docs

- [x] `.claude/CLAUDE.md`, el `README.md` y, al archivar, las specs.
- Commit: `docs: describe how the clones of one remote share a project` (`681baec`)
