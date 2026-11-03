# cli-dispatch — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque la función que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Lo que se muda sin conducta nueva lo fijan también los tests que ya hay, que pasan antes y después sin cambiar lo que esperan. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia y con `crates/clawg-cli/tests/binary.rs` cuando toca un comando que ese archivo corre; la suite y clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea.

Requisitos en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## 0. Antes del código

- [x] El conteo de la exploración, verificado otra vez sobre `a7f7337`.
- [x] El binario de `a7f7337` guardado fuera del repositorio, para comparar la salida al final.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.

## 1. `status` (CD3)

- [ ] Tests en rojo: `status_without_a_daemon_reports_the_store_and_exits_1` y `status_with_a_daemon_reports_its_health_and_exits_0`.
- [ ] `status::command(profile, base_dir) -> (String, u8)`; el brazo imprime y sale con el código.
- Commit: `refactor(status): build the status report and its exit code in its module`

## 2. `CliError` (CD1)

- [ ] Tests: `a_typed_error_reads_as_its_source`, `a_composed_sentence_reads_as_it_was_written`.
- [ ] `crates/clawg-cli/src/error.rs`; `dispatch` y `run_path` con `CliError`; cada `.map_err(|e| e.to_string())` pasa a `?`, y cada frase en `String` entra con `CliError::Failed`.
- Commit: `refactor(cli): fail a command with a typed error`

## 3. `stop` (CD2)

- [ ] Tests: `a_missing_daemon_is_named_in_both_languages`, `the_stop_line_speaks_both_languages`.
- [ ] `stop::command`, `stop::explain` y `stop::report`.
- Commit: `refactor(stop): say what stop did, or why it could not, from its module`

## 4. El daemon corriendo, en un lugar (CD2)

- [ ] Tests: `a_missing_daemon_says_how_to_start_it_in_both_languages`, `a_published_daemon_is_found`.
- [ ] `running.rs` con `Running`; `language` pasa a `preferences.rs`; `Import`, `Undelete`, `Outbox`, `Projects` y `Memories` buscan el daemon con él.
- Commit: `refactor(cli): find the running daemon of a command in one place`

## 5. Un commit por comando (CD4)

Cada uno: el test de la tabla de CD4 primero, después la función del comando y el brazo que la llama.

- [ ] `backup`, en un módulo nuevo. Commit: `refactor(backup): choose between the daemon and the offline copy in its module`
- [ ] `restore`. Commit: `refactor(restore): restore and explain a refusal in its module`
- [ ] `export`. Commit: `refactor(export): resolve the exported project in its module`
- [ ] `import`. Commit: `refactor(import): choose between an import and an undo in its module`
- [ ] `secrets` y `purge`. Commit: `refactor(secrets): run secrets and purge from their module`
- [ ] `metrics report`. Commit: `refactor(metrics): choose the report or the events in its module`
- [ ] `projects`, con `ProjectsAction::profile`. Commit: `refactor(projects): run each projects action from its module`
- [ ] `path`, a `user_path.rs`. Commit: `refactor(path): move clawg path to its module`

## 6. Documentación

- [ ] `docs/reference/data-profiles.md` nombra `language` donde quedó; la fila 112 de `openspec/README.md`; este archivo y `state.yaml`.
- Commit: `docs(openspec): record cli-dispatch applied`

## Verificación

- [ ] `git diff a7f7337 -- crates/clawg-cli/tests/binary.rs` vacío.
- [ ] El binario de `a7f7337` y el del último commit, sobre los mismos casos en un home temporal con el embedder falso, en `en` y `es`: misma salida, mismo stderr, mismo código.
- [ ] La suite y `cargo clippy --workspace --all-targets -- -D warnings` en Linux; Windows por el CI del pull request.
