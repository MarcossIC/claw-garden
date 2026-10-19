# setup-exec-hooks — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. `clawg` a secas no bloquea (R8, D1)

- [ ] Tests primero: `a_bare_clawg_never_blocks` (`cli.rs`), `a_bare_clawg_exits_1` (`tests/binary.rs`).
- [ ] `is_bare_invocation` y la rama de `main.rs`.
- Commit: `fix(cli): exit 1, not 2, when clawg runs without a subcommand`

## 2. Hooks en forma exec y migración de la forma heredada (R3, R6, D2, D3)

- [ ] Tests primero: `the_hook_runs_the_binary_without_a_shell`, `an_old_shell_hook_of_clawg_is_migrated_in_place`, `an_edited_shell_hook_is_still_a_conflict`, `remove_takes_both_forms`; los que leían la cadena pasan a `command` + `args`.
- [ ] El hook del plan, `is_clawg_hook` sobre el hook, `legacy_command`, `migrated` en el plan y el reporte; fuera `shell_word` como escritor.
- [ ] `tests/binary.rs` lee el hook como `command` + `args`.
- Commit: `fix(cli): install the hooks in exec form and migrate the shell form setup wrote`

## 3. Escritura: enlace, modo, `sync_all`, copias privadas (R7, D4)

- [ ] Tests primero (Unix): `a_linked_file_is_written_through_its_link`, `a_file_keeps_its_mode_and_copies_are_owner_only`.
- [ ] `write_file` sobre el destino real, con el modo del original, `sync_all` y copias 0600.
- Commit: `fix(cli): write the client's files through their links and keep their mode`

## 4. Poda de copias (R7, D5)

- [ ] Tests primero: `stale_copies_are_the_oldest_beyond_the_limit`, `only_the_newest_copies_are_kept`.
- [ ] `stale_copies`, la poda en `apply` y su renglón en el reporte.
- Commit: `feat(cli): keep only the five newest setup copies of each file`

## 5. Docs

- [ ] README y `.claude/CLAUDE.md`; `state.yaml`.
- Commit: `docs: describe the exec-form hooks and the setup copies`

## 6. Verificación

- [ ] Suite y clippy en Linux sobre el último commit.
- [ ] `clawg setup claude-code` con el binario de tests en un home temporal, y el `claude` real del PATH.
- [ ] `verify.md`.
