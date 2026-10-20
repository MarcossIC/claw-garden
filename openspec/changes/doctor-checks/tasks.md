# doctor-checks — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## 1. Hechos del store (R3, R4, R6, D1)

- [ ] Tests primero en `inspect.rs`: el store no cambia, esquema anterior, esquema más nuevo, base sana y cada invariante; `snapshot_times_come_from_their_names` en `backup.rs`.
- [ ] `memory::inspect`, `backup::parse_snapshot_time` y `embedder::expected_contract`.
- Commit: `feat(core): read the facts doctor checks without the lock`

## 2. El sobre y los chequeos (R1, R2, R4, R5, R6, R7, R8, D2–D4)

- [ ] Tests primero en `doctor`: sobre, resumen, código de salida y cada chequeo sano y roto.
- [ ] `doctor/` como módulo: `Finding`, los chequeos puros, `mcp`, la lectura en `run`, y el código de salida en `main.rs`.
- Commit: `feat(cli): doctor reports what is wrong with the data directory and the store`

## 3. El binario (R1, R2, R3)

- [ ] Tests en `binary.rs`: `doctor` con el daemon vivo no cambia el store y sale 0; no imprime contenido de filas.
- Commit: `test(cli): run doctor against a live daemon's store`

## 4. Docs

- [ ] `.claude/CLAUDE.md` y estas tareas.
- Commit: `docs: say what doctor checks`

## 5. Verificación

- [ ] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [ ] El CI en verde en Linux y Windows.

## 6. Archive

- [ ] `specs/doctor` con R1–R8; `git mv` a `changes/archive/AAAA-MM-DD-doctor-checks`; `state.yaml` archivado; fila 31, el backlog que resuelve y el orden de ejecución del `README.md` de openspec; subir el minor.
