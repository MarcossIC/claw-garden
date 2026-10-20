# doctor-checks — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Hechos del store (R3, R4, R6, D1)

- [x] Tests en `inspect.rs`: el store no cambia, esquema anterior, esquema más nuevo, base sana y cada invariante; `snapshot_times_come_from_their_names` en `backup.rs`; `the_expected_contract_is_what_the_embedder_records` en `embedder.rs`. Se escribieron junto con el código, en el mismo commit, y no se vieron fallar antes.
- [x] `memory::inspect`, `backup::parse_snapshot_time` y `embedder::expected_contract`.
- Commit: `feat(core): read the facts doctor checks without the lock`

## 2. El sobre y los chequeos (R1, R2, R4, R5, R6, R7, R8, D2–D4)

- [x] Tests en `doctor`: sobre, resumen, código de salida y cada chequeo sano y roto (escritos con el código, como en la tarea 1).
- [x] `doctor/` como módulo: `Finding`, los chequeos puros, `mcp`, la lectura en `run`, y el código de salida en `main.rs`.
- Commit: `feat(cli): doctor reports what is wrong with the data directory and the store`

## 3. El binario (R1, R2, R3)

- [x] Tests en `binary.rs`: `doctor` con el daemon vivo lee el store y sale 0, y con el daemon parado no cambia ningún archivo; no imprime contenido de filas. El primero falló primero: con el modelo falso el contrato salía `mismatch`, porque `doctor` comparaba con el de producción y no con el que verifica el daemon de ese build.
- [x] `bootstrap::serving_contract`, el contrato que verifica `clawg serve`, y `doctor` compara con él.
- Commit: `feat(cli): doctor checks the contract the daemon verifies and reads a live store`

## 4. Docs

- [x] `.claude/CLAUDE.md`, `.gemini/GEMINI.md`, `README.md`, la cita de `doctor` en `specs/install` y estas tareas.
- Commit: `docs: say what doctor checks`

## 5. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 6. Archive

- [x] `specs/doctor` con R1–R8; `git mv` a `changes/archive/2026-10-20-doctor-checks`; `state.yaml` archivado; fila 31, el backlog que resuelve y el orden de ejecución del `README.md` de openspec; subir el minor.
