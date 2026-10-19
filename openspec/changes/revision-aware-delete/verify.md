# revision-aware-delete — Verificación

## Linux

- `cargo test --workspace`: pasa entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- El contrato del agente se bendijo con `CLAWG_BLESS=1`; el diff cambia sólo las descripciones de `delete_memory` (borrar la vigente de una `topic_key` trae la versión anterior) y `restore_memory` (el rechazo por clave y los pares nuevos).
- Cada requisito de la [spec](spec.md) tiene su test, escrito antes que el código. Los de R1 a R8 fallaron primero por la aserción o porque no compilaban.

## Binario real, home temporal

`clawg serve --profile dev --port 0` desde la build de prueba, con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`, sin tocar `~/.clawg*`:

1. Dos saves con `topic_key` `storage/engine`: el segundo responde `replaced: [#1]`.
2. `DELETE` de #2 responde `revived: [#1 "Storage engine is SQLite"]`, y #1 vuelve `active`.
3. `POST …/2/restore` responde `topic_key` y `replaced: [#1]`, y #1 vuelve a quedar `superseded` por #2.
4. `clawg undelete 2`, después de otro borrado, imprime `restored memory #2 (topic storage/engine)` y `superseded again: #1 …`.

## Lo que encontró la verificación

- Borrar pieza por pieza un save largo con clave no revivía nunca a la versión anterior: las filas reemplazadas apuntan a la primera pieza, y borrarla mientras las otras siguen vivas no revive nada. Hasta entonces sólo lo arreglaba el barrido. Ahora el borrado de la última pieza aplica la regla también a la primera (`ee3bd01`).

## Pendiente

- La suite y clippy en Windows, en `target-test`.
- El perfil dev con el puente MCP real y el modelo real: un agente borra la versión vigente de un tema, la ve volver en `search_memory`, y restaura.
- `rag-eval` no aplica: el ranking no cambia y el golden set no tiene claves ni borrados.
