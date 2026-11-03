# clippy-enforced — Tareas

Strict TDD donde hay conducta nueva que fijar (CE1, CE2): el test va primero y falla por la aserción, y el código que lo hace pasar va en el mismo commit. Los arreglos de lints no cambian conducta: los fijan los tests que ya hay, que pasan antes y después. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia; la suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea.

Requisitos en la [spec](spec.md); qué se arregla y qué se anota, en la [propuesta](proposal.md#decisiones).

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| Motivo de `too_many_arguments` | nombrar el refactor que lo resuelve | dice qué falta sin nombrar el cambio: «the parameters have no type that groups them; adding one is a refactor of its own» en las 20 funciones de producción, y «a test fixture that takes each column of the row it seeds» en los 4 fixtures de test (`seed_hashed_row`, `row_in`, `insert_memory_row`, `seed_memory_aged`) | El código no nombra planes ni cambios (`.claude/CLAUDE.md`, «Rules for agents»), y un fixture que siembra columnas no es lo que agrupan los structs de parámetros |
| `double_ended_iterator_last` | `next_back` en los dos | `next_back` en `identity.rs` y `rfind` en `token_splitter.rs`, en el mismo commit | El clippy completo, después de los arreglos, marcó el `filter(..).next_back()` con `clippy::filter_next`: lo había traído el arreglo, no estaba en el conteo |
| Las anotaciones | un commit para `large_enum_variant`, `result_large_err` y `double_must_use` | un commit por lint, como los arreglos | La regla de un commit por lint vale igual para las anotaciones |
| Citas de `ci.yml` en `specs/install` | las de los comandos de clippy | todas las de la sección CI | Las demás ya estaban corridas por cambios anteriores; se verificaron línea por línea |

## 0. Antes del código

- [x] El conteo del paso 0 en la [exploración](exploration.md#paso-0-el-conteo).
- [x] La suite entera sobre la base (`c0bf3af`), en Linux, para comparar.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.

## 1. Arreglos, un commit por lint

Cada commit toca sólo los lugares de su lint, comprobados con `cargo clippy --workspace --all-targets -- -A clippy::all -W clippy::<lint> -D warnings` y los tests de los módulos que cambia.

- [x] `chunks_exact_to_as_chunks` (8): `as_chunks::<N>()`, que también ignora el resto.
- [x] `cloned_ref_to_slice_refs` (8): `std::slice::from_ref`.
- [x] `unnecessary_map_or` (7): `is_some_and`, `is_none_or` o `==`.
- [x] `needless_borrow` (5).
- [x] `err_expect` (5): `expect_err`.
- [x] `manual_contains` (5): `contains`.
- [x] `manual_repeat_n` (4): `std::iter::repeat_n`.
- [x] `useless_vec` (4): arreglos.
- [x] `manual_range_contains` (3): `RangeInclusive::contains`.
- [x] `type_complexity` (19): un alias con el nombre de lo que la tupla es.
- [x] `empty_line_after_doc_comments` (2): la documentación del módulo pasa a `//!`.
- [x] `field_reassign_with_default` (2): el campo en el inicializador.
- [x] `unusual_byte_groupings` (2): las semillas en grupos de cuatro, con el mismo valor.
- [x] `explicit_auto_deref` (2).
- [x] `assertions_on_constants` (2): `const { assert!(…) }`.
- [x] `double_ended_iterator_last` (2): `next_back`, o `rfind` donde había un `filter`.
- [x] `missing_transmute_annotations` (1): los tipos del `transmute` de `sqlite3_auto_extension`.
- [x] `manual_checked_ops` (1): `embed_shares` reparte cuando hay un doc con textos, sin la división condicionada.
- [x] `manual_div_ceil` (1): `div_ceil`.
- [x] `manual_async_fn` (1): `ingest_worker_future` como `async fn`; `ingest_worker` sigue declarando `Send + 'static`.
- [x] `for_unbounded_range` (1): el sondeo cuenta sus vueltas en un ciclo de `MINUTE_EVERY`.
- [x] `bool_assert_comparison`, `useless_conversion`, `needless_lifetimes`, `useless_format`, `identity_op`, `single_range_in_vec_init`, `unnecessary_to_owned`, `unnecessary_operation` y `needless_range_loop` (1 cada uno), cada uno en su commit.

## 2. Anotaciones, un commit por lint (CE3)

- [x] `too_many_arguments` (22): `#[expect(clippy::too_many_arguments, reason = "…")]`, con el motivo de «Frente a lo previsto»; los dos `#[allow(clippy::too_many_arguments)]` pasan a `#[expect]`.
- [x] `large_enum_variant` (6), `result_large_err` (1) y `double_must_use` (1), cada uno en su commit: `#[expect]` con lo que en ese lugar no pasa.

## 3. La tabla (CE1)

- [x] Test primero: `every_package_takes_the_workspace_lints` (`crates/clawg-cli/tests/conventions.rs`), en rojo porque no hay tabla.
- [x] `[workspace.lints.clippy]` en el `Cargo.toml` raíz; `[lints] workspace = true` en `crates/clawg-cli/Cargo.toml`, `crates/clawg-core/Cargo.toml` y `crates/clawg-eval/Cargo.toml`.
- Commit: `build: take clippy's lints from a workspace table`

## 4. El CI (CE2)

- [x] Test primero: `no_workflow_turns_clippy_lints_off`, y los comandos nuevos en `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` y `the_ci_workflow_checks_the_locked_production_build`, en rojo porque los workflows llevan `-A clippy::all`.
- [x] `-A clippy::all` fuera de `.github/workflows/ci.yml` y `.github/workflows/cache.yml`, y el comentario de `ci.yml` que decía que no regían.
- Commit: `ci: enforce clippy's lints`

## 5. Docs (CE4)

- [x] `.claude/CLAUDE.md` y `docs/reference/testing.md`: el comando sin `-A clippy::all`, dónde está la tabla y cómo se anota una excepción.
- [x] R6 de `openspec/specs/quality/spec.md` y la sección CI de `openspec/specs/install/spec.md`, con las líneas de `ci.yml` que citan.
- Commits: `docs: give the clippy command without -A clippy::all` y `docs(specs): require clippy's lints in the quality and CI specs`

## 6. Cierre

- [x] Suite y los dos clippy sobre el último commit, en Linux; Windows en el CI del pull request, y lo que marque ahí, arreglado con el mismo criterio.
- [x] `verify.md`, `state.yaml`, el backlog y la fila 102 de `openspec/README.md`.
- Commit: `docs(openspec): record clippy's lints as enforced and verified`

## Al archivar (fuera de este cambio)

- La línea de origen de `specs/quality` y de `specs/install`, y la fila 102 de `openspec/README.md` con la versión.
