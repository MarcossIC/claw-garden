# embedder-idle-unload — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (el daemon es de todos los objetivos) | L (M antes de [D5](design.md#d5-un-solo-tokenizer-por-proceso-la-inferencia-la-arma-clawg)) | — (después de ingest-outbox, 36, que vuelve durable el `202`, e install-smoke, 40) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md#premisas-que-se-verificaron); lo que sigue está verificado otra vez contra `main` (`84ee27b`).

1. **El modelo vive cargado toda la vida del daemon.** El bootstrap lo carga una vez y lo guarda en un `OnceCell` de una sola escritura (`crates/clawg-core/src/runtime.rs:107-115`); nada lo suelta antes de que el proceso termine. Ese modelo cargado es el piso de memoria del daemon ocioso: ~1 GB de bytes privados con el arreglo del arena ya aplicado.
2. **Soltarlo no alcanzaba en Linux.** La medición del paso 0 ([diseño, D1](design.md#d1-la-memoria-vuelve-medición)) cargó el modelo real cinco veces en un proceso y lo soltó: la memoria residente bajaba de ~1,0 GB a 625-730 MB y no más, porque glibc se queda con lo liberado en sus arenas. No crecía de una vuelta a la otra, así que no es una fuga del asignador; un `malloc_trim` la bajaba a 40-120 MB.
3. **En Windows la memoria crece con cada recarga** ([verify](verify.md#causa)). Cada carga de fastembed vuelve a parsear `tokenizer.json`, y el heap de Windows se queda con lo que libera ese parse. El daemon, además, tiene su propio parse del mismo archivo para el chunker: con el modelo cargado hay dos.

## Objetivo que sirve

Transversal: un proceso sin interfaz que corre todo el día no puede ocupar 1 GB mientras nadie le pide nada.

## Decisiones

Defaults tomados donde la exploración dejaba la pregunta abierta. Se pueden cambiar sin rehacer el diseño.

- **El paso 0 dice que la memoria vuelve**, con un trim en Linux: se suelta el modelo en el mismo proceso. No hace falta el proceso hijo (opción C).
- **Un solo tokenizer por proceso; la inferencia la arma clawg y fastembed se va** ([D5](design.md#d5-un-solo-tokenizer-por-proceso-la-inferencia-la-arma-clawg)). Es lo que hace que la memoria vuelva también en Windows: una recarga sólo vuelve a armar la sesión de ONNX Runtime. Los vectores son los mismos bit a bit.
- **Opción A2, el envoltorio.** `IdleEmbedder` queda en la celda toda la vida del proceso y sólo el modelo de adentro va y viene. La celda, `hold_for_embedder` y `EmbedderStatus` no cambian ([D2](design.md#d2-un-envoltorio-en-la-celda-a2)).
- **Tiempo de ocio: 10 minutos**, configurable con `CLAWG_EMBEDDER_IDLE_MINUTES`; `0` lo deja cargado como hoy. Una variable y no un flag de `serve`, porque el daemon lo lanza el puente y hereda su entorno. Un valor inválido avisa en el log y usa 10: no impide arrancar.
- **Un guardado con el modelo suelto espera la recarga** como cualquier embedding. Si pasa de 10 s responde `202 unconfirmed`, que desde 36 ya está en el outbox. Si la recarga falla, el save responde `503 embedder_unavailable` cuando todavía se lo esperaba, o queda pendiente en el outbox (fallo transitorio, `crates/clawg-core/src/memory/outbox.rs:74-78`) cuando ya se había respondido; nunca queda `failed` por eso.
- **`recall_context` con el modelo suelto lo despierta y espera.** No degrada en silencio: la exploración lo marcaba como el riesgo más visible.
- **Estado visible: un campo aditivo `embedder_resident` en `/health`**, con `embedder` en `ready`. Un puente viejo lo ignora y uno nuevo contra un daemon viejo lee `null`.
- **El SHA-256 se verifica en cada recarga**, igual que al arrancar. Es la opción que no pierde ninguna garantía; el log de fases dice cuánto cuesta, y si pesa se decide aparte.
- **El despertar (opción B) va en `POST /v1/projects`**, sin esperar la carga.

## Alcance

1. `crates/clawg-core/src/memory/idle_embedder.rs`: el envoltorio, la decisión pura de soltar, el barrido y la lectura de la variable.
2. `Embedder::resident` y `Embedder::wake`, con cuerpo por defecto para los embedders que nunca sueltan nada.
3. `E5Embedder` en lugar de `FastEmbedLocal`: la sesión de ONNX Runtime con las opciones de hoy, el tokenizer compartido, la ventana y el relleno a mano, el pooling por media y la normalización; devuelve al sistema lo que liberó la sesión al terminar su hilo (glibc). `fastembed` sale del workspace, y `ort` y `tokenizers` declaran las features que traía.
4. `bootstrap::unloading_when_idle`: la recarga por los mismos `ensure` y `load` del arranque, con el log de fases.
5. `serve`: la variable y el envoltorio; `/health` con `embedder_resident`; `POST /v1/projects` despierta; una recarga fallida responde `503 embedder_not_ready` en búsqueda y restauración.
6. El tokenizer compartido: el worker y la carga del embedder llenan la misma celda una sola vez, de bytes con el SHA-256 verificado.
7. El test de los vectores con el modelo (`crates/clawg-core/tests/e5_golden.rs`) y su fixture, generado con fastembed antes del cambio.
8. Docs: `.claude/CLAUDE.md` y el mapa de `openspec/README.md`.

## Fuera de alcance

- **El proceso hijo (C)**: el paso 0 no lo pide.
- **Un modelo más chico (E)**: cambia vectores; es model-reembed (17).
- **Saltear el SHA-256 en las recargas**: cambia una garantía; queda para cuando el log de fases muestre que pesa.
- **El despertar desde el hook de inicio de sesión**: es agent-lifecycle-hooks (25), que puede llamar a `POST /v1/projects`.
- **El aviso explícito de `recall_context`** mientras el modelo no está: es context-budget (23). Con este cambio ese caso ya no aparece por ocio.
- **`search_project` con una recarga fallida** sigue respondiendo `500` con la causa: el error del embedder llega como texto (`DocIndexError::Query`, `crates/clawg-core/src/memory/project_docs/mod.rs:62`).
- **El chunker del primer arranque** sigue partiendo por bytes hasta el próximo arranque, como hoy: el worker lo arma antes de que exista `tokenizer.json`.

## Cómo se mide el éxito

- Tests que fallan hoy: los 15 de `idle_embedder.rs` y `bootstrap.rs`, los 5 de `tests/api.rs` y el del cliente.
- **Paso 0 en Linux** (hecho, [D1](design.md#d1-la-memoria-vuelve-medición)): tras soltar el modelo, la memoria residente queda por debajo de 300 MB y no crece entre la primera y la quinta vuelta. Resultado: 41-60 MB en las cinco.
- **En Windows y en Linux** ([D5](design.md#r7-sobre-lo-medido)): con el modelo suelto, la mediana de la memoria privada en 10 recargas queda a no más de 64 MB sobre el daemon con sólo el tokenizer, la de las últimas cinco no supera la de las primeras cinco en más de 16 MB, y el log muestra un solo parse de `tokenizer.json`. Con fastembed, en Windows no se cumplía ([verify](verify.md#resultado)).
- **Los vectores son los mismos bit a bit:** el test con el modelo contra el fixture generado con fastembed, y `rag-eval --diff` con `--real` antes y después, sin ningún caso movido.

## Datos y vuelta atrás

No toca el esquema, los identificadores, los estados ni el contrato de embedding. Una recarga que trae otro `model_id` o otra dimensión se rechaza, así que nunca se mezclan espacios. Para volver al comportamiento de antes alcanza con `CLAWG_EMBEDDER_IDLE_MINUTES=0`. Volver a fastembed es revertir [D5](design.md#d5-un-solo-tokenizer-por-proceso-la-inferencia-la-arma-clawg): como los vectores son los mismos, el almacén no se entera.
