# embedder-recovery — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. El hilo contesta un pánico y salta lo cancelado (R1, R2, D1)

- [x] Tests primero: `a_panicking_inference_answers_its_caller_and_ends_the_worker`, `a_request_whose_caller_is_gone_is_not_inferred`.
- [x] `serve_requests`, con `catch_unwind` e `is_closed`; `from_parts` lo usa.
- Commit: `fix(core): answer a panicking inference and drop its session instead of losing the worker`

## 2. Ocio opcional (R5, D4)

- [x] Test primero: `with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered` (su primera mitad: el modelo no se suelta).
- [x] `IdleEmbedder::new` y `unloading_when_idle` con `Option<Duration>`; `clawg serve` envuelve siempre.
- Commit: `refactor(core): wrap the model in IdleEmbedder also when it is never let go`

## 3. Recuperar un hilo muerto (R3, R4, R5, D2, D3)

- [x] Tests primero: `a_model_whose_worker_is_gone_is_loaded_again_within_the_call`, `calls_that_find_the_worker_gone_together_share_one_reload`, `a_model_that_keeps_dying_is_reloaded_at_most_once_a_minute`, la segunda mitad de `with_no_idle_time_the_model_stays_and_a_dead_worker_is_still_recovered` y `without_an_idle_time_the_loaded_model_is_still_wrapped` (`bootstrap.rs`), que pasó aquí desde la tarea 2: con sólo la envoltura, un modelo muerto sigue muerto, y no hay otra conducta que mirar desde `unloading_when_idle`.
- [x] `Slot`, `discard`, `with_model`, `RECOVERY_INTERVAL`.
- Commit: `fix(core): load the model again when its inference worker is gone`

## 4. Docs

- [x] `.claude/CLAUDE.md`, el mapa de `openspec/README.md` y `state.yaml`. `openspec/specs/storage/spec.md` («Ciclo de vida del embedder») se consolida al archivar.
- Commit: `docs: describe how the daemon recovers a dead inference worker`

## 5. Verificación

- [ ] Suite y clippy en Linux sobre el último commit.
- [ ] El CI en verde en Linux y Windows.

## 6. Archive

- [ ] `specs/storage` con R1 a R5; `git mv` a `changes/archive/AAAA-MM-DD-embedder-recovery`; `state.yaml` archivado; fila 61 y orden de ejecución del `README.md` de openspec; subir el minor.
