# search-abstention — Verificación

**Veredicto:** pendiente. La suite y clippy pasan en Linux; faltan el CI en Linux y Windows y la medición con el modelo real (U1 a U4 de la [propuesta](proposal.md#medición-con-el-modelo-real)).

## Linux

Sobre `973d84b`, el último commit de código de la rama (los que siguen sólo tocan documentación), build debug en `target`:

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace --no-fail-fast`: pasa entero.
- `agent_contract.json` no cambia: `the_agent_contract_matches_its_file` pasa sin `CLAWG_BLESS`.
- Un puente anterior, cuya variante `searched` no tiene campos, lee la respuesta nueva `{"state":"searched","hits":0,"withheld":4}` como `searched`: comprobado con un test descartable, fuera de la rama.

Cada requisito tiene sus tests en la [spec](spec.md); todos fallaban antes de su commit, por la aserción o por no compilar. Siete tests existentes contaban con filas de score 0 para tener hits bajo el embedder falso, cuyos vectores sólo coinciden con el mismo texto; sus fixtures se corrigieron sin cambiar lo que miden ([tareas](tasks.md), 1 y 2).

## CI

Pendiente.

## Medición con el modelo real

Pendiente. Necesita el modelo en el caché del perfil `dev` (`rag-eval --real --check-only` sale con 0) y el golden set fuera de git (`docs/eval/golden.json`); `rag-eval` siembra un store temporal y no abre ningún store real. Se corre `rag-eval --real` y `rag-eval --real --agent-view` en el commit anterior al cambio (`9a0ec3f`) y en la punta, y `rag-eval --diff` entre cada par.
