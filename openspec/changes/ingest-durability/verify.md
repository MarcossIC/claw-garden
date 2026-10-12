# ingest-durability — Verificación

**Código:** PR #6, una tarea por commit, más el merge con data-preservation.
**Estado:** los doce requisitos tienen su test en verde, y la medición y la prueba manual están hechas.

## Suite

`cargo test --workspace --no-fail-fast` en Linux, con data-preservation ya en `main`:

| | Passed | Failed |
|---|---|---|
| PR #6 | 1464 | 11 |

- **Los 11 que fallan, fallan igual en `main`.** Son tests que sólo pasan en Windows: `doctor::tests::*` (7) y los fixtures de `rag-eval` en `diff`/`report` (4). Ninguno toca este cambio.
- **La base de 1304 de `config.yaml`** se midió en Windows. Falta correrla en Windows para comparar contra esa cifra.
- **`rag-eval`:** no aplica, no cambia ranking ni recuperación.

## Requisitos

La tabla de trazabilidad de `spec.md` nombra el test de cada requisito; todos pasan. R4 suma `a_save_whose_reply_is_dropped_answers_202_unconfirmed`, que cubre la respuesta soltada sin contestar (el caso del volcado al apagar).

## Latencia del save (tareas 0.2 y 10.2)

Perfil dev con el modelo real (`multilingual-e5-small`), binario debug, saves secuenciales, 100 por tipo. "Corto" es un texto de 25 palabras (1 chunk); "largo", uno de 700 (5 chunks). "Fila" es el tiempo desde el `POST` hasta que la fila aparece en `GET /v1/memories`.

| | Respuesta p50 | Respuesta p99 | Fila p50 | Fila p99 |
|---|---|---|---|---|
| Antes, corto (`202`) | 1 ms | 3 ms | 41 ms | 129 ms |
| Antes, largo (`202`) | 2 ms | 5 ms | 977 ms | 1186 ms |
| Después, corto (`201`) | 32 ms | 51 ms | 35 ms | 53 ms |
| Después, largo (`201`) | 546 ms | 702 ms | 548 ms | 705 ms |

- El save confirmado tarda lo mismo que antes tardaba la fila en existir: la respuesta ahora espera esa escritura.
- **El timeout de 10 s queda:** el p99 de un texto de 5 chunks es 0,7 s, con más de 10 veces de margen.
- El "antes" de la fila incluye el sondeo de `GET /v1/memories` cada 2 ms, que compite con el worker por la base. Por eso su número es más alto que el "después", no porque el worker se haya vuelto más rápido.
- Sin build de release (necesita autorización). En release el chunker y la tokenización son más rápidos, así que estos números son una cota alta.

## Prueba manual (tarea 10.3)

En el perfil dev de un entorno aislado, no en una máquina de usuario. El binario fue el debug.

1. **Sin red en el primer arranque.** El daemon apuntó a un servidor local que respondía `503` en lugar de huggingface (`CLAWG_HF_BASE_URL`). `clawg status` mostró `embedder: "failed"`, `embedder_error: "retry budget exhausted for onnx/model.onnx after 3 attempts"` y `embedder_retryable: true`. Un save respondió `503 embedder_failed` con esa causa.
2. **Vuelve la red.** El servidor local pasó a reenviar a huggingface. En el siguiente reintento (30 s) el modelo se descargó y verificó, y `clawg status` mostró `ready` con el mismo pid: sin reiniciar.
3. **`save_memory` por MCP** (`clawg mcp --no-autostart`) respondió `saved #1`. El mismo texto otra vez respondió `already stored as #1; reinforced it instead of saving a copy`.
4. **`delete_memory`** con `kind: memory, id: 1` respondió `deleted 1`. Repetirlo dio `404`, y `list_memories` quedó vacío.
