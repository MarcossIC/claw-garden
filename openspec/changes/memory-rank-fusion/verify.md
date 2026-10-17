# memory-rank-fusion — Verificación

## Linux

En cada commit de las tareas 1 a 6, `cargo test --workspace` pasa y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings` sale con 0. `agent_contract.json` no cambia: el texto de `search_memory` es el mismo.

Lo que fallaba antes del cambio y pasa después:

- `search_shows_the_rows_an_identifier_match_used_to_hide` (`crates/clawg-cli/tests/api.rs`): con el corte de hoy aplicado a los mismos hits, el daemon devuelve la fila del identificador (score 1,18) y una relacionada (0,39), y corta las otras dos (0,35 × 1,18 = 0,41 contra 0,36 y 0,33). Con el corte nuevo vuelven las cuatro.
- En puro, `exact_hits_do_not_set_the_bar_for_the_others`: el exacto en 1,6 y 0,30 / 0,25 / 0,20 / 0,05 dejan 4 hits, contra 2.

`golden-mine synth --seed 7` sobre una base con `query_lang` (que antes rechazaba) agrega los 30 casos `lexical_related`; sin ellos, el set es byte a byte el de antes (sha256 fijado en `the_other_subsets_do_not_change_when_lexical_related_is_added`).

## CI

El CI (`test (ubuntu-latest)` y `test (windows-latest)`, clippy y la suite) pasa sobre la punta de la rama, `bb8f60d`.

## Pendiente

- La medición con el modelo real (U1 a U6 de la [propuesta](proposal.md#medición-con-el-modelo-real)). Necesita el modelo en el caché del perfil y el golden set fuera de git. Primero se regenera `docs/eval/golden-expanded.json` con `golden-mine synth --seed 7` sobre el `golden.json` actual. La base corre en el commit anterior al del corte (`feat(eval): synthesize cases that expect an identifier row and its related rows`), que ya tiene `lexical_related` y `all_hit_rate` con el corte de antes; el después, en la punta de la rama.
