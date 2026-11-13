# memory-module-layout — Verificación

**Resultado:** aplicado y verificado en Linux, en el CI de Linux y Windows, y en Windows en local ([«En Windows, en local»](#en-windows-en-local)). La suite entera y los dos clippy pasan sobre el último commit de código. Los dos tests que nacieron en rojo pasan. Ningún test de una respuesta ni de los contratos del agente se editó. Queda lo que pide el modelo real, en [«Pendiente»](#pendiente).

## Qué se corrió

Sobre el último commit de código, en Linux, con el directorio de build limpio al empezar, y otra vez, también con el directorio limpio, sobre el merge de `develop` que trajo el archivo de store-boundary y la versión 0.84.0:

| Comando | Resultado |
|---|---|
| `cargo test --workspace` | pasa, sin ningún fallo |
| `cargo clippy --workspace --all-targets -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli -- -D warnings` | sin hallazgos; es el núcleo sin `testing` ni `eval`, como lo construye el release |

Cada commit de un corte y de una carpeta se revisó por separado con `cargo check --workspace --all-targets`, sin ningún warning, y con los tests de los módulos que tocaba.

## Los tests en rojo

| Test | Antes (sobre `develop`) | Después |
|---|---|---|
| `a_memory_folder_names_only_the_folders_its_row_allows` | falló: 50 módulos de `memory` fuera de una carpeta de la tabla | pasa |
| `the_memory_modules_cycle_only_where_listed` | falló: un grupo de 34 módulos, de 52 | pasa: cinco grupos, de 54 módulos |
| `public_memory_modules_reads_declarations_and_reexports` | no compilaba: `public_memory_modules` no existía | pasa |

Los dos primeros quedaron en rojo desde su commit hasta el de las rutas, como dicen las [tareas](tasks.md#cómo-se-trabaja-cada-tarea). Los demás tests de `conventions.rs` pasaron en cada commit.

Para comprobar que la regla ve lo que dice, se agregó a mano `use crate::memory::retrieval::search::RankedHit;` en `store/tier.rs`: falló con `store/tier.rs:11 names retrieval/search`, y la cuenta de ciclos mostró un grupo de 13 módulos entre `store`, `embedding` y `retrieval`. La línea se quitó.

## La cuenta del ciclo

Se cuenta como dice [D7](design.md#d7-la-regla-y-cómo-se-cuenta-el-ciclo), y se repite con:

```bash
cargo test -p clawg-cli --test conventions the_memory_modules_cycle_only_where_listed
```

Pasa cuando los grupos que encuentra son los de `MEMORY_CYCLES`. Para ver la cuenta impresa, alcanza con quitar un grupo de esa lista: el fallo imprime cada grupo con su tamaño.

| Momento | Módulos | Grupos en círculo |
|---|---:|---|
| `develop` | 52 | uno de 34 |
| después de los cortes, con el árbol todavía plano | 54 | cinco: de 7, 5, 4, 2 y 2 |
| al terminar | 54 | los mismos cinco, cada uno dentro de una carpeta |

Los 54 son los 52 más `listing` y `search_log`. Entre carpetas no queda ningún ciclo: la tabla no puede tenerlo, y la regla comprueba que el código no se sale de ella.

Los cinco grupos y su motivo están en `MEMORY_CYCLES` y en la tabla de D7:

- `store`: `backup`, `db`, `dedup`, `error`, `latency`, `store_lock`, `tier`.
- `retrieval`: `listing`, `pieces`, `search`, `search_log`.
- `retrieval`: `prompt_builder`, `workspace`.
- `projects`: `inspect`, `merge`, `project_delete`, `relocate`, `subfolder`.
- `projects`: `project`, `registry`.

## Las comprobaciones de D10

1. **Los movimientos.** `git show --stat -M100%` de cada uno de los ocho commits de carpeta muestra sus fuentes como renombres al 100 % (7, 9, 6, 11, 4, 8, 4 y 5 archivos), más el `mod.rs` nuevo de la carpeta y `memory/mod.rs`. El de `store` cambia además dos líneas de `conventions.rs` (`TRANSLATES` y su comentario).
2. **Las aserciones.** En el diff de `develop...HEAD` de los tres crates, sin `conventions.rs`, las líneas con `assert`, `expect(` o `panic!` que se quitan son las mismas que se agregan, una vez igualada la ruta `crate::memory::<carpeta>::`. Hay una sola de más: el `.expect` del helper `seed_memory_scoped`, que quedó copiado en los tests de `admin` y de `listing`.
3. **Lo que ve un cliente.** `git diff develop...HEAD` no muestra nada en `crates/clawg-cli/src`, `crates/clawg-eval/src`, `crates/clawg-core/tests`, `crates/clawg-cli/tests/api.rs` ni `crates/clawg-cli/tests/binary.rs`. Ahí están los dos contratos del agente, que pasan sin `CLAWG_BLESS`.
4. **`project_docs`.** `git diff develop...HEAD -- crates/clawg-core/src/memory/project_docs` no muestra nada.
5. **La cantidad de tests** de `clawg-core --lib` es la misma antes y después.

## Las citas

- **Specs y docs:** 370 rutas pasaron a su carpeta y 143 citas `ruta:línea` cambiaron de línea o de archivo, en 26 archivos de `openspec/specs` y `docs/reference`. Las que apuntaban a código que salió de `admin.rs`, `store_insert.rs`, `db.rs`, `secrets.rs` o `pieces.rs` nombran ahora el archivo donde quedó.
- **El merge de `develop`** cambió cuatro specs (`portability`, `project`, `sharing` y `storage`) al archivar store-boundary. En esas cuatro se tomó el texto de `develop` y se rehízo el barrido: 124 rutas y 31 líneas.
- **Una cita no se pudo llevar:** `openspec/specs/storage/spec.md` cita `store_insert.rs:2165`, y en `develop` ese archivo tiene 1.702 líneas. Quedó con su número y la ruta nueva.
- **El golden de docs:** las 13 rutas esperadas de `memory`.
- **Para comprobar que ninguna ruta citada falta:**

  ```bash
  rg -o -I 'crates/clawg-core/src/memory/[A-Za-z0-9_/]+(\.rs)?' openspec/specs docs/reference crates/clawg-eval/golden | sort -u | while read p; do [ -e "$p" ] || echo "$p"; done
  ```

  No imprime nada.

## Lo que se resolvió distinto de lo escrito

- **Las carpetas son `pub(crate) mod`, no privadas.** `crates/clawg-core/src/runtime.rs` y `eval_api.rs` están fuera de `memory` y nombran los módulos por su carpeta. Otro crate sigue sin alcanzarlas. El lector de `every_public_memory_module_is_named_outside_core` toma como carpeta cada `pub(crate) mod <carpeta>;` de la raíz.
- **Dos módulos más en los ciclos listados.** El diseño esperaba los cinco grupos sin `latency` ni `search_log`:
  - `latency` nombra `MemoryStore` por la reexportación de la raíz, que la cuenta sigue hasta `db`, y `db` guarda su `KnnLatency`. La exploración no seguía esas reexportaciones, y por eso contaba 32 donde la regla cuenta 34.
  - `search_log` recibe los hits de la búsqueda (`&[RankedHit]`), y la búsqueda lo llama. Sin un archivo nuevo, con las cuatro funciones en `search.rs`, el grupo tendría un módulo menos; se dejaron aparte para no hacer crecer el archivo que más cambios abiertos tocan.
- **La tabla no se corrigió.** Las diez filas quedaron como en el diseño: ninguna dependencia rechazada y ningún par sin uso.
- **`conn_probe` no necesitó reexportación temporal:** nada del núcleo lo nombra por su ruta.
- **Una coma final en un grupo `use`** daba una ruta vacía en `leading_paths`. Se corrigió en el commit de las rutas, con su caso en `leading_paths_opens_groups_under_what_leads_to_them`.
- **Helpers de tests copiados** al partir `admin`: `shared` y `hiding` en `listing`, y `seed_memory_scoped` en `admin`. Un test de `listing` borra con `admin::delete_memory_core`; el código de tests no cuenta para la regla.
- **El commit de rutas** cambió 856 líneas en 64 archivos, una por una: ningún archivo cambió de largo. El diseño contaba 935 apariciones de `crate::memory::`, varias por línea.
- **`docs/reference/testing.md`** dice ahora cuáles son las carpetas, qué fijan las dos reglas y cómo se repite la cuenta. No estaba en las tareas.

## Pendiente

El CI del PR y la corrida en Windows ya se hicieron: están en [«El CI»](#el-ci) y en [«En Windows, en local»](#en-windows-en-local).

Queda lo que ni Linux ni el CI comprueban, porque pide el modelo real:

- **`rag-eval --real --docs`**, que necesita el modelo real. El índice mide docs y código juntos, y la ruta de cada archivo de `memory` cambió, así que hay que ver que ninguna pregunta se mueva por una ruta esperada:

  ```bash
  cargo run -p clawg-eval --bin rag-eval -- --real --docs --tag memory-module-layout
  ```

  ```bash
  cargo run -p clawg-eval --bin rag-eval -- --diff <el último {fecha}-real-docs.json de develop> <el archivo nuevo>
  ```

  Las 11 preguntas cuya respuesta esperada es un fuente de `memory` (las de `crates/clawg-eval/golden/project-docs.json` que nombran `crates/clawg-core/src/memory/`, con 13 rutas entre todas) son las que más pueden moverse. Si una pierde su acierto, hay que mirar si fue por la ruta (el breadcrumb de un pasaje de código lleva la ruta) y anotarlo acá.
- **Los tests con el modelo real** de `project_docs` y de la importación, que están `#[ignore]` y piden `CLAWG_E5_MODEL_DIR`. Este cambio no toca su código; correrlos confirma que las rutas de sus `use` siguen llevando al mismo lugar:

  ```bash
  cargo test -p clawg-core --lib -- --ignored
  ```

Y, fuera de este cambio, pasar las rutas de `project_docs` a la forma con carpeta cuando entre el otro cambio que edita `project_docs/mod.rs` ([D3](design.md#d3-project_docs-es-un-dominio-aparte-y-queda-donde-está)); con eso salen la excepción `NAMES_THROUGH_THE_ROOT` y las dos reexportaciones de más de `memory/mod.rs`.

## El CI

El CI del PR corrió sobre su último commit. El job `test (ubuntu-latest)` y el job `test (windows-latest)` pasaron. El de Windows compila el código `cfg(windows)` de los tres crates, que Linux no compila, y corre la suite con finales de línea CRLF: las reglas nuevas leen fuentes y `mod.rs`, y `declared_for_tests_only` y `production_code` parten las líneas con `str::lines`, que quita el `\r`.

## En Windows, en local

Se corrió el 2026-11-13 en Windows 11, sobre `develop` con este cambio y con la primera parte de core-use-cases, que agregó el módulo `ingest/save`. El build es debug, en `target-test`, limpiado antes de compilar con `cargo clean --workspace --target-dir target-test`, con el modelo falso y sin ningún perfil de datos.

| Comando | Resultado |
|---|---|
| `cargo test --workspace --target-dir target-test` | pasa, sin ningún fallo |
| `cargo clippy --workspace --all-targets --target-dir target-test -- -D warnings` | sin hallazgos |
| `cargo clippy -p clawg-cli --target-dir target-test -- -D warnings` | sin hallazgos |

La búsqueda de [«Las citas»](#las-citas) se repitió sobre las specs ya consolidadas y no imprime nada.

## Al archivar (2026-11-13)

- **Los ids cambian al consolidar.** R1 a R7 chocan con los ids de [`specs/quality`](../../../specs/quality/spec.md), así que R1 a R5 pasan a ML1 a ML5, en «Las carpetas de `memory`». R6 («Ninguna firma, tipo ni conducta cambia») es de conservación y no se consolida. R7 («Las citas siguen llevando al código») tampoco: fue un barrido de una vez, sin un test que lo sostenga, y queda en este archivo con su comando. Este archivo conserva los ids R.
- **Sale en 0.87.0.**
- **Los escenarios «hoy» no se consolidan.** Los de R1 y R4 describen el código de antes del cambio (50 módulos sueltos, un grupo de 34). ML1 y ML4 llevan en su lugar un escenario que vale sobre el árbol de ahora.
- **La lista de módulos de ML1 es la de hoy,** con `ingest/save`, que agregó core-use-cases después de este cambio. La regla pasó sin editarse: un módulo nuevo dentro de una carpeta no la toca mientras nombre sólo lo que su fila permite. ML1 lo dice.
- **Diferencias entre la spec del cambio y el código:** las de [«Lo que se resolvió distinto de lo escrito»](#lo-que-se-resolvió-distinto-de-lo-escrito). R5 pedía que las carpetas no se alcanzaran desde otro crate, y son `pub(crate) mod`, no privadas; ML5 dice lo que hace el código. ML4 lista los cinco grupos como quedaron, con `latency` y `search_log`.
- **SB9 de [`specs/storage`](../../../specs/storage/spec.md)** decía «cada `pub mod` de `crates/clawg-core/src/memory/upkeep/mod.rs`»: el barrido de citas de este cambio había llevado `memory/mod.rs` a una carpeta, como si `mod` fuera un módulo. Vuelve a nombrar la raíz, y dice las dos formas en que la raíz muestra un módulo.
- **Citas:** las de ML1 a ML5 se tomaron de `crates/clawg-cli/tests/conventions.rs` de hoy. En la sección que ya estaba, las tres de R7 de `specs/quality` (`tests_leaving_a_store_open` y sus dos tests) apuntaban a líneas que ya no son las suyas y se corrigieron. Las `ruta:línea` de `proposal.md`, `design.md` y `exploration.md` describen el código de antes de este cambio y no se corrigieron.
- **De «Pendiente»** se corrió lo de Windows. Lo que pide el modelo real no se corrió: `rag-eval --real` lee el modelo de un perfil de datos.
