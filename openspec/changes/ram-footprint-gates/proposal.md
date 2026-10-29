# ram-footprint-gates — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M (C y F); B y E esperan la segunda pasada | 90, archivado | ver [`state.yaml`](state.yaml) |

## Problema

La [primera pasada](exploration.md#primera-pasada-prod-sin-hooks-2026-10-24-a-2026-10-29) se midió en prod, sin hooks. De ella salen dos costos que no dependen de los hooks:

1. **Cada carga del modelo lleva el proceso a ~1,6 GiB**, unos 830 MiB por encima del modelo cargado. Es el máximo del proceso. Pasa al arrancar y unas 11 veces por día. Una parte es el ONNX entero en un `Vec` (448,5 MiB): `read_verified` lo lee a memoria para hashearlo (`crates/clawg-core/src/memory/model_download.rs:605-613`), `ModelFiles::onnx` lo lleva (`:301`) y `build_session` se lo pasa a `commit_from_memory` (`crates/clawg-core/src/memory/embedder.rs:524`). El `Vec` sigue vivo mientras ONNX Runtime arma la sesión.
2. **El refresco de docs de cada inicio de sesión vuelve a embeber archivos enteros.** `refresh` trocea un doc que cambió y embebe todos sus pasajes (`crates/clawg-core/src/memory/project_docs/mod.rs:441-453`), aunque la mayoría no haya cambiado. Los refrescos de más de 1.700 pasajes tuvieron el proceso en ~1,16 GiB de 2 a 6,5 min. Sobre los 62 intervalos entre refrescos reales de prod, el 68,4 % de los pasajes reembebidos tenía el mismo `indexed_text` que el índice ([«F»](exploration.md#f-el-refresco-de-docs-reutiliza-los-pasajes-que-no-cambiaron)).

## Objetivo

Transversal: que una carga y un refresco no pidan más memoria ni más tiempo que lo que su trabajo necesita. No cambia qué se guarda, cómo se busca, los vectores ni el contrato de embedding.

## Decisiones

1. **C verifica `model.onnx` leyéndolo de a partes y lo carga por su ruta** (`commit_from_file`, que ort 2.0.0-rc.12 tiene: `session/builder/impl_commit.rs:45`). Desde el hash hasta que la sesión está armada, el archivo queda abierto. En Windows se abre sin compartir escritura ni borrado (`share_mode`). Así nadie lo puede escribir, renombrar ni borrar entre la verificación y la carga. Al armarse la sesión, o si la carga falla, el archivo se suelta y una reparación puede reemplazarlo. En Unix, la guarda es el directorio `0700`. Esta ventana se aceptó en la exploración.
2. **La corrida que baja el ONNX lo carga como una recarga.** Suelta los bytes bajados y vuelve a verificar el archivo desde la caché. Sólo en esa corrida, el ONNX se hashea dos veces. Los cuatro archivos del tokenizer siguen como hoy.
3. **F reutiliza el vector de cada pasaje de un doc cambiado** cuyo `indexed_text` ya está en el índice de ese doc con el mismo `model_id`. Embebe sólo los demás. El doc se sigue reemplazando entero en una transacción (`replace_doc`, `mod.rs:544`). Un doc de otro modelo no reutiliza nada, ni un archivo movido, porque la ruta está en el `indexed_text`.
4. **`RefreshReport` cuenta los pasajes reutilizados (`reused`),** y la línea `indexed the docs of …` los muestra (`crates/clawg-cli/src/daemon/api.rs:2675`). Con eso, la segunda pasada mide F en prod.
5. **C y F son commits propios** y cada uno puede salir solo.

## Alcance

1. C: el hash por partes, el archivo abierto hasta la sesión, `commit_from_file`, y la corrida que baja.
2. F: la reutilización por `indexed_text` y `model_id`, y `reused` en el reporte y en el log.
3. Los cambios de spec:
   - [`specs/storage`](../../specs/storage/spec.md), «Un hash por archivo, fuera de los hilos del runtime», hoy exige que la carga reciba los mismos bytes que se hashearon.
   - [`specs/project-docs`](../../specs/project-docs/spec.md), «Refresco incremental».
4. `.claude/CLAUDE.md` y la fila 92 de `openspec/README.md`.

### No-alcance

- B y E: los decide la segunda pasada, con hooks.
- D: no pasó su puerta, y queda para import-job (87).
- La deriva del piso y el crecimiento con el modelo siempre cargado: son mediciones aparte, con autorización.
- El orden de las tandas del índice de docs (62) y el tokenizer (91).

## Cómo se mide

Tests que fallan sin el cambio:

1. **C.** La comprobación de la caché devuelve el ONNX verificado sin sus bytes. Un `model.onnx` alterado se sigue rechazando. En Windows, mientras la carga lo tiene, el archivo no se puede escribir, renombrar ni borrar. Después de la carga, sí. Además, ONNX Runtime arma una sesión con `commit_from_file` desde un ONNX mínimo que la verificación tiene abierto.
2. **F.** Si se edita una sección de un doc indexado, el refresco embebe sólo los pasajes que cambiaron, el índice queda como lo deja un reindexado completo y el reporte cuenta los reutilizados. Con otro modelo, no reutiliza ninguno.

**La puerta propia de C**, fijada antes de medir: diez cargas con `commit_from_memory` y diez con `commit_from_file`, cada una en un proceso nuevo, con el modelo real bajado a un directorio temporal sin tocar ningún perfil. Se lee `PeakPagefileUsage` en Windows. C se queda si la mediana del pico baja al menos 300 MiB. Si no baja (porque ONNX Runtime lee el archivo entero a un buffer propio), C se revierte y se cierra con lo medido.

La segunda pasada, con hooks, lee en prod el pico de cada carga y `reused` en cada refresco.

## Datos

No toca `memory.db`, el esquema, los ids, los estados ni el contrato. F lee lo que el índice de docs ya guarda (`project_doc_chunk`, `project_doc_vectors`), que se deriva del disco y se puede rehacer. Volver atrás es revertir los commits: el índice que dejó F es el mismo que dejaría un reindexado.
