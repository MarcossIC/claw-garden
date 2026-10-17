# context-budget — Verificación

**Veredicto:** cumple. La suite y clippy pasan, el CI está en verde en Linux y Windows, y con el modelo real el default de 8.000 bytes no corta ningún prompt: `recall_context` sigue en 39/46 con los mismos tokens, y ningún caso se mueve.

## Linux

Sobre el último commit de la rama, build debug:

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace`: pasa entero (1 ignorado, el de los vectores con el modelo real, `e5_golden`).
- `CLAWG_BLESS=1` del contrato del agente: cambian sólo la descripción de `recall_context` (nombra el presupuesto y `read_memory`) y su esquema (el argumento `budget`).

Cada requisito tiene sus tests en la [spec](spec.md); todos fallaban antes de su commit, por la aserción o por no compilar.

## CI

`test (ubuntu-latest)` y `test (windows-latest)` (clippy y la suite) pasan sobre la punta de la rama, `54539df`, y sobre su merge en `main`, `f3c44ef`.

## Medición con el modelo real

- **Qué necesita:** el modelo en el caché del perfil `dev` (`rag-eval --real --check-only` sale con 0), el golden set fuera de git (`docs/eval/golden.json`) y un build debug de `rag-eval` en `target-test`, después de `cargo clean --workspace`. `rag-eval` siembra un store temporal y no abre ningún store real.
- **Commits:** el después es `f3c44ef`, el merge de este cambio en `main`. El antes es `2026-10-17-real-rank-fusion-tip-agent-view.json`, medido sobre `31420de`. Entre ese commit y la base de este cambio (`af6fef4`) en `crates/` sólo cambian el arena único de malloc de `clawg serve` (`keep_one_malloc_arena`, que fuera de Linux con glibc no hace nada), el test de `e5_golden` y la versión, nada que `rag-eval` ejecute en Windows; por eso la base no se compiló aparte.
- **Comandos** (en Windows): `rag-eval --real --agent-view --tag context-budget-tip`, que escribe `docs/eval/results/2026-10-17-real-context-budget-tip-agent-view.json` (fuera de git), y `rag-eval --diff` contra el archivo de antes. El `git_sha` y el `golden_set.content_sha256` de cada archivo confirman su commit y su golden: `363d3e08854bfa222ccf9944508938e0e58a8dab9808696eca7a10525da0a1fb`, el de english-recall, en los dos.

| Métrica | Antes (`31420de`) | Después (`f3c44ef`) |
|---|---|---|
| `recall_context`, `found` | 39/46 | 39/46 |
| `recall_context`, tokens media / máximo | 1.086,4 / 1.408 | 1.086,4 / 1.408 |
| `search_memory`, `found` | 36/46 | 36/46 |
| `search_memory`, tokens media / máximo | 764,0 / 1.206 | 764,0 / 1.206 |
| Prompts con `=== BUDGET ===` | 0 | 0 |

`--diff` no mueve ningún caso en ninguna métrica: `retrieval`, `similarity`, agent-view y sus pares de idioma (`en>en`, `es>en`, `es>es`), todos con p = 1,0000. Con cero casos movidos `rag-eval` dice "not distinguishable"; lo que muestra la corrida es más fuerte: `found` y tokens son los mismos caso por caso. Es lo que esperaba la [propuesta](proposal.md#cómo-se-mide-el-éxito): el contexto más grande medido (1.408 tokens, unos 5.600 bytes) entra en el default, y el prompt sólo cambia cuando corta.

## Sin correr

- La corrida con un `budget` bajo forzado para calibrar el default: `rag-eval` no tiene la opción ([fuera de alcance](proposal.md#fuera-de-alcance)), y con el default alto no cambia el veredicto. El recorte sobre filas reales del store lo prueba `a_store_of_long_rows_fits_the_budget`.
