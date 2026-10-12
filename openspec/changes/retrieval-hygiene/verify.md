# retrieval-hygiene — Verificación

**Estado:** completa. Los tests pasan y las corridas de `rag-eval --real` dan diff vacío, como se esperaba.

## Tests

### Linux

`cargo test --workspace` en Linux, con el runtime de ONNX tomado del wheel `onnxruntime==1.24.2` de PyPI (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK=1`), porque la descarga de `cdn.pyke.io` estaba bloqueada.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 9a1b13e) | 1.394 | 12 |
| Con el cambio | 1.405 | 12 |

Son 11 tests nuevos y ninguna falla nueva. Las 12 fallas son las mismas en las dos corridas y vienen del entorno, no del código:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- `memory::workspace::tests::write_failure_returns_workspace_io_error`: espera no poder escribir en `C:\Windows\System32`; en Linux esa ruta es un directorio relativo que sí se puede crear.
- 3 de `diff::tests` y 1 de `report::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

### Windows

`cargo test --workspace` sobre el merge del cambio (da15581), en un worktree limpio: 1.418 pasan, 4 fallan.

Las 8 fallas propias de Linux pasan. Las 4 que quedan son las de `diff::tests` y `report::tests`, por la misma razón que en Linux: el worktree no tiene `docs/eval/results/`. Después de este cambio, e2dda9b hizo que esos tests generen sus propios archivos y no dependan de esa carpeta.

## rag-eval

Corridas con `--real` (multilingual-e5-small) sobre `docs/eval/golden.json`, compilando `rag-eval` en el commit anterior al cambio (126641c) y en el merge (da15581). Entre esos dos commits no cambia nada de `crates/clawg-eval`, `eval_api.rs` ni el golden set.

| Corrida | Resultado |
|---|---|
| `rag-eval --real --diff` | Diff vacío: 0 mejoras y 0 empeoramientos en todas las métricas de retrieval y similarity. |
| `rag-eval --real --agent-view --diff` | Diff vacío en retrieval y similarity. |

`--diff` no compara el bloque `agent_view` (filas visibles, costo en tokens). Por eso comparé además los dos JSON de agent-view campo por campo, sin contar fecha, tag, sha y latencias: son idénticos.

Esto confirma lo que se esperaba:

- El cambio no toca el ranking del core, y el eval llama a `store.search` directo.
- El daemon ahora hace el corte antes de responder, pero es el mismo corte que el puente aplicaba después. Aplicarlo dos veces da lo mismo, y las corridas de eval no registran accesos (`crates/clawg-core/src/eval_api.rs:461`).
