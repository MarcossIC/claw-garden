# agent-authorship — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite.

## 1. V20 (AA10)

- [x] Test `a_store_from_before_project_scoped_rules_keeps_its_rows` (`db.rs`): siembra V19 y falla porque los índices empiezan por `agent_id`.
- [x] `V20__project_scoped_indexes.sql`, `migrations.rs` tocado y el checksum fijado.
- Commit: `feat(core): index dedup and topic keys by project`

## 2. El lector en el acceso por id (AA1, AA2)

- [x] Tests de `sharing.rs`, `pieces.rs`, `admin.rs`, `dispute.rs`, `synthesis.rs` y `tests/api.rs`; los que fijaban la frontera por agente pasan a afirmar lo contrario y, con `hide`, lo de antes.
- [x] `Reader`, `reader_in` y `reader_predicate!`; los núcleos con `Option<&Reader>`; el daemon arma el lector de `?agent=` y de `agent_id`.
- Commit: `feat: let an agent read and change every memory it reaches`

## 3. Las reglas por proyecto (AA3–AA6, AA10)

- [x] Tests de `store_insert.rs`, `contradiction.rs`, `worker.rs`, `revival.rs`, `admin.rs`, `synthesis.rs`, `import.rs` y `tests/api.rs`.
- [x] Sin `Reach::OwnOnly`; el worker lee la política del que guarda; dedup, detector, clave, revivir, restore, import y síntesis por proyecto.
- [x] Test `a_keyed_save_replaces_every_holder_of_its_project` (`worker.rs`), lo que una base anterior puede tener.
- Commits: `feat(core): compare a save with its project, not its agent`, `test(core): a keyed save replaces the holders two agents left in a project`

## 4. Las vistas y lo que ve el modelo (AA7, AA8)

- [x] Tests de `prompt_builder.rs`, `dispute.rs`, `admin.rs`, `server.rs` y `tools.rs`.
- [x] ACTIVE CONFLICTS, LAST SESSION, `open_pairs` por proyecto, `reader` en el listado, `list_memories` con autor y `agent`, la clave en cualquier hit del proyecto, la línea `shared:`, las descripciones, `SHARED_NOTE` y `protocol.md`. El contrato se bendice y el diff se revisa.
- Commit: `feat(mcp): show and change other agents' memories from the tools`

## 5. El doctor (AA9)

- [x] Tests de `inspect.rs` y `checks.rs`.
- [x] `TopicVersions` y `HashDuplicates` por proyecto con `agents`; el texto de `fallback_agent`.
- Commit: `feat(doctor): group topic keys and duplicates by project`

## 6. Docs

- [x] `.claude/CLAUDE.md`.
- Commit: `docs: say an agent's id names the author, not who may change a memory`

## 7. Verificación

- [x] `cargo test --workspace` y el clippy de `.claude/CLAUDE.md` en Linux.
- [ ] Lo mismo en Windows, en `target-test`: queda al CI del pull request ([verify](verify.md)).
- [x] `conflict-eval` y `revision-eval chains --fake` dan los mismos números ([verify](verify.md)).
- [x] El binario real en un home temporal, por HTTP y con `clawg sharing` y `clawg doctor` ([verify](verify.md)).
- [ ] Perfil dev, con autorización (no se corrió): dos puentes con `--agent` distintos; uno guarda una nota larga, el otro la lee entera, la corrige, la borra y la restaura; `clawg sharing hide` corta cada una; `clawg doctor` sobre una base de la versión anterior.

## 8. Archive

- [x] Las specs de la propuesta; `git mv` al archivo; subir el minor.
