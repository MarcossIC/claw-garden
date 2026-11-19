# domain-types — Diseño

Las tres decisiones abiertas de la [exploración](exploration.md#decisiones-abiertas) son D3 (los literales del SQL) y, fuera de esta tanda, `ProjectId` y los ids por tabla. Las demás salieron al aplicar.

## D1. Dos direcciones: lo que entra es estricto, lo leído es `Stored<T>`

`MemoryType` y `MemoryStatus` son enums cerrados, `Copy`. Lo que entra por la API se parsea a ellos y un nombre desconocido se rechaza. Lo que se lee de la base es `Stored<T>`:

```rust
pub enum Stored<T> { Known(T), Other(String) }
pub type StoredType = Stored<MemoryType>;
pub type StoredStatus = Stored<MemoryStatus>;
```

| Alternativa | A favor | En contra |
|---|---|---|
| **`Stored<T>` aparte del enum (elegida)** | El enum sigue `Copy` y exhaustivo; el código sólo construye `Other` leyendo (`Stored::read`) | Un tipo más, y `known()` en cada decisión |
| Una variante `Other(String)` dentro de cada enum | Un solo tipo | El enum deja de ser `Copy`, y la API podría recibir o escribir un `Other` |
| Dos enums con su `Other`, sin genérico | Sin trait | Dos copias de `ToSql`, `FromSql`, `Serialize`, `Deserialize`, `Display`, `Debug` y `PartialEq` |

El genérico tiene dos usos y un trait, `StoredName` (`as_str`, `from_name`), que implementan los dos enums. `Stored<T>` no implementa `From<&str>` ni `PartialEq<&str>`: comparar con un literal o construir uno desde texto en código de producción deja de compilar, que es el punto del cambio. Un test que necesita una fila con un nombre arbitrario la construye con `Stored::read`.

`PartialEq` compara el texto, así que un `Other("active")` construido a mano es igual a `Known(Active)`. `Debug` imprime el texto como lo imprimía el `String`, así que ningún log ni mensaje que formateaba una fila cambia.

## D2. Una decisión con un nombre desconocido toma el camino de hoy

Cada `match` lista sus variantes y pone el `None` de `known()` junto a las que no hacen nada, en vez de un comodín: una variante nueva no compila hasta que cada decisión la ubique. Lo que hace `None` es lo que hacía el `_` del `match` sobre texto ([spec R4](spec.md#r4-las-decisiones-se-toman-sobre-el-enum)). `SourceState::of` deja viva a una fuente de estado desconocido porque así la deja hoy `Some(_) => Live`; cambiarlo sería una conducta nueva.

## D3. Los literales siguen en el SQL; un test los comprueba

| Alternativa | A favor | En contra |
|---|---|---|
| **Literales en el texto, comprobados por un test de convenciones (elegida)** | Ninguna consulta cambia; el planificador sigue viendo el literal de los índices parciales | La comprobación es un test, no el compilador |
| Constantes en `concat!`, como `reach_predicate!` | El compilador ve el nombre | Reescribe unas setenta y cinco líneas de SQL de producción, muchas dentro de `const &str` de varias líneas, y el cambio choca con todo lo abierto |
| Parámetros enlazados | Un solo lugar | Un `?` no coincide con el `WHERE` de los índices parciales (`V1__init.sql:54`, `V17__revival.sql:38`, `V18__session_indexes.sql:11`, `V22__memory_lang.sql:19`), y el planificador deja de usarlos |

`every_status_and_type_the_sql_compares_is_one_the_code_knows` lee el código de producción de los tres crates (hasta su primer `#[cfg(test)] mod`) y las migraciones, y cada literal que se compara con `status` o `memory_type` tiene que parsear con `MemoryStatus::from_name` o `MemoryType::from_str`. Un `'superseeded'` en una consulta falla ahí.

## D4. Los filtros son lecturas; el estado de la API se valida

Un filtro por tipo (`ListParams::memory_type`, `MoveFilter::memory_type`) es un `StoredType`: el daemon acepta hoy cualquier texto, y es lo que permite listar o mover las filas `observation` que dejó el esquema V1. El filtro de estado de `GET /v1/memories` ya se validaba contra una lista de tres literales (`STATUSES`); ahora es `MemoryStatus::from_name`, con el mismo mensaje, y la lista se va. En el núcleo, `ListParams::status` sigue siendo `StoredStatus`: el núcleo no decide qué filtros acepta el daemon.

## D5. El evento del outbox lleva `Option<StoredType>`

core-use-cases dejó `MemoryEvent::memory_type` en `Option<String>` (su D5) porque una fila del outbox escrita por otra versión puede traer un nombre que no es de los siete. `StoredType` cumple lo mismo con tipo: lee ese nombre sin fallar, se serializa igual, y `NewMemoryRecord::from_event_and_chunk` cae a `SemanticFact` con `known()`, como caía con `from_db_str`. Una revisión que hereda el tipo de la fila que reemplaza (`topic.rs:164`) pasa el `StoredType` tal cual.

## D6. Lo que no se tipa

- **La entrada de la CLI y del puente MCP** (`SaveArgs`, `ListArgs`, `UpdateBody`, los argumentos de las herramientas): el daemon la valida y su error llega al usuario o al modelo sin cambiar. Parsearla antes cambiaría el mensaje y el esquema que ve el modelo. `StatusArg`, de clap, se convierte a `MemoryStatus` en vez de tener su propio `as_str`.
- **Las claves de los reportes** (`DeleteReport::by_type`, `ImportReport::by_type`, `MemoryStats::by_type`) y las etiquetas de las métricas de sesión: agrupan texto que se muestra, no son una fila de la que se decida algo.
- **`eval_api`**: sus tipos públicos (`EvalHit::memory_type`, los argumentos `&str`) quedan como están, como en el D6 de core-use-cases; convierte en su borde con `to_string` y `StoredType::read`.
- **`LostSource::state`** del cliente: es un `SourceState` (`live`, `deleted`, `superseded`, `purged`), no un estado de fila.

## D7. Los tests

Los tests nuevos de `Stored<T>` y de conservación no compilan antes del cambio. Los existentes que construían una fila con un literal pasan a `Stored::read("...")`, y los que comparaban un campo con un literal, a `.as_str()`: cambia cómo se escribe la aserción, no lo que afirma. Ningún literal de `crates/clawg-cli/tests/api.rs` cambia.
