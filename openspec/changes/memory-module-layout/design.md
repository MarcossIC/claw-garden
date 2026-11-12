# memory-module-layout — Diseño

El enfoque de fondo (el A) está en la [exploración](exploration.md#enfoques-posibles). Acá van el reparto, sus tres decisiones abiertas (D2, D3 y D4) y las decisiones de implementación.

Las `ruta:línea` son de `develop` al escribir esto, antes de mover nada. Las rutas de `memory` se escriben desde `crates/clawg-core/src/memory/`, salvo que digan otra cosa.

## D1. El reparto

Diez carpetas. La columna «Puede nombrar» es la tabla que fija el test ([D7](#d7-la-regla-y-cómo-se-cuenta-el-ciclo)): cada fila nombra sólo filas de más arriba, así que la tabla no puede tener un ciclo.

| Carpeta | Módulos | Puede nombrar |
|---|---|---|
| `domain` | `clock`, `events`, `provenance`, `record`, `redact`, `sharing`, `vector` | — |
| `store` | `backup`, `conn_probe`, `db`, `dedup`, `error`, `latency`, `migrations`, `store_lock`, `tier` | `domain` |
| `embedding` | `chunker/`, `embedder`, `idle_embedder`, `model_download` | `domain`, `store` |
| `retrieval` | `access`, `language`, `listing`, `pieces`, `prompt_builder`, `query_context`, `score`, `search`, `search_log`, `workspace`, y `crowding` (sólo tests) | `domain`, `store`, `embedding` |
| `revision` | `contradiction`, `dispute`, `revival`, `topic` | `domain`, `store`, `retrieval` |
| `project_docs` | la carpeta es el módulo | `domain`, `store`, `embedding`, `retrieval` |
| `projects` | `identity`, `inspect`, `merge`, `project`, `project_delete`, `registry`, `relocate`, `subfolder` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs` |
| `ingest` | `outbox`, `spill`, `store_insert`, `worker` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `projects` |
| `upkeep` | `admin`, `bootstrap`, `maintenance`, `secrets`, `synthesis` | `domain`, `store`, `embedding`, `retrieval`, `revision`, `project_docs`, `projects` |
| `portability` | la carpeta es el módulo | todas menos `project_docs` |

`listing` y `search_log` son los dos módulos que salen de `admin.rs` ([D6](#d6-admin-se-parte-en-tres)).

**Dónde se aparta de la estructura candidata de la exploración, y por qué.** El criterio es el de la exploración: que el árbol diga qué hace el sistema. Donde el nombre y el código no coinciden, manda el código, porque la regla se comprueba contra él.

| Módulo | La exploración | Acá | Por qué |
|---|---|---|---|
| `tier`, `dedup` | `domain` | `store` | La conexión registra sus funciones de SQL al abrir (`db.rs:428-430`), y las dos fallan con la traducción de `error.rs` (`tier.rs:11`, `dedup.rs:10`). En `domain`, `store` dependería de ellas y ellas de `store` |
| `score` | `domain` | `retrieval` | Sólo lo usa la búsqueda (`search.rs`), y es la regla del ranking |
| `identity` | `domain` | `projects` | Deriva la identidad de un proyecto, y sólo lo usan `project`, `registry` y `portability` |
| `chunker` | `ingest` | `embedding` | Corta el texto a la ventana del embedder (`chunker/mod.rs` usa `EMBED_MAX_TOKENS`). Lo usan `project_docs` y `project_delete`, que quedan por debajo de `ingest` |
| `pieces` | `ingest` | `retrieval` | `search` y `pieces` se usan uno al otro (`search.rs:9`, `pieces.rs:14`), y el prompt lee un save entero con `read_save_core` (`prompt_builder.rs:8`) |
| `language` | no existía | `retrieval` | Cuenta los saves por idioma con los predicados de la búsqueda (`language.rs:12`) |
| `spill` | `retrieval` | `ingest` | Es el archivo de ingesta que una versión anterior dejaba al cerrar (`spill.rs:1-4`) |
| `inspect` | `admin` | `projects` | `project_delete` lee el registro con sus funciones (`project_delete.rs:24`), y lo único que `inspect` usa de más arriba que `store` es `MAX_MERGE_HOPS`, de `merge` (`inspect.rs:15`) |
| `bootstrap` | `embedding` | `upkeep` | Reembebe el store cuando cambia el contrato, con `maintenance` (`bootstrap.rs:27`), y usa `chunker::word_tokenizer` (`:479`). Nada de `memory` depende de él: lo llama `crates/clawg-core/src/runtime.rs` |
| `admin/` | carpeta `admin` | carpeta `upkeep` | Los otros crates nombran `clawg_core::memory::admin::…`, y esa ruta no puede cambiar ([D4](#d4-memory-se-conserva-y-las-rutas-públicas-no-cambian)). Una carpeta `admin` con un módulo `admin` adentro dispara `clippy::module_inception`, y con el módulo en su `mod.rs` el movimiento ya no es puro |

**`ingest` queda por encima de `projects`.** El worker resuelve el proyecto canónico de un save antes de guardarlo (`worker.rs:797`, `merge::canonical`). Con `chunker` en `embedding` y `MemoryType` en `domain` ([D5](#d5-los-cortes)), nada de `projects` necesita la ingesta.

**`upkeep` e `ingest` no se nombran.** `upkeep` no figura en la fila de `ingest`, ni al revés.

## D2. `dispute`, `topic` y `contradiction`: la carpeta `revision`

**Decisión:** una carpeta propia, `revision`, con `contradiction`, `dispute`, `topic` y `revival`. Dice cómo se relaciona un save con los anteriores: lo contradice, lo disputa, lo reemplaza, o lo trae de vuelta.

**Por qué no `ingest`:**

- Los usa código que no ingiere nada: `project_delete` (`project_delete.rs:22`), `secrets` (`secrets.rs:16`, `:18`), `maintenance` (`maintenance.rs:14`, `:16`) y `admin` (`admin.rs:12-13`, `:15`, `:21`). Con los cuatro en `ingest`, `projects` dependería de `ingest`, e `ingest` de `projects` por `worker.rs:797`.
- De la ingesta sólo necesitan un tipo: `MemoryType` (`contradiction.rs:15`, `dispute.rs:25`). Con el tipo en `domain`, no les queda ninguna dependencia de `ingest`.
- Lo que sí necesitan es la búsqueda (`contradiction.rs:13`, `dispute.rs:23`, `topic.rs:17`) y los `pieces` (`topic.rs:16`).
- Entre ellos se usan (`topic.rs:12-13`, `revival.rs:12-14`), y la ingesta los usa a ellos (`outbox.rs:21`), no al revés.

**Descartado:** repartirlos (por ejemplo `contradiction` en `retrieval`). `topic` y `revival` nombran a los otros dos, así que separarlos sólo agrega dependencias entre carpetas.

## D3. `project_docs` es un dominio aparte, y queda donde está

**Decisión:** `project_docs` es una fila de la tabla y su carpeta sigue siendo `crates/clawg-core/src/memory/project_docs/`. No se mueve ni se edita.

**Por qué es un dominio aparte:**

- Tiene sus tablas: `project_doc_file`, `project_doc_chunk`, `project_doc_vectors` y `project_doc_fts` (`crates/clawg-core/migrations/V12__project_docs.sql:10`, `:22`, `:36`, `:45`).
- Su código de producción no escribe en `memory`: ningún `INSERT`, `UPDATE` ni `DELETE` de `project_docs/` nombra esa tabla.
- No depende de nada de `projects`. Sus dependencias son `chunker`, `clock`, `embedder`, `redact`, `search`, `vector` y el store (`project_docs/mod.rs:17-23`).
- `projects` depende de él, no al revés (`project_delete.rs`, `drop_root_index_in`).

**Por qué no se mueve:** con una fila propia, la carpeta ya está en su lugar. Y dentro de `projects/` un movimiento no podría ser puro: `production_never_names_the_archive` lee los fuentes de `src/memory/project_docs` por esa ruta (`project_docs/mod.rs:1215`), así que habría que editarlo en el mismo commit.

**Sus rutas.** `project_docs` sigue escribiendo `crate::memory::chunker`, `crate::memory::search` y `crate::memory::MemoryStore`, sin la carpeta. Para que resuelvan, la raíz conserva dos reexportaciones de más ([D4](#d4-memory-se-conserva-y-las-rutas-públicas-no-cambian)), y la regla lo lista como la única carpeta que nombra por la raíz (`NAMES_THROUGH_THE_ROOT`). Otro cambio abierto edita `project_docs/mod.rs`; cuando entre, pasar esas rutas a la forma con carpeta es un commit de `use`, y con él salen la excepción y las dos reexportaciones.

## D4. `memory::` se conserva, y las rutas públicas no cambian

**Decisión:** las carpetas quedan bajo `crate::memory`, y `clawg_core::memory::<módulo>` sigue valiendo para los otros crates.

**Por qué `memory::` se conserva:**

- La raíz del crate tiene otros diez módulos (`crates/clawg-core/src/lib.rs`: `build_info`, `metrics`, `models`, `paths`, `preferences`, `private_dir`, `runtime`, `eval_api`, …) que no son la memoria. Con las carpetas en la raíz, `store` o `domain` dirían menos de lo que dicen bajo `memory`.
- Los otros crates escriben `clawg_core::memory::` 264 veces, en 44 archivos. Pasar las carpetas a la raíz cambia todas, y con ellas `crates/clawg-cli/src/daemon/api.rs`, que otro cambio abierto edita.
- Separar en crates queda fuera. Si se hace, es el momento de decidir los nombres de primer nivel.

**Las rutas públicas.** Las carpetas son módulos del crate (`pub(crate) mod`): otro crate no las alcanza, y `crates/clawg-core/src/runtime.rs` y `eval_api.rs`, que están fuera de `memory`, sí. `memory/mod.rs` reexporta, una por línea, los módulos que hoy son `pub mod`:

```rust
pub(crate) mod store;
pub use store::backup;
pub use store::db;
pub use store::db::MemoryStore;
```

Así:

- `clawg_core::memory::db::MemoryStore` y `clawg_core::memory::MemoryStore` siguen resolviendo. Ningún archivo de `clawg-cli` ni de `clawg-eval` cambia, salvo `conventions.rs`.
- La lista de lo que ven los otros crates queda en un solo archivo, que core-use-cases va a ir achicando.
- `every_public_memory_module_is_named_outside_core` lee esas líneas, además de los `pub mod` que queden (`portability`, `project_docs`). La regla es la misma: un módulo que ningún otro crate nombra no se reexporta.

**Dentro del núcleo se nombra la carpeta:** `crate::memory::store::db::MemoryStore`. La regla rechaza una ruta que llegue a un módulo por la reexportación de la raíz ([D7](#d7-la-regla-y-cómo-se-cuenta-el-ciclo)). Sin eso habría dos formas de escribir cada ruta, y la que no nombra la carpeta esconde hacia dónde va la dependencia.

**Las dos reexportaciones de más,** sólo para `project_docs` ([D3](#d3-project_docs-es-un-dominio-aparte-y-queda-donde-está)): `pub(crate) use retrieval::search;` (`project_docs/mod.rs:21`) y, bajo `cfg(test)`, `pub(crate) use embedding::model_download;` (sus tests, `project_docs/mod.rs:2289`).

**Descartado:**

- **Carpetas `pub` y los otros crates con la ruta nueva** (`clawg_core::memory::store::db`). Cambia 264 rutas, 37 de ellas en `api.rs` y 7 fuera de sus `use`, sin que ningún llamador gane nada: core-use-cases les cambia esas rutas de nuevo.
- **Dejar que el núcleo siga con las rutas planas.** No toca ningún `use`, pero el árbol diría una cosa y el código otra.

## D5. Los cortes

Cada uno mueve un ítem a otro módulo, sin cambiar su firma ni su definición. Donde otro crate nombra el ítem por su ruta de antes, el módulo de antes lo reexporta.

| # | Dependencia invertida | Corte |
|---|---|---|
| 1 | `provenance` → `store_insert` (`provenance.rs:10`), y `contradiction`, `dispute`, `relocate` y `admin` → `store_insert` por `MemoryType` | `MemoryType`, `UnknownMemoryType` y `CreatedByType` (`store_insert.rs:25`, `:103`, `:117`) pasan a `record.rs`, con sus tests. `store_insert` los reexporta: `clawg-cli` escribe `store_insert::MemoryType` |
| 2 | `db` → `embedder` (`db.rs:9`, `:40`) | El `impl EmbeddingIdentity { fn of }` (`db.rs:37-48`) pasa a `embedder.rs`, con su test. El tipo sigue en `db.rs`; un `impl` inherente puede vivir en cualquier módulo del crate, y los llamadores siguen escribiendo `EmbeddingIdentity::of` |
| 3 | `error` → `secrets` (`error.rs:142`) | La definición de `PurgeError` (`secrets.rs:105-120`) pasa a `error.rs`. `secrets` la reexporta. `MemoryError::Purge` no cambia |
| 4 | `search` y `prompt_builder` → `admin` (`search.rs:609`, `:1277`, `:1325`; `prompt_builder.rs:277`) | Las cuatro funciones `format_*` (`admin.rs:36-86`) pasan a `search_log.rs`, con sus tests |
| 5 | `pieces` → `admin` (`pieces.rs:12`), `synthesis` → `admin` (`synthesis.rs:18`) | Los listados y las estadísticas pasan a `listing.rs` ([D6](#d6-admin-se-parte-en-tres)) |
| 6 | `project_delete` → `admin` (`project_delete.rs:20`) | `RESTORE_TWIN`, `RESTORE_TOPIC_HOLDER` y `RESTORE_TEXT_INDEX` (`admin.rs:423`, `:437`, `:456`) pasan a `revival.rs`: son lo que comprueba y escribe quien trae de vuelta una fila borrada, y los usan restaurar una memoria y deshacer el borrado de un proyecto |
| 7 | `chunker` → `pieces` (`chunker/token_splitter.rs:10`) | `MIN_OVERLAP_CHARS` (`pieces.rs:60`) pasa a `chunker/token_splitter.rs`, que es quien deja ese solapamiento; `pieces` lo lee de ahí para volver a unir |
| 8 | `inspect` → `portability` (`inspect.rs:338`) | El nombre de la fuente, `"claude-code"` (`portability/claude_code.rs:16`), pasa a `provenance.rs` como `CLAUDE_CODE`: es lo que el import escribe en `created_by_ref`. `claude_code` lo reexporta como `SOURCE` |

**Lo que no se corta:**

- **`error` ↔ `backup` y `error` ↔ `store_lock`.** `MemoryError` lleva `BackupError` y `StoreOwner` (`error.rs:138`, `:1`), y esos módulos fallan con `MemoryError`. Quedan dentro de `store`, así que no rompen la tabla. Cortarlos es mover las dos definiciones a `error.rs` o quitar las variantes, y lo segundo cambia un tipo. Quedan listados ([D7](#d7-la-regla-y-cómo-se-cuenta-el-ciclo)).
- **Los predicados de visibilidad** (`reader_predicate!`, `reach_predicate`, `reader_bind`, `project_bind`), que viven en `search.rs` y por los que `revision`, `projects`, `ingest` y `upkeep` dependen de la búsqueda. La dirección es la permitida. Sacarlos a un módulo propio edita `search.rs` más de lo que este cambio necesita.

## D6. `admin` se parte en tres

`admin.rs` tiene hoy cuatro cosas (`admin.rs:36-86`, `:103-694`, `:699-960`, `:972-1104`). Quedan así:

| Módulo | Qué | De `admin.rs` |
|---|---|---|
| `search_log` (`retrieval`) | las líneas de log de la búsqueda | las cuatro `format_*` |
| `listing` (`retrieval`) | leer filas: una por id, una lista con filtros, y contarlas | `MemoryAdminRow`, `ListParams`, `query_has_tokens`, `RowFilter`, `list_memories_core`, `SELECT_MEMORY`, `get_memory_core`, `map_memory_row`, y `StatsParams`, `Count`, `StatsTotals`, `MemoryStats`, `memory_stats_core` |
| `admin` (`upkeep`) | cambiar una memoria: actualizarla, borrarla, restaurarla | el resto |

- **Los listados y las estadísticas van juntos** porque comparten `RowFilter` y `push_filters` (`admin.rs:773-850`, `:1029`).
- **Van a `retrieval`** porque `pieces` lee la fila de la cabeza de un save con `get_memory_core` (`pieces.rs:12`) y filtran con los predicados de la búsqueda (`admin.rs:17`).
- **`admin` reexporta** lo que era `pub` en él y pasa a `listing`. `clawg-cli` lo importa de `clawg_core::memory::admin` (`crates/clawg-cli/src/daemon/api.rs`, el `use` de `admin`).
- **Los tests se mueven con su código.** Los helpers que usan los dos lados se copian al módulo que los necesita; son constructores de filas de pocas líneas.

**Descartado:** partir también actualizar, borrar y restaurar. Los tres comparten los helpers de sus tests y las consultas con alcance (`UPDATE_READ`, `RESTORE_READ`), y ninguna dependencia invertida pasa entre ellos.

## D7. La regla, y cómo se cuenta el ciclo

En `crates/clawg-cli/tests/conventions.rs`, con las funciones que ya tiene (`rust_files`, `:26`; el corte del código de producción de `production_lines_naming`, `:140`; `top_level_paths`).

**Cómo se cuenta una dependencia** (`memory_dependencies`). Es una sola forma de contar, y la usan las dos reglas:

1. **Los módulos.** Cada entrada de una carpeta de la tabla es un módulo: un `.rs`, o un directorio con sus fuentes. `project_docs` y `portability` son un módulo cada una. Lo que esté suelto en `memory/` también es un módulo, sin carpeta. `mod.rs` de `memory` no cuenta.
2. **El código de producción** de cada fuente: lo que hay antes de su primer `#[cfg(test)]` seguido de `mod <nombre> {`, sin las líneas de comentario. Un fuente que su `mod.rs` declara bajo `#[cfg(test)]` (`crowding`) no tiene código de producción.
3. **Las rutas.** En ese código, cada ruta que empieza en `crate::memory::` o en un `super::` es una dependencia del módulo al que lleva. Un grupo `{…}` da una ruta por cada elemento.
   - `crate::memory::store::db::…` lleva a `db`, de `store`.
   - `crate::memory::db::…` lleva a `db`, donde esté.
   - `crate::memory::MemoryStore` lleva al módulo del que la raíz lo reexporta.
4. Una ruta al propio módulo no cuenta.

**`a_memory_folder_names_only_the_folders_its_row_allows`** comprueba:

- que cada fila de `MEMORY_FOLDERS` nombra sólo filas de más arriba;
- que no hay módulos sueltos y que cada carpeta de la tabla existe;
- que toda ruta `crate::memory::…` lleva a un módulo;
- que una dependencia entre dos carpetas está en la fila de la que la escribe;
- que la ruta nombra la carpeta, salvo en `project_docs` ([D3](#d3-project_docs-es-un-dominio-aparte-y-queda-donde-está));
- que cada par permitido se usa. Una fila que permite más de lo que el código hace deja pasar una dependencia nueva sin que nadie la decida.

**`the_memory_modules_cycle_only_where_listed`** arma el grafo de módulos con esas dependencias y busca los grupos de más de un módulo en los que cada uno llega a los demás. Tienen que ser exactamente los de `MEMORY_CYCLES`, que lleva el motivo de cada uno. Cuando falla, imprime cada grupo con su tamaño.

**Para repetir la cuenta:**

```bash
cargo test -p clawg-cli --test conventions the_memory_modules_cycle_only_where_listed
```

Sobre `develop` imprime un grupo de 34 módulos, de 52. Al terminar quedan cinco, de 7, 5, 4, 2 y 2 módulos, de 54 (los 52 más `listing` y `search_log`):

| Grupo | Motivo |
|---|---|
| `store`: `backup`, `db`, `dedup`, `error`, `latency`, `store_lock`, `tier` | `MemoryError` lleva `BackupError` y `StoreOwner`, y los módulos que los definen, y los que registran funciones de SQL en la conexión, fallan con `MemoryError`. El store guarda la latencia de su búsqueda vectorial (`db.rs:11`), que se mide sobre el store |
| `retrieval`: `listing`, `pieces`, `search`, `search_log` | un hit lleva el lugar de su pieza en el save (`pieces.rs:14`), un save se lee entero desde la fila de su cabeza (`pieces.rs:12`, `search.rs:9`), y las líneas de log de una búsqueda cuentan sus hits (`RankedHit`) |
| `retrieval`: `prompt_builder`, `workspace` | `workspace.md` se escribe con la sangría del prompt (`workspace.rs`, `indent_continuations`), y el prompt lo lee |
| `projects`: `inspect`, `merge`, `project_delete`, `relocate`, `subfolder` | un merge, un movimiento y un borrado de proyecto corren los pasos de los otros dentro de una transacción (las funciones `_in`) |
| `projects`: `project`, `registry` | registrar una carpeta lee los nombres del registro, y el registro reconoce un id de proyecto por su forma (`is_project_id`) |

La lista es la que dio la cuenta al aplicar. Antes de aplicar, este diseño esperaba los mismos cinco grupos sin `latency` ni `search_log`; [`verify.md`](verify.md#lo-que-se-resolvió-distinto-de-lo-escrito) dice por qué entraron.

**Descartado:** contar con una herramienta aparte (`cargo-modules`, un script). El test ya está donde viven estas reglas, corre en el CI y no agrega una dependencia.

## D8. Cómo se hace cada movimiento

Un commit por carpeta. Cada uno:

1. mueve los archivos con `git mv`, sin tocar su contenido;
2. crea `<carpeta>/mod.rs`, que declara los módulos con la visibilidad y el `cfg` que tenían en `memory/mod.rs`;
3. en `memory/mod.rs`, declara la carpeta y cambia cada `pub mod x;` por `pub use <carpeta>::x;` y cada `pub(crate) mod x;` por `pub(crate) use <carpeta>::x;`.

Con el paso 3, toda ruta `crate::memory::x` del núcleo sigue resolviendo, así que el commit compila y pasa los tests sin editar lo movido. Se puede hacer así porque ningún fuente de `memory` nombra a un hermano con `super::`: los 15 `super::` de producción son de un archivo a otro del mismo módulo (`chunker/chat.rs:18-19`, `portability/translate.rs:24-26`, …).

`conventions.rs` cambia en dos de esos commits, porque nombra archivos: `TRANSLATES` (`conventions.rs:119`) pasa a `store/error.rs` con `store`, y el lector de `every_public_memory_module_is_named_outside_core` aprende las líneas `pub use <carpeta>::<módulo>;` antes del primer movimiento.

Después de las carpetas, **un commit de rutas**: cada `crate::memory::x` del núcleo pasa a `crate::memory::<carpeta>::x`, y salen las reexportaciones `pub(crate)` del paso 3, menos las dos de [D4](#d4-memory-se-conserva-y-las-rutas-públicas-no-cambian). Es una sustitución por tabla (módulo → carpeta) que el compilador comprueba: 935 rutas en 64 archivos. No toca `project_docs/`.

**Orden:** los cortes ([D5](#d5-los-cortes), [D6](#d6-admin-se-parte-en-tres)) van antes de mover, sobre el árbol plano, para que cada carpeta nazca sin dependencias prohibidas.

## D9. Las citas

- **`openspec/specs` y `docs/reference`:** 452 rutas a `crates/clawg-core/src/memory/` en 25 archivos. Cada `crates/clawg-core/src/memory/<módulo>` pasa a su carpeta.
- **Las líneas.** Los movimientos no corren ninguna línea. Los cortes y la partición de `admin.rs` sí. Una cita `ruta:línea` a un archivo que cambió se lleva a la línea donde quedó el mismo texto, en el archivo donde quedó.
- **`crates/clawg-eval/golden/project-docs.json`** nombra 13 rutas de `memory` como respuesta esperada (`:140-190`). Pasan a la ruta nueva, o `rag-eval --docs` contaría como fallo un acierto.
- **No se tocan:** lo archivado, los cambios abiertos de `openspec/changes/`, y los textos de ejemplo que los tests usan como contenido de una memoria (`language.rs:196`, `store_insert.rs:1112`, `crates/clawg-core/tests/e5_golden.rs:265`).

## D10. Cómo se comprueba que nada cambia

- **Los movimientos:** `git show --stat -M100%` de cada commit de carpeta muestra sólo renombres al 100 %, más `mod.rs` y, donde toca, `conventions.rs`.
- **Las aserciones:** en el diff de `develop...HEAD` de los tres crates, ninguna línea de `assert` cambia; las de los tests que se mueven con su código aparecen quitadas en un archivo y agregadas iguales en otro.
- **Lo que ve un cliente:** `crates/clawg-cli/tests/api.rs` y `binary.rs` pasan sin editarse, y los dos contratos del agente pasan sin `CLAWG_BLESS` y con el diff vacío.
- **Los otros crates:** el diff de `crates/clawg-cli/src` y `crates/clawg-eval/src` es vacío.
