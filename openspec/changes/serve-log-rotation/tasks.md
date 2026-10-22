# serve-log-rotation — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. La primera línea del daemon nombra su versión (R2)

- [ ] Tests primero: `the_starting_line_names_the_version_and_the_build` (`daemon/serve.rs`); `a_daemon_names_its_version_first` (`tests/binary.rs`).
- [ ] `starting_line`, registrada al empezar `serve::run`.
- Commit: `feat(daemon): name the version and build first in the daemon's log`

## 2. `serve.log` se rota al abrirse (R1)

- [ ] Tests primero: `nothing_rotates_up_to_the_limit`, `over_the_limit_each_file_moves_one_place`, `rotating_keeps_three_files`, `a_file_open_for_writing_still_rotates`, `a_failed_rotation_is_written_to_the_log` (`serve_log.rs`); `an_autostart_rotates_a_full_serve_log` (`tests/binary.rs`).
- [ ] `serve_log::rotation`, `rotate` y `open`, con `SERVE_LOG_LIMIT` y tres ficheros; `spawn_serve_at` abre el log con `open`.
- Commit: `feat(cli): rotate serve.log past 10 MiB when a daemon is started`

## 3. Docs

- [ ] `.claude/CLAUDE.md` (`clawg serve`) y estas tareas.
- Commit: `docs: say where the daemon logs and how much of it is kept`

## 4. Verificación

- [ ] Suite y clippy en Linux sobre el último commit, y cada commit de código por separado ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 5. Archive

- [ ] «El log del daemon» en `specs/storage`; `git mv` a `changes/archive/<fecha>-serve-log-rotation`; `state.yaml` archivado; fila 72 y el orden de ejecución del `README.md` de openspec; subir el minor.
