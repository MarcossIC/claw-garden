# storage-read-side — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L, por dominio | store-boundary, memory-module-layout, domain-types; se coordina con read-connections | exploración (borrador) |

**Tesis:** leer y escribir no se distinguen en el código. Una lectura y una escritura reciben el mismo `&Connection`, así que sólo una convención impide que una lectura escriba. Las lecturas son más de la mitad del acceso a la base y no necesitan agregados: son consultas por caso de uso. Separarlas con un tipo es la mitad más barata del trabajo de almacenamiento, y es lo que deja a read-connections como un cambio de una sola línea de enrutamiento.

## Censo

Aproximación estática sobre el código de producción de `clawg-core`, sin `eval_api` ni lo que está bajo `#[cfg(test)]`: una función cuenta como escritura si ella o lo que llama, resuelto por nombre, escribe alguna tabla.

- **54 funciones con `.call(` sólo leen; 40 escriben.** Las que leen se reparten en `project_docs/mod.rs` (5), `registry.rs` (5), `admin.rs` (4), `db.rs` (4), `maintenance.rs` (4), `search.rs` (4), `synthesis.rs` (4), `prompt_builder.rs` (3) y otras.
- La clasificación de rutas de [read-connections](../read-connections/exploration.md) coincide: `POST /v1/search`, `POST /v1/context`, `GET /v1/memories`, `GET /v1/stats` y otras son de pura lectura.

## Problema (verificado contra el código)

- **El tipo no distingue.** `run` en `knn_one` recibe `&rusqlite::Connection` (`crates/clawg-core/src/memory/search.rs:576`), igual que `pass` de una mudanza, que escribe (`relocate.rs:587`). Hay 160 referencias a `&Connection` o `&Transaction` en producción.
- **Una lectura en varios pasos no es una foto.** Una búsqueda hace hasta 7 llamadas separadas a la conexión (`search.rs:1295`, `:1310`, `:1322`, `:1334`, `:1352`, `:1397`, `:1427`). Si una escritura entra entre dos de ellas, los pasos leen estados distintos.
- **La consulta y la decisión están juntas.** La clausura de `knn_one` (`search.rs:566-681`) lee, mapea, puntúa, ordena y recorta, todo en el hilo de la conexión. El puntaje y el orden son CPU que hacen esperar a la cola sin necesitarla.
- **Lo que se lee cruza tablas.** `SEARCH_SQL_MEMORY` une `memory_vectors` con `memory` (`search.rs:259`); `fetch_auto_rows` tiene 147 líneas (`workspace.rs:335`). Son modelos de lectura, no entidades.

## Forma candidata

```rust
// Sin Deref a Connection: un ReadTx no tiene execute.
store.read(|r: &ReadTx| retrieval::knn(r, &query)).await?;
```

- `ReadTx` envuelve la conexión y sólo expone preparar y consultar.
- Los módulos de consultas van por caso de uso (`retrieval/queries.rs`, `projects/queries.rs`, `admin/queries.rs`), devuelven tipos del dominio y hacen lo mínimo dentro del hilo.
- Puntuar y ordenar van afuera, en funciones puras.
- **Todavía sin traits.** El tipo concreto ya da la garantía que importa; extraer un trait es storage-ports, cuando exista una segunda implementación.

Por qué consultas por caso de uso y no un repositorio por tabla: la lectura real de este producto une tablas y aplica alcance y proyecto. Forzarla a `memory_repo.find_by(...)` obligaría a traer filas y filtrar en Rust, o a meter un método por cada consulta en un solo trait.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. `ReadTx` concreto y módulos de consultas por dominio** | El compilador impide escribir. Sin abstracción nueva. read-connections cambia sólo adónde va `read` | Muchas funciones por mover |
| B. Sólo una convención de nombres | Barato | Nada lo impide |
| C. Un trait de lectura desde el principio | Ya queda el puerto | Un trait con una sola implementación, contra `config.yaml`: «no abstraction without a real use» |
| D. Esperar a read-connections y usar `query_only` en el lector | El lector de sólo lectura lo impide en tiempo de ejecución | El error aparece al correr, no al compilar, y sólo si esa puerta abre |

**Recomendado: A.**

## Alcance candidato

1. `ReadTx`, `MemoryStore::read` y un test `compile_fail` en un doc comment que comprueba que no se puede escribir. No hace falta ninguna dependencia nueva.
2. Mover las lecturas por dominio: `project_docs`, `registry`, `admin`, `synthesis`, y al final `search` y `prompt_builder`.
3. Sacar del hilo de la conexión el puntaje y el orden de `knn_one`.
4. Fijar en `conventions.rs` que un módulo de consultas no contenga `INSERT`, `UPDATE` ni `DELETE`.

### No-alcance

- Abrir un lector aparte, que es read-connections.
- Cambiar SQL, planes o ranking.
- Traits.

## Decisiones abiertas

1. Si `read` abre una transacción diferida para que varios pasos sean una foto, o si eso se pide aparte. Hoy no hay foto, y nadie lo pidió como requisito; una transacción de lectura larga, además, retiene el WAL.
2. Si `search` pasa a hacer una sola llamada con todos sus pasos, lo que se conecta con search-arm-timing y read-connections.
3. Lecturas que preceden a una escritura en la misma transacción, como el dedup del worker: quedan del lado de la escritura, porque deciden lo que se escribe.

## Cómo se mide

- **Test en rojo:** el `compile_fail` y la regla de `conventions.rs`.
- `rag-eval --diff` con el embedder falso sin casos movidos.
- `perf-bench` y `search.duration` sin regresión; sacar el puntaje del hilo debería bajar lo que espera una escritura.

## Interacciones

- **read-connections:** no depende de este cambio ni este de aquél, y cualquiera puede ir primero. Si read-connections pasa su puerta, `read` se enruta al lector. Si no la pasa, este cambio igual deja la separación.
- **storage-write-side:** es la otra mitad.
- **storage-ports:** el trait de lectura sale de estos módulos, sin rediseñarlos.
- **search-index-ports:** usa la misma separación para `nearest`.
