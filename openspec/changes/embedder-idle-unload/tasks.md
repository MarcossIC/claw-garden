# embedder-idle-unload — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional. La suite completa corre una vez, sobre el último commit (regla del proyecto).

## 0. Paso 0: ¿la memoria vuelve? (D1)

- [x] Cinco cargas y descargas del modelo real en Linux, con y sin trim. Sin trim queda en ~700 MB; con trim, 41-60 MB.

## 1. Devolver la memoria del modelo (R7, D1)

- [x] `return_freed_memory` al terminar el hilo `fastembed-worker` (`malloc_trim` en Linux con glibc; nada en el resto). Sin test automático: lo mide el paso 0.
- Commit: `fix(core): give the memory a dropped model freed back to the system on glibc`

## 2. El envoltorio (R1, R2, R3, R6, D2, D3)

- [x] Los 14 tests de `idle_embedder.rs` y `an_idle_unloading_embedder_reloads_through_the_same_files_and_load` (`bootstrap.rs`): no compilan.
- [x] `IdleEmbedder`, `Embedder::resident`/`wake`, `idle_after`, `bootstrap::unloading_when_idle`.
- Commit: `feat(core): let the embedding model go after a stretch without use and load it again on demand`

## 3. El daemon (R4, R5, R6, D4)

- [x] Los 5 tests de `tests/api.rs` y `a_health_without_residency_still_parses` (`client.rs`): no compilan.
- [x] `serve` con `CLAWG_EMBEDDER_IDLE_MINUTES`, `embedder_resident` en `/health`, `wake` en `POST /v1/projects`, `503 embedder_not_ready` en búsqueda y restauración.
- Commit: `feat(cli): unload the idle model in the daemon, report it in health and wake it on project registration`

## 4. Docs

- [x] `.claude/CLAUDE.md`, el mapa de `openspec/README.md` y este cambio.
- Commit: `docs: describe how the daemon lets the idle embedding model go`

## 5. Verificación

- [x] Suite y clippy en Linux sobre el último commit (2.065 pasan; clippy con código 0).
- [ ] El CI en verde en Linux y Windows.
- [ ] Bytes privados del daemon dev en Windows antes y después del tiempo de ocio (máquina de Marcos).
