# memory-rank-fusion — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila, pasa `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`.

Los commits van en el orden 1, 3, 4, 5, 2, 6: el corte entra después de las herramientas de medición, así el commit anterior a él es la base de la medición con el modelo real.

Con `-D warnings` un campo sin lector es un error, así que `RankedHit.exact` entra con su primer lector: `Serialize` lo lee, de modo que la tarea 1 compila sola. `Hit.exact` del cliente entra con el corte que lo usa (tarea 2).

## 1. La marca de coincidencia exacta en `RankedHit` (R1)

- [x] Test primero (`search.rs`): `a_row_holding_the_query_identifier_comes_back_exact` (**RED**, no compila).
- [x] `RankedHit.exact` desde `RawHit.lexical_match` en `search.rs:525` y `:1060`; `exact: false` en los literales de tests (`access.rs`, `admin.rs`, `prompt_builder.rs`, `search.rs`, `tests/public_api.rs`).
- Commit: `feat(core): mark the search hits that hold every identifier of the query`

## 2. El corte relativo no cuenta los hits exactos (R2, R3)

- [x] Tests primero. `relevance.rs`: los de R2. `tools.rs`: `relevant_hits_keeps_exact_hits_and_cuts_against_the_rest` y `an_exact_hit_from_another_agent_is_shown_by_its_author`. `client.rs`: `a_hit_without_exact_reads_as_inexact`. `tests/api.rs`: `search_shows_the_rows_an_identifier_match_used_to_hide` y `an_identifier_match_from_another_agent_is_exact_and_attributed`, con un embedder de prueba que devuelve vectores fijos por texto. Con la regla de hoy el primero recibe 2 hits en lugar de 4 (comprobado antes del commit).
- [x] `relevant_len` pasa a `shown_hits(&[(f64, bool)], ratio) -> Vec<bool>`; el daemon (`api.rs:911-914`) y el puente (`tools.rs:60-64`) lo usan; `Hit.exact` con `#[serde(default)]`.
- [x] El comentario de `MIN_SHOWN_HITS` y el de `shown_hits` dicen que el mínimo y la referencia son de los hits no exactos.
- Commit: `fix(cli): cut search results against the best hit without an identifier match`

## 3. `all_hit_rate` en agent-view (R4)

- [x] Tests primero (`stats.rs`, `mod.rs`).
- [x] `Sample.all_found`, `ToolStats.all_hit_rate` con `#[serde(default)]`, la consola por subconjunto.
- Commit: `feat(eval): report how often agent-view showed every expected row`

## 4. `query_lang` en la base de `golden-mine synth` (R5)

- [x] Test primero (`synth/golden.rs`): `a_base_case_keeps_its_query_lang` (**RED**, la clave se rechaza hoy).
- [x] `Case.query_lang: Option<String>` con `default` y `skip_serializing_if`; `None` en los casos que genera hoy.
- Commit: `fix(eval): let golden-mine synth read a base whose cases name their query language`

## 5. El subconjunto `lexical_related` (R5)

- [x] Tests primero (`synth/mod.rs`, `synth/templates.rs`, `rag-eval/golden.rs`).
- [x] `Subset::LexicalRelated` en `synth` y `CaseSubset::LexicalRelated` en `rag-eval`; plantillas por idioma y tipo de identificador; `lexical_related` al final de `generate`.
- Commit: `feat(eval): synthesize cases that expect an identifier row and its related rows`

## 6. Documentación

- [x] `.claude/CLAUDE.md`: `exact` en cada hit de `POST /v1/search`, y que el corte relativo no cuenta los hits exactos.
- [x] `golden-mine` y la ayuda de `rag-eval` no nombran los subconjuntos: no cambian.
- Commit: `docs: describe the exact flag of search hits`

## 7. Verificación

- [x] Suite y clippy en Linux en cada commit.
- [x] El CI de Linux y Windows.
- [ ] La medición con el modelo real de la [propuesta](proposal.md#medición-con-el-modelo-real): U1 a U6, en `verify.md`. Necesita el modelo y el golden set fuera de git.
