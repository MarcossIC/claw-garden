# conclusion-upkeep — Verificación

## Linux

- `cargo test --workspace`: pasa entera.
- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: sale con 0.
- El contrato del agente se bendijo con `CLAWG_BLESS=1`; el diff cambia sólo la descripción de `synthesize` (lista también las conclusiones cuyas fuentes se borraron o reemplazaron, y se usa cuando `recall_context` pide revisarlas). Las instrucciones del servidor no cambian: la sección `## Conclusions` de `protocol.md` va después del primer `##`.
- Cada requisito de la [spec](spec.md) tiene su test, escrito antes que el código: los de C1, C2, C4 y C5 fallaron primero porque no compilaban; `purging_keeps_the_links_that_name_the_row_as_a_source` (C6) y `context_reports_conclusions_to_review` y `a_save_from_a_replaced_source_is_refused` (C4, C5) fallaron por la aserción.

## Binario real, home temporal

`clawg serve --profile dev --port 0` desde la build de prueba, con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`, sin tocar `~/.clawg*`:

1. #1 con `topic_key` `deploy/day`, #2, y la conclusión #3 con `derived_from = [1, 2]`. Un segundo save con la clave (#4) reemplaza a #1.
2. Un save con `derived_from = [1]` responde 400: `derived_from names memories that were replaced: 1 (by 4); draw the conclusion from the current ones`.
3. `POST /v1/context` responde `conclusions_to_review: 1`; `POST /v1/synthesis` lista #3 con `live = 1` y `lost = [#1 superseded by 4]`; el contexto siguiente responde 0.
4. Con el daemon parado, `clawg purge 2 --yes`; de vuelta arriba, `synthesize` por el puente MCP real (`clawg mcp --no-autostart`) muestra `#3 semantic_fact origin=conclusion from=#1,#2 live=0/2 lost=#1 replaced by #4,#2 purged …` bajo «Conclusions to review», con la guía.

## Pendiente

- La suite y clippy en Windows, en `target-test` (los corre CI en el PR).
- El perfil dev con el modelo real y Claude Code: un agente ve el aviso de `recall_context`, revisa con `synthesize` y corrige una conclusión.
- `rag-eval` no aplica: no cambia el ranking ni el prompt de `build_memory_prompt`; las líneas nuevas van fuera de él.
