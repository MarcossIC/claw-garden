# project-clone-identity — Verificación

## Linux (2026-10-23)

Sobre `681baec`, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace --locked`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings`, `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info`: salen con 0.
- Cada commit de código de las [tareas](tasks.md) pasó la suite y clippy antes de commitearse.
- `daemon::serve::tests::a_child_started_while_the_port_is_bound_does_not_keep_it` falló dos veces en corridas enteras de la suite (`AddrInUse` al volver a tomar el puerto) y pasó sola y en otras cuatro corridas de `clawg-cli --lib`. No toca nada de este cambio (un `TcpListener` y un `sleep` hijo); queda como inestable, para mirar aparte.
- `agent_contract.json` y `agent_contract.es.json` no cambian: ninguna herramienta ni descripción cambió.

## El binario real

`two_clones_share_their_memories_through_a_real_daemon` (`crates/clawg-cli/tests/binary.rs`) corre el `clawg` de la suite con `CLAWG_TEST_HOME` en un directorio temporal y `CLAWG_TEST_EMBEDDER=fake`: dos carpetas `work/app` y `review/app` con un `.git/config` cuyo `origin` es `git@github.com:acme/app.git`, un `clawg serve --profile dev --port 0`, el registro de `work/app` y un save en su proyecto por HTTP; después `clawg hook session-start` con el `cwd` de `review/app` imprime ese save en su contexto, y `clawg memories list --here` desde `review/app` lo lista.

## RED antes de cada cambio

Los tests se escribieron junto con el código, en el mismo commit, y se corrieron después de escribirlo; no se vio a cada uno fallar contra el código anterior. Lo que los ata al cambio: los de `identity.rs`, `relocate.rs` (los `reconcile_*`, `decide_merge_*`, `merging_*`) y `project.rs` usan funciones y campos que no existían (`origin_key`, `normalize_remote`, `candidate_id`, `merge_in`, `Registration`, `RegisterOutcome::root_id`), así que no compilaban; `a_database_from_the_previous_version_opens_with_no_remote_recorded` lee tablas que V23 no tiene; los de HTTP mandan `assigned` y leen `root_id`, que un daemon anterior ignora o no manda; `two_clones_of_one_remote_share_their_memories`, `a_colliding_root_gets_a_project_of_its_own` y el del binario real fallan con el puente anterior por la aserción, porque cada clone usaba su propio id derivado y una raíz que choca daba 409.

## `rag-eval`

No aplica: la búsqueda y el ranking no cambian. El golden set usa un proyecto fijo, que no se registra.

## Lo que no se midió

- Windows no se corrió a mano: el CI del pull request corre la suite y clippy en Linux y Windows. `origin_key` no tiene código propio de Windows; `strip_verbatim` sigue siendo lo único que difiere.
- El perfil dev, que pide autorización: abrir con esta versión un store con proyectos de dos clones, ver el `pre-rekey`, la línea del log con el número del movimiento y `clawg projects move --undo N`, y medir cuánto tarda unir un proyecto de miles de filas en la conexión única del daemon (un `UPDATE` y una fila de registro por fila, sin vectores).
- `clawg doctor` sobre un store después de unir: la reconciliación deja una fila viva por clave y por texto en los grupos que toca, así que `memory.topic` y `memory.content_hash` no deberían ganar hallazgos por la unión; no hay un test que corra el doctor después.
- Un puente abierto antes de actualizar, junto a un hook nuevo que une su raíz a otro clone: ese puente deja de ver las filas que se movieron hasta que se reinicia ([`design.md`](design.md), D3). No hay test de esa ventana.
