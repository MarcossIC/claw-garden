# agent-eval — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla, por la aserción o porque no compila. Cada grupo es un commit convencional atómico, comprobado con las pruebas de los archivos que cambia; la suite completa y los dos clippy corren una vez, sobre el último commit. Requisitos en la [spec](spec.md) (AE1 a AE6) y decisiones en la [propuesta](proposal.md).

## 1. El tier de una fila sembrada (AE3)

- [x] RED: `a_seeded_row_earns_the_tier_the_store_promotes_it_to` en `eval_api.rs`.
- [x] GREEN: `eval_api::earned_tier`.
- Commit: `feat(eval): expose the tier a seeded row earns`

## 2. El binario (AE1 a AE6)

Un binario es un crate: una función que sólo usa otro módulo todavía ausente es código muerto, así que los módulos entran juntos.

- [x] RED: los tests de `task.rs`, `arm.rs`, `seed.rs`, `stream.rs`, `score.rs`, `rubric.rs`, `replay.rs`, `run.rs` y `main.rs`.
- [x] GREEN: los módulos y el `[[bin]]` en `crates/clawg-eval/Cargo.toml`.
- Commit: `feat(eval): lay out agent sessions by arm and score their stream`

## 3. Las tareas de ejemplo y la corrida en seco (AE1, AE6)

- [x] `data/agent-tasks/`: `example-staging-port` (K1) y `example-count-lines` (N) sobre el fixture `example-tally`, y los streams `example-staging-port.a3`, `example-staging-port.a0` y `example-count-lines.a3`.
- [x] `tests/agent_eval.rs`.
- Commit: `test(eval): replay hand-written sessions of example agent tasks`

## 4. La referencia

- [x] `docs/reference/testing.md`; los punteros de `.claude/CLAUDE.md`; esta propuesta, la spec, las tareas y el mapa de `openspec/README.md`.
- Commit: `docs: describe agent-eval and its dry run`

## 5. Verificación

- [x] Suite completa y clippy sobre el último commit: pasan.

## Después de este cambio

- [ ] Las seis tareas del piloto y su fixture, revisadas antes de correr.
- [ ] El corredor real (daemon de la corrida, `clawg import`, `claude`, `clawg stop`) y el piloto, con autorización y tope de gasto.
