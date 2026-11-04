# hook-run-log — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia y con `crates/clawg-cli/tests/binary.rs` cuando toca un comando que ese archivo corre; la suite y los dos clippy corren una vez, en el último commit de código. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea.

Requisitos en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## 0. Antes del código

- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`; la fila 118 de `openspec/README.md` y la línea de `openspec/ROADMAP.md`.
- Commit: `docs(openspec): propose hook-run-log`

## 1. El registro del hook (HR1 a HR5)

- [x] Tests en rojo: los de HR1 a HR5 en `hook_log.rs`, `hook.rs` y `tests/binary.rs` (no compilan: no hay `hook_log` ni `respond_with_run`).
- [x] `hook_log.rs` con `Run`, `Outcome`, la línea, el agregado con rotación y la lectura tolerante; en `hook.rs`, el motivo de cada fin de `answer` y `respond_with_run`, a la que `respond` le delega; `main.rs` registra antes de imprimir.
- Commit: `feat(hook): record each session start`

## 2. `clawg doctor` muestra las corridas (HR6 y HR7)

- [x] Tests en rojo: los de HR6 y HR7 en `checks.rs`, `setup/cursor.rs` y `tests/binary.rs` (no compilan: no hay `hook_runs` ni `hook_installed`).
- [x] `checks::hook_runs`, pura sobre las corridas, si el hook está instalado, la hora y el perfil; `setup::cursor::hook_installed`; la lectura en `doctor/mod.rs`.
- Commit: `feat(doctor): show the hook's recent session starts`

## 3. Docs

- [x] `docs/reference/commands.md` y `docs/reference/data-profiles.md`.
- [x] README, «Conectarlo a Cursor»: cómo comprobar que anduvo y qué pegar si algo no se ve bien.
- Commit: `docs: describe the hook run record`

## 4. Verificación

- [x] `cargo test --locked --workspace --no-fail-fast --target-dir target-test` y los dos clippy, sobre el último commit de código.
- [x] Una corrida a mano con un `CLAWG_TEST_HOME` temporal: una raíz con un correo falso, dos raíces y sin daemon; el archivo y la salida de doctor en `verify.md`.
- [x] `verify.md`, `state.yaml` y las referencias de [client-setup-cursor](../archive/2026-11-04-client-setup-cursor/verify.md) a este cambio.
- [ ] El CI de Windows del pull request, que todavía no existe.
- Commit: `docs(openspec): record hook-run-log applied`
