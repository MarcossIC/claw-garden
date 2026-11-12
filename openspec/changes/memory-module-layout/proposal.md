# memory-module-layout — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L, casi todo mecánico | store-boundary, que ya está en `develop` | ver [`state.yaml`](state.yaml) |

## Problema

`crates/clawg-core/src/memory/` es una carpeta plana, y sus módulos dependen unos de otros en un solo círculo. Nada en el árbol dice dónde vive el almacenamiento, dónde la búsqueda o dónde la ingesta, y no se puede separar una parte sin arrastrar el resto. El detalle está en la [exploración](exploration.md#problema-verificado-contra-el-código), que es anterior a store-boundary. Lo que sigue es lo mismo, vuelto a comprobar contra `develop`:

1. **Una carpeta plana.** `crates/clawg-core/src/memory/mod.rs:26-98` declara 52 módulos en un nivel: 35 `pub mod`, 16 `pub(crate) mod` y uno de tests (`crowding`, `:97-98`). La exploración contó 51, con 44 `pub`. Sólo `portability`, `chunker` y `project_docs` son carpetas.
2. **Un ciclo de 34 módulos.** Contado como dice el [diseño](design.md#d7-la-regla-y-cómo-se-cuenta-el-ciclo), 34 de los 52 módulos dependen cada uno de sí mismo a través de otros. La exploración contó 32 de 51 sin seguir los nombres que reexporta la raíz (`MemoryStore`, `RankedHit`); con su método, hoy siguen siendo 32. Quitar `error` no lo disuelve: quedan uno de 24 y otro de 3.
3. **Las dependencias invertidas siguen:**
   - `error.rs` depende de `store_lock`, `backup` y `secrets` (`crates/clawg-core/src/memory/error.rs:1`, `:138`, `:142`), y 21 módulos dependen de `error`.
   - La búsqueda depende de `admin` para formatear sus logs (`crates/clawg-core/src/memory/search.rs:609`, `:1277`, `:1325` y `crates/clawg-core/src/memory/prompt_builder.rs:277`; los helpers están en `crates/clawg-core/src/memory/admin.rs:36-86`).
   - `db` conoce al embedder (`crates/clawg-core/src/memory/db.rs:9`, `:40`).
4. **`admin.rs` junta cosas sin relación:** el formato de logs de la búsqueda, actualizar, borrar y restaurar una memoria, los listados y las estadísticas, en 1.105 líneas de producción (`admin.rs:1106` abre los tests). La exploración contó 1.490.
5. **Tipos en lugares casuales.** `MemoryType` vive en `crates/clawg-core/src/memory/store_insert.rs:25` y `CreatedByType` en `:117`. Por eso `provenance`, `contradiction`, `dispute`, `relocate` y `admin` dependen del módulo que inserta.
6. **Los que más arrastran:** `portability` depende de 25 módulos, `admin` de 15, `worker` de 14, `project_delete` de 13 y `store_insert` de 12. Sin depender de ninguno quedan 11; la exploración contó 13, pero `dedup` y `tier` ahora usan la traducción única de `error.rs`.

## Qué resolvió store-boundary, y sale del alcance

- **Qué ven los otros crates.** store-boundary dejó `pub` sólo los módulos y los ítems que `clawg-cli` y `clawg-eval` usan, y lo fijó con `every_public_memory_module_is_named_outside_core` (`crates/clawg-cli/tests/conventions.rs:260`). Este cambio no decide ninguna visibilidad ni cambia ninguna ruta pública: los otros crates siguen escribiendo `clawg_core::memory::<módulo>` ([D4](design.md#d4-memory-se-conserva-y-las-rutas-públicas-no-cambian)). La exploración suponía «una fachada pública»; lo que hay es esa lista de módulos, y este cambio la conserva.
- **`db` ya no conoce `sharing.json` ni los imports en curso.** Antes de store-boundary `db.rs` importaba `sharing`; hoy no. De las dependencias de `db` hacia arriba queda sólo el embedder.
- **El error de almacenamiento.** `MemoryError` ya no lleva tipos de `rusqlite`, `tokio_rusqlite` ni `refinery`, y la traducción de un `call` fallido está en un solo lugar. De «sacar de `error.rs` los errores de otros módulos» queda la dirección: `MemoryError` sigue llevando `BackupError`, `PurgeError` y `StoreOwner`, y este cambio no puede cambiar sus variantes ([D5](design.md#d5-los-cortes)).

## Enfoque

El **A** de la exploración: mover por carpetas, con cada movimiento en un commit que no edita el contenido, las ediciones que rompen una dependencia invertida en commits aparte, y la regla de dependencias fijada con un test que se escribe primero. Separar en crates (el C) queda fuera.

Las tres decisiones abiertas se toman en el [diseño](design.md):

1. **`dispute`, `topic` y `contradiction` van a una carpeta propia, `revision`,** con `revival` ([D2](design.md#d2-dispute-topic-y-contradiction-la-carpeta-revision)). Los usan `projects` y `upkeep` sin pasar por la ingesta, y de la ingesta sólo necesitan `MemoryType`.
2. **`project_docs` es un dominio aparte, y no se mueve** ([D3](design.md#d3-project_docs-es-un-dominio-aparte-y-queda-donde-está)). Tiene sus tablas, no escribe en `memory`, y un test suyo lee su propia carpeta por ruta.
3. **`memory::` se conserva** ([D4](design.md#d4-memory-se-conserva-y-las-rutas-públicas-no-cambian)), y las rutas que usan los otros crates no cambian.

## Alcance

1. **El test en rojo:** la tabla de carpetas y de dependencias permitidas, y la lista de ciclos entre módulos, en `crates/clawg-cli/tests/conventions.rs`.
2. **Romper las dependencias invertidas** ([D5](design.md#d5-los-cortes)), cada una en su commit. Son movimientos de un ítem de un módulo a otro; ninguno cambia una firma ni un tipo.
3. **Partir `admin.rs`** ([D6](design.md#d6-admin-se-parte-en-tres)): el formato de logs de la búsqueda y los listados con las estadísticas salen a módulos propios. Queda actualizar, borrar y restaurar.
4. **Mover por carpetas** ([D1](design.md#d1-el-reparto)), un commit por carpeta, sin editar el contenido de lo que se mueve ([D8](design.md#d8-cómo-se-hace-cada-movimiento)).
5. **Nombrar la carpeta en las rutas del núcleo** (`crate::memory::store::db`), en un commit aparte.
6. **Las citas:** las rutas a `crates/clawg-core/src/memory/` de `openspec/specs` (22 archivos) y `docs/reference` (3 archivos), que son 452, y las rutas esperadas de `crates/clawg-eval/golden/project-docs.json` ([D9](design.md#d9-las-citas)). Lo archivado no se toca.

## Fuera de alcance

Lo que la exploración pone en [«No-alcance»](exploration.md#no-alcance):

- Cambiar firmas, tipos o conducta.
- Traits o capas de lectura y escritura (storage-read-side, storage-write-side).
- Separar los tests a archivos propios por sí solo. Los tests se mueven sólo con el código que prueban.

Tampoco entra:

- Separar en crates.
- El contenido de `crates/clawg-core/src/memory/project_docs/`, que otro cambio abierto edita: no se toca ni una línea ([D3](design.md#d3-project_docs-es-un-dominio-aparte-y-queda-donde-está)).
- `crates/clawg-cli/src/daemon/api.rs` y el resto de `clawg-cli` y `clawg-eval`, salvo `conventions.rs`: sus rutas siguen valiendo.
- Las rutas que citan los cambios abiertos de `openspec/changes/`. Cada uno las vuelve a comprobar cuando se retoma.
- core-use-cases y lo que sigue en la cadena, subir la versión, archivar, `openspec/README.md` y `openspec/ROADMAP.md`.

## Datos guardados

Ninguno. No cambia el esquema, ningún identificador guardado ni ningún archivo del directorio de datos. Las migraciones se siguen leyendo de `crates/clawg-core/migrations/`, que no se mueve. No hace falta migración ni vuelta atrás: una versión anterior abre la misma base.

## Cómo se mide

- **Los dos tests en rojo** (`conventions.rs`), que hoy fallan:
  - `a_memory_folder_names_only_the_folders_its_row_allows` nombra los 50 módulos que no están en una carpeta de la tabla.
  - `the_memory_modules_cycle_only_where_listed` imprime el ciclo de 34 módulos.
- **El ciclo baja de un grupo de 34 a cinco grupos, de 7, 5, 4, 2 y 2 módulos,** cada uno dentro de una carpeta y escrito con su motivo en `MEMORY_CYCLES`. Entre carpetas no queda ninguno.
- **La suite pasa sin cambiar lo que comprueba.** Ninguna aserción cambia. Los tests de una respuesta (`crates/clawg-cli/tests/api.rs`, `binary.rs`) y los dos contratos del agente pasan sin editarse y sin `CLAWG_BLESS`.
- **Los movimientos son puros:** cada commit de movimiento muestra sus archivos como renombrados al 100 %.
- **No hay métrica de `rag-eval`:** el cambio no toca la recuperación. Lo que sí puede moverse es `rag-eval --docs`, que compara rutas: queda como pendiente en local en `verify.md`.
