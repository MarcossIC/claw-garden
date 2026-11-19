# contract-diet — Tareas (el bloque `lean`)

Strict TDD: en cada tarea el test va primero y falla por la aserción (o porque lo que nombra todavía no existe). Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia. La suite completa y los dos clippy corren una vez, en la tarea 5. Todo corre en Linux con el embedder falso; en Windows, con `--target-dir target-test`.

La clase (a) ya está hecha ([exploración](exploration.md#hecho-la-clase-a)); estas tareas son la clase (b), enfoque B1.

## 1. El texto de `lean` (CD1, CD2)

- [x] Tests primero en `crates/clawg-cli/src/mcp/protocol.rs`: `the_lean_protocol_keeps_the_head_of_the_full_one`, `the_lean_protocol_keeps_the_topic_key_rule_and_the_session_summary`, `the_lean_protocol_names_what_the_full_one_names`, `a_language_rewrites_both_rules_of_the_lean_protocol`, `the_lean_protocol_is_at_most_seventy_percent_of_the_full_one`; `a_variant_is_named_by_its_option` con `lean`; y `lean` en `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one`, `the_protocol_says_which_preferences_open_every_session` y `the_protocol_says_where_a_call_without_a_project_goes`.
- [x] Tests primero en `crates/clawg-cli/src/mcp/server.rs`: `every_protocol` suma `lean` en las tres formas, y `the_protocol_says_where_a_key_is_shown`, `the_protocol_describes_the_session_summary` y `the_spanish_protocol_describes_the_session_summary` corren sobre `full` y `lean`.
- [x] `protocol.lean.md` y `protocol.lean.es.md`; `Variant::Lean` en `ALL`, `as_str` y `parse`; `protocol::lean`, con la reescritura del idioma compartida con `text`. `assemble` arma `Lean` (sin `--protocol` todavía no se puede elegir). `protocol.md` y `protocol.es.md` no cambian.
- Commit: `feat(mcp): write a lean protocol that keeps the opening and every rule`.

## 2. `--protocol lean` y el sello (CD3, CD4)

- [x] Tests primero: `each_variant_is_its_block`, `the_protocol_option_writes_its_variant_and_a_run_without_it_keeps_it`, `a_new_version_with_the_same_text_rewrites_nothing` con `lean`, y `a_stamp_with_an_unknown_variant_reads_as_stale` (`crates/clawg-cli/src/setup.rs`); `the_protocol_option_parses_and_excludes_local_remove_and_cursor` con `--protocol lean` y el mensaje con las cuatro (`crates/clawg-cli/src/cli.rs`); `--protocol lean` entre las elecciones que `doctor` da `ok` (`crates/clawg-cli/tests/binary.rs`).
- [x] El mensaje y la ayuda de `--protocol` (`cli.rs:25-27`, `:311-316`).
- [x] `docs/reference/commands.md`, en la entrada de `setup claude-code`: `lean` y que un binario que no la conoce lee el bloque como `stale`.
- Commit: `feat(setup): choose the lean protocol block with --protocol lean`.

## 3. El brazo `a3-lean` (CD5)

- [x] Test primero: `a3_lean_is_a3_with_the_lean_block` (`crates/clawg-eval/src/bin/agent-eval/arm.rs`), y el brazo en las listas que enumeran los brazos con daemon, con `setup` y con bloque.
- [x] `Arm::A3Lean` (`"a3-lean"`), junto a `A3Short` en `ARMS`, `as_str`, `has_clawg`, `has_setup`, `has_protocol` y el plan de `setup` con `--protocol lean`; la ayuda de `agent-eval` (`main.rs:120`).
- [x] `docs/reference/testing.md`, donde lista los brazos.
- Commit: `feat(agent-eval): add the a3-lean arm with the lean protocol block`.

## 4. El estado del cambio

- [x] `state.yaml`, el mapa (`openspec/README.md`) y `openspec/ROADMAP.md`.
- Commit con los documentos de verificación (tarea 5).

## 5. Verificación

- [x] `cargo clean --workspace` (en Windows, con `--target-dir target-test`), y después la suite y los dos clippy de `.claude/CLAUDE.md`, una vez, sobre el último commit.
- [x] `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` pasan sin regenerar nada (CD6); `git diff develop` no toca `protocol.md`, `protocol.es.md`, `agent_contract*.json`, `crates/clawg-cli/src/daemon/` ni `hook.rs`.
- [x] `verify.md`: los requisitos con sus tests, el tamaño medido y la campaña pendiente con su regla.
- Commit: `docs(openspec): verify the lean protocol block and leave its campaign pending`.

## Después de este cambio

- [ ] **La campaña**, paga, después de publicar y con la versión instalada: `a3-lean` contra `a3` en R, D y la K larga, 3 corridas por celda, con la regla de [design](design.md#la-regla-de-decisión). Pendiente en [verify](verify.md#pendiente-la-campaña).
- [ ] **El valor por defecto:** si la regla elige `lean`, un commit propio cambia `DEFAULT_VARIANT`; si no, queda `full`.
