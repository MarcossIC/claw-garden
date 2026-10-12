# retrieval-hygiene — Verificación

**Estado:** parcial. Los tests están; las corridas de `rag-eval --real` quedan pendientes.

## Tests

`cargo test --workspace` en Linux, con el runtime de ONNX tomado del wheel `onnxruntime==1.24.2` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1`), porque la descarga de `cdn.pyke.io` estaba bloqueada.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 9a1b13e) | 1.394 | 12 |
| Con el cambio | 1.405 | 12 |

Son 11 tests nuevos y ninguna falla nueva. Las 12 fallas son las mismas en las dos corridas y vienen del entorno, no del código:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- `memory::workspace::tests::write_failure_returns_workspace_io_error`: espera no poder escribir en `C:\Windows\System32`; en Linux esa ruta es un directorio relativo que sí se puede crear.
- 3 de `diff::tests` y 1 de `report::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

La base del repo (1.304) está medida en Windows con esos archivos presentes; el conteo de acá no es comparable con ese número, sí entre las dos corridas.

## rag-eval

Pendiente: `--real` necesita el modelo multilingual-e5-small, que no se pudo descargar. Lo que se espera:

- `rag-eval --real --diff`: diff vacío. El cambio no toca el ranking del core, y el eval llama a `store.search` directo.
- `rag-eval --real --agent-view --diff`: diff vacío. El daemon ahora corta antes de responder, pero el corte es el mismo que el puente aplicaba después, y aplicarlo dos veces da lo mismo. Las corridas de eval no registran accesos (`crates/clawg-core/src/eval_api.rs:461`).

Si alguna de las dos corridas muestra diferencias, es un bug de este cambio.
