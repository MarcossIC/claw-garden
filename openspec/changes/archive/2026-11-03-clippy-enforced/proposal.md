# clippy-enforced — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); verificado otra vez contra `develop` (`c0bf3af`):

- El CI corre clippy con `-A clippy::all -D warnings` (`.github/workflows/ci.yml:65` y `:71`, `.github/workflows/cache.yml:53` y `:55`). `-A clippy::all` apaga todos los lints de clippy y sólo quedan los warnings del compilador; el comentario de `ci.yml:62` lo dice: «clippy's own lints are not enforced yet».
- Ningún `Cargo.toml` declara `[workspace.lints]`, así que nada dice qué lints rigen el workspace.
- El mismo comando está en `.claude/CLAUDE.md:66`, `docs/reference/testing.md:21` y `:36`, R6 de [`specs/quality`](../../../specs/quality/spec.md) y la sección CI de [`specs/install`](../../../specs/install/spec.md); `crates/clawg-cli/tests/conventions.rs:584` y `:780` fijan su texto en `ci.yml`.
- Sin `-A clippy::all`, `cargo clippy --locked --workspace --all-targets` da 124 hallazgos en 34 lints en Linux ([paso 0](exploration.md#paso-0-el-conteo)).

## Objetivo que sirve

Transversal. No cambia conducta: hace que el paso de clippy del CI revise lo que su nombre dice, antes de que domain-types y cli-dispatch toquen las firmas que más lints dan.

## Decisiones

1. **Enfoque A de la exploración.** La política vive en `[workspace.lints.clippy]` del `Cargo.toml` raíz, y `clawg-cli`, `clawg-core` y `clawg-eval` la toman con `[lints] workspace = true`: Cargo no la hereda sola, y un paquete sin esa tabla queda fuera. El comando pierde `-A clippy::all` y conserva `-D warnings`, que hace fallar tanto un warning del compilador como uno de clippy; la tabla dice qué lints corren, el comando que ninguno pasa. B (sacar el `-A` sin tabla) deja la política repartida entre comandos; C (una línea de base) sobra con un conteo que entra en un cambio.
2. **Primera decisión abierta: lints sueltos fuera del grupo por defecto.** Con el conteo a la vista, se suman tres, y ninguno da un hallazgo después de este cambio:
   - `large_futures`: el daemon y el puente corren en tokio, y `runtime.block_on(dispatch(cli))` (`crates/clawg-cli/src/main.rs:82`) deja la future raíz en la pila del hilo principal, que en Windows es de 1 MiB. Una future se copia entera cada vez que se la mueve (a `block_on`, a `spawn`, dentro de otra), y en un build debug esas copias son lo que desborda la pila. El umbral por defecto es 16 KiB; hoy la más grande del binario mide 9.504 bytes (`crates/clawg-cli/src/main.rs:142`): el lint no pide nada ahora y avisa cuando una lo pase.
   - `allow_attributes` y `allow_attributes_without_reason`: hacen que clippy compruebe la regla de las excepciones (decisión 4). Los únicos dos `#[allow]` de lints que hay pasan a `#[expect]` con motivo.
   - `await_holding_lock` y `await_holding_refcell_ref`, que la exploración nombraba, ya están en el grupo por defecto (`clippy::suspicious`) y dan 0: entran con él.
   - No se suman `unused_async` (`clippy::pedantic`, 3 hallazgos: una función `async` sin `await` no es un error), ni los grupos `pedantic` y `nursery`.
3. **Segunda decisión abierta: `clawg-eval` tiene el mismo nivel que la producción.** Da 23 de los 124, de los mismos lints que el resto, y dos son excepciones con motivo. `rag-eval` decide los cambios de ranking: un error que un lint ve ahí desvía una decisión. Y un paquete con `[lints] workspace = true` no puede sumar ni quitar lints propios (Cargo lo rechaza), así que otro nivel pediría una segunda tabla.
4. **Arreglar o anotar.** Se arregla todo hallazgo cuyo arreglo deja el resultado igual. Se anota con `#[expect(clippy::…, reason = "…")]` en el lugar, nunca con `#[allow]`, sólo lo que:
   - pide un refactor que tiene su propio cambio: los 22 `too_many_arguments` (y los dos que ya tenían `allow`), que resuelve domain-types; al sacar parámetros, el `#[expect]` que ya no hace falta es un warning, y el CI lo pide quitar;
   - señala algo que en ese lugar no pasa: los 6 `large_enum_variant` (un valor por vez que se arma y se consume enseguida; ninguna colección guarda muchos, y un `Box` sumaría una asignación sin achicar nada guardado), el `result_large_err` de `call` (`crates/clawg-cli/src/mcp/server.rs:624`, cuyo `Err` es la respuesta de la herramienta, que rmcp recibe por valor) y el `double_must_use` que genera `async_trait` sobre `Embedder` (`crates/clawg-core/src/memory/embedder.rs:63`).

   Quedan 94 arreglos y 30 anotaciones. Ninguna excepción va a la tabla: una excepción del workspace taparía también los casos nuevos donde el lint sí acierta.
5. **Un commit por lint**, como pide la exploración: cada uno toca sólo los lugares de su lint y se comprueba con los tests de lo que cambia. La tabla y los comandos van después, cuando el workspace ya pasa.

## Alcance

1. Los 124 hallazgos de Linux, arreglados o anotados según la decisión 4, y los dos `#[allow(clippy::too_many_arguments)]`, pasados a `#[expect]`.
2. `[workspace.lints.clippy]` en el `Cargo.toml` raíz con `all` y los tres lints de la decisión 2, y `[lints] workspace = true` en los tres paquetes, con un test que lo fija.
3. `-A clippy::all` fuera de `ci.yml` y `cache.yml`, y los tests de `tests/conventions.rs` que fijan esos comandos, con un test que falla si un workflow apaga lints de clippy.
4. El comando nuevo en `.claude/CLAUDE.md`, `docs/reference/testing.md`, R6 de `specs/quality` y la sección CI de `specs/install`; el backlog y la fila 102 de `openspec/README.md`.
5. Lo que el CI del pull request marque en `windows-latest` (el código `cfg(windows)` no se compila en Linux), con el mismo criterio.

## Fuera de alcance

- Los grupos `clippy::pedantic` y `clippy::nursery`, y `cargo fmt`, que sigue en el backlog.
- Los refactors que tienen su propio cambio: los parámetros de más (domain-types), el despacho de la CLI (cli-dispatch).
- Los `verify.md` de cambios archivados: registran lo que se corrió entonces.
- Subir la versión y archivar.

## Cómo se mide el éxito

- `cargo clippy --locked --workspace --all-targets -- -D warnings` y `cargo clippy --locked -p clawg-cli -- -D warnings` terminan con código 0 en Linux y en Windows (el CI del pull request).
- Tests de `crates/clawg-cli/tests/conventions.rs` que fallan hoy: los paquetes toman la tabla del workspace, y ningún workflow apaga lints de clippy (ver [spec](spec.md)).
- La suite no cambia de resultado: pasa entera antes y después.

## Datos y vuelta atrás

No toca `memory.db`, `metrics.db`, ningún formato ni lo que ve el modelo (`agent_contract.json` no cambia). Volver atrás es revertir los commits; un arreglo revertido sólo vuelve a dar su hallazgo.
