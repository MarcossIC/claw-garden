# project-reassign — Verificación

## Linux (2026-10-22)

Sobre `41997a8` y con la versión subida a 0.52.0, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace --locked`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings` y `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings`: salen con 0.
- Cada commit de código de las [tareas](tasks.md) (`fda2d45`, `981346d`, `fd3fb2b`, `7d46290`, `ee651cf`) pasó la suite y clippy antes de commitearse.
- `the_agent_contract_matches_its_file` se bendijo con `CLAWG_BLESS=1` y el diff se revisó: cambian sólo la descripción de `update_memory`, sus argumentos nuevos `to_project_path` y `clear_project` y la descripción de `user_answered`. Las instrucciones del servidor no cambian (la línea nueva de `protocol.md` está después del primer `##`).

## El binario real

`projects_move_moves_and_undoes_a_seeded_store` (`crates/clawg-cli/tests/binary.rs`) corre el `clawg` de la suite con `CLAWG_TEST_HOME` en un directorio temporal y `CLAWG_TEST_EMBEDDER=fake`: un `clawg serve --profile dev --port 0`, dos saves sin proyecto por HTTP, y `clawg projects move` como proceso aparte sobre el socket: `--from-none --to <repo> --dry-run` informa dos filas y no mueve nada; sin `--dry-run` las mueve, nombra el snapshot `pre-move` y el número del movimiento, y el listado las muestra en el proyecto del repo; `--undo 1` las devuelve a sin proyecto.

## RED antes de cada cambio

Sólo V21 se vio fallar antes del código: `a_database_from_the_previous_version_opens_with_an_empty_move_log` porque `memory_move` no existía, y después `published_migration_checksums_are_pinned` por la fila que faltaba. En los demás commits los tests se escribieron junto con el código, en el mismo commit, y no se corrieron solos contra el código anterior. Lo que los ata al cambio: cada uno usa una firma o una ruta que no existía (`relocate.rs` entero, `MemoryPatch::new` con destino, `UpdateRequest.project_id`, `ListParams.no_project`, `POST /v1/memories/move`, `ProjectsAction::Move`, `UpdateMemoryArgs.to_project_path`), así que no compilaban o respondían 404/422 contra la versión anterior.

## `rag-eval`

No aplica: la búsqueda y el ranking no cambian. Mover una fila cambia en qué proyecto se la busca, no cómo.

## Lo que no se midió

- Windows no se corrió a mano: el CI del pull request corre la suite y clippy en Linux y Windows.
- El perfil dev, que pide autorización, y con él el caso que motiva el cambio: importar con history-import (44), mirar el grupo sin proyecto con `clawg memories list --no-project`, `projects move --dry-run`, mover, buscar desde el destino y desde otro proyecto, medir cuánto tarda un lote de miles de filas en la conexión única del daemon y deshacerlo. Las unidades del store de prueba son de pocas filas; un lote grande es una transacción de `UPDATE` y de inserciones en el registro, sin vectores, y si pesa se parte en tandas.
- `clawg doctor` sobre un store después de mover: el motor rechaza todo lo que dejaría dos titulares de una clave o dos filas vivas de un texto en un proyecto (para el usuario, contra toda fila; para un agente, contra lo que alcanza, como un save), así que `memory.topic` y `memory.content_hash` no ganan hallazgos; no hay un test que corra el doctor después de un movimiento.
- El chequeo de `doctor` para filas bajo un id que no está en `project`, que la exploración dejaba como candidato: quedó fuera del alcance.
