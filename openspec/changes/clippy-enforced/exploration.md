# clippy-enforced — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S–M, según el conteo | — | exploración (borrador) |

**Tesis:** el paso de clippy del CI no aplica ningún lint de clippy. `-A clippy::all` los apaga todos y sólo quedan los warnings del compilador. Se ve verde, pero no revisa lo que su nombre sugiere. Hoy no se sabe cuántos hallazgos hay.

## Problema (verificado contra el código)

- `.github/workflows/ci.yml:65` y `:71`, y `.github/workflows/cache.yml:53` y `:55`, corren con `-A clippy::all -D warnings`. El comentario de `ci.yml:62` lo admite: «clippy's own lints are not enforced yet».
- Ningún `Cargo.toml` declara `[workspace.lints]`.
- `openspec/specs/quality/spec.md:74-80` exige ese comando «hasta que se arreglen los que hay», y `openspec/specs/install/spec.md:137-139` lo repite.
- `crates/clawg-cli/tests/conventions.rs:584` y `:780` fijan el texto del comando en el CI. También lo dan `.claude/CLAUDE.md:66` y `docs/reference/testing.md:36`.
- El backlog de `openspec/README.md` registró 28 lints propios en su momento (7 de `too_many_arguments`). Ese número no se volvió a medir, y el código creció desde entonces.

## Paso 0: el conteo

`cargo clippy --locked --workspace --all-targets`, sin `-A clippy::all`, sobre `c0bf3af`, con la toolchain fijada (1.99.1), en Linux. Se cuenta cada lugar (archivo, línea y columna) una vez: el código que compilan dos targets, la biblioteca y sus tests, da el mismo hallazgo dos veces, y así los 188 mensajes son 124 lugares.

- **124 hallazgos en 34 lints**, todos `warn`: ninguno de `clippy::correctness`, que es `deny`. Por paquete: `clawg-core` 81, `clawg-eval` 23 y `clawg-cli` 20 (2 de ellos en `tests/`).
- `cargo clippy --locked -p clawg-cli`, lo que construye el release, da 48, todos entre los 124.
- Los dos `#[allow(clippy::too_many_arguments)]` que ya había (`crates/clawg-core/src/memory/bootstrap.rs:849`, `crates/clawg-core/src/memory/prompt_builder.rs:2489`) no cuentan: el lint no se emite ahí.
- El código `cfg(windows)` no se compila en Linux: lo cuenta el CI del pull request, en `windows-latest`.

| Lint | Hallazgos |
|---|---|
| `too_many_arguments` | 22 |
| `type_complexity` | 19 |
| `chunks_exact_to_as_chunks` | 8 |
| `cloned_ref_to_slice_refs` | 8 |
| `unnecessary_map_or` | 7 |
| `large_enum_variant` | 6 |
| `needless_borrow` | 5 |
| `err_expect` | 5 |
| `manual_contains` | 5 |
| `manual_repeat_n` | 4 |
| `useless_vec` | 4 |
| `manual_range_contains` | 3 |
| `empty_line_after_doc_comments`, `field_reassign_with_default`, `unusual_byte_groupings`, `explicit_auto_deref`, `assertions_on_constants`, `double_ended_iterator_last` | 2 cada uno |
| `missing_transmute_annotations`, `double_must_use`, `manual_checked_ops`, `manual_div_ceil`, `manual_async_fn`, `for_unbounded_range`, `result_large_err`, `bool_assert_comparison`, `useless_conversion`, `needless_lifetimes`, `useless_format`, `identity_op`, `single_range_in_vec_init`, `unnecessary_to_owned`, `unnecessary_operation`, `needless_range_loop` | 1 cada uno |

Fuera del grupo por defecto, para la primera decisión abierta:

