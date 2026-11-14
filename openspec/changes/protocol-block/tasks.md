# protocol-block — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción (o porque lo que nombra todavía no existe). Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia. La suite completa y los dos clippy corren una vez, en la tarea 7. Todo corre en Linux con el embedder falso; en Windows, con `--target-dir target-test`.

## 1. El sello y la frescura (PB1, PB2)

- [ ] Tests primero en `crates/clawg-cli/src/setup.rs`: `the_block_carries_the_version_that_wrote_it`, `a_stamp_reads_its_version_and_its_choice` (con un sello mal escrito y uno sin `protocol=`) y `a_block_is_current_stale_or_newer` (los tres escenarios de PB2, y uno con `\r\n`).
- [ ] `Stamp`, su línea y su lectura; `block` toma la versión y la escribe después del marcador; `freshness`, pura, compara cuerpos. Ajustar las aserciones que miran el comienzo del bloque (`an_empty_state_plans_the_server_the_hooks_and_the_block`, `setup.rs:1282`) y los llamados a `block` de `doctor/mod.rs:295`, que siguen comparando como hoy.
- Commit: `feat(setup): stamp the protocol block with the version that wrote it`.

## 2. Las variantes (PB5)

- [ ] Tests primero: `the_short_protocol_keeps_the_session_summary_in_both_languages` y `the_short_protocol_has_no_opening` (`crates/clawg-cli/src/mcp/protocol.rs`), `each_variant_is_its_block` (`setup.rs`).
- [ ] `Variant` y `protocol::short`, que corta las dos secciones del texto del idioma; `block` arma cada variante. Sin `--protocol` todavía: el plan sigue escribiendo `full`.
- Commit: `feat(setup): build the short and the empty protocol block`.

## 3. `--protocol` (PB6)

- [ ] Tests primero: `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it` y `a_new_version_with_the_same_text_rewrites_nothing` (`setup.rs`), `the_protocol_option_parses_and_excludes_local_remove_and_cursor` (`crates/clawg-cli/src/cli.rs`), `setup_protocol_short_writes_the_short_block` (`crates/clawg-cli/tests/binary.rs`).
- [ ] La opción en `Command::Setup` (`cli.rs:289`) y en `main.rs:278-290`; `State` lleva la variante pedida y `install_block` resuelve la que queda con el sello instalado; el reporte la nombra. El literal de `State` del arnés (`crates/clawg-eval/src/bin/agent-eval/arm.rs:235-242`) pasa sin variante, así `a3` sigue igual.
- [ ] `docs/reference/commands.md`, en la entrada de `setup claude-code`: el sello, `--protocol` y que sin la opción se conserva la elegida.
- Commit: `feat(setup): choose the protocol block with --protocol`.

## 4. `doctor` (PB3)

- [ ] Tests primero: `protocol_block_findings` con `newer`, con `written_by` y `this_version`, y con el mensaje sin preferencia; los hallazgos nuevos en `the_findings_are_the_same_in_both_languages_but_their_message` (`crates/clawg-cli/src/doctor/checks.rs`); `doctor_marks_an_old_block_stale_without_a_preference` (`tests/binary.rs`).
- [ ] `doctor::protocol_block` corre con `Option<Lang>` (`crates/clawg-cli/src/doctor/mod.rs:185-187`, `:278-296`) y clasifica con `freshness`; `checks::protocol_block` toma la clasificación y la preferencia (`checks.rs:440`).
- [ ] `docs/reference/commands.md`, en la entrada de `doctor`: `protocol_block` con o sin preferencia, y `newer`.
- Commit: `fix(doctor): check the protocol block without a language preference`.

## 5. El aviso de `session-start` (PB4)

- [ ] Tests primero en `crates/clawg-cli/src/hook.rs`: `session_start_says_when_the_block_is_stale` (`stale` en los dos idiomas, sin daemon, y nada con `current`, `newer`, sin bloque o ilegible, ni con `--client cursor`), `what_the_hook_adds_stays_small` con el aviso y el tope de 2.000, `a_block_that_never_reads_costs_the_notice_and_not_the_memory`; y `session_start_names_an_old_protocol_block` (`tests/binary.rs`).
- [ ] `block_notice`, pura; `session_start_text` la recibe y la pone después del idioma; `respond_reading` lee el bloque en paralelo con la preferencia, con el mismo tope, y la suma también cuando no hay respuesta del daemon (`hook.rs:362`). El perfil llega desde `main.rs:128-136`.
- [ ] `docs/reference/commands.md:7`, en la entrada de `clawg hook`: el aviso del bloque.
- Commit: `feat(hook): say at session start when the protocol block differs`.

## 6. El brazo `a3-short` (PB7)

- [ ] Test primero: `a3_short_is_a3_with_the_short_block` (`crates/clawg-eval/src/bin/agent-eval/arm.rs`), y el brazo en las listas que ya enumeran los brazos con servidor y con `setup` (`arm.rs:548` y vecinos).
- [ ] `Arm::A3Short` (`"a3-short"`), en `ARMS`, con el plan de `setup` con `--protocol short`.
- [ ] `docs/reference/testing.md`, donde lista los brazos.
- Commit: `feat(agent-eval): add the a3-short arm with the short protocol block`.

## 7. Verificación

- [ ] `cargo clean --workspace --target-dir target-test`, y después la suite y los dos clippy de `.claude/CLAUDE.md`, una vez, sobre el último commit.
- [ ] `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin regenerar nada (PB8); `git diff` no toca `crates/clawg-cli/src/mcp/agent_contract*.json` ni `crates/clawg-cli/src/daemon/`.
- [ ] El CI en verde en Linux y Windows.
- [ ] `verify.md`: los requisitos con sus tests, y lo pendiente en local.

## Después de este cambio

- **En local, con la versión instalada:** abrir una sesión de Claude Code en una máquina con el bloque de antes de 0.85.0 y ver que el agente nombra la orden; correrla y ver `doctor` en `ok`. Con los scripts de A1 (`docs/completed-impl/hipotesis/A1/`) sobre los transcripts de las semanas siguientes, los `resolve_conflict` por save con cercanas tienen que bajar a menos de 0,15 (umbral de verdict-on-replace).
- **La campaña**, sobre las tareas largas de 129, con los brazos `a3`, `a3-short` y `l0`, 18 corridas nuevas con un tope de 15 USD, y la regla de [design](design.md#d6-la-regla-de-decisión-del-valor-por-defecto), que no se cambia después de ver resultados. Necesita el arnés de 129 aplicado; es paga y la autoriza la persona.
- **El valor por defecto:** si la regla elige `short` u `off`, un commit propio cambia la variante de por defecto en `setup.rs` y sus tests, `docs/reference/commands.md` y `specs/setup` ya consolidada, y sube la versión como un arreglo fuera de openspec. Quien eligió con `--protocol` conserva su variante; quien no eligió toma la nueva la próxima vez que corre `setup`, y el aviso de `session-start` se lo pide.
