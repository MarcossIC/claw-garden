# serve-log-rotation — Verificación

## Linux (2026-10-22)

Sobre `4528b49`, en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- Lo que el CI corre para el build de release: `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` sale con 0, y `cargo test --locked -p clawg-core --lib build_info` pasa.
- Cada commit de código de las [tareas](tasks.md) (`b9a9b62`, `b41c289`) pasó la suite y clippy antes de commitearse.

## El binario real

Con `target/debug/clawg` de un build de tests, `CLAWG_TEST_HOME` en un directorio temporal, `CLAWG_TEST_EMBEDDER=fake` y `CLAWG_TEST_DEFAULT_PORT=off`, con un `serve.log` de 10 MiB más un byte y un `serve.log.1` de 6 bytes en `logs/`: un `clawg mcp` que autoarrancó su daemon dejó `serve.log.2` con los 6 bytes, `serve.log.1` con los 10 MiB más uno y un `serve.log` nuevo cuya primera línea es `clawg serve 0.49.0 starting, profile dev`. Ese home tenía una ruta de socket de más de 108 bytes, así que el daemon se negó a arrancar nombrándola, como pide [`specs/daemon-access`](../../specs/daemon-access/spec.md) R7; la negativa quedó en el `serve.log` nuevo, después de la línea de la versión. El autoarranque que termina escuchando lo cubre `an_autostart_rotates_a_full_serve_log`.

## RED antes de cada cambio

| Requisito | Cómo fallaba |
|---|---|
| R1 | `rotating_keeps_three_files`, `over_the_limit_each_file_moves_one_place`, `a_file_open_for_writing_still_rotates` y `a_failed_rotation_is_written_to_the_log` fallaron por la aserción contra una `rotation` y una `rotate` que no renombraban nada. `an_autostart_rotates_a_full_serve_log` no encontró `serve.log.1`. `nothing_rotates_up_to_the_limit` y `a_log_under_the_limit_is_appended_to` ya pasaban: guardan que un log bajo el tope sigue en append |
| R2 | `the_starting_line_names_the_version_and_the_build` no compilaba sin `starting_line`. `a_daemon_names_its_version_first` vio como primera línea la de las migraciones |

## Lo que no se midió

- Windows no se corrió a mano: `a_file_open_for_writing_still_rotates` y `a_failed_rotation_is_written_to_the_log` fijan allí el renombrado con el fichero abierto y el fallo, y el CI del pull request corre la suite y clippy en Linux y Windows.
- Un daemon que vive semanas pasa el tope sin rotar, por diseño; el próximo arranque lo corta.
