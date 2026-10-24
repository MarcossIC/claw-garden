# history-import — Verificación

## Windows (2026-10-23)

`cargo test --workspace --target-dir target-test` y el clippy de `.claude/CLAUDE.md` en `target-test`, sobre la rama del cambio antes del merge: pasan, ningún test falla, y clippy sale con 0 ([tareas](tasks.md), tarea 10).

## Linux (2026-10-24)

Sobre `develop` con el cambio mergeado (`9a9bc69`), en `target`, con el compilador de `rust-toolchain.toml`. El build no pudo bajar ONNX Runtime de `cdn.pyke.io` (la red lo rechazó), así que `ort-sys` se enlazó dinámicamente contra la misma versión, 1.24.2, sacada del wheel `onnxruntime` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1` y `LD_LIBRARY_PATH` a ese directorio). Ningún test usa el modelo real; el cambio no toca ese enlace.

- `cargo test --workspace --locked --no-fail-fast`: pasa, ningún test falla.
- `cargo clippy --workspace --all-targets --locked -- -A clippy::all -D warnings`, `cargo clippy --locked -p clawg-cli -- -A clippy::all -D warnings` y `cargo test --locked -p clawg-core --lib build_info`: salen con 0.
- `agent_contract.json` y `agent_contract.es.json` pasan sin bendecir: ninguna herramienta ni descripción del puente cambió.

## El binario real (2026-10-23)

El `clawg` de la suite con `CLAWG_TEST_HOME` en un directorio temporal y el modelo real descargado, sin tocar los perfiles:

- Sin el modelo cargado, `503 tokenizer_not_ready` también en `--dry-run`.
- Con él, un export sintético de 9 notas da el mismo reporte en el ensayo y en el import, y un segundo import encuentra las 9.
- Un export más nuevo con una nota de 1.740 palabras la parte en 33 piezas con el tokenizer real, enlazadas, y `/save` de la pieza 17 la devuelve entera; la nota en español reescrita entra como revisión (la vieja `superseded`, causa `revision`).
- Una decisión con un `supersedes` marcado por un agente queda viva y el par `escalated`, con motivo y evidencia; el secreto queda enmascarado, la clave reescrita, la borrada sin vector y `origin_project` guardado.
- Un segundo import sin nada que insertar igual escribe su snapshot `pre-import`, como el de un archivo de `clawg-memories`.

`an_engram_export_imports_through_the_binary` (`crates/clawg-cli/tests/binary.rs`) recorre lo mismo con el modelo falso en cada corrida de la suite.

## `rag-eval`

No aplica: la búsqueda y el ranking no cambian. Lo importado entra por el mismo camino que un save (piezas, cabecera, enmascarado) y el golden set no tiene filas importadas.

## Lo que no se midió

- El perfil dev, que pide autorización: el export real (`engram export --all`) con `--dry-run`, el import, una docena de búsquedas, cuánto tarda, si el daemon contesta mientras tanto y `projects move --origin-project`. Las 3.768 notas medidas en la [exploración](exploration.md) no se importaron con esta versión; el tiempo por nota en volumen y la espera de otras llamadas durante el import quedan sin número.
