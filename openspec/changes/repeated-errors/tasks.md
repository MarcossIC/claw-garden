# repeated-errors — Tareas

TDD estricto: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada grupo es un commit convencional atómico, comprobado con los tests de los archivos que cambia. La suite completa y los dos clippy corren una vez, sobre el último commit (tarea 7), con `--target-dir target-test` en Windows y después de `cargo clean --workspace --target-dir target-test`. Los requisitos están en la [spec](spec.md) (RE1 a RE9) y las decisiones en la [propuesta](proposal.md) y el [diseño](design.md).

**Puerta antes de empezar:** si la fase 4 de la campaña de 129 ya corrió y dio que `a3` de hoy evita el fallo (0 fallos en la sesión 3 en 3 de 3), la tarea 4 (el bloque del arranque) no se aplica. El libro (tarea 2) queda, porque lo usan el aviso del mismo turno y `doctor` ([propuesta](proposal.md#cómo-se-mide-el-éxito)). Si no corrió, se aplica todo.

## 1. La firma de un programa ausente (RE2)

- [x] RED (`tool_failures.rs`): `a_missing_program_is_read_from_each_shell_form`, `a_failure_needs_the_program_in_command_position`, `a_pipe_with_exit_zero_still_shows_the_failure`, `a_redirected_or_chained_command_is_no_success`, `a_secret_or_an_odd_name_is_never_a_program`, `exit_127_alone_and_other_shells_say_nothing`. Los textos de los casos salen de las formas reales de los datos de A2: `/usr/bin/bash: line 1: X: command not found`, con o sin `Exit code 127` delante y repetida, `sh: 1: X: not found` y `zsh: command not found: X`.
- [x] GREEN: `crates/clawg-cli/src/tool_failures.rs` con `missing_programs`, `command_words` y `observe`; el módulo en `lib.rs`; `redact` pasa a `pub` (`crates/clawg-core/src/memory/domain/redact.rs:235`).
- Commit: `feat(hook): recognize a shell's command-not-found in a tool's output`

## 2. El libro y las reglas (RE4, RE5)

- [x] RED (`tool_failures.rs`): `a_line_keeps_only_the_program_the_tool_and_the_session`, `only_the_first_success_after_a_failure_is_written`, `the_ledger_rolls_at_64_kib_without_rewriting`, `no_data_directory_no_ledger`, `a_damaged_line_costs_only_itself`, `two_sessions_make_a_rule_and_one_does_not`, `a_success_in_the_window_cancels_the_rule`, `a_program_on_the_path_is_no_rule`, `failures_older_than_thirty_days_are_forgotten`, `at_most_five_rules_newest_first`. Los directorios salen de un directorio de prueba que el test borra.
- [x] GREEN: `Line`, `parse_lines`, `to_write`, `rules`, `read`, `append` y `on_path`; `hook_log::append` generalizado a `append_rolled` (D2), con los tests de HR4 sin cambios.
- Commit: `feat(hook): keep a bounded ledger of missing programs and the rules it gives`

## 3. Los eventos después de una herramienta y el aviso del mismo turno (RE1, RE3)

- [x] RED: `hook_takes_the_post_tool_events` (`cli.rs`); `the_hook_input_reads_the_tool_fields`, `cursor_and_other_tools_get_nothing_after_a_tool`, `the_same_turn_line_names_the_program_in_the_users_language`, `a_program_that_worked_recently_gets_no_line`, `a_call_without_a_signature_prints_nothing` (`hook.rs`); `a_post_tool_hook_answers_without_a_daemon_and_leaves_the_store_alone` (`tests/binary.rs`).
- [x] GREEN: `HookEvent::PostToolUse` y `HookEvent::PostToolUseFailure`; los campos nuevos de `HookInput`; `after_tool` en `respond_reading`; `POST_TOOL_DEADLINE`; `deadline_of` y `envelope` con los dos eventos; `same_turn_line` en los dos idiomas.
- Commit: `feat(hook): warn in the same turn when a Bash call hits a missing program`

## 4. Las reglas al iniciar la sesión (RE6)

- [x] RED: `session_start_names_the_learned_rules_before_the_evidence`, `the_rules_come_out_of_the_context_budget`, `without_rules_session_start_says_what_it_said`, `what_the_hook_adds_stays_small` ampliado (`hook.rs`); `two_sessions_teach_the_third`, `one_session_teaches_nothing` (`tests/binary.rs`, sin daemon: los dos `post-tool-use-failure` y el `session-start` corren contra el mismo `CLAWG_TEST_HOME`).
- [x] GREEN: `rules_block` en los dos idiomas; la lectura del libro junto a la del idioma; el presupuesto descontado en `answer`; el bloque en `session_start_text` y en el texto de respaldo sin daemon.
- Commit: `feat(hook): name the missing programs it learned when a session starts`

## 5. `setup` instala los dos hooks (RE7)

- [x] RED: `a_run_over_an_install_without_the_tool_hooks_adds_only_them`, `the_tool_hooks_match_bash_only`, `remove_takes_the_tool_hooks_too`, `an_empty_state_plans_the_server_the_hooks_and_the_block` ampliado (`setup.rs`); los tests de `setup/local.rs` que listan los eventos, ampliados a cinco.
- [x] GREEN: `HOOK_EVENTS` con el matcher de cada evento, e `install_hooks` que lo usa (D7). Revisar que los tests de `crates/clawg-eval/src/bin/agent-eval/arm.rs` que miran las claves de `settings.json` sigan pasando: los brazos con `setup` ahora tienen dos eventos más.
- Commit: `feat(setup): install the post-tool hooks for Bash`

## 6. `doctor` (RE8)

- [x] RED (`doctor/checks.rs`): `the_learned_rules_are_listed_with_the_files_that_ask_for_them`, `no_rules_no_finding`, `a_session_start_hook_without_the_tool_hooks_is_reported`.
- [x] GREEN: `checks::tool_rules` y `checks::tool_hooks`; su lectura en `doctor::run` (D8).
- Commit: `feat(doctor): show the missing programs clawg learned and the instructions that ask for them`

## 7. La referencia

- [x] `docs/reference/commands.md`: los dos eventos de `clawg hook`, los dos hooks de `setup` y los hallazgos de `doctor`. `docs/reference/data-profiles.md`: `tool-failures.jsonl`, con lo que guarda, lo que no guarda, la rotación, y que `restore`, `purge`, `export` y los snapshots no lo tocan. La spec, las tareas y el estado de este cambio.
- [x] Suite completa y los dos clippy sobre este commit.
- Commit: `docs: describe the missing-program ledger and the post-tool hooks`

## 8. Verificación (local, en Windows)

- [ ] Suite y clippy pasan. CI de Linux y Windows pasa.
- [ ] Con autorización para `~/.clawg-dev` y el binario publicado que trae este cambio: registrar una entrada real de `PostToolUse` y de `PostToolUseFailure` de Bash, sumarlas como fixtures de los tests de la tarea 3 y confirmar que la firma está en algún texto de `tool_response`.
- [ ] La puerta de latencia de la propuesta (decisión 6): 200 corridas de `clawg hook post-tool-use --profile dev` desde Git Bash con una entrada sin firma, y 200 con una firma. Se anotan p50 y p95 de cada una junto a `clawg --version` y `/usr/bin/true` en la misma tanda. Pasa con p95 ≤ 60 ms en la entrada sin firma.
  - Cómo medir: un binario publicado (no el de `debug`), desde Git Bash en Windows, con `~/.clawg-dev` autorizado, una vez y sin otro proceso pesado. Cada corrida lleva su stdin por un pipe, como la del cliente, y se cronometra con `date +%s%N` antes y después. El costo de `date` y del pipe es el mismo en las cuatro tandas, así que las compara.
  - Entrada sin firma, que es la de casi toda llamada: la de un `rg` que anduvo, `{"session_id":"lat","tool_name":"Bash","tool_input":{"command":"rg todo src"},"tool_response":{"stdout":"a.rs:1: todo","stderr":""}}`. Pasa por el camino que lee el libro, que es el caso caro de verdad.
  - Entrada con firma: `{"session_id":"lat","tool_name":"Bash","tool_input":{"command":"latency-probe-xyz"},"error":"Exit code 127\n/usr/bin/bash: line 1: latency-probe-xyz: command not found"}` a `post-tool-use-failure`. Anota una línea por corrida en `~/.clawg-dev/tool-failures.jsonl`, de una sola sesión, así que no crea una regla; al terminar se borran `tool-failures.jsonl` y `tool-failures.1.jsonl` de ese perfil.
  - Cada tanda: `for i in $(seq 200); do s=$(date +%s%N); printf '%s' "$INPUT" | clawg hook post-tool-use --agent claude-code --profile dev >/dev/null; e=$(date +%s%N); echo $(( (e - s) / 1000000 )); done | sort -n`, y p50 y p95 son los elementos 100 y 190 de la lista. Para `clawg --version` y `/usr/bin/true` la misma línea con ese comando.
  - Con p95 de la entrada sin firma sobre 60 ms no se publica con `PostToolUse` instalado: se baja la lectura del libro al camino con firma, o se descarta el evento y quedan `PostToolUseFailure` y el libro.
- [ ] `verify.md` con lo anterior y lo que queda pendiente en prod.

## 9. La medición en agent-eval (RE9), cuando 129 esté aplicado

- [ ] RED: `a3_notools_is_a3_without_the_tool_hooks`, `a3_notools_reads_and_prints_by_its_name` (`arm.rs`).
- [ ] GREEN: `Arm::A3NoTools`, con su nombre en el parseo y en el reporte. La tarea, el shell y el check son los de 129.
- Commit: `feat(agent-eval): add arm a3-notools, a3 without the post-tool hooks`

## Después de este cambio

- [ ] La campaña paga, con una guía fuera de git como la del piloto: `shelf-eza-missing` con 5 corridas por brazo y la tarea N del piloto con 2, brazos `a3-notools` y `a3`, con tope de gasto, y el `a0` de 129. Se aplica la regla de decisión de la [propuesta](proposal.md#cómo-se-mide-el-éxito) tal como está escrita, y el resultado va a `verify.md`.
- [ ] Correr otra vez `clawg setup claude-code` (y `--local` en el repositorio del piloto) en cada cliente ya configurado. `clawg doctor` lo avisa mientras falte.
