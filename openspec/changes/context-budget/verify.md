# context-budget — Verificación

## Linux

Sobre el último commit de la rama, build debug:

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace`: pasa entero (1 ignorado, el de los vectores con el modelo real, `e5_golden`).
- `CLAWG_BLESS=1` del contrato del agente: cambian sólo la descripción de `recall_context` (nombra el presupuesto y `read_memory`) y su esquema (el argumento `budget`).

Cada requisito tiene sus tests en la [spec](spec.md); todos fallaban antes de su commit, por la aserción o por no compilar.

## Pendiente

- [ ] El CI en verde en Linux y Windows.
- [ ] `rag-eval --real --agent-view` de `main` a la rama, con el default de 8.000 bytes. Necesita el modelo real y `docs/eval/golden.json` (fuera de git). Se espera `found` de `recall_context` en 39/46 y la media de tokens sin aumento: con el máximo medido (1.408 tokens) el default no corta, y el prompt sólo cambia cuando corta. Si algún caso pasa los 8.000 bytes, su prompt termina en `=== BUDGET ===`, que el parser no cuenta como texto de un hit.
- [ ] La misma corrida con un `budget` bajo forzado, para calibrar el default: `rag-eval` no tiene la opción; con el default alto no hace falta para archivar.
