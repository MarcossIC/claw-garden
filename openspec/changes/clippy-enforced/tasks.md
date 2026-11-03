# clippy-enforced — Tareas

Strict TDD donde hay conducta nueva que fijar (CE1, CE2): el test va primero y falla por la aserción, y el código que lo hace pasar va en el mismo commit. Los arreglos de lints no cambian conducta: los fijan los tests que ya hay, que pasan antes y después. Una tarea, un commit convencional, comprobado con los tests de los archivos que cambia; la suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencia a planes, fases, cambios ni números de tarea.

Requisitos en la [spec](spec.md); qué se arregla y qué se anota, en la [propuesta](proposal.md#decisiones).

## 0. Antes del código

- [x] El conteo del paso 0 en la [exploración](exploration.md#paso-0-el-conteo).
- [ ] La suite entera sobre la base (`c0bf3af`), en Linux, para comparar.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.

## 1. Arreglos, un commit por lint

Cada commit toca sólo los lugares de su lint, comprobados con `cargo clippy --workspace --all-targets -- -A clippy::all -W clippy::<lint> -D warnings` y los tests de los módulos que cambia.

- [ ] `chunks_exact_to_as_chunks` (8): `as_chunks::<N>()`, que también ignora el resto.
- [ ] `cloned_ref_to_slice_refs` (8): `std::slice::from_ref`.
- [ ] `unnecessary_map_or` (7): `is_some_and`, `is_none_or` o `==`.
- [ ] `needless_borrow` (5).
- [ ] `err_expect` (5): `expect_err`.
- [ ] `manual_contains` (5): `contains`.
- [ ] `manual_repeat_n` (4): `std::iter::repeat_n`.
- [ ] `useless_vec` (4): arreglos.
- [ ] `manual_range_contains` (3): `RangeInclusive::contains`.
- [ ] `type_complexity` (19): un alias con el nombre de lo que la tupla es.
- [ ] `empty_line_after_doc_comments` (2): la documentación del módulo pasa a `//!`.
- [ ] `field_reassign_with_default` (2): el campo en el inicializador.
- [ ] `unusual_byte_groupings` (2): las semillas en grupos de cuatro, con el mismo valor.
- [ ] `explicit_auto_deref` (2).
- [ ] `assertions_on_constants` (2): `const { assert!(…) }`.
- [ ] `double_ended_iterator_last` (2): `next_back`.
- [ ] `missing_transmute_annotations` (1): los tipos del `transmute` de `sqlite3_auto_extension`.
- [ ] `manual_checked_ops` (1): `embed_shares` reparte cuando hay un doc con textos, sin la división condicionada.
- [ ] `manual_div_ceil` (1): `div_ceil`.
- [ ] `manual_async_fn` (1): `ingest_worker_future` como `async fn`; `ingest_worker` sigue declarando `Send + 'static`.
- [ ] `for_unbounded_range` (1): el sondeo cuenta sus vueltas en un ciclo de `MINUTE_EVERY`.
- [ ] `bool_assert_comparison`, `useless_conversion`, `needless_lifetimes`, `useless_format`, `identity_op`, `single_range_in_vec_init`, `unnecessary_to_owned`, `unnecessary_operation` y `needless_range_loop` (1 cada uno), cada uno en su commit.

## 2. Anotaciones, un commit por lint (CE3)

- [ ] `too_many_arguments` (22): `#[expect(clippy::too_many_arguments, reason = "…")]`, que nombra el refactor que lo resuelve; los dos `#[allow(clippy::too_many_arguments)]` pasan a `#[expect]` con el mismo motivo.
- [ ] `large_enum_variant` (6), `result_large_err` (1) y `double_must_use` (1): `#[expect]` con lo que en ese lugar no pasa.

## 3. La tabla (CE1)

- [ ] Test primero: `every_package_takes_the_workspace_lints` (`crates/clawg-cli/tests/conventions.rs`), en rojo porque no hay tabla.
- [ ] `[workspace.lints.clippy]` en el `Cargo.toml` raíz; `[lints] workspace = true` en `crates/clawg-cli/Cargo.toml`, `crates/clawg-core/Cargo.toml` y `crates/clawg-eval/Cargo.toml`.
- Commit: `build: take clippy's lints from a workspace table`

## 4. El CI (CE2)

- [ ] Test primero: `no_workflow_turns_clippy_lints_off`, y los comandos nuevos en `the_ci_workflow_ignores_only_non_code_paths_and_runs_on_both_platforms` y `the_ci_workflow_checks_the_locked_production_build`, en rojo porque los workflows llevan `-A clippy::all`.
- [ ] `-A clippy::all` fuera de `.github/workflows/ci.yml` y `.github/workflows/cache.yml`, y el comentario de `ci.yml` que decía que no regían.
- Commit: `ci: enforce clippy's lints`

## 5. Docs (CE4)

- [ ] `.claude/CLAUDE.md` y `docs/reference/testing.md`: el comando sin `-A clippy::all`, dónde está la tabla y cómo se anota una excepción.
- [ ] R6 de `openspec/specs/quality/spec.md` y la sección CI de `openspec/specs/install/spec.md`, con las líneas de `ci.yml` que citan.
- Commits: `docs: give the clippy command without -A clippy::all` y `docs(specs): require clippy's lints in the quality and CI specs`

## 6. Cierre

- [ ] Suite y los dos clippy sobre el último commit, en Linux; Windows en el CI del pull request, y lo que marque ahí, arreglado con el mismo criterio.
- [ ] `verify.md`, `state.yaml`, el backlog y la fila 102 de `openspec/README.md`.
- Commit: `docs(openspec): record clippy's lints as enforced and verified`

## Al archivar (fuera de este cambio)

- La línea de origen de `specs/quality` y de `specs/install`, y la fila 102 de `openspec/README.md` con la versión.