- `await_holding_lock` y `await_holding_refcell_ref` ya están en él (`clippy::suspicious`) y no dan nada.
- `large_futures` (`clippy::pedantic`) no da nada con su umbral, 16 KiB. Bajado a 2 KiB con un `clippy.toml` de prueba, la future más grande mide 9.504 bytes en `clawg-cli` (la de `main`, `crates/clawg-cli/src/main.rs:142`), 8.952 en un test de `clawg-core` y 11.560 en un test de `rag-eval`.
- `allow_attributes` y `allow_attributes_without_reason` (`clippy::restriction`) dan los dos `allow` de arriba y nada más.

Ninguno de los 124 pide cambiar conducta: cada uno tiene un arreglo que deja el resultado igual, o un motivo para un `#[expect]` (los 22 `too_many_arguments`, que resuelve domain-types, y los lugares donde lo que el lint cuida no pasa). Con menos de 150, sigue el enfoque A; la [propuesta](proposal.md) dice qué se arregla y qué se anota.

## Lo que ya existe y se reutiliza

- El CI fija la versión de la toolchain (`ci.yml:55`), así que un lint nuevo no rompe una rama sin un cambio explícito de versión.
- `tests/conventions.rs` ya es el lugar donde se fijan reglas del repositorio.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. `[workspace.lints.clippy]` con el grupo por defecto y excepciones con motivo** | Una sola fuente para CI y para local. El comando pierde el `-A`. Se ve en un archivo qué se tolera y por qué | Hay que arreglar o justificar todo lo que aparezca al activarlo |
| B. Sacar `-A clippy::all` de cada comando, sin tabla de lints | El menor cambio | Las excepciones quedan repartidas por el código. CI y local se pueden separar |
| C. Una línea de base que sólo falla con hallazgos nuevos | Se activa enseguida, cualquiera sea el conteo | Herramienta y archivo nuevos para algo que quizás se arregla en un día |

**Recomendado: A**, salvo que el paso 0 dé un número que no quepa en un cambio chico; entonces se discute C.

Una excepción puntual usa `#[expect(clippy::..., reason = "...")]` y no `#[allow]`: el compilador avisa cuando deja de hacer falta, así que al sacar parámetros en domain-types cada excepción se cae sola.

## Alcance candidato

1. **Paso 0, sin código:** correr `cargo clippy --workspace --all-targets --target-dir target-test` en Linux y Windows, y anotar el conteo por lint.
2. Arreglar lo mecánico en commits por lint, y justificar o arreglar el resto.
3. Declarar `[workspace.lints]` en el `Cargo.toml` raíz y agregar `[lints]` con `workspace = true` a los tres paquetes (`clawg-cli`, `clawg-core` y `clawg-eval`): Cargo no la hereda solo, y un paquete sin esa tabla queda fuera de la política. Sacar `-A clippy::all` de los comandos del CI, de los documentos y de los archivos de instrucciones, y actualizar `tests/conventions.rs` y las dos specs.

### No-alcance

- `clippy::pedantic` o `clippy::nursery` como grupos.
- `cargo fmt` en el CI: sigue en el backlog, porque reformatear choca con las ramas abiertas.
- Refactors que un lint sugiera y que tengan su propio cambio: `too_many_arguments` se resuelve en domain-types y acá se anota con `#[expect]`.

## Decisiones abiertas

1. Si se suman lints sueltos fuera del grupo por defecto que importan a un daemon asíncrono, como `await_holding_lock` o `large_futures`. Se decide con el conteo a la vista.
2. Si los binarios de `clawg-eval` tienen el mismo nivel de exigencia que la producción.

## Cómo se mide

- El comando sin `-A clippy::all` termina con código 0 en Linux y Windows.
- El test de `conventions.rs` pasa a fijar el comando nuevo.
- La suite no cambia de resultado: arreglar lints no debe cambiar conducta.

## Interacciones

- **domain-types y cli-dispatch:** bajan varios lints. Conviene activar el control antes, así esos cambios no los reintroducen.
- **Todo cambio abierto:** un lint nuevo puede pedir tocar un archivo que otro cambio está editando, por eso los arreglos van en commits chicos.
