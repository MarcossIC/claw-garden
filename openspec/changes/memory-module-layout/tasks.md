# memory-module-layout — Tareas

## Cómo se trabaja cada tarea

- **El test en rojo va primero** (tarea 2) y queda en rojo hasta la tarea 7: es la medida del cambio, y ningún commit intermedio puede ponerlo en verde. Los demás tests de `conventions.rs` pasan en cada commit.
- **Caracterización.** Este cambio no agrega conducta, así que no tiene otros tests en rojo. Lo que prueba que nada cambió son los tests que ya existen, que pasan sin editarse ([R6](spec.md#r6-ninguna-firma-tipo-ni-conducta-cambia)).
- **Commits.** Cada tarea es un commit convencional que compila y pasa los tests de los archivos que cambia, nombrados al final de la tarea. La suite entera y los dos clippy corren una vez, sobre el último commit (tarea 9).
- **Un corte no mueve archivos, y un movimiento no edita contenido.** Las tareas 3 y 4 editan; la 6 mueve; la 7 vuelve a editar.
- **Windows.** Cada `cargo` lleva `--target-dir target-test`.
- Es una versión nueva, así que antes de probar se limpia el directorio de build (`cargo clean --workspace`, o con `--target-dir target-test` en Windows).

## 1. Artefactos

- [x] `proposal.md`, `spec.md`, `design.md`, `tasks.md` y `state.yaml`.
- Commit: `docs(openspec): propose memory-module-layout`

## 2. El test en rojo (R1, R2, R3, R4)

- [x] En `crates/clawg-cli/tests/conventions.rs`: `MEMORY_FOLDERS`, `ONE_MODULE_FOLDERS`, `NAMES_THROUGH_THE_ROOT` y `MEMORY_CYCLES`; `memory_modules`, `memory_paths`, `module_named`, `memory_dependencies` y `cycles` ([D7](design.md#d7-la-regla-y-cómo-se-cuenta-el-ciclo)).
- [x] Los puros, que pasan desde este commit: `leading_paths_opens_groups_under_what_leads_to_them`, `memory_paths_reads_absolute_and_relative_paths`, `module_named_follows_a_folder_a_module_and_a_reexport` y `cycles_groups_the_nodes_that_reach_each_other`.
- [x] Los dos en rojo: `a_memory_folder_names_only_the_folders_its_row_allows` (50 módulos sueltos) y `the_memory_modules_cycle_only_where_listed` (un grupo de 34).
- Commit: `test: fix the folders of the memory modules and where they may depend in a circle`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

  Fallan esos dos, con esos números, y ningún otro.

## 3. Los cortes (R6)

Uno por commit, en el orden de [D5](design.md#d5-los-cortes). Cada uno mueve un ítem con sus tests y deja una reexportación donde otro crate lo nombra.

- [x] **`MemoryType`, `UnknownMemoryType` y `CreatedByType` a `record.rs`.** `store_insert` los reexporta. `provenance`, `contradiction`, `dispute`, `relocate`, `admin`, `crowding` y `portability` los importan de `record`.
  - Commit: `refactor(core): define the memory type and its author beside the record`
- [x] **`EmbeddingIdentity::of` a `embedder.rs`,** con `embedding_identity_of_reads_all_four_fields_off_one_embedder`.
  - Commit: `refactor(core): read an embedder's identity in the embedder module`
- [x] **`PurgeError` a `error.rs`.** `secrets` lo reexporta.
  - Commit: `refactor(core): define the purge error beside the error that carries it`
- [x] **Las cuatro `format_*` a `search_log.rs`,** con sus ocho tests.
  - Commit: `refactor(core): move the search log lines out of admin`
- [x] **`RESTORE_TWIN`, `RESTORE_TOPIC_HOLDER` y `RESTORE_TEXT_INDEX` a `revival.rs`.**
  - Commit: `refactor(core): keep the statements of a restore beside the revival of a row`
- [x] **`MIN_OVERLAP_CHARS` a `chunker/token_splitter.rs`.**
  - Commit: `refactor(core): define the shortest overlap where the splitter leaves it`
- [x] **`"claude-code"` a `provenance.rs`** como `CLAUDE_CODE`. `portability::claude_code::SOURCE` lo reexporta.
  - Commit: `refactor(core): name the source of a Claude Code import in provenance`
- Tests al cerrar cada uno:

  ```bash
  cargo test -p clawg-core --lib -- memory::<módulos que cambian>
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

## 4. `admin` se parte (R6)

- [x] **Los listados y las estadísticas a `listing.rs`** ([D6](design.md#d6-admin-se-parte-en-tres)), con sus tests. `admin` reexporta lo que era `pub`. `pieces` y `synthesis` importan de `listing`.
- Commit: `refactor(core): move the listings and the statistics out of admin`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-core --lib -- memory::admin memory::listing memory::pieces memory::synthesis
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

  ```bash
  cargo test -p clawg-cli --test api
  ```

## 5. La raíz reexporta (R5)

- [x] **Test en rojo:** `public_memory_modules_reads_declarations_and_reexports` (`conventions.rs`), con la raíz del escenario de R5. Hasta el cambio no compila: `public_memory_modules` no existe.
- [x] `every_public_memory_module_is_named_outside_core` lee los módulos públicos con `public_memory_modules`: los `pub mod <m>;` y los `pub use <carpeta>::<m>;`.
- Commit: `test: read the modules the memory root re-exports from a folder`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

## 6. Las carpetas (R1, R5, R6)

Un commit por carpeta, en el orden de la tabla, como dice [D8](design.md#d8-cómo-se-hace-cada-movimiento): `git mv`, el `mod.rs` de la carpeta y las líneas de `memory/mod.rs`. Ningún fuente movido cambia.

- [x] `domain`: `clock`, `events`, `provenance`, `record`, `redact`, `sharing`, `vector`.
- [x] `store`: `backup`, `conn_probe`, `db`, `dedup`, `error`, `latency`, `migrations`, `store_lock`, `tier`. `TRANSLATES` de `conventions.rs` pasa a `clawg-core/src/memory/store/error.rs`.
- [x] `embedding`: `chunker/`, `embedder`, `idle_embedder`, `model_download`.
- [x] `retrieval`: `access`, `crowding`, `language`, `listing`, `pieces`, `prompt_builder`, `query_context`, `score`, `search`, `search_log`, `workspace`.
- [x] `revision`: `contradiction`, `dispute`, `revival`, `topic`.
- [x] `projects`: `identity`, `inspect`, `merge`, `project`, `project_delete`, `registry`, `relocate`, `subfolder`.
- [x] `ingest`: `outbox`, `spill`, `store_insert`, `worker`.
- [x] `upkeep`: `admin`, `bootstrap`, `maintenance`, `secrets`, `synthesis`.
- Commit de cada una: `refactor(core): move <lo que es> into memory/<carpeta>`
- Tests al cerrar cada una:

  ```bash
  git show --stat -M100% HEAD
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

  ```bash
  cargo test -p clawg-core --lib -- memory::<carpeta>::
  ```

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

  El primero muestra sólo renombres al 100 %, los `mod.rs` y, en `store`, `conventions.rs`. En el último siguen fallando sólo los dos de la tarea 2.

## 7. Las rutas nombran la carpeta (R2, R3, R4)

- [x] Cada `crate::memory::<módulo>` de `crates/clawg-core/src` pasa a `crate::memory::<carpeta>::<módulo>`, y cada nombre que la raíz reexporta (`MemoryStore`, `MemoryError`, …) a la ruta de su módulo. `project_docs/` no se toca.
- [x] Salen de `memory/mod.rs` las reexportaciones `pub(crate)` de la tarea 6, menos las dos de [D4](design.md#d4-memory-se-conserva-y-las-rutas-públicas-no-cambian).
- [x] El comentario de módulo de `memory/mod.rs` dice qué es cada carpeta.
- [x] Los dos tests de la tarea 2 pasan. Si la cuenta de ciclos da otros grupos que los de `MEMORY_CYCLES`, o la tabla permite un par que nadie usa, se corrige con lo que dé y se anota para `verify.md`.
- Commit: `refactor(core): name each memory module by its folder`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-cli --test conventions
  ```

  ```bash
  cargo check --workspace --all-targets
  ```

  ```bash
  cargo check -p clawg-cli
  ```

  ```bash
  cargo test -p clawg-core --lib
  ```

## 8. Las citas (R7)

- [x] `openspec/specs` y `docs/reference`: cada `crates/clawg-core/src/memory/<módulo>` a su carpeta, y cada `ruta:línea` de un archivo que cambió en las tareas 3 y 4, a donde quedó ese texto ([D9](design.md#d9-las-citas)).
- [x] `docs/reference/testing.md` dice cuáles son las carpetas, qué fijan las dos reglas y cómo se repite la cuenta del ciclo.
  - Commit: `docs: cite the memory sources by their folders`
- [x] `crates/clawg-eval/golden/project-docs.json`: las 13 rutas esperadas.
  - Commit: `test(eval): expect the memory sources of the docs golden in their folders`
- Tests al cerrar:

  ```bash
  cargo test -p clawg-eval
  ```

  Y la búsqueda de `verify.md`, que no tiene que listar ninguna ruta que no exista.

## 9. Verificación

- [x] Sobre el último commit de código, en Linux:

  ```bash
  cargo test --workspace
  ```

  ```bash
  cargo clippy --workspace --all-targets -- -D warnings
  ```

  ```bash
  cargo clippy -p clawg-cli -- -D warnings
  ```

- [ ] El CI del PR, que corre también Windows y compila el código `cfg(windows)`.
- [x] Las comprobaciones de [D10](design.md#d10-cómo-se-comprueba-que-nada-cambia).
- [x] `verify.md`: la suite y los clippy, los dos tests en rojo antes y después, la cuenta del ciclo, las comprobaciones de D10, lo que se resolvió distinto de lo escrito, y lo pendiente en local. Por lo menos: `rag-eval --real --docs` con el golden, mirando que ninguna pregunta se mueva por una ruta esperada.
- [x] `state.yaml`.
- Commit: `docs(openspec): record memory-module-layout applied`

El archivo (specs principales, `openspec/README.md`, `openspec/ROADMAP.md`) y la subida de versión no son parte de estas tareas.
