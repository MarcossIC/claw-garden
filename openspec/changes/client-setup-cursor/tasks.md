# client-setup-cursor — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Lo que se mueve sin conducta nueva lo fijan los tests que ya hay, que pasan antes y después sin cambiar lo que esperan. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia y con `crates/clawg-cli/tests/binary.rs` cuando toca un comando que ese archivo corre; la suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea.

Requisitos en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

Lo que se hizo distinto de lo previsto está en [«Frente a lo previsto»](verify.md#frente-a-lo-previsto) de la verificación.

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `592a6e0`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose client-setup-cursor`

## 1. Lo común de `setup`, para dos clientes

Sin conducta nueva: los tests de `setup.rs` pasan sin cambiar lo que esperan.

- [x] La escritura de los archivos de un plan (copia, temporal, poda) en una función que no conoce a `claude`; el renglón de las copias y los fallos del reporte, en otra.
- [x] La palabra de shell de `legacy_command` en su propia función; `split_command`, `is_clawg_program`, `parse_object`, `matches_server`, `is_clawg_server` y `describe_command` al alcance del módulo de Cursor; el error de un archivo con otra forma nombra al cliente.
- Commit: `refactor(setup): share the file writing and the command words between clients`

## 2. El hook en el formato de Cursor (CU6 a CU8)

- [x] Tests en rojo: los de CU6 a CU8 en `cli.rs`, `hook.rs` y `tests/binary.rs` (no compilan: no hay `HookClient` ni `--client`).
- [x] `HookClient` y `--client` en `cli.rs`; en `hook.rs`, `workspace_roots` y `cursor_version` en `HookInput`, la carpeta según el cliente, la salida de Cursor y el silencio de CU8; `main.rs` pasa el cliente.
- Commit: `feat(hook): answer Cursor's sessionStart with --client cursor`

## 3. `clawg setup cursor` (CU1 a CU5)

- [x] Tests en rojo: los de CU1 a CU5 en `cli.rs`, `setup/cursor.rs` y `tests/binary.rs` (no compilan: no hay `SetupClient::Cursor` ni el módulo).
- [x] `setup/cursor.rs` con `State`, `plan`, `apply`, `report`, `run_in` y `run`; `SetupClient::Cursor`; el despacho en `main.rs`.
- Commit: `feat(setup): add clawg setup cursor`

## 4. Docs

- [x] README: conectar Cursor con `clawg setup cursor` en lugar del `mcp.json` a mano, y desinstalarlo.
- [x] `docs/reference/commands.md`: `clawg hook --client` y `clawg setup cursor`.
- Commit: `docs: describe clawg setup cursor and the Cursor hook`

## 5. Verificación

- [x] `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -D warnings` en Linux, sobre el último commit.
- [x] `verify.md`, con lo que queda pendiente en local con Cursor real; `state.yaml`, la fila 115 de `openspec/README.md` y la línea de `openspec/ROADMAP.md`.
- [x] El CI del pull request en Windows, y lo que marque, arreglado: pasó sin ningún arreglo.
- Commit: `docs(openspec): record client-setup-cursor applied`
