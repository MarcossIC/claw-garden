# memory-module-layout — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L, casi todo mecánico | store-boundary | exploración (borrador) |

**Tesis:** `clawg-core::memory` es una carpeta plana de 51 módulos, y 32 de ellos forman un solo ciclo de dependencias. Nada en el árbol dice dónde vive el almacenamiento, dónde la búsqueda o dónde la ingesta, y no se puede separar ninguna parte sin arrastrar el resto. Este cambio ordena por dominio y hace que el orden no se pueda perder.

## Problema (verificado contra el código)

- `crates/clawg-core/src/memory/mod.rs:24-89` declara 51 módulos en un nivel (44 `pub`, 7 `pub(crate)`). Sólo `portability`, `chunker` y `project_docs` son carpetas.
- **Ciclo.** Tomando como dependencia cada `crate::memory::X` y `super::X` en código de producción, 32 de los 51 módulos quedan en un solo grupo fuertemente conexo. Quitar `error` no lo disuelve: quedan uno de 25 y otro de 4.
- **Dependencias invertidas que lo arman:**
  - `error.rs` depende de `backup`, `secrets` y `store_lock` (`error.rs:1`, `:68`, `:72`), y casi todo depende de `error`.
  - La búsqueda depende de `admin` para formatear sus logs (`search.rs:1410`, `:1459`, con los helpers en `admin.rs:43-49`).
  - `db` conoce al embedder (`db.rs:8`, `:40`).
- **`admin.rs` junta cosas sin relación:** el formato de logs de la búsqueda, actualizar y borrar memorias, listados y estadísticas, en 1.490 líneas de producción.
- **Tipos en lugares casuales:** `MemoryType` vive en `store_insert.rs:25`, y la API lo importa de ahí.
- **Módulos que ya son una base limpia:** 13 no dependen de ningún otro de `memory`: `clock`, `dedup`, `events`, `identity`, `latency`, `migrations`, `model_download`, `query_context`, `record`, `redact`, `sharing`, `tier`, `vector`.
- **Los que más arrastran:** `portability` depende de 18 módulos, `project_delete` y `admin` de 14, `worker` de 13 y `store_insert` de 12.

## Estructura candidata

```
memory/
  domain/      tipos y reglas puras: clock, record, tier, score, redact, vector, provenance, events, sharing, identity
  store/       db, migrations, backup, store_lock, conn_probe, latency y el error de almacenamiento
  ingest/      worker, outbox, store_insert, pieces, chunker, contradiction, dispute, revival, topic
  retrieval/   search, query_context, prompt_builder, workspace, access, spill
  embedding/   embedder, idle_embedder, bootstrap, model_download
  projects/    project, registry, subfolder, merge, relocate, project_delete, project_docs/
  portability/ como hoy
  admin/       crud, listados, inspect, synthesis, maintenance, secrets
```

El reparto exacto se decide al escribir el diseño. El criterio es que el árbol diga qué hace el sistema.

Dirección permitida: `domain` no depende de nadie, `store` sólo de `domain`, y el resto de esos dos más lo que el diseño liste. La tabla de dependencias permitidas queda fijada en `crates/clawg-cli/tests/conventions.rs`, que ya es el lugar de estas reglas. Hoy ese test fallaría.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Mover por carpetas, con movimientos puros en commits propios y la regla de dependencias al final** | `git log --follow` sigue funcionando y la revisión es por commit | Choca con cualquier rama abierta sobre estos archivos, así que se hace entre cambios |
| B. Mover cada módulo cuando otro cambio lo toque | Sin ventana de conflicto | Nunca termina, y la regla de dependencias no se puede activar |
| C. Separar en crates ya (`clawg-store`, `clawg-domain`) | El compilador garantiza el límite, y quizás baje el tiempo de compilación | No se puede con un ciclo. Y la ganancia de compilación no está medida |

**Recomendado: A.** C queda como decisión posterior, con el ciclo ya roto y midiendo el tiempo de compilación antes y después.

## Alcance candidato

1. Romper las dependencias invertidas: sacar de `error.rs` los errores de otros módulos, mover el formato de logs de la búsqueda a `retrieval`, y sacar de `db` el conocimiento del embedder.
2. Partir `admin.rs` por responsabilidad.
3. Mover por carpetas, un commit por carpeta, sin editar el contenido en el mismo commit.
4. Fijar la tabla de dependencias en `tests/conventions.rs`.
5. Actualizar las citas: `openspec/specs` y `docs/reference` traen 348 rutas a `crates/clawg-core/src/memory/`, en 25 archivos. Lo archivado no se toca.

### No-alcance

- Cambiar firmas, tipos o conducta.
- Traits o capas de lectura y escritura: son storage-read-side y storage-write-side.
- Separar los tests a archivos propios por sí solo. El backlog ya lo sugiere para cuando se toque el archivo, y acá se puede aprovechar en los archivos que se mueven.

## Decisiones abiertas

1. Dónde van `dispute`, `topic` y `contradiction`: `ingest` los usa al guardar, pero tienen reglas propias.
2. Si `project_docs` queda en `projects` o es un dominio aparte. Tiene sus tablas y no escribe nada de `memory`.
3. Si `memory::` se conserva como nivel o las carpetas pasan a la raíz del crate.

## Cómo se mide

- **Test en rojo:** la tabla de dependencias en `conventions.rs`, que falla hoy.
- La suite pasa igual en cada commit, porque los movimientos no cambian conducta.
- El ciclo baja a cero, o a una lista corta y escrita con su motivo.

## Riesgos

- Las citas `ruta:línea` de specs y docs quedan viejas. Hay que barrerlas en el mismo cambio.
- El índice de docs y `rag-eval --docs` cuentan con rutas: conviene correrlo y ver que no se mueva ninguna pregunta por una ruta esperada.

## Interacciones

- **store-boundary:** va antes. Con una fachada pública, mover un módulo no toca a `clawg-cli` ni a `clawg-eval`.
- **core-use-cases, storage-read-side y storage-write-side:** usan estas carpetas como destino.
- **Cambios abiertos sobre `search.rs`, `worker.rs` y `embedder.rs`:** este cambio se hace entre ellos, no durante.
