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
3. Declarar `[workspace.lints]`, sacar `-A clippy::all` de los comandos del CI, de los documentos y de los archivos de instrucciones, y actualizar `tests/conventions.rs` y las dos specs.

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
