# project-clone-identity — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. La clave de una raíz (CI1)

- [ ] Tests puros de `normalize_remote`, `origin_url` y `git_config_dir` (`identity.rs`), y los de `project_root.rs` con un worktree, un submódulo y una raíz sin `origin`.
- [ ] `origin_key` y sus piezas.
- Commit: `feat(core): read the origin a project root was cloned from`

## 2. V24 (CI10)

- [ ] `a_database_from_the_previous_version_opens_with_no_remote_recorded` (`db.rs`), y el checksum fijado.
- [ ] `V24__project_remote.sql`.
- Commit: junto con la tarea 3, su primer uso.

## 3. Unir y deshacer (CI5, CI6)

- [ ] Tests puros de `reconcile` y de `decide_merge`; tests de `merge_in` y del deshacer sobre `MemoryStore::new_for_test()`.
- [ ] `merge_in`, `reconcile_in`, `memory_move_supersede`, `undo_move_core` que revierte.
- Commit: junto con la tarea 4, porque `merge_in` sólo tiene uso desde el registro.

## 4. El registro asigna (CI2, CI3, CI4, CI7)

- [ ] Tests de `project.rs` y de `tests/api.rs`.
- [ ] `register_project` con `assigned` y `remote`, `Reason::PreRekey`, `RegisterRequest.assigned`, `RegisteredBody.root_id`.
- Commit: `feat(core): give each root an id of its own and join the clones of one remote`

## 5. Los clientes (CI8)

- [ ] Tests de `server.rs`, `hook.rs`, `projects.rs`, `import.rs` y del lookup del export.
- [ ] `Registered` en el cliente, `project_for` con dos ids, los hooks, `projects`, `export`, el import.
- Commit: `feat: use the project id the daemon assigns`

## 6. `doctor` (CI9)

- [ ] `a_project_id_assigned_to_its_root_is_ok`.
- [ ] `is_candidate_for`.
- Commit: `feat(doctor): accept a project id assigned to its root`

## 7. El binario real

- [ ] `two_clones_share_their_memories_through_a_real_daemon` (`tests/binary.rs`).
- Commit: `test(cli): two clones of one remote through a real daemon`

## 8. Docs

- [ ] `.claude/CLAUDE.md` y las specs.
- Commit: `docs: describe how the clones of one remote share a project`
